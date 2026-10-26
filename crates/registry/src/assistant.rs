//! The AI Assistant setting (#968), read for every srelens MCP server.
//!
//! The frontend writes `srelens.assistant` into the durable settings file;
//! `srelens_mcp::assistant` decides what a call may do. This is the half in
//! between: it reads the file on every call (the GUI and a headless process can
//! both be running, as for the settings store itself), and turns the stored
//! stable IDs into every name a call could use for those clusters.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;
use srelens_kube::client_cache::ClientCache;
use srelens_kube::context_resolve::ResolvedContext;
use srelens_mcp::assistant::{AssistantSnapshot, AssistantSource, DisabledCluster, SETTING_KEY};

/// `(enabled, disabled stable IDs)` from the settings file at `settings`.
///
/// No path (no config dir) or no file: on, nothing disabled — there is nothing
/// a person could have set. A file that cannot be read or parsed is an error,
/// so the caller refuses rather than reading it as on. A value of the wrong
/// shape reads as on, exactly as the frontend reads it
/// (`packages/core/src/lib/assistantSetting.ts`): only the boolean `false`
/// turns it off, and non-string IDs are dropped.
pub fn read_assistant(settings: Option<&Path>) -> Result<(bool, Vec<String>), String> {
    let Some(path) = settings else { return Ok((true, Vec::new())) };
    // A string value is the text of the document, as `settingsStorage` hands
    // it to the frontend — read the same way, or the two disagree.
    let value = match crate::settings::read_value(path, SETTING_KEY)? {
        Some(Value::String(text)) => serde_json::from_str(&text).ok(),
        other => other,
    };
    let Some(Value::Object(doc)) = value else {
        return Ok((true, Vec::new()));
    };
    let enabled = doc.get("enabled") != Some(&Value::Bool(false));
    let disabled = doc
        .get("disabledClusters")
        .and_then(Value::as_array)
        .map(|ids| ids.iter().filter_map(Value::as_str).map(str::to_owned).collect())
        .unwrap_or_default();
    Ok((enabled, disabled))
}

/// The setting, read per call, matched against this host's kubeconfig files
/// as its client cache holds them now.
pub struct SettingsAssistant {
    settings: Option<PathBuf>,
    cache: Arc<ClientCache>,
}

impl SettingsAssistant {
    pub fn new(settings: Option<PathBuf>, cache: Arc<ClientCache>) -> Self {
        Self { settings, cache }
    }
}

#[async_trait::async_trait]
impl AssistantSource for SettingsAssistant {
    async fn snapshot(&self) -> Result<AssistantSnapshot, String> {
        let (enabled, ids) = read_assistant(self.settings.as_deref())?;
        if !enabled || ids.is_empty() {
            return Ok(AssistantSnapshot { enabled, disabled: Vec::new() });
        }
        let contexts = srelens_kube::context_resolve::resolve_contexts(&self.cache.paths().await);
        Ok(AssistantSnapshot { enabled, disabled: disabled_clusters(&ids, &contexts) })
    }
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// A stored stable ID with its file made canonical. Stable IDs are ambiguous
/// when a path holds `#`, so the file system decides: the first prefix that is
/// an existing file is the path. None is: the ID as written.
fn canonical_stored(id: &str) -> String {
    id.match_indices('#')
        .map(|(i, _)| (Path::new(&id[..i]), &id[i + 1..]))
        .find(|(path, _)| path.is_file())
        .map_or_else(|| id.to_string(), |(path, name)| format!("{}#{name}", canonical(path).display()))
}

/// A context with only its file and name, so its own `pinned_id()` and `key()`
/// give the identities a listed context of that file and name would have.
fn stand_in(path: &str, name: &str) -> ResolvedContext {
    ResolvedContext {
        display_name: name.to_string(),
        original_name: name.to_string(),
        source: PathBuf::from(path),
        cluster: String::new(),
        server: String::new(),
        user: String::new(),
        namespace: String::new(),
        is_current: false,
        exec_command: None,
        auth_provider: None,
        auth_kind: String::new(),
    }
}

/// The switched-off clusters among `contexts`, each with every name a call
/// could use for it. Matched by canonical file, so the same kubeconfig spelled
/// another way — another slash, a `.`, a headless process's own `KUBECONFIG`
/// spelling — is still the cluster that was switched off. An ID nothing listed
/// matches still refuses by that ID and the context name it ends in.
pub(crate) fn disabled_clusters(ids: &[String], contexts: &[ResolvedContext]) -> Vec<DisabledCluster> {
    let mut disabled = Vec::new();
    for id in ids {
        let key = canonical_stored(id);
        let before = disabled.len();
        for context in contexts {
            if format!("{}#{}", canonical(&context.source).display(), context.original_name) != key {
                continue;
            }
            let names = [
                context.pinned_id(),
                Some(context.stable_id()),
                // An app's cluster scope holds this form (extensions.configure).
                Some(context.key()),
                Some(context.display_name.clone()),
                Some(context.original_name.clone()),
            ];
            disabled.push(DisabledCluster {
                label: context.display_name.clone(),
                names: names.into_iter().flatten().collect(),
            });
        }
        if disabled.len() == before {
            // Placed nowhere: refuse by the ID and by every name it could end
            // in, since a context name may itself hold `#`. Fail closed —
            // matching more can only refuse more.
            // Its pinned ID and key are rebuilt too, for every way the ID can
            // split: a client cached under the pinned ID while the file was
            // readable would otherwise still serve it.
            let mut names = vec![id.clone()];
            for (i, _) in id.match_indices('#') {
                let (path, name) = (&id[..i], &id[i + 1..]);
                names.push(name.to_string());
                let stand_in = stand_in(path, name);
                names.extend(stand_in.pinned_id());
                names.push(stand_in.key());
            }
            names.retain(|name| !name.is_empty());
            disabled.push(DisabledCluster { label: id.clone(), names });
        }
    }
    disabled
}

#[cfg(test)]
mod tests {
    use super::*;
    use srelens_kube::client_cache::ClientCache;
    use srelens_mcp::assistant::AssistantSource;
    use std::path::{Path, PathBuf};

    const KUBECONFIG: &str = "apiVersion: v1\nkind: Config\n\
        clusters: [{name: c, cluster: {server: 'https://127.0.0.1:1'}}]\n\
        contexts: [{name: prod, context: {cluster: c, user: u}}, {name: dev, context: {cluster: c, user: u}}]\n\
        users: [{name: u, user: {}}]\n";

    fn settings_with(dir: &Path, value: &str) -> PathBuf {
        let path = dir.join("settings.json");
        std::fs::write(&path, format!(r#"{{"schema_version":1,"values":{{"srelens.assistant":{value}}}}}"#)).unwrap();
        path
    }

    fn kubeconfig(dir: &Path) -> PathBuf {
        let path = dir.join("config");
        std::fs::write(&path, KUBECONFIG).unwrap();
        path
    }

    #[test]
    fn no_settings_path_means_on_with_nothing_disabled() {
        assert_eq!(read_assistant(None), Ok((true, vec![])));
    }

    #[test]
    fn a_missing_file_means_on() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_assistant(Some(&dir.path().join("settings.json"))), Ok((true, vec![])));
    }

    #[test]
    fn a_corrupt_settings_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert!(read_assistant(Some(&path)).is_err());
    }

    #[test]
    fn a_malformed_value_reads_as_on() {
        let dir = tempfile::tempdir().unwrap();
        let path = settings_with(dir.path(), r#"{"enabled":"false","disabledClusters":"x"}"#);
        assert_eq!(read_assistant(Some(&path)), Ok((true, vec![])));
    }

    /// Review finding: `settingsStorage` hands a string value back as the text
    /// it holds, so the frontend parses a JSON string as the document. Read it
    /// the same way here, or the UI hides what the gate still serves.
    #[test]
    fn a_string_value_is_read_as_the_frontend_reads_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = settings_with(dir.path(), r#""{\"enabled\":false,\"disabledClusters\":[\"/k#prod\"]}""#);
        assert_eq!(read_assistant(Some(&path)), Ok((false, vec!["/k#prod".to_string()])));
        let not_json = settings_with(dir.path(), r#""off""#);
        assert_eq!(read_assistant(Some(&not_json)), Ok((true, vec![])));
    }

    #[test]
    fn a_real_off_and_its_clusters_are_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = settings_with(dir.path(), r#"{"enabled":false,"disabledClusters":["/k#prod",3]}"#);
        assert_eq!(read_assistant(Some(&path)), Ok((false, vec!["/k#prod".to_string()])));
    }

    #[test]
    fn the_same_kubeconfig_spelled_another_way_is_still_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let config = kubeconfig(dir.path());
        let contexts = srelens_kube::context_resolve::resolve_contexts(&[config.clone()]);
        let prod = contexts.iter().find(|c| c.original_name == "prod").unwrap();
        // The same file through `.` and, on Windows, with the other slash.
        let dotted = format!("{}#prod", dir.path().join(".").join("config").display());
        let mut spellings = vec![dotted];
        if cfg!(windows) {
            spellings.push(format!("{}#prod", config.display().to_string().replace('\\', "/")));
        }
        for id in spellings {
            let disabled = disabled_clusters(&[id.clone()], &contexts);
            assert_eq!(disabled.len(), 1, "{id}");
            assert_eq!(disabled[0].label, prod.display_name);
            assert!(disabled[0].names.contains(&prod.pinned_id().unwrap()), "{id}: {:?}", disabled[0].names);
            assert!(disabled[0].names.contains(&"prod".to_string()));
            assert!(!disabled[0].names.contains(&"dev".to_string()));
        }
    }

    /// Review finding: an app's cluster scope holds a context's `key()` —
    /// its stable ID with `%` and `#` encoded — so that form names it too.
    #[test]
    fn a_disabled_cluster_is_known_by_its_key_too() {
        let dir = tempfile::tempdir().unwrap();
        let odd = dir.path().join("team%1");
        std::fs::create_dir(&odd).unwrap();
        let config = kubeconfig(&odd);
        let contexts = srelens_kube::context_resolve::resolve_contexts(&[config]);
        let prod = contexts.iter().find(|c| c.original_name == "prod").unwrap();
        assert_ne!(prod.key(), prod.stable_id(), "the fixture must make the two forms differ");
        let disabled = disabled_clusters(&[prod.stable_id()], &contexts);
        assert!(disabled[0].names.contains(&prod.key()), "{:?}", disabled[0].names);
    }

    /// Review (PR #984): a switched-off cluster whose kubeconfig can no longer
    /// be read is listed nowhere, yet a client cached under its pinned ID could
    /// still serve a call — so the pinned ID and key are rebuilt from the ID.
    #[test]
    fn an_unplaced_disabled_id_still_refuses_its_pinned_id_and_key() {
        let dir = tempfile::tempdir().unwrap();
        let odd = dir.path().join("team%1");
        std::fs::create_dir(&odd).unwrap();
        let config = kubeconfig(&odd);
        let contexts = srelens_kube::context_resolve::resolve_contexts(&[config.clone()]);
        let prod = contexts.iter().find(|c| c.original_name == "prod").unwrap();

        // The file is gone: nothing is listed any more.
        std::fs::remove_file(&config).unwrap();
        let disabled = disabled_clusters(&[prod.stable_id()], &[]);

        assert_eq!(disabled.len(), 1);
        assert!(disabled[0].names.contains(&prod.pinned_id().unwrap()), "{:?}", disabled[0].names);
        assert!(disabled[0].names.contains(&prod.key()), "{:?}", disabled[0].names);
    }

    /// Security review: an ID this process cannot place still refuses by its
    /// context name — fail closed, since this process may spell the file
    /// some way canonicalisation cannot reconcile.
    #[test]
    fn a_disabled_id_no_listed_context_matches_still_refuses_by_that_id_and_its_name() {
        let disabled = disabled_clusters(&["/gone#prod".to_string()], &[]);
        assert_eq!(disabled.len(), 1);
        assert!(disabled[0].names.contains(&"/gone#prod".to_string()));
        assert!(disabled[0].names.contains(&"prod".to_string()));
        // A context name holding `#` is refused whole as well as by its tail.
        let odd = disabled_clusters(&["/gone#team#prod".to_string()], &[]);
        assert!(odd[0].names.contains(&"team#prod".to_string()));
        assert!(odd[0].names.contains(&"prod".to_string()));
    }

    #[tokio::test]
    async fn the_source_reads_the_file_and_the_hosts_kubeconfigs() {
        let dir = tempfile::tempdir().unwrap();
        let config = kubeconfig(dir.path());
        let id = format!("{}#prod", config.display());
        let settings = settings_with(dir.path(), &format!(r#"{{"enabled":true,"disabledClusters":[{}]}}"#, serde_json::json!(id)));
        let source = SettingsAssistant::new(Some(settings), ClientCache::new_many(vec![config]));
        let snapshot = source.snapshot().await.unwrap();
        assert!(snapshot.enabled);
        assert_eq!(snapshot.disabled.len(), 1);
        assert!(snapshot.disabled[0].names.contains(&"prod".to_string()));
    }
}
