//! Tauri adapter for live watches: the streaming core lives in
//! srelens_streams::watch; this module only maps the Tauri command surface.
//!
//! Commands are generic over the runtime (#28): the unit suite below drives
//! them through `tauri::test::MockRuntime`, so this surface counts toward
//! coverage instead of hiding behind the ignore-regex.

use std::sync::Arc;

use serde_json::Value;
use srelens_streams::watch::WatchManager;
use tauri::ipc::Channel;
use tauri::{Runtime, State, Window};

use crate::sink::ChannelSink;
use crate::window_streams::{Stream, WindowStreams};

/// Start watching a watchable resource kind in a namespace, emitting each full
/// sorted snapshot on the caller-provided `channel`. The WebView subscribes to
/// `channel` first, then invokes this, so the initial snapshot can't race
/// ahead of the listener. The snapshots travel on `on_event`, the page's own
/// channel, so no other window receives them (#733).
///
/// The watch belongs to the calling window and stops when it closes or
/// reloads (#700). A watch whose window reloaded while it was starting is
/// stopped at once and refused.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn start_resource_watch<R: Runtime>(
    context: String,
    namespace: String,
    kind: String,
    channel: String,
    kubeconfig_paths: Vec<String>,
    on_event: Channel<Value>,
    window: Window<R>,
    manager: State<'_, WatchManager>,
    owned: State<'_, WindowStreams>,
) -> Result<String, String> {
    let epoch = owned.epoch(window.label());
    let channel = manager
        .start(
            Arc::new(ChannelSink(on_event)),
            context,
            namespace,
            kind,
            channel,
            kubeconfig_paths
                .into_iter()
                .map(std::path::PathBuf::from)
                .collect(),
        )
        .await?;
    owned.keep(window.label(), epoch, Stream::Watch(channel.clone()), || {
        manager.stop(&channel)
    })?;
    Ok(channel)
}

/// Stop a running watch by its channel. Only the window that started it may
/// (#733); a watch no window holds has already ended, and this is a no-op.
#[tauri::command]
pub async fn stop_watch<R: Runtime>(
    channel: String,
    window: Window<R>,
    manager: State<'_, WatchManager>,
    owned: State<'_, WindowStreams>,
) -> Result<(), String> {
    let stream = Stream::Watch(channel.clone());
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

    /// start registers the extra kubeconfig path, spawns the watch task, and
    /// echoes the channel back (the unresolvable context fails INSIDE the
    /// task, surfacing as an error payload on the channel — never here);
    /// stop aborts it, and an unknown channel no-ops.
    #[tokio::test(flavor = "multi_thread")]
    async fn commands_run_against_a_mock_runtime() {
        let app = tauri::test::mock_app();
        app.manage(WatchManager::new(ClientCache::new_many(vec![])));
        app.manage(WindowStreams::default());
        let window = crate::window_streams::tests::mock_window(&app, "main");

        let channel = start_resource_watch(
            "no-such-context".into(),
            "ns".into(),
            "pods".into(),
            "watch:test".into(),
            vec!["/nonexistent/kubeconfig".into()],
            crate::sink::tests::recording().0,
            window.clone(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
        assert_eq!(channel, "watch:test");

        stop_watch(channel, window.clone(), app.state(), app.state())
            .await
            .unwrap();
        stop_watch("watch:unknown".into(), window, app.state(), app.state())
            .await
            .unwrap();
    }
}
