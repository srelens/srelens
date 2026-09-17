//! Durable, native declarative extensions for desktop hosts.
mod catalog;
pub(crate) mod crd;
#[cfg(any(test, feature = "fuzzing"))]
pub mod fuzzing;
mod resource;
mod signing;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use srelens_capability::{Annotations, Capability, CapabilityError, Registry};
use srelens_plugin_host::{
    Manifest, PluginHost, ValidationCode as Code, ValidationError, ValidationErrors,
};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Installed {
    #[serde(
        default,
        rename = "signatureProof",
        skip_serializing_if = "Option::is_none"
    )]
    signature_proof: Option<SignatureProof>,
    /// Why this host refused to trust the stored entry when loading it. Recomputed
    /// on every read, reported to the UI, and never written to disk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    quarantined: Option<String>,
    manifest: Manifest,
    grants: Vec<String>,
    enabled: bool,
    revision: u64,
    settings: serde_json::Map<String, Value>,
    source: Source,
    /// When this version was installed, in seconds since the Unix epoch.
    #[serde(rename = "installedAt")]
    installed_at: u64,
    /// The versions this one replaced, newest first, at most [`KEPT_VERSIONS`].
    history: Vec<PreviousVersion>,
    /// The keys of the kubeconfig contexts the app is enabled for (`ResolvedContext::key`:
    /// `{file}#{name}` with `#` and `%` encoded in each part, as `k8s.listContexts` reports
    /// under `key`); `None` is every cluster. A context's display name is not identity: it
    /// changes when another kubeconfig declares the same name (#265). A stable ID is not
    /// either: `a` + `b#c` and `a#b` + `c` share one (#623).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    contexts: Option<Vec<String>>,
}
/// What the broker answers when an app is used on a cluster it is not enabled for.
const NOT_ENABLED_FOR_CLUSTER: &str = "App is not enabled for this cluster";
impl Installed {
    /// Refuses a limited app on a context outside its list. A context the host could not
    /// resolve is refused too, but with why: whether the app is enabled there is unknown.
    fn check_scope(
        &self,
        context: &Result<srelens_kube::context_resolve::ResolvedContext, String>,
    ) -> Result<(), CapabilityError> {
        let Some(contexts) = &self.contexts else {
            return Ok(());
        };
        match context {
            Ok(resolved) if contexts.contains(&resolved.key()) => Ok(()),
            Ok(_) => Err(CapabilityError::Handler(NOT_ENABLED_FOR_CLUSTER.into())),
            Err(reason) => Err(CapabilityError::Handler(format!(
                "Could not check whether this app is enabled for this cluster: {reason}"
            ))),
        }
    }
}
/// The context a request names, resolved against the kubeconfig files the host connects
/// with, the same way a connection resolves it. When there is no such context, why: no
/// kubeconfig declares it, or the files that could not be read.
///
/// Scope is checked against its key, and the request goes out under its pinned ID:
/// capabilities resolve their context again, and by name a kubeconfig change in between could
/// reach a cluster that took the name since.
async fn request_context(
    cache: &srelens_kube::client_cache::ClientCache,
    context: &str,
) -> Result<srelens_kube::context_resolve::ResolvedContext, String> {
    let paths = cache.paths().await;
    let all = srelens_kube::context_resolve::resolve_contexts(&paths);
    if let Some(resolved) = srelens_kube::context_resolve::find_context(&all, context) {
        // The request goes on under the pinned ID; without one it cannot go on safely.
        if resolved.pinned_id().is_none() {
            return Err(format!(
                "the kubeconfig path of \"{context}\" cannot be made absolute"
            ));
        }
        return Ok(resolved);
    }
    let unreadable = srelens_kube::context_resolve::unreadable_kubeconfigs(&paths);
    Err(if unreadable.is_empty() {
        format!("no kubeconfig declares the context \"{context}\"")
    } else {
        format!(
            "the context \"{context}\" was not found, and these kubeconfig files could not be read: {}",
            unreadable
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join("; ")
        )
    })
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SignatureProof {
    manifest: String,
    signature: Vec<u8>,
}
/// Where an installed version came from. The host decides: `catalog` means the exact
/// bytes of a release listed in the cached catalog, whoever submitted them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum Source {
    Local,
    Catalog,
}
/// A version an update replaced, kept so it can be restored.
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PreviousVersion {
    #[serde(
        default,
        rename = "signatureProof",
        skip_serializing_if = "Option::is_none"
    )]
    signature_proof: Option<SignatureProof>,
    manifest: Manifest,
    grants: Vec<String>,
    revision: u64,
    source: Source,
    #[serde(rename = "installedAt")]
    installed_at: u64,
}
/// How many replaced versions each app keeps for rollback.
const KEPT_VERSIONS: usize = 3;
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Inventory {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    #[serde(rename = "nextRevision")]
    next_revision: u64,
    plugins: Vec<Installed>,
}
impl Default for Inventory {
    fn default() -> Self {
        Self {
            schema_version: 1,
            next_revision: 1,
            plugins: vec![],
        }
    }
}
#[derive(Deserialize, JsonSchema)]
#[serde(tag = "action", deny_unknown_fields)]
enum Configure {
    #[serde(rename = "install")]
    Install {
        #[serde(default)]
        signature: Option<Vec<u8>>,
        manifest: String,
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
    /// Restores a kept version. Its permissions are granted again, so `grants` is the
    /// caller's consent, as at install.
    #[serde(rename = "rollback")]
    Rollback {
        id: String,
        revision: u64,
        grants: Vec<String>,
    },
    /// Limits the app to these kubeconfig context names, or with `null` allows every cluster.
    #[serde(rename = "clusters")]
    Clusters {
        id: String,
        /// Required: leaving it out is refused rather than read as `null`, which would
        /// quietly allow the app on every cluster.
        #[serde(deserialize_with = "Option::deserialize")]
        #[schemars(schema_with = "context_names_or_null")]
        contexts: Option<Vec<String>>,
    },
}
/// The schema of the clusters action's `contexts`: a list of names or `null`, never absent.
fn context_names_or_null(
    generator: &mut schemars::gen::SchemaGenerator,
) -> schemars::schema::Schema {
    <Option<Vec<String>>>::json_schema(generator)
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Read {
    #[serde(default, rename = "useCrdColumns")]
    use_crd_columns: bool,
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
    let mut stored: Value =
        serde_json::from_slice(&raw).map_err(|e| format!("parse extension inventory: {e}"))?;
    let legacy_mode = stored
        .as_object_mut()
        .and_then(|fields| fields.remove("developerMode"));
    if legacy_mode
        .as_ref()
        .is_some_and(|value| !value.is_boolean())
    {
        return Err("invalid legacy extension developer mode".into());
    }
    if legacy_mode == Some(Value::Bool(false)) {
        if let Some(plugins) = stored.get_mut("plugins").and_then(Value::as_array_mut) {
            for plugin in plugins {
                if let Some(enabled) = plugin.get_mut("enabled") {
                    if enabled == &Value::Bool(true) {
                        *enabled = json!(false);
                    }
                }
            }
        }
    }
    // Retire archive installations without invalidating native manifests/settings.
    // The next atomic inventory save drops the old entries from disk as well.
    if let Some(plugins) = stored.get_mut("plugins").and_then(Value::as_array_mut) {
        plugins.retain(|plugin| plugin.get("freelens").is_none_or(Value::is_null));
        for plugin in plugins {
            if let Some(fields) = plugin.as_object_mut() {
                fields.remove("freelens");
            }
        }
    }
    let mut state: Inventory =
        serde_json::from_value(stored).map_err(|e| format!("parse extension inventory: {e}"))?;
    if state.schema_version != 1 {
        return Err("unsupported extension inventory version".into());
    }
    let mut ids = std::collections::BTreeSet::new();
    for plugin in &state.plugins {
        if !ids.insert(plugin.manifest.id.clone()) {
            return Err("duplicate installed extension".into());
        }
    }
    // One entry this host can no longer trust (a rotated key, a tampered proof, an API
    // version it dropped) is disabled on its own instead of failing every other app.
    for plugin in &mut state.plugins {
        plugin.quarantined = reverify(plugin).err();
        if plugin.quarantined.is_some() {
            plugin.enabled = false;
        }
    }
    Ok(state)
}
fn reverify(plugin: &Installed) -> Result<(), String> {
    // An entry stored before the namespace was reserved, or added by hand, gets no more
    // trust from the file than an install would give it.
    if let Some(reason) = unsigned_reserved(&plugin.manifest.id, plugin.signature_proof.is_some()) {
        return Err(reason);
    }
    plugin.manifest.validate()?;
    crd::group_problems(&plugin.manifest).into_result()?;
    if let Some(proof) = &plugin.signature_proof {
        verify_proof(proof, &plugin.manifest)?;
    }
    Ok(())
}
/// The publisher signature verifies over the kept bytes, and those bytes are `manifest`.
fn verify_proof(proof: &SignatureProof, manifest: &Manifest) -> Result<(), String> {
    signing::verify(proof.manifest.as_bytes(), &proof.signature)?;
    let parsed = Manifest::parse(&proof.manifest)?;
    if serde_json::to_value(parsed).map_err(|e| e.to_string())?
        != serde_json::to_value(manifest).map_err(|e| e.to_string())?
    {
        return Err("Installed app does not match its signed manifest".into());
    }
    Ok(())
}
/// The largest inventory `write` saves, measured in its saved form.
const MAX_INVENTORY_BYTES: usize = 1024 * 1024;
/// The inventory exactly as `write` saves it.
fn saved_form(state: &Inventory) -> Result<Vec<u8>, String> {
    // Quarantine is recomputed on every load. Persisting it would also make the file
    // unreadable to hosts that predate the field.
    let mut stored = serde_json::to_value(state).map_err(|e| e.to_string())?;
    if let Some(plugins) = stored.get_mut("plugins").and_then(Value::as_array_mut) {
        for plugin in plugins.iter_mut().filter_map(Value::as_object_mut) {
            plugin.remove("quarantined");
        }
    }
    serde_json::to_vec_pretty(&stored).map_err(|e| e.to_string())
}
fn write(path: &Path, state: &Inventory) -> Result<(), String> {
    let raw = saved_form(state)?;
    if raw.len() > MAX_INVENTORY_BYTES {
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
/// The manifest's own rules and this app's narrower ones, reporting every violation.
fn validate_app(
    manifest: &Manifest,
    grants: &[String],
    core: Arc<Registry>,
) -> Result<(), ValidationErrors> {
    let mut problems = manifest.validate().err().unwrap_or_default();
    for (index, binding) in manifest.capabilities.iter().enumerate() {
        let at = format!("capabilities[{index}]");
        if !matches!(
            binding.target.as_str(),
            "k8s.listCustomResource" | "k8s.listEvents"
        ) {
            problems.push(
                Code::UnsupportedTarget,
                format!("{at}.target"),
                "This app version supports only k8s.listCustomResource and k8s.listEvents readers",
            );
            continue;
        }
        let Some(target) = core.get(&binding.target) else {
            problems.push(
                Code::UnsupportedTarget,
                format!("{at}.target"),
                "This host does not provide the reader",
            );
            continue;
        };
        if !target.annotations.read_only
            || target.annotations.requires_confirm
            || target.annotations.sensitive
            || target.annotations.destructive
        {
            problems.push(
                Code::UnsupportedTarget,
                format!("{at}.target"),
                "An app reader cannot dispatch a gated or mutating host operation",
            );
            continue;
        }
        for (position, key) in binding.inputs.iter().enumerate() {
            if key != "context" && key != "namespace" {
                problems.push(
                    Code::InvalidBinding,
                    format!("{at}.inputs[{position}]"),
                    format!("\"{key}\" is not an input; readers accept only context and namespace"),
                );
            }
        }
        let accepts = |key: &str| binding.inputs.iter().any(|input| input == key);
        if binding.target == "k8s.listEvents" {
            if !binding.arguments.is_empty() {
                problems.push(
                    Code::InvalidBinding,
                    format!("{at}.arguments"),
                    "Event readers take no bound arguments",
                );
            }
            if !accepts("context") || !accepts("namespace") {
                problems.push(
                    Code::InvalidBinding,
                    format!("{at}.inputs"),
                    "Event readers must accept the host context and namespace",
                );
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
                problems.push(
                    Code::InvalidBinding,
                    format!("{at}.arguments.{key}"),
                    format!("Bind a custom-resource {key} of letters, digits, . and -"),
                );
            }
        }
        if !accepts("context") {
            problems.push(
                Code::InvalidBinding,
                format!("{at}.inputs"),
                "Accept the cluster context from the host view",
            );
        }
        for key in ["context", "namespace"] {
            if binding.arguments.contains_key(key) {
                problems.push(
                    Code::InvalidBinding,
                    format!("{at}.arguments.{key}"),
                    format!("{key} comes from the host view and cannot be bound"),
                );
            }
        }
        match binding.arguments.get("namespaced") {
            Some(Value::Bool(true)) if !accepts("namespace") => problems.push(
                Code::InvalidBinding,
                format!("{at}.inputs"),
                "A namespaced reader must accept the host namespace",
            ),
            Some(Value::Bool(_)) => {}
            _ => problems.push(
                Code::InvalidBinding,
                format!("{at}.arguments.namespaced"),
                "Bind the resource scope as a boolean",
            ),
        }
    }
    // Built-in groups such as apps are not custom resources, whatever their syntax.
    let groups: Vec<_> = crd::group_problems(manifest)
        .0
        .into_iter()
        .filter(|found| !problems.0.iter().any(|p| p.path == found.path))
        .collect();
    problems.0.extend(groups);
    // Table surfaces have a resource-row contract; event readers are only valid
    // in the explicitly typed dashboard event slot.
    let contributions: [(&str, Vec<&String>); 3] = [
        (
            "pages",
            manifest
                .contributions
                .pages
                .iter()
                .map(|p| &p.capability)
                .collect(),
        ),
        (
            "detailTabs",
            manifest
                .contributions
                .detail_tabs
                .iter()
                .map(|p| &p.capability)
                .collect(),
        ),
        (
            "detailLinks",
            manifest
                .contributions
                .detail_links
                .iter()
                .map(|p| &p.capability)
                .collect(),
        ),
    ];
    for (list, capabilities) in contributions {
        for (index, name) in capabilities.into_iter().enumerate() {
            // An undeclared capability is already reported by the manifest's own rules.
            let binding = manifest.capabilities.iter().find(|b| &b.name == name);
            if binding.is_some_and(|b| b.target != "k8s.listCustomResource") {
                problems.push(
                    Code::UnsupportedTarget,
                    format!("contributions.{list}[{index}].capability"),
                    format!("\"{name}\" must be a k8s.listCustomResource reader to back a table"),
                );
            }
        }
    }
    for (index, page) in manifest.contributions.pages.iter().enumerate() {
        let Some(status) = &page.status_columns else {
            continue;
        };
        let Some(binding) = manifest
            .capabilities
            .iter()
            .find(|b| b.name == page.capability)
        else {
            continue;
        };
        let count = binding
            .arguments
            .get("printerColumns")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        for (field, column) in [
            ("ready", Some(status.ready)),
            ("suspended", status.suspended),
            ("progressing", status.progressing),
        ] {
            // Indices of 64 and above are already reported by the manifest's own rules.
            if let Some(column) = column.filter(|&column| column >= count && column < 64) {
                problems.push(
                    Code::InvalidValue,
                    format!("contributions.pages[{index}].statusColumns.{field}"),
                    format!("Column {column} is not one of the {count} declared printerColumns"),
                );
            }
        }
    }
    for permission in &manifest.permissions {
        if !grants.contains(permission) {
            problems.push(
                Code::PermissionMismatch,
                "permissions",
                format!("{permission} was not granted"),
            );
        }
    }
    // The broker checks each binding against its target's input schema whatever else is
    // wrong, except where the target itself is refused or the path is already reported.
    let host = PluginHost::new(core);
    for (index, binding) in manifest.capabilities.iter().enumerate() {
        let target = format!("capabilities[{index}].target");
        if problems
            .0
            .iter()
            .any(|p| p.code == Code::UnsupportedTarget && p.path == target)
        {
            continue;
        }
        let found: Vec<_> = host
            .binding_problems(index, binding)
            .into_iter()
            .filter(|found| !problems.0.iter().any(|p| p.path == found.path))
            .collect();
        problems.0.extend(found);
    }
    problems.into_result()
}
/// Why an app under `id` cannot be trusted without a publisher signature, when it has none
/// and `id` is in a trusted publisher's namespace. Install refuses it; loading quarantines
/// a stored one, which `enable` then refuses; rollback refuses to restore one.
fn unsigned_reserved(id: &str, signed: bool) -> Option<String> {
    (!signed && signing::reserved(id))
        .then(|| format!("App ID {id} is reserved for signed srelens releases"))
}
/// Every reason installing `source` with these grants and signature would be refused.
fn check_install(
    source: &str,
    grants: &[String],
    signature: Option<&[u8]>,
    core: Arc<Registry>,
) -> Result<Manifest, ValidationErrors> {
    let manifest = Manifest::decode(source)?;
    let mut problems = validate_app(&manifest, grants, core)
        .err()
        .unwrap_or_default();
    // Without this, a pasted manifest could replace a signed app, or take an
    // official ID and its logo, differing from the real one only by a label.
    if let Some(reason) = unsigned_reserved(&manifest.id, signature.is_some()) {
        problems.push(
            Code::ReservedId,
            "id",
            format!(
                "{reason}. Install it from the Catalog, or give your local manifest its own ID."
            ),
        );
    }
    // The signature covers the exact bytes, so it is checked whatever else is wrong.
    if let Some(signature) = signature {
        if let Err(reason) = signing::verify_for(&manifest.id, source.as_bytes(), signature) {
            problems.push(Code::InvalidSignature, "", reason);
        }
    }
    problems.into_result()?;
    Ok(manifest)
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn take_revision(state: &mut Inventory) -> Result<u64, String> {
    let revision = state.next_revision;
    state.next_revision = revision
        .checked_add(1)
        .ok_or("extension revision limit reached")?;
    Ok(revision)
}
fn mutate(path: &Path, core: Arc<Registry>, input: Configure) -> Result<Inventory, String> {
    let _lock = super::settings::write_lock(path)?;
    let mut state = read(path)?;
    match input {
        Configure::Install {
            manifest: source,
            grants,
            signature,
        } => {
            let manifest = check_install(&source, &grants, signature.as_deref(), core)?;
            let checksum = format!(
                "{:x}",
                <sha2::Sha256 as sha2::Digest>::digest(source.as_bytes())
            );
            let origin = if catalog::cached_release(
                &path.with_extension("catalog.json"),
                &manifest.id,
                &checksum,
            ) {
                Source::Catalog
            } else {
                Source::Local
            };
            let signature_proof = signature.map(|signature| SignatureProof {
                manifest: source,
                signature,
            });
            let revision = take_revision(&mut state)?;
            let previous = state
                .plugins
                .iter()
                .position(|p| p.manifest.id == manifest.id)
                .map(|i| state.plugins.remove(i));
            // An update keeps the app's settings and clusters, and the version it replaces
            // for rollback.
            let (settings, history, contexts) = match previous {
                Some(Installed {
                    signature_proof: replaced_proof,
                    manifest: replaced,
                    grants: replaced_grants,
                    revision: replaced_revision,
                    source: replaced_source,
                    installed_at: replaced_at,
                    settings,
                    mut history,
                    contexts,
                    ..
                }) => {
                    history.insert(
                        0,
                        PreviousVersion {
                            signature_proof: replaced_proof,
                            manifest: replaced,
                            grants: replaced_grants,
                            revision: replaced_revision,
                            source: replaced_source,
                            installed_at: replaced_at,
                        },
                    );
                    history.truncate(KEPT_VERSIONS);
                    (settings, history, contexts)
                }
                None => (Default::default(), Vec::new(), None),
            };
            state.plugins.push(Installed {
                signature_proof,
                quarantined: None,
                manifest,
                grants,
                enabled: true,
                revision,
                settings,
                source: origin,
                installed_at: now(),
                history,
                contexts,
            });
            state
                .plugins
                .sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
            // Kept versions give way, oldest first, before the saved inventory outgrows its
            // limit; the update itself is not refused. Other apps' versions are left alone.
            while saved_form(&state)?.len() > MAX_INVENTORY_BYTES {
                let updated = state
                    .plugins
                    .iter_mut()
                    .find(|p| p.revision == revision)
                    .ok_or("the updated app is missing from the inventory")?;
                if updated.history.pop().is_none() {
                    break;
                }
            }
        }
        Configure::Rollback {
            id,
            revision,
            grants,
        } => {
            let next = take_revision(&mut state)?;
            let app = state
                .plugins
                .iter_mut()
                .find(|p| p.manifest.id == id)
                .ok_or("Extension is not installed")?;
            let index = app
                .history
                .iter()
                .position(|p| p.revision == revision)
                .ok_or("That version of the app is no longer kept")?;
            let target = app.history[index].clone();
            // A restored version is checked as installing it now would be: against its
            // publisher signature, and against this host's rules with the grants given now.
            if let Some(reason) =
                unsigned_reserved(&target.manifest.id, target.signature_proof.is_some())
            {
                return Err(format!("{reason}. Reinstall it from the Catalog."));
            }
            if let Some(proof) = &target.signature_proof {
                verify_proof(proof, &target.manifest)?;
            }
            validate_app(&target.manifest, &grants, core)?;
            // Going back discards the versions after the restored one.
            app.history.drain(..=index);
            app.signature_proof = target.signature_proof;
            app.manifest = target.manifest;
            app.grants = grants;
            app.source = target.source;
            app.installed_at = target.installed_at;
            app.quarantined = None;
            // A new revision, so views pinned to the rolled-away version refresh.
            app.revision = next;
        }
        Configure::Clusters { id, contexts } => {
            if let Some(contexts) = &contexts {
                // An empty list would be a second way to disable the app.
                if contexts.is_empty() || contexts.len() > 256 {
                    return Err("Choose 1–256 clusters, or allow the app on every cluster".into());
                }
                let mut seen = std::collections::BTreeSet::new();
                for context in contexts {
                    if context.trim().is_empty() || !seen.insert(context.as_str()) {
                        return Err(format!(
                            "Cluster names must be non-empty and listed once: {context:?}"
                        ));
                    }
                }
            }
            state
                .plugins
                .iter_mut()
                .find(|p| p.manifest.id == id)
                .ok_or("Extension is not installed")?
                .contexts = contexts;
        }
        Configure::Enable { id, enabled } => {
            let p = state
                .plugins
                .iter_mut()
                .find(|p| p.manifest.id == id)
                .ok_or("Extension is not installed")?;
            if enabled {
                if let Some(reason) = &p.quarantined {
                    return Err(format!(
                        "This app can't be enabled: {reason}. Remove it or reinstall it from the Catalog."
                    ));
                }
                validate_app(&p.manifest, &p.grants, core)?;
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
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ValidateIn {
    manifest: String,
    #[serde(default)]
    grants: Vec<String>,
    #[serde(default)]
    signature: Option<Vec<u8>>,
}
#[derive(Serialize, Deserialize, JsonSchema)]
struct ValidationReport {
    /// Empty when the manifest could be installed with these grants.
    errors: Vec<ValidationError>,
}
pub fn register(
    reg: &mut Registry,
    path: PathBuf,
    core: Arc<Registry>,
    cache: Arc<srelens_kube::client_cache::ClientCache>,
) {
    catalog::register(reg, path.with_extension("catalog.json"), core.clone());
    resource::register(reg, path.clone(), core.clone(), cache.clone());
    let p = path.clone();
    reg.register(Capability::typed::<Empty, Inventory, _, _>(
        "extensions.list",
        "List installed declarative extensions",
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
    let c = core.clone();
    reg.register(Capability::typed::<ValidateIn, ValidationReport, _, _>(
        "extensions.validate",
        "Check a declarative extension manifest exactly as installing it would and return every problem; does not install it",
        Annotations::READ_ONLY,
        move |input: ValidateIn| {
            let c = c.clone();
            async move {
                let errors =
                    check_install(&input.manifest, &input.grants, input.signature.as_deref(), c)
                        .err()
                        .unwrap_or_default()
                        .0;
                Ok::<_, CapabilityError>(ValidationReport { errors })
            }
        },
    ));
    reg.register(Capability::typed::<Read, Value, _, _>(
        "extensions.read",
        "Read a declared custom-resource contribution from an enabled extension",
        Annotations::READ_ONLY,
        move |input: Read| {
            let p = path.clone();
            let c = core.clone();
            let k = cache.clone();
            async move {
                let resolved = request_context(&k, &input.context).await;
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
                    .ok_or_else(|| {
                        CapabilityError::Handler(
                            "Extension was disabled, removed or updated; refresh the view".into(),
                        )
                    })?;
                plugin.check_scope(&resolved)?;
                validate_app(&plugin.manifest, &plugin.grants, c.clone())
                    .map_err(|errors| CapabilityError::Handler(errors.to_string()))?;
                let mut manifest = plugin.manifest.clone();
                if input.use_crd_columns {
                    if let Some(binding) = manifest.capabilities.iter_mut().find(|b| {
                        b.name == input.capability && b.target == "k8s.listCustomResource"
                    }) {
                        binding
                            .arguments
                            .insert("useCrdColumns".into(), json!(true));
                    }
                }
                let context = resolved
                    .ok()
                    .and_then(|context| context.pinned_id())
                    .unwrap_or(input.context);
                if let Some(binding) =
                    plugin.manifest.capabilities.iter().find(|b| {
                        b.name == input.capability && b.target == "k8s.listCustomResource"
                    })
                {
                    crd::require(&c, &context, binding).await?;
                }
                let mut registry = Registry::new();
                let _registration = PluginHost::new(c)
                    .register(&mut registry, manifest, &plugin.grants)
                    .map_err(CapabilityError::Handler)?;
                let mut args = json!({ "context": context });
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

    /// The inventory's JSON Schema, which includes the manifest's, is what @srelens/core
    /// holds its extension types to (packages/core/src/lib/extensionTypes.test.ts). It
    /// MUST equal these types; regenerate with `UPDATE_CATALOG=1 cargo test -p srelens-registry`.
    #[test]
    fn extension_inventory_schema_json_is_in_sync() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../packages/core/src/lib/extension-inventory.schema.json"
        );
        let want = serde_json::to_value(schemars::schema_for!(Inventory)).unwrap();
        if std::env::var("UPDATE_CATALOG").is_ok() {
            std::fs::write(path, serde_json::to_string_pretty(&want).unwrap() + "\n").unwrap();
            return;
        }
        let got: Value = std::fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or(Value::Null);
        // Compare parsed values, never text: map key order depends on which serde_json
        // features the rest of the workspace enables (see AGENTS.md).
        assert!(
            got == want,
            "extension-inventory.schema.json is stale — run UPDATE_CATALOG=1 cargo test -p srelens-registry"
        );
    }

    #[tokio::test]
    async fn validation_reports_manifest_and_host_problems_together() {
        let dir = tempfile::tempdir().unwrap();
        let reg = setup(&dir.path().join("extensions.json"));
        let grants = json!(["k8s.listCustomResource"]);
        let validate = |manifest: String| {
            reg.invoke(
                "extensions.validate",
                json!({"manifest": manifest, "grants": grants}),
            )
        };
        assert_eq!(validate(manifest()).await.unwrap(), json!({"errors": []}));

        let mut value: Value = serde_json::from_str(&manifest()).unwrap();
        // One manifest rule and two host rules, each independent of the others.
        value["id"] = json!("Not a domain");
        value["capabilities"][0]["arguments"]["plural"] = json!("");
        value["contributions"]["pages"][0]["statusColumns"] = json!({"ready": 5});
        let report = validate(value.to_string()).await.unwrap();
        let mut problems: Vec<(String, String)> = report["errors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|error| {
                (
                    error["code"].as_str().unwrap().to_owned(),
                    error["path"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        problems.sort();
        assert_eq!(
            problems,
            [
                (
                    "EXTENSION_INVALID_BINDING".to_owned(),
                    "capabilities[0].arguments.plural".to_owned()
                ),
                ("EXTENSION_INVALID_ID".to_owned(), "id".to_owned()),
                (
                    "EXTENSION_INVALID_VALUE".to_owned(),
                    "contributions.pages[0].statusColumns.ready".to_owned()
                ),
            ]
        );

        // Installing it is refused with every problem, and nothing is saved.
        let refused = reg
            .invoke(
                "extensions.configure",
                json!({"action":"install","manifest": value.to_string(),"grants": grants}),
            )
            .await
            .unwrap_err()
            .to_string();
        for (_, path) in &problems {
            assert!(refused.contains(path.as_str()), "{refused}");
        }
        assert_eq!(
            reg.invoke("extensions.list", json!({})).await.unwrap()["plugins"],
            json!([])
        );

        // A reserved ID needs the publisher signature, which validation also checks.
        let official = include_str!("../tests/fixtures/argocd-manifest.json");
        let unsigned = validate(official.into()).await.unwrap();
        assert_eq!(unsigned["errors"][0]["code"], "EXTENSION_RESERVED_ID");
        assert_eq!(unsigned["errors"][0]["path"], "id");
        let signature = include_bytes!("../tests/fixtures/argocd-manifest.sig").to_vec();
        let signed = reg
            .invoke(
                "extensions.validate",
                json!({"manifest": official, "grants": grants, "signature": signature}),
            )
            .await
            .unwrap();
        assert_eq!(signed, json!({"errors": []}));
        let tampered = reg
            .invoke(
                "extensions.validate",
                json!({"manifest": official.replace("Argo CD", "Argo CE"), "grants": grants, "signature": signature}),
            )
            .await
            .unwrap();
        assert_eq!(tampered["errors"][0]["code"], "EXTENSION_INVALID_SIGNATURE");

        // Broker and signature checks do not wait for the other problems to be fixed.
        let codes_and_paths = |report: &Value| {
            let mut found: Vec<(String, String)> = report["errors"]
                .as_array()
                .unwrap()
                .iter()
                .map(|error| {
                    (
                        error["code"].as_str().unwrap().to_owned(),
                        error["path"].as_str().unwrap().to_owned(),
                    )
                })
                .collect();
            found.sort();
            found
        };
        let mut value: Value = serde_json::from_str(&manifest()).unwrap();
        value["id"] = json!("Not a domain");
        value["capabilities"][0]["arguments"]["bogus"] = json!("x");
        assert_eq!(
            codes_and_paths(&validate(value.to_string()).await.unwrap()),
            [
                (
                    "EXTENSION_INVALID_BINDING".to_owned(),
                    "capabilities[0].arguments.bogus".to_owned()
                ),
                ("EXTENSION_INVALID_ID".to_owned(), "id".to_owned()),
            ]
        );
        let renamed = official.replace("\"name\": \"Argo CD\"", "\"name\": \"\"");
        assert_ne!(renamed, official);
        let report = reg
            .invoke(
                "extensions.validate",
                json!({"manifest": renamed, "grants": grants, "signature": signature}),
            )
            .await
            .unwrap();
        assert_eq!(
            codes_and_paths(&report),
            [
                ("EXTENSION_INVALID_SIGNATURE".to_owned(), String::new()),
                ("EXTENSION_INVALID_VALUE".to_owned(), "name".to_owned()),
            ]
        );
    }

    fn setup(path: &std::path::Path) -> Registry {
        setup_with(path, vec![]).0
    }
    /// A registry whose client cache connects with `kubeconfigs`, returned so a test can
    /// change the files it resolves contexts from.
    fn setup_with(
        path: &std::path::Path,
        kubeconfigs: Vec<PathBuf>,
    ) -> (Registry, Arc<srelens_kube::client_cache::ClientCache>) {
        let cache = srelens_kube::client_cache::ClientCache::new_many(kubeconfigs.clone());
        let core = crate::build_registry_with_paths(cache.clone(), kubeconfigs);
        let mut reg = Registry::new();
        register(
            &mut reg,
            path.to_path_buf(),
            std::sync::Arc::new(core),
            cache.clone(),
        );
        (reg, cache)
    }
    #[test]
    fn signed_install_rechecks_and_persists_proof_without_trusting_labels() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("apps.json");
        let source = include_str!("../tests/fixtures/argocd-manifest.json");
        let signature = include_bytes!("../tests/fixtures/argocd-manifest.sig").to_vec();
        let install = |manifest: String, signature: Vec<u8>| {
            serde_json::from_value::<Configure>(json!({
                "action": "install", "manifest": manifest,
                "signature": signature, "grants": ["k8s.listCustomResource"]
            }))
            .unwrap()
        };
        mutate(
            &path,
            fake_core(),
            install(source.into(), signature.clone()),
        )
        .unwrap();
        assert!(read(&path).unwrap().plugins[0].signature_proof.is_some());
        assert!(mutate(&path, fake_core(), install(format!("{source} "), signature)).is_err());
        let mut stored: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        stored["plugins"][0]["manifest"]["name"] = json!("Tampered");
        fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();
        let state = read(&path).unwrap();
        assert!(!state.plugins[0].enabled);
        assert!(state.plugins[0]
            .quarantined
            .as_deref()
            .unwrap()
            .contains("does not match"));
    }
    /// The example manifest under an unreserved ID, as a local author would install it.
    pub(super) fn manifest() -> String {
        include_str!("../tests/fixtures/argocd-manifest.json")
            .replace("\"org.srelens.argocd\"", "\"org.example.argocd\"")
    }
    fn signed_argocd() -> Configure {
        Configure::Install {
            signature: Some(include_bytes!("../tests/fixtures/argocd-manifest.sig").to_vec()),
            manifest: include_str!("../tests/fixtures/argocd-manifest.json").into(),
            grants: vec!["k8s.listCustomResource".into()],
        }
    }
    fn configure(path: &Path, input: Value) -> Result<Inventory, String> {
        let input = serde_json::from_value::<Configure>(input).map_err(|e| e.to_string())?;
        mutate(path, fake_core(), input)
    }
    /// The example manifest at another version.
    fn manifest_at(version: &str) -> String {
        let mut value: Value = serde_json::from_str(&manifest()).unwrap();
        value["version"] = json!(version);
        value.to_string()
    }
    #[test]
    fn update_then_rollback_restores_the_previous_manifest_and_grants_under_a_new_revision() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("apps.json");
        let first = install(&path, fake_core());
        configure(
            &path,
            json!({"action":"settings","id":"org.example.argocd","settings":{"team":"platform"}}),
        )
        .unwrap();
        // The update also reads events, so it asks for a second grant.
        let mut updated: Value = serde_json::from_str(&manifest_at("0.3.0")).unwrap();
        updated["permissions"] = json!(["k8s.listCustomResource", "k8s.listEvents"]);
        updated["capabilities"].as_array_mut().unwrap().push(json!({
            "name":"events", "title":"Events", "target":"k8s.listEvents",
            "arguments":{}, "inputs":["context","namespace"]
        }));
        let state = configure(
            &path,
            json!({"action":"install","manifest":updated.to_string(),
                "grants":["k8s.listCustomResource","k8s.listEvents"]}),
        )
        .unwrap();
        let app = &state.plugins[0];
        let second = app.revision;
        assert_eq!(app.manifest.version, "0.3.0");
        assert_eq!(app.history.len(), 1);
        assert_eq!(app.history[0].revision, first);
        assert_eq!(app.history[0].manifest.version, "0.2.0");
        assert_eq!(app.history[0].grants, ["k8s.listCustomResource"]);

        // Rolling back grants the older manifest's permissions again, so it takes them.
        let rollback = |grants: Value, revision: u64| {
            configure(
                &path,
                json!({"action":"rollback","id":"org.example.argocd","revision":revision,"grants":grants}),
            )
        };
        assert!(rollback(json!([]), first).is_err());
        let state = rollback(json!(["k8s.listCustomResource"]), first).unwrap();
        let app = &state.plugins[0];
        assert_eq!(
            serde_json::to_value(&app.manifest).unwrap(),
            serde_json::to_value(Manifest::parse(&manifest()).unwrap()).unwrap()
        );
        assert_eq!(app.grants, ["k8s.listCustomResource"]);
        assert!(app.revision > second, "open views see a new revision");
        assert!(
            app.history.is_empty(),
            "versions after the restored one are discarded"
        );
        assert_eq!(app.settings["team"], "platform");
        assert_eq!(read(&path).unwrap().plugins[0].revision, app.revision);
        // Only a kept version can be restored.
        assert!(rollback(json!(["k8s.listCustomResource"]), second).is_err());
    }
    #[test]
    fn install_records_its_source_and_time() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("apps.json");
        let started = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        install(&path, fake_core());
        let local = read(&path).unwrap().plugins[0].clone();
        assert_eq!(serde_json::to_value(&local.source).unwrap(), "local");
        assert!(local.installed_at >= started);
        // The exact bytes of a cached catalog release are recorded as from the catalog, even
        // when the cache is stale: the host decides this, not the caller.
        let catalog: Value =
            serde_json::from_slice(include_bytes!("../tests/fixtures/extension-catalog.json"))
                .unwrap();
        fs::write(
            path.with_extension("catalog.json"),
            serde_json::to_vec(&json!({
                "catalog": catalog, "fetchedAt": 0, "stale": false, "error": null, "incompatible": []
            }))
            .unwrap(),
        )
        .unwrap();
        mutate(&path, fake_core(), signed_argocd()).unwrap();
        let state = read(&path).unwrap();
        let source = |id: &str| {
            let app = state.plugins.iter().find(|p| p.manifest.id == id).unwrap();
            serde_json::to_value(&app.source).unwrap()
        };
        assert_eq!(source("org.srelens.argocd"), "catalog");
        assert_eq!(
            source("org.example.argocd"),
            "local",
            "other bytes are not the release"
        );
    }
    #[test]
    fn kept_versions_give_way_before_the_inventory_outgrows_its_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("apps.json");
        // A valid manifest with thousands of small entries. The inventory is saved
        // pretty-printed, so each copy takes about 400 KiB there: the installed version
        // and three kept ones cannot all fit in 1 MiB.
        let large = |version: &str| {
            let mut value: Value = serde_json::from_str(&manifest_at(version)).unwrap();
            value["capabilities"][0]["arguments"]["printerColumns"] =
                json!(vec![json!({"name": "c", "jsonPath": ".a"}); 4300]);
            value.to_string()
        };
        for minor in 1..=4 {
            let version = format!("0.{minor}.0");
            configure(
                &path,
                json!({"action":"install","manifest":large(&version),"grants":["k8s.listCustomResource"]}),
            )
            .unwrap_or_else(|error| panic!("install {version}: {error}"));
        }
        let app = &read(&path).unwrap().plugins[0];
        assert_eq!(app.manifest.version, "0.4.0");
        assert!(
            !app.history.is_empty() && app.history.len() < KEPT_VERSIONS,
            "kept {} versions",
            app.history.len()
        );
        assert_eq!(
            app.history[0].manifest.version, "0.3.0",
            "the newest are kept"
        );
        assert!(fs::metadata(&path).unwrap().len() <= 1024 * 1024);
    }
    #[tokio::test]
    async fn an_app_limited_to_some_clusters_is_refused_on_the_others() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        let config = kubeconfig(dir.path(), "clusters.yaml", &["cluster/a", "cluster/b"]);
        let (reg, _cache) = setup_with(&path, vec![config.clone()]);
        let a = format!("{}#cluster/a", config.display());
        let revision = install(&path, fake_core());
        let limit = |contexts: Value| {
            configure(
                &path,
                json!({"action":"clusters","id":"org.example.argocd","contexts":contexts}),
            )
        };
        // An empty list would be a second way to disable the app; a blank or repeated name
        // is a mistake.
        assert!(limit(json!([])).is_err());
        assert!(limit(json!([" "])).is_err());
        assert!(limit(json!([&a, &a])).is_err());
        let only_a = Some(vec![a.clone()]);
        assert_eq!(limit(json!([&a])).unwrap().plugins[0].contexts, only_a);

        let selected =
            json!({"id":"org.example.argocd","revision":revision,"capability":"applications"});
        let on = |context: &str, extra: Value| {
            let mut payload = selected.clone();
            payload["context"] = json!(context);
            payload
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            payload
        };
        let resource = on("cluster/b", json!({"namespace":"team","name":"app"}));
        for (capability, payload) in [
            ("extensions.read", on("cluster/b", json!({"namespace":""}))),
            ("extensions.resource", resource.clone()),
            (
                "extensions.action",
                json!({"resource":resource,"action":"suspend","uid":"u","resourceVersion":"1"}),
            ),
        ] {
            let error = reg
                .invoke(capability, payload)
                .await
                .unwrap_err()
                .to_string();
            assert!(
                error.contains("App is not enabled for this cluster"),
                "{capability}: {error}"
            );
        }

        // An update keeps the list, like settings; clearing it allows every cluster again.
        let updated = configure(
            &path,
            json!({"action":"install","manifest":manifest_at("0.2.0"),"grants":["k8s.listCustomResource"]}),
        )
        .unwrap();
        assert_eq!(updated.plugins[0].contexts, only_a);
        assert_eq!(limit(Value::Null).unwrap().plugins[0].contexts, None);
    }
    #[test]
    fn the_clusters_action_needs_an_explicit_list_or_null() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        install(&path, fake_core());
        let only_a = Some(vec!["cluster/a".to_owned()]);
        configure(
            &path,
            json!({"action":"clusters","id":"org.example.argocd","contexts":["cluster/a"]}),
        )
        .unwrap();
        // Leaving the list out must not quietly allow the app on every cluster.
        assert!(configure(
            &path,
            json!({"action":"clusters","id":"org.example.argocd"})
        )
        .is_err());
        assert_eq!(read(&path).unwrap().plugins[0].contexts, only_a);
        // The input schema callers such as MCP clients see says the same.
        let reg = setup(&path);
        let input = &reg.get("extensions.configure").unwrap().input_schema;
        let clusters = input["oneOf"]
            .as_array()
            .expect("a tagged union of actions")
            .iter()
            .find(|variant| variant["properties"]["action"]["enum"] == json!(["clusters"]))
            .expect("a clusters action");
        assert!(
            clusters["required"]
                .as_array()
                .is_some_and(|required| required.contains(&json!("contexts"))),
            "{clusters}"
        );
    }
    /// A kubeconfig declaring each of `contexts` against an unreachable server, for tests
    /// that resolve context names without a cluster.
    pub(super) fn kubeconfig(dir: &Path, file: &str, contexts: &[&str]) -> PathBuf {
        let path = dir.join(file);
        let mut yaml = String::from(
            "apiVersion: v1\nkind: Config\nclusters:\n- name: c\n  cluster:\n    server: https://127.0.0.1:1\nusers:\n- name: u\n  user: {}\ncontexts:\n",
        );
        for context in contexts {
            yaml.push_str(&format!(
                "- name: {context}\n  context:\n    cluster: c\n    user: u\n"
            ));
        }
        fs::write(&path, yaml).unwrap();
        path
    }
    #[tokio::test]
    async fn clusters_are_kept_by_stable_id_so_a_same_named_context_cannot_inherit_access() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        let first = kubeconfig(dir.path(), "first.yaml", &["default"]);
        let (reg, cache) = setup_with(&path, vec![first.clone()]);
        let revision = install(&path, fake_core());
        let first_default = format!("{}#default", first.display());
        configure(
            &path,
            json!({"action":"clusters","id":"org.example.argocd","contexts":[first_default]}),
        )
        .unwrap();
        let refused = |context: &str| {
            let reg = reg.clone();
            let payload = json!({"id":"org.example.argocd","revision":revision,
                "capability":"applications","context":context,"namespace":""});
            async move {
                reg.invoke("extensions.read", payload)
                    .await
                    .is_err_and(|error| error.to_string().contains(NOT_ENABLED_FOR_CLUSTER))
            }
        };
        // The chosen context is allowed; the read then fails to reach the fixture server.
        assert!(!refused("default").await);

        // Another kubeconfig declaring `default` renames both to `first/default` and
        // `second/default`: the app follows its own cluster, not the name.
        let second = kubeconfig(dir.path(), "second.yaml", &["default"]);
        cache.ensure_paths(vec![second.clone()]).await;
        assert!(!refused("first/default").await);
        assert!(refused("second/default").await);

        // With the first kubeconfig gone, the remaining `default` is another cluster.
        cache.set_paths(vec![second]).await;
        assert!(refused("default").await);
    }
    /// Scope is checked against one resolution of the context name. The read goes out under
    /// the stable ID that was checked, so a kubeconfig change in between cannot hand the name
    /// to another cluster when the capability resolves it again.
    #[tokio::test]
    async fn a_read_goes_to_the_cluster_its_scope_was_checked_against() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        let first = kubeconfig(dir.path(), "first.yaml", &["default"]);
        let core = fake_core();
        let revision = install(&path, core.clone());
        let mut reg = Registry::new();
        register(
            &mut reg,
            path,
            core,
            srelens_kube::client_cache::ClientCache::new_many(vec![first.clone()]),
        );
        let output = reg
            .invoke(
                "extensions.read",
                json!({"id":"org.example.argocd","revision":revision,
                    "capability":"applications","context":"default","namespace":""}),
            )
            .await
            .unwrap();
        assert_eq!(
            output["context"],
            format!("srelens-context:{}#default", first.display())
        );
    }
    /// A context can be named anything, including another context's stable ID. A request the
    /// broker sends under that ID reaches the context the ID names, and never the one that
    /// happens to be called it, even once the named context is gone.
    #[test]
    fn a_context_named_like_a_stable_id_cannot_take_a_pinned_request() {
        let dir = tempfile::tempdir().unwrap();
        let first = kubeconfig(dir.path(), "first.yaml", &["default"]);
        let first_default = format!("srelens-context:{}#default", first.display());
        let impostor = kubeconfig(dir.path(), "impostor.yaml", &[&first_default]);
        let reached = |paths: &[PathBuf]| {
            srelens_kube::context_resolve::resolve_context(paths, &first_default)
                .map(|context| context.source)
        };
        // Alone, the ID reaches its context; with the impostor listed, the string means two
        // things, so it reaches nothing rather than either.
        assert_eq!(reached(&[first.clone()]), Some(first.clone()));
        assert_eq!(reached(&[impostor.clone(), first]), None);
        assert_eq!(reached(&[impostor]), None);
    }
    /// A kubeconfig can be given by a relative path, and its contexts' stable IDs keep that
    /// path: they are persisted (context profiles, remembered namespaces, app clusters), so
    /// they must not change. A checked request still goes on under an absolute ID, which a
    /// context named after it cannot take.
    #[tokio::test]
    async fn a_relative_kubeconfig_keeps_its_stable_id_but_requests_pin_an_absolute_one() {
        let dir = tempfile::tempdir_in(".").unwrap();
        kubeconfig(dir.path(), "first.yaml", &["default"]);
        let relative = PathBuf::from(dir.path().file_name().unwrap()).join("first.yaml");
        let stable = format!("{}#default", relative.display());
        let given = [relative.clone()];
        let listed = srelens_kube::context_resolve::resolve_contexts(&given);
        assert_eq!(listed[0].stable_id(), stable);

        let path = dir.path().join("extensions.json");
        let core = fake_core();
        let revision = install(&path, core.clone());
        configure(
            &path,
            json!({"action":"clusters","id":"org.example.argocd","contexts":[&stable]}),
        )
        .unwrap();
        let mut reg = Registry::new();
        let cache = srelens_kube::client_cache::ClientCache::new_many(vec![relative.clone()]);
        register(&mut reg, path, core, cache);
        let output = reg
            .invoke(
                "extensions.read",
                json!({"id":"org.example.argocd","revision":revision,
                    "capability":"applications","context":"default","namespace":""}),
            )
            .await
            .unwrap();
        let pinned = output["context"].as_str().unwrap().to_owned();
        assert!(
            Path::new(pinned.strip_prefix("srelens-context:").unwrap()).is_absolute()
                && pinned.ends_with("first.yaml#default"),
            "{pinned}"
        );

        let impostor = kubeconfig(dir.path(), "impostor.yaml", &[&pinned]);
        let reached = |paths: &[PathBuf]| {
            srelens_kube::context_resolve::resolve_context(paths, &pinned)
                .map(|context| context.original_name)
        };
        assert_eq!(reached(&[relative.clone()]), Some("default".to_owned()));
        assert_eq!(reached(&[impostor.clone(), relative]), None);
        assert_eq!(reached(&[impostor]), None);
    }
    /// A file path and a context name can both contain `#`, so two contexts can share one
    /// stable ID: `a` + `b#c` and `a#b` + `c`. The cluster list keys on the context key, which
    /// encodes both parts, so the chosen cluster is the only one that gets the app, even
    /// when the other is added after the chosen one is gone (they need never coexist).
    #[tokio::test]
    async fn an_apps_cluster_list_keys_on_the_context_key_not_the_shared_stable_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        let first = kubeconfig(dir.path(), "a", &["b#c"]);
        let second = kubeconfig(dir.path(), "a#b", &["c"]);
        let listed =
            srelens_kube::context_resolve::resolve_contexts(&[first.clone(), second.clone()]);
        assert_eq!(listed[0].stable_id(), listed[1].stable_id());
        let (reg, cache) = setup_with(&path, vec![first.clone(), second.clone()]);
        let revision = install(&path, fake_core());
        configure(
            &path,
            json!({"action":"clusters","id":"org.example.argocd","contexts":[listed[0].key()]}),
        )
        .unwrap();
        let refused = |context: String| {
            let reg = reg.clone();
            let payload = json!({"id":"org.example.argocd","revision":revision,
                "capability":"applications","context":context,"namespace":""});
            async move {
                reg.invoke("extensions.read", payload)
                    .await
                    .is_err_and(|error| error.to_string().contains(NOT_ENABLED_FOR_CLUSTER))
            }
        };
        assert!(!refused(listed[0].pinned_id().unwrap()).await);
        assert!(refused(listed[1].pinned_id().unwrap()).await);
        // The chosen context is gone and the other is the sole holder of the stable ID.
        cache.set_paths(vec![second]).await;
        assert!(refused("c".to_owned()).await);
    }
    /// A limited app is refused on a context the host cannot resolve, but with why: whether
    /// the app is enabled there is unknown, which is not the same as not enabled.
    #[tokio::test]
    async fn an_unresolvable_context_is_refused_with_the_reason_not_as_a_denial() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        let first = kubeconfig(dir.path(), "first.yaml", &["default"]);
        let (reg, _cache) = setup_with(&path, vec![first.clone()]);
        let revision = install(&path, fake_core());
        configure(
            &path,
            json!({"action":"clusters","id":"org.example.argocd",
                "contexts":[format!("{}#default", first.display())]}),
        )
        .unwrap();
        let refusal = |context: &str| {
            let reg = reg.clone();
            let payload = json!({"id":"org.example.argocd","revision":revision,
                "capability":"applications","context":context,"namespace":""});
            async move {
                reg.invoke("extensions.read", payload)
                    .await
                    .unwrap_err()
                    .to_string()
            }
        };
        let missing = refusal("elsewhere").await;
        assert!(!missing.contains(NOT_ENABLED_FOR_CLUSTER), "{missing}");
        assert!(
            missing.contains("no kubeconfig declares the context \"elsewhere\""),
            "{missing}"
        );

        // The allowed context's kubeconfig can no longer be read: still refused, and says so.
        fs::write(&first, "contexts: [").unwrap();
        let unreadable = refusal("default").await;
        assert!(
            !unreadable.contains(NOT_ENABLED_FOR_CLUSTER),
            "{unreadable}"
        );
        assert!(
            unreadable.contains("could not be read") && unreadable.contains("first.yaml"),
            "{unreadable}"
        );

        // Only the path: a parse error can quote the file's contents, credentials included.
        fs::write(
            &first,
            "apiVersion: v1\nkind: Config\nusers: \"hunter2-token\"\n",
        )
        .unwrap();
        let quoted = refusal("default").await;
        assert!(quoted.contains("first.yaml"), "{quoted}");
        assert!(!quoted.contains("hunter2-token"), "{quoted}");
    }
    #[test]
    fn history_keeps_the_three_versions_before_the_installed_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("apps.json");
        let mut revisions = Vec::new();
        for minor in 1..=5 {
            let state = configure(
                &path,
                json!({"action":"install","manifest":manifest_at(&format!("0.{minor}.0")),
                    "grants":["k8s.listCustomResource"]}),
            )
            .unwrap();
            revisions.push(state.plugins[0].revision);
        }
        let app = &read(&path).unwrap().plugins[0];
        assert_eq!(app.manifest.version, "0.5.0");
        let kept: Vec<_> = app
            .history
            .iter()
            .map(|previous| (previous.manifest.version.as_str(), previous.revision))
            .collect();
        assert_eq!(
            kept,
            [
                ("0.4.0", revisions[3]),
                ("0.3.0", revisions[2]),
                ("0.2.0", revisions[1])
            ]
        );
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

    #[test]
    fn native_only_inventory_skips_retired_archives_and_preserves_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        let native = json!({"manifest":serde_json::from_str::<Value>(&manifest()).unwrap(),"grants":["k8s.listCustomResource"],"enabled":true,"revision":2,"settings":{"namespace":"team"},"source":"local","installedAt":1,"history":[]});
        let retired =
            json!({"freelens":"legacy-archive","manifest":{"id":"org.freelensapp.fluxcd"}});
        fs::write(&path, serde_json::to_vec(&json!({"schemaVersion":1,"developerMode":true,"nextRevision":3,"plugins":[retired,native.clone()]})).unwrap()).unwrap();
        let state = read(&path).unwrap();
        assert_eq!(state.plugins.len(), 1);
        assert_eq!(serde_json::to_value(&state.plugins[0]).unwrap(), native);
        assert_eq!(state.next_revision, 3);
        write(&path, &state).unwrap();
        assert!(!fs::read_to_string(&path).unwrap().contains("freelens"));
        assert!(serde_json::from_value::<Configure>(
            json!({"action":"installArchive","archive":"anything","grants":[]})
        )
        .is_err());
        assert!(setup(&path).get("extensions.freelensRead").is_none());
    }
    #[test]
    fn legacy_mode_migration_preserves_state_without_reactivating_disabled_extensions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        install(&path, fake_core());
        let mut stored: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        stored["plugins"][0]["settings"] = json!({"team":"platform"});
        for enabled in [true, false] {
            stored["developerMode"] = json!(enabled);
            fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();
            let state = read(&path).unwrap();
            assert_eq!(state.plugins[0].enabled, enabled);
            assert_eq!(state.plugins[0].revision, 1);
            assert_eq!(state.plugins[0].settings["team"], "platform");
            write(&path, &state).unwrap();
            let saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            assert!(saved.get("developerMode").is_none());
        }
        stored["developerMode"] = json!("false");
        fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();
        assert!(read(&path).is_err());
        stored["developerMode"] = json!(false);
        stored["plugins"] = json!([42]);
        fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();
        assert!(read(&path).is_err());
    }
    #[tokio::test]
    async fn lifecycle_is_persisted_without_developer_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        let reg = setup(&path);
        let install =
            json!({"action":"install","manifest":manifest(),"grants":["k8s.listCustomResource"]});
        reg.invoke("extensions.configure", install.clone())
            .await
            .unwrap();
        let state = setup(&path)
            .invoke("extensions.list", json!({}))
            .await
            .unwrap();
        assert_eq!(state["plugins"][0]["manifest"]["id"], "org.example.argocd");
        assert_eq!(state["plugins"][0]["enabled"], true);
        reg.invoke(
            "extensions.configure",
            json!({"action":"settings","id":"org.example.argocd","settings":{"team":"platform"}}),
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
            json!({"action":"enable","id":"org.example.argocd","enabled":false}),
        )
        .await
        .unwrap();
        let state = reg.invoke("extensions.list", json!({})).await.unwrap();
        assert_eq!(state["plugins"][0]["enabled"], false);
        assert!(reg
            .invoke(
                "extensions.configure",
                json!({"action":"enable","id":"org.example.argocd","enabled":true})
            )
            .await
            .is_ok());
        reg.invoke(
            "extensions.configure",
            json!({"action":"remove","id":"org.example.argocd"}),
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
    /// The reader binding of `manifest()` retargeted at a built-in API.
    fn bind_builtin(source: &mut Value, group: &str, plural: &str, kind: &str) {
        let arguments = &mut source["capabilities"][0]["arguments"];
        arguments["group"] = json!(group);
        arguments["version"] = json!("v1");
        arguments["plural"] = json!(plural);
        arguments["kind"] = json!(kind);
    }
    #[tokio::test]
    async fn built_in_api_groups_cannot_be_installed() {
        let dir = tempfile::tempdir().unwrap();
        let reg = setup(&dir.path().join("extensions.json"));
        for (group, plural, kind) in [
            ("apps", "deployments", "Deployment"),
            ("batch", "jobs", "Job"),
            ("apps.", "deployments", "Deployment"),
        ] {
            let mut source: Value = serde_json::from_str(&manifest()).unwrap();
            bind_builtin(&mut source, group, plural, kind);
            let manifest = source.to_string();
            let grants = json!(["k8s.listCustomResource"]);
            let report = reg
                .invoke(
                    "extensions.validate",
                    json!({"manifest": manifest, "grants": grants}),
                )
                .await
                .unwrap();
            assert_eq!(
                report["errors"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|e| (e["code"].as_str().unwrap(), e["path"].as_str().unwrap()))
                    .collect::<Vec<_>>(),
                [(
                    "EXTENSION_INVALID_BINDING",
                    "capabilities[0].arguments.group"
                )],
                "{group}"
            );
            let refused = reg
                .invoke(
                    "extensions.configure",
                    json!({"action":"install","manifest": manifest,"grants": grants}),
                )
                .await
                .unwrap_err()
                .to_string();
            assert!(
                refused.contains("capabilities[0].arguments.group"),
                "{refused}"
            );
        }
        assert_eq!(
            reg.invoke("extensions.list", json!({})).await.unwrap()["plugins"],
            json!([])
        );
    }
    /// A dotted group under `k8s.io` may be a CRD's, so installing it is left to the
    /// per-cluster check; the built-in `networking.k8s.io` is refused there instead.
    #[tokio::test]
    async fn crd_groups_under_k8s_io_install_and_are_checked_per_cluster() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        let mut core = (*fake_core()).clone();
        serve_crds(&mut core, &["gateways.gateway.networking.k8s.io/v1"]);
        let core = Arc::new(core);
        let reader = |id: &str, group: &str, plural: &str, kind: &str| {
            let mut source: Value = serde_json::from_str(&manifest()).unwrap();
            source["id"] = json!(id);
            bind_builtin(&mut source, group, plural, kind);
            Configure::Install {
                signature: None,
                manifest: source.to_string(),
                grants: vec!["k8s.listCustomResource".into()],
            }
        };
        let state = mutate(
            &path,
            core.clone(),
            reader(
                "org.example.gateway",
                "gateway.networking.k8s.io",
                "gateways",
                "Gateway",
            ),
        )
        .unwrap();
        let gateway = find(&state, "org.example.gateway").revision;
        let state = mutate(
            &path,
            core.clone(),
            reader(
                "org.example.ingress",
                "networking.k8s.io",
                "ingresses",
                "Ingress",
            ),
        )
        .unwrap();
        let ingress = find(&state, "org.example.ingress").revision;
        assert!(read(&path)
            .unwrap()
            .plugins
            .iter()
            .all(|p| p.enabled && p.quarantined.is_none()));

        let mut reg = Registry::new();
        register(
            &mut reg,
            path,
            core,
            srelens_kube::client_cache::ClientCache::new_many(vec![]),
        );
        let read_args = |id: &str, revision: u64| json!({"id":id,"revision":revision,"capability":"applications","context":"staging","namespace":""});
        assert!(reg
            .invoke("extensions.read", read_args("org.example.gateway", gateway))
            .await
            .is_ok());
        let refused = reg
            .invoke("extensions.read", read_args("org.example.ingress", ingress))
            .await
            .unwrap_err()
            .to_string();
        assert!(
            refused.contains("No CustomResourceDefinition ingresses.networking.k8s.io serving v1"),
            "{refused}"
        );
    }
    #[tokio::test]
    async fn a_stored_app_binding_a_built_in_group_is_quarantined_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        let core = fake_core();
        let revision = install(&path, core.clone());
        let mut stored: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        bind_builtin(
            &mut stored["plugins"][0]["manifest"],
            "apps",
            "deployments",
            "Deployment",
        );
        fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();

        let state = read(&path).unwrap();
        let app = &state.plugins[0];
        assert!(!app.enabled);
        let reason = app.quarantined.as_deref().unwrap();
        assert!(
            reason.contains("capabilities[0].arguments.group"),
            "{reason}"
        );

        let mut reg = Registry::new();
        register(
            &mut reg,
            path.clone(),
            core,
            srelens_kube::client_cache::ClientCache::new_many(vec![]),
        );
        assert!(reg
            .invoke(
                "extensions.read",
                json!({"id":"org.example.argocd","revision":revision,
                    "capability":"applications","context":"staging","namespace":""}),
            )
            .await
            .is_err());
    }
    #[tokio::test]
    async fn a_read_is_refused_unless_the_cluster_has_the_bound_crd() {
        static LISTED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        let revision = install(&path, fake_core());
        let read_on = |crds: &'static [&'static str], failing: bool| {
            let mut core = (*fake_core()).clone();
            let mut cap = core.get("k8s.listCustomResource").unwrap().clone();
            cap.handler = Arc::new(|args| {
                LISTED.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Box::pin(async move { Ok(args) })
            });
            core.register(cap);
            serve_crds(&mut core, crds);
            if failing {
                let mut cap = core.get(crd::CHECK).unwrap().clone();
                cap.handler = Arc::new(|_| {
                    Box::pin(async { Err(CapabilityError::Handler("forbidden".into())) })
                });
                core.register(cap);
            }
            let mut reg = Registry::new();
            register(
                &mut reg,
                path.clone(),
                Arc::new(core),
                srelens_kube::client_cache::ClientCache::new_many(vec![]),
            );
            async move {
                reg.invoke(
                    "extensions.read",
                    json!({"id":"org.example.argocd","revision":revision,
                        "capability":"applications","context":"staging","namespace":""}),
                )
                .await
            }
        };
        // A dotted group served by an aggregated API, or a CRD this cluster lacks.
        let absent = read_on(&[], false).await.unwrap_err().to_string();
        assert!(
            absent
                .contains("No CustomResourceDefinition applications.argoproj.io serving v1alpha1"),
            "{absent}"
        );
        // The CRD exists but does not serve the bound version, which another API may.
        assert!(read_on(&["applications.argoproj.io/v1beta1"], false)
            .await
            .is_err());
        // A lookup that failed says so, and is not reported as an absence.
        let failed = read_on(&["applications.argoproj.io/v1alpha1"], true)
            .await
            .unwrap_err()
            .to_string();
        assert!(failed.contains("Could not confirm"), "{failed}");
        assert!(failed.contains("forbidden"), "{failed}");
        assert_eq!(LISTED.load(std::sync::atomic::Ordering::SeqCst), 0);

        assert!(read_on(&["applications.argoproj.io/v1alpha1"], false)
            .await
            .is_ok());
        assert_eq!(LISTED.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
    pub(super) fn fake_core() -> Arc<Registry> {
        let mut core = crate::build_registry_with_paths(
            srelens_kube::client_cache::ClientCache::new_many(vec![]),
            vec![],
        );
        let mut cap = core.get("k8s.listCustomResource").unwrap().clone();
        cap.handler = Arc::new(|args| Box::pin(async move { Ok(args) }));
        core.register(cap);
        serve_crds(&mut core, &["applications.argoproj.io/v1alpha1"]);
        Arc::new(core)
    }
    /// Answers the broker's CRD check as a cluster whose CRDs serve exactly these
    /// `{plural}.{group}/{version}` would.
    pub(super) fn serve_crds(core: &mut Registry, names: &'static [&'static str]) {
        let mut cap =
            crd::check_capability(srelens_kube::client_cache::ClientCache::new_many(vec![]));
        cap.handler = Arc::new(move |args| {
            Box::pin(async move {
                let name = format!(
                    "{}.{}/{}",
                    args["plural"].as_str().unwrap_or_default(),
                    args["group"].as_str().unwrap_or_default(),
                    args["version"].as_str().unwrap_or_default()
                );
                Ok(json!(names.contains(&name.as_str())))
            })
        });
        core.register(cap);
    }
    pub(super) fn install(path: &Path, core: Arc<Registry>) -> u64 {
        mutate(
            path,
            core,
            Configure::Install {
                signature: None,
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
        register(
            &mut reader,
            path.clone(),
            core.clone(),
            srelens_kube::client_cache::ClientCache::new_many(vec![]),
        );
        let args = json!({"id":"org.example.argocd","revision":revision,"capability":"applications","context":"staging","namespace":"argo"});
        let output = reader
            .invoke("extensions.read", args.clone())
            .await
            .unwrap();
        assert_eq!(output["group"], "argoproj.io");
        assert_eq!(output["context"], "staging");
        assert_eq!(output["namespace"], "argo");
        assert!(output.get("useCrdColumns").is_none());
        let mut column_args = args.clone();
        column_args["useCrdColumns"] = json!(true);
        let with_columns = reader.invoke("extensions.read", column_args).await.unwrap();
        assert_eq!(with_columns["useCrdColumns"], true);
        assert_eq!(with_columns["group"], "argoproj.io");
        assert_eq!(with_columns["namespace"], "argo");
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
                id: "org.example.argocd".into(),
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
                id: "org.example.argocd".into(),
                enabled: true,
            },
        )
        .unwrap();
        assert!(reader.invoke("extensions.read", args.clone()).await.is_ok());
        mutate(
            &path,
            core.clone(),
            Configure::Settings {
                id: "org.example.argocd".into(),
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
                id: "org.example.argocd".into(),
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
                    signature: None,
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
                    signature: None,
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
                json!({"action":"install","manifest":manifest(),"grants":["k8s.listCustomResource"]})
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
        for id in [
            "extensions.read",
            "extensions.catalog",
            "extensions.catalogManifest",
            "extensions.validate",
        ] {
            assert!(reg.get(id).unwrap().annotations.read_only);
        }
        let mcp = srelens_mcp::McpServer::new(Arc::new(reg));
        assert_eq!(mcp.list_tools().len(), 8);
        use srelens_mcp::{stdio::handle_request, Transport};
        for args in [
            json!({"action":"install","manifest":manifest(),"grants":["k8s.listCustomResource"]}),
            json!({"action":"install","manifest":manifest(),"grants":["k8s.listCustomResource"],"_confirm":true}),
        ] {
            let request = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"extensions.configure","arguments":args}});
            let denied = handle_request(&mcp, &request, Transport::Stdio)
                .await
                .unwrap();
            assert_eq!(denied["result"]["isError"], true, "{denied}");
        }
        assert_eq!(
            mcp.call_tool("extensions.list", json!({})).await.unwrap()["plugins"],
            json!([])
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
                            signature: None,
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
    fn find<'a>(state: &'a Inventory, id: &str) -> &'a Installed {
        state.plugins.iter().find(|p| p.manifest.id == id).unwrap()
    }
    #[tokio::test]
    async fn one_app_failing_reverification_is_quarantined_without_breaking_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        let core = fake_core();
        mutate(&path, core.clone(), signed_argocd()).unwrap();
        let local = install(&path, core.clone());
        let mut stored: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let signed = stored["plugins"]
            .as_array()
            .unwrap()
            .iter()
            .position(|p| p["manifest"]["id"] == "org.srelens.argocd")
            .unwrap();
        let byte = &mut stored["plugins"][signed]["signatureProof"]["signature"][0];
        *byte = json!(byte.as_u64().unwrap() ^ 1);
        fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();

        let state = read(&path).unwrap();
        let quarantined = find(&state, "org.srelens.argocd");
        assert!(!quarantined.enabled);
        let reason = quarantined.quarantined.clone().unwrap();
        assert!(reason.contains("signature"), "{reason}");
        let healthy = find(&state, "org.example.argocd");
        assert!(healthy.enabled && healthy.quarantined.is_none());

        let mut reg = Registry::new();
        register(
            &mut reg,
            path.clone(),
            core.clone(),
            srelens_kube::client_cache::ClientCache::new_many(vec![]),
        );
        let listed = reg.invoke("extensions.list", json!({})).await.unwrap();
        assert!(listed["plugins"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["quarantined"] == json!(reason)));
        let read_args = |id: &str, revision: u64| json!({"id":id,"revision":revision,"capability":"applications","context":"staging","namespace":"argo"});
        assert!(reg
            .invoke("extensions.read", read_args("org.example.argocd", local))
            .await
            .is_ok());
        assert!(reg
            .invoke(
                "extensions.read",
                read_args("org.srelens.argocd", quarantined.revision)
            )
            .await
            .is_err());
        let refused = mutate(
            &path,
            core.clone(),
            Configure::Enable {
                id: "org.srelens.argocd".into(),
                enabled: true,
            },
        )
        .err()
        .unwrap();
        assert!(refused.contains(&reason), "{refused}");

        // Saving another change persists the disable, never the computed reason.
        mutate(
            &path,
            core.clone(),
            Configure::Settings {
                id: "org.example.argocd".into(),
                settings: Default::default(),
            },
        )
        .unwrap();
        let saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(saved["plugins"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p.get("quarantined").is_none()));
        assert_eq!(saved["plugins"][signed]["enabled"], false);

        // Reinstalling the verified release clears the quarantine.
        mutate(&path, core, signed_argocd()).unwrap();
        let state = read(&path).unwrap();
        let restored = find(&state, "org.srelens.argocd");
        assert!(restored.enabled && restored.quarantined.is_none());
    }
    #[test]
    fn a_manifest_this_host_no_longer_accepts_is_quarantined_but_duplicates_stay_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        install(&path, fake_core());
        let mut stored: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        stored["plugins"][0]["manifest"]["srelensApiVersion"] = json!("^99");
        fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();
        let state = read(&path).unwrap();
        assert!(!state.plugins[0].enabled);
        assert!(state.plugins[0]
            .quarantined
            .as_deref()
            .unwrap()
            .contains("requires API"));
        let copy = stored["plugins"][0].clone();
        stored["plugins"].as_array_mut().unwrap().push(copy);
        fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();
        assert!(read(&path).err().unwrap().contains("duplicate"));
    }
    #[test]
    fn unsigned_installs_cannot_claim_the_reserved_official_namespace() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        let core = fake_core();
        let official = include_str!("../tests/fixtures/argocd-manifest.json");
        let unsigned = |manifest: &str| Configure::Install {
            signature: None,
            manifest: manifest.into(),
            grants: vec!["k8s.listCustomResource".into()],
        };
        let refused = mutate(&path, core.clone(), unsigned(official))
            .err()
            .unwrap();
        assert!(refused.contains("reserved"), "{refused}");
        assert!(read(&path).unwrap().plugins.is_empty());

        // A signed install cannot be replaced by an unsigned manifest under the same ID.
        mutate(&path, core.clone(), signed_argocd()).unwrap();
        let before = fs::read(&path).unwrap();
        assert!(mutate(&path, core.clone(), unsigned(official))
            .err()
            .unwrap()
            .contains("reserved"));
        assert_eq!(fs::read(&path).unwrap(), before);

        let lookalike = official.replace("\"org.srelens.argocd\"", "\"org.srelensx.argocd\"");
        assert!(mutate(&path, core, unsigned(&lookalike)).is_ok());
    }
    /// The saved inventory with the signature proof stripped from the app at `pointer`.
    fn strip_proof(path: &Path, pointer: &str) {
        let mut stored: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        let removed = stored
            .pointer_mut(pointer)
            .and_then(Value::as_object_mut)
            .unwrap()
            .remove("signatureProof");
        assert!(removed.is_some(), "{pointer} had no signature proof");
        fs::write(path, serde_json::to_vec(&stored).unwrap()).unwrap();
    }
    #[test]
    fn a_stored_unsigned_app_under_a_reserved_id_is_quarantined_and_cannot_be_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        let core = fake_core();
        mutate(&path, core.clone(), signed_argocd()).unwrap();
        install(&path, core.clone());
        // Signed and unsigned-elsewhere apps load as they were saved.
        let state = read(&path).unwrap();
        for id in ["org.srelens.argocd", "org.example.argocd"] {
            let app = find(&state, id);
            assert!(app.enabled && app.quarantined.is_none(), "{id}");
        }
        // As an entry saved before the namespace was reserved would be.
        let official = state
            .plugins
            .iter()
            .position(|p| p.manifest.id == "org.srelens.argocd")
            .unwrap();
        strip_proof(&path, &format!("/plugins/{official}"));

        let state = read(&path).unwrap();
        let stored = find(&state, "org.srelens.argocd");
        assert!(!stored.enabled);
        let reason = stored.quarantined.clone().unwrap();
        assert_eq!(
            reason,
            "App ID org.srelens.argocd is reserved for signed srelens releases"
        );
        let local = find(&state, "org.example.argocd");
        assert!(local.enabled && local.quarantined.is_none());

        let refused = mutate(
            &path,
            core.clone(),
            Configure::Enable {
                id: "org.srelens.argocd".into(),
                enabled: true,
            },
        )
        .err()
        .unwrap();
        assert!(refused.contains(&reason), "{refused}");
        assert!(
            refused.contains("reinstall it from the Catalog"),
            "{refused}"
        );

        // Reinstalling the signed release lifts it.
        mutate(&path, core, signed_argocd()).unwrap();
        let restored = read(&path).unwrap();
        let restored = find(&restored, "org.srelens.argocd");
        assert!(restored.enabled && restored.quarantined.is_none());
    }
    #[test]
    fn rollback_refuses_an_unsigned_version_under_a_reserved_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        let core = fake_core();
        mutate(&path, core.clone(), signed_argocd()).unwrap();
        mutate(&path, core.clone(), signed_argocd()).unwrap();
        strip_proof(&path, "/plugins/0/history/0");
        let state = read(&path).unwrap();
        let app = find(&state, "org.srelens.argocd");
        assert!(app.enabled && app.quarantined.is_none());
        let before = fs::read(&path).unwrap();
        let refused = mutate(
            &path,
            core,
            Configure::Rollback {
                id: "org.srelens.argocd".into(),
                revision: app.history[0].revision,
                grants: vec!["k8s.listCustomResource".into()],
            },
        )
        .err()
        .unwrap();
        assert!(
            refused.contains("reserved for signed srelens releases"),
            "{refused}"
        );
        assert_eq!(fs::read(&path).unwrap(), before);
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
