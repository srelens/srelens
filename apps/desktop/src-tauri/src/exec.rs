//! Tauri adapter for in-pod exec: the streaming core lives in
//! srelens_streams::exec; this module only maps the Tauri command surface.

use std::sync::Arc;

use srelens_streams::exec::{ExecManager, ExecOpts};
use tauri::{AppHandle, State};

use crate::sink::TauriSink;

/// Open an interactive shell into a pod. Returns the session id; stdout streams
/// on `exec:out:<id>` and an `exec:exit:<id>` event fires (with an optional
/// error string) when the session ends.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn start_pod_exec(
    context: String,
    namespace: String,
    pod: String,
    container: Option<String>,
    shell: Option<String>,
    command: Option<Vec<String>>,
    cols: Option<u16>,
    rows: Option<u16>,
    app: AppHandle,
    manager: State<'_, ExecManager>,
) -> Result<u64, String> {
    manager
        .start(
            Arc::new(TauriSink(app)),
            context,
            namespace,
            pod,
            ExecOpts {
                container,
                shell,
                command,
                cols,
                rows,
            },
        )
        .await
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
pub async fn exec_close(session: u64, manager: State<'_, ExecManager>) -> Result<(), String> {
    manager.close(session);
    Ok(())
}
