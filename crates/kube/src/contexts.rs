//! The `k8s.listContexts` capability — reads the kubeconfig and returns its
//! contexts. Surfaced to both the UI and MCP via the shared registry.

use std::path::PathBuf;
use std::sync::Arc;

use srelens_capability::{Annotations, Capability, CapabilityError};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use kube::config::{AuthInfo, Kubeconfig};

use crate::client_cache::ClientCache;
use crate::context_resolve::{resolve_context, resolve_contexts, ResolvedContext};
use crate::local_cluster::classify;

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ListContextsIn {
    /// Additional kubeconfig files merged after the default files.
    pub paths: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ContextDto {
    pub name: String,
    /// Identity that survives a rename: the declaring file plus the context's
    /// name within it. `name` is presentation only — it gains a `file/` prefix
    /// as soon as another kubeconfig declares the same context name, so
    /// anything the app remembers per context must key on this instead (#265).
    #[serde(rename = "stableId")]
    pub stable_id: String,
    pub cluster: String,
    pub server: String,
    /// The context's default namespace from the kubeconfig
    /// (`contexts[].context.namespace`); empty when unset — used to scope
    /// views for namespace-restricted credentials that can't list namespaces.
    pub namespace: String,
    #[serde(rename = "isCurrent")]
    pub is_current: bool,
    /// Whether this context points at a local development cluster (kind, k3d,
    /// minikube, docker-desktop, kiac, vind, …). Classified precision-first:
    /// only a tool-generated name earns this, and cloud auth always wins as
    /// remote, so a production cluster is never marked local. See
    /// [`crate::local_cluster`].
    #[serde(rename = "isLocal")]
    pub is_local: bool,
    /// The detected local provider (e.g. `"kind"`, `"vind"`), when `isLocal`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// The kubeconfig this context was declared in. Taken from the same place
    /// `stable_id` takes it — NOT parsed back out of `stable_id`, whose
    /// `file/` prefix appears only on a name collision (#265), so a unique
    /// name would yield nothing.
    #[serde(rename = "sourceFile")]
    pub source_file: String,
    /// The credential MECHANISM, never the credential. One of `client
    /// certificate`, `token`, `basic`, `exec plugin · <command>`, `oidc`,
    /// `impersonation`, `none`. Exec ARGUMENTS are deliberately excluded:
    /// they routinely carry client IDs and sometimes secrets, and this string
    /// is rendered in a table.
    #[serde(rename = "authKind")]
    pub auth_kind: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListContextsOut {
    pub contexts: Vec<ContextDto>,
}

/// Build the capability over the shared cache. Supplying `paths` replaces the
/// additional kubeconfig files and invalidates authenticated clients.
/// `managed_dir` is the app's own kubeconfig folder, or `None` to disable that
/// discovery. It is passed in rather than looked up globally so the capability
/// stays hermetic — a global lookup makes every test depend on whatever is in
/// the developer's real config directory. The DIRECTORY is fixed but its
/// CONTENTS are read on each call, which is what lets a config pasted or
/// dropped in while the app runs resolve without a restart (#256).
pub fn list_contexts_capability(
    cache: Arc<ClientCache>,
    default_paths: Vec<PathBuf>,
    managed_dir: Option<PathBuf>,
) -> Capability {
    Capability::typed::<ListContextsIn, ListContextsOut, _, _>(
        "k8s.listContexts",
        "list the kube contexts available in the kubeconfig",
        Annotations::READ_ONLY,
        move |input: ListContextsIn| {
            let cache = cache.clone();
            let default_paths = default_paths.clone();
            let managed_dir = managed_dir.clone();
            async move {
                // A caller that names its own files rebuilds from the static
                // defaults; one that doesn't keeps whatever is already active,
                // so an MCP `{}` call never discards paths the desktop set.
                let mut paths = match input.paths {
                    Some(additional) => {
                        let mut paths = default_paths;
                        for path in additional.into_iter().map(PathBuf::from) {
                            if !paths.contains(&path) {
                                paths.push(path);
                            }
                        }
                        paths
                    }
                    None => cache.paths().await,
                };
                // Both branches then reconcile with the disk, so discovery
                // behaves the same however the capability is invoked.
                //
                // Read HERE, not captured at registry-build time: the folder's
                // contents change while the app runs, and a startup snapshot
                // would miss anything pasted or dropped in afterwards (#256).
                if let Some(dir) = &managed_dir {
                    for managed in crate::connect::kubeconfig_files_in(dir) {
                        if !paths.contains(&managed) {
                            paths.push(managed);
                        }
                    }
                }
                // `default_paths` and the cache seed are both snapshots, so a
                // kubeconfig deleted while the app runs would otherwise be
                // reintroduced on every call and sit in the cache forever —
                // where `load_kubeconfigs` is strict and fails the
                // merged-resolution fallback on the missing file. Only ABSENT
                // files are dropped: one that exists but is malformed still
                // reaches the reader and surfaces its parse error, which the
                // caller needs to see.
                paths.retain(|path| path.exists());
                cache.set_paths(paths).await;
                // Enumerate every context across all files with duplicate-name
                // disambiguation, so contexts that share a name (e.g. `default`
                // across per-cluster kubeconfigs) are all visible and each
                // resolves to its own file — kube-rs merge would drop them.
                let paths = cache.paths().await;
                let resolved = resolve_contexts(&paths);
                // Resilient to a bad additional file. An empty result is only an
                // error when *no* kubeconfig could be read at all; a readable file
                // with zero contexts (e.g. after deleting the last one) is fine.
                if resolved.is_empty()
                    && !paths.iter().any(|path| kube::config::Kubeconfig::read_from(path).is_ok())
                {
                    return Err(CapabilityError::Handler(
                        "no kubeconfig contexts could be read".to_string(),
                    ));
                }
                let contexts = resolved.into_iter().map(build_context_dto).collect();
                Ok(ListContextsOut { contexts })
            }
        },
    )
}

/// Build the DTO for one resolved context. Shared by the capability above and
/// by `dto_for` in tests, so the fixture exercises the exact same mapping
/// rather than a hand-rolled duplicate of it.
fn build_context_dto(rc: ResolvedContext) -> ContextDto {
    // Classify on the raw in-file name (the disambiguating prefix is a file
    // stem, not a signal of local/remote).
    let class = classify(
        &rc.original_name,
        &rc.cluster,
        &rc.server,
        rc.exec_command.as_deref(),
        rc.auth_provider.as_deref(),
    );
    let auth_kind = auth_info_for(&rc)
        .map(|info| auth_kind_of(&info))
        .unwrap_or_else(|| "none".to_string());
    ContextDto {
        is_current: rc.is_current,
        stable_id: rc.stable_id(),
        source_file: rc.source.display().to_string(),
        name: rc.display_name,
        cluster: rc.cluster,
        server: rc.server,
        namespace: rc.namespace,
        is_local: class.is_local,
        provider: class.provider.map(|provider| provider.as_str().to_string()),
        auth_kind,
    }
}

/// Look up the full `AuthInfo` a resolved context uses, by re-reading its
/// declaring file. `ResolvedContext` only carries the pieces `local_cluster`
/// classification needs (`exec_command`, `auth_provider` name) — not the full
/// auth block — so the auth kind is derived here, straight from the
/// kubeconfig, rather than by widening `ResolvedContext` for this alone.
fn auth_info_for(rc: &ResolvedContext) -> Option<AuthInfo> {
    let config = Kubeconfig::read_from(&rc.source).ok()?;
    config
        .auth_infos
        .into_iter()
        .find(|entry| entry.name == rc.user)
        .and_then(|entry| entry.auth_info)
}

/// Map a kubeconfig's `auth-info` to the credential MECHANISM it names —
/// never the credential. Exec ARGUMENTS are deliberately dropped: they
/// routinely carry client IDs and, in bad kubeconfigs, secrets, and this
/// string ends up rendered in a table.
fn auth_kind_of(auth: &AuthInfo) -> String {
    if let Some(exec) = &auth.exec {
        let command = exec.command.as_deref().unwrap_or("unknown");
        return format!("exec plugin · {command}");
    }
    if let Some(provider) = &auth.auth_provider {
        // The legacy `auth-provider` block (oidc, and the now-deprecated gcp
        // and azure plugins) is a provider-driven mechanism like exec, not a
        // bare credential — bucketed here as `oidc`. Only a plain-text field
        // the kubeconfig already carries (e.g. an email) is ever appended;
        // nothing from `provider.config` that could be a token or secret is.
        return match provider.config.get("email") {
            Some(account) => format!("oidc · {account}"),
            None => "oidc".to_string(),
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

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteContextIn {
    pub context: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DeleteContextOut {
    pub success: bool,
}

/// Build the capability that deletes a context (and its now-unused cluster and
/// user entries) from whichever kubeconfig file it was found in.
///
/// The kubeconfig is rewritten via `serde_yaml`, so any existing YAML comments
/// or formatting in the source file are not preserved — this is a known
/// limitation. The write itself is atomic: the new contents are written to a
/// temporary file in the same directory and then renamed over the original,
/// so a failure mid-write cannot leave the kubeconfig truncated or corrupted.
pub fn delete_context_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<DeleteContextIn, DeleteContextOut, _, _>(
        "k8s.deleteContext",
        "delete a context and its associated cluster and user from its kubeconfig source",
        Annotations::DESTRUCTIVE,
        move |input: DeleteContextIn| {
            let cache = cache.clone();
            async move {
                let paths = cache.paths().await;
                // Map the display name back to its owning file + in-file name, so
                // a disambiguated duplicate (`kube_prod/default`) deletes the right
                // context and only from its own file — never a same-named sibling.
                let resolved = resolve_context(&paths, &input.context);
                let (context_name, scan_paths): (String, Vec<PathBuf>) = match resolved {
                    Some(target) => (target.original_name, vec![target.source]),
                    None => (input.context, paths),
                };
                let mut found = false;

                for path in scan_paths {
                    if !tokio::fs::try_exists(&path).await.unwrap_or(false) {
                        continue;
                    }
                    let content = match tokio::fs::read_to_string(&path).await {
                        Ok(c) => c,
                        Err(_) => continue,
                    };

                    let mut yaml_value: serde_yaml::Value = match serde_yaml::from_str(&content) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };

                    if let Some(mapping) = yaml_value.as_mapping_mut() {
                        if let Some(contexts) = mapping.get_mut("contexts").and_then(|c| c.as_sequence_mut()) {
                            let mut target_index = None;
                            let mut cluster_to_remove = None;
                            let mut user_to_remove = None;

                            for (i, ctx) in contexts.iter().enumerate() {
                                if let Some(name) = ctx.get("name").and_then(|n| n.as_str()) {
                                    if name == context_name {
                                        target_index = Some(i);
                                        if let Some(context_data) = ctx.get("context") {
                                            cluster_to_remove = context_data.get("cluster").and_then(|c| c.as_str()).map(String::from);
                                            user_to_remove = context_data.get("user").and_then(|u| u.as_str()).map(String::from);
                                        }
                                        break;
                                    }
                                }
                            }

                            if let Some(idx) = target_index {
                                contexts.remove(idx);

                                // Check if cluster or user are used by any other contexts in this file
                                let mut cluster_used = false;
                                let mut user_used = false;
                                for ctx in contexts.iter() {
                                    if let Some(context_data) = ctx.get("context") {
                                        if let Some(c) = context_data.get("cluster").and_then(|c| c.as_str()) {
                                            if Some(c) == cluster_to_remove.as_deref() {
                                                cluster_used = true;
                                            }
                                        }
                                        if let Some(u) = context_data.get("user").and_then(|u| u.as_str()) {
                                            if Some(u) == user_to_remove.as_deref() {
                                                user_used = true;
                                            }
                                        }
                                    }
                                }

                                // Remove the cluster if no longer used
                                if !cluster_used {
                                    if let Some(cluster_name) = cluster_to_remove {
                                        if let Some(clusters) = mapping.get_mut("clusters").and_then(|c| c.as_sequence_mut()) {
                                            if let Some(pos) = clusters.iter().position(|c| c.get("name").and_then(|n| n.as_str()) == Some(&cluster_name)) {
                                                clusters.remove(pos);
                                            }
                                        }
                                    }
                                }

                                // Remove the user if no longer used
                                if !user_used {
                                    if let Some(user_name) = user_to_remove {
                                        if let Some(users) = mapping.get_mut("users").and_then(|u| u.as_sequence_mut()) {
                                            if let Some(pos) = users.iter().position(|u| u.get("name").and_then(|n| n.as_str()) == Some(&user_name)) {
                                                users.remove(pos);
                                            }
                                        }
                                    }
                                }

                                // Update current-context if needed
                                if let Some(current_context) = mapping.get_mut("current-context") {
                                    if current_context.as_str() == Some(&context_name) {
                                        mapping.remove("current-context");
                                    }
                                }

                                // Write back atomically: write to a temp file in the same
                                // directory, then rename over the original so a mid-write
                                // failure can't truncate or corrupt the kubeconfig.
                                let updated_yaml = serde_yaml::to_string(&yaml_value)
                                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                                let dir =
                                    path.parent().unwrap_or_else(|| std::path::Path::new("."));
                                let tmp = dir.join(format!(
                                    ".{}.srelens-tmp",
                                    path.file_name()
                                        .and_then(|f| f.to_str())
                                        .unwrap_or("kubeconfig")
                                ));
                                tokio::fs::write(&tmp, &updated_yaml)
                                    .await
                                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                                #[cfg(unix)]
                                {
                                    use std::os::unix::fs::PermissionsExt;
                                    if let Ok(meta) = tokio::fs::metadata(&path).await {
                                        let _ = tokio::fs::set_permissions(
                                            &tmp,
                                            std::fs::Permissions::from_mode(
                                                meta.permissions().mode(),
                                            ),
                                        )
                                        .await;
                                    }
                                }
                                tokio::fs::rename(&tmp, &path)
                                    .await
                                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;

                                found = true;
                                break;
                            }
                        }
                    }
                }

                if found {
                    cache.invalidate(&context_name).await;
                    Ok(DeleteContextOut { success: true })
                } else {
                    Err(CapabilityError::Handler(format!(
                        "Context '{}' not found in any loaded kubeconfigs",
                        context_name
                    )))
                }
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use srelens_capability::Registry;
    use serde_json::json;

    /// Build the DTO for a context declared in `file`, with no cluster/auth
    /// details — enough to exercise `build_context_dto`'s file/id handling
    /// without touching disk (`auth_info_for` fails closed to `None` for a
    /// file that doesn't exist, which is fine: these tests don't assert on
    /// `auth_kind`).
    fn dto_for(name: &str, file: &str) -> ContextDto {
        build_context_dto(ResolvedContext {
            display_name: name.to_string(),
            original_name: name.to_string(),
            source: PathBuf::from(file),
            cluster: String::new(),
            server: String::new(),
            user: String::new(),
            namespace: String::new(),
            is_current: false,
            exec_command: None,
            auth_provider: None,
        })
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

    #[test]
    fn context_dto_names_the_file_it_was_declared_in() {
        let dto = dto_for("prod-eu", "/home/dana/.kube/config");
        assert_eq!(dto.source_file, "/home/dana/.kube/config");
    }

    #[test]
    fn a_unique_context_name_still_carries_its_file() {
        // `display_name` (the DTO's `name`) gains a `stem/` prefix only on a
        // cross-file collision (#265); a unique name carries none. `stable_id`
        // itself is unconditionally `{source}#{original_name}` — it always
        // embeds the path, collision or not — so it can't be what
        // distinguishes this case. The point stands regardless: `source_file`
        // must come from `ResolvedContext.source` directly, the same place
        // `stable_id` takes it, not by parsing either string back apart.
        let dto = dto_for("only-one", "/home/dana/.kube/edge.yaml");
        assert_eq!(dto.name, "only-one", "unique name carries no collision prefix");
        assert_eq!(dto.source_file, "/home/dana/.kube/edge.yaml");
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
    fn capability_has_expected_id_and_annotations() {
        let path = PathBuf::from("/nonexistent");
        let cap = list_contexts_capability(ClientCache::new(path.clone()), vec![path], None);
        assert_eq!(cap.id, "k8s.listContexts");
        assert!(cap.annotations.read_only);
    }

    #[tokio::test]
    async fn reads_and_parses_a_kubeconfig_file() {
        let dir = std::env::temp_dir();
        let path = dir.join("srelens-test-kubeconfig.yaml");
        tokio::fs::write(
            &path,
            "clusters:\n  - name: a\n    cluster: { server: https://a }\ncontexts:\n  - name: ctx-a\n    context: { cluster: a, user: user-a }\n",
        )
        .await
        .unwrap();

        let mut reg = Registry::new();
        reg.register(list_contexts_capability(ClientCache::new(path.clone()), vec![path.clone()], None));
        let out = reg.invoke("k8s.listContexts", json!({})).await.unwrap();

        assert_eq!(out["contexts"][0]["name"], "ctx-a");
        assert_eq!(out["contexts"][0]["server"], "https://a");
        let _ = tokio::fs::remove_file(&path).await;
    }

    #[tokio::test]
    async fn missing_file_is_a_handler_error() {
        let mut reg = Registry::new();
        let path = PathBuf::from("/no/such/kubeconfig");
        reg.register(list_contexts_capability(ClientCache::new(path.clone()), vec![path], None));
        let err = reg.invoke("k8s.listContexts", json!({})).await.unwrap_err();
        assert!(matches!(err, CapabilityError::Handler(_)));
    }

    #[tokio::test]
    async fn exposes_the_context_declared_namespace() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "srelens-ctx-namespace-kubeconfig-{}.yaml",
            std::process::id()
        ));
        tokio::fs::write(
            &path,
            r#"apiVersion: v1
kind: Config
clusters:
- name: c
  cluster: { server: "https://example.com" }
contexts:
- name: team-a-ctx
  context: { cluster: c, user: u, namespace: team-a }
- name: no-ns-ctx
  context: { cluster: c, user: u }
users:
- name: u
  user: {}
"#,
        )
        .await
        .unwrap();

        let mut reg = Registry::new();
        reg.register(list_contexts_capability(ClientCache::new(path.clone()), vec![path.clone()], None));
        let out = reg.invoke("k8s.listContexts", json!({})).await.unwrap();
        let contexts = out["contexts"].as_array().unwrap();

        let team_a = contexts.iter().find(|c| c["name"] == "team-a-ctx").unwrap();
        assert_eq!(team_a["namespace"], "team-a");

        let no_ns = contexts.iter().find(|c| c["name"] == "no-ns-ctx").unwrap();
        assert_eq!(no_ns["namespace"], "");

        let _ = tokio::fs::remove_file(&path).await;
    }

    #[tokio::test]
    async fn classifies_local_and_remote_contexts() {
        let dir = std::env::temp_dir();
        // Unique per process so concurrent test runs don't collide on the path.
        let path = dir.join(format!("srelens-classify-kubeconfig-{}.yaml", std::process::id()));
        // A local kind cluster (client-cert auth) and a managed EKS cluster
        // (aws exec plugin) side by side.
        tokio::fs::write(
            &path,
            r#"apiVersion: v1
kind: Config
clusters:
- name: kind-dev
  cluster: { server: "https://127.0.0.1:6443" }
- name: eks-prod
  cluster: { server: "https://abc123.gr7.us-east-1.eks.amazonaws.com" }
contexts:
- name: kind-dev
  context: { cluster: kind-dev, user: kind-dev }
- name: eks-prod
  context: { cluster: eks-prod, user: eks-prod }
users:
- name: kind-dev
  user: { client-certificate-data: abc }
- name: eks-prod
  user:
    exec:
      apiVersion: client.authentication.k8s.io/v1beta1
      command: aws
      args: ["eks", "get-token"]
"#,
        )
        .await
        .unwrap();

        let mut reg = Registry::new();
        reg.register(list_contexts_capability(ClientCache::new(path.clone()), vec![path.clone()], None));
        let out = reg.invoke("k8s.listContexts", json!({})).await.unwrap();
        let contexts = out["contexts"].as_array().unwrap();

        let kind = contexts.iter().find(|c| c["name"] == "kind-dev").unwrap();
        assert_eq!(kind["isLocal"], true);
        assert_eq!(kind["provider"], "kind");

        // Managed EKS: remote despite reachability, and `provider` is omitted.
        let eks = contexts.iter().find(|c| c["name"] == "eks-prod").unwrap();
        assert_eq!(eks["isLocal"], false);
        assert!(eks["provider"].is_null());

        let _ = tokio::fs::remove_file(&path).await;
    }

    #[tokio::test]
    async fn a_kubeconfig_dropped_into_the_managed_folder_resolves_without_a_restart() {
        // The #256 case: the capability is built once at startup, and the file
        // appears afterwards. Reading the folder per call — rather than
        // capturing its contents — is what makes it visible.
        let dir = std::env::temp_dir().join(format!("srelens-managed-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let base = dir.join("base.yaml");
        tokio::fs::write(
            &base,
            "clusters:\n- name: a\n  cluster: { server: https://a }\ncontexts:\n- name: ctx-a\n  context: { cluster: a, user: user-a }\n",
        ).await.unwrap();

        let managed = dir.join("managed");
        std::fs::create_dir_all(&managed).unwrap();

        let cache = ClientCache::new(base.clone());
        let mut reg = Registry::new();
        reg.register(list_contexts_capability(
            cache,
            vec![base.clone()],
            Some(managed.clone()),
        ));

        // Registered while the folder is empty.
        let before = reg.invoke("k8s.listContexts", json!({ "paths": [] })).await.unwrap();
        assert_eq!(before["contexts"].as_array().unwrap().len(), 1);

        // Now a config is pasted in, long after the capability was built.
        tokio::fs::write(
            managed.join("pasted.yaml"),
            "clusters:\n- name: b\n  cluster: { server: https://b }\ncontexts:\n- name: ctx-b\n  context: { cluster: b, user: user-b }\n",
        ).await.unwrap();

        let after = reg.invoke("k8s.listContexts", json!({ "paths": [] })).await.unwrap();
        let names: Vec<String> = after["contexts"].as_array().unwrap().iter()
            .map(|c| c["name"].as_str().unwrap().to_string()).collect();
        assert!(names.iter().any(|n| n == "ctx-b"), "pasted context missing: {names:?}");

        // …and removing it takes the context away again, no restart either.
        std::fs::remove_file(managed.join("pasted.yaml")).unwrap();
        let removed = reg.invoke("k8s.listContexts", json!({ "paths": [] })).await.unwrap();
        let names: Vec<String> = removed["contexts"].as_array().unwrap().iter()
            .map(|c| c["name"].as_str().unwrap().to_string()).collect();
        assert!(!names.iter().any(|n| n == "ctx-b"), "deleted context lingered: {names:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_managed_file_deleted_at_runtime_leaves_the_active_path_set() {
        let dir = std::env::temp_dir().join(format!("srelens-repro-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let base = dir.join("base.yaml");
        std::fs::write(&base, "clusters:\n- name: a\n  cluster: { server: https://a }\ncontexts:\n- name: ctx-a\n  context: { cluster: a, user: user-a }\n").unwrap();
        let managed = dir.join("managed");
        std::fs::create_dir_all(&managed).unwrap();
        let pasted = managed.join("pasted.yaml");
        std::fs::write(&pasted, "clusters:\n- name: b\n  cluster: { server: https://b }\ncontexts:\n- name: ctx-b\n  context: { cluster: b, user: user-b }\n").unwrap();

        // Startup: default_kubeconfig_paths() included the managed file.
        let startup_paths = vec![base.clone(), pasted.clone()];
        let cache = ClientCache::new(base.clone());
        let mut reg = Registry::new();
        reg.register(list_contexts_capability(cache.clone(), startup_paths, Some(managed.clone())));

        // The user deletes it while the app runs.
        std::fs::remove_file(&pasted).unwrap();

        let out = reg.invoke("k8s.listContexts", json!({ "paths": [] })).await.unwrap();
        let names: Vec<String> = out["contexts"].as_array().unwrap().iter()
            .map(|c| c["name"].as_str().unwrap().to_string()).collect();
        assert_eq!(names, ["ctx-a"], "the deleted context must be gone");

        // The important half: default_paths is a startup SNAPSHOT that still
        // holds the deleted file, so without pruning it would be reintroduced
        // on every call and linger in the cache — where the strict merged
        // fallback in load_kubeconfigs fails on the missing file.
        let active = cache.paths().await;
        assert!(
            !active.contains(&pasted),
            "deleted kubeconfig still active: {active:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn discovery_self_heals_for_a_caller_that_sends_no_paths() {
        // An MCP client calls k8s.listContexts with {}. Reconciliation used to
        // run only when `paths` was present, so such a caller kept whatever
        // the startup cache seed held — including files since deleted.
        let dir = std::env::temp_dir().join(format!("srelens-nopaths-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let base = dir.join("base.yaml");
        std::fs::write(&base, "clusters:\n- name: a\n  cluster: { server: https://a }\ncontexts:\n- name: ctx-a\n  context: { cluster: a, user: user-a }\n").unwrap();
        let managed = dir.join("managed");
        std::fs::create_dir_all(&managed).unwrap();
        let seeded = managed.join("seeded.yaml");
        std::fs::write(&seeded, "clusters:\n- name: b\n  cluster: { server: https://b }\ncontexts:\n- name: ctx-b\n  context: { cluster: b, user: user-b }\n").unwrap();

        // Cache seeded at startup with the managed file, as the desktop does.
        let cache = ClientCache::new_many(vec![base.clone(), seeded.clone()]);
        let mut reg = Registry::new();
        reg.register(list_contexts_capability(
            cache.clone(),
            vec![base.clone()],
            Some(managed.clone()),
        ));

        // A NEW file appears, and the seeded one is deleted.
        std::fs::write(managed.join("added.yaml"), "clusters:\n- name: c\n  cluster: { server: https://c }\ncontexts:\n- name: ctx-c\n  context: { cluster: c, user: user-c }\n").unwrap();
        std::fs::remove_file(&seeded).unwrap();

        // Invoked with NO paths key at all.
        let out = reg.invoke("k8s.listContexts", json!({})).await.unwrap();
        let names: Vec<String> = out["contexts"].as_array().unwrap().iter()
            .map(|c| c["name"].as_str().unwrap().to_string()).collect();
        assert!(names.iter().any(|n| n == "ctx-c"), "new file not discovered: {names:?}");
        assert!(!names.iter().any(|n| n == "ctx-b"), "deleted file lingered: {names:?}");

        let active = cache.paths().await;
        assert!(!active.contains(&seeded), "deleted path still active: {active:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn merges_additional_kubeconfig_files() {
        let dir = std::env::temp_dir();
        let first = dir.join("srelens-contexts-first.yaml");
        let second = dir.join("srelens-contexts-second.yaml");
        tokio::fs::write(
            &first,
            "clusters:\n- name: a\n  cluster: { server: https://a }\ncontexts:\n- name: ctx-a\n  context: { cluster: a, user: user-a }\n",
        ).await.unwrap();
        tokio::fs::write(
            &second,
            "clusters:\n- name: b\n  cluster: { server: https://b }\ncontexts:\n- name: ctx-b\n  context: { cluster: b, user: user-b }\n",
        ).await.unwrap();

        let cache = ClientCache::new(first.clone());
        let mut reg = Registry::new();
        reg.register(list_contexts_capability(cache, vec![first.clone()], None));
        let out = reg.invoke(
            "k8s.listContexts",
            json!({ "paths": [second.to_string_lossy()] }),
        ).await.unwrap();
        assert_eq!(out["contexts"].as_array().unwrap().len(), 2);
        assert_eq!(out["contexts"][1]["name"], "ctx-b");

        let _ = tokio::fs::remove_file(first).await;
        let _ = tokio::fs::remove_file(second).await;
    }

    #[tokio::test]
    async fn deletes_a_context_from_kubeconfig_file() {
        let dir = std::env::temp_dir();
        let unique = format!(
            "srelens-test-delete-{}-{}.yaml",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        let path = dir.join(unique);
        tokio::fs::write(
            &path,
            "clusters:\n  - name: a\n    cluster: { server: https://a }\ncontexts:\n  - name: ctx-a\n    context: { cluster: a, user: user-a }\nusers:\n  - name: user-a\n    user: {}\n",
        )
        .await
        .unwrap();

        let cache = ClientCache::new(path.clone());
        let mut reg = Registry::new();
        reg.register(delete_context_capability(cache.clone()));
        reg.register(list_contexts_capability(cache.clone(), vec![path.clone()], None));

        // Delete the context
        let out = reg.invoke("k8s.deleteContext", json!({ "context": "ctx-a" })).await.unwrap();
        assert_eq!(out["success"], true);

        // List contexts to check that it is empty
        let out_list = reg.invoke("k8s.listContexts", json!({})).await.unwrap();
        assert_eq!(out_list["contexts"].as_array().unwrap().len(), 0);

        // Verify the file content on disk has no clusters, contexts, or users
        let file_content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(!file_content.contains("ctx-a"));
        assert!(!file_content.contains("https://a"));
        assert!(!file_content.contains("user-a"));

        let _ = tokio::fs::remove_file(&path).await;
    }
}
