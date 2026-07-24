//! Tauri adapter for live watches: the streaming core lives in
//! srelens_streams::watch; this module only maps the Tauri command surface.

use std::sync::Arc;

use srelens_streams::watch::WatchManager;
use tauri::{AppHandle, State};

use crate::sink::TauriSink;

/// Start watching a watchable resource kind in a namespace, emitting each full
/// sorted snapshot on the caller-provided `channel`. The WebView subscribes to
/// `channel` first, then invokes this, so the initial snapshot can't race
/// ahead of the listener.
#[tauri::command]
pub async fn start_resource_watch(
    context: String,
    namespace: String,
    kind: String,
    channel: String,
    kubeconfig_paths: Vec<String>,
    app: AppHandle,
    manager: State<'_, WatchManager>,
) -> Result<String, String> {
    manager
        .start(
            Arc::new(TauriSink(app)),
            context,
            namespace,
            kind,
            channel,
            kubeconfig_paths
                .into_iter()
                .map(std::path::PathBuf::from)
                .collect(),
        )
        .await
}

/// Stop a running watch by its channel.
#[tauri::command]
pub async fn stop_watch(channel: String, manager: State<'_, WatchManager>) -> Result<(), String> {
    manager.stop(&channel);
    Ok(())
}
