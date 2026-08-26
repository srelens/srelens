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

use kube::config::{AuthInfo, Kubeconfig};

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
    /// The credential MECHANISM this context's `auth-info` uses — never the
    /// credential itself. See [`auth_kind_of`] for exactly what values this
    /// takes. Computed once here, while the file is already parsed in memory,
    /// rather than by re-reading the kubeconfig again per context.
    pub auth_kind: String,
}

impl ResolvedContext {
    /// An identity that survives a rename.
    ///
    /// `display_name` is not identity: it gains a `file/` prefix the moment
    /// another kubeconfig declares the same context name, so a context the
    /// user never touched can be renamed by adding an unrelated file — taking
    /// every per-context setting keyed by that name with it (#265). The
    /// declaring file plus the name inside that file does not move.
    pub fn stable_id(&self) -> String {
        format!("{}#{}", self.source.display(), self.original_name)
    }
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
            let user_name = context.user.unwrap_or_default();
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
            let auth_kind = auth
                .map(auth_kind_of)
                .unwrap_or_else(|| "none".to_string());

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
                auth_kind,
            });
        }
    }
    out
}

/// Map a kubeconfig's `auth-info` to the credential MECHANISM it names —
/// never the credential. Exec ARGUMENTS are deliberately dropped: they
/// routinely carry client IDs and, in bad kubeconfigs, secrets, and this
/// string ends up rendered in a table.
///
/// A legacy `auth-provider` block (`gcp`, `azure`, `oidc`, or any other name a
/// plugin registers) is named as exactly what the kubeconfig calls it —
/// never generalised to `oidc` for all of them, which would assert something
/// the source doesn't say. The only extra detail ever appended is a plain-text
/// `email` already sitting in that provider's config map; nothing else from
/// that map is ever surfaced, since that's where a refresh token or secret
/// would live.
fn auth_kind_of(auth: &AuthInfo) -> String {
    if let Some(exec) = &auth.exec {
        let command = exec.command.as_deref().unwrap_or("unknown");
        return format!("exec plugin · {command}");
    }
    if let Some(provider) = &auth.auth_provider {
        return match provider.config.get("email") {
            Some(account) => format!("{} · {account}", provider.name),
            None => provider.name.clone(),
        };
    }
    if auth.client_certificate.is_some() || auth.client_certificate_data.is_some() {
        return "client certificate".to_string();
    }
    if auth.token.is_some() || auth.token_file.is_some() {
        return "token".to_string();
    }
    if auth.username.is_some() || auth.password.is_some() {
        return "basic".to_string();
    }
    if auth.impersonate.is_some() {
        return "impersonation".to_string();
    }
    "none".to_string()
}

/// Read each path and resolve its contexts. Unreadable files are skipped so one
/// bad path can't hide every other cluster.
pub fn resolve_contexts(paths: &[PathBuf]) -> Vec<ResolvedContext> {
    resolve_contexts_with(paths, |path| Kubeconfig::read_from(path).ok())
}

/// Same as [`resolve_contexts`], but takes the file reader as a parameter so
/// a test can count how many times each path is actually read. Kept private:
/// this seam exists for that one pinning test, not as a public extension
/// point.
fn resolve_contexts_with(
    paths: &[PathBuf],
    mut read: impl FnMut(&Path) -> Option<Kubeconfig>,
) -> Vec<ResolvedContext> {
    let configs: Vec<SourceConfig> = paths
        .iter()
        .filter_map(|path| read(path).map(|config| SourceConfig { source: path.clone(), config }))
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

    /// An exec-plugin `AuthInfo` naming `command`, with `args` attached the
    /// way a real exec plugin's arguments would be — so the leak test has
    /// something to actually catch if `auth_kind_of` ever started including
    /// them.
    fn exec_auth(command: &str, args: &[&str]) -> AuthInfo {
        AuthInfo {
            exec: Some(kube::config::ExecConfig {
                command: Some(command.to_string()),
                args: Some(args.iter().map(|a| a.to_string()).collect()),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn token_auth(token: &str) -> AuthInfo {
        serde_yaml::from_str(&format!("token: \"{token}\"\n")).expect("valid auth-info fixture")
    }

    fn client_cert_auth() -> AuthInfo {
        AuthInfo {
            client_certificate_data: Some("c2VjcmV0LWNlcnQ=".to_string()),
            ..Default::default()
        }
    }

    fn empty_auth() -> AuthInfo {
        AuthInfo::default()
    }

    /// A legacy `auth-provider` block naming `provider`, with an arbitrary
    /// config map — used to pin that the provider's own name is surfaced
    /// verbatim, and that nothing from `config` leaks except a plain-text
    /// `email` when present.
    fn auth_provider(provider: &str, config: &[(&str, &str)]) -> AuthInfo {
        AuthInfo {
            auth_provider: Some(kube::config::AuthProviderConfig {
                name: provider.to_string(),
                config: config.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
                other: Default::default(),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn auth_kind_names_the_mechanism_and_never_the_secret() {
        assert_eq!(auth_kind_of(&exec_auth("gcloud", &["--client-id", "s3cr3t"])), "exec plugin · gcloud");
        assert_eq!(auth_kind_of(&token_auth("eyJhbGciOi.very.secret")), "token");
        assert_eq!(auth_kind_of(&client_cert_auth()), "client certificate");
        assert_eq!(auth_kind_of(&empty_auth()), "none");
    }

    #[test]
    fn auth_kind_leaks_no_credential_material() {
        let kind = auth_kind_of(&token_auth("eyJhbGciOi.very.secret"));
        assert!(!kind.contains("eyJ"), "auth kind must not carry the token: {kind}");
        let exec = auth_kind_of(&exec_auth("gcloud", &["--client-id", "s3cr3t"]));
        assert!(!exec.contains("s3cr3t"), "auth kind must not carry exec args: {exec}");
    }

    #[test]
    fn legacy_auth_provider_is_named_as_what_it_is() {
        // Each provider is named verbatim — never generalised to `oidc`,
        // which would assert something the kubeconfig doesn't say.
        assert_eq!(auth_kind_of(&auth_provider("gcp", &[])), "gcp");
        assert_eq!(auth_kind_of(&auth_provider("azure", &[])), "azure");
        assert_eq!(auth_kind_of(&auth_provider("oidc", &[])), "oidc");
        assert_eq!(auth_kind_of(&auth_provider("my-custom-plugin", &[])), "my-custom-plugin");
    }

    #[test]
    fn auth_provider_without_email_yields_the_bare_kind() {
        // No plain-text account field present — nothing is invented.
        let kind = auth_kind_of(&auth_provider("oidc", &[("client-id", "abc123")]));
        assert_eq!(kind, "oidc");
    }

    #[test]
    fn auth_provider_with_email_appends_only_the_email() {
        let kind = auth_kind_of(&auth_provider("oidc", &[("email", "dana@example.com")]));
        assert_eq!(kind, "oidc · dana@example.com");
    }

    #[test]
    fn auth_provider_never_surfaces_any_other_config_key() {
        // Secrets and refresh material live in this map under other keys —
        // only `email` is ever allowed through, and only its value.
        let kind = auth_kind_of(&auth_provider(
            "oidc",
            &[
                ("id-token", "eyJ.super.secret"),
                ("refresh-token", "another-secret"),
                ("client-secret", "shh"),
            ],
        ));
        assert_eq!(kind, "oidc", "no email present, so no key from config should leak in");
        assert!(!kind.contains("secret"));
    }

    #[test]
    fn stable_id_survives_the_rename_a_new_file_causes() {
        // The #265 case: a second file declaring the same context name renames
        // the FIRST one, which is how per-context settings got orphaned.
        let alone = resolve_from(&[cfg("/k/prod.yaml", PROD)]);
        assert_eq!(alone[0].display_name, "default", "no clash, no prefix");
        let id_before = alone[0].stable_id();

        let clashing = resolve_from(&[cfg("/k/prod.yaml", PROD), cfg("/k/stage.yaml", STAGE)]);
        let prod = clashing
            .iter()
            .find(|c| c.source == PathBuf::from("/k/prod.yaml"))
            .expect("prod context still resolves");

        // The user-facing name changed out from under the user…
        assert_ne!(prod.display_name, "default", "display name gains a prefix");
        // …but identity did not, which is what settings must key on.
        assert_eq!(prod.stable_id(), id_before, "stable id must not move");

        // And the two colliding contexts remain distinguishable.
        let ids: Vec<String> = clashing.iter().map(|c| c.stable_id()).collect();
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], ids[1], "each file's context has its own identity");
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

    #[test]
    fn a_file_with_several_contexts_is_read_once_not_once_per_context() {
        // auth_kind used to be computed by re-reading each context's owning
        // kubeconfig separately (once per CONTEXT); it must now come from the
        // single parse `resolve_contexts` already does (once per FILE).
        let path = PathBuf::from("/fake/multi.yaml");
        let yaml = "clusters:\n- name: c\n  cluster: { server: https://c }\ncontexts:\n- name: a\n  context: { cluster: c, user: u }\n- name: b\n  context: { cluster: c, user: u }\n- name: c\n  context: { cluster: c, user: u }\nusers:\n- name: u\n  user: { token: t }\n";

        let reads = std::cell::Cell::new(0usize);
        let resolved = resolve_contexts_with(&[path.clone()], |p| {
            assert_eq!(p, path.as_path());
            reads.set(reads.get() + 1);
            Kubeconfig::from_yaml(yaml).ok()
        });

        assert_eq!(resolved.len(), 3, "all three contexts in the file resolve");
        for context in &resolved {
            assert_eq!(context.auth_kind, "token", "auth_kind still computed correctly");
        }
        assert_eq!(reads.get(), 1, "the file must be read once, not once per context");
    }
}
