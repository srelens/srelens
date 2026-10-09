//! The generic app stream contract (#565): one host-owned stream kind that
//! every app-facing streaming source — resource polls today; watches, logs,
//! exec, port-forwards, traces and long AI tasks later — plugs into without
//! changing what travels on the wire.
//!
//! A stream is opened by a **view** of an app and owned by it. The manager
//! knows three owners' facts — the app, the installed revision the view was
//! rendered from, and the view's own id — and ends streams by any of them:
//! a view that closes ends its streams, an app that is disabled, removed or
//! updated ends every stream it opened, and nothing else is touched.
//!
//! A host may also name the **window** — the client connection — a stream was
//! opened through, so a window that closes or reloads ends what it opened
//! even though its views never said so (#700). Each window has an epoch that
//! every such ending bumps: an open that began before it is refused when it
//! lands.
//!
//! Every stream carries the same frames on its channel, in this order:
//!
//! - `open` once, before anything else;
//! - `data` any number of times, with a per-stream `seq`;
//! - exactly one terminal frame: `close` with a reason, or `error` with a
//!   code and a message. An error is never a close: a stream that failed, or
//!   was stopped for exceeding a limit, must not read as one that ended.
//!
//! `cancel` is the client's half: it ends one stream, and repeating it — or
//! naming a stream that already ended — changes nothing.

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use schemars::JsonSchema;
use serde::Serialize;
use serde_json::{json, Value};
use tokio::task::AbortHandle;

use crate::sink::EventSink;

/// Who opened a stream. The manager never interprets `view`; it only groups by it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamOwner {
    pub app: String,
    pub revision: u64,
    pub view: String,
    /// The window it was opened through, when the host names one. Never
    /// taken from the client: the host reads it off the connection.
    pub window: Option<StreamWindow>,
}

/// The window a stream was opened through, and that window's epoch — from
/// [`AppStreams::window_epoch`] — when the open began.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamWindow {
    /// The host's name for the window, e.g. a desktop window's label.
    pub label: String,
    pub epoch: u64,
}

/// Per-app caps, counted across every view of the app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamLimits {
    /// Streams one app may have open at once.
    pub max_open_per_app: usize,
    /// Data frames one app may send per second, over all its streams.
    pub messages_per_second: u32,
}

impl Default for StreamLimits {
    fn default() -> Self {
        Self {
            max_open_per_app: 8,
            messages_per_second: 50,
        }
    }
}

/// Why a stream ended normally. Sent as `close.reason`, camelCase.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum CloseReason {
    /// The source finished on its own.
    Completed,
    /// The client cancelled this stream.
    Cancelled,
    /// The view that opened it closed.
    ViewClosed,
    /// The app was disabled (by a person, or by the host's policy).
    AppDisabled,
    /// The app was updated or rolled back: the view's revision is gone.
    AppUpdated,
    /// The app was removed.
    AppRemoved,
    /// The window that opened it closed.
    WindowClosed,
    /// The window that opened it reloaded: the page that owned it is gone.
    WindowReloaded,
}

/// Why a stream failed. Sent as `error.code`, camelCase.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ErrorCode {
    /// The source failed: the cluster refused, timed out or was unreachable.
    Source,
    /// The app exceeded its message rate; the host stopped the stream.
    RateLimited,
}

/// A refused `open`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenError {
    TooManyStreams {
        app: String,
        limit: usize,
    },
    /// The window closed or reloaded after the open began.
    WindowGone {
        window: String,
    },
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenError::TooManyStreams { app, limit } => write!(
                f,
                "App {app} already has {limit} open streams, the most one app may have; close a view or cancel a stream first"
            ),
            OpenError::WindowGone { window } => write!(
                f,
                "The window {window} closed or reloaded while this stream was opening; nothing is left to receive it"
            ),
        }
    }
}

/// A window asked to cancel a stream it did not open (#733).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForeignStream {
    pub stream: String,
}

impl std::fmt::Display for ForeignStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Stream {} was not opened by this window; only the window that opened it may end it",
            self.stream
        )
    }
}

/// Why [`StreamEmitter::data`] refused a frame. The source should return.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stopped {
    /// The stream already ended (cancelled, closed or failed).
    Ended,
    /// This frame took the app past its rate; the stream now ended with an error.
    RateLimited,
}

/// One open stream, as the Inspector reads it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
pub struct StreamMetrics {
    pub stream: String,
    pub view: String,
    pub revision: u64,
    /// What the stream carries, e.g. `read`. Named by the source that opened it.
    pub source: String,
    pub messages: u64,
    pub bytes: u64,
}

/// One app's streams since this process started.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AppStreamMetrics {
    pub app: String,
    /// Streams open now.
    pub open_streams: usize,
    /// Streams ever opened.
    pub opened: u64,
    /// Data frames sent, over every stream.
    pub messages: u64,
    /// Bytes of `data` payload sent, as compact JSON.
    pub bytes: u64,
    /// Streams stopped for exceeding the message rate.
    pub rate_limited: u64,
    /// Open refused because the app was at its cap.
    pub refused: u64,
    /// Streams ended because the window that opened them closed or reloaded.
    pub window_ended: u64,
    pub streams: Vec<StreamMetrics>,
}

/// Owns every app stream in this process, and their lifetime: dropping it
/// aborts every source it started. Not `Clone`, so there is one owner.
pub struct AppStreams {
    inner: Arc<Inner>,
}

impl Drop for AppStreams {
    /// The sources hold the shared state, so without this they would outlive
    /// the manager as detached loops no later manager could cancel or end.
    /// No terminal frame is sent: the host that would deliver it is going away.
    fn drop(&mut self) {
        let entries: Vec<Entry> = {
            let mut state = self.inner.state.lock().unwrap();
            state.streams.drain().map(|(_, entry)| entry).collect()
        };
        for entry in entries {
            *entry.shared.ended.lock().unwrap() = true;
            if let Some(handle) = entry.handle {
                handle.abort();
            }
        }
    }
}

struct Inner {
    limits: StreamLimits,
    next: AtomicU64,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    streams: HashMap<String, Entry>,
    apps: HashMap<String, AppCounters>,
    /// Each named window's epoch; absent is 0. Kept after a window closes:
    /// a label can be reused, and a late open must still be refused.
    windows: HashMap<String, u64>,
}

struct Entry {
    owner: StreamOwner,
    source: String,
    shared: Arc<Shared>,
    /// Aborts the source task; the supervisor that awaits it then ends too.
    handle: Option<AbortHandle>,
    /// Open order, for the metrics.
    ordinal: u64,
}

#[derive(Default)]
struct AppCounters {
    opened: u64,
    messages: u64,
    bytes: u64,
    rate_limited: u64,
    refused: u64,
    window_ended: u64,
    /// Token bucket over `messages_per_second`: tokens left, and when it was last topped up.
    tokens: f64,
    topped: Option<Instant>,
}

/// What a stream's task and the manager share. `ended` is read and written
/// under its own lock, around every emit, so no frame follows the terminal one.
struct Shared {
    id: String,
    channel: String,
    sink: Arc<dyn EventSink>,
    ended: Mutex<bool>,
    seq: AtomicU64,
    messages: AtomicU64,
    bytes: AtomicU64,
}

/// A source's handle on its stream. Cloneable, so a source may emit from
/// several tasks; every clone stops once the stream ends.
#[derive(Clone)]
pub struct StreamEmitter {
    inner: Arc<Inner>,
    shared: Arc<Shared>,
    app: String,
}

impl AppStreams {
    pub fn new(limits: StreamLimits) -> Self {
        Self {
            inner: Arc::new(Inner {
                limits,
                next: AtomicU64::new(1),
                state: Mutex::new(State::default()),
            }),
        }
    }

    /// Open a stream on `channel` and run `source` until it returns or the
    /// stream is ended. The client subscribes to `channel` before calling
    /// this; the `open` frame is emitted before `source` starts, so it is
    /// always the first thing the client sees. Returns the stream id.
    pub fn open<F, Fut>(
        &self,
        owner: StreamOwner,
        source: &str,
        sink: Arc<dyn EventSink>,
        channel: String,
        run: F,
    ) -> Result<String, OpenError>
    where
        F: FnOnce(StreamEmitter) -> Fut,
        Fut: Future<Output = Result<(), String>> + Send + 'static,
    {
        let ordinal = self.inner.next.fetch_add(1, Ordering::Relaxed);
        let id = format!("s-{ordinal}");
        let shared = Arc::new(Shared {
            id: id.clone(),
            channel,
            sink,
            ended: Mutex::new(false),
            seq: AtomicU64::new(0),
            messages: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
        });
        let app = owner.app.clone();
        {
            let mut state = self.inner.state.lock().unwrap();
            // Checked under the lock `end_window` bumps the epoch under: an
            // open either lands before that ending and is ended by it, or is
            // refused here.
            if let Some(window) = &owner.window {
                if state.windows.get(&window.label).copied().unwrap_or(0) != window.epoch {
                    return Err(OpenError::WindowGone {
                        window: window.label.clone(),
                    });
                }
            }
            let limit = self.inner.limits.max_open_per_app;
            let open = state
                .streams
                .values()
                .filter(|e| e.owner.app == app)
                .count();
            let counters = state.apps.entry(app.clone()).or_default();
            if open >= limit {
                counters.refused += 1;
                return Err(OpenError::TooManyStreams { app, limit });
            }
            counters.opened += 1;
            state.streams.insert(
                id.clone(),
                Entry {
                    owner,
                    source: source.to_owned(),
                    shared: shared.clone(),
                    handle: None,
                    ordinal,
                },
            );
        }
        shared.emit_unless_ended(json!({ "type": "open", "stream": id }));
        let future = run(StreamEmitter {
            inner: self.inner.clone(),
            shared,
            app,
        });
        // The source runs in a task of its own, so a panic in it is a JoinError
        // here, not a lost stream: the supervisor still sends the terminal
        // frame and frees the slot. Aborting the source ends the supervisor.
        let source = tokio::spawn(future);
        let handle = source.abort_handle();
        let inner = self.inner.clone();
        let task_id = id.clone();
        tokio::spawn(async move {
            let terminal = match source.await {
                Ok(Ok(())) => close_frame(&task_id, CloseReason::Completed),
                Ok(Err(message)) => error_frame(&task_id, ErrorCode::Source, &message),
                Err(error) if error.is_panic() => error_frame(
                    &task_id,
                    ErrorCode::Source,
                    "The stream source crashed; the host stopped this stream",
                ),
                // Aborted: whoever aborted it already sent the terminal frame.
                Err(_) => return,
            };
            inner.finish(&task_id, terminal);
        });
        match self.inner.state.lock().unwrap().streams.get_mut(&id) {
            Some(entry) => entry.handle = Some(handle),
            // Ended before its task was recorded: nothing else will stop it.
            None => handle.abort(),
        }
        Ok(id)
    }

    /// End one stream with `close: cancelled`. Idempotent: `false`, and no
    /// frame, when the stream is unknown or already ended.
    pub fn cancel(&self, stream: &str) -> bool {
        self.inner
            .finish(stream, close_frame(stream, CloseReason::Cancelled))
    }

    /// End every stream `view` opened with `close: viewClosed`. Returns how many.
    pub fn close_view(&self, view: &str) -> usize {
        self.end_where(|owner| (owner.view == view).then_some(CloseReason::ViewClosed))
    }

    /// [`AppStreams::cancel`], for a host that names the calling window: a
    /// stream another window opened, or no window did, is refused and keeps
    /// running. Unknown or ended is still `Ok(false)`.
    pub fn cancel_in_window(&self, stream: &str, window: &str) -> Result<bool, ForeignStream> {
        let opened_here = {
            let state = self.inner.state.lock().unwrap();
            match state.streams.get(stream) {
                None => return Ok(false),
                Some(entry) => opened_in(&entry.owner, window),
            }
        };
        if !opened_here {
            return Err(ForeignStream {
                stream: stream.to_owned(),
            });
        }
        Ok(self.cancel(stream))
    }

    /// [`AppStreams::close_view`], for a host that names the calling window:
    /// ends only the streams that window opened for `view`. Views are named by
    /// the page, so another window may have one of the same name.
    pub fn close_view_in_window(&self, view: &str, window: &str) -> usize {
        self.end_where(|owner| {
            (owner.view == view && opened_in(owner, window)).then_some(CloseReason::ViewClosed)
        })
    }

    /// `window`'s epoch now. A host reads it before an open does anything
    /// that awaits, and names it in the owner's [`StreamWindow`].
    pub fn window_epoch(&self, window: &str) -> u64 {
        let state = self.inner.state.lock().unwrap();
        state.windows.get(window).copied().unwrap_or(0)
    }

    /// End every stream `window` opened with `reason` (`windowClosed` or
    /// `windowReloaded`), and bump its epoch so an open that began before
    /// this is refused. Returns how many ended.
    pub fn end_window(&self, window: &str, reason: CloseReason) -> usize {
        let chosen: Vec<(String, String)> = {
            let mut state = self.inner.state.lock().unwrap();
            *state.windows.entry(window.to_owned()).or_default() += 1;
            state
                .streams
                .iter()
                .filter(|(_, e)| opened_in(&e.owner, window))
                .map(|(id, e)| (id.clone(), e.owner.app.clone()))
                .collect()
        };
        let mut ended = 0;
        for (id, app) in chosen {
            if self.inner.finish(&id, close_frame(&id, reason)) {
                ended += 1;
                let mut state = self.inner.state.lock().unwrap();
                state.apps.entry(app).or_default().window_ended += 1;
            }
        }
        ended
    }

    /// End every stream for which `decide` names a reason. Lifecycle changes
    /// (disable, update, removal) come through here.
    pub fn end_where(&self, decide: impl Fn(&StreamOwner) -> Option<CloseReason>) -> usize {
        let chosen: Vec<(String, CloseReason)> = {
            let state = self.inner.state.lock().unwrap();
            state
                .streams
                .iter()
                .filter_map(|(id, entry)| decide(&entry.owner).map(|reason| (id.clone(), reason)))
                .collect()
        };
        chosen
            .into_iter()
            .filter(|(id, reason)| self.inner.finish(id, close_frame(id, *reason)))
            .count()
    }

    /// Per-app counters and the open streams, sorted by app, then by when
    /// each stream opened.
    pub fn metrics(&self) -> Vec<AppStreamMetrics> {
        let state = self.inner.state.lock().unwrap();
        let mut apps: Vec<AppStreamMetrics> = state
            .apps
            .iter()
            .map(|(app, counters)| {
                let mut open: Vec<&Entry> = state
                    .streams
                    .values()
                    .filter(|e| &e.owner.app == app)
                    .collect();
                open.sort_by_key(|e| e.ordinal);
                AppStreamMetrics {
                    app: app.clone(),
                    open_streams: open.len(),
                    opened: counters.opened,
                    messages: counters.messages,
                    bytes: counters.bytes,
                    rate_limited: counters.rate_limited,
                    refused: counters.refused,
                    window_ended: counters.window_ended,
                    streams: open
                        .into_iter()
                        .map(|e| StreamMetrics {
                            stream: e.shared.id.clone(),
                            view: e.owner.view.clone(),
                            revision: e.owner.revision,
                            source: e.source.clone(),
                            messages: e.shared.messages.load(Ordering::Relaxed),
                            bytes: e.shared.bytes.load(Ordering::Relaxed),
                        })
                        .collect(),
                }
            })
            .collect();
        apps.sort_by(|a, b| a.app.cmp(&b.app));
        apps
    }

    /// The caps this manager holds every app to.
    pub fn limits(&self) -> StreamLimits {
        self.inner.limits
    }

    /// The owners of the open streams, for a host deciding what to end.
    pub fn owners(&self) -> Vec<StreamOwner> {
        let state = self.inner.state.lock().unwrap();
        state.streams.values().map(|e| e.owner.clone()).collect()
    }
}

impl Inner {
    /// End `stream` with `terminal`, once. The first caller wins; everyone
    /// after gets `false` and sends nothing.
    fn finish(&self, stream: &str, terminal: Value) -> bool {
        let Some(entry) = self.state.lock().unwrap().streams.remove(stream) else {
            return false;
        };
        let sent = entry.shared.end(terminal);
        if let Some(handle) = entry.handle {
            handle.abort();
        }
        sent
    }

    /// Take one message from `app`'s bucket and count it, or refuse it.
    fn admit(&self, app: &str, bytes: u64) -> bool {
        let rate = f64::from(self.limits.messages_per_second);
        let mut state = self.state.lock().unwrap();
        let counters = state.apps.entry(app.to_owned()).or_default();
        let now = Instant::now();
        counters.tokens = match counters.topped {
            None => rate,
            Some(then) => {
                (counters.tokens + now.duration_since(then).as_secs_f64() * rate).min(rate)
            }
        };
        counters.topped = Some(now);
        if counters.tokens < 1.0 {
            return false;
        }
        counters.tokens -= 1.0;
        counters.messages += 1;
        counters.bytes += bytes;
        true
    }
}

impl Shared {
    fn emit_unless_ended(&self, frame: Value) -> bool {
        let ended = self.ended.lock().unwrap();
        if *ended {
            return false;
        }
        self.sink.emit(&self.channel, frame);
        true
    }

    /// Send the terminal frame, unless one was sent already.
    fn end(&self, terminal: Value) -> bool {
        let mut ended = self.ended.lock().unwrap();
        if *ended {
            return false;
        }
        *ended = true;
        self.sink.emit(&self.channel, terminal);
        true
    }
}

/// Whether `owner` was opened through the window labelled `window`.
fn opened_in(owner: &StreamOwner, window: &str) -> bool {
    owner.window.as_ref().is_some_and(|w| w.label == window)
}

fn close_frame(stream: &str, reason: CloseReason) -> Value {
    json!({ "type": "close", "stream": stream, "reason": reason })
}

fn error_frame(stream: &str, code: ErrorCode, message: &str) -> Value {
    json!({ "type": "error", "stream": stream, "code": code, "message": message })
}

impl StreamEmitter {
    /// Send one `data` frame. Refused once the stream ended, and refused —
    /// ending the stream with `error: rateLimited` — when this frame takes the
    /// app past its message rate.
    pub fn data(&self, value: Value) -> Result<(), Stopped> {
        let bytes = serde_json::to_string(&value)
            .map(|s| s.len() as u64)
            .unwrap_or(0);
        let ended = self.shared.ended.lock().unwrap();
        if *ended {
            return Err(Stopped::Ended);
        }
        if !self.inner.admit(&self.app, bytes) {
            drop(ended);
            let limit = self.inner.limits.messages_per_second;
            let message = format!(
                "App {} sent more than {limit} stream messages per second, the most one app may send; the host stopped this stream",
                self.app
            );
            let terminal = error_frame(&self.shared.id, ErrorCode::RateLimited, &message);
            if self.inner.finish(&self.shared.id, terminal) {
                let mut state = self.inner.state.lock().unwrap();
                state.apps.entry(self.app.clone()).or_default().rate_limited += 1;
            }
            return Err(Stopped::RateLimited);
        }
        let seq = self.shared.seq.fetch_add(1, Ordering::Relaxed) + 1;
        self.shared.messages.fetch_add(1, Ordering::Relaxed);
        self.shared.bytes.fetch_add(bytes, Ordering::Relaxed);
        self.shared.sink.emit(
            &self.shared.channel,
            json!({ "type": "data", "stream": self.shared.id, "seq": seq, "data": value }),
        );
        Ok(())
    }

    /// Whether the stream has ended; a polling source checks this between reads.
    pub fn is_ended(&self) -> bool {
        *self.shared.ended.lock().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::TestSink;
    use std::time::Duration;

    fn owner(app: &str, revision: u64, view: &str) -> StreamOwner {
        StreamOwner {
            app: app.into(),
            revision,
            view: view.into(),
            window: None,
        }
    }

    /// `owner`, opened through `window` at the epoch it has now.
    fn in_window(streams: &AppStreams, app: &str, view: &str, window: &str) -> StreamOwner {
        StreamOwner {
            window: Some(StreamWindow {
                label: window.into(),
                epoch: streams.window_epoch(window),
            }),
            ..owner(app, 1, view)
        }
    }

    /// A source that never ends by itself.
    fn forever(_: StreamEmitter) -> impl Future<Output = Result<(), String>> + Send {
        std::future::pending()
    }

    async fn eventually(what: &str, check: impl Fn() -> bool) {
        for _ in 0..200 {
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

    #[tokio::test(flavor = "multi_thread")]
    async fn open_comes_first_then_data_then_one_close() {
        let streams = AppStreams::new(StreamLimits::default());
        let sink = Arc::new(TestSink::default());
        let id = streams
            .open(
                owner("a", 1, "v"),
                "test",
                sink.clone(),
                "extstream:1".into(),
                |tx| async move {
                    tx.data(json!({"n": 1})).map_err(|e| format!("{e:?}"))?;
                    tx.data(json!({"n": 2})).map_err(|e| format!("{e:?}"))?;
                    Ok(())
                },
            )
            .unwrap();
        eventually("close", || {
            types(&sink, "extstream:1").contains(&"close".into())
        })
        .await;
        let frames = sink.payloads_for("extstream:1");
        assert_eq!(
            types(&sink, "extstream:1"),
            ["open", "data", "data", "close"]
        );
        assert_eq!(frames[0], json!({"type": "open", "stream": id}));
        assert_eq!(
            frames[1],
            json!({"type": "data", "stream": id, "seq": 1, "data": {"n": 1}})
        );
        assert_eq!(frames[2]["seq"], 2);
        assert_eq!(
            frames[3],
            json!({"type": "close", "stream": id, "reason": "completed"})
        );
        assert!(streams.owners().is_empty(), "an ended stream is not open");
    }

    /// Say what you know: a failed source is an `error`, never a `close`.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_failed_source_ends_with_an_error_not_a_close() {
        let streams = AppStreams::new(StreamLimits::default());
        let sink = Arc::new(TestSink::default());
        let id = streams
            .open(
                owner("a", 1, "v"),
                "test",
                sink.clone(),
                "extstream:e".into(),
                |_| async { Err("the cluster said no".to_string()) },
            )
            .unwrap();
        eventually("error", || types(&sink, "extstream:e").len() == 2).await;
        assert_eq!(types(&sink, "extstream:e"), ["open", "error"]);
        assert_eq!(
            sink.payloads_for("extstream:e")[1],
            json!({"type": "error", "stream": id, "code": "source", "message": "the cluster said no"})
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cancel_is_idempotent_and_nothing_follows_the_close() {
        let streams = AppStreams::new(StreamLimits::default());
        let sink = Arc::new(TestSink::default());
        let (tx_out, rx_out) = std::sync::mpsc::channel();
        let id = streams
            .open(
                owner("a", 1, "v"),
                "test",
                sink.clone(),
                "extstream:c".into(),
                move |tx| {
                    tx_out.send(tx).unwrap();
                    std::future::pending()
                },
            )
            .unwrap();
        let emitter = rx_out.recv().unwrap();
        assert!(streams.cancel(&id));
        assert!(!streams.cancel(&id), "a second cancel changes nothing");
        assert!(
            !streams.cancel("s-unknown"),
            "an unknown stream is not an error"
        );
        assert_eq!(emitter.data(json!(1)), Err(Stopped::Ended));
        assert!(emitter.is_ended());
        assert_eq!(types(&sink, "extstream:c"), ["open", "close"]);
        assert_eq!(sink.payloads_for("extstream:c")[1]["reason"], "cancelled");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn closing_a_view_ends_its_streams_and_only_those() {
        let streams = AppStreams::new(StreamLimits::default());
        let sink = Arc::new(TestSink::default());
        for (o, ch) in [
            (owner("a", 1, "page-1"), "extstream:a1"),
            (owner("a", 1, "page-1"), "extstream:a2"),
            (owner("a", 1, "page-2"), "extstream:a3"),
            (owner("b", 4, "page-1b"), "extstream:b1"),
        ] {
            streams
                .open(o, "test", sink.clone(), ch.into(), forever)
                .unwrap();
        }
        assert_eq!(streams.close_view("page-1"), 2);
        assert_eq!(streams.close_view("page-1"), 0, "closing again is a no-op");
        for ch in ["extstream:a1", "extstream:a2"] {
            assert_eq!(types(&sink, ch), ["open", "close"]);
            assert_eq!(sink.payloads_for(ch)[1]["reason"], "viewClosed");
        }
        for ch in ["extstream:a3", "extstream:b1"] {
            assert_eq!(types(&sink, ch), ["open"], "{ch} belongs to another view");
        }
        let mut views: Vec<_> = streams.owners().into_iter().map(|o| o.view).collect();
        views.sort();
        assert_eq!(views, ["page-1b", "page-2"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn lifecycle_ends_the_streams_it_names_with_their_reason() {
        let streams = AppStreams::new(StreamLimits::default());
        let sink = Arc::new(TestSink::default());
        for (o, ch) in [
            (owner("a", 1, "v1"), "extstream:old"),
            (owner("a", 2, "v2"), "extstream:new"),
            (owner("b", 1, "v3"), "extstream:other"),
        ] {
            streams
                .open(o, "test", sink.clone(), ch.into(), forever)
                .unwrap();
        }
        // App `a` moved to revision 2: only what revision 1 opened ends.
        let ended = streams
            .end_where(|o| (o.app == "a" && o.revision != 2).then_some(CloseReason::AppUpdated));
        assert_eq!(ended, 1);
        assert_eq!(
            sink.payloads_for("extstream:old")[1]["reason"],
            "appUpdated"
        );
        assert_eq!(types(&sink, "extstream:new"), ["open"]);
        let ended = streams.end_where(|o| (o.app == "a").then_some(CloseReason::AppDisabled));
        assert_eq!(ended, 1);
        assert_eq!(
            sink.payloads_for("extstream:new")[1]["reason"],
            "appDisabled"
        );
        assert_eq!(
            types(&sink, "extstream:other"),
            ["open"],
            "another app is untouched"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_open_stream_cap_is_per_app_and_refuses_with_a_clear_error() {
        let limits = StreamLimits {
            max_open_per_app: 2,
            ..StreamLimits::default()
        };
        let streams = AppStreams::new(limits);
        let sink = Arc::new(TestSink::default());
        let first = streams
            .open(
                owner("a", 1, "v"),
                "test",
                sink.clone(),
                "extstream:1".into(),
                forever,
            )
            .unwrap();
        streams
            .open(
                owner("a", 1, "w"),
                "test",
                sink.clone(),
                "extstream:2".into(),
                forever,
            )
            .unwrap();
        let refused = streams
            .open(
                owner("a", 1, "v"),
                "test",
                sink.clone(),
                "extstream:3".into(),
                forever,
            )
            .unwrap_err();
        assert_eq!(
            refused,
            OpenError::TooManyStreams {
                app: "a".into(),
                limit: 2
            }
        );
        assert!(
            refused.to_string().contains("already has 2 open streams"),
            "{refused}"
        );
        assert!(
            sink.payloads_for("extstream:3").is_empty(),
            "a refused open sends no frame"
        );
        // Another app has its own allowance, and a freed slot can be reused.
        streams
            .open(
                owner("b", 1, "x"),
                "test",
                sink.clone(),
                "extstream:4".into(),
                forever,
            )
            .unwrap();
        streams.cancel(&first);
        streams
            .open(
                owner("a", 1, "v"),
                "test",
                sink.clone(),
                "extstream:5".into(),
                forever,
            )
            .unwrap();
        let a = streams
            .metrics()
            .into_iter()
            .find(|m| m.app == "a")
            .unwrap();
        assert_eq!((a.open_streams, a.opened, a.refused), (2, 3, 1));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn exceeding_the_message_rate_ends_the_stream_with_a_clear_error() {
        let limits = StreamLimits {
            messages_per_second: 3,
            ..StreamLimits::default()
        };
        let streams = AppStreams::new(limits);
        let sink = Arc::new(TestSink::default());
        let (tx_out, rx_out) = std::sync::mpsc::channel();
        let id = streams
            .open(
                owner("a", 1, "v"),
                "test",
                sink.clone(),
                "extstream:r".into(),
                move |tx| async move {
                    let mut results = vec![];
                    for n in 0..5 {
                        results.push(tx.data(json!(n)));
                    }
                    tx_out.send(results).unwrap();
                    Ok(())
                },
            )
            .unwrap();
        let results = rx_out.recv().unwrap();
        assert_eq!(&results[..3], &[Ok(()), Ok(()), Ok(())]);
        assert_eq!(results[3], Err(Stopped::RateLimited));
        assert_eq!(results[4], Err(Stopped::Ended));
        eventually("error", || types(&sink, "extstream:r").len() == 5).await;
        assert_eq!(
            types(&sink, "extstream:r"),
            ["open", "data", "data", "data", "error"]
        );
        let error = &sink.payloads_for("extstream:r")[4];
        assert_eq!(error["code"], "rateLimited");
        assert_eq!(error["stream"], json!(id));
        assert!(
            error["message"]
                .as_str()
                .unwrap()
                .contains("more than 3 stream messages per second"),
            "{error}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(
            types(&sink, "extstream:r").len(),
            5,
            "no close follows the error"
        );
        let a = streams
            .metrics()
            .into_iter()
            .find(|m| m.app == "a")
            .unwrap();
        assert_eq!((a.rate_limited, a.messages, a.open_streams), (1, 3, 0));
    }

    /// Dropped when the source's future is dropped, which is what an abort does.
    struct DropFlag(Arc<std::sync::atomic::AtomicBool>);
    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    /// A panicking source still ends its stream, with an error, and frees its
    /// slot: a crash must not read as a stream that is still live, nor hold
    /// one of the app's open streams forever.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_panicking_source_ends_with_an_error_and_frees_its_slot() {
        let limits = StreamLimits {
            max_open_per_app: 1,
            ..StreamLimits::default()
        };
        let streams = AppStreams::new(limits);
        let sink = Arc::new(TestSink::default());
        let id = streams
            .open(
                owner("a", 1, "v"),
                "test",
                sink.clone(),
                "extstream:p".into(),
                |_| async {
                    if true {
                        panic!("the source broke");
                    }
                    Ok(())
                },
            )
            .unwrap();
        eventually("error", || types(&sink, "extstream:p").len() == 2).await;
        assert_eq!(types(&sink, "extstream:p"), ["open", "error"]);
        let error = &sink.payloads_for("extstream:p")[1];
        assert_eq!(error["code"], "source");
        assert_eq!(error["stream"], json!(id));
        assert!(
            error["message"].as_str().unwrap().contains("crashed"),
            "{error}"
        );
        assert!(
            streams.owners().is_empty(),
            "the crashed stream is not open"
        );
        streams
            .open(
                owner("a", 1, "v"),
                "test",
                sink.clone(),
                "extstream:p2".into(),
                forever,
            )
            .expect("the slot is free again");
    }

    /// The manager owns its streams' lifetime: dropping it aborts every source,
    /// rather than leaving detached loops that no later manager can reach.
    #[tokio::test(flavor = "multi_thread")]
    async fn dropping_the_manager_stops_every_stream_it_opened() {
        let streams = AppStreams::new(StreamLimits::default());
        let sink = Arc::new(TestSink::default());
        let stopped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = stopped.clone();
        streams
            .open(
                owner("a", 1, "v"),
                "test",
                sink.clone(),
                "extstream:d".into(),
                move |tx| async move {
                    let _flag = DropFlag(flag);
                    loop {
                        if tx.data(json!(1)).is_err() {
                            return Ok(());
                        }
                        // 20 a second: under the 50/s cap, so only the drop can stop it.
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                },
            )
            .unwrap();
        eventually("data", || {
            types(&sink, "extstream:d").contains(&"data".to_owned())
        })
        .await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(
            !stopped.load(Ordering::SeqCst),
            "the source runs until the drop"
        );
        drop(streams);
        eventually("the source stopped", || stopped.load(Ordering::SeqCst)).await;
        let sent = sink.payloads_for("extstream:d").len();
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(
            sink.payloads_for("extstream:d").len(),
            sent,
            "nothing is sent after the drop"
        );
        assert!(
            !types(&sink, "extstream:d").contains(&"error".to_owned()),
            "not stopped by the rate cap"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn metrics_count_messages_and_payload_bytes_per_stream_and_app() {
        let streams = AppStreams::new(StreamLimits::default());
        let sink = Arc::new(TestSink::default());
        let (tx_out, rx_out) = std::sync::mpsc::channel();
        let id = streams
            .open(
                owner("a", 7, "v"),
                "read",
                sink.clone(),
                "extstream:m".into(),
                move |tx| {
                    tx.data(json!({"k": "vv"})).unwrap(); // {"k":"vv"} is 10 bytes
                    tx.data(json!([1, 2])).unwrap(); // [1,2] is 5 bytes
                    tx_out.send(()).unwrap();
                    std::future::pending()
                },
            )
            .unwrap();
        rx_out.recv().unwrap();
        let metrics = streams.metrics();
        assert_eq!(metrics.len(), 1);
        let a = &metrics[0];
        assert_eq!(
            (a.app.as_str(), a.open_streams, a.messages, a.bytes),
            ("a", 1, 2, 15)
        );
        assert_eq!(
            a.streams,
            [StreamMetrics {
                stream: id.clone(),
                view: "v".into(),
                revision: 7,
                source: "read".into(),
                messages: 2,
                bytes: 15
            }]
        );
        streams.cancel(&id);
        let a = &streams.metrics()[0];
        assert_eq!(
            (a.open_streams, a.messages, a.bytes),
            (0, 2, 15),
            "totals outlive the stream"
        );
        assert!(a.streams.is_empty());
        assert_eq!(
            serde_json::to_value(a).unwrap()["openStreams"],
            0,
            "camelCase on the wire"
        );
    }

    /// A window cancels its own streams and no one else's (#733): another
    /// window's stream, or one no window opened, is refused by name and keeps
    /// running. An unknown or ended stream is still `false`, not an error.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_window_cancels_only_the_streams_it_opened() {
        let streams = AppStreams::new(StreamLimits::default());
        let sink = Arc::new(TestSink::default());
        let mine = streams
            .open(
                in_window(&streams, "a", "page#1", "main"),
                "test",
                sink.clone(),
                "extstream:mine".into(),
                forever,
            )
            .unwrap();
        let theirs = streams
            .open(
                in_window(&streams, "a", "page#1", "ctx-1"),
                "test",
                sink.clone(),
                "extstream:theirs".into(),
                forever,
            )
            .unwrap();
        let unowned = streams
            .open(
                owner("a", 1, "page#1"),
                "test",
                sink.clone(),
                "extstream:unowned".into(),
                forever,
            )
            .unwrap();

        for other in [&theirs, &unowned] {
            let refused = streams.cancel_in_window(other, "main").unwrap_err();
            assert_eq!(
                refused,
                ForeignStream {
                    stream: other.clone()
                }
            );
            assert_eq!(
                refused.to_string(),
                format!("Stream {other} was not opened by this window; only the window that opened it may end it")
            );
        }
        assert_eq!(types(&sink, "extstream:theirs"), ["open"], "still running");
        assert_eq!(types(&sink, "extstream:unowned"), ["open"]);

        assert_eq!(streams.cancel_in_window(&mine, "main"), Ok(true));
        assert_eq!(
            streams.cancel_in_window(&mine, "main"),
            Ok(false),
            "idempotent"
        );
        assert_eq!(streams.cancel_in_window("s-unknown", "main"), Ok(false));
        assert_eq!(types(&sink, "extstream:mine"), ["open", "close"]);
        assert_eq!(streams.cancel_in_window(&theirs, "ctx-1"), Ok(true));
    }

    /// Views are named by the page, so two windows can each have a view
    /// called `page#1`. Closing it in one window ends that window's streams
    /// for the view, never the other window's or an unowned one's (#733).
    #[tokio::test(flavor = "multi_thread")]
    async fn closing_a_view_in_a_window_leaves_the_same_view_in_other_windows() {
        let streams = AppStreams::new(StreamLimits::default());
        let sink = Arc::new(TestSink::default());
        for (o, ch) in [
            (in_window(&streams, "a", "page#1", "main"), "extstream:m1"),
            (in_window(&streams, "b", "page#1", "main"), "extstream:m2"),
            (in_window(&streams, "a", "page#2", "main"), "extstream:m3"),
            (in_window(&streams, "a", "page#1", "ctx-1"), "extstream:c1"),
            (owner("a", 1, "page#1"), "extstream:u1"),
        ] {
            streams
                .open(o, "test", sink.clone(), ch.into(), forever)
                .unwrap();
        }
        assert_eq!(streams.close_view_in_window("page#1", "main"), 2);
        assert_eq!(streams.close_view_in_window("page#1", "main"), 0);
        for ch in ["extstream:m1", "extstream:m2"] {
            assert_eq!(sink.payloads_for(ch)[1]["reason"], "viewClosed");
        }
        for ch in ["extstream:m3", "extstream:c1", "extstream:u1"] {
            assert_eq!(types(&sink, ch), ["open"], "{ch} is not main's page#1");
        }
    }

    /// A window that closes ends every stream it opened, of every app and
    /// view, with `windowClosed` — and not one stream another window opened,
    /// though it is the same app on the same page.
    #[tokio::test(flavor = "multi_thread")]
    async fn ending_a_window_ends_its_streams_and_no_other_windows() {
        let streams = AppStreams::new(StreamLimits::default());
        let sink = Arc::new(TestSink::default());
        for (o, ch) in [
            (in_window(&streams, "a", "page#1", "main"), "extstream:m1"),
            (in_window(&streams, "a", "page#2", "main"), "extstream:m2"),
            (in_window(&streams, "b", "card#1", "main"), "extstream:m3"),
            (in_window(&streams, "a", "page#3", "ctx-1"), "extstream:c1"),
            (owner("a", 1, "unowned"), "extstream:u1"),
        ] {
            streams
                .open(o, "test", sink.clone(), ch.into(), forever)
                .unwrap();
        }
        assert_eq!(streams.end_window("main", CloseReason::WindowClosed), 3);
        assert_eq!(
            streams.end_window("main", CloseReason::WindowClosed),
            0,
            "ending it again ends nothing"
        );
        for ch in ["extstream:m1", "extstream:m2", "extstream:m3"] {
            assert_eq!(types(&sink, ch), ["open", "close"]);
            assert_eq!(sink.payloads_for(ch)[1]["reason"], "windowClosed");
        }
        for ch in ["extstream:c1", "extstream:u1"] {
            assert_eq!(types(&sink, ch), ["open"], "{ch} is not the main window's");
        }
        let metrics = streams.metrics();
        let ended: Vec<_> = metrics
            .iter()
            .map(|m| (m.app.as_str(), m.window_ended, m.open_streams))
            .collect();
        assert_eq!(ended, [("a", 2, 2), ("b", 1, 0)]);
        assert_eq!(
            serde_json::to_value(&metrics[0]).unwrap()["windowEnded"],
            2,
            "camelCase on the wire"
        );
    }

    /// An open that began before its window reloaded — the old page asked,
    /// then went away while the host was still authorizing — is refused when
    /// it lands, with no frame, rather than left running for nobody.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_open_begun_before_its_window_reloaded_is_refused() {
        let streams = AppStreams::new(StreamLimits::default());
        let sink = Arc::new(TestSink::default());
        let stale = in_window(&streams, "a", "page#1", "main");
        assert_eq!(streams.end_window("main", CloseReason::WindowReloaded), 0);
        let refused = streams
            .open(
                stale,
                "test",
                sink.clone(),
                "extstream:late".into(),
                forever,
            )
            .unwrap_err();
        assert_eq!(
            refused,
            OpenError::WindowGone {
                window: "main".into()
            }
        );
        assert!(
            refused.to_string().contains("closed or reloaded"),
            "{refused}"
        );
        assert!(sink.payloads_for("extstream:late").is_empty());
        assert!(streams.owners().is_empty());
        // The reloaded page opens at the new epoch, and another window never
        // had its epoch moved.
        streams
            .open(
                in_window(&streams, "a", "page#2", "main"),
                "test",
                sink.clone(),
                "extstream:new".into(),
                forever,
            )
            .unwrap();
        assert_eq!(streams.window_epoch("ctx-1"), 0);
        let a = &streams.metrics()[0];
        assert_eq!((a.opened, a.refused, a.open_streams), (1, 0, 1));
    }

    /// Reloads used to pin an app at its cap with nothing listening (#700):
    /// every reload now frees what the page before it held.
    #[tokio::test(flavor = "multi_thread")]
    async fn after_many_reloads_an_app_can_still_open_up_to_its_cap() {
        let streams = AppStreams::new(StreamLimits::default());
        let sink = Arc::new(TestSink::default());
        let cap = streams.limits().max_open_per_app;
        for reload in 0..5 {
            for n in 0..cap {
                streams
                    .open(
                        in_window(&streams, "a", &format!("page#{reload}"), "main"),
                        "test",
                        sink.clone(),
                        format!("extstream:{reload}-{n}"),
                        forever,
                    )
                    .unwrap_or_else(|e| panic!("reload {reload}, stream {n}: {e}"));
            }
            assert!(streams
                .open(
                    in_window(&streams, "a", "page", "main"),
                    "test",
                    sink.clone(),
                    "extstream:over".into(),
                    forever,
                )
                .is_err());
            assert_eq!(streams.end_window("main", CloseReason::WindowReloaded), cap);
        }
        let a = &streams.metrics()[0];
        assert_eq!(
            (a.open_streams, a.opened, a.window_ended, a.refused),
            (0, 5 * cap as u64, 5 * cap as u64, 5)
        );
        assert_eq!(
            sink.payloads_for("extstream:4-0")[1]["reason"],
            "windowReloaded"
        );
    }
}
