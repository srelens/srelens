//! Tauri adapter for the local kubectl terminal: the PTY core lives in
//! srelens_streams::terminal; this module maps the Tauri command surface and
//! merges desktop kubeconfig discovery with any pasted/extra kubeconfigs.

use std::sync::Arc;

use srelens_streams::terminal::TerminalManager;
use tauri::{AppHandle, Runtime, State, Window};

use crate::sink::TauriSink;
use crate::window_streams::{Stream, WindowStreams};

/// Start a local shell scoped to `context`. Returns the session id; output
/// streams on `term:out:<channel>` and a `term:exit:<channel>` event fires
/// when it ends, where `channel` is the caller-supplied subscription token.
///
/// The shell belongs to the calling window and is killed when it closes or
/// reloads (#735). One whose window reloaded while it was starting is killed
/// at once and refused.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn start_terminal<R: Runtime>(
    context: String,
    extra_kubeconfigs: Vec<String>,
    channel: String,
    cols: Option<u16>,
    rows: Option<u16>,
    app: AppHandle<R>,
    window: Window<R>,
    manager: State<'_, TerminalManager>,
    owned: State<'_, WindowStreams>,
) -> Result<u64, String> {
    let epoch = owned.epoch(window.label());
    let mut paths = crate::capabilities::all_kubeconfig_paths();
    paths.extend(extra_kubeconfigs.iter().map(std::path::PathBuf::from));
    let session = manager
        .start(
            Arc::new(TauriSink(app)),
            context,
            paths,
            channel,
            cols,
            rows,
        )
        .await?;
    owned.keep(window.label(), epoch, Stream::Terminal(session), || {
        manager.close(session)
    })?;
    Ok(session)
}

/// Forward keystrokes / pasted input to a terminal's stdin. Only the window
/// that opened the terminal may type into it (#733, #735): it is a shell on
/// this machine. A terminal no window holds has ended, and this is a no-op.
#[tauri::command]
pub async fn terminal_input<R: Runtime>(
    session: u64,
    data: String,
    window: Window<R>,
    manager: State<'_, TerminalManager>,
    owned: State<'_, WindowStreams>,
) -> Result<(), String> {
    if owned.check(window.label(), &Stream::Terminal(session))? {
        manager.input(session, &data);
    }
    Ok(())
}

/// Resize a terminal's PTY (columns/rows) to match the xterm viewport; as
/// [`terminal_input`].
#[tauri::command]
pub async fn terminal_resize<R: Runtime>(
    session: u64,
    cols: u16,
    rows: u16,
    window: Window<R>,
    manager: State<'_, TerminalManager>,
    owned: State<'_, WindowStreams>,
) -> Result<(), String> {
    if owned.check(window.label(), &Stream::Terminal(session))? {
        manager.resize(session, cols, rows);
    }
    Ok(())
}

/// Close a terminal: kill the shell and drop the session; as
/// [`terminal_input`].
#[tauri::command]
pub async fn terminal_close<R: Runtime>(
    session: u64,
    window: Window<R>,
    manager: State<'_, TerminalManager>,
    owned: State<'_, WindowStreams>,
) -> Result<(), String> {
    let stream = Stream::Terminal(session);
    if owned.check(window.label(), &stream)? {
        manager.close(session);
        owned.disown(window.label(), &stream);
    }
    Ok(())
}
