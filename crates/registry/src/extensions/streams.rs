//! App streams (#565): the extension broker's half of the generic stream
//! contract in `srelens_streams::app`.
//!
//! The host opens a stream on behalf of one **view** of an installed app — a
//! page, a detail tab, a panel — and the view owns it. This module decides
//! whether the app may open it (the same authority `extensions.read` checks:
//! installed, enabled, this revision, this cluster), builds the source, and
//! ends streams when the app's lifecycle moves: every inventory write, from
//! any registry in this process, is announced here, so a disable, an update,
//! a rollback or a removal ends exactly the streams the old state opened.
//!
//! Two sources: `read`, which re-runs one of the app's declared readers on an
//! interval through `extensions.read`'s own path, so every tick rechecks
//! everything a single read would; and `watch` (#566), which follows the kind
//! a declared reader lists and says when it changed, so the view reads again
//! through that same path. Logs, exec and port-forwards (#567) and metric
//! providers (#569) are further `source` kinds on the same wire; none of them
//! changes the frames.

use super::{columns, crd, read_contribution, resolver_app, Inventory, InventoryKey, Read, Store};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use srelens_capability::{Annotations, Capability, CapabilityError, Registry};
use srelens_kube::watch::{CustomWatchTarget, KindSignal};
use srelens_plugin_host::Binding;
use srelens_streams::app::{
    AppStreamMetrics, AppStreams, CloseReason, StreamEmitter, StreamLimits, StreamOwner,
};
use srelens_streams::EventSink;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;
use tokio::time::Instant;

/// Every app stream's channel starts with this, so a caller cannot point one
/// at another stream kind's channel (`exec:out:…`, `watch:…`).
pub const CHANNEL_PREFIX: &str = "extstream:";
const MAX_CHANNEL: usize = 200;
const MAX_VIEW: usize = 200;
/// A `read` source's interval, in seconds: the default, and the bounds.
const DEFAULT_INTERVAL: u64 = 15;
const MIN_INTERVAL: u64 = 5;
const MAX_INTERVAL: u64 = 300;

/// What `@srelens/core`'s `openExtensionView(…).open(…)` sends. Every
/// multi-word field is renamed to the wrapper's camelCase spelling, and
/// unknown fields — the snake_case spelling among them — are refused.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenStreamIn {
    /// The app's ID.
    pub id: String,
    /// The installed revision the view was rendered from.
    pub revision: u64,
    /// The view that opens the stream and owns it. Unique per mounted view.
    pub view: String,
    /// The channel the caller already listens on. Starts with `extstream:`.
    pub channel: String,
    pub context: String,
    #[serde(default)]
    pub namespace: String,
    pub source: StreamSourceIn,
}

/// What the stream carries.
#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum StreamSourceIn {
    /// Re-run a declared reader every `intervalSeconds` (5–300, default 15).
    #[serde(rename = "read")]
    Read {
        capability: String,
        #[serde(default, rename = "intervalSeconds")]
        interval_seconds: Option<u64>,
    },
    /// Follow the kind a declared reader lists, and say when it changed (#566).
    #[serde(rename = "watch")]
    Watch { capability: String },
}

/// The channel inventory announcements reach a window on (#566).
pub const INVENTORY_CHANNEL: &str = "extensions:inventory";

/// What one watch session is asked to follow.
pub struct WatchScope {
    /// The pinned context the reader would read through.
    pub context: String,
    /// The view's namespace; empty for every namespace. Ignored for a cluster-scoped kind.
    pub namespace: String,
    pub target: CustomWatchTarget,
    /// Where the session reports what it saw. Never carries an object.
    pub signals: UnboundedSender<KindSignal>,
}

/// Starts one watch session: a list, then follow, returning at its first
/// error. The host's is `srelens_kube::watch::watch_kind_once`; tests script one.
pub type WatchSession = Arc<
    dyn Fn(WatchScope) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> + Send + Sync,
>;

/// A watch source's clock: how long `changed` frames are coalesced for, and
/// how long a lost watch waits before it lists again (doubling to the cap).
#[derive(Clone, Copy, Debug)]
pub struct WatchTiming {
    pub window: Duration,
    pub backoff: Duration,
    pub max_backoff: Duration,
}

impl Default for WatchTiming {
    fn default() -> Self {
        Self {
            window: Duration::from_secs(1),
            backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(30),
        }
    }
}

fn kube_session(cache: Arc<srelens_kube::client_cache::ClientCache>) -> WatchSession {
    Arc::new(move |scope: WatchScope| {
        let cache = cache.clone();
        Box::pin(async move {
            let signals = scope.signals;
            srelens_kube::watch::watch_kind_once(
                cache,
                scope.context,
                scope.namespace,
                scope.target,
                move |signal| {
                    let _ = signals.send(signal);
                },
            )
            .await
        })
    })
}

/// The kind `binding` lists, as the host knows it — never as the app says,
/// for a built-in reader — or why this binding has no kind to watch.
fn watched_kind(binding: &Binding) -> Result<CustomWatchTarget, String> {
    let text = |key: &str| {
        binding
            .arguments
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("The reader \"{}\" binds no {key}", binding.name))
    };
    if binding.target == "k8s.listCustomResource" {
        return Ok(CustomWatchTarget {
            group: text("group")?,
            version: text("version")?,
            kind: text("kind")?,
            plural: text("plural")?,
            // Install refuses a binding without it; refused here too, rather
            // than read as cluster-wide.
            namespaced: binding
                .arguments
                .get("namespaced")
                .and_then(Value::as_bool)
                .ok_or_else(|| {
                    format!("The reader \"{}\" binds no namespaced scope", binding.name)
                })?,
        });
    }
    if binding.target == "k8s.listEvents" {
        return Ok(CustomWatchTarget {
            group: String::new(),
            version: "v1".into(),
            kind: "Event".into(),
            plural: "events".into(),
            namespaced: true,
        });
    }
    let identity = srelens_plugin_host::builtin_reader_identity(&binding.target)
        .ok_or_else(|| format!("{} lists no kind this host can watch", binding.target))?;
    let field = |key: &str| identity[key].as_str().unwrap_or_default().to_owned();
    Ok(CustomWatchTarget {
        group: field("group"),
        version: field("version"),
        kind: field("kind"),
        plural: field("plural"),
        namespaced: identity["namespaced"] == true,
    })
}

/// The reply to an open.
#[derive(Debug, Serialize)]
pub struct OpenStreamOut {
    pub stream: String,
    pub channel: String,
}

/// The app streams of one extension inventory, shared by every registry in
/// this process that serves it: the desktop UI's and an MCP server's registry
/// see the same streams, and a lifecycle change made through either ends them.
pub struct ExtensionStreams {
    path: Store,
    core: Arc<Registry>,
    cache: Arc<srelens_kube::client_cache::ClientCache>,
    snapshots: columns::JoinCache,
    streams: AppStreams,
    /// How a watch source follows a kind; replaced by tests.
    watcher: Mutex<WatchSession>,
    timing: Mutex<WatchTiming>,
    /// Bumped on every announced inventory write, after the lifecycle's own
    /// endings: every watch rechecks what it may still follow.
    inventory: tokio::sync::watch::Sender<u64>,
    /// Windows told of every announced inventory write (#566).
    listeners: Mutex<Vec<Arc<dyn EventSink>>>,
}

impl ExtensionStreams {
    /// Open a stream for one view. `input` is the caller's JSON, parsed here
    /// so the host command and the tests read it the same way.
    pub async fn open(
        &self,
        sink: Arc<dyn EventSink>,
        input: Value,
    ) -> Result<OpenStreamOut, String> {
        let input: OpenStreamIn =
            serde_json::from_value(input).map_err(|e| format!("invalid stream request: {e}"))?;
        check_channel(&input.channel)?;
        if input.view.trim().is_empty() || input.view.len() > MAX_VIEW {
            return Err(format!(
                "A stream needs the view that owns it: a non-empty view id of at most {MAX_VIEW} characters"
            ));
        }
        let (capability, interval_seconds) = match &input.source {
            StreamSourceIn::Read {
                capability,
                interval_seconds,
            } => (capability.clone(), *interval_seconds),
            StreamSourceIn::Watch { .. } => return self.open_watch(sink, input).await,
        };
        let interval = interval_seconds.unwrap_or(DEFAULT_INTERVAL);
        if !(MIN_INTERVAL..=MAX_INTERVAL).contains(&interval) {
            return Err(format!(
                "A read stream's interval must be between {MIN_INTERVAL} and {MAX_INTERVAL} seconds, not {interval}"
            ));
        }
        // Authorized here, so a view rendered from a stale revision hears it
        // from `open` rather than from its first frame. Every tick checks again.
        let (state, index, _) = resolver_app(
            self.path.clone(),
            &self.core,
            &self.cache,
            &input.id,
            input.revision,
            input.context.clone(),
        )
        .await
        .map_err(|e| e.to_string())?;
        if !state.plugins[index]
            .manifest
            .capabilities
            .iter()
            .any(|binding| binding.name == capability)
        {
            return Err(format!(
                "App {} declares no reader named \"{capability}\"",
                input.id
            ));
        }
        let owner = StreamOwner {
            app: input.id.clone(),
            revision: input.revision,
            view: input.view,
        };
        let ask = ReadAsk {
            id: input.id,
            revision: input.revision,
            capability,
            context: input.context,
            namespace: input.namespace,
        };
        let (path, core, cache, snapshots) = (
            self.path.clone(),
            self.core.clone(),
            self.cache.clone(),
            self.snapshots.clone(),
        );
        let stream = self
            .streams
            .open(
                owner,
                "read",
                sink,
                input.channel.clone(),
                move |tx| async move {
                    loop {
                        let value = read_contribution(
                            path.clone(),
                            core.clone(),
                            cache.clone(),
                            snapshots.clone(),
                            ask.read(),
                        )
                        .await
                        .map_err(|e| e.to_string())?;
                        // Refused only when the stream already ended; its terminal
                        // frame is sent, so there is nothing left to say.
                        if tx.data(value).is_err() {
                            return Ok(());
                        }
                        tokio::time::sleep(Duration::from_secs(interval)).await;
                    }
                },
            )
            .map_err(|e| e.to_string())?;
        Ok(OpenStreamOut {
            stream,
            channel: input.channel,
        })
    }

    /// Cancel one stream. Idempotent: `false` when it had already ended.
    pub fn cancel(&self, stream: &str) -> bool {
        self.streams.cancel(stream)
    }

    /// End every stream `view` opened. Returns how many.
    pub fn close_view(&self, view: &str) -> usize {
        self.streams.close_view(view)
    }

    pub fn metrics(&self) -> Vec<AppStreamMetrics> {
        self.streams.metrics()
    }

    /// Tell `sink` of every inventory write this process announces, as
    /// `{ "type": "changed" }` on [`INVENTORY_CHANNEL`]. The desktop host
    /// passes its window's sink, so the app list reads again instead of polling.
    pub fn listen_inventory(&self, sink: Arc<dyn EventSink>) {
        self.listeners.lock().unwrap().push(sink);
    }

    /// Replace how watches follow a kind, and their clock. Test support.
    #[cfg(test)]
    pub(super) fn script_watches(&self, session: WatchSession, timing: WatchTiming) {
        *self.watcher.lock().unwrap() = session;
        *self.timing.lock().unwrap() = timing;
    }

    /// Open a `watch` source: authorized as a read would be, then a
    /// [`WatchSession`] per list, reconnected afresh until it fails for good.
    async fn open_watch(
        &self,
        sink: Arc<dyn EventSink>,
        input: OpenStreamIn,
    ) -> Result<OpenStreamOut, String> {
        let StreamSourceIn::Watch { capability } = &input.source else {
            unreachable!("open_watch is called for a watch source");
        };
        if !input.namespace.is_empty() && !super::cards::namespace_name(&input.namespace) {
            return Err("Namespace must be a Kubernetes namespace name".into());
        }
        let ask = WatchAsk {
            id: input.id.clone(),
            revision: input.revision,
            capability: capability.clone(),
            context: input.context.clone(),
        };
        let (context, binding) = ask.authorize(&self.path, &self.core, &self.cache).await?;
        // At the open, either refusal refuses it, as a read would be refused.
        let target = match WatchAsk::target(&self.core, &context, &binding).await {
            crd::Served::Yes(target) => target,
            crd::Served::No(why) | crd::Served::Unknown(why) => return Err(why),
        };
        // The reader's own scope: a binding that takes no namespace lists
        // every one, and so is watched in every one.
        let namespace = if binding.inputs.iter().any(|input| input == "namespace") {
            input.namespace.clone()
        } else {
            String::new()
        };
        let owner = StreamOwner {
            app: input.id.clone(),
            revision: input.revision,
            view: input.view,
        };
        let follow = Follow {
            ask,
            context,
            namespace,
            target,
            binding,
            path: self.path.clone(),
            core: self.core.clone(),
            cache: self.cache.clone(),
            snapshots: self.snapshots.clone(),
            session: self.watcher.lock().unwrap().clone(),
            timing: *self.timing.lock().unwrap(),
            inventory: self.inventory.subscribe(),
        };
        let stream = self
            .streams
            .open(owner, "watch", sink, input.channel.clone(), move |tx| {
                follow.run(tx)
            })
            .map_err(|e| e.to_string())?;
        Ok(OpenStreamOut {
            stream,
            channel: input.channel,
        })
    }

    /// End the streams `state` no longer authorizes: an app that is gone, is
    /// off (by a person, by policy or by quarantine), or is at another revision.
    fn reconcile(&self, state: &Inventory) {
        self.streams.end_where(|owner| {
            match state.plugins.iter().find(|p| p.manifest.id == owner.app) {
                None => Some(CloseReason::AppRemoved),
                Some(p) if !p.enabled || p.policy_blocked.is_some() || p.quarantined.is_some() => {
                    Some(CloseReason::AppDisabled)
                }
                Some(p) if p.revision != owner.revision => Some(CloseReason::AppUpdated),
                Some(_) => None,
            }
        });
        // After the endings above, so a stream the lifecycle ended hears
        // that, and not a recheck's refusal.
        self.inventory.send_modify(|generation| *generation += 1);
        let listeners = self.listeners.lock().unwrap().clone();
        for sink in listeners {
            sink.emit(INVENTORY_CHANNEL, json!({ "type": "changed" }));
        }
    }
}

/// What a watch stream follows, and may keep following only while this app
/// may still read it.
struct WatchAsk {
    id: String,
    revision: u64,
    capability: String,
    context: String,
}

impl WatchAsk {
    /// The app's authority a read of the binding checks — installed, enabled,
    /// this revision, this cluster, the binding valid against its grants —
    /// answered with the pinned context and the binding. Asks the inventory,
    /// not the cluster; [`WatchAsk::kind_served`] asks the cluster.
    async fn authorize(
        &self,
        path: &Store,
        core: &Arc<Registry>,
        cache: &Arc<srelens_kube::client_cache::ClientCache>,
    ) -> Result<(String, Binding), String> {
        if self.context.trim().is_empty() {
            return Err("An explicit cluster context is required".into());
        }
        let (state, index, context) = resolver_app(
            path.clone(),
            core,
            cache,
            &self.id,
            self.revision,
            self.context.clone(),
        )
        .await
        .map_err(|e| e.to_string())?;
        let binding = state.plugins[index]
            .manifest
            .capabilities
            .iter()
            .find(|binding| binding.name == self.capability)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "App {} declares no reader named \"{}\"",
                    self.id, self.capability
                )
            })?;
        Ok((context, binding))
    }

    /// What the watch follows on this cluster: the binding's kind at the
    /// version a read of it resolves to now (#547) — the first version it
    /// accepts that the CRD serves — or why there is none. Every other
    /// binding's kind is built in and needs no lookup.
    async fn target(
        core: &Registry,
        context: &str,
        binding: &Binding,
    ) -> crd::Served<CustomWatchTarget> {
        if binding.target != "k8s.listCustomResource" {
            return match watched_kind(binding) {
                Ok(target) => crd::Served::Yes(target),
                Err(why) => crd::Served::No(why),
            };
        }
        let version = match crd::serves(core, context, binding).await {
            crd::Served::Yes(version) => version,
            crd::Served::No(why) => return crd::Served::No(why),
            crd::Served::Unknown(why) => return crd::Served::Unknown(why),
        };
        // The version is bound where a read binds it, never taken from the app.
        let mut at = binding.clone();
        at.arguments.insert("version".into(), json!(version));
        match watched_kind(&at) {
            Ok(target) => crd::Served::Yes(target),
            Err(why) => crd::Served::No(why),
        }
    }
}

/// One watch stream's source.
struct Follow {
    ask: WatchAsk,
    context: String,
    namespace: String,
    target: CustomWatchTarget,
    binding: Binding,
    path: Store,
    core: Arc<Registry>,
    cache: Arc<srelens_kube::client_cache::ClientCache>,
    snapshots: columns::JoinCache,
    session: WatchSession,
    timing: WatchTiming,
    inventory: tokio::sync::watch::Receiver<u64>,
}

/// Why a watch source stops sending: its stream ended, so return quietly.
struct Ended;

impl Follow {
    /// Send one frame, first dropping the shared reader snapshot when the
    /// frame asks the view to read again, so the read lists the cluster.
    fn send(&self, tx: &StreamEmitter, event: Value, reread: bool) -> Result<(), Ended> {
        if reread {
            columns::forget_reader(
                &self.snapshots,
                &self.ask.id,
                self.ask.revision,
                &self.ask.capability,
            );
        }
        // Refused only once the stream ended — cancelled, closed, or stopped
        // for its rate with `error: rateLimited` — whose terminal frame is sent.
        tx.data(event).map_err(|_| Ended)
    }

    async fn run(mut self, tx: StreamEmitter) -> Result<(), String> {
        let mut backoff = self.timing.backoff;
        let mut reconnecting = false;
        loop {
            let (signals, mut heard) = tokio::sync::mpsc::unbounded_channel();
            let mut session = (self.session)(WatchScope {
                context: self.context.clone(),
                namespace: self.namespace.clone(),
                target: self.target.clone(),
                signals,
            });
            let mut last: Option<Instant> = None;
            let mut flush: Option<Instant> = None;
            let lost = loop {
                let due = flush;
                tokio::select! {
                    biased;
                    Some(signal) = heard.recv() => match signal {
                        KindSignal::Listed => {
                            reconnecting = false;
                            backoff = self.timing.backoff;
                            flush = None;
                            last = Some(Instant::now());
                            if self.send(&tx, json!({"event": "synced"}), true).is_err() {
                                return Ok(());
                            }
                        }
                        KindSignal::Changed if flush.is_some() => {}
                        KindSignal::Changed => {
                            let now = Instant::now();
                            match last {
                                Some(at) if now < at + self.timing.window => {
                                    flush = Some(at + self.timing.window);
                                }
                                _ => {
                                    last = Some(now);
                                    if self.send(&tx, json!({"event": "changed"}), true).is_err() {
                                        return Ok(());
                                    }
                                }
                            }
                        }
                    },
                    () = async { tokio::time::sleep_until(due.unwrap()).await }, if due.is_some() => {
                        flush = None;
                        last = Some(Instant::now());
                        if self.send(&tx, json!({"event": "changed"}), true).is_err() {
                            return Ok(());
                        }
                    }
                    changed = self.inventory.changed() => {
                        if changed.is_err() {
                            return Ok(()); // The host's streams are going away.
                        }
                        self.ask.authorize(&self.path, &self.core, &self.cache).await?;
                    }
                    result = &mut session => break match result {
                        Ok(()) => "The watch ended".to_owned(),
                        Err(message) => message,
                    },
                }
            };
            drop(session);
            // Forbidden does not heal: the stream ends with why.
            if srelens_kube::watch::is_forbidden_kind_watch_error(&lost) {
                return Err(lost);
            }
            // The kind is no longer served at the version followed: a read would
            // now resolve again (#547), and so does the watch, below. Only when it
            // resolves to the same kind again is the loss the answer.
            let unserved = srelens_kube::watch::is_permanent_kind_watch_error(&lost);
            let gone = lost.clone();
            let mut lost = lost;
            // Starting again is following again: only while the app may, and
            // only while the cluster serves the kind — at the version a read
            // resolves to now. A check the cluster could not answer is one more
            // failed attempt, not an ending.
            loop {
                if !reconnecting {
                    reconnecting = true;
                    let frame = json!({"event": "reconnecting", "message": lost});
                    if self.send(&tx, frame, false).is_err() {
                        return Ok(());
                    }
                }
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(self.timing.max_backoff);
                self.ask
                    .authorize(&self.path, &self.core, &self.cache)
                    .await?;
                match WatchAsk::target(&self.core, &self.context, &self.binding).await {
                    crd::Served::Yes(target) if unserved && target == self.target => {
                        // The loss itself, not a later lookup's: it is what says why.
                        let lost = gone;
                        return Err(lost);
                    }
                    crd::Served::Yes(target) => {
                        self.target = target;
                        break;
                    }
                    crd::Served::No(why) => return Err(why),
                    crd::Served::Unknown(why) => lost = why,
                }
            }
        }
    }
}

/// One `extensions.read` call, kept so a stream can make it again each tick.
struct ReadAsk {
    id: String,
    revision: u64,
    capability: String,
    context: String,
    namespace: String,
}

impl ReadAsk {
    fn read(&self) -> Read {
        Read {
            use_crd_columns: false,
            id: self.id.clone(),
            revision: self.revision,
            capability: self.capability.clone(),
            context: self.context.clone(),
            namespace: self.namespace.clone(),
            card: None,
            namespaces: vec![],
        }
    }
}

/// A channel a host can emit on as a Tauri event name, reserved for app streams.
fn check_channel(channel: &str) -> Result<(), String> {
    let Some(rest) = channel.strip_prefix(CHANNEL_PREFIX) else {
        return Err(format!(
            "A stream channel must start with \"{CHANNEL_PREFIX}\""
        ));
    };
    if rest.is_empty()
        || channel.len() > MAX_CHANNEL
        || !rest
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'/' | b':' | b'_'))
    {
        return Err(format!(
            "A stream channel is \"{CHANNEL_PREFIX}\" and at most {MAX_CHANNEL} characters of letters, digits, '-', '/', ':' and '_'"
        ));
    }
    Ok(())
}

/// The live [`ExtensionStreams`] per inventory.
fn live() -> &'static Mutex<HashMap<InventoryKey, Weak<ExtensionStreams>>> {
    static LIVE: OnceLock<Mutex<HashMap<InventoryKey, Weak<ExtensionStreams>>>> = OnceLock::new();
    LIVE.get_or_init(Default::default)
}

/// Tell the streams of the inventory `key` names that it was written as `state`.
pub(super) fn announce(key: &InventoryKey, state: &Inventory) {
    let streams = live().lock().unwrap().get(key).and_then(Weak::upgrade);
    if let Some(streams) = streams {
        streams.reconcile(state);
    }
}

/// What `extensions.streams` answers: the metrics, and the limits they are held to.
#[derive(Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct StreamsOut {
    apps: Vec<AppStreamMetrics>,
    max_open_per_app: usize,
    messages_per_second: u32,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct Empty {}

/// Join (or start) the streams of the inventory at `path`, and register
/// `extensions.streams`, the read path the Inspector (#575) reads open streams and
/// traffic from.
pub(super) fn register(
    reg: &mut Registry,
    path: Store,
    core: Arc<Registry>,
    cache: Arc<srelens_kube::client_cache::ClientCache>,
    snapshots: columns::JoinCache,
) -> Arc<ExtensionStreams> {
    let streams = {
        let key = path.key();
        let mut live = live().lock().unwrap();
        match live.get(&key).and_then(Weak::upgrade) {
            Some(streams) => streams,
            None => {
                let streams = Arc::new(ExtensionStreams {
                    path: path.clone(),
                    core,
                    watcher: Mutex::new(kube_session(cache.clone())),
                    cache,
                    snapshots,
                    streams: AppStreams::new(StreamLimits::default()),
                    timing: Mutex::new(WatchTiming::default()),
                    inventory: tokio::sync::watch::channel(0).0,
                    listeners: Mutex::new(Vec::new()),
                });
                live.retain(|_, weak| weak.strong_count() > 0);
                live.insert(key, Arc::downgrade(&streams));
                streams
            }
        }
    };
    let metrics = streams.clone();
    reg.register(Capability::typed::<Empty, StreamsOut, _, _>(
        "extensions.streams",
        "Report the open app streams in this process and the traffic each app has sent",
        Annotations::READ_ONLY,
        move |_| {
            let metrics = metrics.clone();
            async move {
                let limits = metrics.streams.limits();
                Ok::<_, CapabilityError>(StreamsOut {
                    apps: metrics.metrics(),
                    max_open_per_app: limits.max_open_per_app,
                    messages_per_second: limits.messages_per_second,
                })
            }
        },
    ));
    streams
}

#[cfg(test)]
mod tests {
    use super::super::tests::{configure, fake_core, install, manifest};
    use super::*;
    use serde_json::json;
    use srelens_streams::test_util::TestSink;
    use std::path::{Path, PathBuf};

    /// What `@srelens/core` sends, byte for byte: `extensionStreams.test.ts`
    /// holds the wrapper to this same file.
    const WRAPPER_PAYLOAD: &str =
        include_str!("../../../../packages/core/src/lib/extension-stream-open.json");
    const APP: &str = "org.example.argocd";
    const OTHER: &str = "org.example.second";

    fn setup(dir: &Path) -> (PathBuf, Registry, Arc<ExtensionStreams>) {
        let path = dir.join("extensions.json");
        let mut reg = Registry::new();
        let streams = super::super::register(
            &mut reg,
            path.clone(),
            fake_core(),
            srelens_kube::client_cache::ClientCache::new_many(vec![]),
        );
        (path, reg, streams)
    }

    fn install_other(path: &Path) -> u64 {
        let state = configure(
            path,
            json!({"action": "install", "manifest": manifest().replace(APP, OTHER),
                   "grants": ["k8s.listCustomResource"]}),
        )
        .unwrap();
        state
            .plugins
            .iter()
            .find(|p| p.manifest.id == OTHER)
            .unwrap()
            .revision
    }

    fn request(id: &str, revision: u64, view: &str, channel: &str) -> Value {
        json!({
            "id": id, "revision": revision, "view": view, "channel": channel,
            "context": "cluster/a", "namespace": "team",
            "source": {"kind": "read", "capability": "applications"},
        })
    }

    async fn eventually(what: &str, check: impl Fn() -> bool) {
        for _ in 0..300 {
            if check() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("never happened: {what}");
    }

    fn types(sink: &TestSink, channel: &str) -> Vec<String> {
        sink.payloads_for(channel)
            .iter()
            .map(|f| f["type"].as_str().unwrap_or_default().to_owned())
            .collect()
    }

    /// Open and wait for the first tick, so a later frame is the lifecycle's.
    async fn open(streams: &ExtensionStreams, sink: &Arc<TestSink>, input: Value) -> String {
        let channel = input["channel"].as_str().unwrap().to_owned();
        let out = streams.open(sink.clone(), input).await.unwrap();
        assert_eq!(out.channel, channel);
        eventually("first data", || {
            types(sink, &channel).contains(&"data".to_owned())
        })
        .await;
        out.stream
    }

    #[test]
    fn the_wrappers_payload_deserializes_and_snake_case_is_rejected() {
        let payload: Value = serde_json::from_str(WRAPPER_PAYLOAD).unwrap();
        let input: OpenStreamIn = serde_json::from_value(payload.clone()).unwrap();
        assert_eq!(input.id, APP);
        assert_eq!(input.view, "org.example.argocd/page:applications#1");
        assert_eq!(input.channel, "extstream:1-k2j3h4");
        assert_eq!(input.namespace, "team");
        assert_eq!(
            input.source,
            StreamSourceIn::Read {
                capability: "applications".into(),
                interval_seconds: Some(30)
            }
        );
        let mut snake = payload.clone();
        snake["source"] =
            json!({"kind": "read", "capability": "applications", "interval_seconds": 30});
        let error = serde_json::from_value::<OpenStreamIn>(snake)
            .unwrap_err()
            .to_string();
        assert!(error.contains("interval_seconds"), "{error}");
        let mut unknown = payload;
        unknown["viewId"] = json!("x");
        assert!(serde_json::from_value::<OpenStreamIn>(unknown).is_err());
    }

    /// The real source end to end: the app's own reader, through
    /// `extensions.read`'s path, and its traffic in `extensions.streams`.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_read_stream_sends_the_readers_answer_and_is_counted() {
        let dir = tempfile::tempdir().unwrap();
        let (path, reg, streams) = setup(dir.path());
        let revision = install(&path, fake_core());
        let sink = Arc::new(TestSink::default());
        let stream = open(
            &streams,
            &sink,
            request(APP, revision, "v1", "extstream:read"),
        )
        .await;
        let frames = sink.payloads_for("extstream:read");
        assert_eq!(frames[0], json!({"type": "open", "stream": stream}));
        // The fake reader answers with the arguments the broker sent it.
        assert_eq!(frames[1]["data"]["plural"], "applications", "{}", frames[1]);
        assert_eq!(frames[1]["data"]["namespace"], "team", "{}", frames[1]);
        let out = reg.invoke("extensions.streams", json!({})).await.unwrap();
        assert_eq!(out["maxOpenPerApp"], 8);
        assert_eq!(out["messagesPerSecond"], 50);
        assert_eq!(out["apps"][0]["app"], APP, "{out}");
        assert_eq!(out["apps"][0]["openStreams"], 1, "{out}");
        assert_eq!(out["apps"][0]["streams"][0]["view"], "v1", "{out}");
        assert_eq!(out["apps"][0]["streams"][0]["source"], "read", "{out}");
        assert!(out["apps"][0]["bytes"].as_u64().unwrap() > 0, "{out}");
        assert!(streams.cancel(&stream));
        assert!(!streams.cancel(&stream), "cancel is idempotent");
        assert_eq!(types(&sink, "extstream:read"), ["open", "data", "close"]);
    }

    /// Say what you know: a read that fails ends the stream with an error.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_failed_read_is_an_error_frame_not_a_close() {
        let dir = tempfile::tempdir().unwrap();
        let (path, _reg, streams) = setup(dir.path());
        let revision = install(&path, fake_core());
        let sink = Arc::new(TestSink::default());
        let mut input = request(APP, revision, "v", "extstream:bad");
        input["namespace"] = json!("Not_A_Namespace");
        streams.open(sink.clone(), input).await.unwrap();
        eventually("error", || types(&sink, "extstream:bad").len() == 2).await;
        let error = &sink.payloads_for("extstream:bad")[1];
        assert_eq!(error["type"], "error");
        assert_eq!(error["code"], "source");
        assert!(
            error["message"]
                .as_str()
                .unwrap()
                .contains("Namespace must be"),
            "{error}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn disabling_an_app_ends_every_stream_it_opened_and_only_those() {
        let dir = tempfile::tempdir().unwrap();
        let (path, _reg, streams) = setup(dir.path());
        let revision = install(&path, fake_core());
        let other = install_other(&path);
        let sink = Arc::new(TestSink::default());
        open(
            &streams,
            &sink,
            request(APP, revision, "page", "extstream:a1"),
        )
        .await;
        open(
            &streams,
            &sink,
            request(APP, revision, "tab", "extstream:a2"),
        )
        .await;
        open(
            &streams,
            &sink,
            request(OTHER, other, "page-b", "extstream:b1"),
        )
        .await;
        configure(
            &path,
            json!({"action": "enable", "id": APP, "enabled": false}),
        )
        .unwrap();
        for channel in ["extstream:a1", "extstream:a2"] {
            assert_eq!(types(&sink, channel).last().unwrap(), "close", "{channel}");
            assert_eq!(
                sink.payloads_for(channel).last().unwrap()["reason"],
                "appDisabled"
            );
        }
        assert!(
            !types(&sink, "extstream:b1").contains(&"close".to_owned()),
            "another app's stream stays open"
        );
        let open_apps: Vec<_> = streams
            .metrics()
            .into_iter()
            .filter(|m| m.open_streams > 0)
            .map(|m| m.app)
            .collect();
        assert_eq!(open_apps, [OTHER]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_update_ends_the_old_revisions_streams_and_a_removal_ends_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let (path, _reg, streams) = setup(dir.path());
        let first = install(&path, fake_core());
        let other = install_other(&path);
        let sink = Arc::new(TestSink::default());
        open(&streams, &sink, request(APP, first, "old", "extstream:old")).await;
        open(&streams, &sink, request(OTHER, other, "b", "extstream:b")).await;
        let second = install(&path, fake_core());
        assert_ne!(first, second, "an update assigns a new revision");
        assert_eq!(
            sink.payloads_for("extstream:old").last().unwrap()["reason"],
            "appUpdated"
        );
        assert!(!types(&sink, "extstream:b").contains(&"close".to_owned()));
        // The old view cannot reopen on the revision it was rendered from.
        let stale = streams
            .open(sink.clone(), request(APP, first, "old", "extstream:stale"))
            .await
            .unwrap_err();
        assert!(stale.contains("refresh the view"), "{stale}");
        open(
            &streams,
            &sink,
            request(APP, second, "new", "extstream:new"),
        )
        .await;
        configure(&path, json!({"action": "remove", "id": APP})).unwrap();
        assert_eq!(
            sink.payloads_for("extstream:new").last().unwrap()["reason"],
            "appRemoved"
        );
        assert!(!types(&sink, "extstream:b").contains(&"close".to_owned()));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn closing_a_view_ends_its_streams_and_only_those() {
        let dir = tempfile::tempdir().unwrap();
        let (path, _reg, streams) = setup(dir.path());
        let revision = install(&path, fake_core());
        let sink = Arc::new(TestSink::default());
        open(
            &streams,
            &sink,
            request(APP, revision, "page#1", "extstream:p1"),
        )
        .await;
        open(
            &streams,
            &sink,
            request(APP, revision, "page#1", "extstream:p2"),
        )
        .await;
        open(
            &streams,
            &sink,
            request(APP, revision, "page#2", "extstream:p3"),
        )
        .await;
        assert_eq!(streams.close_view("page#1"), 2);
        assert_eq!(streams.close_view("page#1"), 0);
        for channel in ["extstream:p1", "extstream:p2"] {
            assert_eq!(
                sink.payloads_for(channel).last().unwrap()["reason"],
                "viewClosed"
            );
        }
        assert_eq!(types(&sink, "extstream:p3"), ["open", "data"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn open_is_refused_with_the_reason() {
        let dir = tempfile::tempdir().unwrap();
        let (path, _reg, streams) = setup(dir.path());
        let revision = install(&path, fake_core());
        let sink = Arc::new(TestSink::default());
        let refused = |input: Value| {
            let streams = &streams;
            let sink = sink.clone();
            async move { streams.open(sink, input).await.unwrap_err() }
        };
        let mut unknown = request(APP, revision, "v", "extstream:x");
        unknown["source"]["capability"] = json!("nope");
        assert!(refused(unknown)
            .await
            .contains("declares no reader named \"nope\""));
        let foreign = request(APP, revision, "v", "exec:out:1");
        assert!(refused(foreign).await.contains("extstream:"));
        let odd = request(APP, revision, "v", "extstream:a b");
        assert!(refused(odd).await.contains("channel"));
        let mut fast = request(APP, revision, "v", "extstream:x");
        fast["source"]["intervalSeconds"] = json!(1);
        assert!(refused(fast).await.contains("between 5 and 300 seconds"));
        let blank = request(APP, revision, "", "extstream:x");
        assert!(refused(blank).await.contains("view"));
        let snake = json!({"id": APP, "revision": revision, "view": "v", "channel": "extstream:x",
                           "context": "c", "source": {"kind": "read", "capability": "applications", "interval_seconds": 5}});
        assert!(refused(snake).await.starts_with("invalid stream request"));
        assert!(
            refused(request("org.example.missing", 1, "v", "extstream:x"))
                .await
                .contains("removed")
        );
        assert!(
            sink.payloads_for("extstream:x").is_empty(),
            "a refused open sends no frame"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_per_app_cap_refuses_one_more_stream_with_a_clear_error() {
        let dir = tempfile::tempdir().unwrap();
        let (path, reg, streams) = setup(dir.path());
        let revision = install(&path, fake_core());
        let sink = Arc::new(TestSink::default());
        for n in 0..8 {
            streams
                .open(
                    sink.clone(),
                    request(APP, revision, "v", &format!("extstream:{n}")),
                )
                .await
                .unwrap();
        }
        let refused = streams
            .open(sink.clone(), request(APP, revision, "v", "extstream:9"))
            .await
            .unwrap_err();
        assert!(refused.contains("already has 8 open streams"), "{refused}");
        let out = reg.invoke("extensions.streams", json!({})).await.unwrap();
        assert_eq!(out["apps"][0]["refused"], 1, "{out}");
        assert_eq!(streams.close_view("v"), 8);
    }

    // ---- The `watch` source (#566) ----

    const WATCH_PAYLOAD: &str =
        include_str!("../../../../packages/core/src/lib/extension-stream-watch.json");

    /// One step of a scripted watch session.
    #[derive(Clone)]
    enum Step {
        Signal(KindSignal),
        /// Changes sent back to back, with no pause between them, so the burst
        /// is inside one window by construction. A pause per signal is not:
        /// on Windows each 5 ms sleep takes a ~15.6 ms timer tick (#711).
        Burst(usize),
        Fail(&'static str),
    }
    use Step::{Burst, Fail, Signal};
    const LISTED: Step = Signal(KindSignal::Listed);
    const CHANGED: Step = Signal(KindSignal::Changed);

    type Seen = Arc<Mutex<Vec<(String, String, CustomWatchTarget)>>>;

    /// Sessions in order, one script each; a session past the last script
    /// lists and stays up. Records the scope every session was asked for.
    fn scripted(scripts: Vec<Vec<Step>>) -> (WatchSession, Seen) {
        let queue = Arc::new(Mutex::new(std::collections::VecDeque::from(scripts)));
        let seen: Seen = Arc::default();
        let record = seen.clone();
        let session: WatchSession = Arc::new(move |scope: WatchScope| {
            record.lock().unwrap().push((
                scope.context.clone(),
                scope.namespace.clone(),
                scope.target.clone(),
            ));
            let script = queue.lock().unwrap().pop_front().unwrap_or(vec![LISTED]);
            Box::pin(async move {
                for step in script {
                    match step {
                        Signal(signal) => {
                            let _ = scope.signals.send(signal);
                            tokio::time::sleep(Duration::from_millis(5)).await;
                        }
                        Burst(n) => {
                            for _ in 0..n {
                                let _ = scope.signals.send(KindSignal::Changed);
                            }
                        }
                        Fail(message) => return Err(message.to_owned()),
                    }
                }
                std::future::pending().await
            })
        });
        (session, seen)
    }

    const FAST: WatchTiming = WatchTiming {
        window: Duration::from_millis(300),
        backoff: Duration::from_millis(10),
        max_backoff: Duration::from_millis(40),
    };

    fn watch(id: &str, revision: u64, view: &str, channel: &str) -> Value {
        let mut input = request(id, revision, view, channel);
        input["source"] = json!({"kind": "watch", "capability": "applications"});
        input
    }

    /// The `data` of every data frame on `channel`.
    fn events(sink: &TestSink, channel: &str) -> Vec<Value> {
        sink.payloads_for(channel)
            .into_iter()
            .filter(|f| f["type"] == "data")
            .map(|f| f["data"].clone())
            .collect()
    }

    fn names(sink: &TestSink, channel: &str) -> Vec<String> {
        events(sink, channel)
            .iter()
            .map(|e| e["event"].as_str().unwrap_or_default().to_owned())
            .collect()
    }

    /// Event and built-in readers are watched as the host names their kind;
    /// a built-in reader's app-supplied arguments (refused at install anyway)
    /// could not change it.
    #[test]
    fn event_and_built_in_readers_watch_the_kind_the_host_names() {
        let binding = |target: &str, arguments: Value| Binding {
            name: "r".into(),
            title: "R".into(),
            target: target.into(),
            arguments: arguments.as_object().unwrap().clone(),
            inputs: vec!["context".into(), "namespace".into()],
            versions: vec![],
            json_path_overrides: Default::default(),
        };
        let events = watched_kind(&binding("k8s.listEvents", json!({}))).unwrap();
        assert_eq!(
            (
                events.group.as_str(),
                events.version.as_str(),
                events.plural.as_str()
            ),
            ("", "v1", "events")
        );
        assert!(events.namespaced);
        let deployments = watched_kind(&binding(
            "k8s.listDeployments",
            json!({"group": "", "plural": "secrets", "kind": "Secret"}),
        ))
        .unwrap();
        assert_eq!(
            (deployments.group.as_str(), deployments.plural.as_str()),
            ("apps", "deployments")
        );
        let nodes = watched_kind(&binding("k8s.listNodes", json!({}))).unwrap();
        assert!(
            !nodes.namespaced,
            "a cluster-scoped kind ignores the namespace"
        );
        assert!(watched_kind(&binding("k8s.listSecrets", json!({}))).is_err());
        // A custom resource's scope is the binding's to state, never guessed:
        // without it the reader cannot run, so there is nothing to watch.
        let crd = json!({"group": "argoproj.io", "version": "v1alpha1", "kind": "Application", "plural": "applications"});
        let unscoped = watched_kind(&binding("k8s.listCustomResource", crd.clone())).unwrap_err();
        assert!(unscoped.contains("namespaced"), "{unscoped}");
        let mut odd = crd.clone();
        odd["namespaced"] = json!("yes");
        assert!(watched_kind(&binding("k8s.listCustomResource", odd)).is_err());
        for scope in [true, false] {
            let mut scoped = crd.clone();
            scoped["namespaced"] = json!(scope);
            let target = watched_kind(&binding("k8s.listCustomResource", scoped)).unwrap();
            assert_eq!(target.namespaced, scope);
        }
    }

    #[test]
    fn the_wrappers_watch_payload_deserializes() {
        let payload: Value = serde_json::from_str(WATCH_PAYLOAD).unwrap();
        let input: OpenStreamIn = serde_json::from_value(payload.clone()).unwrap();
        assert_eq!(
            input.source,
            StreamSourceIn::Watch {
                capability: "applications".into()
            }
        );
        let mut extra = payload;
        extra["source"]["intervalSeconds"] = json!(5);
        assert!(
            serde_json::from_value::<OpenStreamIn>(extra).is_err(),
            "a watch has no interval"
        );
    }

    /// A watch follows the bound kind — the binding's group, version and
    /// plural — in the view's cluster and namespace, and nothing else.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_watch_follows_only_the_bound_kind_in_the_views_scope() {
        let dir = tempfile::tempdir().unwrap();
        let (path, _reg, streams) = setup(dir.path());
        let revision = install(&path, fake_core());
        let (session, seen) = scripted(vec![vec![LISTED]]);
        streams.script_watches(session, FAST);
        let sink = Arc::new(TestSink::default());
        streams
            .open(sink.clone(), watch(APP, revision, "v", "extstream:w"))
            .await
            .unwrap();
        eventually("synced", || names(&sink, "extstream:w") == ["synced"]).await;
        let (context, namespace, target) = seen.lock().unwrap()[0].clone();
        assert_eq!(context, "cluster/a");
        assert_eq!(namespace, "team");
        assert_eq!(
            target,
            CustomWatchTarget {
                group: "argoproj.io".into(),
                version: "v1alpha1".into(),
                kind: "Application".into(),
                plural: "applications".into(),
                namespaced: true,
            }
        );
        let refused = |input: Value| {
            let streams = &streams;
            let sink = sink.clone();
            async move { streams.open(sink, input).await.unwrap_err() }
        };
        let mut undeclared = watch(APP, revision, "v", "extstream:x");
        undeclared["source"]["capability"] = json!("secrets");
        assert!(refused(undeclared)
            .await
            .contains("declares no reader named \"secrets\""));
        let mut odd = watch(APP, revision, "v", "extstream:x");
        odd["namespace"] = json!("Not_A_Namespace");
        assert!(refused(odd).await.contains("Namespace must be"));
        let mut blank = watch(APP, revision, "v", "extstream:x");
        blank["context"] = json!(" ");
        assert!(refused(blank).await.contains("explicit cluster context"));
        // An app limited to other clusters may not follow this one.
        configure(
            &path,
            json!({"action": "clusters", "id": APP, "contexts": ["elsewhere"]}),
        )
        .unwrap();
        let scoped = refused(watch(APP, revision, "v", "extstream:x")).await;
        assert!(scoped.contains("enabled for this cluster"), "{scoped}");
        assert_eq!(
            seen.lock().unwrap().len(),
            1,
            "no refused open started a watch"
        );
        assert!(sink.payloads_for("extstream:x").is_empty());
    }

    /// A burst of changes inside one window is one `changed` frame, not one
    /// per event; a change after a quiet window is sent at once.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_burst_of_changes_is_coalesced_into_one_frame_per_window() {
        let dir = tempfile::tempdir().unwrap();
        let (path, _reg, streams) = setup(dir.path());
        let revision = install(&path, fake_core());
        let (session, _) = scripted(vec![vec![LISTED, Burst(20)]]);
        streams.script_watches(session, FAST);
        let sink = Arc::new(TestSink::default());
        streams
            .open(sink.clone(), watch(APP, revision, "v", "extstream:c"))
            .await
            .unwrap();
        eventually("the burst's frame", || {
            names(&sink, "extstream:c") == ["synced", "changed"]
        })
        .await;
        tokio::time::sleep(FAST.window * 2).await;
        assert_eq!(names(&sink, "extstream:c"), ["synced", "changed"]);
        for event in events(&sink, "extstream:c") {
            assert_eq!(
                event.as_object().unwrap().keys().collect::<Vec<_>>(),
                ["event"],
                "a frame names the event and nothing of an object: {event}"
            );
        }
    }

    /// A lost watch says so at once, and then lists again from nothing — a new
    /// session, never a resume — which says `synced`. `410 Gone` is one such loss.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_lost_watch_says_reconnecting_then_relists_and_410_is_one() {
        let dir = tempfile::tempdir().unwrap();
        let (path, _reg, streams) = setup(dir.path());
        let revision = install(&path, fake_core());
        let gone = "ErrorResponse { status: \"Failure\", message: \"too old resource version\", reason: \"Expired\", code: 410 }";
        let (session, seen) = scripted(vec![
            vec![LISTED, Fail("connection reset by peer")],
            vec![Fail("error trying to connect: connection refused")],
            vec![LISTED, CHANGED, Fail(gone)],
            vec![LISTED],
        ]);
        streams.script_watches(session, FAST);
        let sink = Arc::new(TestSink::default());
        streams
            .open(sink.clone(), watch(APP, revision, "v", "extstream:r"))
            .await
            .unwrap();
        eventually("two relists", || names(&sink, "extstream:r").len() == 5).await;
        // One `reconnecting` per loss, however many attempts it takes; and the
        // change still waiting out its window when the 410 came is not sent on
        // its own: the relist after it is what reports it.
        assert_eq!(
            names(&sink, "extstream:r"),
            ["synced", "reconnecting", "synced", "reconnecting", "synced"],
        );
        let frames = events(&sink, "extstream:r");
        assert_eq!(frames[1]["message"], "connection reset by peer");
        assert!(frames[3]["message"].as_str().unwrap().contains("410"));
        assert_eq!(
            seen.lock().unwrap().len(),
            4,
            "every attempt is a fresh list"
        );
        assert_eq!(
            types(&sink, "extstream:r").last().unwrap(),
            "data",
            "still open"
        );
    }

    const V1A1: CrdAnswer = Ok(Some("v1alpha1"));

    #[tokio::test(flavor = "multi_thread")]
    async fn a_crd_check_that_cannot_answer_on_reconnect_keeps_reconnecting() {
        let w = crd_on_reconnect(
            vec![V1A1, Err("apiserver timed out"), V1A1],
            vec![vec![LISTED, Fail("connection reset by peer")]],
            None,
        )
        .await;
        eventually("reconnect", || {
            names(&w.sink, "extstream:crd") == ["synced", "reconnecting", "synced"]
        })
        .await;
        // An inventory write asks the app's authority again, not the cluster.
        let before = w.answers.lock().unwrap().len();
        configure(
            &w.path,
            json!({"action": "settings", "id": APP, "settings": {}}),
        )
        .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(w.answers.lock().unwrap().len(), before);
        assert_eq!(names(&w.sink, "extstream:crd").len(), 3, "still following");
    }

    /// Only a CRD the cluster answers is gone ends the watch, with why.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_crd_gone_on_reconnect_ends_the_watch_with_why() {
        let w = crd_on_reconnect(
            vec![V1A1, Ok(None)],
            vec![vec![LISTED, Fail("connection reset by peer")]],
            None,
        )
        .await;
        eventually("error", || {
            last_error(&w.sink).is_some_and(|m| m.contains("No CustomResourceDefinition"))
        })
        .await;
    }

    /// #547: a binding that accepts several versions is watched at the one a
    /// read resolves to. When the cluster stops serving it, the watch resolves
    /// again, as a read would, and follows the next; when it serves none of
    /// them, the watch ends with the refusal a read gives.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_multi_version_binding_is_watched_at_the_version_a_read_resolves() {
        let unserved = "ApiError: the server could not find the requested resource: NotFound (ErrorResponse { status: \"Failure\", code: 404 })";
        let w = crd_on_reconnect(
            vec![Ok(Some("v1")), Ok(Some("v1alpha1")), Ok(None), Ok(None)],
            vec![vec![LISTED, Fail(unserved)], vec![LISTED, Fail(unserved)]],
            Some(json!(["v1", "v1alpha1"])),
        )
        .await;
        eventually("error", || last_error(&w.sink).is_some()).await;
        let versions: Vec<_> = w
            .seen
            .lock()
            .unwrap()
            .iter()
            .map(|(_, _, t)| t.version.clone())
            .collect();
        assert_eq!(
            versions,
            ["v1", "v1alpha1"],
            "the first served, then the next"
        );
        assert_eq!(
            names(&w.sink, "extstream:crd"),
            ["synced", "reconnecting", "synced", "reconnecting"]
        );
        // Exactly what a read of the binding says now.
        let read = w
            .reg
            .invoke(
                "extensions.read",
                json!({"id": APP, "revision": w.revision, "capability": "applications",
                       "context": "cluster/a", "namespace": "team"}),
            )
            .await
            .unwrap_err()
            .to_string();
        let watched = last_error(&w.sink).unwrap();
        assert!(read.contains(&watched), "read: {read}\nwatch: {watched}");
        assert!(watched.contains("any of v1, v1alpha1"), "{watched}");
    }

    /// A 404 on a kind that still resolves to the same version is the answer:
    /// the watch ends with it rather than reconnecting for ever.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_unserved_kind_that_resolves_the_same_ends_the_watch() {
        let w = crd_on_reconnect(
            vec![V1A1, V1A1],
            vec![vec![LISTED, Fail("404 Not Found")]],
            None,
        )
        .await;
        eventually("error", || {
            last_error(&w.sink).as_deref() == Some("404 Not Found")
        })
        .await;
    }

    type CrdAnswer = Result<Option<&'static str>, &'static str>;

    fn last_error(sink: &TestSink) -> Option<String> {
        let last = sink.payloads_for("extstream:crd").last().cloned()?;
        (last["type"] == "error").then(|| last["message"].as_str().unwrap_or_default().to_owned())
    }

    /// A watch opened by [`crd_on_reconnect`], and what it ran over.
    struct CrdWatch {
        sink: Arc<TestSink>,
        answers: Arc<Mutex<std::collections::VecDeque<CrdAnswer>>>,
        seen: Seen,
        reg: Registry,
        path: PathBuf,
        revision: u64,
        _dir: tempfile::TempDir,
    }

    /// A watch on the Argo CD reader, optionally accepting `versions`, over a
    /// cluster whose CRD check answers `answers` in turn (the first is the
    /// open's) and whose sessions follow `scripts`.
    async fn crd_on_reconnect(
        answers: Vec<CrdAnswer>,
        scripts: Vec<Vec<Step>>,
        versions: Option<Value>,
    ) -> CrdWatch {
        let answers = Arc::new(Mutex::new(std::collections::VecDeque::from(answers)));
        let checks = answers.clone();
        let mut core = (*fake_core()).clone();
        let mut check =
            crd::check_capability(srelens_kube::client_cache::ClientCache::new_many(vec![]));
        check.handler = Arc::new(move |_| {
            let next = checks.lock().unwrap().pop_front().unwrap_or(V1A1);
            Box::pin(async move {
                next.map(|served| json!(served))
                    .map_err(|e| CapabilityError::Handler(e.into()))
            })
        });
        core.register(check);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        let mut reg = Registry::new();
        let streams = super::super::register(
            &mut reg,
            path.clone(),
            Arc::new(core),
            srelens_kube::client_cache::ClientCache::new_many(vec![]),
        );
        let revision = match versions {
            None => install(&path, fake_core()),
            Some(versions) => {
                let mut manifest: Value = serde_json::from_str(&manifest()).unwrap();
                let reader = &mut manifest["capabilities"][0];
                reader["arguments"]
                    .as_object_mut()
                    .unwrap()
                    .remove("version");
                reader["versions"] = versions;
                let state = configure(
                    &path,
                    json!({"action": "install", "manifest": manifest.to_string(),
                           "grants": ["k8s.listCustomResource"]}),
                )
                .unwrap();
                state.plugins[0].revision
            }
        };
        let (session, seen) = scripted(scripts);
        streams.script_watches(session, FAST);
        let sink = Arc::new(TestSink::default());
        streams
            .open(sink.clone(), watch(APP, revision, "v", "extstream:crd"))
            .await
            .unwrap();
        CrdWatch {
            sink,
            answers,
            seen,
            reg,
            path,
            revision,
            _dir: dir,
        }
    }

    /// Forbidden: the stream fails with why, rather than reconnecting for ever.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_permanent_watch_error_ends_the_stream_with_the_reason() {
        let dir = tempfile::tempdir().unwrap();
        let (path, _reg, streams) = setup(dir.path());
        let revision = install(&path, fake_core());
        let forbidden = "applications.argoproj.io is forbidden: User cannot watch resource";
        let (session, _) = scripted(vec![vec![LISTED, Fail(forbidden)]]);
        streams.script_watches(session, FAST);
        let sink = Arc::new(TestSink::default());
        streams
            .open(sink.clone(), watch(APP, revision, "v", "extstream:f"))
            .await
            .unwrap();
        eventually("error", || {
            types(&sink, "extstream:f").last().map(String::as_str) == Some("error")
        })
        .await;
        let last = sink.payloads_for("extstream:f").last().unwrap().clone();
        assert_eq!(last["code"], "source");
        assert_eq!(last["message"], forbidden);
        assert_eq!(
            names(&sink, "extstream:f"),
            ["synced"],
            "no reconnecting for a denial"
        );
    }

    /// An update ends a watch as `appUpdated`; a change to what the app may
    /// reach, made without one, ends it at the host's announcement with why.
    #[tokio::test(flavor = "multi_thread")]
    async fn lifecycle_and_scope_changes_end_a_watch_with_their_reason() {
        let dir = tempfile::tempdir().unwrap();
        let (path, _reg, streams) = setup(dir.path());
        let first = install(&path, fake_core());
        let (session, _) = scripted(vec![]);
        streams.script_watches(session, FAST);
        let sink = Arc::new(TestSink::default());
        streams
            .open(sink.clone(), watch(APP, first, "old", "extstream:u"))
            .await
            .unwrap();
        eventually("synced", || names(&sink, "extstream:u") == ["synced"]).await;
        let second = install(&path, fake_core());
        let closed = sink.payloads_for("extstream:u").last().unwrap().clone();
        assert_eq!(closed["type"], "close");
        assert_eq!(closed["reason"], "appUpdated");
        streams
            .open(sink.clone(), watch(APP, second, "new", "extstream:s"))
            .await
            .unwrap();
        eventually("synced", || names(&sink, "extstream:s") == ["synced"]).await;
        configure(
            &path,
            json!({"action": "clusters", "id": APP, "contexts": ["elsewhere"]}),
        )
        .unwrap();
        eventually("error", || {
            types(&sink, "extstream:s").last().map(String::as_str) == Some("error")
        })
        .await;
        let last = sink.payloads_for("extstream:s").last().unwrap().clone();
        assert_eq!(last["code"], "source");
        assert!(
            last["message"]
                .as_str()
                .unwrap()
                .contains("enabled for this cluster"),
            "{last}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn watches_count_against_the_apps_stream_cap() {
        let dir = tempfile::tempdir().unwrap();
        let (path, _reg, streams) = setup(dir.path());
        let revision = install(&path, fake_core());
        let (session, _) = scripted(vec![]);
        streams.script_watches(session, FAST);
        let sink = Arc::new(TestSink::default());
        for n in 0..8 {
            let channel = format!("extstream:w{n}");
            streams
                .open(sink.clone(), watch(APP, revision, "v", &channel))
                .await
                .unwrap();
        }
        let refused = streams
            .open(sink.clone(), watch(APP, revision, "v", "extstream:w9"))
            .await
            .unwrap_err();
        assert!(refused.contains("already has 8 open streams"), "{refused}");
        let metrics = streams.metrics();
        assert!(metrics[0].streams.iter().all(|s| s.source == "watch"));
        assert_eq!(streams.close_view("v"), 8);
    }

    /// Whatever the cluster serves on a watch — here a Secret, values and
    /// all — the stream carries only that something changed.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_watch_frame_never_carries_a_watched_objects_values() {
        let dir = tempfile::tempdir().unwrap();
        let (path, _reg, streams) = setup(dir.path());
        let revision = install(&path, fake_core());
        let served = json!({
            "apiVersion": "v1", "kind": "Secret",
            "metadata": {"name": "db", "namespace": "team",
                         "annotations": {"note": "annotation-plaintext"}},
            "data": {"password": "cGxhaW50ZXh0LXZhbHVl"},
            "stringData": {"password": "plaintext-value"}
        });
        let session: WatchSession = Arc::new(move |scope: WatchScope| {
            let served = served.clone();
            Box::pin(async move {
                // What a session can report is a signal; the object stays here.
                let _object = served;
                let _ = scope.signals.send(KindSignal::Listed);
                tokio::time::sleep(Duration::from_millis(5)).await;
                let _ = scope.signals.send(KindSignal::Changed);
                std::future::pending().await
            })
        });
        let mut timing = FAST;
        timing.window = Duration::ZERO;
        streams.script_watches(session, timing);
        let sink = Arc::new(TestSink::default());
        streams
            .open(sink.clone(), watch(APP, revision, "v", "extstream:p"))
            .await
            .unwrap();
        eventually("both", || names(&sink, "extstream:p").len() == 2).await;
        let wire = serde_json::to_string(&sink.payloads_for("extstream:p")).unwrap();
        // The frames are shown on failure, never the fixture value that leaked.
        for (index, fragment) in ["plaintext", "cGxhaW50", "password", "db"]
            .iter()
            .enumerate()
        {
            assert!(
                !wire.contains(fragment),
                "fixture fragment #{index} left the host in a watch frame; frames: {}",
                sink.payloads_for("extstream:p").len()
            );
        }
    }

    /// Coalescing is what keeps a watch under the cap; without it, a flood
    /// still ends the stream with the clear `rateLimited` error.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_flood_past_the_rate_still_ends_as_rate_limited() {
        let dir = tempfile::tempdir().unwrap();
        let (path, _reg, streams) = setup(dir.path());
        let revision = install(&path, fake_core());
        let session: WatchSession = Arc::new(|scope: WatchScope| {
            Box::pin(async move {
                let _ = scope.signals.send(KindSignal::Listed);
                for _ in 0..200 {
                    let _ = scope.signals.send(KindSignal::Changed);
                    tokio::task::yield_now().await;
                }
                std::future::pending().await
            })
        });
        let mut timing = FAST;
        timing.window = Duration::ZERO;
        streams.script_watches(session, timing);
        let sink = Arc::new(TestSink::default());
        streams
            .open(sink.clone(), watch(APP, revision, "v", "extstream:q"))
            .await
            .unwrap();
        eventually("rate limited", || {
            types(&sink, "extstream:q").last().map(String::as_str) == Some("error")
        })
        .await;
        let last = sink.payloads_for("extstream:q").last().unwrap().clone();
        assert_eq!(last["code"], "rateLimited", "{last}");
        // The bucket's 50, plus whatever refilled while the flood ran, then the stop.
        let sent = names(&sink, "extstream:q").len();
        assert!((50..60).contains(&sent), "{sent} frames");
    }

    /// A column or card read after a watch frame lists the cluster, not the
    /// snapshot from before the change.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_change_drops_the_shared_snapshot_of_that_reader() {
        let dir = tempfile::tempdir().unwrap();
        let (path, _reg, streams) = setup(dir.path());
        let revision = install(&path, fake_core());
        let (session, _) = scripted(vec![]);
        streams.script_watches(session, FAST);
        let key = columns::reader_key(APP, revision, "pinned", "team", "applications", "v1alpha1");
        let other = columns::reader_key(APP, revision, "pinned", "team", "other", "v1alpha1");
        let fresh = || -> columns::SlotState {
            Ok(Some((
                std::time::Instant::now(),
                Arc::new(vec![json!({"name": "a"})]),
            )))
        };
        for key in [&key, &other] {
            streams
                .snapshots
                .lock()
                .unwrap()
                .insert(key.clone(), Arc::new(tokio::sync::Mutex::new(fresh())));
        }
        let sink = Arc::new(TestSink::default());
        streams
            .open(sink.clone(), watch(APP, revision, "v", "extstream:k"))
            .await
            .unwrap();
        eventually("synced", || names(&sink, "extstream:k") == ["synced"]).await;
        let slot = |key: &columns::CacheKey| {
            let map = streams.snapshots.lock().unwrap();
            let slot = map.get(key).unwrap().clone();
            let state = slot.try_lock().unwrap();
            matches!(&*state, Ok(Some(_)))
        };
        assert!(!slot(&key), "the watched reader's snapshot is dropped");
        assert!(slot(&other), "another reader's is kept");
    }

    /// Every announced inventory write reaches a listening window, whichever
    /// registry made it.
    #[tokio::test(flavor = "multi_thread")]
    async fn every_inventory_write_is_announced_to_listening_windows() {
        let dir = tempfile::tempdir().unwrap();
        let (path, _reg, streams) = setup(dir.path());
        let window = Arc::new(TestSink::default());
        streams.listen_inventory(window.clone());
        let revision = install(&path, fake_core());
        configure(
            &path,
            json!({"action": "enable", "id": APP, "enabled": false}),
        )
        .unwrap();
        assert!(revision > 0);
        assert_eq!(
            window.payloads_for(INVENTORY_CHANNEL),
            [json!({"type": "changed"}), json!({"type": "changed"})]
        );
    }

    /// The whole watch path against a real cluster: a Deployments reader's
    /// kind in a throwaway namespace this test creates and deletes. With
    /// `SRELENS_LIVE_RESTART` naming the cluster's node container, it also
    /// restarts it and expects `reconnecting`, then a fresh list's `synced`.
    ///
    /// `SRELENS_LIVE_CONTEXT=kind-x cargo test -p srelens-registry --lib live_watch -- --ignored --nocapture`
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs a live cluster (SRELENS_LIVE_CONTEXT) and kubectl on PATH"]
    async fn live_watch_follows_a_real_cluster() {
        let context = std::env::var("SRELENS_LIVE_CONTEXT").expect("SRELENS_LIVE_CONTEXT");
        const NS: &str = "srelens-watch-live";
        let kubectl = |args: &[&str]| {
            let out = std::process::Command::new("kubectl")
                .arg("--context")
                .arg(&context)
                .args(args)
                .output()
                .expect("kubectl");
            println!(
                "kubectl {}: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stdout).trim()
            );
            out.status.success()
        };
        kubectl(&["create", "namespace", NS]);
        let paths = crate::default_kubeconfig_paths();
        let cache = srelens_kube::client_cache::ClientCache::new_many(paths.clone());
        let core = Arc::new({
            let mut core = crate::build_registry_with_paths(cache.clone(), paths);
            core.register(crd::check_capability(cache.clone()));
            core
        });
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        let mut reg = Registry::new();
        let streams = super::super::register(&mut reg, path.clone(), core.clone(), cache.clone());
        let manifest = json!({
            "id": "org.example.deployments", "name": "Deployments", "version": "1.0.0",
            "srelensApiVersion": "^0.3", "kind": "declarative",
            "permissions": ["k8s.listDeployments"],
            "capabilities": [{"name": "deployments", "title": "Deployments",
                "target": "k8s.listDeployments", "arguments": {}, "inputs": ["context", "namespace"]}],
            "contributions": {"pages": [],
                "detailTabs": [], "detailLinks": []}
        });
        let state = super::super::mutate(
            &path,
            core,
            super::super::Configure::Install {
                signature: None,
                manifest: manifest.to_string(),
                grants: vec!["k8s.listDeployments".into()],
                reviewed_revision: None,
            },
        )
        .expect("the app installs");
        let revision = state.plugins[0].revision;
        streams.script_watches(
            kube_session(cache),
            WatchTiming {
                window: Duration::from_secs(1),
                backoff: Duration::from_secs(1),
                max_backoff: Duration::from_secs(5),
            },
        );
        let sink = Arc::new(TestSink::default());
        let channel = "extstream:live";
        streams
            .open(
                sink.clone(),
                json!({"id": "org.example.deployments", "revision": revision, "view": "live",
                       "channel": channel, "context": context, "namespace": NS,
                       "source": {"kind": "watch", "capability": "deployments"}}),
            )
            .await
            .expect("the watch opens");
        let wait = |what: &'static str, count: usize| {
            let sink = sink.clone();
            async move {
                for _ in 0..1200 {
                    if names(&sink, channel).len() >= count {
                        println!("{what}: {:?}", events(&sink, channel));
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                panic!("never happened: {what}: {:?}", sink.payloads_for(channel));
            }
        };
        wait("first list", 1).await;
        assert_eq!(names(&sink, channel), ["synced"]);
        assert!(kubectl(&[
            "-n",
            NS,
            "create",
            "deployment",
            "web",
            "--image=registry.k8s.io/pause:3.9"
        ]));
        wait("create", 2).await;
        let listed = reg
            .invoke(
                "extensions.read",
                json!({"id": "org.example.deployments", "revision": revision,
                       "capability": "deployments", "context": context, "namespace": NS}),
            )
            .await
            .expect("the page's read");
        println!("read after create: {listed}");
        assert!(listed.to_string().contains("\"web\""), "{listed}");
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert!(kubectl(&[
            "-n",
            NS,
            "scale",
            "deployment",
            "web",
            "--replicas=2"
        ]));
        wait("modify", 3).await;
        if let Ok(node) = std::env::var("SRELENS_LIVE_RESTART") {
            let before = names(&sink, channel).len();
            let restarted = std::process::Command::new("docker")
                .args(["restart", &node])
                .status()
                .expect("docker");
            assert!(restarted.success());
            // A rollout may still report a change or two before the node goes.
            let after = |event: &'static str, from: usize| {
                let sink = sink.clone();
                async move {
                    for _ in 0..1800 {
                        let seen = names(&sink, channel);
                        if let Some(at) = seen.iter().skip(from).position(|name| name == event) {
                            println!("{event}: {:?}", &events(&sink, channel)[from..]);
                            return from + at;
                        }
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                    panic!("never happened: {event}: {:?}", sink.payloads_for(channel));
                }
            };
            let lost = after("reconnecting", before).await;
            let relisted = after("synced", lost).await;
            assert!(
                names(&sink, channel)[lost..relisted]
                    .iter()
                    .all(|name| name == "reconnecting"),
                "nothing claims a change while the watch is down"
            );
            assert!(kubectl(&["--request-timeout=30s", "get", "namespace", NS]));
        }
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let before = names(&sink, channel).len();
        assert!(kubectl(&["-n", NS, "delete", "deployment", "web"]));
        wait("delete", before + 1).await;
        let wire = serde_json::to_string(&sink.payloads_for(channel)).unwrap();
        assert!(!wire.contains("web"), "no object name on the wire: {wire}");
        assert_eq!(streams.close_view("live"), 1);
        kubectl(&["delete", "namespace", NS, "--wait=false"]);
    }
}
