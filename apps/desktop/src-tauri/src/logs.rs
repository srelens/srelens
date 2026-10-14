//! Tauri adapter for live log tails: the streaming core lives in
//! srelens_streams::logs; this module only maps the Tauri command surface.
//!
//! Commands are generic over the runtime (#28): the unit suite below drives
//! them through `tauri::test::MockRuntime`, so this surface counts toward
//! coverage instead of hiding behind the ignore-regex.

use std::sync::Arc;

use srelens_streams::logs::{LogStreamManager, LogTarget};
use tauri::{AppHandle, Runtime, State, Window};

use crate::sink::TauriSink;
use crate::window_streams::{Stream, WindowStreams};

/// Start following the given targets, emitting each line as a `LogLine` on the
/// caller-provided `channel`. The WebView subscribes to `channel` first, then
/// invokes this, so the initial tail lines can't race ahead of the listener.
///
/// The stream belongs to the calling window and stops when it closes or
/// reloads (#735). One whose window reloaded while it was starting is stopped
/// at once and refused.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn start_log_stream<R: Runtime>(
    context: String,
    namespace: String,
    targets: Vec<LogTarget>,
    channel: String,
    timestamps: Option<bool>,
    since_seconds: Option<i64>,
    tail_lines: Option<i64>,
    app: AppHandle<R>,
    window: Window<R>,
    manager: State<'_, LogStreamManager>,
    owned: State<'_, WindowStreams>,
) -> Result<(), String> {
    let epoch = owned.epoch(window.label());
    manager
        .start(
            Arc::new(TauriSink(app)),
            context,
            namespace,
            targets,
            channel.clone(),
            timestamps,
            since_seconds,
            tail_lines,
        )
        .await?;
    owned.keep(window.label(), epoch, Stream::Log(channel.clone()), || {
        manager.stop(&channel)
    })
}

/// Stop a log-tail stream and abort all of its follow tasks. Only the window
/// that started it may (#733, #735); one no window holds has ended, and this
/// is a no-op.
#[tauri::command]
pub async fn stop_log_stream<R: Runtime>(
    channel: String,
    window: Window<R>,
    manager: State<'_, LogStreamManager>,
    owned: State<'_, WindowStreams>,
) -> Result<(), String> {
    let stream = Stream::Log(channel.clone());
    if owned.check(window.label(), &stream)? {
        manager.stop(&channel);
        owned.disown(window.label(), &stream);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use srelens_kube::client_cache::ClientCache;
    use tauri::Manager;

    /// The empty-target refusal comes back synchronously; a real target is
    /// accepted (its follow task dies later on the unresolvable context) and
    /// stop tears the stream down — plus the unknown-channel no-op.
    #[tokio::test(flavor = "multi_thread")]
    async fn commands_run_against_a_mock_runtime() {
        let app = tauri::test::mock_app();
        app.manage(LogStreamManager::new(ClientCache::new_many(vec![])));
        app.manage(WindowStreams::default());
        let window = crate::window_streams::tests::mock_window(&app, "main");

        let e = start_log_stream(
            "no-such-context".into(),
            "ns".into(),
            vec![],
            "logs:test".into(),
            None,
            None,
            None,
            app.handle().clone(),
            window.clone(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap_err();
        assert!(e.contains("without a pod target"), "unexpected error: {e}");

        start_log_stream(
            "no-such-context".into(),
            "ns".into(),
            vec![LogTarget {
                pod: "pod-0".into(),
                container: None,
                label: String::new(),
            }],
            "logs:test".into(),
            Some(true),
            Some(60),
            Some(100),
            app.handle().clone(),
            window.clone(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();

        stop_log_stream("logs:test".into(), window.clone(), app.state(), app.state())
            .await
            .unwrap();
        stop_log_stream("logs:unknown".into(), window, app.state(), app.state())
            .await
            .unwrap();
    }
}
