//! Durable, developer-mode declarative extensions for desktop hosts.
use base64::Engine;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use srelens_capability::{Annotations, Capability, CapabilityError, Registry};
use srelens_plugin_host::{Manifest, PluginHost};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Installed {
    manifest: Manifest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    freelens: Option<String>,
    grants: Vec<String>,
    enabled: bool,
    revision: u64,
    settings: serde_json::Map<String, Value>,
}
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Inventory {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    #[serde(rename = "developerMode")]
    developer_mode: bool,
    #[serde(rename = "nextRevision")]
    next_revision: u64,
    plugins: Vec<Installed>,
}
impl Default for Inventory {
    fn default() -> Self {
        Self {
            schema_version: 1,
            developer_mode: false,
            next_revision: 1,
            plugins: vec![],
        }
    }
}
#[derive(Deserialize, JsonSchema)]
#[serde(tag = "action", deny_unknown_fields)]
enum Configure {
    #[serde(rename = "developerMode")]
    DeveloperMode { enabled: bool },
    #[serde(rename = "install")]
    Install {
        manifest: String,
        grants: Vec<String>,
    },
    #[serde(rename = "installArchive")]
    InstallArchive {
        archive: String,
        grants: Vec<String>,
    },
    #[serde(rename = "enable")]
    Enable { id: String, enabled: bool },
    #[serde(rename = "remove")]
    Remove { id: String },
    #[serde(rename = "settings")]
    Settings {
        id: String,
        settings: serde_json::Map<String, Value>,
    },
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Read {
    id: String,
    revision: u64,
    capability: String,
    context: String,
    #[serde(default)]
    namespace: String,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Empty {}

fn read(path: &Path) -> Result<Inventory, String> {
    let raw = match fs::read(path) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Inventory::default()),
        Err(e) => return Err(format!("read extension inventory: {e}")),
    };
    if raw.len() > 1024 * 1024 {
        return Err("extension inventory exceeds 1 MiB".into());
    }
    let state: Inventory =
        serde_json::from_slice(&raw).map_err(|e| format!("parse extension inventory: {e}"))?;
    if state.schema_version != 1 {
        return Err("unsupported extension inventory version".into());
    }
    let mut ids = std::collections::BTreeSet::new();
    for plugin in &state.plugins {
        plugin.manifest.validate()?;
        if !ids.insert(&plugin.manifest.id) {
            return Err("duplicate installed extension".into());
        }
    }
    Ok(state)
}
fn write(path: &Path, state: &Inventory) -> Result<(), String> {
    let raw = serde_json::to_vec_pretty(state).map_err(|e| e.to_string())?;
    if raw.len() > 1024 * 1024 {
        return Err("extension inventory exceeds 1 MiB".into());
    }
    let parent = path
        .parent()
        .ok_or("extension inventory has no parent directory")?;
    let result = (|| -> Result<(), std::io::Error> {
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(&raw)?;
        file.as_file().sync_all()?;
        file.persist(path).map_err(|e| e.error)?;
        Ok(())
    })();
    result.map_err(|e| format!("save extension inventory: {e}"))
}
fn validate_app(manifest: &Manifest, grants: &[String], core: Arc<Registry>) -> Result<(), String> {
    for binding in &manifest.capabilities {
        if !matches!(
            binding.target.as_str(),
            "k8s.listCustomResource" | "k8s.listEvents"
        ) || binding
            .inputs
            .iter()
            .any(|key| key != "context" && key != "namespace")
        {
            return Err("This app version supports only read-only custom-resource and event extensions with context/namespace inputs".into());
        }
        let target = core
            .get(&binding.target)
            .ok_or("extension reader is unavailable")?;
        if !target.annotations.read_only
            || target.annotations.requires_confirm
            || target.annotations.sensitive
            || target.annotations.destructive
        {
            return Err(
                "This app extension reader cannot dispatch a gated or mutating host operation"
                    .into(),
            );
        }
        if binding.target == "k8s.listEvents" {
            if !binding.arguments.is_empty()
                || !binding.inputs.iter().any(|k| k == "context")
                || !binding.inputs.iter().any(|k| k == "namespace")
            {
                return Err("Event extensions must accept the host context and namespace without bound arguments".into());
            }
            continue;
        }
        // Core resources, including Secrets, must not be disguised as custom resources.
        for key in ["group", "version", "plural", "kind"] {
            let text = binding
                .arguments
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or("");
            if text.is_empty()
                || text == "."
                || text == ".."
                || !text
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b".-".contains(&c))
            {
                return Err(format!("Extension must bind a valid custom-resource {key}"));
            }
        }
        if !binding.inputs.iter().any(|key| key == "context")
            || binding.arguments.contains_key("context")
            || binding.arguments.contains_key("namespace")
        {
            return Err("Cluster and namespace must come from the host view".into());
        }
        if !binding
            .arguments
            .get("namespaced")
            .is_some_and(Value::is_boolean)
        {
            return Err("Extension must bind resource scope".into());
        }
        if binding.arguments.get("namespaced") == Some(&json!(true))
            && !binding.inputs.iter().any(|key| key == "namespace")
        {
            return Err("Namespaced extensions must accept the host namespace".into());
        }
    }
    // Table surfaces have a resource-row contract; event readers are only valid
    // in the explicitly typed dashboard event slot.
    for name in manifest
        .contributions
        .pages
        .iter()
        .map(|p| &p.capability)
        .chain(
            manifest
                .contributions
                .detail_tabs
                .iter()
                .map(|p| &p.capability),
        )
        .chain(
            manifest
                .contributions
                .row_actions
                .iter()
                .map(|p| &p.capability),
        )
    {
        if !manifest
            .capabilities
            .iter()
            .any(|b| &b.name == name && b.target == "k8s.listCustomResource")
        {
            return Err("Resource contributions must reference a custom-resource reader".into());
        }
    }
    for page in &manifest.contributions.pages {
        if let Some(status) = &page.status_columns {
            let count = manifest
                .capabilities
                .iter()
                .find(|b| b.name == page.capability)
                .and_then(|b| b.arguments.get("printerColumns"))
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            if [Some(status.ready), status.suspended, status.progressing]
                .into_iter()
                .flatten()
                .any(|i| i >= count)
            {
                return Err("Status columns must reference declared printer columns".into());
            }
        }
    }
    let mut temp = Registry::new();
    PluginHost::new(core).register(&mut temp, manifest.clone(), grants)?;
    Ok(())
}
fn mutate(path: &Path, core: Arc<Registry>, input: Configure) -> Result<Inventory, String> {
    let _lock = super::settings::write_lock(path)?;
    let mut state = read(path)?;
    match input {
        Configure::DeveloperMode { enabled } => {
            state.developer_mode = enabled;
            if !enabled {
                for p in &mut state.plugins {
                    p.enabled = false;
                }
            }
        }
        Configure::Install { manifest, grants } => {
            if !state.developer_mode {
                return Err(
                    "Enable extension developer mode before installing unsigned local manifests"
                        .into(),
                );
            }
            let manifest = Manifest::parse(&manifest)?;
            validate_app(&manifest, &grants, core)?;
            let revision = state.next_revision;
            state.next_revision = revision
                .checked_add(1)
                .ok_or("extension revision limit reached")?;
            let previous = state
                .plugins
                .iter()
                .position(|p| p.manifest.id == manifest.id)
                .map(|i| state.plugins.remove(i));
            state.plugins.push(Installed {
                manifest,
                freelens: None,
                grants,
                enabled: true,
                revision,
                settings: previous.map(|p| p.settings).unwrap_or_default(),
            });
            state
                .plugins
                .sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
        }
        Configure::InstallArchive { archive, grants } => {
            if !state.developer_mode {
                return Err(
                    "Enable developer mode before installing a compatibility package".into(),
                );
            }
            if grants != ["freelens.flux.read"] {
                return Err("The archive requires the freelens.flux.read grant".into());
            }
            let package = decode_archive(&archive)?;
            let mut manifest =
                Manifest::parse(include_str!("../../../examples/extensions/flux.json"))?;
            manifest.id = "org.freelensapp.fluxcd".into();
            manifest.name = "FluxCD (Freelens)".into();
            manifest.version = package.version;
            // Host navigation entry only: the package supplies the real pages.
            manifest.contributions.pages.truncate(1);
            manifest.contributions.detail_tabs.clear();
            manifest.contributions.row_actions.clear();
            manifest.contributions.pages[0].dashboard = None;
            manifest.contributions.pages[0].title = "FluxCD".into();
            let revision = state.next_revision;
            state.next_revision = revision
                .checked_add(1)
                .ok_or("Extension revision limit reached")?;
            let settings = state
                .plugins
                .iter()
                .find(|p| p.manifest.id == manifest.id)
                .map(|p| p.settings.clone())
                .unwrap_or_default();
            state.plugins.retain(|p| p.manifest.id != manifest.id);
            state.plugins.push(Installed {
                manifest,
                freelens: Some(archive),
                grants,
                enabled: true,
                revision,
                settings,
            });
        }
        Configure::Enable { id, enabled } => {
            if enabled && !state.developer_mode {
                return Err("Unsigned extensions require developer mode".into());
            }
            let p = state
                .plugins
                .iter_mut()
                .find(|p| p.manifest.id == id)
                .ok_or("Extension is not installed")?;
            if enabled {
                if let Some(archive) = &p.freelens {
                    decode_archive(archive)?;
                } else {
                    validate_app(&p.manifest, &p.grants, core)?;
                }
            }
            p.enabled = enabled;
        }
        Configure::Remove { id } => {
            let i = state
                .plugins
                .iter()
                .position(|p| p.manifest.id == id)
                .ok_or("Extension is not installed")?;
            state.plugins.remove(i);
        }
        Configure::Settings { id, settings } => {
            state
                .plugins
                .iter_mut()
                .find(|p| p.manifest.id == id)
                .ok_or("Extension is not installed")?
                .settings = settings;
        }
    }
    write(path, &state)?;
    Ok(state)
}
fn decode_archive(archive: &str) -> Result<srelens_plugin_host::freelens::FluxArchive, String> {
    if archive.len() > 1400000 {
        return Err("Archive exceeds size limit".into());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(archive)
        .map_err(|e| e.to_string())?;
    srelens_plugin_host::freelens::FluxArchive::parse(&bytes)
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct FreelensRead {
    id: String,
    revision: u64,
    context: String,
    operation: String,
    #[serde(default)]
    group: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    plural: String,
    #[serde(default)]
    kind: String,
}
fn flux_group(group: &str) -> bool {
    matches!(
        group,
        "source.toolkit.fluxcd.io"
            | "kustomize.toolkit.fluxcd.io"
            | "helm.toolkit.fluxcd.io"
            | "image.toolkit.fluxcd.io"
            | "notification.toolkit.fluxcd.io"
            | "fluxcd.controlplane.io"
    )
}
async fn invoke_freelens_read(
    core: &Registry,
    id: &str,
    arguments: Value,
) -> Result<Value, CapabilityError> {
    let capability = core
        .get(id)
        .ok_or_else(|| CapabilityError::Handler("Extension reader unavailable".into()))?;
    let annotations = &capability.annotations;
    if !annotations.read_only
        || annotations.requires_confirm
        || annotations.sensitive
        || annotations.destructive
    {
        return Err(CapabilityError::Handler(
            "The compatibility runtime cannot invoke a gated or mutating reader".into(),
        ));
    }
    core.invoke(id, arguments).await
}
fn register_freelens(reg: &mut Registry, path: PathBuf, core: Arc<Registry>) {
    reg.register(Capability::typed::<FreelensRead, Value, _, _>(
        "extensions.freelensRead", "Read Flux custom resources for an enabled, audited Freelens renderer", Annotations::READ_ONLY,
        move |input: FreelensRead| {
            let path = path.clone(); let core = core.clone();
            async move {
                let fail = |message: &str| CapabilityError::Handler(message.into());
                if input.context.trim().is_empty() { return Err(fail("An explicit cluster context is required")); }
                let state = tokio::task::spawn_blocking(move || read(&path)).await.map_err(|e|fail(&e.to_string()))?.map_err(|e|fail(&e))?;
                let plugin = state.plugins.iter().find(|p| p.manifest.id == input.id && p.revision == input.revision && p.enabled && state.developer_mode)
                    .ok_or_else(||fail("Extension was disabled, removed or updated; refresh the view"))?;
                if plugin.grants != ["freelens.flux.read"] { return Err(fail("Freelens read permission is not granted")); }
                let package = decode_archive(plugin.freelens.as_deref().ok_or_else(||fail("Not a Freelens package"))?).map_err(|e|fail(&e))?;
                if !matches!(input.operation.as_str(), "bootstrap" | "resource" | "events") { return Err(fail("Unsupported Freelens operation; only reads are allowed")); }
                if input.operation == "events" {
                    let mut result = invoke_freelens_read(&core, "k8s.listEvents",json!({"context":input.context,"namespace":""})).await?;
                    if let Some(events) = result["events"].as_array_mut() { events.retain(|e| e["objectApiVersion"].as_str().and_then(|v|v.split_once('/')).is_some_and(|(g,_)|flux_group(g))); }
                    return Ok(result);
                }
                let result = invoke_freelens_read(&core, "k8s.listCrds",json!({"context":input.context})).await?;
                let crds: Vec<Value> = result["crds"].as_array().ok_or_else(||fail("CRD discovery returned no result"))?.iter().filter(|c| c["group"].as_str().is_some_and(flux_group)).cloned().collect();
                if input.operation == "bootstrap" {
                    let namespaces = invoke_freelens_read(&core, "k8s.listNamespaces",json!({"context":input.context})).await?;
                    return Ok(json!({"source":package.renderer,"crds":crds,"namespaces":namespaces["namespaces"]}));
                }
                let crd = crds.iter().find(|c| c["group"] == input.group && c["plural"] == input.plural && c["kind"] == input.kind && c["versions"].as_array().is_some_and(|versions|versions.contains(&json!(input.version))))
                    .ok_or_else(||fail("The requested Flux API is not served by this cluster"))?;
                invoke_freelens_read(&core, "k8s.listCustomResource",json!({"context":input.context,"group":input.group,"version":input.version,"plural":input.plural,"kind":crd["kind"],"namespaced":crd["namespaced"],"namespace":"","includeObjects":true})).await
            }
        }
    ));
}
pub fn register(reg: &mut Registry, path: PathBuf, core: Arc<Registry>) {
    let p = path.clone();
    reg.register(Capability::typed::<Empty, Inventory, _, _>(
        "extensions.list",
        "List installed declarative extensions and developer-mode state",
        Annotations::READ_ONLY,
        move |_| {
            let p = p.clone();
            async move {
                tokio::task::spawn_blocking(move || read(&p))
                    .await
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?
                    .map_err(CapabilityError::Handler)
            }
        },
    ));
    let p = path.clone();
    let c = core.clone();
    reg.register(Capability::typed::<Configure, Inventory, _, _>(
        "extensions.configure",
        "Install, enable, remove or configure local extensions; requires approval",
        Annotations::MUTATING,
        move |input| {
            let p = p.clone();
            let c = c.clone();
            async move {
                tokio::task::spawn_blocking(move || mutate(&p, c, input))
                    .await
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?
                    .map_err(CapabilityError::Handler)
            }
        },
    ));
    register_freelens(reg, path.clone(), core.clone());
    reg.register(Capability::typed::<Read, Value, _, _>(
        "extensions.read",
        "Read a declared custom-resource contribution from an enabled extension",
        Annotations::READ_ONLY,
        move |input: Read| {
            let p = path.clone();
            let c = core.clone();
            async move {
                if input.context.trim().is_empty() {
                    return Err(CapabilityError::InvalidInput(
                        "An explicit cluster context is required".into(),
                    ));
                }
                if !input.namespace.is_empty()
                    && (input.namespace.len() > 63
                        || !input
                            .namespace
                            .bytes()
                            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
                        || input.namespace.starts_with('-')
                        || input.namespace.ends_with('-'))
                {
                    return Err(CapabilityError::InvalidInput(
                        "Namespace must be a Kubernetes namespace name".into(),
                    ));
                }
                let state = tokio::task::spawn_blocking(move || read(&p))
                    .await
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?
                    .map_err(CapabilityError::Handler)?;
                let plugin = state
                    .plugins
                    .iter()
                    .find(|p| {
                        p.manifest.id == input.id && p.enabled && p.revision == input.revision
                    })
                    .filter(|_| state.developer_mode)
                    .ok_or_else(|| {
                        CapabilityError::Handler(
                            "Extension was disabled, removed or updated; refresh the view".into(),
                        )
                    })?;
                validate_app(&plugin.manifest, &plugin.grants, c.clone())
                    .map_err(CapabilityError::Handler)?;
                let mut registry = Registry::new();
                let _registration = PluginHost::new(c)
                    .register(&mut registry, plugin.manifest.clone(), &plugin.grants)
                    .map_err(CapabilityError::Handler)?;
                let mut args = json!({"context":input.context});
                if plugin
                    .manifest
                    .capabilities
                    .iter()
                    .find(|b| b.name == input.capability)
                    .is_some_and(|b| b.inputs.iter().any(|k| k == "namespace"))
                {
                    args["namespace"] = json!(input.namespace);
                }
                registry
                    .invoke(&format!("plugin/{}/{}", input.id, input.capability), args)
                    .await
            }
        },
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use srelens_capability::Registry;
    fn setup(path: &std::path::Path) -> Registry {
        let core = crate::build_registry_with_paths(
            srelens_kube::client_cache::ClientCache::new_many(vec![]),
            vec![],
        );
        let mut reg = Registry::new();
        register(&mut reg, path.to_path_buf(), std::sync::Arc::new(core));
        reg
    }
    fn manifest() -> String {
        include_str!("../../../examples/extensions/argocd.json").into()
    }
    #[tokio::test]
    async fn archive_installation_is_durable_and_reads_are_revoked() {
        use base64::Engine;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        let reg = setup(&path);
        let archive = base64::engine::general_purpose::STANDARD.encode(include_bytes!(
            "../../plugin-host/tests/fixtures/freelens-flux-5.3.1.tgz"
        ));
        let install =
            json!({"action":"installArchive","archive":archive,"grants":["freelens.flux.read"]});
        assert!(reg
            .invoke("extensions.configure", install.clone())
            .await
            .is_err());
        reg.invoke(
            "extensions.configure",
            json!({"action":"developerMode","enabled":true}),
        )
        .await
        .unwrap();
        let state = reg.invoke("extensions.configure", install).await.unwrap();
        assert_eq!(state["plugins"][0]["manifest"]["name"], "FluxCD (Freelens)");
        assert!(state["plugins"][0]["freelens"].is_string());
        let input = json!({"id":"org.freelensapp.fluxcd","revision":1,"context":"test","operation":"bootstrap"});
        reg.invoke(
            "extensions.configure",
            json!({"action":"enable","id":"org.freelensapp.fluxcd","enabled":false}),
        )
        .await
        .unwrap();
        assert!(reg
            .invoke("extensions.freelensRead", input)
            .await
            .unwrap_err()
            .to_string()
            .contains("disabled"));
        assert_eq!(
            setup(&path)
                .invoke("extensions.list", json!({}))
                .await
                .unwrap()["plugins"][0]["enabled"],
            false
        );
    }

    #[tokio::test]
    async fn freelens_broker_reads_only_discovered_flux_resources_in_explicit_context() {
        use base64::Engine;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        let mut core = Registry::new();
        core.register(Capability::read_only("k8s.listCrds","fixture",|args|async move {
            assert_eq!(args["context"], "staging");
            Ok(json!({"crds":[{"group":"source.toolkit.fluxcd.io","kind":"GitRepository","plural":"gitrepositories","versions":["v1"],"namespaced":true},{"group":"other.example","kind":"Secret","plural":"secrets","versions":["v1"]}]}))
        }));
        core.register(Capability::read_only(
            "k8s.listNamespaces",
            "fixture",
            |_| async { Ok(json!({"namespaces":["flux-system"]})) },
        ));
        core.register(Capability::read_only("k8s.listCustomResource","fixture",|args|async move {
            assert_eq!(args["context"],"staging");assert_eq!(args["includeObjects"],true);assert_eq!(args["group"],"source.toolkit.fluxcd.io");assert_eq!(args["namespace"],"");
            Ok(json!({"objects":[{"metadata":{"name":"repo"},"spec":{"url":"https://example.test"}}]}))
        }));
        core.register(Capability::read_only("k8s.listEvents","fixture",|_|async {Ok(json!({"events":[{"objectApiVersion":"v1"},{"objectApiVersion":"source.toolkit.fluxcd.io/v1"}]}))}));
        let mut reg = Registry::new();
        register(&mut reg, path, Arc::new(core));
        reg.invoke(
            "extensions.configure",
            json!({"action":"developerMode","enabled":true}),
        )
        .await
        .unwrap();
        let archive = base64::engine::general_purpose::STANDARD.encode(include_bytes!(
            "../../plugin-host/tests/fixtures/freelens-flux-5.3.1.tgz"
        ));
        reg.invoke(
            "extensions.configure",
            json!({"action":"installArchive","archive":archive,"grants":["freelens.flux.read"]}),
        )
        .await
        .unwrap();
        let mut request = json!({"id":"org.freelensapp.fluxcd","revision":1,"context":"staging","operation":"bootstrap"});
        let bootstrap = reg
            .invoke("extensions.freelensRead", request.clone())
            .await
            .unwrap();
        assert_eq!(bootstrap["crds"].as_array().unwrap().len(), 1);
        assert!(bootstrap["source"]
            .as_str()
            .unwrap()
            .contains("kubeObjectDetailItems"));
        request["operation"] = json!("events");
        assert_eq!(
            reg.invoke("extensions.freelensRead", request.clone())
                .await
                .unwrap()["events"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        request["operation"] = json!("resource");
        request["group"] = json!("source.toolkit.fluxcd.io");
        request["version"] = json!("v1");
        request["kind"] = json!("GitRepository");
        request["plural"] = json!("gitrepositories");
        assert_eq!(
            reg.invoke("extensions.freelensRead", request.clone())
                .await
                .unwrap()["objects"][0]["metadata"]["name"],
            "repo"
        );
        for (key, value) in [
            ("group", "other.example"),
            ("group", ""),
            ("version", "v99"),
            ("plural", "secrets"),
            ("operation", "patch"),
            ("context", ""),
        ] {
            let mut bad = request.clone();
            bad[key] = json!(value);
            assert!(
                reg.invoke("extensions.freelensRead", bad).await.is_err(),
                "{key}={value}"
            );
        }
        request["revision"] = json!(99);
        assert!(reg
            .invoke("extensions.freelensRead", request)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn compatibility_reader_cannot_bypass_a_host_consent_gate() {
        let mut core = Registry::new();
        let mut capability = Capability::read_only("fixture", "fixture", |_| async {
            panic!("gated handler must not execute");
            #[allow(unreachable_code)]
            Ok(json!({}))
        });
        capability.annotations = Annotations::SENSITIVE_READ;
        core.register(capability);
        assert!(invoke_freelens_read(&core, "fixture", json!({}))
            .await
            .is_err());
    }
    #[test]
    fn events_are_an_explicit_read_only_grant() {
        let core = Arc::new(crate::build_registry_with_paths(
            srelens_kube::client_cache::ClientCache::new_many(vec![]),
            vec![],
        ));
        let mut value: Value =
            serde_json::from_str(include_str!("../../../examples/extensions/flux.json")).unwrap();
        let parsed = Manifest::parse(&value.to_string()).unwrap();
        let grants = vec!["k8s.listCustomResource".into(), "k8s.listEvents".into()];
        assert!(validate_app(&parsed, &grants, core.clone()).is_ok());
        assert!(validate_app(&parsed, &["k8s.listCustomResource".into()], core.clone()).is_err());
        value["contributions"]["pages"][1]["capability"] = json!("events");
        let invalid = Manifest::parse(&value.to_string()).unwrap();
        assert!(validate_app(&invalid, &grants, core).is_err());
    }

    #[tokio::test]
    async fn lifecycle_is_persisted_and_unsigned_extensions_require_developer_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        let reg = setup(&path);
        let install =
            json!({"action":"install","manifest":manifest(),"grants":["k8s.listCustomResource"]});
        assert!(reg
            .invoke("extensions.configure", install.clone())
            .await
            .is_err());
        reg.invoke(
            "extensions.configure",
            json!({"action":"developerMode","enabled":true}),
        )
        .await
        .unwrap();
        reg.invoke("extensions.configure", install.clone())
            .await
            .unwrap();
        let state = setup(&path)
            .invoke("extensions.list", json!({}))
            .await
            .unwrap();
        assert_eq!(state["plugins"][0]["manifest"]["id"], "org.srelens.argocd");
        assert_eq!(state["plugins"][0]["enabled"], true);
        reg.invoke(
            "extensions.configure",
            json!({"action":"settings","id":"org.srelens.argocd","settings":{"team":"platform"}}),
        )
        .await
        .unwrap();
        let state = setup(&path)
            .invoke("extensions.list", json!({}))
            .await
            .unwrap();
        assert_eq!(state["plugins"][0]["settings"]["team"], "platform");
        reg.invoke(
            "extensions.configure",
            json!({"action":"developerMode","enabled":false}),
        )
        .await
        .unwrap();
        let state = reg.invoke("extensions.list", json!({})).await.unwrap();
        assert_eq!(state["plugins"][0]["enabled"], false);
        assert!(reg
            .invoke(
                "extensions.configure",
                json!({"action":"enable","id":"org.srelens.argocd","enabled":true})
            )
            .await
            .is_err());
        reg.invoke(
            "extensions.configure",
            json!({"action":"remove","id":"org.srelens.argocd"}),
        )
        .await
        .unwrap();
        assert_eq!(
            setup(&path)
                .invoke("extensions.list", json!({}))
                .await
                .unwrap()["plugins"],
            json!([])
        );
    }
    #[tokio::test]
    async fn invalid_grants_and_non_crd_operations_cannot_be_installed() {
        let dir = tempfile::tempdir().unwrap();
        let reg = setup(&dir.path().join("extensions.json"));
        reg.invoke(
            "extensions.configure",
            json!({"action":"developerMode","enabled":true}),
        )
        .await
        .unwrap();
        assert!(reg
            .invoke(
                "extensions.configure",
                json!({"action":"install","manifest":manifest(),"grants":[]})
            )
            .await
            .is_err());
        let mut source: serde_json::Value = serde_json::from_str(&manifest()).unwrap();
        source["capabilities"][0]["arguments"]["group"] = json!("");
        assert!(reg.invoke("extensions.configure",json!({"action":"install","manifest":source.to_string(),"grants":["k8s.listCustomResource"]})).await.is_err());
        assert_eq!(
            reg.invoke("extensions.list", json!({})).await.unwrap()["plugins"],
            json!([])
        );
    }
    fn fake_core() -> Arc<Registry> {
        let mut core = crate::build_registry_with_paths(
            srelens_kube::client_cache::ClientCache::new_many(vec![]),
            vec![],
        );
        let mut cap = core.get("k8s.listCustomResource").unwrap().clone();
        cap.handler = Arc::new(|args| Box::pin(async move { Ok(args) }));
        core.register(cap);
        Arc::new(core)
    }
    fn install(path: &Path, core: Arc<Registry>) -> u64 {
        mutate(
            path,
            core.clone(),
            Configure::DeveloperMode { enabled: true },
        )
        .unwrap();
        mutate(
            path,
            core,
            Configure::Install {
                manifest: manifest(),
                grants: vec!["k8s.listCustomResource".into()],
            },
        )
        .unwrap()
        .plugins[0]
            .revision
    }
    #[tokio::test]
    async fn reads_are_bound_to_host_scope_and_revoked_across_registry_instances() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        let core = fake_core();
        let revision = install(&path, core.clone());
        let mut reader = Registry::new();
        register(&mut reader, path.clone(), core.clone());
        let args = json!({"id":"org.srelens.argocd","revision":revision,"capability":"applications","context":"staging","namespace":"argo"});
        let output = reader
            .invoke("extensions.read", args.clone())
            .await
            .unwrap();
        assert_eq!(output["group"], "argoproj.io");
        assert_eq!(output["context"], "staging");
        assert_eq!(output["namespace"], "argo");
        let mut missing_context = args.clone();
        missing_context["context"] = json!("");
        assert!(reader
            .invoke("extensions.read", missing_context)
            .await
            .is_err());
        for namespace in ["../v1/namespaces/default", "%2e%2e", "/", "-invalid"] {
            let mut invalid_namespace = args.clone();
            invalid_namespace["namespace"] = json!(namespace);
            assert!(reader
                .invoke("extensions.read", invalid_namespace)
                .await
                .is_err());
        }
        let mut override_scope = args.clone();
        override_scope["group"] = json!("");
        assert!(reader
            .invoke("extensions.read", override_scope)
            .await
            .is_err());
        mutate(
            &path,
            core.clone(),
            Configure::Enable {
                id: "org.srelens.argocd".into(),
                enabled: false,
            },
        )
        .unwrap();
        assert!(reader
            .invoke("extensions.read", args.clone())
            .await
            .is_err());
        mutate(
            &path,
            core.clone(),
            Configure::Enable {
                id: "org.srelens.argocd".into(),
                enabled: true,
            },
        )
        .unwrap();
        assert!(reader.invoke("extensions.read", args.clone()).await.is_ok());
        mutate(
            &path,
            core.clone(),
            Configure::Settings {
                id: "org.srelens.argocd".into(),
                settings: json!({"team":"platform"}).as_object().unwrap().clone(),
            },
        )
        .unwrap();
        let newer = install(&path, core.clone());
        assert!(newer > revision);
        assert_eq!(read(&path).unwrap().plugins[0].settings["team"], "platform");
        assert!(reader
            .invoke("extensions.read", args.clone())
            .await
            .is_err());
        let mut fresh = args;
        fresh["revision"] = json!(newer);
        assert!(reader
            .invoke("extensions.read", fresh.clone())
            .await
            .is_ok());
        mutate(
            &path,
            core,
            Configure::Remove {
                id: "org.srelens.argocd".into(),
            },
        )
        .unwrap();
        assert!(reader.invoke("extensions.read", fresh).await.is_err());
    }
    #[test]
    fn invalid_update_keeps_installed_revision_and_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        let core = fake_core();
        install(&path, core.clone());
        let before = fs::read(&path).unwrap();
        for (field, value) in [
            ("target", json!("k8s.deleteResource")),
            ("inputs", json!(["context", "group"])),
            ("inputs", json!(["context"])),
        ] {
            let mut source: Value = serde_json::from_str(&manifest()).unwrap();
            source["capabilities"][0][field] = value;
            assert!(mutate(
                &path,
                core.clone(),
                Configure::Install {
                    manifest: source.to_string(),
                    grants: vec!["k8s.listCustomResource".into()]
                }
            )
            .is_err());
            assert_eq!(fs::read(&path).unwrap(), before);
        }
        for (field, value) in [
            ("context", json!("prod")),
            ("namespace", json!("prod")),
            ("namespaced", json!("true")),
            ("group", json!("")),
            ("group", json!("..")),
            ("version", json!(".")),
            ("version", json!("../v1")),
        ] {
            let mut source: Value = serde_json::from_str(&manifest()).unwrap();
            source["capabilities"][0]["arguments"][field] = value;
            assert!(mutate(
                &path,
                core.clone(),
                Configure::Install {
                    manifest: source.to_string(),
                    grants: vec!["k8s.listCustomResource".into()]
                }
            )
            .is_err());
            assert_eq!(fs::read(&path).unwrap(), before);
        }
    }
    #[tokio::test]
    async fn corrupt_store_and_failed_saves_report_errors_without_resetting_inventory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        fs::write(&path, "not json").unwrap();
        let reg = setup(&path);
        assert!(reg.invoke("extensions.list", json!({})).await.is_err());
        assert!(reg
            .invoke(
                "extensions.configure",
                json!({"action":"developerMode","enabled":true})
            )
            .await
            .is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "not json");
        let directory = dir.path().join("directory");
        fs::create_dir(&directory).unwrap();
        assert!(write(&directory, &Inventory::default()).is_err());
        assert!(directory.is_dir());
    }
    #[tokio::test]
    async fn mcp_discovery_preserves_lifecycle_consent_and_read_only_annotations() {
        let dir = tempfile::tempdir().unwrap();
        let reg = setup(&dir.path().join("extensions.json"));
        assert!(
            reg.get("extensions.configure")
                .unwrap()
                .annotations
                .requires_confirm
        );
        assert!(reg.get("extensions.read").unwrap().annotations.read_only);
        assert!(
            reg.get("extensions.freelensRead")
                .unwrap()
                .annotations
                .read_only
        );
        let mcp = srelens_mcp::McpServer::new(Arc::new(reg));
        assert_eq!(mcp.list_tools().len(), 4);
        use srelens_mcp::{stdio::handle_request, Transport};
        for args in [
            json!({"action":"developerMode","enabled":true}),
            json!({"action":"developerMode","enabled":true,"_confirm":true}),
        ] {
            let request = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"extensions.configure","arguments":args}});
            let denied = handle_request(&mcp, &request, Transport::Stdio)
                .await
                .unwrap();
            assert_eq!(denied["result"]["isError"], true, "{denied}");
        }
        assert_eq!(
            mcp.call_tool("extensions.list", json!({})).await.unwrap()["developerMode"],
            false
        );
    }
    #[test]
    fn concurrent_updates_do_not_lose_other_extensions_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        let core = fake_core();
        install(&path, core.clone());
        std::thread::scope(|scope| {
            for n in 0..8 {
                let path = &path;
                let core = core.clone();
                scope.spawn(move || {
                    let mut source: Value = serde_json::from_str(&manifest()).unwrap();
                    source["id"] = json!(format!("org.test.extension{n}"));
                    mutate(
                        path,
                        core,
                        Configure::Install {
                            manifest: source.to_string(),
                            grants: vec!["k8s.listCustomResource".into()],
                        },
                    )
                    .unwrap();
                });
            }
        });
        let state = read(&path).unwrap();
        assert_eq!(state.plugins.len(), 9);
        assert_eq!(state.next_revision, 10);
    }
    #[test]
    fn facade_refuses_a_host_reader_that_requires_stronger_consent() {
        let mut core = (*fake_core()).clone();
        let mut cap = core.get("k8s.listCustomResource").unwrap().clone();
        cap.annotations = Annotations::SENSITIVE_READ;
        core.register(cap);
        assert!(validate_app(
            &Manifest::parse(&manifest()).unwrap(),
            &["k8s.listCustomResource".into()],
            Arc::new(core)
        )
        .is_err());
    }
}
