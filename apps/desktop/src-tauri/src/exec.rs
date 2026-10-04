//! Tauri adapter for in-pod exec: the streaming core lives in
//! srelens_streams::exec; this module only maps the Tauri command surface.
//!
//! Commands are generic over the runtime (#28): the unit suite below drives
//! them through `tauri::test::MockRuntime`, so this surface counts toward
//! coverage instead of hiding behind the ignore-regex.

use std::sync::Arc;

use serde_json::Value;
use srelens_streams::exec::{ExecManager, ExecOpts};
use tauri::ipc::Channel;
use tauri::{AppHandle, Runtime, State, Window};

use crate::node_shells::{self, NodeShells, ShellEnd, ShellSink};
use crate::sink::ChannelSink;
use crate::window_streams::{Stream, WindowStreams};

/// Open an interactive shell into a pod. Returns the session id; stdout streams
/// on `exec:out:<channel>` and an `exec:exit:<channel>` event fires (with an
/// optional error string) when the session ends, where `channel` is the
/// caller-supplied subscription token — the WebView subscribes to it before
/// this call, so an exec that dies in the same tick it spawns cannot outrun
/// the listener. Both travel on `on_event`, the page's own channel, so no
/// other window receives them (#733).
///
/// The session belongs to the calling window and is closed when it closes or
/// reloads (#700). One whose window reloaded while it was starting is closed
/// at once and refused.
///
/// A node shell — an exec into a debug pod this window created — owns that
/// pod from here, and it is deleted when the session ends however it ends,
/// including when it fails to start (#734; see `crate::node_shells`).
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
    on_event: Channel<Value>,
    window: Window<R>,
    app: AppHandle<R>,
    manager: State<'_, ExecManager>,
    owned: State<'_, WindowStreams>,
    shells: State<'_, NodeShells>,
) -> Result<u64, String> {
    let epoch = owned.epoch(window.label());
    let end = Arc::new(ShellEnd::default());
    let sink = ShellSink {
        inner: ChannelSink(on_event),
        exit: format!("exec:exit:{channel}"),
        end: end.clone(),
        app: app.clone(),
    };
    let started = manager
        .start(
            Arc::new(sink),
            context.clone(),
            namespace.clone(),
            pod.clone(),
            channel,
            ExecOpts {
                container,
                shell,
                command,
                cols,
                rows,
            },
        )
        .await;
    let debug_pod = owned.take_debug_pod(window.label(), &context, &namespace, &pod);
    let kept = started.and_then(|session| {
        owned
            .keep(window.label(), epoch, Stream::Exec(session), || {
                manager.close(session)
            })
            .map(|()| session)
    });
    match (kept, debug_pod) {
        (Ok(session), Some(debug_pod)) => {
            shells.attach(&app, &owned, window.label(), session, debug_pod);
            end.started(&app, session);
            Ok(session)
        }
        (Ok(session), None) => Ok(session),
        // No shell will run in it now.
        (Err(e), Some(debug_pod)) => {
            node_shells::delete_soon(&app, debug_pod);
            Err(e)
        }
        (Err(e), None) => Err(e),
    }
}

/// Forward a keystroke / input string to an exec session's stdin. Only the
/// window that opened the session may type into it (#733); a session no
/// window holds has ended, and this is a no-op.
#[tauri::command]
pub async fn exec_input<R: Runtime>(
    session: u64,
    data: String,
    window: Window<R>,
    manager: State<'_, ExecManager>,
    owned: State<'_, WindowStreams>,
) -> Result<(), String> {
    if owned.check(window.label(), &Stream::Exec(session))? {
        manager.input(session, data).await;
    }
    Ok(())
}

/// Resize an exec session's remote PTY to `cols` x `rows`; as [`exec_input`].
#[tauri::command]
pub async fn exec_resize<R: Runtime>(
    session: u64,
    cols: u16,
    rows: u16,
    window: Window<R>,
    manager: State<'_, ExecManager>,
    owned: State<'_, WindowStreams>,
) -> Result<(), String> {
    if owned.check(window.label(), &Stream::Exec(session))? {
        manager.resize(session, cols, rows).await;
    }
    Ok(())
}

/// Close an exec session and abort its task; as [`exec_input`].
#[tauri::command]
pub async fn exec_close<R: Runtime>(
    session: u64,
    window: Window<R>,
    app: AppHandle<R>,
    manager: State<'_, ExecManager>,
    owned: State<'_, WindowStreams>,
    shells: State<'_, NodeShells>,
) -> Result<(), String> {
    let stream = Stream::Exec(session);
    if owned.check(window.label(), &stream)? {
        manager.close(session);
        owned.disown(window.label(), &stream);
        if let Some(debug_pod) = shells.release(session) {
            node_shells::delete_soon(&app, debug_pod);
        }
    }
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
        app.manage(NodeShells::default());
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
            crate::sink::tests::recording().0,
            window.clone(),
            app.handle().clone(),
            app.state(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();

        exec_input(id, "ls\n".into(), window.clone(), app.state(), app.state())
            .await
            .unwrap();
        exec_resize(id, 120, 40, window.clone(), app.state(), app.state())
            .await
            .unwrap();
        exec_close(
            id,
            window.clone(),
            app.handle().clone(),
            app.state(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
        // Unknown session: every command stays a quiet no-op.
        exec_input(id + 1, "x".into(), window.clone(), app.state(), app.state())
            .await
            .unwrap();
        exec_resize(id + 1, 80, 24, window.clone(), app.state(), app.state())
            .await
            .unwrap();
        exec_close(
            id + 1,
            window,
            app.handle().clone(),
            app.state(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
    }
}
