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
///
/// `command`, when given, runs first and the shell follows it (#820). It adds
/// nothing this command did not already grant: the caller is handed a shell on
/// this machine either way, and the web host denies the command outright.
///
/// `namespace`, when given, is the terminal's default namespace (#846).
///
/// `readOnly`, when true, makes the shell's `kubectl` and `helm` refuse every
/// subcommand that is not a read: the page asks for it on a protected cluster
/// (#101). Absent means false.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn start_terminal<R: Runtime>(
    context: String,
    extra_kubeconfigs: Vec<String>,
    channel: String,
    cols: Option<u16>,
    rows: Option<u16>,
    command: Option<String>,
    namespace: Option<String>,
    read_only: Option<bool>,
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
            command,
            namespace,
            read_only.unwrap_or(false),
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tauri::{Listener, Manager};

    /// One context, `srelens-101-ro`, at an address nothing answers on.
    const KUBECONFIG: &str = "apiVersion: v1\nkind: Config\n\
        clusters:\n- name: c\n  cluster:\n    server: https://127.0.0.1:1\n\
        users:\n- name: u\n  user: {}\n\
        contexts:\n- name: srelens-101-ro\n  context:\n    cluster: c\n    user: u\n";

    /// What the page sends, `readOnly` as core's `startLocalTerminal` spells
    /// it, reaches the shell's guards: a write typed in the terminal is
    /// refused there (#101).
    #[test]
    fn a_terminal_the_page_starts_read_only_refuses_a_write() {
        let config = tempfile::tempdir().unwrap();
        let path = config.path().join("config");
        std::fs::write(&path, KUBECONFIG).unwrap();
        let app = tauri::test::mock_builder()
            .invoke_handler(tauri::generate_handler![start_terminal])
            .manage(TerminalManager::new())
            .manage(WindowStreams::default())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        let out = Arc::new(Mutex::new(String::new()));
        let heard = Arc::clone(&out);
        app.listen_any("term:out:t-ro", move |e| {
            let chunk: String = serde_json::from_str(e.payload()).unwrap_or_default();
            heard.lock().unwrap().push_str(&chunk);
        });

        let request = tauri::webview::InvokeRequest {
            cmd: "start_terminal".into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: "tauri://localhost".parse().unwrap(),
            body: tauri::ipc::InvokeBody::Json(serde_json::json!({
                "context": "srelens-101-ro",
                "extraKubeconfigs": [path.display().to_string()],
                "channel": "t-ro",
                "cols": 200,
                "rows": 24,
                "command": null,
                "namespace": null,
                "readOnly": true,
            })),
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.to_string(),
        };
        let session: u64 = tauri::test::get_ipc_response(&webview, request)
            .unwrap()
            .deserialize()
            .unwrap();
        app.state::<TerminalManager>()
            .input(session, "kubectl delete pods --all\n");
        let refused = (0..100).any(|_| {
            std::thread::sleep(std::time::Duration::from_millis(100));
            out.lock().unwrap().contains("this terminal is read-only")
        });
        app.state::<TerminalManager>().close(session);

        assert!(refused, "{}", out.lock().unwrap());
    }
}
