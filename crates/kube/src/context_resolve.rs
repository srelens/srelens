//! Context enumeration with duplicate-name disambiguation.
//!
//! Kubeconfig merge (kubectl semantics, [`kube::config::Kubeconfig::merge`])
//! silently drops any context/cluster/user whose name already exists —
//! "first file wins". A user who adds many per-cluster kubeconfigs that reuse a
//! generic context name (`default`, `kubernetes-admin@kubernetes`) therefore
//! sees only the first of each name, and can only ever connect to that one.
//!
//! We instead enumerate every context across all files and give each a unique
//! *display name*: a name that is globally unique is kept as-is (kubectl-
//! compatible, zero change for normal setups); a name that appears in more than
//! one file is prefixed with its source file's stem — `default` becomes
//! `kube_prod/default` — so every cluster is visible and each display name
//! round-trips to the exact file that owns it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use kube::config::Kubeconfig;

/// One context resolved to a unique, user-facing identity plus everything
/// needed to enumerate it and to reconnect via its own file.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedContext {
    /// Unique, user-facing name (disambiguated on collision).
    pub display_name: String,
    /// The context's name within its own file (what kube-rs expects).
    pub original_name: String,
    /// The kubeconfig file that declares this context.
    pub source: PathBuf,
    pub cluster: String,
    pub server: String,
    pub user: String,
    pub namespace: String,
    pub is_current: bool,
    /// The user's exec auth command, if any (for local/remote classification).
    pub exec_command: Option<String>,
    /// The user's auth-provider name, if any (for classification).
    pub auth_provider: Option<String>,
}

/// A parsed kubeconfig paired with the file it came from.
pub struct SourceConfig {
    pub source: PathBuf,
    pub config: Kubeconfig,
}

/// File stem used to disambiguate a colliding name (`~/.kube/prod.yaml` → `prod`).
fn source_tag(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or("kubeconfig")
        .to_string()
}

/// Enumerate every context across the parsed configs (in file order), assigning
/// each a unique display name. Pure over parsed input so it is unit-testable
/// without touching the filesystem.
pub fn resolve_from(configs: &[SourceConfig]) -> Vec<ResolvedContext> {
    // The effective current-context is the first one set, matching merge's
    // `self.current_context.or(next)` fold over the files in order.
    let global_current = configs
        .iter()
        .find_map(|sc| sc.config.current_context.clone())
        .filter(|current| !current.is_empty());

    // Count each original name across all files so we only prefix real clashes.
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for sc in configs {
        for named in &sc.config.contexts {
            *counts.entry(named.name.as_str()).or_default() += 1;
        }
    }

    let mut used: HashMap<String, usize> = HashMap::new();
    let mut current_taken = false;
    let mut out = Vec::new();

    for sc in configs {
        let tag = source_tag(&sc.source);
        for named in &sc.config.contexts {
            let original = named.name.clone();
            // Prefix only names that appear in more than one file.
            let base = if counts.get(original.as_str()).copied().unwrap_or(0) > 1 {
                format!("{tag}/{original}")
            } else {
                original.clone()
            };
            // Guarantee global uniqueness even if base itself repeats (two files
            // with the same stem and context name): suffix with a counter.
            let seen = used.entry(base.clone()).or_insert(0);
            *seen += 1;
            let display_name = if *seen == 1 { base.clone() } else { format!("{base} ({seen})") };

            let context = named.context.clone().unwrap_or_default();
            let cluster_name = context.cluster;
            let user_name = context.user;
            let server = sc
                .config
                .clusters
                .iter()
                .find(|cluster| cluster.name == cluster_name)
                .and_then(|cluster| cluster.cluster.as_ref())
                .and_then(|cluster| cluster.server.clone())
                .unwrap_or_default();
            let auth = sc
                .config
                .auth_infos
                .iter()
                .find(|entry| entry.name == user_name)
                .and_then(|entry| entry.auth_info.as_ref());
            let exec_command = auth
                .and_then(|info| info.exec.as_ref())
                .and_then(|exec| exec.command.clone());
            let auth_provider = auth
                .and_then(|info| info.auth_provider.as_ref())
                .map(|provider| provider.name.clone());

            let is_current = !current_taken
                && global_current.as_deref() == Some(original.as_str());
            if is_current {
                current_taken = true;
            }

            out.push(ResolvedContext {
                display_name,
                original_name: original,
                source: sc.source.clone(),
                cluster: cluster_name,
                server,
                user: user_name,
                namespace: context.namespace.unwrap_or_default(),
                is_current,
                exec_command,
                auth_provider,
            });
        }
    }
    out
}

/// Read each path and resolve its contexts. Unreadable files are skipped so one
/// bad path can't hide every other cluster.
pub fn resolve_contexts(paths: &[PathBuf]) -> Vec<ResolvedContext> {
    let configs: Vec<SourceConfig> = paths
        .iter()
        .filter_map(|path| {
            Kubeconfig::read_from(path)
                .ok()
                .map(|config| SourceConfig { source: path.clone(), config })
        })
        .collect();
    resolve_from(&configs)
}

/// Find a resolved context by display name, falling back to a raw original name
/// (for MCP/tests that pass the kubeconfig's own context name directly).
pub fn resolve_context(paths: &[PathBuf], name: &str) -> Option<ResolvedContext> {
    let all = resolve_contexts(paths);
    all.iter()
        .find(|context| context.display_name == name)
        .or_else(|| all.iter().find(|context| context.original_name == name))
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(source: &str, yaml: &str) -> SourceConfig {
        SourceConfig {
            source: PathBuf::from(source),
            config: Kubeconfig::from_yaml(yaml).unwrap(),
        }
    }

    const PROD: &str = "apiVersion: v1\nkind: Config\ncurrent-context: default\nclusters:\n  - name: c\n    cluster: { server: https://prod:6443 }\nusers:\n  - name: u\n    user: {}\ncontexts:\n  - name: default\n    context: { cluster: c, user: u }\n";
    const STAGE: &str = "apiVersion: v1\nkind: Config\ncurrent-context: default\nclusters:\n  - name: c\n    cluster: { server: https://stage:6443 }\nusers:\n  - name: u\n    user: {}\ncontexts:\n  - name: default\n    context: { cluster: c, user: u }\n";

    #[test]
    fn unique_names_are_kept_verbatim() {
        let a = cfg(
            "/home/config",
            "clusters:\n  - name: ca\n    cluster: { server: https://a }\ncontexts:\n  - name: ctx-a\n    context: { cluster: ca, user: u }\n  - name: ctx-b\n    context: { cluster: ca, user: u }\n",
        );
        let resolved = resolve_from(&[a]);
        assert_eq!(
            resolved.iter().map(|c| c.display_name.as_str()).collect::<Vec<_>>(),
            vec!["ctx-a", "ctx-b"],
        );
        assert_eq!(resolved[0].original_name, "ctx-a");
    }

    #[test]
    fn colliding_names_are_prefixed_with_the_source_stem() {
        let resolved = resolve_from(&[
            cfg("/kube/kube_prod.yaml", PROD),
            cfg("/kube/kube_stage.yaml", STAGE),
        ]);
        // Both survive (nothing dropped) and each maps to its own server.
        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[0].display_name, "kube_prod/default");
        assert_eq!(resolved[1].display_name, "kube_stage/default");
        assert_eq!(resolved[0].original_name, "default");
        assert_eq!(resolved[1].original_name, "default");
        assert_eq!(resolved[0].server, "https://prod:6443");
        assert_eq!(resolved[1].server, "https://stage:6443");
    }

    #[test]
    fn a_name_unique_across_files_stays_plain_even_when_others_collide() {
        let uniq = cfg(
            "/kube/main",
            "clusters:\n  - name: c\n    cluster: { server: https://m }\ncontexts:\n  - name: my-cluster\n    context: { cluster: c, user: u }\n",
        );
        let resolved = resolve_from(&[uniq, cfg("/kube/kube_prod.yaml", PROD), cfg("/kube/kube_stage.yaml", STAGE)]);
        let names: Vec<_> = resolved.iter().map(|c| c.display_name.as_str()).collect();
        assert!(names.contains(&"my-cluster"));
        assert!(names.contains(&"kube_prod/default"));
        assert!(names.contains(&"kube_stage/default"));
    }

    #[test]
    fn identical_stem_and_name_get_a_counter_suffix() {
        // Two different dirs, same file stem `config`, same context name.
        let resolved = resolve_from(&[cfg("/a/config", PROD), cfg("/b/config", STAGE)]);
        assert_eq!(resolved[0].display_name, "config/default");
        assert_eq!(resolved[1].display_name, "config/default (2)");
    }

    #[test]
    fn first_file_to_set_current_context_wins() {
        let mut resolved = resolve_from(&[
            cfg("/kube/kube_prod.yaml", PROD),
            cfg("/kube/kube_stage.yaml", STAGE),
        ]);
        assert!(resolved[0].is_current, "prod's default should be current");
        assert!(!resolved[1].is_current);
        // Only one is ever current.
        resolved.retain(|c| c.is_current);
        assert_eq!(resolved.len(), 1);
    }

    #[test]
    fn lookup_round_trips_display_and_falls_back_to_original() {
        let resolved = resolve_from(&[
            cfg("/kube/kube_prod.yaml", PROD),
            cfg("/kube/kube_stage.yaml", STAGE),
        ]);
        let by_display = resolved.iter().find(|c| c.display_name == "kube_stage/default").unwrap();
        assert_eq!(by_display.source, PathBuf::from("/kube/kube_stage.yaml"));
        assert_eq!(by_display.original_name, "default");
    }
}
