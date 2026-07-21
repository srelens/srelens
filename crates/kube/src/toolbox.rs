//! Toolbox diagnosis engine (pure).
//!
//! The top onboarding failure for exec-auth kubeconfigs is a missing tool:
//! `kubectl oidc-login` fails when `kubectl-oidc_login` isn't installed. This
//! module reads the exec-auth blocks of loaded kubeconfigs and turns each
//! context into the set of external binaries it depends on, classified by
//! whether srelens can install them (kubectl / krew plugins) or only report
//! them (cloud CLIs). Resolution of those requirements against the app's PATH
//! is a separate step; this half is pure string work and fully unit-tested.

use crate::context_resolve::resolve_context;
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
                // Resolve the (possibly disambiguated) display name to its owning
                // file so a duplicate-named context diagnoses against its own
                // kubeconfig rather than the first-merged one.
                let resolved = resolve_context(&paths, &input.context).ok_or_else(|| {
                    CapabilityError::InvalidInput(format!("unknown context: {}", input.context))
                })?;
                let config = kube::config::Kubeconfig::read_from(&resolved.source)
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                let yaml = serde_yaml::to_string(&config)
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                let all = context_requirements(&yaml)
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                let ctx = all
                    .into_iter()
                    .find(|c| c.context == resolved.original_name)
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
    helm_install, install_binary, install_from_targz, install_krew, krew_archive, kubectl_install,
    parse_github_latest_tag, InstallError, Platform, HELM_LATEST_RELEASE_URL,
    KREW_LATEST_RELEASE_URL, KUBECTL_STABLE_URL,
};

/// The directory srelens installs managed tools into: `~/.srelens/bin`.
pub fn srelens_bin_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(".srelens").join("bin")
}

/// Where krew installs its shim (`kubectl-krew`) and plugin binaries.
pub fn krew_bin_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(".krew").join("bin")
}

/// The tools srelens can manage, in display order.
const MANAGED_TOOLS: [&str; 3] = ["kubectl", "krew", "helm"];

/// One managed tool's inventory entry for the Toolbox "Tools" section.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ToolStatusDto {
    pub name: String,
    pub installed: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    /// `managed` (srelens installed it under a managed dir) or `system`.
    pub source: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct StatusOut {
    pub tools: Vec<ToolStatusDto>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct StatusIn {}

/// Extract the first `vMAJOR.MINOR.PATCH` token from a tool's version output.
/// kubectl/krew/helm each wrap the version in different surrounding text (plain
/// "Client Version: v1.30.2", JSON `"gitVersion":"v1.30.2"`, or "v3.16.2+g…"),
/// so we scan for the first well-formed semver rather than parse each format.
pub fn first_semver(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] != b'v' || i + 1 >= bytes.len() || !bytes[i + 1].is_ascii_digit() {
            continue;
        }
        let start = i + 1;
        let mut j = start;
        while j < bytes.len() && (bytes[j].is_ascii_digit() || bytes[j] == b'.') {
            j += 1;
        }
        let parts: Vec<&str> = text[start..j].split('.').collect();
        if parts.len() >= 3 && parts[..3].iter().all(|p| !p.is_empty()) {
            return Some(format!("v{}.{}.{}", parts[0], parts[1], parts[2]));
        }
    }
    None
}

/// `managed` when the resolved binary lives under one of the srelens-managed
/// dirs (`~/.srelens/bin`, `~/.krew/bin`), else `system`.
fn tool_source(path: &Path, managed_dirs: &[PathBuf]) -> &'static str {
    if managed_dirs.iter().any(|dir| path.starts_with(dir)) {
        "managed"
    } else {
        "system"
    }
}

/// Read-only capability: inventory the managed CLI toolchain (kubectl, krew,
/// helm) — whether each is installed, where, its version, and whether srelens
/// manages it. The resolution environment is injected so it's deterministic
/// under test; production supplies [`SearchPaths::from_env`], a real filesystem
/// check, and a per-tool version probe.
pub fn status_capability(
    search: SearchPaths,
    managed_dirs: Vec<PathBuf>,
    is_file: impl Fn(&Path) -> bool + Send + Sync + 'static,
    version_of: impl Fn(&str, &Path) -> Option<String> + Send + Sync + 'static,
) -> Capability {
    let search = Arc::new(search);
    let managed_dirs = Arc::new(managed_dirs);
    let is_file = Arc::new(is_file);
    let version_of = Arc::new(version_of);
    Capability::typed::<StatusIn, StatusOut, _, _>(
        "toolbox.status",
        "inventory the managed CLI toolchain (kubectl, krew, helm): whether each \
         is installed, its path and version, and whether srelens manages it",
        Annotations::READ_ONLY,
        move |_input: StatusIn| {
            let search = search.clone();
            let managed_dirs = managed_dirs.clone();
            let is_file = is_file.clone();
            let version_of = version_of.clone();
            async move {
                let tools = MANAGED_TOOLS
                    .iter()
                    .map(|&name| match locate(name, &search, &|p| is_file(p)) {
                        Some(found) => {
                            let path = Path::new(&found.path);
                            ToolStatusDto {
                                name: name.to_string(),
                                installed: true,
                                version: version_of(name, path),
                                source: Some(tool_source(path, &managed_dirs).to_string()),
                                path: Some(found.path),
                            }
                        }
                        None => ToolStatusDto {
                            name: name.to_string(),
                            installed: false,
                            path: None,
                            version: None,
                            source: None,
                        },
                    })
                    .collect();
                Ok(StatusOut { tools })
            }
        },
    )
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

/// Core kubectl install, shared by the `toolbox.installKubectl` capability and
/// the streaming Tauri command: resolve the latest stable version, download and
/// verify, install into `install_dir`. `fetch` is injected — a plain blocking
/// GET for the capability, or a progress-emitting one for the streaming command.
pub fn run_kubectl_install<F>(install_dir: &Path, fetch: &F) -> Result<InstallToolOut, InstallError>
where
    F: Fn(&str) -> Result<Vec<u8>, InstallError>,
{
    let platform = Platform::current()?;
    let raw = fetch(KUBECTL_STABLE_URL)?;
    let version = std::str::from_utf8(&raw)
        .map_err(|_| InstallError::Download("kubectl version response was not UTF-8".to_string()))?
        .trim()
        .to_string();
    let plan = kubectl_install(&version, &platform, install_dir);
    let path = install_binary(&plan, fetch)?;
    Ok(InstallToolOut { tool: "kubectl".to_string(), version, path: path.to_string_lossy().into_owned() })
}

/// Core helm install, shared by the capability and the streaming command.
pub fn run_helm_install<F>(install_dir: &Path, fetch: &F) -> Result<InstallToolOut, InstallError>
where
    F: Fn(&str) -> Result<Vec<u8>, InstallError>,
{
    let platform = Platform::current()?;
    let body = fetch(HELM_LATEST_RELEASE_URL)?;
    let version = parse_github_latest_tag(&body)?;
    let plan = helm_install(&version, &platform, install_dir);
    let path = install_from_targz(&plan, fetch)?;
    Ok(InstallToolOut { tool: "helm".to_string(), version, path: path.to_string_lossy().into_owned() })
}

/// Core krew install (download + verify + bootstrap), shared by the capability
/// and the streaming command.
pub fn run_krew_install<F, R>(
    staging_dir: &Path,
    fetch: &F,
    run: &R,
) -> Result<InstallToolOut, InstallError>
where
    F: Fn(&str) -> Result<Vec<u8>, InstallError>,
    R: Fn(&Path, &[&str]) -> Result<(), InstallError>,
{
    let platform = Platform::current()?;
    let body = fetch(KREW_LATEST_RELEASE_URL)?;
    let version = parse_github_latest_tag(&body)?;
    let plan = krew_archive(&version, &platform, staging_dir);
    install_krew(&plan, fetch, run)?;
    Ok(InstallToolOut {
        tool: "krew".to_string(),
        version,
        path: krew_bin_dir().join("kubectl-krew").to_string_lossy().into_owned(),
    })
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
                    run_kubectl_install(&install_dir, &fetch).map_err(to_handler)
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
                    run_helm_install(&install_dir, &fetch).map_err(to_handler)
                })
                .await
                .map_err(|e| CapabilityError::Handler(e.to_string()))?
            }
        },
    )
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct InstallKrewIn {}

/// Confirm-gated capability: download the latest krew, verify it, and run its
/// self-bootstrap (`krew install krew`) to populate `~/.krew`. `fetch` (blocking
/// HTTP) and `run` (executes the bootstrap command) are injected; production
/// supplies a reqwest GET and a `std::process::Command` runner.
pub fn install_krew_capability<F, R>(staging_dir: PathBuf, fetch: F, run: R) -> Capability
where
    F: Fn(&str) -> Result<Vec<u8>, InstallError> + Send + Sync + Clone + 'static,
    R: Fn(&Path, &[&str]) -> Result<(), InstallError> + Send + Sync + Clone + 'static,
{
    Capability::typed::<InstallKrewIn, InstallToolOut, _, _>(
        "toolbox.installKrew",
        "download the latest krew, verify it, and bootstrap it into ~/.krew \
         (the engine for kubectl plugin installs)",
        Annotations::MUTATING,
        move |_input: InstallKrewIn| {
            let staging_dir = staging_dir.clone();
            let fetch = fetch.clone();
            let run = run.clone();
            async move {
                tokio::task::spawn_blocking(move || {
                    run_krew_install(&staging_dir, &fetch, &run).map_err(to_handler)
                })
                .await
                .map_err(|e| CapabilityError::Handler(e.to_string()))?
            }
        },
    )
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchPluginsIn {
    /// Substring to search the krew index for.
    pub query: String,
}

/// One krew index entry.
#[derive(Debug, Serialize, JsonSchema)]
pub struct PluginDto {
    pub name: String,
    pub description: String,
    pub installed: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SearchPluginsOut {
    pub plugins: Vec<PluginDto>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PluginIn {
    /// The krew plugin name (e.g. `oidc-login`).
    pub plugin: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PluginActionOut {
    pub plugin: String,
    /// krew's own output from the operation.
    pub output: String,
}

/// Parse `kubectl krew search` output — a padded `NAME  DESCRIPTION  INSTALLED`
/// table — into plugin rows. Lines before the header are skipped.
pub fn parse_krew_search(stdout: &str) -> Vec<PluginDto> {
    stdout
        .lines()
        .skip_while(|line| !line.trim_start().starts_with("NAME"))
        .skip(1)
        .filter(|line| !line.trim().is_empty())
        .filter_map(parse_search_row)
        .collect()
}

fn parse_search_row(line: &str) -> Option<PluginDto> {
    let name = line.split_whitespace().next()?.to_string();
    let installed_token = line.split_whitespace().next_back()?;
    let installed = installed_token.eq_ignore_ascii_case("yes");
    // Description is what's between the name and the trailing INSTALLED column.
    let mid = line.trim();
    let mid = mid.strip_prefix(&name).unwrap_or(mid).trim_start();
    let description = mid.strip_suffix(installed_token).unwrap_or(mid).trim().to_string();
    Some(PluginDto { name, description, installed })
}

/// Read-only capability: search the krew plugin index. `run` executes
/// `kubectl-krew <args>` and returns its stdout (injected for testing).
pub fn search_plugins_capability<R>(run: R) -> Capability
where
    R: Fn(&[&str]) -> Result<String, InstallError> + Send + Sync + Clone + 'static,
{
    Capability::typed::<SearchPluginsIn, SearchPluginsOut, _, _>(
        "toolbox.searchPlugins",
        "search the krew index for kubectl plugins (name, description, installed)",
        Annotations::READ_ONLY,
        move |input: SearchPluginsIn| {
            let run = run.clone();
            async move {
                tokio::task::spawn_blocking(move || {
                    let out = run(&["search", input.query.as_str()]).map_err(to_handler)?;
                    Ok(SearchPluginsOut { plugins: parse_krew_search(&out) })
                })
                .await
                .map_err(|e| CapabilityError::Handler(e.to_string()))?
            }
        },
    )
}

/// Shared builder for the confirm-gated plugin mutations (install / upgrade /
/// uninstall), which differ only in the krew subcommand.
fn plugin_action_capability<R>(
    id: &'static str,
    summary: &'static str,
    verb: &'static str,
    run: R,
) -> Capability
where
    R: Fn(&[&str]) -> Result<String, InstallError> + Send + Sync + Clone + 'static,
{
    Capability::typed::<PluginIn, PluginActionOut, _, _>(
        id,
        summary,
        Annotations::MUTATING,
        move |input: PluginIn| {
            let run = run.clone();
            async move {
                tokio::task::spawn_blocking(move || {
                    let output = run(&[verb, input.plugin.as_str()]).map_err(to_handler)?;
                    Ok(PluginActionOut { plugin: input.plugin, output })
                })
                .await
                .map_err(|e| CapabilityError::Handler(e.to_string()))?
            }
        },
    )
}

pub fn install_plugin_capability<R>(run: R) -> Capability
where
    R: Fn(&[&str]) -> Result<String, InstallError> + Send + Sync + Clone + 'static,
{
    plugin_action_capability(
        "toolbox.installPlugin",
        "install a kubectl plugin from the krew index",
        "install",
        run,
    )
}

pub fn upgrade_plugin_capability<R>(run: R) -> Capability
where
    R: Fn(&[&str]) -> Result<String, InstallError> + Send + Sync + Clone + 'static,
{
    plugin_action_capability(
        "toolbox.upgradePlugin",
        "upgrade an installed krew plugin",
        "upgrade",
        run,
    )
}

pub fn remove_plugin_capability<R>(run: R) -> Capability
where
    R: Fn(&[&str]) -> Result<String, InstallError> + Send + Sync + Clone + 'static,
{
    plugin_action_capability(
        "toolbox.removePlugin",
        "remove an installed krew plugin",
        "uninstall",
        run,
    )
}

#[cfg(test)]
mod plugin_tests {
    use super::*;

    const SAMPLE: &str = "\
NAME            DESCRIPTION                              INSTALLED
access-matrix   Show an RBAC access matrix               no
oidc-login      Log in to the cluster via OIDC           yes
";

    #[test]
    fn parse_krew_search_reads_name_description_and_installed() {
        let plugins = parse_krew_search(SAMPLE);
        assert_eq!(plugins.len(), 2);
        assert_eq!(plugins[0].name, "access-matrix");
        assert_eq!(plugins[0].description, "Show an RBAC access matrix");
        assert!(!plugins[0].installed);
        assert_eq!(plugins[1].name, "oidc-login");
        assert_eq!(plugins[1].description, "Log in to the cluster via OIDC");
        assert!(plugins[1].installed);
    }

    #[test]
    fn parse_krew_search_is_empty_without_rows() {
        assert!(parse_krew_search("").is_empty());
        assert!(parse_krew_search("NAME  DESCRIPTION  INSTALLED\n").is_empty());
    }
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
    async fn install_krew_resolves_downloads_and_bootstraps() {
        let dir = tempfile::tempdir().unwrap();
        let platform = Platform::current().unwrap();
        let plan = krew_archive("v9.9.9", &platform, dir.path());
        let archive = make_targz(&[(format!("./{}", plan.member).as_str(), b"#!/bin/sh\n")]);
        let fetch = net(vec![
            (KREW_LATEST_RELEASE_URL.to_string(), br#"{"tag_name":"v9.9.9"}"#.to_vec()),
            (plan.sha256_url.clone(), sha256_hex(&archive).into_bytes()),
            (plan.archive_url.clone(), archive),
        ]);
        let bootstrapped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = bootstrapped.clone();
        let run = move |_bin: &Path, args: &[&str]| {
            assert_eq!(args, ["install", "krew"]);
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        };

        let mut reg = Registry::new();
        reg.register(install_krew_capability(dir.path().to_path_buf(), fetch, run));
        let out = reg.invoke("toolbox.installKrew", json!({})).await.unwrap();

        assert_eq!(out["tool"], "krew");
        assert_eq!(out["version"], "v9.9.9");
        assert!(out["path"].as_str().unwrap().ends_with("kubectl-krew"));
        assert!(bootstrapped.load(std::sync::atomic::Ordering::SeqCst), "krew bootstrap ran");
    }

    #[tokio::test]
    async fn install_krew_is_confirm_gated() {
        let cap = install_krew_capability(
            PathBuf::from("/tmp/x"),
            net(vec![]),
            |_b: &Path, _a: &[&str]| Ok(()),
        );
        assert!(cap.annotations.requires_confirm);
    }

    #[tokio::test]
    async fn search_plugins_runs_krew_search_and_parses_results() {
        let run = |args: &[&str]| {
            assert_eq!(args, ["search", "oidc"]);
            Ok("NAME        DESCRIPTION       INSTALLED\noidc-login  OIDC login        yes\n".to_string())
        };
        let mut reg = Registry::new();
        reg.register(search_plugins_capability(run));
        let out = reg.invoke("toolbox.searchPlugins", json!({ "query": "oidc" })).await.unwrap();
        let plugins = out["plugins"].as_array().unwrap();
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0]["name"], "oidc-login");
        assert_eq!(plugins[0]["installed"], true);
    }

    #[tokio::test]
    async fn install_plugin_runs_krew_install_and_is_confirm_gated() {
        use std::sync::{Arc, Mutex};
        let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let run = move |args: &[&str]| {
            sink.lock().unwrap().extend(args.iter().map(|s| s.to_string()));
            Ok("Installed plugin: oidc-login".to_string())
        };
        let cap = install_plugin_capability(run);
        assert!(cap.annotations.requires_confirm);

        let mut reg = Registry::new();
        reg.register(cap);
        let out = reg
            .invoke("toolbox.installPlugin", json!({ "plugin": "oidc-login" }))
            .await
            .unwrap();
        assert_eq!(out["plugin"], "oidc-login");
        assert_eq!(*seen.lock().unwrap(), vec!["install", "oidc-login"]);
    }

    #[tokio::test]
    async fn remove_plugin_uses_the_uninstall_verb() {
        use std::sync::{Arc, Mutex};
        let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let run = move |args: &[&str]| {
            sink.lock().unwrap().extend(args.iter().map(|s| s.to_string()));
            Ok(String::new())
        };
        let mut reg = Registry::new();
        reg.register(remove_plugin_capability(run));
        reg.invoke("toolbox.removePlugin", json!({ "plugin": "oidc-login" })).await.unwrap();
        assert_eq!(*seen.lock().unwrap(), vec!["uninstall", "oidc-login"]);
    }

    #[tokio::test]
    async fn status_inventories_installed_and_missing_managed_tools() {
        let dir = tempfile::tempdir().unwrap();
        let managed = dir.path().join(".srelens/bin");
        std::fs::create_dir_all(&managed).unwrap();
        std::fs::write(managed.join("kubectl"), b"#!/bin/sh\n").unwrap();

        let cap = status_capability(
            SearchPaths { app_path: managed.to_string_lossy().into_owned(), system_path: String::new() },
            vec![managed.clone()],
            |p| p.is_file(),
            |name, _p| (name == "kubectl").then(|| "v1.30.2".to_string()),
        );
        let mut reg = Registry::new();
        reg.register(cap);
        let out = reg.invoke("toolbox.status", json!({})).await.unwrap();

        let tools = out["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 3);
        let kubectl = &tools[0];
        assert_eq!(kubectl["name"], "kubectl");
        assert_eq!(kubectl["installed"], true);
        assert_eq!(kubectl["source"], "managed");
        assert_eq!(kubectl["version"], "v1.30.2");
        assert!(kubectl["path"].as_str().unwrap().ends_with("/kubectl"));
        // krew + helm absent.
        assert_eq!(tools[1]["installed"], false);
        assert_eq!(tools[1]["version"], serde_json::Value::Null);
        assert_eq!(tools[2]["installed"], false);
    }

    #[tokio::test]
    async fn status_classifies_a_tool_outside_managed_dirs_as_system() {
        let dir = tempfile::tempdir().unwrap();
        let sysbin = dir.path().join("usr/local/bin");
        std::fs::create_dir_all(&sysbin).unwrap();
        std::fs::write(sysbin.join("helm"), b"#!/bin/sh\n").unwrap();

        let cap = status_capability(
            SearchPaths { app_path: sysbin.to_string_lossy().into_owned(), system_path: String::new() },
            vec![dir.path().join(".srelens/bin")], // managed dir, not where helm lives
            |p| p.is_file(),
            |_name, _p| None,
        );
        let mut reg = Registry::new();
        reg.register(cap);
        let out = reg.invoke("toolbox.status", json!({})).await.unwrap();
        let helm = &out["tools"][2];
        assert_eq!(helm["name"], "helm");
        assert_eq!(helm["installed"], true);
        assert_eq!(helm["source"], "system");
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
mod version_tests {
    use super::first_semver;

    #[test]
    fn extracts_the_first_semver_across_tool_output_shapes() {
        assert_eq!(first_semver("Client Version: v1.30.2").as_deref(), Some("v1.30.2"));
        assert_eq!(first_semver("v3.16.2+g4f50ac1").as_deref(), Some("v3.16.2"));
        assert_eq!(
            first_semver(r#"{"clientVersion":{"gitVersion":"v1.30.2","major":"1"}}"#).as_deref(),
            Some("v1.30.2"),
        );
        assert_eq!(first_semver("GitTag           v0.4.4").as_deref(), Some("v0.4.4"));
    }

    #[test]
    fn returns_none_without_a_semver() {
        assert_eq!(first_semver("no version here"), None);
        assert_eq!(first_semver("v1.2"), None); // needs three components
        assert_eq!(first_semver(""), None);
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
