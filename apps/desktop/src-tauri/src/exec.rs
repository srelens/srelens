//! Tauri adapter for in-pod exec: the streaming core lives in
//! srelens_streams::exec; this module only maps the Tauri command surface.
//!
//! Commands are generic over the runtime (#28): the unit suite below drives
//! them through `tauri::test::MockRuntime`, so this surface counts toward
//! coverage instead of hiding behind the ignore-regex.

use std::sync::Arc;

use srelens_streams::exec::{ExecManager, ExecOpts};
use tauri::{AppHandle, Runtime, State, Window};

use crate::sink::TauriSink;
use crate::window_streams::WindowStreams;

/// Open an interactive shell into a pod. Returns the session id; stdout streams
/// on `exec:out:<channel>` and an `exec:exit:<channel>` event fires (with an
/// optional error string) when the session ends, where `channel` is the
/// caller-supplied subscription token — the WebView subscribes to it before
/// this call, so an exec that dies in the same tick it spawns cannot outrun
/// the listener.
///
/// The session belongs to the calling window and is closed when it closes or
/// reloads (#700). One whose window reloaded while it was starting is closed
/// at once and refused.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn start_pod_exec<R: Runtime>(
    context: String,
    namespace: String,
    pod: String,
    container: Option<String>,
    shell: Option<String>,
    command: Option<Vec<String>>,
    channel: String,
    cols: Option<u16>,
    rows: Option<u16>,
    window: Window<R>,
    app: AppHandle<R>,
    manager: State<'_, ExecManager>,
    owned: State<'_, WindowStreams>,
) -> Result<u64, String> {
    let epoch = owned.epoch(window.label());
    let session = manager
        .start(
            Arc::new(TauriSink(app)),
            context,
            namespace,
            pod,
            channel,
            ExecOpts {
                container,
                shell,
                command,
                cols,
                rows,
            },
        )
        .await?;
    owned.keep_exec(&manager, window.label(), epoch, session)
}

/// Forward a keystroke / input string to an exec session's stdin.
#[tauri::command]
pub async fn exec_input(
    session: u64,
    data: String,
    manager: State<'_, ExecManager>,
) -> Result<(), String> {
    manager.input(session, data).await;
    Ok(())
}

/// Resize an exec session's remote PTY to `cols` x `rows`.
#[tauri::command]
pub async fn exec_resize(
    session: u64,
    cols: u16,
    rows: u16,
    manager: State<'_, ExecManager>,
) -> Result<(), String> {
    manager.resize(session, cols, rows).await;
    Ok(())
}

/// Close an exec session and abort its task.
#[tauri::command]
pub async fn exec_close<R: Runtime>(
    session: u64,
    window: Window<R>,
    manager: State<'_, ExecManager>,
    owned: State<'_, WindowStreams>,
) -> Result<(), String> {
    manager.close(session);
    owned.disown_exec(window.label(), session);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use srelens_kube::client_cache::ClientCache;
    use tauri::Manager;

    /// The full command surface against a MockRuntime app: start hands back a
    /// session id immediately (connection failures surface later as an
    /// `exec:exit` event), and input/resize/close are no-ops for a session
    /// whose task already died — exactly the WebView's teardown race.
    #[tokio::test(flavor = "multi_thread")]
    async fn commands_run_against_a_mock_runtime() {
        let app = tauri::test::mock_app();
        app.manage(ExecManager::new(ClientCache::new_many(vec![])));
        app.manage(WindowStreams::default());
        let window = crate::window_streams::tests::mock_window(&app, "main");

        let id = start_pod_exec(
            "no-such-context".into(),
            "ns".into(),
            "pod".into(),
            None,
            None,
            None,
            "exec-0-abcd".into(),
            Some(80),
            Some(24),
            window.clone(),
            app.handle().clone(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();

        exec_input(id, "ls\n".into(), app.state()).await.unwrap();
        exec_resize(id, 120, 40, app.state()).await.unwrap();
        exec_close(id, window.clone(), app.state(), app.state())
            .await
            .unwrap();
        // Unknown session: every command stays a quiet no-op.
        exec_input(id + 1, "x".into(), app.state()).await.unwrap();
        exec_resize(id + 1, 80, 24, app.state()).await.unwrap();
        exec_close(id + 1, window, app.state(), app.state())
            .await
            .unwrap();
    }
}
