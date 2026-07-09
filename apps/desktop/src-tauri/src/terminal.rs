//! Local pseudo-terminal bridge: spawns the user's login shell in a PTY,
//! scoped to a kube context, and streams stdout to the WebView over Tauri
//! events (stdin/resize come back the same way). This is the in-app `kubectl`
//! terminal — a LOCAL process on the user's machine, distinct from the in-pod
//! exec bridge. Desktop-only; never exposed through the MCP capability registry.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use tauri::{AppHandle, Emitter, State};

struct Session {
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    child: Mutex<Box<dyn Child + Send + Sync>>,
    /// The temp overlay kubeconfig to clean up when the session closes.
    overlay: PathBuf,
}

/// Tauri-managed state owning running local terminals (keyed by numeric id).
pub struct TerminalManager {
    next_id: AtomicU64,
    sessions: Mutex<HashMap<u64, Arc<Session>>>,
}

impl TerminalManager {
    pub fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            sessions: Mutex::new(HashMap::new()),
        }
    }
}

/// Write a **standalone** kubeconfig that contains only `context` (and the one
/// cluster/user it references) to a private temp file. Pointing `KUBECONFIG` at
/// just this file locks the terminal to that cluster — no other contexts are
/// listed or switchable. Returns the temp file path.
fn write_locked_kubeconfig(id: u64, context: &str, extra: &[String]) -> Result<PathBuf, String> {
    let mut paths: Vec<PathBuf> = crate::capabilities::default_kubeconfig_paths();
    paths.extend(extra.iter().map(PathBuf::from));
    let yaml = srelens_kube::connect::single_context_kubeconfig_yaml(&paths, context)?;

    let path = std::env::temp_dir().join(format!("srelens-term-{id}.kubeconfig"));
    std::fs::write(&path, yaml).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(path)
}

/// Start a local shell scoped to `context`. Returns the session id; output
/// streams on `term:out:<id>` and a `term:exit:<id>` event fires when it ends.
#[tauri::command]
pub async fn start_terminal(
    context: String,
    extra_kubeconfigs: Vec<String>,
    cols: Option<u16>,
    rows: Option<u16>,
    app: AppHandle,
    manager: State<'_, TerminalManager>,
) -> Result<u64, String> {
    let id = manager.next_id.fetch_add(1, Ordering::SeqCst);
    let overlay = write_locked_kubeconfig(id, &context, &extra_kubeconfigs)?;
    let kubeconfig = overlay.clone().into_os_string();

    let pty = native_pty_system();
    let pair = pty
        .openpty(PtySize {
            rows: rows.unwrap_or(24),
            cols: cols.unwrap_or(80),
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| e.to_string())?;

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
    let mut cmd = CommandBuilder::new(&shell);
    // Propagate the parent environment (PATH is already resolved by fix-path-env
    // at startup, so kubectl / helm / cloud CLIs are found), then scope kubectl.
    for (key, value) in std::env::vars() {
        cmd.env(key, value);
    }
    cmd.env("KUBECONFIG", &kubeconfig);
    cmd.env("TERM", "xterm-256color");
    cmd.env("SRELENS_CONTEXT", &context);
    if let Ok(home) = std::env::var("HOME") {
        cmd.cwd(home);
    }

    let child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
    // Drop the slave so the reader sees EOF once the child exits.
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let writer = pair.master.take_writer().map_err(|e| e.to_string())?;

    let session = Arc::new(Session {
        master: Mutex::new(pair.master),
        writer: Mutex::new(writer),
        child: Mutex::new(child),
        overlay: overlay.clone(),
    });
    manager.sessions.lock().unwrap().insert(id, session);

    // Blocking read loop on a dedicated thread (portable-pty readers are sync).
    let out_channel = format!("term:out:{id}");
    let exit_channel = format!("term:exit:{id}");
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let chunk = String::from_utf8_lossy(&buf[..n]).to_string();
                    let _ = app.emit(&out_channel, chunk);
                }
                Err(_) => break,
            }
        }
        let _ = app.emit(&exit_channel, Option::<String>::None);
        let _ = std::fs::remove_file(&overlay);
    });

    Ok(id)
}

/// Forward keystrokes / pasted input to a terminal's stdin.
#[tauri::command]
pub async fn terminal_input(
    session: u64,
    data: String,
    manager: State<'_, TerminalManager>,
) -> Result<(), String> {
    let s = manager.sessions.lock().unwrap().get(&session).cloned();
    if let Some(s) = s {
        let mut writer = s.writer.lock().unwrap();
        let _ = writer.write_all(data.as_bytes());
        let _ = writer.flush();
    }
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
    let s = manager.sessions.lock().unwrap().get(&session).cloned();
    if let Some(s) = s {
        let _ = s.master.lock().unwrap().resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
    }
    Ok(())
}

/// Close a terminal: kill the shell and drop the session.
#[tauri::command]
pub async fn terminal_close(session: u64, manager: State<'_, TerminalManager>) -> Result<(), String> {
    if let Some(s) = manager.sessions.lock().unwrap().remove(&session) {
        let _ = s.child.lock().unwrap().kill();
        let _ = std::fs::remove_file(&s.overlay);
    }
    Ok(())
}
