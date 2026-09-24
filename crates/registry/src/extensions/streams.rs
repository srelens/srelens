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
//! One source ships with the contract: `read`, which re-runs one of the app's
//! declared readers on an interval through `extensions.read`'s own path, so
//! every tick rechecks everything a single read would. Watches (#566), logs,
//! exec and port-forwards (#567) and metric providers (#569) are further
//! `source` kinds on the same wire; none of them changes the frames.

use super::{columns, read_contribution, resolver_app, Inventory, Read};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use srelens_capability::{Annotations, Capability, CapabilityError, Registry};
use srelens_streams::app::{AppStreamMetrics, AppStreams, CloseReason, StreamLimits, StreamOwner};
use srelens_streams::EventSink;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

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
    path: PathBuf,
    core: Arc<Registry>,
    cache: Arc<srelens_kube::client_cache::ClientCache>,
    snapshots: columns::JoinCache,
    streams: AppStreams,
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
        let StreamSourceIn::Read {
            capability,
            interval_seconds,
        } = input.source;
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

/// The live [`ExtensionStreams`] per inventory path.
fn live() -> &'static Mutex<HashMap<PathBuf, Weak<ExtensionStreams>>> {
    static LIVE: OnceLock<Mutex<HashMap<PathBuf, Weak<ExtensionStreams>>>> = OnceLock::new();
    LIVE.get_or_init(Default::default)
}

/// Tell the streams of `path` that the inventory was written as `state`.
pub(super) fn announce(path: &Path, state: &Inventory) {
    let streams = live().lock().unwrap().get(path).and_then(Weak::upgrade);
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

/// Join (or start) the streams of `path`, and register `extensions.streams`,
/// the read path the Inspector (#575) reads open streams and traffic from.
pub(super) fn register(
    reg: &mut Registry,
    path: PathBuf,
    core: Arc<Registry>,
    cache: Arc<srelens_kube::client_cache::ClientCache>,
    snapshots: columns::JoinCache,
) -> Arc<ExtensionStreams> {
    let streams = {
        let mut live = live().lock().unwrap();
        match live.get(&path).and_then(Weak::upgrade) {
            Some(streams) => streams,
            None => {
                let streams = Arc::new(ExtensionStreams {
                    path: path.clone(),
                    core,
                    cache,
                    snapshots,
                    streams: AppStreams::new(StreamLimits::default()),
                });
                live.retain(|_, weak| weak.strong_count() > 0);
                live.insert(path, Arc::downgrade(&streams));
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
}
