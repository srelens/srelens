//! Live log-tail core: follows one or more pod/container log streams and
//! pushes each line to an EventSink. A stream can span many targets (e.g.
//! every pod of a Deployment); they multiplex onto one channel so the
//! frontend manages a single subscription.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use srelens_kube::client_cache::ClientCache;
use srelens_kube::logs::{stream_pod_logs_resilient, Line, StreamOpts};
use tokio::task::JoinHandle;

use crate::log_spool::LogSpool;
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
}

/// Owns running log-tail streams (keyed by channel).
pub struct LogStreamManager {
    cache: Arc<ClientCache>,
    streams: Mutex<HashMap<String, Stream>>,
}

impl LogStreamManager {
    pub fn new(cache: Arc<ClientCache>) -> Self {
        Self {
            cache,
            streams: Mutex::new(HashMap::new()),
        }
    }

    /// Start following the given targets, emitting each line as a `LogLine` on
    /// `channel`. The subscriber attaches to `channel` first, then calls this,
    /// so the initial tail lines can't race ahead of the listener.
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

        let handles = targets
            .into_iter()
            .map(|t| {
                let cache = self.cache.clone();
                let sink = sink.clone();
                let channel = channel.clone();
                let context = context.clone();
                let namespace = namespace.clone();
                let source = t.label.clone();
                tokio::spawn(async move {
                    let (line_sink, line_channel) = (sink.clone(), channel.clone());
                    let (status_sink, status_channel) = (sink.clone(), channel.clone());
                    // Both closures need the label: the line closure takes
                    // `source` by move, so the status closure gets its own copy.
                    let status_source = source.clone();
                    stream_pod_logs_resilient(
                        cache,
                        context,
                        namespace,
                        t.pod,
                        t.container,
                        opts,
                        move |line: Line| {
                            if let Ok(v) = serde_json::to_value(LogLine {
                                source: source.clone(),
                                line: line.text,
                                truncated: line.truncated,
                            }) {
                                line_sink.emit(&line_channel, v);
                            }
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
                })
            })
            .collect();

        self.streams
            .lock()
            .unwrap()
            .insert(channel, Stream { handles });
        Ok(())
    }

    /// Stop a log-tail stream and abort all of its follow tasks.
    pub fn stop(&self, channel: &str) {
        if let Some(stream) = self.streams.lock().unwrap().remove(channel) {
            for h in stream.handles {
                h.abort();
            }
        }
    }

    /// Abort every running log-tail stream (used when a user's environment is
    /// dropped).
    pub fn shutdown_all(&self) {
        let mut streams = self.streams.lock().unwrap();
        for (_, stream) in streams.drain() {
            for h in stream.handles {
                h.abort();
            }
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
}
