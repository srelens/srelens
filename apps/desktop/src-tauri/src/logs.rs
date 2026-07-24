//! Tauri adapter for live log tails: the streaming core lives in
//! srelens_streams::logs; this module only maps the Tauri command surface.

use std::sync::Arc;

use srelens_streams::logs::{LogStreamManager, LogTarget};
use tauri::{AppHandle, State};

use crate::sink::TauriSink;

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
    manager
        .start(
            Arc::new(TauriSink(app)),
            context,
            namespace,
            targets,
            channel,
            timestamps,
            since_seconds,
            tail_lines,
        )
        .await
}

/// Stop a log-tail stream and abort all of its follow tasks.
#[tauri::command]
pub async fn stop_log_stream(
    channel: String,
    manager: State<'_, LogStreamManager>,
) -> Result<(), String> {
    manager.stop(&channel);
    Ok(())
}
