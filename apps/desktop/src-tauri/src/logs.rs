//! Live log-tail bridge: follows one or more pod/container log streams and
//! pushes each line to the WebView over a single Tauri event channel. A stream
//! can span many targets (e.g. every pod of a Deployment); they multiplex onto
//! one id so the frontend manages a single subscription.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use srelens_kube::client_cache::ClientCache;
use srelens_kube::logs::{stream_pod_logs_resilient, StreamOpts};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tokio::task::JoinHandle;

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

/// A line emitted on `logs:line:<id>`: its source tag and text.
#[derive(Debug, Clone, Serialize)]
pub struct LogLine {
    pub source: String,
    pub line: String,
}

struct Stream {
    handles: Vec<JoinHandle<()>>,
}

/// Tauri-managed state owning running log-tail streams (keyed by channel).
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

    fn abort(&self, channel: &str) {
        if let Some(stream) = self.streams.lock().unwrap().remove(channel) {
            for h in stream.handles {
                h.abort();
            }
        }
    }
}

/// Start following the given targets, emitting each line as a `LogLine` on the
/// caller-provided `channel`. The WebView subscribes to `channel` first, then
/// invokes this, so the initial tail lines can't race ahead of the listener.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn start_log_stream(
    context: String,
    namespace: String,
    targets: Vec<LogTarget>,
    channel: String,
    timestamps: Option<bool>,
    since_seconds: Option<i64>,
    tail_lines: Option<i64>,
    app: AppHandle,
    manager: State<'_, LogStreamManager>,
) -> Result<(), String> {
    if targets.is_empty() {
        return Err("cannot start live logs without a pod target".into());
    }
    manager.abort(&channel);
    let opts = StreamOpts {
        tail_lines: tail_lines.unwrap_or(STREAM_TAIL_LINES),
        since_seconds,
        timestamps: timestamps.unwrap_or(false),
    };

    let handles = targets
        .into_iter()
        .map(|t| {
            let cache = manager.cache.clone();
            let app = app.clone();
            let channel = channel.clone();
            let context = context.clone();
            let namespace = namespace.clone();
            let source = t.label.clone();
            tokio::spawn(async move {
                let (line_app, line_channel) = (app.clone(), channel.clone());
                let (status_app, status_channel) = (app.clone(), channel.clone());
                stream_pod_logs_resilient(
                    cache,
                    context,
                    namespace,
                    t.pod,
                    t.container,
                    opts,
                    move |line| {
                        let _ = line_app.emit(
                            &line_channel,
                            LogLine {
                                source: source.clone(),
                                line,
                            },
                        );
                    },
                    move |status| {
                        let _ = status_app.emit(&status_channel, serde_json::json!({ "status": status }));
                    },
                )
                .await;
            })
        })
        .collect();

    manager
        .streams
        .lock()
        .unwrap()
        .insert(channel, Stream { handles });
    Ok(())
}

/// Stop a log-tail stream and abort all of its follow tasks.
#[tauri::command]
pub async fn stop_log_stream(
    channel: String,
    manager: State<'_, LogStreamManager>,
) -> Result<(), String> {
    manager.abort(&channel);
    Ok(())
}
