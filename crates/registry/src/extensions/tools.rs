//! Installed apps' operations as MCP tools (#574): every reader, declared action and
//! sidecar operation of an app in use, as `plugin/<id>/<operation>`.
//!
//! One snapshot per inventory in this process, shared by every MCP server over it. A
//! snapshot is a registry of its own, built by [`PluginHost::register_tools`] from the
//! apps in use, whose schemas and annotations are the host's. Its tools do not call a
//! host capability themselves: each routes to the broker's own path for its kind, with
//! every check a call through that path makes, on every call —
//!
//! - a reader, through `extensions.read`'s ([`read_contribution`]): the app, its
//!   revision, the cluster it is enabled for, the CRD the cluster serves (#601), and a
//!   `network.http` binding's allowlist and secrets;
//! - a declared action, through `extensions.action`'s ([`resource::run_action`]): the
//!   same, the kind the reader it names lists, and the primitive's own guards;
//! - a sidecar operation, to the app's sidecar ([`AppSidecars`]), after the same
//!   inventory check and the operation's declared inputs.
//!
//! Whether a call may run at all is the MCP server's consent gate, which reads the
//! tool's annotations: the one host policy every gated tool goes through, and the one
//! a person answers in the desktop's host confirmation (#552).
//!
//! **Lifecycle.** Every announced inventory write in this process rebuilds the snapshot
//! when the apps in use changed — installed, updated, rolled back, enabled, disabled,
//! blocked, quarantined or removed — and bumps [`AppTools`]'s change counter, which the
//! MCP transports send `notifications/tools/list_changed` on. A change another process
//! made (the app's settings window, a headless `srelens --mcp-stdio`) is found the next
//! time a tool is listed or called, or by a push session's poll. The snapshot it
//! replaces is revoked ([`Registration::revoke`]): a caller still holding it sees its
//! tools and cannot run them.
use super::{
    columns, package, read_contribution, resource, sidecars::AppSidecars, validate_app, Inventory,
    Read, Store, MAX_INVENTORY_BYTES,
};
use serde_json::{Map, Value};
use srelens_capability::{BoxFuture, CapabilityError, Registry};
use srelens_plugin_host::{is_pod_target, PluginHost, Registration, SecretStore, ToolRoute};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How often a push session asks for the tools unprompted, so a change another process
/// made reaches its client. Reading an unchanged inventory costs one bounded file read
/// and a hash; the kubeconfig watcher polls about as often.
pub const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Installed apps' tools for one inventory in this process.
pub struct AppTools {
    inner: Arc<Inner>,
}

struct Inner {
    store: Store,
    core: Arc<Registry>,
    cache: Arc<srelens_kube::client_cache::ClientCache>,
    snapshots: columns::JoinCache,
    secrets: Arc<dyn SecretStore>,
    sidecars: Arc<AppSidecars>,
    /// The snapshot every call and listing reads; swapped whole, never changed.
    snapshot: Mutex<Arc<Registry>>,
    /// Held while the snapshot is checked or rebuilt, so two never interleave.
    built: Mutex<Built>,
    changes: tokio::sync::watch::Sender<u64>,
}

/// What the current snapshot was built from.
#[derive(Default)]
struct Built {
    registrations: Vec<Registration>,
    /// Each app in use, at its revision; `None` before the first build.
    apps: Option<Vec<(String, u64)>>,
    /// The SHA-256 of the saved inventory last read; `None` before the first.
    checked: Option<String>,
}

/// What one of an app's tools is, for routing.
#[derive(Clone)]
enum Kind {
    Reader,
    /// An action on the reader it names.
    Action {
        resource: String,
    },
    Operation,
}

impl AppTools {
    pub(super) fn new(
        store: Store,
        core: Arc<Registry>,
        cache: Arc<srelens_kube::client_cache::ClientCache>,
        snapshots: columns::JoinCache,
        secrets: Arc<dyn SecretStore>,
        sidecars: AppSidecars,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                store,
                core,
                cache,
                snapshots,
                secrets,
                sidecars: Arc::new(sidecars),
                snapshot: Mutex::new(Arc::new(Registry::new())),
                built: Mutex::default(),
                changes: tokio::sync::watch::channel(0).0,
            }),
        }
    }

    /// Check the inventory and rebuild the snapshot if the apps in use changed. Blocking.
    pub(super) fn refresh(&self) {
        self.inner.refresh();
    }

    /// Answer the calls of executable apps' sidecars through `host` (#573): its registry,
    /// who confirms a write, and the audit trail. An MCP host calls this once it holds its
    /// registry; a sidecar started before keeps the broker it was started with.
    pub fn serve_sidecars(&self, host: super::sidecars::SidecarHost) {
        self.inner.sidecars.serve(host);
    }

    pub(super) async fn call_native(
        &self,
        input: super::operations::CallOperation,
    ) -> Result<Value, CapabilityError> {
        let (app, params) = self.native_authority(&input).await?;
        if app.manifest.operation(&input.operation).and_then(|op| op.view.as_ref()).is_some_and(|view| view.stream) {
            return Err(CapabilityError::InvalidInput("Open a streaming operation through an app view".into()));
        }
        self.inner.sidecars.request(&app, &input.operation, params).await
    }

    pub(super) async fn native_authority(&self, input: &super::operations::CallOperation) -> Result<(super::Installed, Map<String, Value>), CapabilityError> {
        if input.context.is_empty() {
            return Err(CapabilityError::InvalidInput("An app operation needs a pinned cluster".into()));
        }
        let (state, index, context) = super::resolver_app(
            self.inner.store.clone(), &self.inner.core, &self.inner.cache,
            &input.id, input.revision, input.context.clone(),
        ).await?;
        if input.params.get("clusterId").is_some_and(|value| value.as_str() != Some(&context)) {
            return Err(CapabilityError::InvalidInput("The operation's cluster differs from its pinned cluster".into()));
        }
        let app = state.plugins[index].clone();
        // Native views currently serve reading apps. An app that can write
        // needs the operation-level consent its dynamic MCP tool already has.
        if !app.manifest.actions.is_empty() { return Err(CapabilityError::InvalidInput("This app's operations require confirmation through its installed-app tools".into())); }
        let operation = app.manifest.operation(&input.operation).ok_or_else(|| CapabilityError::InvalidInput("This app does not declare the operation".into()))?;
        let params = operation.check_input(&Value::Object(input.params.clone())).map_err(CapabilityError::InvalidInput)?;
        Ok((app, params))
    }

    pub(super) async fn open_native_stream(&self, app: &super::Installed, method: &str, params: Map<String, Value>) -> Result<srelens_plugin_host::sidecar::SidecarStream, CapabilityError> {
        self.inner.sidecars.open_stream(app, method, params).await
    }

    /// End at once the sidecars of apps `state` no longer holds. Blocking-safe.
    pub(super) fn end_uninstalled(&self, state: &Inventory) {
        self.inner.sidecars.end_uninstalled(state);
    }

    /// Start sidecars with `launcher` from now on. Test support.
    #[cfg(test)]
    pub(super) fn script_sidecars(
        &self,
        launcher: Arc<dyn srelens_plugin_host::sidecar::Launcher>,
    ) {
        self.inner.sidecars.script(launcher);
    }
}

/// Each app in use, at its revision: what a snapshot is built from.
fn in_use(state: &Inventory) -> Vec<(String, u64)> {
    let mut apps: Vec<(String, u64)> = state
        .plugins
        .iter()
        .filter(|app| app.runs())
        .map(|app| (app.manifest.id.clone(), app.revision))
        .collect();
    apps.sort();
    apps
}

impl Inner {
    fn refresh(&self) {
        let mut built = self.built.lock().unwrap_or_else(|e| e.into_inner());
        // Unchanged bytes are unchanged apps: one bounded read and a hash, no parse.
        let saved = match self.store.load(MAX_INVENTORY_BYTES) {
            Ok(saved) => saved,
            Err(why) => {
                log::warn!("app tools: {why}");
                return;
            }
        };
        let digest = saved
            .as_deref()
            .map(package::sha256_hex)
            .unwrap_or_default();
        if built.checked.as_deref() == Some(digest.as_str()) {
            return;
        }
        // An inventory that cannot be read offers no tools: fail closed, as every read
        // through the broker does.
        let state = self.store.read().unwrap_or_else(|why| {
            log::warn!("app tools: {why}");
            Inventory::default()
        });
        built.checked = Some(digest);
        self.sidecars.reconcile(&state);
        // An app's tools follow from its manifest and grants at a revision, so the
        // same apps at the same revisions are the same tools: a settings save, a
        // cluster scope change or a switch the tools do not read rebuilds nothing.
        let apps = in_use(&state);
        if built.apps.as_ref() == Some(&apps) {
            return;
        }
        let (registry, registrations) = self.build(&state);
        // Nothing to tell a client when no tools came or went.
        let changed = !(registrations.is_empty() && built.registrations.is_empty());
        *self.snapshot.lock().unwrap_or_else(|e| e.into_inner()) = Arc::new(registry);
        for replaced in std::mem::replace(&mut built.registrations, registrations) {
            replaced.revoke();
        }
        built.apps = Some(apps);
        drop(built);
        if changed {
            self.changes.send_modify(|generation| *generation += 1);
        }
    }

    /// A snapshot of every app in use whose tools can be registered. An app that cannot
    /// — no longer valid against this host, say — is left out, and its calls through the
    /// broker are refused for the same reason.
    fn build(&self, state: &Inventory) -> (Registry, Vec<Registration>) {
        let host = PluginHost::new(self.core.clone());
        let mut registry = Registry::new();
        let mut registrations = Vec::new();
        for app in state.plugins.iter().filter(|app| app.runs()) {
            let id = &app.manifest.id;
            if let Err(problems) = validate_app(&app.manifest, &app.grants, self.core.clone()) {
                log::warn!("app tools: {id} offers none: {problems}");
                continue;
            }
            match host.register_tools(&mut registry, &app.manifest, &app.grants, self.route(app)) {
                Ok(registration) => registrations.push(registration),
                Err(why) => log::warn!("app tools: {id} offers none: {why}"),
            }
        }
        (registry, registrations)
    }

    /// How `app`'s tools run, at its revision.
    fn route(&self, app: &super::Installed) -> ToolRoute {
        let manifest = &app.manifest;
        let mut kinds: HashMap<String, Kind> = manifest
            .capabilities
            .iter()
            .filter(|binding| !is_pod_target(&binding.target))
            .map(|binding| (binding.name.clone(), Kind::Reader))
            .collect();
        for action in &manifest.actions {
            let kind = Kind::Action {
                resource: action.resource.clone(),
            };
            kinds.insert(action.name.clone(), kind);
        }
        for operation in manifest
            .sidecar
            .iter()
            .flat_map(|sidecar| &sidecar.operations)
            .filter(|operation| !operation.view.as_ref().is_some_and(|view| view.stream))
        {
            kinds.insert(operation.name.clone(), Kind::Operation);
        }
        let kinds = Arc::new(kinds);
        let (id, revision) = (manifest.id.clone(), app.revision);
        let store = self.store.clone();
        let core = self.core.clone();
        let cache = self.cache.clone();
        let snapshots = self.snapshots.clone();
        let secrets = self.secrets.clone();
        let sidecars = self.sidecars.clone();
        Arc::new(
            move |name: String, input: Map<String, Value>| -> BoxFuture<_> {
                let kind = kinds.get(&name).cloned();
                let (id, store, core, cache) =
                    (id.clone(), store.clone(), core.clone(), cache.clone());
                let (snapshots, secrets, sidecars) =
                    (snapshots.clone(), secrets.clone(), sidecars.clone());
                Box::pin(async move {
                    // Checked by the tool against its schema: every input is a string.
                    let text = |key: &str| {
                        input
                            .get(key)
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned()
                    };
                    match kind {
                        Some(Kind::Reader) => {
                            let read = Read {
                                cursor: input.get("cursor").and_then(Value::as_str).map(str::to_owned),
                                use_crd_columns: false,
                                id,
                                revision,
                                capability: name,
                                context: text("context"),
                                namespace: text("namespace"),
                                card: None,
                                namespaces: Vec::new(),
                            };
                            read_contribution(store, core, cache, snapshots, secrets, read).await
                        }
                        Some(Kind::Action { resource }) => {
                            let action = resource::Action {
                                resource: resource::Selection {
                                    id,
                                    revision,
                                    capability: resource,
                                    context: text("context"),
                                    namespace: text("namespace"),
                                    name: text("name"),
                                },
                                action: name,
                                uid: text("uid"),
                                resource_version: text("resourceVersion"),
                            };
                            resource::run_action(store, core, cache, action).await
                        }
                        Some(Kind::Operation) => {
                            run_operation(store, core, &sidecars, &id, revision, &name, input).await
                        }
                        None => Err(CapabilityError::NotFound(format!("plugin/{id}/{name}"))),
                    }
                })
            },
        )
    }
}

/// A sidecar operation: the app still installed, on, at this revision and valid, the
/// operation still declared and the input still fitting its declaration, then the
/// request to the app's sidecar.
async fn run_operation(
    store: Store,
    core: Arc<Registry>,
    sidecars: &AppSidecars,
    id: &str,
    revision: u64,
    name: &str,
    input: Map<String, Value>,
) -> Result<Value, CapabilityError> {
    let state = tokio::task::spawn_blocking(move || store.read())
        .await
        .map_err(|e| CapabilityError::Handler(e.to_string()))?
        .map_err(CapabilityError::Handler)?;
    if let Some(reason) = state
        .plugins
        .iter()
        .find(|app| app.manifest.id == id)
        .and_then(|app| app.policy_blocked.as_ref())
    {
        return Err(CapabilityError::Handler(reason.clone()));
    }
    let app = state
        .plugins
        .iter()
        .find(|app| app.manifest.id == id && app.enabled && app.revision == revision)
        .ok_or_else(|| {
            CapabilityError::Handler(
                "App was disabled, removed or updated; list its tools again".into(),
            )
        })?;
    validate_app(&app.manifest, &app.grants, core)
        .map_err(|errors| CapabilityError::Handler(errors.to_string()))?;
    let operation = app.manifest.operation(name).ok_or_else(|| {
        CapabilityError::InvalidInput(format!("{id} declares no operation \"{name}\""))
    })?;
    let input = operation
        .check_input(&Value::Object(input))
        .map_err(CapabilityError::InvalidInput)?;
    sidecars.request(app, name, input).await
}

#[async_trait::async_trait]
impl srelens_mcp::ToolSource for AppTools {
    async fn tools(&self) -> Arc<Registry> {
        let inner = self.inner.clone();
        // A failed refresh leaves the snapshot as it was; its calls still check the
        // inventory through the broker.
        let _ = tokio::task::spawn_blocking(move || inner.refresh()).await;
        self.current()
    }

    fn current(&self) -> Arc<Registry> {
        self.inner
            .snapshot
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn changes(&self) -> tokio::sync::watch::Receiver<u64> {
        self.inner.changes.subscribe()
    }

    fn poll_interval(&self) -> Duration {
        POLL_INTERVAL
    }
}
