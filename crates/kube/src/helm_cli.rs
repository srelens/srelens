//! Runs the user's installed `helm` (resolved on PATH by fix-path-env) against
//! a context-scoped kubeconfig. Pure arg-builders here; the process runner and
//! capabilities build on them.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use srelens_capability::{Annotations, Capability, CapabilityError};

/// Find `program` on the `PATH`-style `path_var`; first existing candidate wins.
/// `is_file` is injected so this is unit-testable without touching the disk.
pub(crate) fn resolve_on_path(
    program: &str,
    path_var: &str,
    is_file: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    for dir in std::env::split_paths(path_var) {
        let candidate = dir.join(program);
        if is_file(&candidate) {
            return Some(candidate);
        }
    }
    None
}

/// Locate the user's `helm` on PATH, with a friendly error when it's missing.
pub fn helm_binary() -> Result<PathBuf, String> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    resolve_on_path("helm", &path.to_string_lossy(), |p| p.is_file())
        .ok_or_else(|| "helm not found on PATH — install Helm to manage releases".to_string())
}

fn ns_args(args: &mut Vec<String>, namespace: Option<&str>) {
    if let Some(ns) = namespace {
        args.push("--namespace".into());
        args.push(ns.into());
    }
}

fn values_arg(args: &mut Vec<String>, values_file: Option<&Path>) {
    if let Some(f) = values_file {
        args.push("--values".into());
        args.push(f.display().to_string());
    }
}

fn version_arg(args: &mut Vec<String>, version: Option<&str>) {
    if let Some(v) = version {
        args.push("--version".into());
        args.push(v.into());
    }
}

pub fn install_args(
    name: &str,
    chart: &str,
    namespace: Option<&str>,
    values_file: Option<&Path>,
    version: Option<&str>,
) -> Vec<String> {
    let mut a = vec!["install".to_string(), name.to_string(), chart.to_string()];
    if let Some(ns) = namespace {
        a.push("--namespace".into());
        a.push(ns.into());
        a.push("--create-namespace".into());
    }
    values_arg(&mut a, values_file);
    version_arg(&mut a, version);
    a.push("--output".into());
    a.push("json".into());
    a
}

pub fn upgrade_args(
    name: &str,
    chart: &str,
    namespace: Option<&str>,
    values_file: Option<&Path>,
    version: Option<&str>,
) -> Vec<String> {
    let mut a = vec!["upgrade".to_string(), name.to_string(), chart.to_string()];
    ns_args(&mut a, namespace);
    values_arg(&mut a, values_file);
    version_arg(&mut a, version);
    a.push("--output".into());
    a.push("json".into());
    a
}

pub fn rollback_args(name: &str, revision: i64, namespace: Option<&str>) -> Vec<String> {
    let mut a = vec![
        "rollback".to_string(),
        name.to_string(),
        revision.to_string(),
    ];
    ns_args(&mut a, namespace);
    a
}

pub fn uninstall_args(name: &str, namespace: Option<&str>) -> Vec<String> {
    let mut a = vec!["uninstall".to_string(), name.to_string()];
    ns_args(&mut a, namespace);
    a
}

pub fn repo_add_args(name: &str, url: &str) -> Vec<String> {
    vec!["repo".into(), "add".into(), name.into(), url.into()]
}

pub fn repo_update_args() -> Vec<String> {
    vec!["repo".into(), "update".into()]
}

pub fn template_args(
    name: &str,
    chart: &str,
    namespace: Option<&str>,
    values_file: Option<&Path>,
    version: Option<&str>,
) -> Vec<String> {
    let mut a = vec!["template".to_string(), name.to_string(), chart.to_string()];
    ns_args(&mut a, namespace);
    values_arg(&mut a, values_file);
    version_arg(&mut a, version);
    a
}

pub fn version_args() -> Vec<String> {
    vec!["version".into(), "--short".into()]
}

pub fn search_repo_args(chart: &str) -> Vec<String> {
    vec![
        "search".into(),
        "repo".into(),
        chart.into(),
        "--versions".into(),
        "--output".into(),
        "json".into(),
    ]
}

use crate::client_cache::ClientCache;

/// A temp file removed (best-effort) when dropped — so a dropped/cancelled
/// future or an early return can't leak a credential/values file.
pub struct TempFile(pub std::path::PathBuf);
impl TempFile {
    pub fn path(&self) -> &std::path::Path {
        &self.0
    }
}
impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Upper bound on a single `helm` invocation (install/upgrade/rollback/etc.):
/// long enough for slow charts/clusters, short enough to bound a leaked
/// process if the caller is cancelled or the cluster hangs.
const HELM_TIMEOUT_SECS: u64 = 600;

/// Best-effort cleanup of temp files left behind by a crashed/killed prior
/// run: scans the OS temp dir for `srelens-helm-*` entries older than 10
/// minutes and removes them. The age guard keeps a second app instance from
/// deleting another running instance's fresh temp files.
pub fn sweep_stale_temp_files() {
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    // Generous window: a second srelens instance starting mid-op must not sweep
    // the temp files of a long-running helm op belonging to the first instance.
    let cutoff = std::time::Duration::from_secs(60 * 60);
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with("srelens-helm-") {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        let Ok(age) = std::time::SystemTime::now().duration_since(modified) else {
            continue;
        };
        if age > cutoff {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// helm args go through `Command::args` (no shell), but a name/chart/namespace
/// beginning with `-` would be parsed by helm as a flag. Reject it at the
/// capability boundary (these ops are MCP-reachable writes).
fn flag_like(value: &str) -> bool {
    value.starts_with('-')
}

/// Collected result of a one-shot `helm` invocation.
pub struct HelmRunOut {
    pub stdout: String,
    pub stderr: String,
    pub code: i32,
}

/// Map a completed helm run to Ok(stdout) or a message carrying the failure.
pub fn classify_output(out: &HelmRunOut) -> Result<String, String> {
    if out.code == 0 {
        return Ok(out.stdout.clone());
    }
    let detail = if out.stderr.trim().is_empty() {
        out.stdout.trim()
    } else {
        out.stderr.trim()
    };
    Err(format!("helm exited with code {}: {}", out.code, detail))
}

/// Run the user's `helm` with `args` scoped to `context`, collecting output.
/// Writes a temp 0600 kubeconfig for `--kubeconfig`/`KUBECONFIG`, removed via
/// `TempFile`'s `Drop` on every exit path (success, error, or timeout).
pub async fn run_helm(
    cache: &ClientCache,
    context: &str,
    args: &[String],
) -> Result<String, String> {
    let bin = helm_binary()?;
    let paths = cache.paths().await;
    let kc = TempFile(crate::connect::write_single_context_kubeconfig(
        &paths, context,
    )?);
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(HELM_TIMEOUT_SECS),
        tokio::process::Command::new(&bin)
            .args(args)
            .env("KUBECONFIG", kc.path())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| format!("helm timed out after {HELM_TIMEOUT_SECS}s"))?
    .map_err(|e| e.to_string())?;
    let run = HelmRunOut {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code().unwrap_or(-1),
    };
    classify_output(&run)
}

/// Run the user's `helm` with `args` and no scoped kubeconfig — for subcommands
/// that don't touch a cluster (`version`, repo add/update). Inherits the
/// process environment.
pub async fn run_helm_local(args: &[String]) -> Result<String, String> {
    let bin = helm_binary()?;
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(HELM_TIMEOUT_SECS),
        tokio::process::Command::new(&bin)
            .args(args)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| format!("helm timed out after {HELM_TIMEOUT_SECS}s"))?
    .map_err(|e| e.to_string())?;
    let run = HelmRunOut {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code().unwrap_or(-1),
    };
    classify_output(&run)
}

const CONFIRM: Annotations = Annotations {
    read_only: false,
    destructive: false,
    requires_confirm: true,
    sensitive: false,
};

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HelmOpOut {
    /// Raw stdout from helm (JSON for install/upgrade, text otherwise).
    pub output: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HelmVersionOut {
    pub version: String,
}

static VALUES_SEQ: AtomicU64 = AtomicU64::new(1);

/// Write `yaml` to a private temp file (0600) for `helm --values`; returns None
/// for empty/whitespace values so no `-f` flag is added.
pub fn write_values_file(yaml: &str) -> Result<Option<PathBuf>, String> {
    if yaml.trim().is_empty() {
        return Ok(None);
    }
    let id = VALUES_SEQ.fetch_add(1, Ordering::SeqCst);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!(
        "srelens-helm-values-{}-{nanos}-{}.yaml",
        std::process::id(),
        id
    ));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(&path).map_err(|e| e.to_string())?;
    use std::io::Write as _;
    file.write_all(yaml.as_bytes()).map_err(|e| e.to_string())?;
    Ok(Some(path))
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct HelmVersionIn {
    pub context: String,
}

pub fn helm_version_capability(_cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<HelmVersionIn, HelmVersionOut, _, _>(
        "k8s.helmVersion",
        "report the installed Helm client version (detects whether helm is available)",
        Annotations::READ_ONLY,
        move |_input: HelmVersionIn| async move {
            let out = run_helm_local(&version_args())
                .await
                .map_err(CapabilityError::Handler)?;
            Ok(HelmVersionOut {
                version: out.trim().to_string(),
            })
        },
    )
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct HelmInstallIn {
    pub context: String,
    pub name: String,
    pub chart: String,
    #[serde(default)]
    pub namespace: Option<String>,
    /// User values as YAML; empty means chart defaults.
    #[serde(default)]
    pub values: String,
    /// Chart version to install; omitted means the latest.
    #[serde(default)]
    pub version: Option<String>,
}

/// Reject a flag-like release/chart name; `label` names the field in the error.
fn check_not_flag_like(label: &str, value: &str) -> Result<(), CapabilityError> {
    if flag_like(value) {
        return Err(CapabilityError::Handler(format!(
            "invalid {label} '{value}': must not start with '-'"
        )));
    }
    Ok(())
}

/// Reject a flag-like namespace when one was actually supplied.
fn check_namespace_not_flag_like(namespace: Option<&str>) -> Result<(), CapabilityError> {
    if let Some(ns) = namespace {
        if !ns.is_empty() {
            return check_not_flag_like("namespace", ns);
        }
    }
    Ok(())
}

pub fn helm_install_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<HelmInstallIn, HelmOpOut, _, _>(
        "k8s.helmInstall",
        "install a Helm chart as a new release",
        CONFIRM,
        move |input: HelmInstallIn| {
            let cache = cache.clone();
            async move {
                check_not_flag_like("release name", &input.name)?;
                check_not_flag_like("chart", &input.chart)?;
                check_namespace_not_flag_like(input.namespace.as_deref())?;
                if let Some(v) = input.version.as_deref() {
                    check_not_flag_like("version", v)?;
                }
                let vf = write_values_file(&input.values)
                    .map_err(CapabilityError::Handler)?
                    .map(TempFile);
                let args = install_args(
                    &input.name,
                    &input.chart,
                    input.namespace.as_deref(),
                    vf.as_ref().map(|g| g.path()),
                    input.version.as_deref(),
                );
                let out = run_helm(&cache, &input.context, &args).await;
                Ok(HelmOpOut {
                    output: out.map_err(CapabilityError::Handler)?,
                })
            }
        },
    )
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct HelmUpgradeIn {
    pub context: String,
    pub name: String,
    pub chart: String,
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub values: String,
    /// Chart version to upgrade to; omitted means the latest.
    #[serde(default)]
    pub version: Option<String>,
}

pub fn helm_upgrade_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<HelmUpgradeIn, HelmOpOut, _, _>(
        "k8s.helmUpgrade",
        "upgrade an existing Helm release (new chart version and/or values)",
        CONFIRM,
        move |input: HelmUpgradeIn| {
            let cache = cache.clone();
            async move {
                check_not_flag_like("release name", &input.name)?;
                check_not_flag_like("chart", &input.chart)?;
                check_namespace_not_flag_like(input.namespace.as_deref())?;
                if let Some(v) = input.version.as_deref() {
                    check_not_flag_like("version", v)?;
                }
                let vf = write_values_file(&input.values)
                    .map_err(CapabilityError::Handler)?
                    .map(TempFile);
                let args = upgrade_args(
                    &input.name,
                    &input.chart,
                    input.namespace.as_deref(),
                    vf.as_ref().map(|g| g.path()),
                    input.version.as_deref(),
                );
                let out = run_helm(&cache, &input.context, &args).await;
                Ok(HelmOpOut {
                    output: out.map_err(CapabilityError::Handler)?,
                })
            }
        },
    )
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct HelmRollbackIn {
    pub context: String,
    pub name: String,
    pub revision: i64,
    #[serde(default)]
    pub namespace: Option<String>,
}

pub fn helm_rollback_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<HelmRollbackIn, HelmOpOut, _, _>(
        "k8s.helmRollback",
        "roll a Helm release back to a previous revision",
        CONFIRM,
        move |input: HelmRollbackIn| {
            let cache = cache.clone();
            async move {
                check_not_flag_like("release name", &input.name)?;
                check_namespace_not_flag_like(input.namespace.as_deref())?;
                // A negative revision would render as a flag-like arg ("-1").
                if input.revision < 1 {
                    return Err(CapabilityError::Handler(
                        "invalid revision: must be 1 or greater".to_string(),
                    ));
                }
                let args = rollback_args(&input.name, input.revision, input.namespace.as_deref());
                let out = run_helm(&cache, &input.context, &args)
                    .await
                    .map_err(CapabilityError::Handler)?;
                Ok(HelmOpOut { output: out })
            }
        },
    )
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct HelmUninstallIn {
    pub context: String,
    pub name: String,
    #[serde(default)]
    pub namespace: Option<String>,
}

pub fn helm_uninstall_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<HelmUninstallIn, HelmOpOut, _, _>(
        "k8s.helmUninstall",
        "uninstall a Helm release",
        Annotations::DESTRUCTIVE,
        move |input: HelmUninstallIn| {
            let cache = cache.clone();
            async move {
                check_not_flag_like("release name", &input.name)?;
                check_namespace_not_flag_like(input.namespace.as_deref())?;
                let args = uninstall_args(&input.name, input.namespace.as_deref());
                let out = run_helm(&cache, &input.context, &args)
                    .await
                    .map_err(CapabilityError::Handler)?;
                Ok(HelmOpOut { output: out })
            }
        },
    )
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct HelmTemplateIn {
    pub context: String,
    pub name: String,
    pub chart: String,
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub values: String,
    /// Chart version to render; omitted means the latest.
    #[serde(default)]
    pub version: Option<String>,
}

pub fn helm_template_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<HelmTemplateIn, HelmOpOut, _, _>(
        "k8s.helmTemplate",
        "render a chart's manifests locally (helm template) for preview",
        Annotations::READ_ONLY,
        move |input: HelmTemplateIn| {
            let cache = cache.clone();
            async move {
                check_not_flag_like("release name", &input.name)?;
                check_not_flag_like("chart", &input.chart)?;
                check_namespace_not_flag_like(input.namespace.as_deref())?;
                if let Some(v) = input.version.as_deref() {
                    check_not_flag_like("version", v)?;
                }
                let vf = write_values_file(&input.values)
                    .map_err(CapabilityError::Handler)?
                    .map(TempFile);
                let args = template_args(
                    &input.name,
                    &input.chart,
                    input.namespace.as_deref(),
                    vf.as_ref().map(|g| g.path()),
                    input.version.as_deref(),
                );
                let out = run_helm(&cache, &input.context, &args).await;
                Ok(HelmOpOut {
                    output: out.map_err(CapabilityError::Handler)?,
                })
            }
        },
    )
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct HelmRepoAddIn {
    pub context: String,
    pub name: String,
    pub url: String,
}

pub fn helm_repo_add_capability(_cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<HelmRepoAddIn, HelmOpOut, _, _>(
        "k8s.helmRepoAdd",
        "add a chart repository to the local Helm config",
        CONFIRM,
        move |input: HelmRepoAddIn| async move {
            check_not_flag_like("repo name", &input.name)?;
            check_not_flag_like("repo url", &input.url)?;
            let args = repo_add_args(&input.name, &input.url);
            let out = run_helm_local(&args)
                .await
                .map_err(CapabilityError::Handler)?;
            Ok(HelmOpOut { output: out })
        },
    )
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct HelmRepoUpdateIn {
    pub context: String,
}

pub fn helm_repo_update_capability(_cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<HelmRepoUpdateIn, HelmOpOut, _, _>(
        "k8s.helmRepoUpdate",
        "refresh the local cache of chart repositories",
        Annotations::MUTATING,
        move |_input: HelmRepoUpdateIn| async move {
            let out = run_helm_local(&repo_update_args())
                .await
                .map_err(CapabilityError::Handler)?;
            Ok(HelmOpOut { output: out })
        },
    )
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct HelmSearchRepoIn {
    pub context: String,
    pub chart: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HelmChartRef {
    /// Full ref, e.g. "bitnami/nginx".
    pub name: String,
    pub version: String,
    pub app_version: String,
    pub description: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HelmSearchRepoOut {
    pub entries: Vec<HelmChartRef>,
}

/// `helm search repo <chart> --versions -o json` prints keys already matching
/// these field names (`app_version` etc), so no rename is needed to parse it.
#[derive(Debug, Deserialize)]
struct RawChartSearchEntry {
    name: String,
    version: String,
    app_version: String,
    description: String,
}

impl From<RawChartSearchEntry> for HelmChartRef {
    fn from(r: RawChartSearchEntry) -> Self {
        HelmChartRef {
            name: r.name,
            version: r.version,
            app_version: r.app_version,
            description: r.description,
        }
    }
}

/// `helm search repo` matches by substring across every configured repo, so a
/// search for "nginx" also returns "nginx-ingress". Keep only entries whose
/// chart short-name (the part after the last '/') exactly matches `chart`.
pub fn matching_chart_refs(entries: Vec<HelmChartRef>, chart: &str) -> Vec<HelmChartRef> {
    entries
        .into_iter()
        .filter(|e| e.name.rsplit('/').next() == Some(chart))
        .collect()
}

/// Interpret the result of `run_helm_local(&search_repo_args(chart))`: parse
/// the JSON entries on success (filtered to exact chart-name matches), treat
/// helm's "no results found" exit as an empty list (no repo match is a normal
/// outcome, not a failure), and propagate any other error unchanged.
fn parse_search_repo_result(
    result: Result<String, String>,
    chart: &str,
) -> Result<Vec<HelmChartRef>, String> {
    let out = match result {
        Ok(out) => out,
        Err(e) => {
            if e.to_lowercase().contains("no results found") {
                return Ok(vec![]);
            }
            return Err(e);
        }
    };
    let raw: Vec<RawChartSearchEntry> = serde_json::from_str(&out).map_err(|e| e.to_string())?;
    let entries: Vec<HelmChartRef> = raw.into_iter().map(HelmChartRef::from).collect();
    Ok(matching_chart_refs(entries, chart))
}

pub fn helm_search_repo_capability(_cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<HelmSearchRepoIn, HelmSearchRepoOut, _, _>(
        "k8s.helmSearchRepo",
        "search configured Helm repos for a chart by name, resolving its full ref and available versions",
        Annotations::READ_ONLY,
        move |input: HelmSearchRepoIn| async move {
            check_not_flag_like("chart", &input.chart)?;
            let args = search_repo_args(&input.chart);
            let result = run_helm_local(&args).await;
            let entries =
                parse_search_repo_result(result, &input.chart).map_err(CapabilityError::Handler)?;
            Ok(HelmSearchRepoOut { entries })
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client_cache::ClientCache;
    use std::sync::Arc;

    fn cache() -> Arc<ClientCache> {
        ClientCache::new_many(vec![])
    }

    #[test]
    fn capability_ids_and_annotations() {
        let v = helm_version_capability(cache());
        assert_eq!(v.id, "k8s.helmVersion");
        assert!(v.annotations.read_only);

        let install = helm_install_capability(cache());
        assert_eq!(install.id, "k8s.helmInstall");
        assert!(install.annotations.requires_confirm);
        assert!(!install.annotations.destructive);

        let upgrade = helm_upgrade_capability(cache());
        assert_eq!(upgrade.id, "k8s.helmUpgrade");
        assert!(upgrade.annotations.requires_confirm);

        let rollback = helm_rollback_capability(cache());
        assert_eq!(rollback.id, "k8s.helmRollback");
        assert!(rollback.annotations.requires_confirm);

        let uninstall = helm_uninstall_capability(cache());
        assert_eq!(uninstall.id, "k8s.helmUninstall");
        assert!(uninstall.annotations.destructive);
        assert!(uninstall.annotations.requires_confirm);

        let template = helm_template_capability(cache());
        assert_eq!(template.id, "k8s.helmTemplate");
        assert!(template.annotations.read_only);

        let repo_add = helm_repo_add_capability(cache());
        assert_eq!(repo_add.id, "k8s.helmRepoAdd");
        assert!(repo_add.annotations.requires_confirm);

        let repo_update = helm_repo_update_capability(cache());
        assert_eq!(repo_update.id, "k8s.helmRepoUpdate");
        assert!(!repo_update.annotations.read_only);
        assert!(!repo_update.annotations.destructive);
    }

    #[test]
    fn flag_like_rejects_leading_dash() {
        assert!(flag_like("-x"));
        assert!(!flag_like("web"));
        assert!(!flag_like("bitnami/nginx"));
        assert!(!flag_like(""));
    }

    #[tokio::test]
    async fn install_rejects_flag_like_release_name() {
        let install = helm_install_capability(cache());
        let input = serde_json::json!({
            "context": "ctx",
            "name": "--evil",
            "chart": "bitnami/nginx",
            "namespace": null,
            "values": ""
        });
        let err = (install.handler)(input).await.unwrap_err();
        match err {
            CapabilityError::Handler(msg) => assert!(msg.contains("release name")),
            other => panic!("expected Handler error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn rollback_rejects_non_positive_revision() {
        // A negative revision would render as the flag-like arg "-1".
        let rollback = helm_rollback_capability(cache());
        let input = serde_json::json!({
            "context": "ctx",
            "name": "web",
            "revision": -1,
            "namespace": null
        });
        let err = (rollback.handler)(input).await.unwrap_err();
        match err {
            CapabilityError::Handler(msg) => assert!(msg.contains("revision")),
            other => panic!("expected Handler error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn repo_add_rejects_flag_like_url() {
        let repo_add = helm_repo_add_capability(cache());
        let input = serde_json::json!({
            "context": "ctx",
            "name": "bitnami",
            "url": "--some-flag"
        });
        let err = (repo_add.handler)(input).await.unwrap_err();
        match err {
            CapabilityError::Handler(msg) => assert!(msg.contains("repo url")),
            other => panic!("expected Handler error, got {other:?}"),
        }
    }

    #[test]
    fn resolves_program_in_second_path_dir() {
        let found = resolve_on_path("helm", "/nope:/bin", |p| p == Path::new("/bin/helm"));
        assert_eq!(found, Some(PathBuf::from("/bin/helm")));
    }

    #[test]
    fn resolve_returns_none_when_absent() {
        assert_eq!(resolve_on_path("helm", "/a:/b", |_| false), None);
    }

    #[test]
    fn install_args_include_chart_namespace_and_values() {
        let a = install_args(
            "web",
            "bitnami/nginx",
            Some("apps"),
            Some(Path::new("/tmp/v.yaml")),
            None,
        );
        assert_eq!(
            a,
            vec![
                "install",
                "web",
                "bitnami/nginx",
                "--namespace",
                "apps",
                "--create-namespace",
                "--values",
                "/tmp/v.yaml",
                "--output",
                "json",
            ]
        );
    }

    #[test]
    fn install_args_include_version_when_given() {
        let a = install_args(
            "web",
            "bitnami/nginx",
            None,
            Some(Path::new("/tmp/v.yaml")),
            Some("18.1.0"),
        );
        assert_eq!(
            a,
            vec![
                "install",
                "web",
                "bitnami/nginx",
                "--values",
                "/tmp/v.yaml",
                "--version",
                "18.1.0",
                "--output",
                "json",
            ]
        );
    }

    #[test]
    fn upgrade_args_without_namespace_or_values() {
        let a = upgrade_args("web", "bitnami/nginx", None, None, None);
        assert_eq!(
            a,
            vec!["upgrade", "web", "bitnami/nginx", "--output", "json"]
        );
    }

    #[test]
    fn upgrade_args_include_version_when_given() {
        let a = upgrade_args("web", "bitnami/nginx", Some("apps"), None, Some("18.1.0"));
        assert_eq!(
            a,
            vec![
                "upgrade",
                "web",
                "bitnami/nginx",
                "--namespace",
                "apps",
                "--version",
                "18.1.0",
                "--output",
                "json",
            ]
        );
    }

    #[test]
    fn rollback_args_carry_revision() {
        assert_eq!(
            rollback_args("web", 3, Some("apps")),
            vec!["rollback", "web", "3", "--namespace", "apps"]
        );
    }

    #[test]
    fn uninstall_args_carry_namespace() {
        assert_eq!(
            uninstall_args("web", Some("apps")),
            vec!["uninstall", "web", "--namespace", "apps"]
        );
    }

    #[test]
    fn repo_and_version_args() {
        assert_eq!(
            repo_add_args("bitnami", "https://charts.bitnami.com/bitnami"),
            vec![
                "repo",
                "add",
                "bitnami",
                "https://charts.bitnami.com/bitnami"
            ]
        );
        assert_eq!(repo_update_args(), vec!["repo", "update"]);
        assert_eq!(version_args(), vec!["version", "--short"]);
    }

    #[test]
    fn template_args_render_without_output_json() {
        let a = template_args(
            "web",
            "bitnami/nginx",
            Some("apps"),
            Some(Path::new("/tmp/v.yaml")),
            None,
        );
        assert_eq!(
            a,
            vec![
                "template",
                "web",
                "bitnami/nginx",
                "--namespace",
                "apps",
                "--values",
                "/tmp/v.yaml"
            ]
        );
    }

    #[test]
    fn template_args_include_version_when_given_and_no_output_json() {
        let a = template_args("web", "bitnami/nginx", None, None, Some("18.1.0"));
        assert_eq!(
            a,
            vec!["template", "web", "bitnami/nginx", "--version", "18.1.0"]
        );
    }

    #[test]
    fn search_repo_args_build_versioned_json_search() {
        assert_eq!(
            search_repo_args("nginx"),
            vec!["search", "repo", "nginx", "--versions", "--output", "json"]
        );
    }

    fn chart_ref(name: &str, version: &str) -> HelmChartRef {
        HelmChartRef {
            name: name.to_string(),
            version: version.to_string(),
            app_version: "1.0.0".to_string(),
            description: "a chart".to_string(),
        }
    }

    #[test]
    fn matching_chart_refs_keeps_only_exact_short_name_matches() {
        let entries = vec![
            chart_ref("bitnami/nginx", "18.1.0"),
            chart_ref("bitnami/nginx-ingress", "4.0.0"),
            chart_ref("other/nginx", "2.0.0"),
        ];
        let matched = matching_chart_refs(entries, "nginx");
        let names: Vec<&str> = matched.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["bitnami/nginx", "other/nginx"]);
    }

    #[test]
    fn parse_search_repo_result_filters_and_parses_json() {
        let json = r#"[
            {"name":"bitnami/nginx","version":"18.1.0","app_version":"1.27.0","description":"d"},
            {"name":"bitnami/nginx-ingress","version":"4.0.0","app_version":"1.11.0","description":"d"}
        ]"#;
        let entries = parse_search_repo_result(Ok(json.to_string()), "nginx").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "bitnami/nginx");
        assert_eq!(entries[0].app_version, "1.27.0");
    }

    #[test]
    fn parse_search_repo_result_treats_no_results_as_empty() {
        let entries = parse_search_repo_result(
            Err("helm exited with code 1: Error: no results found".to_string()),
            "nginx",
        )
        .unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn parse_search_repo_result_propagates_other_errors() {
        let err = parse_search_repo_result(Err("helm not found on PATH".to_string()), "nginx")
            .unwrap_err();
        assert_eq!(err, "helm not found on PATH");
    }

    #[test]
    fn search_repo_capability_id_and_read_only() {
        let search = helm_search_repo_capability(cache());
        assert_eq!(search.id, "k8s.helmSearchRepo");
        assert!(search.annotations.read_only);
    }

    #[tokio::test]
    async fn search_repo_rejects_flag_like_chart() {
        let search = helm_search_repo_capability(cache());
        let input = serde_json::json!({ "context": "ctx", "chart": "--evil" });
        let err = (search.handler)(input).await.unwrap_err();
        match err {
            CapabilityError::Handler(msg) => assert!(msg.contains("chart")),
            other => panic!("expected Handler error, got {other:?}"),
        }
    }

    #[test]
    fn classify_ok_returns_stdout() {
        let out = HelmRunOut {
            stdout: "done".into(),
            stderr: String::new(),
            code: 0,
        };
        assert_eq!(classify_output(&out), Ok("done".to_string()));
    }

    #[test]
    fn classify_nonzero_returns_stderr_message() {
        let out = HelmRunOut {
            stdout: String::new(),
            stderr: "release not found".into(),
            code: 1,
        };
        let err = classify_output(&out).unwrap_err();
        assert!(err.contains("code 1"));
        assert!(err.contains("release not found"));
    }

    #[test]
    fn classify_nonzero_falls_back_to_stdout_when_stderr_empty() {
        let out = HelmRunOut {
            stdout: "oops".into(),
            stderr: "   ".into(),
            code: 2,
        };
        assert!(classify_output(&out).unwrap_err().contains("oops"));
    }
}
