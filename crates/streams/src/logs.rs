//! Live log-tail core: follows one or more pod/container log streams and
//! sends their lines to an EventSink as frames, keeping the history on disk
//! when asked (#914). A stream can span many targets (e.g.
//! every pod of a Deployment); they multiplex onto one channel so the
//! frontend manages a single subscription.

use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use srelens_kube::client_cache::ClientCache;
use srelens_kube::logs::{stream_pod_logs_resilient, Line, StreamOpts};
use tokio::task::JoinHandle;

use crate::log_spool::{LogSpool, SpoolDirs, SpoolPage, SpoolSnapshot};
use crate::sink::EventSink;

const STREAM_TAIL_LINES: i64 = 200;

/// One pod/container to follow, with a display label for prefixing lines when
/// several targets share a stream.
#[derive(Debug, Clone, Deserialize)]
pub struct LogTarget {
    pub pod: String,
    #[serde(default)]
    pub container: Option<String>,
    /// Source tag (e.g. "pod/container"); empty when a single target.
    #[serde(default)]
    pub label: String,
}

/// A line emitted on the stream channel: its source tag and text, and whether
/// the host cut it at [`srelens_kube::logs::MAX_LOG_LINE_BYTES`] (#747). The
/// flag is sent only on a cut line, so every other line's payload is as it was.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogLine {
    pub source: String,
    pub line: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

/// A connection-state change emitted on the stream channel, tagged with the
/// SAME source as that target's lines. A stream fans out over many targets,
/// each reconnecting on its own schedule; without this tag a subscriber can
/// only guess whether one pod dropped out or all of them did, and guessing
/// is what it used to do. The `status` key is also what distinguishes this
/// payload from a `LogLine` on the shared channel.
#[derive(Debug, Clone, Serialize)]
pub struct LogStatusEvent {
    pub source: String,
    pub status: String,
}

/// How often a stream's lines go out, as one frame (#914). One event per line
/// was one WebView script eval per line, and one WebSocket frame per line on a
/// 256-frame queue that closes the socket when it fills.
pub const FRAME_EVERY: Duration = Duration::from_millis(50);
/// The most lines one frame carries: the UI's window. Past it, the oldest wait
/// on disk (or, with no spool, are lost as the UI's ring would have lost them).
pub const FRAME_MAX_LINES: usize = 5000;
/// The most bytes of lines one frame carries.
pub const FRAME_MAX_BYTES: usize = 512 * 1024;

/// One stream's lines between the target tasks that read them and the frames
/// that send them: numbered, spooled, and held until the next frame.
///
/// Lock order: the writer's mutex, then the spool's.
///
/// ponytail: one mutex per stream, taken per line; per-target buffers if a
/// fan-out of hundreds of pods ever contends on it.
pub(crate) struct LogWriter {
    next_seq: u64,
    pending: VecDeque<LogLine>,
    pending_bytes: usize,
    spool: Option<Arc<Mutex<LogSpool>>>,
    /// Why the spool stopped, until the next frame says so.
    pub(crate) notice: Option<String>,
    /// Targets still following. One whose follow ends (a completed init container)
    /// counts itself out, and frames stop once none are left.
    pub(crate) running: usize,
}

impl LogWriter {
    /// A writer for a stream of `targets` targets. Each line goes to `spool` first
    /// when there is one; `None` keeps only the live frames.
    pub(crate) fn new(spool: Option<Arc<Mutex<LogSpool>>>, targets: usize) -> Self {
        Self {
            next_seq: 1,
            pending: VecDeque::new(),
            pending_bytes: 0,
            spool,
            notice: None,
            running: targets,
        }
    }

    /// Number `line` and hold it for the next frame, spooling it first when there
    /// is a spool. Past either cap only the oldest pending lines are dropped, never
    /// the line just pushed. A spool that fails to append is failed and dropped:
    /// the next `take` sends its reason, and the line still goes out.
    pub(crate) fn push(&mut self, line: LogLine) {
        if let Some(spool) = &self.spool {
            let mut spool = spool.lock().unwrap();
            match spool.append(&line) {
                Ok(seq) => debug_assert_eq!(
                    seq, self.next_seq,
                    "the spool and the writer number a line alike"
                ),
                Err(e) => {
                    // Losing the history never costs the live tail.
                    let reason = e.to_string();
                    spool.fail(reason.clone());
                    drop(spool);
                    self.spool = None;
                    self.notice = Some(reason);
                }
            }
        }
        self.next_seq += 1;
        self.pending_bytes += weight(&line);
        self.pending.push_back(line);
        while self.pending.len() > FRAME_MAX_LINES
            || (self.pending_bytes > FRAME_MAX_BYTES && self.pending.len() > 1)
        {
            let old = self.pending.pop_front().expect("pending is not empty");
            self.pending_bytes -= weight(&old);
        }
    }

    /// What goes out now: the spool notice, if one is due, then the frame.
    pub(crate) fn take(&mut self) -> Vec<serde_json::Value> {
        let mut out = Vec::new();
        if let Some(reason) = self.notice.take() {
            out.push(serde_json::json!({ "spool": { "failed": reason } }));
        }
        if !self.pending.is_empty() {
            let seq = self.next_seq - self.pending.len() as u64;
            let lines: Vec<LogLine> = self.pending.drain(..).collect();
            self.pending_bytes = 0;
            let mut frame = serde_json::json!({ "seq": seq, "lines": lines });
            if let Some(spool) = &self.spool {
                frame["first"] = serde_json::json!(spool.lock().unwrap().first());
            }
            out.push(frame);
        }
        out
    }
}

/// A line's share of a frame: its text and tag, plus the JSON around them.
///
/// ponytail: this counts the raw text and a flat 32 bytes, not the serialized JSON.
/// Quote- or ANSI-heavy lines escape to more bytes, so a real frame can exceed
/// FRAME_MAX_BYTES; count the serialized size if that ever matters.
fn weight(line: &LogLine) -> usize {
    line.source.len() + line.line.len() + 32
}

struct Stream {
    handles: Vec<JoinHandle<()>>,
    /// The stream's history, when it keeps one (#914). Held here, not by the
    /// writer: a stream whose targets all completed stops writing but stays
    /// readable until it is stopped.
    spool: Option<Arc<Mutex<LogSpool>>>,
    /// The pod a line with no source tag came from — the lone target's — so an
    /// export can name it the way the screen's own export does.
    lone_pod: Option<String>,
}

/// A stream's history as a read or an export holds it: the spool, kept alive
/// until the read ends, a snapshot of it, and the lone target's pod.
struct Held {
    spool: Arc<Mutex<LogSpool>>,
    snapshot: SpoolSnapshot,
    lone_pod: Option<String>,
}

/// Owns running log-tail streams (keyed by channel).
pub struct LogStreamManager {
    cache: Arc<ClientCache>,
    streams: Mutex<HashMap<String, Stream>>,
    spools: Option<SpoolDirs>,
}

impl LogStreamManager {
    pub fn new(cache: Arc<ClientCache>) -> Self {
        Self {
            cache,
            streams: Mutex::new(HashMap::new()),
            spools: None,
        }
    }

    /// Keep each stream's history on disk under `dirs` when its start asks for
    /// it (#914). Without this, a request for history is ignored.
    pub fn with_spools(mut self, dirs: SpoolDirs) -> Self {
        self.spools = Some(dirs);
        self
    }

    /// Start following the given targets. Their lines go out on `channel` as
    /// frames (`{seq, first?, lines}`) every [`FRAME_EVERY`]; each target's
    /// status changes go out at once. The subscriber attaches to `channel`
    /// first, then calls this, so the initial tail can't race the listener.
    /// `spool_max_bytes` keeps the stream's history on disk, capped by the
    /// manager's [`SpoolDirs`].
    #[allow(clippy::too_many_arguments)]
    pub async fn start(
        &self,
        sink: Arc<dyn EventSink>,
        context: String,
        namespace: String,
        targets: Vec<LogTarget>,
        channel: String,
        timestamps: Option<bool>,
        since_seconds: Option<i64>,
        tail_lines: Option<i64>,
        spool_max_bytes: Option<u64>,
    ) -> Result<(), String> {
        if targets.is_empty() {
            return Err("cannot start live logs without a pod target".into());
        }
        self.stop(&channel);
        let opts = StreamOpts {
            tail_lines: tail_lines.unwrap_or(STREAM_TAIL_LINES),
            since_seconds,
            timestamps: timestamps.unwrap_or(false),
            ..StreamOpts::default()
        };

        // A spool that cannot be made costs the history, never the tail: the
        // stream opens live-only and its first frame says why.
        let mut notice = None;
        let spool = match (spool_max_bytes, &self.spools) {
            (Some(asked), Some(dirs)) => dirs.cap(asked).and_then(|cap| {
                match dirs
                    .next_stream_dir()
                    .and_then(|dir| LogSpool::create(dir, cap))
                {
                    Ok(spool) => Some(Arc::new(Mutex::new(spool))),
                    Err(e) => {
                        notice = Some(e.to_string());
                        None
                    }
                }
            }),
            _ => None,
        };
        let lone_pod = targets
            .iter()
            .find(|t| t.label.is_empty())
            .map(|t| t.pod.clone());
        // Fresh from `LogSpool::create`: `LogWriter` asserts the spool numbers from 1.
        let writer = Arc::new(Mutex::new(LogWriter::new(spool.clone(), targets.len())));
        writer.lock().unwrap().notice = notice;

        let mut handles: Vec<JoinHandle<()>> = targets
            .into_iter()
            .map(|t| {
                let cache = self.cache.clone();
                let (status_sink, status_channel) = (sink.clone(), channel.clone());
                let context = context.clone();
                let namespace = namespace.clone();
                let source = t.label.clone();
                let writer = writer.clone();
                tokio::spawn(async move {
                    let status_source = source.clone();
                    let lines = writer.clone();
                    stream_pod_logs_resilient(
                        cache,
                        context,
                        namespace,
                        t.pod,
                        t.container,
                        opts,
                        move |line: Line| {
                            // ponytail: `push` does blocking file I/O (a BufWriter
                            // flush, a rotation's `remove_file`) under a std mutex,
                            // on a runtime worker. Briefly stalling that worker is
                            // the ceiling; a dedicated writer thread, or batching
                            // through `spawn_blocking`, if it ever shows up.
                            lines.lock().unwrap().push(LogLine {
                                source: source.clone(),
                                line: line.text,
                                truncated: line.truncated,
                            })
                        },
                        move |status| {
                            if let Ok(v) = serde_json::to_value(LogStatusEvent {
                                source: status_source.clone(),
                                status: status.to_string(),
                            }) {
                                status_sink.emit(&status_channel, v);
                            }
                        },
                    )
                    .await;
                    // Only a completed target gets here; the rest run until stopped.
                    writer.lock().unwrap().running -= 1;
                })
            })
            .collect();
        handles.push(tokio::spawn(send_frames(writer, sink, channel.clone())));

        self.adopt(
            channel,
            Stream {
                handles,
                spool,
                lone_pod,
            },
        );
        Ok(())
    }

    /// Register a started stream under `channel`. One that a racing `start` put
    /// there first is displaced, and stopped like any other: dropping it alone
    /// would detach its tasks and leave them running, unowned, with their history.
    fn adopt(&self, channel: String, stream: Stream) {
        // The displaced stream leaves the map before it is touched: dropping it
        // can delete its history, which must not happen under the map's lock.
        let displaced = self.streams.lock().unwrap().insert(channel, stream);
        if let Some(displaced) = displaced {
            for h in displaced.handles {
                h.abort();
            }
        }
    }

    /// Whether `channel` keeps its history on disk.
    pub fn keeps_history(&self, channel: &str) -> bool {
        self.streams
            .lock()
            .unwrap()
            .get(channel)
            .is_some_and(|s| s.spool.is_some())
    }

    /// Lines `[before - limit, before)` of `channel`'s history.
    pub async fn read(
        &self,
        channel: &str,
        before: u64,
        limit: usize,
    ) -> Result<SpoolPage, String> {
        let Held {
            spool, snapshot, ..
        } = self.snapshot(channel)?;
        tokio::task::spawn_blocking(move || {
            let _keep = spool; // a stop mid-read waits for the read to finish
            snapshot.read(before, limit)
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
    }

    /// Write `channel`'s history to `out` as `<source> | <line>`: every line,
    /// or those stamped at or after `since`. Returns how many were written.
    pub async fn export<W: Write + Send + 'static>(
        &self,
        channel: &str,
        mut out: W,
        since: Option<SystemTime>,
    ) -> Result<u64, String> {
        let Held {
            spool,
            snapshot,
            lone_pod,
        } = self.snapshot(channel)?;
        let lone = lone_pod.unwrap_or_default();
        tokio::task::spawn_blocking(move || {
            let _keep = spool;
            let name_of = |source: &str| {
                if source.is_empty() {
                    lone.clone()
                } else {
                    source.to_string()
                }
            };
            let written = snapshot.export(&mut out, since, &name_of)?;
            out.flush()?;
            Ok::<_, std::io::Error>(written)
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
    }

    /// What a read or an export needs of `channel`. The spool, the pod and so the
    /// snapshot all come from one look at the map, so a `stop` between lookups
    /// can't separate them.
    fn snapshot(&self, channel: &str) -> Result<Held, String> {
        let (spool, lone_pod) = self
            .streams
            .lock()
            .unwrap()
            .get(channel)
            .and_then(|s| Some((s.spool.clone()?, s.lone_pod.clone())))
            .ok_or_else(|| "this log stream keeps no history".to_string())?;
        // ponytail: `snapshot` flushes the spool's `BufWriter` (file I/O) here, on the
        // async caller, and takes the spool's lock, which a target's `push` may hold
        // while it writes. Both are brief; to keep them off the runtime, move this call
        // into the `tokio::task::spawn_blocking` closure of `read` and `export`.
        let snapshot = spool
            .lock()
            .unwrap()
            .snapshot()
            .map_err(|e| e.to_string())?;
        Ok(Held {
            spool,
            snapshot,
            lone_pod,
        })
    }

    /// Stop a log-tail stream and abort all of its follow tasks. Its history
    /// goes when the last task holding it has dropped.
    pub fn stop(&self, channel: &str) {
        // Out of the map in one statement: dropping the stream can delete its
        // history, which must not happen under the map's lock.
        let stream = self.streams.lock().unwrap().remove(channel);
        if let Some(stream) = stream {
            for h in stream.handles {
                h.abort();
            }
        }
    }

    /// Abort every running log-tail stream and remove every spool (a user's
    /// environment dropped, or the app quitting).
    pub fn shutdown_all(&self) {
        let streams: Vec<Stream> = self
            .streams
            .lock()
            .unwrap()
            .drain()
            .map(|(_, stream)| stream)
            .collect();
        for stream in streams {
            for h in stream.handles {
                h.abort();
            }
            // An aborted task may be mid-`push`, and a segment it opens after the
            // root's removal began would keep the root from going. A failed spool
            // has closed its file and takes no more lines, so none can.
            if let Some(spool) = &stream.spool {
                spool
                    .lock()
                    .unwrap()
                    .fail("the stream was shut down".into());
            }
        }
        if let Some(dirs) = &self.spools {
            dirs.remove_root();
        }
    }
}

/// Send a stream's lines as one frame every [`FRAME_EVERY`], until every
/// target has completed and the last of their lines is out.
async fn send_frames(writer: Arc<Mutex<LogWriter>>, sink: Arc<dyn EventSink>, channel: String) {
    let mut tick = tokio::time::interval(FRAME_EVERY);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        let (out, done) = {
            let mut writer = writer.lock().unwrap();
            let out = writer.take();
            (out, writer.running == 0)
        };
        for payload in out {
            sink.emit(&channel, payload);
        }
        if done {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::TestSink;
    use serde_json::json;

    #[tokio::test(flavor = "multi_thread")]
    async fn rejects_empty_targets() {
        let manager = LogStreamManager::new(ClientCache::new_many(vec![]));
        let sink = Arc::new(TestSink::default());
        let err = manager
            .start(
                sink,
                "ctx".into(),
                "ns".into(),
                vec![],
                "logs:1".into(),
                None,
                None,
                None,
                None,
            )
            .await
            .unwrap_err();
        assert!(err.contains("without a pod target"));
    }

    /// The status event has to name its target, the way each line already
    /// does. Neither target can connect here (the cache holds no kubeconfig),
    /// so each one's first attempt fails and emits its own "reconnecting"
    /// before backing off — two events, one per target, each tagged.
    #[tokio::test(flavor = "multi_thread")]
    async fn status_events_name_the_target_they_came_from() {
        let manager = LogStreamManager::new(ClientCache::new_many(vec![]));
        let sink = Arc::new(TestSink::default());
        manager
            .start(
                sink.clone(),
                "ctx".into(),
                "ns".into(),
                vec![
                    LogTarget {
                        pod: "web-1".into(),
                        container: Some("app".into()),
                        label: "web-1".into(),
                    },
                    LogTarget {
                        pod: "web-2".into(),
                        container: Some("app".into()),
                        label: "web-2".into(),
                    },
                ],
                "logs:1".into(),
                None,
                None,
                None,
                None,
            )
            .await
            .unwrap();

        let mut status_payloads = Vec::new();
        for _ in 0..200 {
            let payloads = sink.payloads_for("logs:1");
            status_payloads = payloads
                .into_iter()
                .filter(|p| p.get("status").is_some())
                .collect();
            if status_payloads.len() >= 2 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        manager.stop("logs:1");

        assert_eq!(status_payloads.len(), 2, "expected one status per target");
        let mut sources: Vec<String> = status_payloads
            .iter()
            .map(|p| p["source"].as_str().unwrap_or("<missing>").to_string())
            .collect();
        sources.sort();
        assert_eq!(sources, vec!["web-1".to_string(), "web-2".to_string()]);
        for p in &status_payloads {
            assert_eq!(p["status"], "reconnecting");
        }
    }

    /// The payload the frontend reads: `truncated` only on a cut line, so a
    /// whole line's payload is `{source, line}` as before.
    #[test]
    fn a_cut_line_says_so_and_a_whole_one_is_unchanged() {
        let line = |truncated| {
            serde_json::to_value(LogLine {
                source: "web-1".into(),
                line: "text".into(),
                truncated,
            })
            .unwrap()
        };
        assert_eq!(
            line(true),
            serde_json::json!({"source": "web-1", "line": "text", "truncated": true})
        );
        assert_eq!(
            line(false),
            serde_json::json!({"source": "web-1", "line": "text"})
        );
    }

    #[test]
    fn stop_unknown_channel_is_noop() {
        let manager = LogStreamManager::new(ClientCache::new_many(vec![]));
        manager.stop("nope");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn shutdown_all_stops_streams() {
        let manager = LogStreamManager::new(ClientCache::new_many(vec![]));
        let sink = Arc::new(TestSink::default());
        manager
            .start(
                sink,
                "ctx".into(),
                "ns".into(),
                vec![LogTarget {
                    pod: "p".into(),
                    container: None,
                    label: "p".into(),
                }],
                "logs:1".into(),
                None,
                None,
                None,
                None,
            )
            .await
            .unwrap();
        manager.shutdown_all(); // no panic; subsequent stop is a no-op
        manager.stop("logs:1");
    }

    fn ll(n: usize) -> LogLine {
        LogLine {
            source: String::new(),
            line: format!("l{n}"),
            truncated: false,
        }
    }

    #[test]
    fn a_frame_carries_what_arrived_since_the_last_numbered_from_one() {
        let mut w = LogWriter::new(None, 1);
        w.push(ll(1));
        w.push(ll(2));
        assert_eq!(
            w.take(),
            vec![
                json!({"seq": 1, "lines": [{"source": "", "line": "l1"}, {"source": "", "line": "l2"}]})
            ]
        );
        assert!(w.take().is_empty(), "nothing new, nothing sent");
        w.push(ll(3));
        assert_eq!(w.take()[0]["seq"], 3);
    }

    #[test]
    fn under_a_flood_a_frame_keeps_the_newest_lines_and_its_seq_skips_the_rest() {
        let mut w = LogWriter::new(None, 1);
        for n in 1..=FRAME_MAX_LINES + 250 {
            w.push(ll(n));
        }
        let frame = &w.take()[0];
        assert_eq!(frame["seq"], 251);
        let lines = frame["lines"].as_array().unwrap();
        assert_eq!(lines.len(), FRAME_MAX_LINES);
        assert_eq!(lines[0]["line"], "l251");
        assert_eq!(lines[FRAME_MAX_LINES - 1]["line"], "l5250");
    }

    #[test]
    fn a_frame_stays_within_its_byte_cap() {
        let mut w = LogWriter::new(None, 1);
        let big = "x".repeat(100 * 1024);
        for n in 1..=10 {
            w.push(LogLine {
                source: String::new(),
                line: format!("{n} {big}"),
                truncated: false,
            });
        }
        let frame = &w.take()[0];
        let lines = frame["lines"].as_array().unwrap();
        assert_eq!(lines.len(), 5);
        assert_eq!(frame["seq"], 6);
        assert!(
            lines[0]["line"].as_str().unwrap().starts_with("6 "),
            "the oldest line kept is the 6th push"
        );
        assert!(
            lines[4]["line"].as_str().unwrap().starts_with("10 "),
            "and the newest is the 10th"
        );
    }

    #[test]
    fn a_spooled_frame_names_the_oldest_line_on_disk_and_the_spool_has_every_line() {
        let tmp = tempfile::tempdir().unwrap();
        let spool = Arc::new(Mutex::new(
            LogSpool::create(tmp.path().join("s"), 64 << 20).unwrap(),
        ));
        let mut w = LogWriter::new(Some(spool.clone()), 1);
        for n in 1..=3 {
            w.push(ll(n));
        }
        let frame = w.take();
        assert_eq!(frame[0]["seq"], 1);
        assert_eq!(frame[0]["first"], 1);
        let page = spool
            .lock()
            .unwrap()
            .snapshot()
            .unwrap()
            .read(4, 10)
            .unwrap();
        assert_eq!(
            page.seq, 1,
            "the spool numbers its first line as the frame does"
        );
        let texts: Vec<&str> = page.lines.iter().map(|l| l.line.as_str()).collect();
        assert_eq!(texts, ["l1", "l2", "l3"]);
    }

    #[test]
    fn a_spool_that_fails_says_so_once_and_the_lines_still_go_out() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("s");
        let spool = Arc::new(Mutex::new(LogSpool::create(dir.clone(), 64 << 20).unwrap()));
        // Its first segment cannot be made: the directory has gone.
        std::fs::remove_dir_all(&dir).unwrap();
        let mut w = LogWriter::new(Some(spool.clone()), 1);
        w.push(ll(1));
        let out = w.take();
        assert!(out[0]["spool"]["failed"].is_string(), "{out:?}");
        assert_eq!(out[1]["seq"], 1);
        assert!(
            out[1].get("first").is_none(),
            "no history to point at any more"
        );
        w.push(ll(2));
        let next = w.take();
        assert_eq!(next.len(), 1, "the notice is sent once");
        assert_eq!(next[0]["seq"], 2);
        assert!(spool.lock().unwrap().failed().is_some(), "reads say why");
    }

    #[test]
    fn a_spool_that_fails_mid_stream_keeps_the_seq_running_and_says_why() {
        let tmp = tempfile::tempdir().unwrap();
        let spool = Arc::new(Mutex::new(
            LogSpool::create(tmp.path().join("s"), 64 << 20).unwrap(),
        ));
        let mut w = LogWriter::new(Some(spool.clone()), 1);
        for n in 1..=3 {
            w.push(ll(n));
        }
        assert_eq!(w.take()[0]["seq"], 1);
        // Forced, not by deleting the directory: on Windows the directory holding the
        // open segment cannot be removed; on Unix the open file survives that, so no
        // append fails until a new segment opens.
        spool.lock().unwrap().fail("disk full".into());
        w.push(ll(4));
        w.push(ll(5));
        let out = w.take();
        assert_eq!(out[0]["spool"]["failed"], "disk full");
        assert_eq!(out[1]["seq"], 4, "the seq goes on from the last frame's");
        assert_eq!(out[1]["lines"].as_array().unwrap().len(), 2);
        assert!(
            out[1].get("first").is_none(),
            "no history to point at any more"
        );
    }

    use crate::log_spool::{SpoolDirs, DESKTOP_SPOOL_CEILING};
    use std::time::Duration;

    fn one_target() -> Vec<LogTarget> {
        vec![LogTarget {
            pod: "web-1".into(),
            container: None,
            label: String::new(),
        }]
    }

    fn only_root(base: &std::path::Path) -> std::path::PathBuf {
        std::fs::read_dir(base)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path()
    }

    /// An export target the test can read back after the manager has written it.
    struct Shared(Arc<Mutex<Vec<u8>>>);

    impl Write for Shared {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// Neither target can connect (the cache holds no kubeconfig), so each one
    /// pushes its `[error: …]` line straight away: two lines, framed.
    #[tokio::test(flavor = "multi_thread")]
    async fn lines_go_out_as_frames_not_one_event_per_line() {
        let manager = LogStreamManager::new(ClientCache::new_many(vec![]));
        let sink = Arc::new(TestSink::default());
        let targets = vec![
            LogTarget {
                pod: "web-1".into(),
                container: None,
                label: "web-1".into(),
            },
            LogTarget {
                pod: "web-2".into(),
                container: None,
                label: "web-2".into(),
            },
        ];
        manager
            .start(
                sink.clone(),
                "ctx".into(),
                "ns".into(),
                targets,
                "logs:1".into(),
                None,
                None,
                None,
                None,
            )
            .await
            .unwrap();
        let mut sources = std::collections::BTreeSet::new();
        for _ in 0..300 {
            sources = sink
                .payloads_for("logs:1")
                .iter()
                .filter_map(|p| p.get("lines").and_then(|l| l.as_array()).cloned())
                .flatten()
                .map(|l| l["source"].as_str().unwrap_or("<missing>").to_string())
                .collect();
            if sources.len() >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        manager.stop("logs:1");
        assert_eq!(
            sources.into_iter().collect::<Vec<_>>(),
            ["web-1", "web-2"],
            "each target's line arrives in a frame, tagged with its source"
        );
        assert!(
            sink.payloads_for("logs:1")
                .iter()
                .all(|p| p.get("line").is_none()),
            "no line is sent on its own any more"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_spooled_stream_reads_back_its_lines_and_stop_removes_them() {
        let tmp = tempfile::tempdir().unwrap();
        let manager = LogStreamManager::new(ClientCache::new_many(vec![])).with_spools(
            SpoolDirs::per_process(tmp.path().to_path_buf(), DESKTOP_SPOOL_CEILING),
        );
        let sink = Arc::new(TestSink::default());
        manager
            .start(
                sink.clone(),
                "ctx".into(),
                "ns".into(),
                one_target(),
                "logs:1".into(),
                None,
                None,
                None,
                Some(64 << 20),
            )
            .await
            .unwrap();
        assert!(manager.keeps_history("logs:1"));
        let mut page = None;
        for _ in 0..300 {
            let p = manager.read("logs:1", u64::MAX, 10).await.unwrap();
            if !p.lines.is_empty() {
                page = Some(p);
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let page = page.expect("the error line reached the spool");
        assert_eq!((page.first, page.seq), (1, 1));
        assert!(
            page.lines[0].line.starts_with("[error:"),
            "{:?}",
            page.lines[0]
        );

        let stream_dir = only_root(tmp.path()).join("0");
        assert!(stream_dir.is_dir());
        manager.stop("logs:1");
        for _ in 0..300 {
            if !stream_dir.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            !stream_dir.exists(),
            "stopping a stream deletes its history"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_stream_that_asked_for_no_history_has_none() {
        let tmp = tempfile::tempdir().unwrap();
        let manager = LogStreamManager::new(ClientCache::new_many(vec![])).with_spools(
            SpoolDirs::per_process(tmp.path().to_path_buf(), DESKTOP_SPOOL_CEILING),
        );
        manager
            .start(
                Arc::new(TestSink::default()),
                "ctx".into(),
                "ns".into(),
                one_target(),
                "logs:1".into(),
                None,
                None,
                None,
                None,
            )
            .await
            .unwrap();
        assert!(!manager.keeps_history("logs:1"));
        let err = manager.read("logs:1", 10, 10).await.unwrap_err();
        assert!(err.contains("keeps no history"), "{err}");
        assert!(
            std::fs::read_dir(tmp.path()).unwrap().next().is_none(),
            "nothing written"
        );
        manager.stop("logs:1");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_export_names_the_lone_target_by_its_pod() {
        let tmp = tempfile::tempdir().unwrap();
        let manager = LogStreamManager::new(ClientCache::new_many(vec![])).with_spools(
            SpoolDirs::per_process(tmp.path().to_path_buf(), DESKTOP_SPOOL_CEILING),
        );
        manager
            .start(
                Arc::new(TestSink::default()),
                "ctx".into(),
                "ns".into(),
                one_target(),
                "logs:1".into(),
                None,
                None,
                None,
                Some(64 << 20),
            )
            .await
            .unwrap();
        for _ in 0..300 {
            if !manager
                .read("logs:1", u64::MAX, 1)
                .await
                .unwrap()
                .lines
                .is_empty()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let out = Arc::new(Mutex::new(Vec::new()));
        let n = manager
            .export("logs:1", Shared(out.clone()), None)
            .await
            .unwrap();
        assert!(n >= 1);
        let text = String::from_utf8(out.lock().unwrap().clone()).unwrap();
        assert!(text.starts_with("web-1 | [error:"), "{text}");
        manager.stop("logs:1");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn shutdown_all_removes_every_spool() {
        let tmp = tempfile::tempdir().unwrap();
        let manager = LogStreamManager::new(ClientCache::new_many(vec![])).with_spools(
            SpoolDirs::per_process(tmp.path().to_path_buf(), DESKTOP_SPOOL_CEILING),
        );
        manager
            .start(
                Arc::new(TestSink::default()),
                "ctx".into(),
                "ns".into(),
                one_target(),
                "logs:1".into(),
                None,
                None,
                None,
                Some(64 << 20),
            )
            .await
            .unwrap();
        assert!(std::fs::read_dir(tmp.path()).unwrap().next().is_some());
        manager.shutdown_all();
        assert!(
            std::fs::read_dir(tmp.path()).unwrap().next().is_none(),
            "the root is gone"
        );
    }

    /// History that cannot be made costs the history, never the tail.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_spool_that_cannot_be_made_leaves_the_stream_live_and_says_why() {
        let tmp = tempfile::tempdir().unwrap();
        // The root would go under a file, so it cannot be made.
        let file = tmp.path().join("not-a-dir");
        std::fs::write(&file, b"").unwrap();
        let manager = LogStreamManager::new(ClientCache::new_many(vec![]))
            .with_spools(SpoolDirs::fixed(file.join("logs"), DESKTOP_SPOOL_CEILING));
        let sink = Arc::new(TestSink::default());
        manager
            .start(
                sink.clone(),
                "ctx".into(),
                "ns".into(),
                one_target(),
                "logs:1".into(),
                None,
                None,
                None,
                Some(64 << 20),
            )
            .await
            .unwrap();
        assert!(!manager.keeps_history("logs:1"));
        let mut payloads = Vec::new();
        for _ in 0..300 {
            payloads = sink.payloads_for("logs:1");
            if payloads.iter().any(|p| p.get("spool").is_some())
                && payloads.iter().any(|p| p.get("lines").is_some())
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        manager.stop("logs:1");
        let notice = payloads
            .iter()
            .position(|p| p["spool"]["failed"].is_string())
            .unwrap_or_else(|| panic!("a spool notice: {payloads:?}"));
        let first_frame = payloads
            .iter()
            .position(|p| p.get("lines").is_some())
            .unwrap_or_else(|| panic!("the live lines still go out: {payloads:?}"));
        assert!(
            notice < first_frame,
            "the notice comes before the first frame: {payloads:?}"
        );
    }

    /// A manager with no spool directories has nowhere to keep history, so a
    /// request for it is ignored: no history, and no notice either, because
    /// nothing failed.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_manager_without_spools_ignores_a_request_for_history() {
        let manager = LogStreamManager::new(ClientCache::new_many(vec![]));
        let sink = Arc::new(TestSink::default());
        manager
            .start(
                sink.clone(),
                "ctx".into(),
                "ns".into(),
                one_target(),
                "logs:1".into(),
                None,
                None,
                None,
                Some(64 << 20),
            )
            .await
            .unwrap();
        assert!(!manager.keeps_history("logs:1"));
        let mut payloads = Vec::new();
        for _ in 0..300 {
            payloads = sink.payloads_for("logs:1");
            if payloads.iter().any(|p| p.get("lines").is_some()) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        manager.stop("logs:1");
        assert!(
            payloads.iter().any(|p| p.get("lines").is_some()),
            "the live lines go out: {payloads:?}"
        );
        assert!(
            payloads.iter().all(|p| p.get("spool").is_none()),
            "no notice: {payloads:?}"
        );
    }

    /// The spool belongs to the stream's entry, not to its writer: once every
    /// task has ended, the history still reads back and exports, until `stop`.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_spool_stays_readable_after_its_writer_ends() {
        let tmp = tempfile::tempdir().unwrap();
        let manager = LogStreamManager::new(ClientCache::new_many(vec![])).with_spools(
            SpoolDirs::per_process(tmp.path().to_path_buf(), DESKTOP_SPOOL_CEILING),
        );
        manager
            .start(
                Arc::new(TestSink::default()),
                "ctx".into(),
                "ns".into(),
                one_target(),
                "logs:1".into(),
                None,
                None,
                None,
                Some(64 << 20),
            )
            .await
            .unwrap();
        for _ in 0..300 {
            if !manager
                .read("logs:1", u64::MAX, 1)
                .await
                .unwrap()
                .lines
                .is_empty()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        // End every task of the stream, which drops its writer and its Arc on
        // the spool. The guard is gone by the end of the statement.
        let handles = std::mem::take(
            &mut manager
                .streams
                .lock()
                .unwrap()
                .get_mut("logs:1")
                .expect("the stream is running")
                .handles,
        );
        for h in handles {
            h.abort();
            let _ = h.await;
        }

        let stream_dir = only_root(tmp.path()).join("0");
        assert!(stream_dir.is_dir(), "the history outlives its writer");
        let page = manager.read("logs:1", u64::MAX, 10).await.unwrap();
        assert!(
            page.lines[0].line.starts_with("[error:"),
            "{:?}",
            page.lines
        );
        let out = Arc::new(Mutex::new(Vec::new()));
        let n = manager
            .export("logs:1", Shared(out.clone()), None)
            .await
            .unwrap();
        assert!(n >= 1);
        let text = String::from_utf8(out.lock().unwrap().clone()).unwrap();
        assert!(text.starts_with("web-1 | [error:"), "{text}");

        manager.stop("logs:1");
        for _ in 0..300 {
            if !stream_dir.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(!stream_dir.exists(), "stop deletes it at last");
    }

    /// `start` stops the channel's old stream first, but two starts that race on one
    /// channel both pass that and then register: the second finds the first's stream
    /// in the map. (One `start` after another never gets there, as the first is
    /// already stopped, so the registration is driven directly here.) The stream it
    /// displaces must be stopped, not left running with nothing to stop it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_stream_displaced_by_a_racing_start_is_stopped_and_its_history_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let manager = LogStreamManager::new(ClientCache::new_many(vec![])).with_spools(
            SpoolDirs::per_process(tmp.path().to_path_buf(), DESKTOP_SPOOL_CEILING),
        );
        let sink = Arc::new(TestSink::default());
        for channel in ["logs:1", "logs:2"] {
            manager
                .start(
                    sink.clone(),
                    "ctx".into(),
                    "ns".into(),
                    one_target(),
                    channel.into(),
                    None,
                    None,
                    None,
                    Some(64 << 20),
                )
                .await
                .unwrap();
        }
        let root = only_root(tmp.path());
        let (displaced_dir, kept_dir) = (root.join("0"), root.join("1"));
        assert!(displaced_dir.is_dir() && kept_dir.is_dir());
        let tasks: Vec<_> = manager.streams.lock().unwrap()["logs:1"]
            .handles
            .iter()
            .map(|h| h.abort_handle())
            .collect();

        // The second start's stream, registered where the first one's is.
        let second = manager.streams.lock().unwrap().remove("logs:2").unwrap();
        manager.adopt("logs:1".into(), second);

        for _ in 0..300 {
            if tasks.iter().all(|t| t.is_finished()) && !displaced_dir.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            tasks.iter().all(|t| t.is_finished()),
            "the displaced stream's tasks end"
        );
        assert!(!displaced_dir.exists(), "and its history goes");
        assert!(
            kept_dir.is_dir(),
            "the stream that displaced it keeps its own"
        );
        assert!(manager.keeps_history("logs:1"));
        manager.stop("logs:1");
    }

    /// A frame goes out on each tick, and once every target has completed the
    /// ticker sends what the targets left and stops.
    #[tokio::test(start_paused = true)]
    async fn frames_stop_once_every_target_has_completed_and_the_last_lines_are_out() {
        let writer = Arc::new(Mutex::new(LogWriter::new(None, 1)));
        let sink = Arc::new(TestSink::default());
        let task = tokio::spawn(send_frames(writer.clone(), sink.clone(), "logs:1".into()));
        writer.lock().unwrap().push(ll(1));
        tokio::time::sleep(FRAME_EVERY * 2).await;
        assert_eq!(sink.payloads_for("logs:1").len(), 1);
        {
            let mut w = writer.lock().unwrap();
            w.push(ll(2));
            w.running = 0;
        }
        tokio::time::sleep(FRAME_EVERY * 2).await;
        assert!(task.is_finished());
        assert_eq!(sink.payloads_for("logs:1")[1]["lines"][0]["line"], "l2");
    }
}
