//! Tauri adapter for the local kubectl terminal: the PTY core lives in
//! srelens_streams::terminal; this module maps the Tauri command surface and
//! merges desktop kubeconfig discovery with any pasted/extra kubeconfigs.

use std::sync::Arc;

use srelens_streams::terminal::TerminalManager;
use tauri::{AppHandle, State};

use crate::sink::TauriSink;

/// Start a local shell scoped to `context`. Returns the session id; output
/// streams on `term:out:<channel>` and a `term:exit:<channel>` event fires
/// when it ends, where `channel` is the caller-supplied subscription token.
#[tauri::command]
pub async fn start_terminal(
    context: String,
    extra_kubeconfigs: Vec<String>,
    channel: String,
    cols: Option<u16>,
    rows: Option<u16>,
    app: AppHandle,
    manager: State<'_, TerminalManager>,
) -> Result<u64, String> {
    let mut paths = crate::capabilities::default_kubeconfig_paths();
    paths.extend(extra_kubeconfigs.iter().map(std::path::PathBuf::from));
    manager
        .start(
            Arc::new(TauriSink(app)),
            context,
            paths,
            channel,
            cols,
            rows,
        )
        .await
}

/// Forward keystrokes / pasted input to a terminal's stdin.
#[tauri::command]
pub async fn terminal_input(
    session: u64,
    data: String,
    manager: State<'_, TerminalManager>,
) -> Result<(), String> {
    manager.input(session, &data);
    Ok(())
}

/// Resize a terminal's PTY (columns/rows) to match the xterm viewport.
#[tauri::command]
pub async fn terminal_resize(
    session: u64,
    cols: u16,
    rows: u16,
    manager: State<'_, TerminalManager>,
) -> Result<(), String> {
    manager.resize(session, cols, rows);
    Ok(())
}

/// Close a terminal: kill the shell and drop the session.
#[tauri::command]
pub async fn terminal_close(
    session: u64,
    manager: State<'_, TerminalManager>,
) -> Result<(), String> {
    manager.close(session);
    Ok(())
}
