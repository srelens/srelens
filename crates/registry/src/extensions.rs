//! Durable, native declarative extensions for desktop hosts.
mod app_policy;
#[cfg(test)]
mod app_policy_tests;
mod app_settings;
#[cfg(test)]
mod budget_tests;
mod cards;
mod catalog;
mod columns;
pub(crate) mod crd;
#[cfg(test)]
mod executable_tests;
#[cfg(any(test, feature = "fuzzing"))]
pub mod fuzzing;
mod http_policy;
#[cfg(test)]
mod http_test_support;
pub mod inspector;
mod limits;
mod links;
pub(crate) mod network;
pub(crate) mod package;
#[cfg(test)]
mod package_tests;
mod panels;
pub mod pods;
mod providers;
#[cfg(test)]
mod pods_tests;
#[cfg(test)]
mod policy_tests;
mod resource;
mod secret_store;
pub use secret_store::{declares_secret_setting, SECRET_STORE_ANNOTATIONS};
#[cfg(test)]
mod secrets_tests;
#[cfg(test)]
mod settings_tests;
#[cfg(test)]
mod sidecar_tests;
pub mod sidecars;
mod signing;
mod store;
pub mod streams;
pub mod tools;
mod operations;
mod availability;
pub(crate) mod jobs;
#[cfg(test)]
mod tools_tests;
mod trust;
#[cfg(test)]
mod version_tests;
use app_settings::{checked_settings, drop_secret_values, setting_scope};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use srelens_capability::{Annotations, Capability, CapabilityError, Registry};
use srelens_plugin_host::{
    Manifest, PluginHost, ValidationCode as Code, ValidationError, ValidationErrors,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

pub use app_policy::{AppPolicy, SharedPolicy, MAX_POLICY_BYTES};
pub use catalog::SharedCatalog;
pub use store::{InventoryKey, InventoryLock, InventoryStore};
pub use trust::TrustRoot;

/// An inventory, as every capability that reads or changes one holds it: where it is kept,
/// and the root its apps' publisher signatures are verified against on every read.
#[derive(Clone)]
struct Store {
    at: Arc<dyn InventoryStore>,
    trust: TrustRoot,
}

impl Store {
    /// The inventory, every app's manifest and signature proof verified again, held to
    /// the policy in force (#578).
    fn read(&self) -> Result<Inventory, String> {
        read_under(&*self.at, &self.trust)
    }

    /// The inventory as saved, verified as [`Store::read`] verifies it, with no policy's
    /// verdicts applied: what `configure` changes and saves.
    fn read_saved(&self) -> Result<Inventory, String> {
        read_saved_under(&*self.at, &self.trust)
    }

    /// The administrator's policy this inventory is held to now, if any (#578).
    fn policy(&self) -> Option<Arc<AppPolicy>> {
        self.at.policy()
    }
}

#[cfg(test)]
impl Store {
    /// An inventory file under the root this build pins, as the tests name one.
    fn file(path: impl Into<PathBuf>) -> Self {
        Self {
            at: Arc::new(path.into()),
            trust: TrustRoot::pinned(),
        }
    }
}

impl std::ops::Deref for Store {
    type Target = dyn InventoryStore;
    fn deref(&self) -> &Self::Target {
        &*self.at
    }
}

/// Where one registry keeps its apps: the inventory, and the catalog cache that says
/// which installed versions are catalog releases.
#[derive(Clone)]
pub struct Apps {
    inventory: Store,
    catalog: catalog::CatalogCache,
    /// Where installed packages are unpacked (#562), one private directory per app.
    /// `None` on a host that keeps no files for its apps, which refuses to install a
    /// package and offers a catalog release's single-file manifest instead.
    packages: Option<PathBuf>,
    /// Where each app's sidecar data directory is kept (#573), one per app,
    /// named by `srelens_plugin_host::sidecar::data`. Apart from `packages`,
    /// whose pruning would remove it. `None` where no sidecar runs.
    data: Option<PathBuf>,
}

impl Apps {
    /// One web user's apps (#515): their own inventory, and the catalog every user of the
    /// server shares and none of them can write. The web host keeps no app files: a
    /// package's directory would be on the shared server, not the user's (#562). Their
    /// signatures are verified against the root the shared catalog is.
    pub fn with_shared_catalog(inventory: Arc<dyn InventoryStore>, catalog: SharedCatalog) -> Self {
        let catalog = catalog::CatalogCache::Shared(catalog);
        Self {
            inventory: Store {
                at: inventory,
                trust: catalog.trust().clone(),
            },
            catalog,
            packages: None,
            data: None,
        }
    }

    /// The root of the apps' data directories: where the host that starts an
    /// app's sidecar opens its [`srelens_plugin_host::sidecar::data::DataDir`].
    /// Whatever is there for an app that is no longer installed is removed with
    /// the next change to the inventory, so a later app with the same ID starts
    /// empty.
    pub fn data_root(&self) -> Option<&Path> {
        self.data.as_deref()
    }

    /// These apps, held to `policy` on every call (#578): each read of the inventory
    /// applies the policy in force then, so replacing it governs the next call.
    pub fn governed_by(self, policy: SharedPolicy) -> Self {
        Self {
            inventory: Store {
                at: Arc::new(app_policy::Governed {
                    inventory: self.inventory.at,
                    policy,
                }),
                trust: self.inventory.trust,
            },
            ..self
        }
    }

    /// Whether these apps are held to an administrator's policy whose ceiling names a
    /// host `network.http` may reach.
    pub(crate) fn has_network_ceiling(&self) -> bool {
        self.inventory
            .policy()
            .is_some_and(|policy| policy.reaches_network())
    }

    /// The desktop's layout at `path`, with its catalog and every app's signature verified
    /// against `trust` rather than the root this build pins: a test's root.
    pub fn with_trust(path: PathBuf, trust: TrustRoot) -> Self {
        Self {
            catalog: catalog::CatalogCache::Owned {
                path: path.with_extension("catalog.json"),
                trust: trust.clone(),
            },
            packages: Some(path.with_extension("packages")),
            data: Some(path.with_extension("data")),
            inventory: Store {
                at: Arc::new(path),
                trust,
            },
        }
    }
}

/// The desktop's layout: the inventory file, this host's own catalog cache beside it, the
/// directory installed packages are unpacked into beside that, and the apps' data, all
/// verified against the root this build pins.
impl From<PathBuf> for Apps {
    fn from(path: PathBuf) -> Self {
        Self::with_trust(path, TrustRoot::pinned())
    }
}

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
    /// Recomputed policy denial, distinct from a failed publisher verification.
    #[serde(
        default,
        rename = "policyBlocked",
        skip_serializing_if = "Option::is_none"
    )]
    policy_blocked: Option<String>,
    /// Who signed the installed version, when its proof verified on this read: "Signed by
    /// <name>". Recomputed on every read, like `quarantined`, and never written to disk.
    #[serde(default, rename = "signedBy", skip_serializing_if = "Option::is_none")]
    signed_by: Option<trust::Signer>,
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
    /// Whether the app's `network.http` requests may use plain HTTP to this computer
    /// (loopback), such as a Prometheus behind `kubectl port-forward` (#568). Off until
    /// a person turns it on for this app; kept across updates, as `contexts` is.
    #[serde(
        default,
        rename = "allowLoopbackHttp",
        skip_serializing_if = "std::ops::Not::not"
    )]
    allow_loopback_http: bool,
    /// The SHA-256 of the digest list of the package this version was unpacked from
    /// (#562), which names its directory; absent for a single-file manifest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    package: Option<String>,
    /// The package's logo as a `data:` URL, as `extensions.list` reports it: read from the
    /// package's files and checked against its digest list. Decoration only, never a sign
    /// of who published the app. Recomputed for every list, ignored when read from disk
    /// and never written there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    icon: Option<String>,
}
/// What the broker answers when an app is used on a cluster it is not enabled for.
const NOT_ENABLED_FOR_CLUSTER: &str = "App is not enabled for this cluster";
impl Installed {
    /// Whether the app is in use: on, and neither blocked by policy nor quarantined.
    fn runs(&self) -> bool {
        self.enabled && self.policy_blocked.is_none() && self.quarantined.is_none()
    }

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
    /// For a package (#562): the exact digest list `signature` covers, which names
    /// `manifest` as the package's `extension.json`. Absent for a single-file manifest,
    /// whose signature covers `manifest` itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    digests: Option<String>,
    /// The signed publisher delegation that vouched for the signature (#559), so the app
    /// is verified again on every load with no catalog at hand. Absent when this build's
    /// shipped delegations vouch for it, as they do for every release signed before
    /// #559, which keeps those inventories readable by the hosts that wrote them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    delegation: Option<trust::Envelope>,
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
    /// The package it was unpacked from, as [`Installed`] names it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    package: Option<String>,
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
    #[serde(default, rename = "allowUnsignedApps")]
    allow_unsigned_apps: bool,
    plugins: Vec<Installed>,
    /// Whether the host can store an app's secret now (#543), as
    /// `extensions.list` reports it. Recomputed for every answer, ignored when
    /// read from disk and never written there.
    #[serde(
        default,
        rename = "secretStore",
        skip_serializing_if = "Option::is_none"
    )]
    secret_store: Option<secret_store::SecretStoreState>,
    /// The administrator's policy this inventory is held to (#578), on a host that has
    /// one. Reported by every read, never read from disk and never written there.
    #[serde(default, skip_deserializing, skip_serializing_if = "Option::is_none")]
    policy: Option<AppPolicy>,
}

impl Default for Inventory {
    fn default() -> Self {
        Self {
            schema_version: 1,
            next_revision: 1,
            allow_unsigned_apps: false,
            plugins: vec![],
            secret_store: None,
            policy: None,
        }
    }
}
/// Recheck installed app authority for each native contribution read.
async fn resolver_app(
    inventory: Store,
    core: &Arc<Registry>,
    client_cache: &Arc<srelens_kube::client_cache::ClientCache>,
    id: &str,
    revision: u64,
    context: String,
) -> Result<(Inventory, usize, String), CapabilityError> {
    let app = resolve_app(inventory, core, client_cache, id, revision, context).await?;
    Ok((app.state, app.index, app.context))
}
/// An installed app that may answer on a cluster, as [`resolve_app`] found it.
struct ResolvedApp {
    state: Inventory,
    /// The app's position in `state.plugins`.
    index: usize,
    /// The context the request goes out under: its pinned ID when it resolved.
    context: String,
    /// The context as the host's kubeconfig files declare it, or why it did not
    /// resolve. A provider binds its name as `${cluster}` (#569).
    resolved: Result<srelens_kube::context_resolve::ResolvedContext, String>,
}
/// [`resolver_app`], keeping the resolved context.
async fn resolve_app(
    inventory: Store,
    core: &Arc<Registry>,
    client_cache: &Arc<srelens_kube::client_cache::ClientCache>,
    id: &str,
    revision: u64,
    context: String,
) -> Result<ResolvedApp, CapabilityError> {
    let resolved = request_context(client_cache, &context).await;
    let state = tokio::task::spawn_blocking(move || inventory.read())
        .await
        .map_err(|error| CapabilityError::Handler(error.to_string()))?
        .map_err(CapabilityError::Handler)?;
    let index = state
        .plugins
        .iter()
        .position(|plugin| plugin.manifest.id == id)
        .ok_or_else(|| CapabilityError::Handler("Extension was removed; refresh the view".into()))?;
    let plugin = &state.plugins[index];
    if let Some(reason) = &plugin.policy_blocked {
        return Err(CapabilityError::Handler(reason.clone()));
    }
    if !plugin.enabled || plugin.revision != revision {
        return Err(CapabilityError::Handler(
            "Extension was disabled or updated; refresh the view".into(),
        ));
    }
    plugin.check_scope(&resolved)?;
    validate_app(&plugin.manifest, &plugin.grants, core.clone())
        .map_err(|errors| CapabilityError::Handler(errors.to_string()))?;
    let context = resolved
        .as_ref()
        .ok()
        .and_then(|context| context.pinned_id())
        .unwrap_or(context);
    Ok(ResolvedApp {
        state,
        index,
        context,
        resolved,
    })
}
#[derive(Deserialize, JsonSchema)]
#[serde(tag = "action", deny_unknown_fields)]
enum Configure {
    #[serde(rename = "unsignedApps")]
    UnsignedApps {
        #[serde(rename = "allowUnsignedApps")]
        allow_unsigned_apps: bool,
    },
    #[serde(rename = "install")]
    Install {
        /// An Ed25519 signature: exactly 64 bytes.
        #[serde(default, deserialize_with = "limits::signature")]
        #[schemars(length(equal = 64))]
        signature: Option<Vec<u8>>,
        /// The key the signature names (#559): its ID, 64 lowercase hex characters.
        #[serde(default, rename = "keyId", deserialize_with = "limits::key_id")]
        #[schemars(length(equal = 64))]
        key_id: Option<String>,
        /// The manifest text: at most 256 KiB.
        #[serde(deserialize_with = "limits::manifest")]
        #[schemars(length(max = 262144))]
        manifest: String,
        grants: Vec<String>,
        /// Revision displayed by the host preview. An update must name it so a
        /// different version cannot be silently replaced after consent.
        #[serde(default, rename = "reviewedRevision")]
        reviewed_revision: Option<u64>,
    },
    /// Installs or updates from a package file (#562), sent as base64. The host reads and
    /// verifies the package again, whatever was reviewed; `grants` and `reviewedRevision`
    /// are the caller's consent, as for `install`.
    #[serde(rename = "installPackage")]
    InstallPackage {
        /// The `.srelens-extension` file as base64: at most 512 MiB once decoded.
        #[serde(deserialize_with = "limits::package")]
        #[schemars(with = "String")]
        package: Vec<u8>,
        grants: Vec<String>,
        #[serde(default, rename = "reviewedRevision")]
        reviewed_revision: Option<u64>,
    },
    /// Installs or updates a catalog release's package (#562), which the host downloads
    /// and verifies again. `sha256` names the release, as `extensions.catalogManifest`
    /// takes it; `packageSha256` is the exact package that was reviewed.
    #[serde(rename = "installCatalogPackage")]
    InstallCatalogPackage {
        id: String,
        sha256: String,
        #[serde(rename = "packageSha256")]
        package_sha256: String,
        grants: Vec<String>,
        #[serde(default, rename = "reviewedRevision")]
        reviewed_revision: Option<u64>,
    },
    #[serde(rename = "enable")]
    Enable { id: String, enabled: bool },
    #[serde(rename = "remove")]
    Remove { id: String },
    #[serde(rename = "settings")]
    Settings {
        id: String,
        /// At most 64 KiB as compact JSON.
        #[serde(deserialize_with = "limits::settings")]
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
    /// Lets the app's `network.http` requests use plain HTTP to this computer, or stops
    /// them (#568).
    #[serde(rename = "loopbackHttp")]
    LoopbackHttp {
        id: String,
        #[serde(rename = "allowLoopbackHttp")]
        allow_loopback_http: bool,
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
    /// A dashboard card's id: return only the rows that card counted (#540).
    #[serde(default)]
    #[schemars(length(max = 64))]
    card: Option<String>,
    /// With `card` and no `namespace`: the several namespaces the card counted
    /// in, so its target page shows exactly those rows (#540).
    #[serde(default)]
    namespaces: Vec<String>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Empty {}

/// [`read_under`] the root this build pins: for tests, which name an inventory by its file.
#[cfg(test)]
fn read<S: InventoryStore + ?Sized>(store: &S) -> Result<Inventory, String> {
    read_under(store, &TrustRoot::pinned())
}

/// The inventory in `store`, checked as every load checks it, each app's signature proof
/// against `trust`, and held to the policy in force on this host, if any (#578). What
/// every capability that uses an app reads.
fn read_under<S: InventoryStore + ?Sized>(
    store: &S,
    trust: &TrustRoot,
) -> Result<Inventory, String> {
    let mut state = read_saved_under(store, trust)?;
    app_policy::govern(&mut state, store.policy().as_deref());
    Ok(state)
}

/// The inventory as saved, checked as every load checks it, each app's manifest and
/// signature proof verified again against `trust`, but not yet held to a policy: what
/// `configure` changes and saves, so a policy's verdicts are never saved.
fn read_saved_under<S: InventoryStore + ?Sized>(
    store: &S,
    trust: &TrustRoot,
) -> Result<Inventory, String> {
    // One byte past the limit is enough to refuse it, so an oversized inventory is never
    // loaded whole.
    let Some(raw) = store.load(MAX_INVENTORY_BYTES)? else {
        return Ok(Inventory::default());
    };
    if raw.len() > MAX_INVENTORY_BYTES {
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
    // The store's state is the host's to report now, never the file's.
    state.secret_store = None;
    let mut ids = std::collections::BTreeSet::new();
    for plugin in &state.plugins {
        if !ids.insert(plugin.manifest.id.clone()) {
            return Err("duplicate installed extension".into());
        }
    }
    // One entry this host can no longer trust (a rotated key, a tampered proof, an API
    // version it dropped) is disabled on its own instead of failing every other app.
    for plugin in &mut state.plugins {
        drop_secret_values(plugin);
        // The logo is the host's to read from the package now, never the file's.
        plugin.icon = None;
        match reverify(plugin, trust) {
            Ok(signed_by) => {
                plugin.quarantined = None;
                plugin.signed_by = signed_by;
            }
            Err(reason) => {
                plugin.quarantined = Some(reason);
                plugin.signed_by = None;
                plugin.enabled = false;
            }
        }
    }
    apply_unsigned_policy(&mut state);
    Ok(state)
}

const UNSIGNED_POLICY_REASON: &str = "Turn on \"Allow unsigned apps to modify clusters and run code\" in Settings → Apps to enable this app";

/// Whether an unsigned app needs the unsigned-apps setting. Keep the kind match
/// exhaustive: a new kind cannot be added without deciding. Source labels and IDs grant
/// no trust.
///
/// A declarative app needs it when it writes (declared actions) or runs code in
/// the cluster (a `k8s.exec` binding, #567): a command can change whatever its
/// container may. An executable app (#574) always does, writes or not: it runs code
/// on this computer, sandboxed or not.
fn needs_unsigned_policy(manifest: &Manifest) -> bool {
    match manifest.kind {
        srelens_plugin_host::ManifestKind::Declarative => {
            !manifest.actions.is_empty()
                || manifest
                    .capabilities
                    .iter()
                    .any(|binding| binding.target == srelens_plugin_host::POD_EXEC)
        }
        srelens_plugin_host::ManifestKind::Executable => true,
    }
}
fn check_unsigned_policy(manifest: &Manifest, verified: bool, allow: bool) -> Result<(), String> {
    if !allow && !verified && needs_unsigned_policy(manifest) {
        return Err(UNSIGNED_POLICY_REASON.into());
    }
    Ok(())
}
fn apply_unsigned_policy(state: &mut Inventory) {
    for plugin in &mut state.plugins {
        plugin.policy_blocked = check_unsigned_policy(
            &plugin.manifest,
            plugin.signature_proof.is_some() && plugin.quarantined.is_none(),
            state.allow_unsigned_apps,
        )
        .err();
        if plugin.policy_blocked.is_some() {
            plugin.enabled = false;
        }
    }
}
/// Who signed a stored app, if anyone; an error when the host no longer trusts it.
fn reverify(plugin: &Installed, trust: &TrustRoot) -> Result<Option<trust::Signer>, String> {
    // An entry stored before the namespace was reserved, or added by hand, gets no more
    // trust from the file than an install would give it. Loading has no catalog at hand,
    // so the namespaces checked are the ones this build ships a delegation for.
    if let Some(reason) = unsigned_reserved(
        &plugin.manifest.id,
        plugin.signature_proof.is_some(),
        &trust.shipped(),
    ) {
        return Err(reason);
    }
    plugin.manifest.validate()?;
    crd::group_problems(&plugin.manifest).into_result()?;
    check_package_name(plugin.package.as_deref())?;
    // Its binaries are in its package, and there is none to run them from (#574).
    if plugin.manifest.sidecar.is_some() && plugin.package.is_none() {
        return Err("Installed executable app has no package to run its sidecar from".into());
    }
    plugin
        .signature_proof
        .as_ref()
        .map(|proof| verify_proof(proof, &plugin.manifest, plugin.package.as_deref(), trust))
        .transpose()
}
/// A package version is named by a SHA-256, as its directory is.
fn check_package_name(package: Option<&str>) -> Result<(), String> {
    if package.is_some_and(|package| {
        package.len() != 64
            || !package
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }) {
        return Err("Installed app names its package by something other than a SHA-256".into());
    }
    Ok(())
}
/// Who signed the kept bytes, and that those bytes are `manifest`: for a single-file
/// manifest, the manifest's own; for a package (#562), the digest list, which names the
/// manifest and is the one the version was `unpacked` as. The publisher is the one the
/// proof's own delegation names, verified under `trust`; a proof with none is verified
/// under the delegations this build ships.
fn verify_proof(
    proof: &SignatureProof,
    manifest: &Manifest,
    unpacked: Option<&str>,
    trust: &TrustRoot,
) -> Result<trust::Signer, String> {
    if let Some(reason) = trust.unavailable() {
        return Err(reason);
    }
    let delegations = match &proof.delegation {
        // A delegation this build ships for the same publisher at the same or a later
        // version replaces the kept one, so a key a newer delegation withdrew stops
        // vouching for what it signed.
        Some(delegation) => trust::Delegations::merged(
            &trust.shipped(),
            &trust::Delegations::new(vec![trust.publisher(delegation)?])?,
        ),
        None => trust.shipped(),
    };
    let signer = match &proof.digests {
        None => signing::verify(
            proof.manifest.as_bytes(),
            &proof.signature,
            None,
            &delegations,
        )?,
        Some(digests) => {
            package::verify_signed(digests, &proof.signature, &proof.manifest, &delegations)?
        }
    };
    let proven = proof
        .digests
        .as_deref()
        .map(|digests| package::sha256_hex(digests.as_bytes()));
    if proven.as_deref() != unpacked {
        return Err("Installed app does not match its signed package".into());
    }
    let parsed = Manifest::parse(&proof.manifest)?;
    if serde_json::to_value(parsed).map_err(|e| e.to_string())?
        != serde_json::to_value(manifest).map_err(|e| e.to_string())?
    {
        return Err("Installed app does not match its signed manifest".into());
    }
    Ok(signer)
}
/// The largest inventory `write` saves, measured in its saved form.
const MAX_INVENTORY_BYTES: usize = 1024 * 1024;
/// The inventory exactly as `write` saves it.
fn saved_form(state: &Inventory) -> Result<Vec<u8>, String> {
    // The one place every save passes: a secret setting holding anything but
    // its reference is refused here, whichever path put it there, so a secret
    // value cannot reach the file (#542, #543).
    if let Some(problem) = state
        .plugins
        .iter()
        .flat_map(|plugin| plugin.manifest.stored_secret_problems(&plugin.settings))
        .next()
    {
        return Err(format!(
            "refusing to save the extension inventory: {problem}"
        ));
    }
    // Quarantine is recomputed on every load. Persisting it would also make the file
    // unreadable to hosts that predate the field.
    let mut stored = serde_json::to_value(state).map_err(|e| e.to_string())?;
    if let Some(fields) = stored.as_object_mut() {
        fields.remove("secretStore");
        fields.remove("policy");
    }
    if let Some(plugins) = stored.get_mut("plugins").and_then(Value::as_array_mut) {
        for plugin in plugins.iter_mut().filter_map(Value::as_object_mut) {
            plugin.remove("quarantined");
            plugin.remove("policyBlocked");
            plugin.remove("icon");
            plugin.remove("signedBy");
        }
    }
    serde_json::to_vec_pretty(&stored).map_err(|e| e.to_string())
}
fn write<S: InventoryStore + ?Sized>(store: &S, state: &Inventory) -> Result<(), String> {
    // A policy's verdicts are its own, never the user's: whatever path reached here,
    // an inventory a policy was applied to is refused, and the change goes back to
    // `Store::read_saved` (#578).
    if state.policy.is_some() {
        return Err("refusing to save an inventory held to a policy's verdicts".into());
    }
    let raw = saved_form(state)?;
    if raw.len() > MAX_INVENTORY_BYTES {
        return Err("extension inventory exceeds 1 MiB".into());
    }
    store.save(&raw)
}
/// The `k8s.listCustomResource` input the host fills from `statusResolvers`.
const STATUS_RULES_ARGUMENT: &str = "statusRules";

/// The manifest's own rules and this app's narrower ones, reporting every violation.
fn validate_app(
    manifest: &Manifest,
    grants: &[String],
    core: Arc<Registry>,
) -> Result<(), ValidationErrors> {
    let mut problems = manifest.validate().err().unwrap_or_default();
    for (index, binding) in manifest.capabilities.iter().enumerate() {
        let at = format!("capabilities[{index}]");
        // Brokered HTTP (#568): a request to one of the app's granted hosts, checked
        // by its own rules rather than a reader's.
        if binding.target == srelens_plugin_host::NETWORK_HTTP {
            if core.get(&binding.target).is_none() {
                problems.push(
                    Code::UnsupportedTarget,
                    format!("{at}.target"),
                    "This host does not provide network.http",
                );
            } else {
                network::binding_problems(manifest, index, binding, &mut problems);
            }
            continue;
        }
        // Logs, exec and port-forwards (#567): their scope and what they run are the
        // manifest's own rules; here, only whether this host provides them.
        if srelens_plugin_host::is_pod_target(&binding.target) {
            if core.get(&binding.target).is_none() {
                problems.push(
                    Code::UnsupportedTarget,
                    format!("{at}.target"),
                    format!("This host does not provide {}", binding.target),
                );
            }
            continue;
        }
        if binding.target == "k8s.runJob" {
            if manifest.kind != srelens_plugin_host::ManifestKind::Executable || core.get(&binding.target).is_none() {
                problems.push(Code::UnsupportedTarget,format!("{at}.target"),"Jobs require an executable app and a host that provides k8s.runJob");
            }
            if !binding.inputs.is_empty() || !binding.versions.is_empty() || !binding.json_path_overrides.is_empty() {
                problems.push(Code::InvalidBinding,format!("{at}.inputs"),"Job bindings fix their template; caller values use declared inputNames");
            }
            continue;
        }
        let builtin = srelens_plugin_host::builtin_reader_identity(&binding.target);
        if builtin.is_none()
            && !matches!(
                binding.target.as_str(),
                "k8s.listCustomResource" | "k8s.listEvents" | "k8s.listWorkloadImages"
            )
        {
            problems.push(
                Code::UnsupportedTarget,
                format!("{at}.target"),
                "This host supports custom-resource, event, and narrow workload/node summary readers",
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
        if binding.target == "k8s.listWorkloadImages" {
            if binding.arguments.len() != 1 || !matches!(binding.arguments.get("kind").and_then(Value::as_str), Some("Deployment" | "StatefulSet" | "DaemonSet"))
                || !accepts("context") || !accepts("namespace") {
                problems.push(Code::InvalidBinding, format!("{at}.arguments"), "A workload image reader fixes kind to Deployment, StatefulSet or DaemonSet and accepts context plus namespace only");
            }
            continue;
        }
        if let Some(identity) = builtin {
            let namespaced = identity["namespaced"] == true;
            if !binding.arguments.is_empty()
                || !accepts("context")
                || accepts("namespace") != namespaced
            {
                problems.push(Code::InvalidBinding, format!("{at}.arguments"),
                    "Built-in summary readers take no bound arguments and accept context plus namespace only for workloads");
            }
            continue;
        }
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
        // A reader that lists `versions` binds none here; the manifest's own rules hold
        // each listed one to the same characters (#547).
        for key in ["group", "version", "plural", "kind"] {
            if key == "version" && !binding.versions.is_empty() {
                continue;
            }
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
        // The host copies a kind's `statusResolvers` rules into the read it
        // sends; a second spelling in the binding would leave two rule lists
        // for one kind and the reader picking whichever it looked at.
        if binding.arguments.contains_key(STATUS_RULES_ARGUMENT) {
            problems.push(
                Code::InvalidBinding,
                format!("{at}.arguments.{STATUS_RULES_ARGUMENT}"),
                "Status rules are declared in contributions.statusResolvers",
            );
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
    // A declared action binds a host action primitive (#549) and nothing else.
    // The readers above and these are separate grants: `k8s.annotate` in
    // `permissions` buys an action, never a reader, and the reverse.
    for (index, action) in manifest.actions.iter().enumerate() {
        let at = format!("actions[{index}].target");
        if !srelens_kube::action_primitives::PRIMITIVES.contains(&action.target.as_str()) {
            problems.push(
                Code::UnsupportedTarget,
                at,
                format!(
                    "An action binds a host action primitive: {}",
                    srelens_kube::action_primitives::PRIMITIVES.join(", ")
                ),
            );
            continue;
        }
        if manifest
            .capabilities
            .iter()
            .find(|b| b.name == action.resource)
            .is_some_and(|b| srelens_plugin_host::builtin_reader_identity(&b.target).is_some())
            && !matches!(
                action.target.as_str(),
                "k8s.requestRolloutRestart" | "k8s.requestCordonNode"
            )
        {
            problems.push(
                Code::UnsupportedTarget,
                at.clone(),
                "Built-in readers scope only reviewed workload restart or node cordon actions",
            );
        }
        if core.get(&action.target).is_none() {
            problems.push(
                Code::UnsupportedTarget,
                at,
                "This host does not provide the action primitive",
            );
        }
    }
    // A badge sits on a row of a built-in table the host lists itself. The
    // kind must be one this host reads in exactly that group, and never a
    // Secret, whose metadata the host redacts on every ungated read.
    for (index, badge) in manifest.contributions.badges.iter().enumerate() {
        for (position, kind) in badge.for_kinds.iter().enumerate() {
            let Some((group, name)) = kind.split_once('/') else {
                continue; // Reported by the manifest's own rules.
            };
            let builtin = srelens_kube::manifest::gvk_for(name)
                .is_some_and(|(gvk, _)| gvk.group == group && gvk.kind == name);
            if !builtin || (group.is_empty() && name == "Secret") {
                problems.push(
                    Code::UnsupportedTarget,
                    format!("contributions.badges[{index}].forKinds[{position}]"),
                    format!("{kind} is not a built-in kind this host badges"),
                );
            }
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
        if !grants.iter().any(|grant| permission == grant.as_str()) {
            problems.push(
                Code::PermissionMismatch,
                "permissions",
                format!("{} was not granted", permission.capability()),
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
            .binding_problems(index, manifest, binding)
            .into_iter()
            .filter(|found| !problems.0.iter().any(|p| p.path == found.path))
            .collect();
        problems.0.extend(found);
    }
    // And each declared action against the primitive it names, which is where
    // the primitive's own rules for a bound template are applied.
    for (index, action) in manifest.actions.iter().enumerate() {
        let target = format!("actions[{index}].target");
        if problems
            .0
            .iter()
            .any(|p| p.code == Code::UnsupportedTarget && p.path == target)
        {
            continue;
        }
        let found: Vec<_> = host
            .action_problems(index, manifest, action)
            .into_iter()
            .filter(|found| !problems.0.iter().any(|p| p.path == found.path))
            .collect();
        problems.0.extend(found);
    }
    problems.into_result()
}
/// [`validate_app`] with every permission granted, for the MCP catalog's own check that
/// it lists every reader an app may bind.
#[cfg(test)]
pub(crate) fn validate_app_for_tests(
    manifest: &Manifest,
    core: Arc<Registry>,
) -> Result<(), ValidationErrors> {
    validate_app(manifest, &manifest.permission_names(), core)
}
/// Why an app under `id` cannot be trusted without a publisher signature, when it has none
/// and `id` is in a namespace delegated to a publisher. Install refuses it; loading
/// quarantines a stored one, which `enable` then refuses; rollback refuses to restore one.
fn unsigned_reserved(id: &str, signed: bool, delegations: &trust::Delegations) -> Option<String> {
    let publisher = delegations.owner(id).filter(|_| !signed)?;
    Some(format!(
        "App ID {id} is reserved for releases signed by {}",
        publisher.name
    ))
}
/// A publisher signature an install verified, and the delegation to keep as its evidence.
#[derive(Debug)]
struct Signed {
    signer: trust::Signer,
    /// `None` when this build's shipped delegations vouch for it without one (see
    /// [`SignatureProof::delegation`]).
    delegation: Option<trust::Envelope>,
}
/// Every reason installing `source` with these grants and signature would be refused.
/// With `digests`, `source` is a package's `extension.json` (#562), the list must name it,
/// and `signature` is over the list.
fn check_install(
    source: &str,
    grants: &[String],
    signature: Option<(&[u8], Option<&str>)>,
    digests: Option<&str>,
    authority: &Result<catalog::Authority, String>,
    trust: &TrustRoot,
    core: Arc<Registry>,
) -> Result<(Manifest, Option<Signed>), ValidationErrors> {
    let manifest = Manifest::decode(source)?;
    let mut problems = validate_app(&manifest, grants, core)
        .err()
        .unwrap_or_default();
    // The rules a new install meets that an installed app is not re-held to.
    problems.0.extend(manifest.install_problems());
    // An executable app's binaries come in its package (#574).
    package::binary_problems(&manifest, digests, &mut problems);
    // With no root to verify against, which IDs are reserved for signed publishers is
    // unknown, so nothing is installed: an unsigned app could otherwise take a publisher's
    // ID and replace its signed installation.
    let authority = match authority {
        Ok(authority) => authority,
        Err(reason) => {
            problems.push(Code::ReservedId, "id", format!("{NO_ROOT}: {reason}"));
            return Err(problems);
        }
    };
    // Without this, a pasted manifest could replace a signed app, or take a publisher's
    // ID and its logo, differing from the real one only by a label.
    if let Some(reason) = unsigned_reserved(&manifest.id, signature.is_some(), &authority.reserved)
    {
        problems.push(
            Code::ReservedId,
            "id",
            format!(
                "{reason}. Install it from the Catalog, or give your local manifest its own ID."
            ),
        );
    }
    // The signature covers the exact bytes, so it is checked whatever else is wrong.
    let mut signed = None;
    match (signature, digests) {
        (None, None) => {}
        (None, Some(digests)) => {
            if let Err(reason) = package::check_manifest_listed(digests, source) {
                problems.push(Code::InvalidSignature, "", reason);
            }
        }
        (Some((signature, key_id)), digests) => {
            // A single-file manifest's signature covers the manifest; a package's (#562)
            // covers its digest list, which names the manifest. Either way the key is
            // looked up by the app's ID, under the same delegations.
            let check = |delegations: &trust::Delegations, key_id: Option<&str>| match digests {
                None => signing::verify_for(
                    &manifest.id,
                    source.as_bytes(),
                    signature,
                    key_id,
                    delegations,
                ),
                Some(digests) => package::verify_signed(digests, signature, source, delegations),
            };
            let known = &authority.signers;
            let verified = check(known, key_id).and_then(|signer| {
                let publisher = known
                    .owner(&manifest.id)
                    .ok_or("the signing publisher vanished")?;
                // Evidence only where this build's own delegations could not vouch for it
                // on the next load.
                let shipped = check(&trust.shipped(), None);
                Ok(Signed {
                    signer,
                    delegation: shipped.is_err().then(|| publisher.envelope.clone()),
                })
            });
            match verified {
                Ok(verified) => signed = Some(verified),
                Err(reason) => problems.push(
                    Code::InvalidSignature,
                    "",
                    match &authority.expired {
                        Some(expired) => format!("{reason} ({expired})"),
                        None => reason,
                    },
                ),
            }
        }
    }
    problems.into_result()?;
    Ok((manifest, signed))
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
/// A version about to be installed, with everything install checks already checked.
struct Incoming {
    manifest: Manifest,
    grants: Vec<String>,
    reviewed_revision: Option<u64>,
    signature_proof: Option<SignatureProof>,
    /// Who signed it, as the install verified it (#559).
    signed_by: Option<trust::Signer>,
    source: Source,
    package: Option<String>,
}
/// Installs `incoming`, or updates the app it replaces, which must be the revision the
/// caller reviewed, and one `policy` allows (#578): every way of installing comes here.
fn install(
    state: &mut Inventory,
    incoming: Incoming,
    policy: Option<&AppPolicy>,
) -> Result<(), String> {
    let Incoming {
        manifest,
        grants,
        reviewed_revision,
        signature_proof,
        signed_by,
        source: origin,
        package,
    } = incoming;
    // A signer is known only for a signature `check_install` verified.
    let publisher = signed_by.as_ref().map(|signer| signer.id.as_str());
    if let Some(refusal) = policy.and_then(|policy| policy.refusal(&manifest, publisher)) {
        return Err(refusal.reason);
    }
    let current_revision = state
        .plugins
        .iter()
        .find(|app| app.manifest.id == manifest.id)
        .map(|app| app.revision);
    if current_revision != reviewed_revision {
        return Err("App changed since permission review; review this update again".into());
    }
    let revision = take_revision(state)?;
    let previous = state
        .plugins
        .iter()
        .position(|p| p.manifest.id == manifest.id)
        .map(|i| state.plugins.remove(i));
    // An update keeps the app's settings and clusters, and the version it replaces
    // for rollback.
    let (settings, history, contexts, allow_loopback_http) = match previous {
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
            allow_loopback_http,
            package: replaced_package,
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
                    package: replaced_package,
                },
            );
            history.truncate(KEPT_VERSIONS);
            // Only what the new version still declares, and still
            // accepts, carries over (#542).
            (
                manifest.retain_settings(settings),
                history,
                contexts,
                allow_loopback_http,
            )
        }
        None => (Default::default(), Vec::new(), None, false),
    };
    state.plugins.push(Installed {
        signature_proof,
        quarantined: None,
        policy_blocked: None,
        signed_by,
        manifest,
        grants,
        enabled: true,
        revision,
        settings,
        source: origin,
        installed_at: now(),
        history,
        contexts,
        allow_loopback_http,
        package,
        icon: None,
    });
    state
        .plugins
        .sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
    // Kept versions give way, oldest first, before the saved inventory outgrows its
    // limit; the update itself is not refused. Other apps' versions are left alone.
    while saved_form(state)?.len() > MAX_INVENTORY_BYTES {
        let updated = state
            .plugins
            .iter_mut()
            .find(|p| p.revision == revision)
            .ok_or("the updated app is missing from the inventory")?;
        if updated.history.pop().is_none() {
            break;
        }
    }
    Ok(())
}
/// Why a host with no directory for its apps' files refuses a package.
const NO_PACKAGES: &str = "This host keeps no files for its apps, so it cannot install a package; install the app's single-file manifest instead";
/// Installs the package `archive` (#562): read and verified whole, checked as installing
/// its manifest would be (with the signature over its digest list), then unpacked into
/// its private directory. The inventory's save that follows is what installs it.
/// Why nothing installs on a host with no usable root: which app IDs are reserved for
/// signed publishers is unknown, so an unsigned app could take one.
const NO_ROOT: &str =
    "This host cannot tell which app IDs are reserved for signed publishers, so it installs nothing";
/// The package `archive`, read under the publishers `authority` trusts, or refused for the
/// cause its failure has: the missing root when there is none, as `check_install` says, and
/// the expired catalog when only that catalog's delegations would have verified it. A
/// failed trust lookup is not reported as a fact about the package.
fn read_package(
    archive: &[u8],
    authority: &Result<catalog::Authority, String>,
) -> Result<package::Package, String> {
    let authority = authority
        .as_ref()
        .map_err(|reason| format!("{NO_ROOT}: {reason}"))?;
    package::read(archive, &mut package::Discard, &authority.signers).map_err(|reason| {
        match &authority.expired {
            Some(expired)
                if package::read(archive, &mut package::Discard, &authority.reserved).is_ok() =>
            {
                format!("{reason} ({expired})")
            }
            _ => reason,
        }
    })
}
fn install_package(
    apps: &Apps,
    state: &mut Inventory,
    core: Arc<Registry>,
    archive: &[u8],
    grants: Vec<String>,
    reviewed_revision: Option<u64>,
    policy: Option<&AppPolicy>,
) -> Result<(), String> {
    let root = apps.packages.as_deref().ok_or(NO_PACKAGES)?;
    let authority = apps.catalog.authority();
    let verified = read_package(archive, &authority)?;
    package::check_installable(&verified)?;
    let (manifest, signed) = check_install(
        &verified.manifest,
        &grants,
        verified
            .signature
            .as_deref()
            .map(|signature| (signature, None)),
        Some(&verified.digests),
        &authority,
        &apps.inventory.trust,
        core,
    )?;
    check_unsigned_policy(
        &manifest,
        verified.signature.is_some(),
        state.allow_unsigned_apps,
    )?;
    let origin = if apps.catalog.lists_package(&manifest.id, &verified.sha256) {
        Source::Catalog
    } else {
        Source::Local
    };
    let signature_proof = verified.signature.clone().map(|signature| SignatureProof {
        manifest: verified.manifest.clone(),
        signature,
        digests: Some(verified.digests.clone()),
        delegation: signed.as_ref().and_then(|signed| signed.delegation.clone()),
    });
    install(
        state,
        Incoming {
            manifest,
            grants,
            reviewed_revision,
            signature_proof,
            signed_by: signed.map(|signed| signed.signer),
            source: origin,
            package: Some(verified.digest.clone()),
        },
        policy,
    )?;
    // Last, once nothing else can refuse the install. `read_package` has refused every
    // package already when there is no authority.
    let signers = &authority.as_ref().map_err(Clone::clone)?.signers;
    package::unpack(root, archive, &verified, signers)
}
/// Every package version each app keeps, current and for rollback, by app ID.
fn kept_packages(
    state: &Inventory,
) -> std::collections::BTreeMap<String, std::collections::BTreeSet<String>> {
    state
        .plugins
        .iter()
        .map(|app| {
            let versions = app
                .package
                .iter()
                .chain(
                    app.history
                        .iter()
                        .filter_map(|version| version.package.as_ref()),
                )
                .cloned()
                .collect();
            (app.manifest.id.clone(), versions)
        })
        .collect()
}
/// Each installed package's logo, for `extensions.list`, read from its files and checked
/// against its digest list. A quarantined app shows none: a logo is decoration, and a
/// familiar one beside an app the host no longer trusts would say otherwise.
fn report_icons(packages: Option<&Path>, state: &mut Inventory) {
    let Some(root) = packages else {
        return;
    };
    for plugin in &mut state.plugins {
        let Some(digest) = plugin
            .package
            .as_deref()
            .filter(|_| plugin.quarantined.is_none())
        else {
            continue;
        };
        match package::installed_icon(root, &plugin.manifest.id, digest) {
            Ok(icon) => plugin.icon = icon,
            Err(reason) => log::warn!("{}: no logo shown: {reason}", plugin.manifest.id),
        }
    }
}
/// [`configure`] on the desktop's layout with no secret store, for the lifecycle tests
/// that name an inventory by its file (see [`Apps`]'s `From<PathBuf>`).
#[cfg(test)]
fn mutate(path: &Path, core: Arc<Registry>, input: Configure) -> Result<Inventory, String> {
    configure(
        &Apps::from(path.to_path_buf()),
        core,
        &srelens_plugin_host::NoSecretStore,
        input,
    )
}
/// `extensions.configure`: one read-modify-write of `apps`'s inventory, under its lock,
/// then the host's secret store made to follow the inventory: whatever the change
/// dropped — an app, a setting an update no longer declares as a secret — is deleted
/// from the store (#543).
fn configure(
    apps: &Apps,
    core: Arc<Registry>,
    secrets: &dyn srelens_plugin_host::SecretStore,
    input: Configure,
) -> Result<Inventory, String> {
    // A catalog package is downloaded before the inventory is locked: every other change
    // waits on that lock, and a download may take the whole of its timeout.
    let downloaded = match &input {
        Configure::InstallCatalogPackage {
            id,
            sha256,
            package_sha256,
            ..
        } => {
            apps.packages.as_ref().ok_or(NO_PACKAGES)?;
            Some(apps.catalog.download_package(id, sha256, package_sha256)?)
        }
        _ => None,
    };
    let store = &apps.inventory;
    let _lock = store.lock()?;
    // The inventory as saved: the policy's verdicts are applied to the answer, never
    // saved, so lifting a policy restores each app as its user left it (#578).
    let mut state = store.read_saved()?;
    // The apps installed now, so that those this change uninstalls can be told
    // apart afterwards (their AppContainer profiles, on Windows).
    #[cfg(windows)]
    let installed_before: Vec<String> = state
        .plugins
        .iter()
        .map(|app| app.manifest.id.clone())
        .collect();
    let policy = store.policy();
    // Held to the publisher that signed it, as verified now: none for an unsigned app.
    let refusal = |manifest: &Manifest, signer: Option<&trust::Signer>| -> Result<(), String> {
        let publisher = signer.map(|signer| signer.id.as_str());
        match policy
            .as_deref()
            .and_then(|policy| policy.refusal(manifest, publisher))
        {
            Some(refusal) => Err(refusal.reason),
            None => Ok(()),
        }
    };
    let required = |id: &str, change: &str| -> Result<(), String> {
        if policy.as_deref().is_some_and(|policy| policy.requires(id)) {
            return Err(format!(
                "The administrator's policy requires {id}, so it can't be {change}"
            ));
        }
        Ok(())
    };
    match input {
        Configure::UnsignedApps {
            allow_unsigned_apps,
        } => {
            // Turning it off disables each unsigned app that writes, and a required
            // one may not be disabled (#578).
            if !allow_unsigned_apps {
                for app in &state.plugins {
                    let verified = app.signature_proof.is_some() && app.quarantined.is_none();
                    if check_unsigned_policy(&app.manifest, verified, false).is_err() {
                        required(&app.manifest.id, "disabled")?;
                    }
                }
            }
            state.allow_unsigned_apps = allow_unsigned_apps;
        }
        Configure::Install {
            manifest: source,
            grants,
            signature,
            key_id,
            reviewed_revision,
        } => {
            let (manifest, signed) = check_install(
                &source,
                &grants,
                signature
                    .as_deref()
                    .map(|signature| (signature, key_id.as_deref())),
                None,
                &apps.catalog.authority(),
                &store.trust,
                core,
            )?;
            check_unsigned_policy(&manifest, signature.is_some(), state.allow_unsigned_apps)?;
            let checksum = package::sha256_hex(source.as_bytes());
            let origin = if apps.catalog.lists_release(&manifest.id, &checksum) {
                Source::Catalog
            } else {
                Source::Local
            };
            let signature_proof = signature.map(|signature| SignatureProof {
                manifest: source,
                signature,
                digests: None,
                delegation: signed.as_ref().and_then(|signed| signed.delegation.clone()),
            });
            install(
                &mut state,
                Incoming {
                    manifest,
                    grants,
                    reviewed_revision,
                    signature_proof,
                    signed_by: signed.map(|signed| signed.signer),
                    source: origin,
                    package: None,
                },
                policy.as_deref(),
            )?;
        }
        Configure::InstallPackage {
            package,
            grants,
            reviewed_revision,
        } => install_package(
            apps,
            &mut state,
            core,
            &package,
            grants,
            reviewed_revision,
            policy.as_deref(),
        )?,
        Configure::InstallCatalogPackage {
            grants,
            reviewed_revision,
            ..
        } => {
            let archive = downloaded.ok_or("The catalog package was not downloaded")?;
            install_package(
                apps,
                &mut state,
                core,
                &archive,
                grants,
                reviewed_revision,
                policy.as_deref(),
            )?;
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
            let authority = apps.catalog.authority().map_err(|reason| {
                format!("This host cannot tell which app IDs are reserved for signed publishers, so it restores nothing: {reason}")
            })?;
            if let Some(reason) = unsigned_reserved(
                &target.manifest.id,
                target.signature_proof.is_some(),
                &authority.reserved,
            ) {
                return Err(format!("{reason}. Reinstall it from the Catalog."));
            }
            let signed_by = target
                .signature_proof
                .as_ref()
                .map(|proof| {
                    verify_proof(
                        proof,
                        &target.manifest,
                        target.package.as_deref(),
                        &store.trust,
                    )
                })
                .transpose()?;
            validate_app(&target.manifest, &grants, core)?;
            check_unsigned_policy(
                &target.manifest,
                target.signature_proof.is_some(),
                state.allow_unsigned_apps,
            )?;
            refusal(&target.manifest, signed_by.as_ref())?;
            // Going back discards the versions after the restored one.
            app.history.drain(..=index);
            app.signature_proof = target.signature_proof;
            // Settings are kept as an update keeps them: what the restored
            // version declares and accepts (#542).
            app.settings = target
                .manifest
                .retain_settings(std::mem::take(&mut app.settings));
            app.manifest = target.manifest;
            app.grants = grants;
            app.source = target.source;
            app.package = target.package;
            app.installed_at = target.installed_at;
            app.quarantined = None;
            app.signed_by = signed_by;
            // A new revision, so views pinned to the rolled-away version refresh.
            app.revision = next;
        }
        Configure::LoopbackHttp {
            id,
            allow_loopback_http,
        } => {
            let app = state
                .plugins
                .iter_mut()
                .find(|p| p.manifest.id == id)
                .ok_or("Extension is not installed")?;
            if allow_loopback_http && app.manifest.network_hosts().is_empty() {
                return Err(format!(
                    "{id} does not request {}",
                    srelens_plugin_host::NETWORK_HTTP
                ));
            }
            // Under a policy, this computer is the host every user shares: its loopback
            // is not one person's to open to an app.
            if allow_loopback_http && policy.is_some() {
                return Err(
                    "The administrator's policy allows network.http over HTTPS only".into(),
                );
            }
            app.allow_loopback_http = allow_loopback_http;
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
                check_unsigned_policy(
                    &p.manifest,
                    p.signature_proof.is_some(),
                    state.allow_unsigned_apps,
                )?;
                refusal(&p.manifest, p.signed_by.as_ref())?;
                validate_app(&p.manifest, &p.grants, core)?;
            } else {
                required(&id, "disabled")?;
            }
            p.enabled = enabled;
        }
        Configure::Remove { id } => {
            let i = state
                .plugins
                .iter()
                .position(|p| p.manifest.id == id)
                .ok_or("Extension is not installed")?;
            required(&id, "removed")?;
            state.plugins.remove(i);
        }
        Configure::Settings { id, settings } => {
            let app = state
                .plugins
                .iter_mut()
                .find(|p| p.manifest.id == id)
                .ok_or("Extension is not installed")?;
            app.settings = checked_settings(&app.manifest, &app.settings, settings, core)?;
        }
    }
    apply_unsigned_policy(&mut state);
    write(&*store.at, &state)?;
    // Under the same lock as every install, so no version is removed while one unpacks.
    if let Some(root) = &apps.packages {
        package::prune(root, &kept_packages(&state));
    }
    // Its sidecar first, so nothing still runs out of the directories below (#574).
    streams::end_uninstalled_sidecars(&store.key(), &state);
    // And the data an uninstalled app's sidecar kept (#573). Best effort, as the
    // packages are: a directory that cannot be removed now is tried again with the
    // next change.
    if let Some(root) = &apps.data {
        let installed: Vec<&str> = state.plugins.iter().map(|app| app.manifest.id.as_str()).collect();
        if let Err(error) = srelens_plugin_host::sidecar::data::prune(root, &installed) {
            log::warn!(
                "could not remove an uninstalled app's data under {}: {error}",
                root.display()
            );
        }
    }
    // And, on Windows, an uninstalled app's AppContainer profile (#573): its folder
    // and its registry storage, both of which its sidecar could write outside its
    // data directory. Best effort, as the rest is. The app's sidecar was ended above
    // (#574); one a call still held may outlive this, and its profile goes with the
    // next change.
    #[cfg(windows)]
    for id in uninstalled(&installed_before, &state) {
        if let Err(error) = srelens_plugin_host::sidecar::sandbox::delete_profile(id) {
            log::warn!("could not delete the AppContainer profile of the uninstalled app {id}: {error}");
        }
    }
    app_policy::govern(&mut state, policy.as_deref());
    secret_store::sweep(secrets, &state);
    streams::announce(&store.key(), &state);
    Ok(state)
}

/// Of the apps in `before`, the ones `after` no longer holds: what a change
/// uninstalled.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
fn uninstalled<'a>(before: &'a [String], after: &Inventory) -> Vec<&'a str> {
    before
        .iter()
        .filter(|id| !after.plugins.iter().any(|app| app.manifest.id == **id))
        .map(String::as_str)
        .collect()
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ValidateIn {
    /// The manifest text: at most 256 KiB.
    #[serde(deserialize_with = "limits::manifest")]
    #[schemars(length(max = 262144))]
    manifest: String,
    #[serde(default)]
    grants: Vec<String>,
    /// An Ed25519 signature: exactly 64 bytes.
    #[serde(default, deserialize_with = "limits::signature")]
    #[schemars(length(equal = 64))]
    signature: Option<Vec<u8>>,
    /// The key the signature names (#559): its ID, 64 lowercase hex characters.
    #[serde(default, rename = "keyId", deserialize_with = "limits::key_id")]
    #[schemars(length(equal = 64))]
    key_id: Option<String>,
    /// For a package's manifest (#562): the package's digest list, exactly as
    /// `extensions.packageManifest` or `extensions.catalogManifest` returned it, at most
    /// 64 KiB. The manifest must be the one it names, and `signature` is over the list.
    #[serde(default, deserialize_with = "limits::digests")]
    #[schemars(length(max = 65536))]
    digests: Option<String>,
}
/// `extensions.packageManifest`: a package file to verify for review.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PackageIn {
    /// The `.srelens-extension` file as base64: at most 512 MiB once decoded.
    #[serde(deserialize_with = "limits::package")]
    #[schemars(with = "String")]
    package: Vec<u8>,
}
#[derive(Serialize, Deserialize, JsonSchema)]
struct ValidationReport {
    /// Empty when the manifest could be installed with these grants.
    errors: Vec<ValidationError>,
    #[serde(rename = "permissionDiff", skip_serializing_if = "Option::is_none")]
    permission_diff: Option<PermissionDiff>,
    /// Who signed it, when the signature verified: the publisher delegated its namespace.
    #[serde(rename = "signedBy", skip_serializing_if = "Option::is_none")]
    signed_by: Option<trust::Signer>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct PermissionDiff {
    previous_revision: Option<u64>,
    added: Vec<String>,
    removed: Vec<String>,
    unchanged: Vec<String>,
}

// Canonicalise nested argument objects so a Cargo feature changing serde_json's map
// ordering cannot turn an unchanged scope into a spurious removal and addition.
fn canonical(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by_key(|(key, _)| *key);
            format!(
                "{{{}}}",
                entries
                    .iter()
                    .map(|(key, value)| format!("{}:{}", serde_json::to_string(key).unwrap(), canonical(value)))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
        Value::Array(items) => format!(
            "[{}]",
            items.iter().map(canonical).collect::<Vec<_>>().join(",")
        ),
        _ => value.to_string(),
    }
}

fn access_items(manifest: &Manifest, grants: &[String]) -> std::collections::BTreeSet<String> {
    let mut access = std::collections::BTreeSet::new();
    for grant in grants {
        access.insert(format!("Grant {grant}"));
    }
    // Which secrets the host keeps for the app (#543): an update that keeps
    // one more is new access, even under the same grant.
    let mut secrets: Vec<&str> = manifest
        .settings
        .iter()
        .filter(|setting| {
            setting.setting_type == srelens_capability::settings::SettingType::SecretReference
        })
        .map(|setting| setting.id.as_str())
        .collect();
    if !secrets.is_empty() {
        secrets.sort_unstable();
        access.insert(format!(
            "Keep secrets for settings [{}] with {}",
            secrets.join(","),
            srelens_plugin_host::SECRET_STORE_PERMISSION
        ));
    }
    // Where the app may reach (#568), one item per host, so an update that adds a host
    // shows as that host: new access under the same grant. A host read from a url
    // setting carries the setting's declaration, as an interpolated argument does.
    for host in manifest.network_hosts() {
        let reference = serde_json::Map::from_iter([("host".to_owned(), json!(host))]);
        access.insert(format!(
            "Reach {host}{} with {}",
            setting_scope(manifest, &reference),
            srelens_plugin_host::NETWORK_HTTP
        ));
    }
    // What a reader reads: its arguments and, when it lists several, the versions it may
    // read and the paths each moves (#547). Another accepted version reads more, and a
    // moved path changes what an action's precondition checks there.
    let reads = |binding: &srelens_plugin_host::Binding| {
        let mut identity = binding.arguments.clone();
        if !binding.versions.is_empty() {
            identity.insert("versions".into(), json!(binding.versions));
        }
        if !binding.json_path_overrides.is_empty() {
            identity.insert(
                "jsonPathOverrides".into(),
                json!(binding.json_path_overrides),
            );
        }
        canonical(&Value::Object(identity))
    };
    // Which pods a pod permission reaches by namespace (#567), one item per
    // namespace, as `network.http`'s hosts are.
    for permission in &manifest.permissions {
        for namespace in permission.namespaces() {
            access.insert(format!(
                "Reach any pod in namespace {namespace} with {}",
                permission.capability()
            ));
        }
    }
    for binding in &manifest.capabilities {
        if let Some(item) = pod_access(manifest, binding, &reads) {
            access.insert(item);
            continue;
        }
        if binding.target == "k8s.runJob" {
            access.insert(format!("Run a scoped container Job with {}", reads(binding)));
            continue;
        }
        access.insert(format!(
            "Read {} with {}{}",
            binding.target,
            reads(binding),
            setting_scope(manifest, &binding.arguments)
        ));
    }
    // What each provider asks (#569): its whole template, the binding it goes
    // through and where it is shown, so a changed query shows as changed access.
    for provider in manifest.providers() {
        let (verb, what) = match provider.kind {
            srelens_plugin_host::ProviderKind::Metrics => ("Query", "metrics"),
            srelens_plugin_host::ProviderKind::Logs => ("Follow", "logs"),
            srelens_plugin_host::ProviderKind::Traces => ("Search", "traces"),
        };
        let mut kinds = provider.for_kinds.to_vec();
        kinds.sort();
        let polling: &str = if provider.kind == srelens_plugin_host::ProviderKind::Logs {
            &format!(
                ", asking again every {} s while a log view is open",
                providers::ProviderTiming::default().poll.as_secs()
            )
        } else {
            ""
        };
        access.insert(format!(
            "{verb} {what} {} with {} {} through {} on [{}]{polling}",
            provider.id,
            provider.language.title(),
            serde_json::to_string(provider.query).unwrap_or_default(),
            provider.capability,
            kinds.join(",")
        ));
    }
    for action in &manifest.actions {
        let reader = manifest
            .capabilities
            .iter()
            .find(|binding| binding.name == action.resource);
        let scope = reader
            .map(|binding| format!("{} {}", binding.target, reads(binding)))
            .unwrap_or_else(|| action.resource.clone());
        // Preconditions are enforced on the fresh object by the host action
        // binding. Removing one broadens access even if its primitive and
        // resource kind did not change. Reason text and predicate order do not.
        let mut preconditions: Vec<_> = action
            .preconditions
            .iter()
            .map(|predicate| {
                let mut fields = serde_json::Map::new();
                fields.insert("jsonPath".into(), Value::String(predicate.json_path.clone()));
                if let Some(value) = &predicate.equals {
                    fields.insert("equals".into(), value.clone());
                }
                if let Some(value) = &predicate.not_equals {
                    fields.insert("notEquals".into(), value.clone());
                }
                if let Some(value) = predicate.present {
                    fields.insert("present".into(), Value::Bool(value));
                }
                if let Some(value) = predicate.absent {
                    fields.insert("absent".into(), Value::Bool(value));
                }
                canonical(&Value::Object(fields))
            })
            .collect();
        preconditions.sort();
        access.insert(format!(
            "Action {} on {} with {}{} preconditions [{}]",
            action.target,
            scope,
            canonical(&Value::Object(action.arguments.clone())),
            setting_scope(manifest, &action.arguments),
            preconditions.join(",")
        ));
    }
    access
}

/// What a pod binding (#567) reaches and does, in words: the command it runs,
/// the port it forwards, and whose pods — each object of a reader's kind, by
/// the reader's own identity and where its selector is read, or the granted
/// namespaces. `None` for any other binding.
fn pod_access(
    manifest: &Manifest,
    binding: &srelens_plugin_host::Binding,
    reads: &dyn Fn(&srelens_plugin_host::Binding) -> String,
) -> Option<String> {
    use srelens_plugin_host::{PodScope, POD_EXEC, POD_FORWARD, POD_LOGS};
    if !srelens_plugin_host::is_pod_target(&binding.target) {
        return None;
    }
    let pods = match manifest.pod_scope(binding).ok()? {
        PodScope::Namespaces(_) => "pods in a granted namespace".to_owned(),
        PodScope::Selected { reader, selector } => {
            let kind = srelens_plugin_host::builtin_reader_identity(&reader.target)
                .and_then(|identity| identity["kind"].as_str().map(str::to_owned))
                .or_else(|| {
                    reader
                        .arguments
                        .get("kind")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| reader.name.clone());
            let at = if srelens_plugin_host::builtin_reader_identity(&reader.target).is_some() {
                String::new()
            } else {
                format!(" at {selector}")
            };
            format!(
                "pods selected by each {kind} {} {} reads{at}",
                reader.target,
                reads(reader)
            )
        }
    };
    let container = srelens_plugin_host::pod_container(binding)
        .map(|container| format!("container {container} of "))
        .unwrap_or_default();
    Some(match binding.target.as_str() {
        POD_LOGS => format!("Stream logs of {container}{pods}, with {POD_LOGS}"),
        POD_EXEC => format!(
            "Run {} in {container}{pods}, with {POD_EXEC}",
            canonical(binding.arguments.get("command").unwrap_or(&Value::Null))
        ),
        _ => {
            let port = srelens_plugin_host::forward_port(binding).unwrap_or_default();
            let via = if srelens_plugin_host::forward_via_service(binding) {
                " through a Service to"
            } else {
                " of"
            };
            format!("Forward port {port}{via} {pods} to a local port, with {POD_FORWARD}")
        }
    })
}

fn permission_diff(
    previous: Option<(&Manifest, &[String], u64)>,
    manifest: &Manifest,
    grants: &[String],
) -> PermissionDiff {
    let current = access_items(manifest, grants);
    let old = previous.map(|(manifest, grants, _)| access_items(manifest, grants)).unwrap_or_default();
    PermissionDiff {
        previous_revision: previous.map(|(_, _, revision)| revision),
        added: current.difference(&old).cloned().collect(),
        removed: old.difference(&current).cloned().collect(),
        unchanged: current.intersection(&old).cloned().collect(),
    }
}
/// [`register_with_secrets`] on a host with no secret store: secrets cannot
/// be set, and `extensions.list` says so. For the lifecycle tests; every
/// registry build names its store.
#[cfg(test)]
pub fn register(
    reg: &mut Registry,
    apps: impl Into<Apps>,
    core: Arc<Registry>,
    cache: Arc<srelens_kube::client_cache::ClientCache>,
) -> Arc<streams::ExtensionStreams> {
    register_with_secrets(
        reg,
        apps,
        core,
        cache,
        Arc::new(srelens_plugin_host::NoSecretStore),
    )
}
/// Every `extensions.*` capability over `apps` — an inventory file for the desktop's
/// layout, or one web user's inventory and the shared catalog — with `secrets` keeping
/// apps' secret settings (#543): the desktop vault, or
/// [`srelens_plugin_host::NoSecretStore`].
pub fn register_with_secrets(
    reg: &mut Registry,
    apps: impl Into<Apps>,
    core: Arc<Registry>,
    cache: Arc<srelens_kube::client_cache::ClientCache>,
    secrets: Arc<dyn srelens_plugin_host::SecretStore>,
) -> Arc<streams::ExtensionStreams> {
    register_apps(reg, apps.into(), core, cache, Some(secrets))
}
/// The `extensions.*` capabilities on a host that keeps no app secrets at all: the
/// web host, until per-user secret storage exists (#522). `extension.secretStore` is
/// not registered, so there is no way to hand it one, and `extensions.list` reports
/// the store as unavailable.
pub fn register_without_secrets(
    reg: &mut Registry,
    apps: impl Into<Apps>,
    core: Arc<Registry>,
    cache: Arc<srelens_kube::client_cache::ClientCache>,
) -> Arc<streams::ExtensionStreams> {
    register_apps(reg, apps.into(), core, cache, None)
}
fn register_apps(
    reg: &mut Registry,
    apps: Apps,
    core: Arc<Registry>,
    cache: Arc<srelens_kube::client_cache::ClientCache>,
    secrets: Option<Arc<dyn srelens_plugin_host::SecretStore>>,
) -> Arc<streams::ExtensionStreams> {
    let path = apps.inventory.clone();
    let secrets = match secrets {
        Some(secrets) => {
            secret_store::register(reg, path.clone(), secrets.clone());
            secrets
        }
        None => Arc::new(srelens_plugin_host::NoSecretStore),
    };
    catalog::register(
        reg,
        apps.catalog.clone(),
        core.clone(),
        apps.packages.is_some(),
    );
    let packages = apps.packages.clone();
    let runtime = apps.clone();
    resource::register(reg, path.clone(), core.clone(), cache.clone());
    availability::register(reg, path.clone(), core.clone(), cache.clone());
    jobs::register(reg, &apps, core.clone(), cache.clone());
    // One snapshot of each granted reader, shared by table columns, dashboard
    // cards and a card's target page, so the three agree and list it once.
    let snapshots = columns::JoinCache::default();
    columns::register(reg, path.clone(), core.clone(), cache.clone(), snapshots.clone());
    cards::register(reg, path.clone(), core.clone(), cache.clone(), snapshots.clone());
    let p = path.clone();
    let s = secrets.clone();
    reg.register(Capability::typed::<Empty, Inventory, _, _>(
        "extensions.list",
        "List installed declarative extensions",
        Annotations::READ_ONLY,
        move |_| {
            let p = p.clone();
            let s = s.clone();
            let packages = packages.clone();
            async move {
                tokio::task::spawn_blocking(move || {
                    let mut state = p.read()?;
                    secret_store::report(s.as_ref(), &mut state);
                    report_icons(packages.as_deref(), &mut state);
                    Ok(state)
                })
                .await
                .map_err(|e| CapabilityError::Handler(e.to_string()))?
                .map_err(CapabilityError::Handler)
            }
        },
    ));
    let c = core.clone();
    let s = secrets.clone();
    let validate_catalog = apps.catalog.clone();
    let package_catalog = apps.catalog.clone();
    reg.register(Capability::typed::<Configure, Inventory, _, _>(
        "extensions.configure",
        "Install, enable, remove or configure local extensions; requires approval",
        Annotations::MUTATING,
        move |input| {
            let apps = apps.clone();
            let c = c.clone();
            let s = s.clone();
            async move {
                tokio::task::spawn_blocking(move || configure(&apps, c, s.as_ref(), input))
                    .await
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?
                    .map_err(CapabilityError::Handler)
            }
        },
    ));
    let c = core.clone();
    let p = path.clone();
    reg.register(Capability::typed::<ValidateIn, ValidationReport, _, _>(
        "extensions.validate",
        "Check a declarative extension manifest exactly as installing it would and return every problem; does not install it",
        Annotations::READ_ONLY,
        move |input: ValidateIn| {
            let c = c.clone();
            let p = p.clone();
            let catalog = validate_catalog.clone();
            async move {
                let trust = p.trust.clone();
                let (state, authority) = tokio::task::spawn_blocking(move || {
                    p.read().map(|state| (state, catalog.authority()))
                })
                    .await.map_err(|e| CapabilityError::Handler(e.to_string()))?
                    .map_err(CapabilityError::Handler)?;
                let signature = input.signature.as_deref().map(|signature| (signature, input.key_id.as_deref()));
                let (errors, permission_diff, signed_by) = match check_install(&input.manifest, &input.grants, signature, input.digests.as_deref(), &authority, &trust, c) {
                    Err(problems) => (problems.0, None, None),
                    Ok((manifest, signed)) => {
                        // Reported where the app writes or runs code: its kind for an executable
                        // app, else its actions, else its exec bindings.
                        let at = if manifest.sidecar.is_some() { "kind" } else if manifest.actions.is_empty() { "capabilities" } else { "actions" };
                        let mut errors = check_unsigned_policy(&manifest, input.signature.is_some(), state.allow_unsigned_apps)
                            .err().map(|reason| vec![ValidationError::new(Code::InvalidValue, at, reason)])
                            .unwrap_or_default();
                        // The administrator's policy (#578), as install would apply it.
                        let publisher = signed.as_ref().map(|signed| signed.signer.id.as_str());
                        if let Some(refusal) = state.policy.as_ref().and_then(|policy| policy.refusal(&manifest, publisher)) {
                            errors.push(ValidationError::new(Code::PolicyRefused, refusal.path, refusal.reason));
                        }
                        let previous = state.plugins.iter().find(|app| app.manifest.id == manifest.id)
                            .map(|app| (&app.manifest, app.grants.as_slice(), app.revision));
                        let diff = errors.is_empty().then(|| permission_diff(previous, &manifest, &input.grants));
                        (errors, diff, signed.map(|signed| signed.signer))
                    },
                };
                Ok::<_, CapabilityError>(ValidationReport { errors, permission_diff, signed_by })
            }
        },
    ));
    reg.register(Capability::typed::<PackageIn, catalog::Review, _, _>(
        "extensions.packageManifest",
        "Verify an app package file (.srelens-extension) and return its manifest for permission review; does not install it",
        Annotations::READ_ONLY,
        move |input: PackageIn| {
            let catalog = package_catalog.clone();
            async move {
            tokio::task::spawn_blocking(move || {
                // A package's signature is checked as an install checks it: by the publisher
                // delegated its app ID's namespace (#559).
                let verified = read_package(&input.package, &catalog.authority())?;
                package::check_installable(&verified)?;
                Ok(catalog::Review::of_package(&verified))
            })
            .await
            .map_err(|e| CapabilityError::Handler(e.to_string()))?
            .map_err(CapabilityError::Handler)
            }
        },
    ));
    let reader_snapshots = snapshots.clone();
    let reader_path = path.clone();
    let reader_core = core.clone();
    let reader_cache = cache.clone();
    let reader_secrets = secrets.clone();
    reg.register(Capability::typed::<Read, Value, _, _>(
        "extensions.read",
        "Read a declared custom-resource contribution, or send a declared network.http request, from an enabled extension",
        Annotations::READ_ONLY,
        move |input: Read| {
            read_contribution(
                reader_path.clone(),
                reader_core.clone(),
                reader_cache.clone(),
                reader_snapshots.clone(),
                reader_secrets.clone(),
                input,
            )
        },
    ));
    providers::register(reg, path.clone(), core.clone(), cache.clone(), secrets.clone());
    let streams = streams::register(reg, &runtime, core, cache, snapshots, secrets);
    operations::register(reg, streams.clone());
    inspector::register(reg, path, streams.clone());
    streams
}

/// `extensions.read`: every check it makes is made again on each call, which
/// is what lets a stream re-run it on every tick (#565).
async fn read_contribution(
    p: Store,
    c: Arc<Registry>,
    k: Arc<srelens_kube::client_cache::ClientCache>,
    snapshots: columns::JoinCache,
    secrets: Arc<dyn srelens_plugin_host::SecretStore>,
    input: Read,
) -> Result<Value, CapabilityError> {
    let resolved = request_context(&k, &input.context).await;
    if input.context.trim().is_empty() {
        return Err(CapabilityError::InvalidInput(
            "An explicit cluster context is required".into(),
        ));
    }
    if input.card.as_ref().is_some_and(|card| card.len() > 64) {
        return Err(CapabilityError::InvalidInput(
            "A dashboard card id is at most 64 characters".into(),
        ));
    }
    cards::check_card_namespaces(
        input.card.is_some(),
        &input.namespace,
        &input.namespaces,
    )?;
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
    let state = tokio::task::spawn_blocking(move || p.read())
        .await
        .map_err(|e| CapabilityError::Handler(e.to_string()))?
        .map_err(CapabilityError::Handler)?;
    if let Some(reason) = state
        .plugins
        .iter()
        .find(|p| p.manifest.id == input.id)
        .and_then(|p| p.policy_blocked.as_ref())
    {
        return Err(CapabilityError::Handler(reason.clone()));
    }
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
    let context = resolved
        .ok()
        .and_then(|context| context.pinned_id())
        .unwrap_or(input.context);
    // A network.http binding (#568) is a request to one of the app's hosts, sent by
    // the broker with the app's allowlist and secrets. The checks above are the ones
    // every app request makes; the cluster is not part of the request.
    if plugin
        .manifest
        .capabilities
        .iter()
        .any(|b| b.name == input.capability && b.target == srelens_plugin_host::NETWORK_HTTP)
    {
        if input.card.is_some() {
            return Err(CapabilityError::InvalidInput(
                "A dashboard card counts a custom-resource reader, not a network.http request"
                    .into(),
            ));
        }
        return network::read(
            &c,
            secrets.as_ref(),
            plugin,
            &input.capability,
            state.policy.as_ref(),
        )
        .await;
    }
    // A pod binding (#567) is a session a view opens as a stream, never a read.
    if let Some(binding) = plugin
        .manifest
        .capabilities
        .iter()
        .find(|b| b.name == input.capability && srelens_plugin_host::is_pod_target(&b.target))
    {
        return Err(CapabilityError::InvalidInput(format!(
            "\"{}\" is a {} binding: a view opens it as a stream, not a read",
            binding.name, binding.target
        )));
    }
    // A custom-resource reader reads the version this cluster serves, through
    // that version's paths (#547); `crd::resolved` is also the #601 check. A
    // stream re-runs this on every tick, so it follows a discovery change too.
    let custom = plugin
        .manifest
        .capabilities
        .iter()
        .any(|b| b.name == input.capability && b.target == "k8s.listCustomResource");
    let mut manifest = if custom {
        crd::resolved(&c, &context, &plugin.manifest, &input.capability)
            .await?
            .0
    } else {
        plugin.manifest.clone()
    };
    // The kind's status rules travel in the host's binding, copied
    // out of `statusResolvers` the way an action's preconditions
    // are: the reader evaluates them on the whole object, which
    // never leaves it (#541).
    if let Some(rules) = manifest
        .status_rules_for_binding(&input.capability)
        .map(serde_json::to_value)
        .transpose()
        .map_err(|e| CapabilityError::Handler(format!("status rules: {e}")))?
    {
        if let Some(binding) = manifest.capabilities.iter_mut().find(|b| {
            b.name == input.capability && b.target == "k8s.listCustomResource"
        }) {
            binding
                .arguments
                .insert(STATUS_RULES_ARGUMENT.into(), rules);
        }
    }
    if input.use_crd_columns {
        if let Some(binding) = manifest.capabilities.iter_mut().find(|b| {
            b.name == input.capability && b.target == "k8s.listCustomResource"
        }) {
            binding
                .arguments
                .insert("useCrdColumns".into(), json!(true));
        }
    }
    // A card's target shows only what the card counted. Worked out before
    // the page's own read, so a card that cannot be evaluated fails the
    // read rather than leaving every row on a page titled by the card.
    let counted = match &input.card {
        Some(card) => Some(
            cards::card_rows(
                &snapshots,
                &k,
                &c,
                plugin,
                card,
                &input.capability,
                &context,
                &input.namespace,
                &input.namespaces,
            )
            .await?,
        ),
        None => None,
    };
    let mut registry = Registry::new();
    let _registration = PluginHost::new(c)
        .register_with_settings(
            &mut registry,
            manifest,
            &plugin.grants,
            &plugin.settings,
        )
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
    let mut out = registry
        .invoke(&format!("plugin/{}/{}", input.id, input.capability), args)
        .await?;
    if let Some(counted) = counted {
        let items = out
            .get_mut("items")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| {
                CapabilityError::Handler(
                    "The page's read returned no rows to narrow to the card's".into(),
                )
            })?;
        // A row the card's snapshot lacks — created since it was read — is
        // left out rather than shown as something the card counted.
        items.retain(|item| {
            let key = (
                item["namespace"].as_str().unwrap_or("").to_owned(),
                item["name"].as_str().unwrap_or("").to_owned(),
            );
            counted.contains(&key)
        });
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
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
        let valid = validate(manifest()).await.unwrap();
        assert_eq!(valid["errors"], json!([]));
        assert!(valid["permissionDiff"].is_object());

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
        assert!(unsigned["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["code"] == "EXTENSION_RESERVED_ID" && e["path"] == "id"));
        let signature = include_bytes!("../tests/fixtures/argocd-manifest.sig").to_vec();
        let signed = reg
            .invoke(
                "extensions.validate",
                json!({"manifest": official, "grants": grants, "signature": signature}),
            )
            .await
            .unwrap();
        assert_eq!(signed["errors"].as_array().unwrap().len(), 1);
        assert_eq!(signed["errors"][0]["code"], "EXTENSION_API_INCOMPATIBLE");
        let tampered = reg
            .invoke(
                "extensions.validate",
                json!({"manifest": official.replace("Argo CD", "Argo CE"), "grants": grants, "signature": signature}),
            )
            .await
            .unwrap();
        assert!(tampered["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["code"] == "EXTENSION_INVALID_SIGNATURE"));

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
                (
                    "EXTENSION_API_INCOMPATIBLE".to_owned(),
                    "srelensApiVersion".to_owned()
                ),
                ("EXTENSION_INVALID_SIGNATURE".to_owned(), String::new()),
                ("EXTENSION_INVALID_VALUE".to_owned(), "name".to_owned()),
            ]
        );
    }

    #[tokio::test]
    async fn oversized_caller_inputs_are_refused_with_the_field_and_its_limit() {
        let dir = tempfile::tempdir().unwrap();
        let reg = setup(&dir.path().join("extensions.json"));
        let grants = json!(["k8s.listCustomResource"]);
        let refused = |capability: &'static str, input: Value| {
            let reg = &reg;
            async move {
                match reg.invoke(capability, input).await {
                    Err(CapabilityError::InvalidInput(message)) => message,
                    other => panic!("{capability} was not refused as invalid input: {other:?}"),
                }
            }
        };

        let huge_signature = vec![0u8; 1024 * 1024];
        let message = refused(
            "extensions.validate",
            json!({"manifest": manifest(), "grants": grants, "signature": huge_signature}),
        )
        .await;
        assert!(
            message.contains("signature must be exactly 64 bytes"),
            "{message}"
        );
        let message = refused(
            "extensions.configure",
            json!({"action": "install", "manifest": manifest(), "grants": grants, "signature": huge_signature}),
        )
        .await;
        assert!(
            message.contains("signature must be exactly 64 bytes"),
            "{message}"
        );
        let message = refused(
            "extensions.validate",
            json!({"manifest": manifest(), "grants": grants, "signature": [1, 2, 3]}),
        )
        .await;
        assert!(
            message.contains("signature must be exactly 64 bytes"),
            "{message}"
        );

        let huge_manifest = " ".repeat(srelens_plugin_host::MAX_MANIFEST_BYTES + 1);
        for (capability, input) in [
            (
                "extensions.validate",
                json!({"manifest": huge_manifest, "grants": grants}),
            ),
            (
                "extensions.configure",
                json!({"action": "install", "manifest": huge_manifest, "grants": grants}),
            ),
        ] {
            let message = refused(capability, input).await;
            assert!(message.contains("manifest exceeds 256 KiB"), "{message}");
        }

        // Within the limits, the calls behave as before.
        assert_eq!(
            reg.invoke(
                "extensions.validate",
                json!({"manifest": manifest(), "grants": grants})
            )
            .await
            .unwrap()["errors"],
            json!([])
        );
        reg.invoke(
            "extensions.configure",
            json!({"action": "install", "manifest": manifest(), "grants": grants}),
        )
        .await
        .unwrap();
        let message = refused(
            "extensions.configure",
            json!({"action": "settings", "id": "org.example.argocd",
                   "settings": {"note": "x".repeat(64 * 1024)}}),
        )
        .await;
        assert!(message.contains("settings exceed 64 KiB"), "{message}");
        let listed = reg.invoke("extensions.list", json!({})).await.unwrap();
        assert_eq!(listed["plugins"][0]["settings"], json!({}));
        reg.invoke(
            "extensions.configure",
            json!({"action": "settings", "id": "org.example.argocd", "settings": {"team": "platform"}}),
        )
        .await
        .unwrap();
    }

    #[test]
    fn an_oversized_inventory_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        fs::write(&path, vec![b' '; MAX_INVENTORY_BYTES + 1]).unwrap();
        assert_eq!(
            read(&path).err().as_deref(),
            Some("extension inventory exceeds 1 MiB")
        );
        // Exactly at the limit it is parsed, so the refusal above is about size alone.
        fs::write(&path, vec![b' '; MAX_INVENTORY_BYTES]).unwrap();
        let parsed = read(&path).err().unwrap_or_default();
        assert!(parsed.starts_with("parse extension inventory"), "{parsed}");
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
    fn authentic_retired_release_cannot_install_and_tampering_is_still_reported() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("apps.json");
        let source = include_str!("../tests/fixtures/argocd-manifest.json");
        let signature = include_bytes!("../tests/fixtures/argocd-manifest.sig");
        let shipped = TrustRoot::pinned().shipped();
        signing::verify_for(
            "org.srelens.argocd",
            source.as_bytes(),
            signature,
            None,
            &shipped,
        )
        .unwrap();
        let reason = mutate(&path, fake_core(), signed_argocd()).err().unwrap();
        assert!(reason.contains("requires API ^0.1"), "{reason}");
        assert!(read(&path).unwrap().plugins.is_empty());
        let tampered = check_install(
            &format!("{source} "),
            &["k8s.listCustomResource".into()],
            Some((signature, None)),
            None,
            &Apps::from(path.clone()).catalog.authority(),
            &TrustRoot::pinned(),
            fake_core(),
        )
        .unwrap_err();
        assert!(tampered
            .0
            .iter()
            .any(|error| error.code == Code::InvalidSignature));
        // Existing signed bytes remain in the inventory, quarantined under this host.
        seed_retired_signed_app(&path);
        let state = read(&path).unwrap();
        let app = &state.plugins[0];
        assert!(!app.enabled);
        assert!(app
            .quarantined
            .as_deref()
            .unwrap()
            .contains("requires API ^0.1"));
        assert_eq!(app.signature_proof.as_ref().unwrap().manifest, source);
    }
    /// An authentic installation written by a previous host, never installed through
    /// this host's API or re-signed. Preserve the production proof exactly.
    fn seed_retired_signed_app(path: &Path) {
        let fixture: Inventory =
            serde_json::from_str(include_str!("../tests/fixtures/extension-inventory.json"))
                .unwrap();
        let mut app = fixture
            .plugins
            .into_iter()
            .find(|app| app.manifest.id == "org.srelens.argocd")
            .unwrap();
        app.history.clear();
        let mut state = read(path).unwrap();
        app.revision = take_revision(&mut state).unwrap();
        if let Some(previous) = state
            .plugins
            .iter()
            .position(|p| p.manifest.id == app.manifest.id)
        {
            let previous = state.plugins.remove(previous);
            app.history.push(PreviousVersion {
                signature_proof: previous.signature_proof,
                manifest: previous.manifest,
                grants: previous.grants,
                revision: previous.revision,
                source: previous.source,
                installed_at: previous.installed_at,
                package: previous.package,
            });
        }
        state.plugins.push(app);
        write(path, &state).unwrap();
    }
    /// The example manifest under an unreserved ID, as a local author would install it.
    /// It declares one setting, `team`, so the lifecycle tests can save one
    /// (#542: a save is held to the manifest's declarations).
    pub(super) fn manifest() -> String {
        let source = include_str!("../tests/fixtures/argocd-manifest.json")
            .replace("\"org.srelens.argocd\"", "\"org.example.argocd\"")
            .replace("\"^0.1\"", "\"^0.4\"");
        let mut value: Value = serde_json::from_str(&source).unwrap();
        value["settings"] = json!([{"id": "team", "type": "string", "title": "Team"}]);
        value.to_string()
    }
    fn signed_argocd() -> Configure {
        Configure::Install {
            signature: Some(include_bytes!("../tests/fixtures/argocd-manifest.sig").to_vec()),
            key_id: None,
            manifest: include_str!("../tests/fixtures/argocd-manifest.json").into(),
            grants: vec!["k8s.listCustomResource".into()],
            reviewed_revision: None,
        }
    }
    pub(super) fn configure(path: &Path, input: Value) -> Result<Inventory, String> {
        let mut input = input;
        if input["action"] == "install" && input.get("reviewedRevision").is_none() {
            if let Some(id) = input["manifest"].as_str().and_then(|source| Manifest::parse(source).ok()).map(|manifest| manifest.id) {
                if let Some(app) = read(path)?.plugins.iter().find(|app| app.manifest.id == id) {
                    input["reviewedRevision"] = json!(app.revision);
                }
            }
        }
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
    fn update_preview_separates_added_removed_and_unchanged_access() {
        let mut old: Manifest = serde_json::from_str(&manifest()).unwrap();
        let mut new = old.clone();
        old.permissions.push("k8s.listEvents".into());
        new.permissions.push("k8s.annotate".into());
        let preview = permission_diff(Some((&old, &["k8s.listCustomResource".into(), "k8s.listEvents".into()][..], 7)), &new, &["k8s.listCustomResource".into(), "k8s.annotate".into()]);
        assert_eq!(preview.previous_revision, Some(7));
        assert!(preview.added.iter().any(|entry| entry.contains("k8s.annotate")));
        assert!(preview.removed.iter().any(|entry| entry.contains("k8s.listEvents")));
        assert!(preview.unchanged.iter().any(|entry| entry.contains("k8s.listCustomResource")));
    }
    #[test]
    fn changing_a_reader_kind_or_bound_action_is_an_access_change_even_with_same_grants() {
        let mut old: Manifest = serde_json::from_str(&manifest()).unwrap();
        old.actions.push(serde_json::from_value(json!({"name":"renew","title":"Renew","target":"k8s.annotate","resource":"applications","arguments":{"key":"old"}})).unwrap());
        let mut next = old.clone();
        next.capabilities[0].arguments.insert("kind".into(), json!("OtherApplication"));
        next.actions[0].arguments.insert("key".into(), json!("new"));
        let grants = ["k8s.listCustomResource".into(), "k8s.annotate".into()];
        let diff = permission_diff(Some((&old, &grants, 4)), &next, &grants);
        assert_eq!(diff.added.len(), 2, "reader kind and action scope changed: {diff:?}");
        assert_eq!(diff.removed.len(), 2, "the old reader and action scopes were removed: {diff:?}");
        assert_eq!(diff.unchanged.len(), 2, "grant names alone did not change: {diff:?}");
    }
    #[test]
    fn removing_an_enforced_action_precondition_is_reported_as_broader_access() {
        let mut guarded: Manifest = serde_json::from_str(&manifest()).unwrap();
        guarded.actions.push(serde_json::from_value(json!({
            "name":"reconcile", "title":"Reconcile", "target":"k8s.annotate",
            "resource":"applications", "arguments":{"key":"reconcile"},
            "preconditions":[{"jsonPath":".spec.suspend","notEquals":true,"reason":"Resume first"}]
        })).unwrap());
        let mut unguarded = guarded.clone();
        unguarded.actions[0].preconditions.clear();
        let grants = ["k8s.listCustomResource".into(), "k8s.annotate".into()];
        let diff = permission_diff(Some((&guarded, &grants, 3)), &unguarded, &grants);
        assert_eq!(diff.added.len(), 1, "unguarded action must be new access: {diff:?}");
        assert_eq!(diff.removed.len(), 1, "guarded action must be removed: {diff:?}");
    }
    #[tokio::test]
    async fn validation_previews_installed_access_and_stale_review_cannot_replace_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        let old_revision = install(&path, fake_core());
        let mut next: Value = serde_json::from_str(&manifest()).unwrap();
        next["version"] = json!("0.3.0");
        let source = next.to_string();
        let reg = setup(&path);
        let preview = reg.invoke("extensions.validate", json!({"manifest":source,"grants":["k8s.listCustomResource"]})).await.unwrap();
        assert_eq!(preview["errors"], json!([]));
        assert_eq!(preview["permissionDiff"]["previousRevision"], old_revision);
        let before = fs::read(&path).unwrap();
        let refused = reg.invoke("extensions.configure", json!({"action":"install","manifest":source,"grants":["k8s.listCustomResource"]})).await;
        assert!(refused.is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        let installed = reg.invoke("extensions.configure", json!({"action":"install","manifest":source,"grants":["k8s.listCustomResource"],"reviewedRevision":old_revision})).await.unwrap();
        assert_eq!(installed["plugins"][0]["manifest"]["version"], "0.3.0");
        let before = fs::read(&path).unwrap();
        let stale = reg.invoke("extensions.configure", json!({"action":"install","manifest":source,"grants":["k8s.listCustomResource"],"reviewedRevision":old_revision})).await;
        assert!(stale.is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
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
        let mut release: Value =
            serde_json::from_slice(include_bytes!("../tests/fixtures/extension-catalog.json"))
                .unwrap();
        let source = manifest().replace("org.example.argocd", "org.example.catalog");
        let entry = &mut release["extensions"][0];
        entry["id"] = json!("org.example.catalog");
        entry["repository"] = json!("https://github.com/example/catalog");
        entry["release"]["manifestUrl"] =
            json!("https://github.com/example/catalog/releases/download/v0.2.0/manifest.json");
        entry["release"]["srelensApiVersion"] = json!("^0.3");
        use sha2::Digest;
        entry["release"]["sha256"] =
            json!(format!("{:x}", sha2::Sha256::digest(source.as_bytes())));
        fs::write(
            path.with_extension("catalog.json"),
            catalog::test_cache(
                &[trust::testing::srelens_publisher()],
                release["extensions"].clone(),
                0,
            ),
        )
        .unwrap();
        mutate(
            &path,
            fake_core(),
            Configure::Install {
                manifest: source,
                signature: None,
                key_id: None,
                grants: vec!["k8s.listCustomResource".into()],
                reviewed_revision: None,
            },
        )
        .unwrap();
        let state = read(&path).unwrap();
        assert!(find(&state, "org.example.catalog").source == Source::Catalog);
        assert!(find(&state, "org.example.argocd").source == Source::Local);
    }
    #[test]
    fn kept_versions_give_way_before_the_inventory_outgrows_its_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("apps.json");
        // A valid manifest padded under the 256 KiB decode limit. The inventory is
        // saved pretty-printed, so the installed version and three kept ones cannot
        // all fit in 1 MiB. Inflate via `$schema` (host-ignored) rather than
        // printerColumns, which are capped at 32 (#609).
        let large = |version: &str| {
            let mut value: Value = serde_json::from_str(&manifest_at(version)).unwrap();
            let base = value.to_string().len();
            let pad = (256 * 1024usize).saturating_sub(base + 32);
            value["$schema"] = json!("x".repeat(pad));
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
        let path_segment = file.replace('#', "%23");
        let mut yaml = format!(
            "apiVersion: v1\nkind: Config\nclusters:\n- name: c\n  cluster:\n    server: https://127.0.0.1:1/{path_segment}\nusers:\n- name: u\n  user: {{}}\ncontexts:\n",
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
    /// An app page asks the host by the pinned ID `k8s.listContexts` reports (#695), so two
    /// contexts that share a stable ID each read their own cluster.
    #[tokio::test]
    async fn each_context_sharing_a_stable_id_reads_its_own_cluster_by_its_listed_pinned_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        let first = kubeconfig(dir.path(), "a", &["b#c"]);
        let second = kubeconfig(dir.path(), "a#b", &["c"]);
        let core = fake_core();
        let listed = core
            .invoke("k8s.listContexts", json!({"paths": [&first, &second]}))
            .await
            .unwrap();
        let contexts = listed["contexts"].as_array().unwrap();
        assert_eq!(contexts[0]["stableId"], contexts[1]["stableId"]);
        let pinned: Vec<Value> = contexts.iter().map(|c| c["pinnedId"].clone()).collect();
        assert!(
            pinned.iter().all(Value::is_string) && pinned[0] != pinned[1],
            "{pinned:?}"
        );
        let revision = install(&path, core.clone());
        let mut reg = Registry::new();
        let cache = srelens_kube::client_cache::ClientCache::new_many(vec![first, second]);
        register(&mut reg, path, core, cache);
        let read = |context: Value| {
            let payload = json!({"id":"org.example.argocd","revision":revision,
                "capability":"applications","context":context,"namespace":""});
            reg.invoke("extensions.read", payload)
        };
        for id in &pinned {
            assert_eq!(read(id.clone()).await.unwrap()["context"], *id);
        }
        // The shared stable ID names two contexts, so it is pinned to neither.
        let shared = read(contexts[0]["stableId"].clone()).await;
        assert!(shared.map_or(true, |out| !pinned.contains(&out["context"])));
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
        let grants = parsed.permission_names();
        let without_events: Vec<_> = grants
            .iter()
            .filter(|grant| grant.as_str() != "k8s.listEvents")
            .cloned()
            .collect();
        assert!(validate_app(&parsed, &grants, core.clone()).is_ok());
        assert!(validate_app(&parsed, &without_events, core.clone()).is_err());
        // The example's card targets this page; it would be refused first, for its own reason.
        value["contributions"].as_object_mut().unwrap().remove("dashboardCards");
        value["contributions"]["pages"][1]["capability"] = json!("events");
        let invalid = Manifest::decode(&value.to_string()).unwrap();
        // The specific refusal: an event reader cannot back a table. Other
        // problems this edit also causes (the dashboard page now names a
        // kind with no resolver) must not stand in for it.
        let refused = validate_app(&invalid, &grants, core).unwrap_err();
        assert!(
            refused.0.iter().any(|p| p.code == Code::UnsupportedTarget
                && p.path == "contributions.pages[1].capability"
                && p.message.contains("k8s.listCustomResource reader")),
            "{refused}"
        );
    }

    #[test]
    fn builtin_actions_are_narrowly_bound_to_host_reader_identity() {
        let core = Arc::new(crate::build_registry_with_paths(
            srelens_kube::client_cache::ClientCache::new_many(vec![]),
            vec![],
        ));
        for (reader, target, arguments) in [
            (
                "k8s.listDeployments",
                "k8s.requestRolloutRestart",
                json!({}),
            ),
            (
                "k8s.listStatefulSets",
                "k8s.requestRolloutRestart",
                json!({}),
            ),
            ("k8s.listDaemonSets", "k8s.requestRolloutRestart", json!({})),
            (
                "k8s.listNodes",
                "k8s.requestCordonNode",
                json!({"unschedulable":true}),
            ),
            (
                "k8s.listNodes",
                "k8s.requestCordonNode",
                json!({"unschedulable":false}),
            ),
        ] {
            let mut source: Value = serde_json::from_str(&manifest()).unwrap();
            source["contributions"] = json!({"pages":[],"detailTabs":[],"detailLinks":[]});
            source["permissions"] = json!([reader, target]);
            source["capabilities"] = json!([{"name":"objects","title":"Objects","target":reader,"arguments":{},
                "inputs":if reader == "k8s.listNodes" {vec!["context"]} else {vec!["context","namespace"]}}]);
            source["actions"] = json!([{"name":"request","title":"Request","target":target,"resource":"objects","arguments":arguments}]);
            let parsed = Manifest::parse(&source.to_string()).unwrap();
            let grants = parsed.permission_names();
            validate_app(&parsed, &grants, core.clone()).unwrap();
            assert!(validate_app(&parsed, &[reader.into()], core.clone()).is_err());
            for denied in [
                "k8s.annotate",
                "k8s.setFields",
                "k8s.mergePatch",
                "k8s.drainNode",
            ] {
                let mut wrong = source.clone();
                wrong["permissions"] = json!([reader, denied]);
                wrong["actions"][0]["target"] = json!(denied);
                let wrong = Manifest::parse(&wrong.to_string()).unwrap();
                assert!(
                    validate_app(&wrong, &wrong.permission_names(), core.clone()).is_err(),
                    "{reader}: {denied}"
                );
            }
            let mut forged = source.clone();
            forged["capabilities"][0]["arguments"] = json!({"group":"evil.io","kind":"Secret"});
            let forged = Manifest::parse(&forged.to_string()).unwrap();
            assert!(validate_app(&forged, &grants, core.clone()).is_err());
        }
    }

    /// An app's readers and its declared actions are separate grants against
    /// separate host capabilities (#549): a reader still cannot bind a write,
    /// and an action can only bind one of the host's action primitives.
    #[test]
    fn declared_actions_bind_the_host_primitives_and_nothing_else() {
        let core = Arc::new(crate::build_registry_with_paths(
            srelens_kube::client_cache::ClientCache::new_many(vec![]),
            vec![],
        ));
        let mut value: Value =
            serde_json::from_str(include_str!("../../../examples/extensions/argocd.json")).unwrap();
        value["permissions"] = json!(["k8s.listCustomResource", "k8s.annotate"]);
        value["actions"] = json!([{
            "name":"refresh", "title":"Refresh", "target":"k8s.annotate", "resource":"applications",
            "arguments":{"key":"argocd.argoproj.io/refresh","value":"normal"}
        }]);
        // The example's palette commands name the actions replaced above.
        value["contributions"]["commands"] = json!([]);
        let grants = vec!["k8s.listCustomResource".into(), "k8s.annotate".into()];
        let parsed = Manifest::parse(&value.to_string()).unwrap();
        validate_app(&parsed, &grants, core.clone()).unwrap();

        // The action target has to be granted like any other permission.
        assert!(validate_app(&parsed, &["k8s.listCustomResource".into()], core.clone()).is_err());

        // A reader binding still cannot dispatch a write.
        let mut writing_reader = value.clone();
        writing_reader["capabilities"][0]["target"] = json!("k8s.annotate");
        writing_reader["capabilities"][0]["arguments"] =
            json!({"key":"a.example.io/b","value":"$now"});
        writing_reader["permissions"] = json!(["k8s.annotate"]);
        // Decoded, not parsed: the manifest's own rules also refuse it now
        // (its status resolver names a kind no reader lists), and this case
        // is about what `validate_app` says of the reader's target.
        let parsed = Manifest::decode(&writing_reader.to_string()).unwrap();
        let refused = validate_app(&parsed, &["k8s.annotate".into()], core.clone()).unwrap_err();
        assert!(
            refused
                .0
                .iter()
                .any(|p| p.code == Code::UnsupportedTarget && p.path == "capabilities[0].target"),
            "{refused}"
        );

        // And an action cannot bind a host mutation that is not a primitive.
        let mut wrong = value;
        wrong["actions"][0]["target"] = json!("k8s.deletePod");
        wrong["permissions"] = json!(["k8s.listCustomResource", "k8s.deletePod"]);
        let parsed = Manifest::parse(&wrong.to_string()).unwrap();
        let refused = validate_app(
            &parsed,
            &["k8s.listCustomResource".into(), "k8s.deletePod".into()],
            core,
        )
        .unwrap_err();
        assert!(
            refused
                .0
                .iter()
                .any(|p| p.code == Code::UnsupportedTarget && p.path == "actions[0].target"),
            "{refused}"
        );
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
                key_id: None,
                manifest: source.to_string(),
                grants: vec!["k8s.listCustomResource".into()],
                reviewed_revision: None,
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
    pub(crate) fn fake_core() -> Arc<Registry> {
        let mut core = crate::build_registry_with_paths(
            srelens_kube::client_cache::ClientCache::new_many(vec![]),
            vec![],
        );
        let mut cap = core.get("k8s.listCustomResource").unwrap().clone();
        cap.handler = Arc::new(|args| Box::pin(async move { Ok(args) }));
        core.register(cap);
        serve_crds(&mut core, &["applications.argoproj.io/v1alpha1"]);
        // The broker's own, as `build_registry_and_app_streams` registers them (#568, #567).
        core.register(network::capability());
        core.register(jobs::capability());
        for capability in pods::capabilities() {
            core.register(capability);
        }
        Arc::new(core)
    }
    /// Answers the broker's CRD check as a cluster whose CRDs serve exactly these
    /// `{plural}.{group}/{version}` would: the first listed version served, or none.
    pub(super) fn serve_crds(core: &mut Registry, names: &'static [&'static str]) {
        let mut cap =
            crd::check_capability(srelens_kube::client_cache::ClientCache::new_many(vec![]));
        cap.handler = Arc::new(move |args| {
            Box::pin(async move {
                let served = args["versions"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .find(|version| {
                        let name = format!(
                            "{}.{}/{version}",
                            args["plural"].as_str().unwrap_or_default(),
                            args["group"].as_str().unwrap_or_default(),
                        );
                        names.contains(&name.as_str())
                    })
                    .map(str::to_owned);
                Ok(json!(served))
            })
        });
        core.register(cap);
    }
    pub(super) fn install(path: &Path, core: Arc<Registry>) -> u64 {
        let reviewed_revision = read(path).unwrap().plugins.iter()
            .find(|app| app.manifest.id == "org.example.argocd")
            .map(|app| app.revision);
        mutate(
            path,
            core,
            Configure::Install {
                signature: None,
                key_id: None,
                manifest: manifest(),
                grants: vec!["k8s.listCustomResource".into()],
                reviewed_revision,
            },
        )
        .unwrap()
        .plugins[0]
            .revision
    }
    /// The example manifest with a status resolver for its Application reader.
    fn manifest_with_status() -> Value {
        let mut value: Value = serde_json::from_str(&manifest()).unwrap();
        value["contributions"]["statusResolvers"] = json!([{
            "forKinds": ["argoproj.io/Application"],
            "rules": [
                {"when": [{"jsonPath": ".status.health.status", "equals": "Healthy"}],
                 "status": "healthy", "label": "Healthy"},
                {"when": [], "status": "unknown", "label": "Unknown"}
            ]
        }]);
        value
    }

    #[tokio::test]
    async fn a_read_binds_the_status_rules_declared_for_the_readers_kind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        let core = fake_core();
        let declared = manifest_with_status();
        let revision = mutate(
            &path,
            core.clone(),
            Configure::Install {
                signature: None,
                key_id: None,
                manifest: declared.to_string(),
                grants: vec!["k8s.listCustomResource".into()],
                reviewed_revision: None,
            },
        )
        .unwrap()
        .plugins[0]
            .revision;
        let mut reader = Registry::new();
        register(
            &mut reader,
            path.clone(),
            core.clone(),
            srelens_kube::client_cache::ClientCache::new_many(vec![]),
        );
        // `fake_core`'s reader echoes what it was sent: the host's binding,
        // with the rules copied out of the manifest under the reader's
        // camelCase input name.
        let sent = reader
            .invoke(
                "extensions.read",
                json!({"id":"org.example.argocd","revision":revision,"capability":"applications",
                    "context":"staging","namespace":"argo"}),
            )
            .await
            .unwrap();
        assert_eq!(
            sent["statusRules"],
            declared["contributions"]["statusResolvers"][0]["rules"]
        );
        // Without a resolver, nothing is bound.
        let revision = install(&path, core.clone());
        let sent = reader
            .invoke(
                "extensions.read",
                json!({"id":"org.example.argocd","revision":revision,"capability":"applications",
                    "context":"staging","namespace":"argo"}),
            )
            .await
            .unwrap();
        assert!(sent.get("statusRules").is_none(), "{sent}");
    }

    #[test]
    fn status_rules_are_the_hosts_to_bind_and_badges_sit_on_built_in_kinds() {
        let core = fake_core();
        let grants = vec!["k8s.listCustomResource".to_owned()];
        let paths = |value: &Value| {
            let manifest: Manifest = serde_json::from_value(value.clone()).unwrap();
            let mut found: Vec<String> = validate_app(&manifest, &grants, core.clone())
                .err()
                .map(|errors| errors.0.into_iter().map(|e| e.path).collect())
                .unwrap_or_default();
            found.sort();
            found
        };
        let mut value = manifest_with_status();
        assert_eq!(paths(&value), Vec::<String>::new());
        // One spelling: an app does not bind the reader's rules itself.
        value["capabilities"][0]["arguments"]["statusRules"] = json!([]);
        assert_eq!(paths(&value), vec!["capabilities[0].arguments.statusRules"]);
        let mut value = manifest_with_status();
        value["contributions"]["badges"] = json!([{
            "id": "argo", "forKinds": ["apps/Deployment", "/Secret", "argoproj.io/Application", "acme.io/Deployment"],
            "rules": [{"when": [{"jsonPath": ".metadata.annotations['argocd.argoproj.io/tracking-id']", "present": true}],
                "status": "healthy", "label": "Argo CD"}]
        }]);
        assert_eq!(
            paths(&value),
            vec![
                "contributions.badges[0].forKinds[1]",
                "contributions.badges[0].forKinds[2]",
                "contributions.badges[0].forKinds[3]",
            ]
        );
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
        let reviewed_revision = Some(read(&path).unwrap().plugins[0].revision);
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
                    key_id: None,
                    manifest: source.to_string(),
                    grants: vec!["k8s.listCustomResource".into()],
                    reviewed_revision,
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
                    key_id: None,
                    manifest: source.to_string(),
                    grants: vec!["k8s.listCustomResource".into()],
                    reviewed_revision,
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
            "extensions.resolveColumns",
            "extensions.resolveCards",
            "extensions.resolvePanels",
            "extensions.resolveLinks",
            "extensions.resolveReverseLinks",
            "extensions.catalog",
            "extensions.catalogManifest",
            "extensions.packageManifest",
            "extensions.validate",
            "extensions.streams",
            "extensions.pods",
            "extensions.queryProvider",
        ] {
            assert!(reg.get(id).unwrap().annotations.read_only);
        }
        // The secret store (#543) is gated, and sensitive: what goes through
        // it is secret material.
        let store = reg.get("extension.secretStore").unwrap().annotations;
        assert!(store.requires_confirm && store.sensitive && !store.read_only);
        let job = reg.get("extensions.runJob").unwrap().annotations;
        assert!(job.requires_confirm && !job.read_only && !job.sensitive);
        let mcp = srelens_mcp::McpServer::new(Arc::new(reg));
        assert_eq!(mcp.list_tools().len(), 20);
        assert!(mcp.list_tools().iter().any(|tool| tool.name == "extensions.bindingAvailability"));
        assert!(!mcp.list_tools().iter().any(|tool| tool.name == "extensions.callOperation"));
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
                            key_id: None,
                            manifest: source.to_string(),
                            grants: vec!["k8s.listCustomResource".into()],
                            reviewed_revision: None,
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
        seed_retired_signed_app(&path);
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
        let proof = quarantined.signature_proof.as_ref().unwrap();
        assert!(signing::verify_for(
            &quarantined.manifest.id,
            proof.manifest.as_bytes(),
            &proof.signature,
            None,
            &TrustRoot::pinned().shipped(),
        )
        .unwrap_err()
        .contains("signature"));
        let reason = quarantined.quarantined.clone().unwrap();
        assert!(reason.contains("requires API ^0.1"), "{reason}");
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

        // Authenticity alone cannot lift quarantine for a retired API.
        assert!(mutate(&path, core, signed_argocd())
            .err()
            .unwrap()
            .contains("requires API ^0.1"));
        assert!(!find(&read(&path).unwrap(), "org.srelens.argocd").enabled);
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
            key_id: None,
            manifest: manifest.into(),
            grants: vec!["k8s.listCustomResource".into()],
            reviewed_revision: None,
        };
        let refused = mutate(&path, core.clone(), unsigned(official))
            .err()
            .unwrap();
        assert!(refused.contains("reserved"), "{refused}");
        assert!(read(&path).unwrap().plugins.is_empty());

        // A signed install cannot be replaced by an unsigned manifest under the same ID.
        seed_retired_signed_app(&path);
        let before = fs::read(&path).unwrap();
        assert!(mutate(&path, core.clone(), unsigned(official))
            .err()
            .unwrap()
            .contains("reserved"));
        assert_eq!(fs::read(&path).unwrap(), before);

        let lookalike = official
            .replace("\"org.srelens.argocd\"", "\"org.srelensx.argocd\"")
            .replace("^0.1", "^0.3");
        assert!(mutate(&path, core, unsigned(&lookalike)).is_ok());
    }
    /// A catalog cache beside the inventory at `path` that delegates srelens and the test
    /// publisher Example Labs, and lists nothing.
    pub(super) fn cache_delegating_example_labs(path: &Path) {
        fs::write(
            path.with_extension("catalog.json"),
            catalog::test_cache(
                &[
                    trust::testing::srelens_publisher(),
                    trust::testing::example_publisher(),
                ],
                json!([]),
                now(),
            ),
        )
        .unwrap();
    }
    /// The example manifest as app `id`, and Example Labs' signature over it, naming its key.
    pub(super) fn signed_by_example_labs(id: &str) -> (String, Vec<u8>, String) {
        let key = trust::testing::key(trust::testing::EXAMPLE_SEED);
        let source = manifest().replacen("org.example.argocd", id, 1);
        let signature = key.sign(source.as_bytes()).as_ref().to_vec();
        (source, signature, trust::testing::id(&key))
    }
    fn install_signed(source: &str, signature: Vec<u8>, key_id: Option<String>) -> Configure {
        Configure::Install {
            signature: Some(signature),
            key_id,
            manifest: source.into(),
            grants: vec!["k8s.listCustomResource".into()],
            reviewed_revision: None,
        }
    }
    /// Acceptance (#559): a test publisher's key signs an app in its namespace, and the app
    /// installs as signed by that publisher — and stays so on every load, with no catalog
    /// at hand, because the install kept the delegation that vouched for it.
    #[tokio::test]
    async fn a_publisher_key_signs_an_app_in_its_namespace_and_it_installs_as_signed_by_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        cache_delegating_example_labs(&path);
        let (source, signature, key_id) = signed_by_example_labs("com.example-labs.gitops");

        let mut reg = Registry::new();
        register(
            &mut reg,
            path.clone(),
            fake_core(),
            srelens_kube::client_cache::ClientCache::new_many(vec![]),
        );
        // The review names the signer before anything is installed.
        let report = reg
            .invoke(
                "extensions.validate",
                json!({"manifest": source, "grants": ["k8s.listCustomResource"], "signature": signature, "keyId": key_id}),
            )
            .await
            .unwrap();
        assert_eq!(report["errors"], json!([]), "{report}");
        assert_eq!(
            report["signedBy"],
            json!({"id": "example", "name": "Example Labs"})
        );

        let state = mutate(
            &path,
            fake_core(),
            install_signed(&source, signature.clone(), Some(key_id.clone())),
        )
        .unwrap();
        let app = find(&state, "com.example-labs.gitops");
        assert_eq!(app.signed_by.as_ref().unwrap().name, "Example Labs");
        let proof = app.signature_proof.as_ref().unwrap();
        assert!(
            proof.delegation.is_some(),
            "this build ships no delegation for Example Labs"
        );
        // Reported by `extensions.list` as the caller reads it, and never saved.
        let listed = reg.invoke("extensions.list", json!({})).await.unwrap();
        assert_eq!(
            listed["plugins"][0]["signedBy"],
            json!({"id": "example", "name": "Example Labs"})
        );
        let saved = fs::read_to_string(&path).unwrap();
        assert!(!saved.contains("signedBy"), "{saved}");

        // With the catalog gone, the kept delegation still vouches for it.
        fs::remove_file(path.with_extension("catalog.json")).unwrap();
        let state = read(&path).unwrap();
        let app = find(&state, "com.example-labs.gitops");
        assert!(app.enabled && app.quarantined.is_none());
        assert_eq!(app.signed_by.as_ref().unwrap().name, "Example Labs");

        // The same signature naming no key is verified against the publisher's keys.
        let dir = tempfile::tempdir().unwrap();
        let other = dir.path().join("extensions.json");
        cache_delegating_example_labs(&other);
        let state = mutate(
            &other,
            fake_core(),
            install_signed(&source, signature, None),
        )
        .unwrap();
        assert_eq!(
            find(&state, "com.example-labs.gitops")
                .signed_by
                .as_ref()
                .unwrap()
                .name,
            "Example Labs"
        );
    }
    /// Acceptance (#559): the same key cannot sign an app in srelens's namespace, whether
    /// the signature names it or not, and a kept delegation cannot be moved to such an app.
    #[test]
    fn a_publisher_key_cannot_sign_an_app_in_the_srelens_namespace() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        cache_delegating_example_labs(&path);
        let (source, signature, key_id) = signed_by_example_labs("org.srelens.gitops");
        let refused = mutate(
            &path,
            fake_core(),
            install_signed(&source, signature.clone(), Some(key_id)),
        )
        .err()
        .unwrap();
        assert!(refused.contains("signed only by srelens"), "{refused}");
        let refused = mutate(
            &path,
            fake_core(),
            install_signed(&source, signature.clone(), None),
        )
        .err()
        .unwrap();
        assert!(refused.contains("signature is invalid"), "{refused}");
        assert!(read(&path).unwrap().plugins.is_empty());

        // An inventory edited to give that app Example Labs' kept delegation: quarantined.
        // The delegation does not cover the ID; srelens's does, and its key did not sign it.
        let (own, own_signature, own_key) = signed_by_example_labs("com.example-labs.gitops");
        mutate(
            &path,
            fake_core(),
            install_signed(&own, own_signature, Some(own_key)),
        )
        .unwrap();
        let mut stored: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let app = &mut stored["plugins"][0];
        app["manifest"] = serde_json::from_str(&source).unwrap();
        app["signatureProof"]["manifest"] = json!(source);
        app["signatureProof"]["signature"] = json!(signature);
        fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();
        let state = read(&path).unwrap();
        let moved = find(&state, "org.srelens.gitops");
        assert!(!moved.enabled && moved.signed_by.is_none());
        let reason = moved.quarantined.as_deref().unwrap();
        assert!(reason.contains("signature is invalid"), "{reason}");
    }
    /// A build with no usable root cannot tell which IDs are reserved, so it installs and
    /// restores nothing, unsigned or not, and says why (#559).
    #[test]
    fn a_host_with_no_pinned_root_installs_and_restores_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        let apps = Apps::with_trust(path.clone(), trust::testing::placeholder_root());
        let unsigned = |id: &str| Configure::Install {
            signature: None,
            key_id: None,
            manifest: manifest().replacen("org.example.argocd", id, 1),
            grants: vec!["k8s.listCustomResource".into()],
            reviewed_revision: None,
        };
        for id in ["org.srelens.argocd", "org.example.argocd"] {
            let refused = super::configure(
                &apps,
                fake_core(),
                &srelens_plugin_host::NoSecretStore,
                unsigned(id),
            )
            .err()
            .unwrap();
            assert!(refused.contains("installs nothing"), "{id}: {refused}");
            assert!(refused.contains("key ceremony"), "{id}: {refused}");
        }
        assert!(read(&path).unwrap().plugins.is_empty());
        // A kept version is not restored either.
        mutate(&path, fake_core(), unsigned("org.example.argocd")).unwrap();
        mutate(
            &path,
            fake_core(),
            Configure::Install {
                signature: None,
                key_id: None,
                manifest: manifest_at("0.2.0"),
                grants: vec!["k8s.listCustomResource".into()],
                reviewed_revision: Some(1),
            },
        )
        .unwrap();
        let refused = super::configure(
            &apps,
            fake_core(),
            &srelens_plugin_host::NoSecretStore,
            Configure::Rollback {
                id: "org.example.argocd".into(),
                revision: 1,
                grants: vec!["k8s.listCustomResource".into()],
            },
        )
        .err()
        .unwrap();
        assert!(refused.contains("restores nothing"), "{refused}");
    }
    /// A delegation this build ships for a publisher, at the same or a later version,
    /// replaces the one an install kept: a key the newer delegation withdrew stops
    /// vouching for what it signed (#559).
    #[test]
    fn a_later_shipped_delegation_replaces_the_one_an_install_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        cache_delegating_example_labs(&path);
        let (source, signature, key_id) = signed_by_example_labs("com.example-labs.gitops");
        mutate(
            &path,
            fake_core(),
            install_signed(&source, signature, Some(key_id)),
        )
        .unwrap();
        let example = |version, key| {
            trust::testing::publisher_at(
                version,
                "example",
                "Example Labs",
                &[trust::testing::public(&trust::testing::key(key))],
                &["com.example-labs"],
            )
        };
        let srelens = trust::testing::srelens_publisher();
        // A build shipping version 2 of Example Labs' delegation, without the key that
        // signed the app: quarantined.
        let rotated = trust::testing::root_shipping(&[srelens.clone(), example(2, 0x88)]);
        let state = read_under(&path, &rotated).unwrap();
        let app = find(&state, "com.example-labs.gitops");
        assert!(
            !app.enabled && app.signed_by.is_none(),
            "{:?}",
            app.quarantined
        );
        // One shipping the version the install kept: still signed by Example Labs.
        let same =
            trust::testing::root_shipping(&[srelens, example(1, trust::testing::EXAMPLE_SEED)]);
        let state = read_under(&path, &same).unwrap();
        assert_eq!(
            find(&state, "com.example-labs.gitops")
                .signed_by
                .as_ref()
                .unwrap()
                .name,
            "Example Labs"
        );
    }
    /// A catalog that rotates a publisher this build ships changes who signs for it only
    /// with a later version of its delegation (#559 review): at the same version the
    /// shipped delegation stands, at install as on every later load, so an app is never
    /// installed as signed and then quarantined on its next read.
    #[test]
    fn a_catalog_rotates_a_shipped_publisher_only_with_a_later_delegation() {
        let rotated_key = trust::testing::key(0x77);
        let id = "org.srelens.gitops";
        let source = manifest().replacen("org.example.argocd", id, 1);
        let signature = rotated_key.sign(source.as_bytes()).as_ref().to_vec();
        let key_id = trust::testing::id(&rotated_key);
        let catalog_rotating_srelens_at = |path: &Path, version| {
            fs::write(
                path.with_extension("catalog.json"),
                catalog::test_cache(
                    &[trust::testing::publisher_at(
                        version,
                        "srelens",
                        "srelens",
                        &[trust::testing::public(&rotated_key)],
                        &["org.srelens"],
                    )],
                    json!([]),
                    now(),
                ),
            )
            .unwrap();
        };
        let install = || install_signed(&source, signature.clone(), Some(key_id.clone()));

        // The shipped srelens delegation is version 1: a catalog's version 1 with another
        // key does not replace it, so the install is refused up front.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        catalog_rotating_srelens_at(&path, 1);
        let Err(error) = mutate(&path, fake_core(), install()) else {
            panic!("a same-version rotation replaced the shipped srelens delegation");
        };
        assert!(error.contains("srelens"), "{error}");
        assert!(read(&path).unwrap().plugins.is_empty());

        // Version 2 moves srelens on: the app installs, and its next load agrees.
        catalog_rotating_srelens_at(&path, 2);
        let state = mutate(&path, fake_core(), install()).unwrap();
        let app = find(&state, id);
        assert!(
            app.enabled && app.quarantined.is_none(),
            "{:?}",
            app.quarantined
        );
        assert!(app.signature_proof.as_ref().unwrap().delegation.is_some());
        let state = read(&path).unwrap();
        let app = find(&state, id);
        assert!(
            app.enabled && app.quarantined.is_none(),
            "{:?}",
            app.quarantined
        );
        assert_eq!(app.signed_by.as_ref().unwrap().id, "srelens");
    }
    /// An expired catalog still reserves its namespaces but vouches for no key: a host kept
    /// from newer catalogs does not go on trusting a key they may have withdrawn (#559).
    #[test]
    fn an_expired_catalog_reserves_namespaces_but_vouches_for_no_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        fs::write(
            path.with_extension("catalog.json"),
            catalog::test_cache_expiring(
                &[
                    trust::testing::srelens_publisher(),
                    trust::testing::example_publisher(),
                ],
                json!([]),
                now(),
                "2020-01-01T00:00:00Z",
            ),
        )
        .unwrap();
        let (source, signature, key_id) = signed_by_example_labs("com.example-labs.gitops");
        let refused = mutate(
            &path,
            fake_core(),
            install_signed(&source, signature, Some(key_id)),
        )
        .err()
        .unwrap();
        assert!(refused.contains("expired on 2020-01-01"), "{refused}");
        let unsigned = Configure::Install {
            signature: None,
            key_id: None,
            manifest: source,
            grants: vec!["k8s.listCustomResource".into()],
            reviewed_revision: None,
        };
        let refused = mutate(&path, fake_core(), unsigned).err().unwrap();
        assert!(
            refused.contains("reserved for releases signed by Example Labs"),
            "{refused}"
        );
    }
    /// Namespaces are lowercase, and a local manifest's ID may not be: a change of case does
    /// not step outside a reservation.
    #[test]
    fn a_reserved_namespace_holds_whatever_the_case_of_the_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        let unsigned = Configure::Install {
            signature: None,
            key_id: None,
            manifest: manifest().replacen("org.example.argocd", "org.Srelens.argocd", 1),
            grants: vec!["k8s.listCustomResource".into()],
            reviewed_revision: None,
        };
        let refused = mutate(&path, fake_core(), unsigned).err().unwrap();
        assert!(
            refused.contains("reserved for releases signed by srelens"),
            "{refused}"
        );
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
        seed_retired_signed_app(&path);
        install(&path, core.clone());
        // The current local app remains usable; the authentic retired app is quarantined.
        let state = read(&path).unwrap();
        assert!(find(&state, "org.example.argocd").enabled);
        assert!(!find(&state, "org.srelens.argocd").enabled);
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
            "App ID org.srelens.argocd is reserved for releases signed by srelens"
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

        // A replacement must also target a supported API; the old signature is insufficient.
        assert!(mutate(&path, core, signed_argocd())
            .err()
            .unwrap()
            .contains("requires API ^0.1"));
    }
    #[test]
    fn rollback_refuses_an_unsigned_version_under_a_reserved_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        let core = fake_core();
        seed_retired_signed_app(&path);
        seed_retired_signed_app(&path);
        strip_proof(&path, "/plugins/0/history/0");
        let state = read(&path).unwrap();
        let app = find(&state, "org.srelens.argocd");
        assert!(!app.enabled && app.quarantined.is_some());
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
            refused.contains("reserved for releases signed by srelens"),
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
