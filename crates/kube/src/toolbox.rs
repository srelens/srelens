//! Toolbox diagnosis engine (pure).
//!
//! The top onboarding failure for exec-auth kubeconfigs is a missing tool:
//! `kubectl oidc-login` fails when `kubectl-oidc_login` isn't installed. This
//! module reads the exec-auth blocks of loaded kubeconfigs and turns each
//! context into the set of external binaries it depends on, classified by
//! whether srelens can install them (kubectl / krew plugins) or only report
//! them (cloud CLIs). Resolution of those requirements against the app's PATH
//! is a separate step; this half is pure string work and fully unit-tested.

use crate::connect::load_kubeconfigs;
use crate::helm_cli::resolve_on_path;
use crate::kubeconfig::KubeError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use srelens_capability::{Annotations, Capability, CapabilityError};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// What kind of tool a requirement is — decides whether srelens can fix it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequirementKind {
    /// The `kubectl` binary itself.
    Kubectl,
    /// A kubectl plugin installable via krew. `plugin` is the krew plugin name
    /// (dashes, e.g. `oidc-login`); the binary it installs is in
    /// [`Requirement::binary`] (`kubectl-oidc_login`).
    KrewPlugin { plugin: String },
    /// A tool srelens does not manage (cloud CLI, custom binary): detected and
    /// reported with a vendor link, never installed.
    External,
}

/// One external binary a context's exec-auth depends on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requirement {
    /// The binary to resolve on PATH (`kubectl`, `kubectl-oidc_login`, `aws`),
    /// or an absolute path when the exec `command` was written as one.
    pub binary: String,
    pub kind: RequirementKind,
}

/// The exec-auth tool requirements of a single kubeconfig context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextRequirements {
    pub context: String,
    /// Empty when the context's user has no exec block — nothing external is
    /// needed, which is a healthy state, not an error.
    pub requirements: Vec<Requirement>,
}

/// Parse a kubeconfig document into each context's exec-auth requirement set.
/// Contexts are returned in document order; a context whose user has no exec
/// block yields an empty requirement list.
pub fn context_requirements(yaml: &str) -> Result<Vec<ContextRequirements>, KubeError> {
    let raw: Raw = serde_yaml::from_str(yaml).map_err(|e| KubeError::Parse(e.to_string()))?;
    let exec_of = |user: &str| {
        raw.users
            .iter()
            .find(|u| u.name == user)
            .and_then(|u| u.user.exec.as_ref())
    };
    Ok(raw
        .contexts
        .iter()
        .map(|c| ContextRequirements {
            context: c.name.clone(),
            requirements: exec_of(&c.context.user)
                .map(|e| requirements_for_exec(&e.command, &e.args))
                .unwrap_or_default(),
        })
        .collect())
}

/// Turn one exec block (`command` + `args`) into the binaries it needs.
fn requirements_for_exec(command: &str, args: &[String]) -> Vec<Requirement> {
    let (command, args) = strip_env_wrapper(command, args);
    let is_path = command.contains('/');
    let base = command.rsplit('/').next().unwrap_or(command);

    // A path or a non-kubectl binary is a single external tool checked as
    // written; kubectl deployments deliberately name it in full.
    if base != "kubectl" {
        return vec![Requirement {
            binary: if is_path { command.to_string() } else { base.to_string() },
            kind: RequirementKind::External,
        }];
    }

    // `kubectl` (or `/path/to/kubectl`) always needs kubectl itself.
    let mut reqs = vec![Requirement {
        binary: if is_path { command.to_string() } else { "kubectl".to_string() },
        kind: RequirementKind::Kubectl,
    }];
    // The first non-flag argument is the plugin invocation, e.g. `oidc-login`.
    // kubectl resolves `kubectl <plugin>` to the binary `kubectl-<plugin>` with
    // dashes rewritten to underscores; krew installs it under the dashed name.
    if let Some(plugin) = args.iter().find(|a| !a.starts_with('-')) {
        reqs.push(Requirement {
            binary: format!("kubectl-{}", plugin.replace('-', "_")),
            kind: RequirementKind::KrewPlugin { plugin: plugin.clone() },
        });
    }
    reqs
}

/// `command: env, args: [FOO=bar, aws, ...]` is a wrapper — unwrap to the real
/// command and its arguments. Leading `NAME=VALUE` tokens are the injected
/// environment and are skipped.
fn strip_env_wrapper<'a>(command: &'a str, args: &'a [String]) -> (&'a str, &'a [String]) {
    let base = command.rsplit('/').next().unwrap_or(command);
    if base != "env" {
        return (command, args);
    }
    let real = args.iter().position(|a| !a.contains('='));
    match real {
        Some(i) => (args[i].as_str(), &args[i + 1..]),
        None => (command, args),
    }
}

#[derive(Deserialize)]
struct Raw {
    #[serde(default)]
    contexts: Vec<RawContext>,
    #[serde(default)]
    users: Vec<RawUser>,
}
#[derive(Deserialize)]
struct RawContext {
    name: String,
    #[serde(default)]
    context: RawContextData,
}
#[derive(Deserialize, Default)]
struct RawContextData {
    #[serde(default)]
    user: String,
}
#[derive(Deserialize)]
struct RawUser {
    name: String,
    #[serde(default)]
    user: RawUserData,
}
#[derive(Deserialize, Default)]
struct RawUserData {
    #[serde(default)]
    exec: Option<RawExec>,
}
#[derive(Deserialize)]
struct RawExec {
    #[serde(default)]
    command: String,
    #[serde(default)]
    args: Vec<String>,
}

/// The directories the resolver searches, as PATH-style strings (matching the
/// rest of the crate). A hit in `app_path` is usable now; a hit only in
/// `system_path` is present-but-not-visible-to-the-app.
pub struct SearchPaths {
    /// The app's effective PATH (post `fix-path-env`) plus `~/.srelens/bin` and
    /// `~/.krew/bin`.
    pub app_path: String,
    /// Broader locations a tool might live in that the app doesn't search.
    pub system_path: String,
}

/// Whether a requirement is satisfied, and if so from where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// On the app's effective PATH (or an absolute exec path that exists) —
    /// usable now. `version` is populated for kubectl.
    Found { path: String, version: Option<String> },
    /// Present on the system but off the app's PATH — needs a PATH fix, not an
    /// install.
    NotOnAppPath { path: String },
    /// Not found anywhere searched.
    Missing,
}

/// A requirement paired with where (if anywhere) it resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRequirement {
    pub requirement: Requirement,
    pub resolution: Resolution,
}

/// A context's exec-auth requirements, each resolved against the search paths —
/// the single type that drives the health UI, the error deep-link, and the
/// `toolbox.diagnoseContext` capability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosisReport {
    pub context: String,
    pub items: Vec<ResolvedRequirement>,
}

/// Locate `binary` against the search paths. A bare name is searched in the app
/// path first (usable), then the system path (present-but-hidden). A command
/// written as a path is exec'd directly by client-go, so PATH is irrelevant: it
/// resolves iff the file exists where written.
pub fn locate(
    binary: &str,
    paths: &SearchPaths,
    is_file: &impl Fn(&Path) -> bool,
) -> Option<Located> {
    // A command written as a path is exec'd directly by client-go; PATH is
    // irrelevant, so it's usable iff the file exists exactly where written.
    if binary.contains('/') {
        return is_file(Path::new(binary))
            .then(|| Located { path: binary.to_string(), on_app_path: true });
    }
    if let Some(p) = resolve_on_path(binary, &paths.app_path, is_file) {
        return Some(Located { path: p.to_string_lossy().into_owned(), on_app_path: true });
    }
    resolve_on_path(binary, &paths.system_path, is_file)
        .map(|p| Located { path: p.to_string_lossy().into_owned(), on_app_path: false })
}

/// Where a requirement was found and whether the app can use it as-is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    pub path: String,
    pub on_app_path: bool,
}

/// Resolve every requirement of a context into a [`DiagnosisReport`].
/// `kubectl_version` is injected (it runs a subprocess in production) and is
/// consulted only for a found kubectl binary.
pub fn diagnose(
    ctx: &ContextRequirements,
    paths: &SearchPaths,
    is_file: &impl Fn(&Path) -> bool,
    kubectl_version: &impl Fn(&Path) -> Option<String>,
) -> DiagnosisReport {
    let items = ctx
        .requirements
        .iter()
        .map(|req| {
            let resolution = match locate(&req.binary, paths, is_file) {
                None => Resolution::Missing,
                Some(l) if !l.on_app_path => Resolution::NotOnAppPath { path: l.path },
                Some(l) => {
                    // Only kubectl reports a client version.
                    let version = matches!(req.kind, RequirementKind::Kubectl)
                        .then(|| kubectl_version(Path::new(&l.path)))
                        .flatten();
                    Resolution::Found { path: l.path, version }
                }
            };
            ResolvedRequirement { requirement: req.clone(), resolution }
        })
        .collect();
    DiagnosisReport { context: ctx.context.clone(), items }
}

impl SearchPaths {
    /// Build from the process environment: the app PATH (already resolved by
    /// `fix-path-env` at startup) with the managed dirs `~/.srelens/bin` and
    /// `~/.krew/bin` prepended, plus common system locations that back the
    /// "present but not on the app PATH" check.
    pub fn from_env() -> SearchPaths {
        let home = std::env::var("HOME").unwrap_or_default();
        let path = std::env::var_os("PATH").unwrap_or_default();
        let managed = [
            PathBuf::from(&home).join(".srelens/bin"),
            PathBuf::from(&home).join(".krew/bin"),
        ];
        let app_dirs = managed.iter().cloned().chain(std::env::split_paths(&path));
        let system_dirs = ["/usr/local/bin", "/opt/homebrew/bin", "/usr/bin", "/bin"]
            .iter()
            .map(PathBuf::from)
            .chain(std::iter::once(PathBuf::from(&home).join(".local/bin")));
        SearchPaths {
            app_path: join_paths(app_dirs),
            system_path: join_paths(system_dirs),
        }
    }
}

fn join_paths(dirs: impl IntoIterator<Item = PathBuf>) -> String {
    std::env::join_paths(dirs)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DiagnoseContextIn {
    /// The kube context to diagnose.
    pub context: String,
}

/// One requirement's status, flattened for the UI and MCP.
#[derive(Debug, Serialize, JsonSchema)]
pub struct RequirementStatusDto {
    pub binary: String,
    /// `kubectl` | `krew-plugin` | `external`.
    pub kind: String,
    /// The krew plugin name when `kind == krew-plugin`.
    pub plugin: Option<String>,
    /// Whether srelens can install it (kubectl or a krew plugin).
    pub installable: bool,
    /// `found` | `not-on-app-path` | `missing`.
    pub status: String,
    pub path: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DiagnoseContextOut {
    pub context: String,
    /// Empty when the context needs no external tools (healthy).
    pub items: Vec<RequirementStatusDto>,
}

fn kind_fields(kind: &RequirementKind) -> (&'static str, Option<String>, bool) {
    match kind {
        RequirementKind::Kubectl => ("kubectl", None, true),
        RequirementKind::KrewPlugin { plugin } => ("krew-plugin", Some(plugin.clone()), true),
        RequirementKind::External => ("external", None, false),
    }
}

fn status_fields(res: Resolution) -> (&'static str, Option<String>, Option<String>) {
    match res {
        Resolution::Found { path, version } => ("found", Some(path), version),
        Resolution::NotOnAppPath { path } => ("not-on-app-path", Some(path), None),
        Resolution::Missing => ("missing", None, None),
    }
}

/// Read-only capability: diagnose one context's exec-auth tool requirements.
/// The resolution environment (`search`, `is_file`) is injected so it's
/// deterministic under test; production supplies [`SearchPaths::from_env`] and a
/// real filesystem check.
pub fn diagnose_context_capability(
    kubeconfig_paths: Vec<PathBuf>,
    search: SearchPaths,
    is_file: impl Fn(&Path) -> bool + Send + Sync + 'static,
) -> Capability {
    let search = Arc::new(search);
    let is_file = Arc::new(is_file);
    Capability::typed::<DiagnoseContextIn, DiagnoseContextOut, _, _>(
        "toolbox.diagnoseContext",
        "diagnose a kube context's exec-auth tool requirements: which external \
         tools it needs and whether each is installed, off the app PATH, or missing",
        Annotations::READ_ONLY,
        move |input: DiagnoseContextIn| {
            let paths = kubeconfig_paths.clone();
            let search = search.clone();
            let is_file = is_file.clone();
            async move {
                let merged = load_kubeconfigs(&paths).map_err(CapabilityError::Handler)?;
                let yaml = serde_yaml::to_string(&merged)
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                let all = context_requirements(&yaml)
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                let ctx = all
                    .into_iter()
                    .find(|c| c.context == input.context)
                    .ok_or_else(|| {
                        CapabilityError::InvalidInput(format!("unknown context: {}", input.context))
                    })?;
                // Version probing (a subprocess) lands with the install
                // capabilities; None for now keeps this read pure.
                let report = diagnose(&ctx, &search, &|p| is_file(p), &|_p| None);
                let items = report
                    .items
                    .into_iter()
                    .map(|item| {
                        let (kind, plugin, installable) = kind_fields(&item.requirement.kind);
                        let (status, path, version) = status_fields(item.resolution);
                        RequirementStatusDto {
                            binary: item.requirement.binary,
                            kind: kind.to_string(),
                            plugin,
                            installable,
                            status: status.to_string(),
                            path,
                            version,
                        }
                    })
                    .collect();
                Ok(DiagnoseContextOut { context: report.context, items })
            }
        },
    )
}

use crate::toolbox_install::{
    helm_install, install_binary, install_from_targz, kubectl_install, parse_github_latest_tag,
    InstallError, Platform, HELM_LATEST_RELEASE_URL, KUBECTL_STABLE_URL,
};

/// The directory srelens installs managed tools into: `~/.srelens/bin`.
pub fn srelens_bin_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(".srelens").join("bin")
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct InstallKubectlIn {}

#[derive(Debug, Serialize, JsonSchema)]
pub struct InstallToolOut {
    pub tool: String,
    pub version: String,
    pub path: String,
}

fn to_handler(e: InstallError) -> CapabilityError {
    CapabilityError::Handler(e.to_string())
}

/// Confirm-gated capability: download the latest stable kubectl into
/// `~/.srelens/bin`, verified against dl.k8s.io's published checksum. `fetch`
/// (a blocking HTTP GET) is injected so the capability is testable without a
/// real network; production supplies a reqwest-backed one at registration.
pub fn install_kubectl_capability<F>(install_dir: PathBuf, fetch: F) -> Capability
where
    F: Fn(&str) -> Result<Vec<u8>, InstallError> + Send + Sync + Clone + 'static,
{
    Capability::typed::<InstallKubectlIn, InstallToolOut, _, _>(
        "toolbox.installKubectl",
        "download the latest stable kubectl into ~/.srelens/bin, verified against \
         the dl.k8s.io checksum",
        Annotations::MUTATING,
        move |_input: InstallKubectlIn| {
            let install_dir = install_dir.clone();
            let fetch = fetch.clone();
            async move {
                // The install does blocking HTTP + filesystem work; keep it off
                // the async runtime.
                tokio::task::spawn_blocking(move || {
                    let platform = Platform::current().map_err(to_handler)?;
                    let raw = fetch(KUBECTL_STABLE_URL).map_err(to_handler)?;
                    let version = std::str::from_utf8(&raw)
                        .map_err(|e| CapabilityError::Handler(e.to_string()))?
                        .trim()
                        .to_string();
                    let plan = kubectl_install(&version, &platform, &install_dir);
                    let path = install_binary(&plan, &fetch).map_err(to_handler)?;
                    Ok(InstallToolOut {
                        tool: "kubectl".to_string(),
                        version,
                        path: path.to_string_lossy().into_owned(),
                    })
                })
                .await
                .map_err(|e| CapabilityError::Handler(e.to_string()))?
            }
        },
    )
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct InstallHelmIn {}

/// Confirm-gated capability: download the latest helm release into
/// `~/.srelens/bin`, verified against helm's published checksum. Version is
/// resolved from GitHub's latest-release API (helm has no `stable.txt`).
pub fn install_helm_capability<F>(install_dir: PathBuf, fetch: F) -> Capability
where
    F: Fn(&str) -> Result<Vec<u8>, InstallError> + Send + Sync + Clone + 'static,
{
    Capability::typed::<InstallHelmIn, InstallToolOut, _, _>(
        "toolbox.installHelm",
        "download the latest helm release into ~/.srelens/bin, verified against \
         its published checksum",
        Annotations::MUTATING,
        move |_input: InstallHelmIn| {
            let install_dir = install_dir.clone();
            let fetch = fetch.clone();
            async move {
                tokio::task::spawn_blocking(move || {
                    let platform = Platform::current().map_err(to_handler)?;
                    let body = fetch(HELM_LATEST_RELEASE_URL).map_err(to_handler)?;
                    let version = parse_github_latest_tag(&body).map_err(to_handler)?;
                    let plan = helm_install(&version, &platform, &install_dir);
                    let path = install_from_targz(&plan, &fetch).map_err(to_handler)?;
                    Ok(InstallToolOut {
                        tool: "helm".to_string(),
                        version,
                        path: path.to_string_lossy().into_owned(),
                    })
                })
                .await
                .map_err(|e| CapabilityError::Handler(e.to_string()))?
            }
        },
    )
}

#[cfg(test)]
mod capability_tests {
    use super::*;
    use serde_json::json;
    use srelens_capability::Registry;

    /// Write a kubeconfig with an oidc-login exec context to a temp file.
    fn kubeconfig_with_oidc(dir: &Path) -> PathBuf {
        let path = dir.join("config");
        std::fs::write(
            &path,
            r#"
apiVersion: v1
kind: Config
clusters:
  - name: c
    cluster: { server: https://x }
contexts:
  - name: dev
    context: { cluster: c, user: oidc }
users:
  - name: oidc
    user:
      exec:
        apiVersion: client.authentication.k8s.io/v1beta1
        command: kubectl
        args: ["oidc-login", "get-token"]
"#,
        )
        .unwrap();
        path
    }

    #[tokio::test]
    async fn diagnoses_a_context_reporting_found_kubectl_and_missing_plugin() {
        let dir = tempfile::tempdir().unwrap();
        let kubeconfig = kubeconfig_with_oidc(dir.path());
        // A bin dir holding kubectl but not the plugin.
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("kubectl"), b"#!/bin/sh\n").unwrap();
        let search = SearchPaths {
            app_path: bin.to_string_lossy().into_owned(),
            system_path: String::new(),
        };

        let mut reg = Registry::new();
        reg.register(diagnose_context_capability(vec![kubeconfig], search, |p| p.is_file()));
        let out = reg
            .invoke("toolbox.diagnoseContext", json!({ "context": "dev" }))
            .await
            .unwrap();

        assert_eq!(out["context"], "dev");
        let items = out["items"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["kind"], "kubectl");
        assert_eq!(items[0]["status"], "found");
        assert!(items[0]["path"].as_str().unwrap().ends_with("/kubectl"));
        assert_eq!(items[1]["kind"], "krew-plugin");
        assert_eq!(items[1]["plugin"], "oidc-login");
        assert_eq!(items[1]["status"], "missing");
        assert_eq!(items[1]["installable"], true);
    }

    #[tokio::test]
    async fn an_unknown_context_is_an_input_error() {
        let dir = tempfile::tempdir().unwrap();
        let kubeconfig = kubeconfig_with_oidc(dir.path());
        let mut reg = Registry::new();
        reg.register(diagnose_context_capability(
            vec![kubeconfig],
            SearchPaths { app_path: String::new(), system_path: String::new() },
            |p| p.is_file(),
        ));
        let err = reg
            .invoke("toolbox.diagnoseContext", json!({ "context": "nope" }))
            .await
            .unwrap_err();
        assert!(format!("{err:?}").contains("unknown context"));
    }

    use std::collections::HashMap;

    /// A fake blocking HTTP: URL -> bytes. Clone/Send/Sync so it satisfies the
    /// capability's `fetch` bound.
    fn net(entries: Vec<(String, Vec<u8>)>) -> impl Fn(&str) -> Result<Vec<u8>, InstallError> + Clone {
        let map: HashMap<String, Vec<u8>> = entries.into_iter().collect();
        move |url: &str| {
            map.get(url).cloned().ok_or_else(|| InstallError::Download(format!("404 {url}")))
        }
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        hex::encode(Sha256::digest(bytes))
    }

    #[tokio::test]
    async fn install_kubectl_resolves_stable_downloads_and_verifies() {
        let dir = tempfile::tempdir().unwrap();
        let install_dir = dir.path().join("bin");
        let platform = Platform::current().unwrap();
        let plan = kubectl_install("v9.9.9", &platform, &install_dir);
        let payload = b"#!/bin/sh\necho kubectl\n";
        let fetch = net(vec![
            (KUBECTL_STABLE_URL.to_string(), b"v9.9.9\n".to_vec()),
            (plan.sha256_url.clone(), sha256_hex(payload).into_bytes()),
            (plan.binary_url.clone(), payload.to_vec()),
        ]);

        let mut reg = Registry::new();
        reg.register(install_kubectl_capability(install_dir, fetch));
        let out = reg.invoke("toolbox.installKubectl", json!({})).await.unwrap();

        assert_eq!(out["tool"], "kubectl");
        assert_eq!(out["version"], "v9.9.9");
        let installed = out["path"].as_str().unwrap();
        assert_eq!(std::fs::read(installed).unwrap(), payload);
    }

    #[tokio::test]
    async fn install_kubectl_is_confirm_gated() {
        let cap = install_kubectl_capability(PathBuf::from("/tmp/x"), net(vec![]));
        assert!(cap.annotations.requires_confirm, "installs must require consent");
        assert!(!cap.annotations.read_only);
    }

    fn make_targz(members: &[(&str, &[u8])]) -> Vec<u8> {
        use flate2::{write::GzEncoder, Compression};
        let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::fast()));
        for (name, bytes) in members {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, name, *bytes).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    #[tokio::test]
    async fn install_helm_resolves_latest_downloads_and_extracts() {
        let dir = tempfile::tempdir().unwrap();
        let install_dir = dir.path().join("bin");
        let platform = Platform::current().unwrap();
        let plan = helm_install("v9.9.9", &platform, &install_dir);
        let payload = b"#!/bin/sh\necho helm\n";
        let archive = make_targz(&[(plan.member.as_str(), payload)]);
        let fetch = net(vec![
            (HELM_LATEST_RELEASE_URL.to_string(), br#"{"tag_name":"v9.9.9"}"#.to_vec()),
            (plan.sha256_url.clone(), sha256_hex(&archive).into_bytes()),
            (plan.archive_url.clone(), archive),
        ]);

        let mut reg = Registry::new();
        reg.register(install_helm_capability(install_dir, fetch));
        let out = reg.invoke("toolbox.installHelm", json!({})).await.unwrap();

        assert_eq!(out["tool"], "helm");
        assert_eq!(out["version"], "v9.9.9");
        assert_eq!(std::fs::read(out["path"].as_str().unwrap()).unwrap(), payload);
    }

    #[tokio::test]
    async fn install_helm_is_confirm_gated() {
        let cap = install_helm_capability(PathBuf::from("/tmp/x"), net(vec![]));
        assert!(cap.annotations.requires_confirm);
    }

    #[tokio::test]
    async fn install_kubectl_surfaces_a_checksum_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let install_dir = dir.path().join("bin");
        let platform = Platform::current().unwrap();
        let plan = kubectl_install("v9.9.9", &platform, &install_dir);
        let fetch = net(vec![
            (KUBECTL_STABLE_URL.to_string(), b"v9.9.9".to_vec()),
            (plan.sha256_url.clone(), sha256_hex(b"expected").into_bytes()),
            (plan.binary_url.clone(), b"tampered".to_vec()),
        ]);
        let mut reg = Registry::new();
        reg.register(install_kubectl_capability(install_dir.clone(), fetch));
        let err = reg.invoke("toolbox.installKubectl", json!({})).await.unwrap_err();
        assert!(format!("{err:?}").to_lowercase().contains("checksum"));
        assert!(!plan.target.exists());
    }
}

#[cfg(test)]
mod resolution_tests {
    use super::*;

    /// A fake filesystem: the given paths exist and are executable.
    fn fs(existing: &[&str]) -> impl Fn(&Path) -> bool {
        let owned: Vec<std::path::PathBuf> = existing.iter().map(std::path::PathBuf::from).collect();
        move |p: &Path| owned.iter().any(|e| e == p)
    }

    fn paths() -> SearchPaths {
        SearchPaths { app_path: "/app/bin".into(), system_path: "/usr/bin".into() }
    }

    fn kubectl_req() -> Requirement {
        Requirement { binary: "kubectl".into(), kind: RequirementKind::Kubectl }
    }

    #[test]
    fn a_binary_on_the_app_path_is_found() {
        assert_eq!(
            locate("kubectl", &paths(), &fs(&["/app/bin/kubectl"])),
            Some(Located { path: "/app/bin/kubectl".into(), on_app_path: true }),
        );
    }

    #[test]
    fn a_binary_only_on_the_system_path_is_not_on_app_path() {
        assert_eq!(
            locate("kubectl", &paths(), &fs(&["/usr/bin/kubectl"])),
            Some(Located { path: "/usr/bin/kubectl".into(), on_app_path: false }),
        );
    }

    #[test]
    fn a_binary_found_nowhere_is_none() {
        assert_eq!(locate("kubectl", &paths(), &fs(&[])), None);
    }

    #[test]
    fn an_absolute_command_resolves_at_its_written_path_and_is_usable() {
        let p = "/opt/sdk/gke-gcloud-auth-plugin";
        assert_eq!(
            locate(p, &paths(), &fs(&[p])),
            Some(Located { path: p.into(), on_app_path: true }),
        );
        assert_eq!(locate(p, &paths(), &fs(&[])), None);
    }

    #[test]
    fn diagnose_reports_found_kubectl_with_version_and_missing_plugin_in_order() {
        let ctx = ContextRequirements {
            context: "dev".into(),
            requirements: vec![
                kubectl_req(),
                Requirement {
                    binary: "kubectl-oidc_login".into(),
                    kind: RequirementKind::KrewPlugin { plugin: "oidc-login".into() },
                },
            ],
        };
        let report = diagnose(
            &ctx,
            &paths(),
            &fs(&["/app/bin/kubectl"]),
            &|_p| Some("v1.30.2".into()),
        );
        assert_eq!(
            report,
            DiagnosisReport {
                context: "dev".into(),
                items: vec![
                    ResolvedRequirement {
                        requirement: kubectl_req(),
                        resolution: Resolution::Found {
                            path: "/app/bin/kubectl".into(),
                            version: Some("v1.30.2".into()),
                        },
                    },
                    ResolvedRequirement {
                        requirement: Requirement {
                            binary: "kubectl-oidc_login".into(),
                            kind: RequirementKind::KrewPlugin { plugin: "oidc-login".into() },
                        },
                        resolution: Resolution::Missing,
                    },
                ],
            },
        );
    }

    #[test]
    fn version_is_only_probed_for_kubectl_not_other_found_tools() {
        let ctx = ContextRequirements {
            context: "eks".into(),
            requirements: vec![Requirement {
                binary: "aws".into(),
                kind: RequirementKind::External,
            }],
        };
        // The version probe would panic if called for a non-kubectl tool.
        let report = diagnose(&ctx, &paths(), &fs(&["/app/bin/aws"]), &|_p| {
            panic!("kubectl_version must not be called for external tools")
        });
        assert_eq!(
            report.items[0].resolution,
            Resolution::Found { path: "/app/bin/aws".into(), version: None },
        );
    }

    #[test]
    fn a_tool_off_the_app_path_reports_not_on_app_path() {
        let ctx = ContextRequirements {
            context: "eks".into(),
            requirements: vec![Requirement {
                binary: "aws".into(),
                kind: RequirementKind::External,
            }],
        };
        let report = diagnose(&ctx, &paths(), &fs(&["/usr/bin/aws"]), &|_p| None);
        assert_eq!(
            report.items[0].resolution,
            Resolution::NotOnAppPath { path: "/usr/bin/aws".into() },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reqs(yaml: &str) -> Vec<ContextRequirements> {
        context_requirements(yaml).expect("parse")
    }

    const OIDC: &str = r#"
apiVersion: v1
kind: Config
contexts:
  - name: dev
    context:
      cluster: c
      user: oidc
users:
  - name: oidc
    user:
      exec:
        command: kubectl
        args: ["oidc-login", "get-token", "--oidc-issuer-url=https://x"]
"#;

    #[test]
    fn kubectl_plugin_needs_kubectl_and_the_krew_binary() {
        assert_eq!(
            reqs(OIDC),
            vec![ContextRequirements {
                context: "dev".into(),
                requirements: vec![
                    Requirement { binary: "kubectl".into(), kind: RequirementKind::Kubectl },
                    Requirement {
                        binary: "kubectl-oidc_login".into(),
                        kind: RequirementKind::KrewPlugin { plugin: "oidc-login".into() },
                    },
                ],
            }],
        );
    }

    #[test]
    fn a_context_with_no_exec_block_has_no_requirements() {
        let yaml = r#"
contexts:
  - name: plain
    context: { cluster: c, user: static }
users:
  - name: static
    user:
      token: abc
"#;
        assert_eq!(reqs(yaml)[0].requirements, vec![]);
    }

    #[test]
    fn a_cloud_cli_exec_is_external_not_installable() {
        let yaml = r#"
contexts:
  - name: eks
    context: { cluster: c, user: aws }
users:
  - name: aws
    user:
      exec:
        command: aws
        args: ["eks", "get-token", "--cluster-name", "prod"]
"#;
        assert_eq!(
            reqs(yaml)[0].requirements,
            vec![Requirement { binary: "aws".into(), kind: RequirementKind::External }],
        );
    }

    #[test]
    fn an_env_prefixed_command_resolves_to_the_real_binary() {
        let yaml = r#"
contexts:
  - name: eks
    context: { cluster: c, user: aws }
users:
  - name: aws
    user:
      exec:
        command: env
        args: ["AWS_PROFILE=prod", "aws", "eks", "get-token"]
"#;
        assert_eq!(
            reqs(yaml)[0].requirements,
            vec![Requirement { binary: "aws".into(), kind: RequirementKind::External }],
        );
    }

    #[test]
    fn an_absolute_path_command_is_checked_as_written() {
        let yaml = r#"
contexts:
  - name: gke
    context: { cluster: c, user: g }
users:
  - name: g
    user:
      exec:
        command: /opt/google-cloud-sdk/bin/gke-gcloud-auth-plugin
"#;
        assert_eq!(
            reqs(yaml)[0].requirements,
            vec![Requirement {
                binary: "/opt/google-cloud-sdk/bin/gke-gcloud-auth-plugin".into(),
                kind: RequirementKind::External,
            }],
        );
    }

    #[test]
    fn kubectl_with_only_flags_needs_only_kubectl() {
        let yaml = r#"
contexts:
  - name: k
    context: { cluster: c, user: u }
users:
  - name: u
    user:
      exec:
        command: kubectl
        args: ["--kubeconfig=/x"]
"#;
        assert_eq!(
            reqs(yaml)[0].requirements,
            vec![Requirement { binary: "kubectl".into(), kind: RequirementKind::Kubectl }],
        );
    }

    #[test]
    fn contexts_are_returned_in_document_order() {
        let yaml = r#"
contexts:
  - name: b
    context: { cluster: c, user: aws }
  - name: a
    context: { cluster: c, user: aws }
users:
  - name: aws
    user:
      exec: { command: aws, args: ["eks", "get-token"] }
"#;
        let names: Vec<_> = reqs(yaml).into_iter().map(|c| c.context).collect();
        assert_eq!(names, vec!["b", "a"]);
    }
}
