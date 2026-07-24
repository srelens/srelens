//! Live log-tail core: follows one or more pod/container log streams and
//! pushes each line to an EventSink. A stream can span many targets (e.g.
//! every pod of a Deployment); they multiplex onto one channel so the
//! frontend manages a single subscription.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use srelens_kube::client_cache::ClientCache;
use srelens_kube::logs::{stream_pod_logs_resilient, StreamOpts};
use tokio::task::JoinHandle;

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

/// A line emitted on the stream channel: its source tag and text.
#[derive(Debug, Clone, Serialize)]
pub struct LogLine {
    pub source: String,
    pub line: String,
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
                    stream_pod_logs_resilient(
                        cache,
                        context,
                        namespace,
                        t.pod,
                        t.container,
                        opts,
                        move |line| {
                            if let Ok(v) = serde_json::to_value(LogLine {
                                source: source.clone(),
                                line,
                            }) {
                                line_sink.emit(&line_channel, v);
                            }
                        },
                        move |status| {
                            status_sink
                                .emit(&status_channel, serde_json::json!({ "status": status }));
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
}
