//! In-app MCP HTTP server lifecycle and `srelens` CLI install, driven from the
//! Settings → MCP section. The HTTP server shares the app's authenticated
//! client cache, so an MCP client can drive the same clusters the GUI sees.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use srelens_kube::client_cache::ClientCache;
use tauri::State;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::capabilities::build_registry_with;

struct Running {
    addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    handle: JoinHandle<()>,
}

/// Tauri-managed state owning the running MCP HTTP server (if any).
pub struct McpHttpManager {
    cache: Arc<ClientCache>,
    running: Mutex<Option<Running>>,
}

impl McpHttpManager {
    pub fn new(cache: Arc<ClientCache>) -> Self {
        Self {
            cache,
            running: Mutex::new(None),
        }
    }
}

fn stop_running(manager: &McpHttpManager) {
    if let Some(mut running) = manager.running.lock().unwrap().take() {
        if let Some(tx) = running.shutdown.take() {
            let _ = tx.send(());
        }
        running.handle.abort();
    }
}

fn url_for(addr: SocketAddr) -> String {
    format!("http://{addr}/mcp")
}

/// Start (or restart) the loopback MCP HTTP server on `port`. Returns its URL.
#[tauri::command]
pub async fn mcp_http_start(port: u16, manager: State<'_, McpHttpManager>) -> Result<String, String> {
    stop_running(&manager);

    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    // Bind up front so a port conflict is reported to the UI immediately.
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("Could not bind {addr}: {e}"))?;

    let registry = build_registry_with(manager.cache.clone());
    let server = srelens_mcp::McpServer::new(Arc::new(registry));
    let (tx, rx) = oneshot::channel();
    let handle = tokio::spawn(async move {
        let _ = srelens_mcp::http::serve_http_with_shutdown(
            server,
            listener,
            async {
                let _ = rx.await;
            },
            None,
        )
        .await;
    });

    *manager.running.lock().unwrap() = Some(Running {
        addr,
        shutdown: Some(tx),
        handle,
    });
    Ok(url_for(addr))
}

/// Stop the MCP HTTP server if running.
#[tauri::command]
pub fn mcp_http_stop(manager: State<'_, McpHttpManager>) -> Result<(), String> {
    stop_running(&manager);
    Ok(())
}

/// The MCP HTTP server's URL if it's currently running.
#[tauri::command]
pub fn mcp_http_status(manager: State<'_, McpHttpManager>) -> Option<String> {
    manager
        .running
        .lock()
        .unwrap()
        .as_ref()
        .map(|running| url_for(running.addr))
}

/// Where the `srelens` CLI symlink is installed, and whether it points at us.
#[derive(Debug, Serialize)]
pub struct CliStatus {
    installed: bool,
    /// The install path (`~/.local/bin/srelens`).
    path: String,
    /// What the symlink resolves to, if present.
    links_to: Option<String>,
    /// Whether the install directory is on the current `$PATH`.
    on_path: bool,
}

/// User-writable install dir — no elevation needed, unlike `/usr/local/bin`.
fn cli_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".local").join("bin"))
}

fn cli_path() -> Option<std::path::PathBuf> {
    cli_dir().map(|dir| dir.join("srelens"))
}

/// Whether `dir` is one of the entries in `$PATH`.
fn dir_on_path(dir: &std::path::Path) -> bool {
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).any(|entry| entry == dir))
        .unwrap_or(false)
}

/// Report whether the `srelens` CLI is installed and where it points.
#[tauri::command]
pub fn srelens_cli_status() -> CliStatus {
    let dir = cli_dir();
    let path = cli_path();
    CliStatus {
        installed: path.as_ref().is_some_and(|p| p.exists()),
        path: path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        links_to: path
            .as_ref()
            .and_then(|p| std::fs::read_link(p).ok())
            .map(|p| p.to_string_lossy().to_string()),
        on_path: dir.as_deref().is_some_and(dir_on_path),
    }
}

/// Symlink the running executable to `~/.local/bin/srelens` so MCP clients can
/// spawn `srelens --mcp-stdio`. Creates the directory if needed (no elevation);
/// returns the install path on success, or the manual command on failure.
#[tauri::command]
pub fn install_srelens_cli() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;

    #[cfg(unix)]
    {
        let dir = cli_dir().ok_or("Could not resolve $HOME")?;
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("Could not create {} ({e})", dir.display()))?;
        let target = dir.join("srelens");
        // Replace any existing symlink/file at the target.
        if target.symlink_metadata().is_ok() {
            let _ = std::fs::remove_file(&target);
        }
        match std::os::unix::fs::symlink(&exe, &target) {
            Ok(()) => Ok(target.to_string_lossy().to_string()),
            Err(e) => Err(format!(
                "Could not write {} ({e}). Run this in a terminal:\n  ln -sf \"{}\" \"{}\"",
                target.display(),
                exe.display(),
                target.display()
            )),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = exe;
        Err("Installing the srelens CLI is only supported on macOS/Linux.".to_string())
    }
}
