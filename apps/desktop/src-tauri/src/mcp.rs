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

fn stop_running(manager: &McpHttpManager, pending: &crate::mcp_confirm::Pending) {
    // Release every in-flight confirm as denied first, so a call blocked on a
    // dialog fails fast instead of hanging until its timeout once the
    // transport it belongs to is gone.
    pending.deny_all();
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

/// Bind `port` and serve the MCP HTTP transport on it, replacing whatever was
/// previously running. Shared by `mcp_http_start` (a fresh start/restart from
/// the toggle) and `mcp_token_rotate` (a same-port restart so a freshly
/// rotated token takes effect at once instead of leaving the old token
/// accepted by an already-running listener). Bind happens before anything
/// else so a port conflict is reported to the caller immediately.
async fn start_server(
    port: u16,
    app: &tauri::AppHandle,
    manager: &McpHttpManager,
    pending: &Arc<crate::mcp_confirm::Pending>,
    token: srelens_mcp::auth::Token,
    audit_path: &std::path::Path,
) -> Result<String, String> {
    stop_running(manager, pending);

    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("Could not bind {addr}: {e}"))?;

    let registry = build_registry_with(manager.cache.clone());
    let server = srelens_mcp::McpServer::new(Arc::new(registry))
        .with_policy(Arc::new(crate::mcp_confirm::PromptUser::new(
            app.clone(),
            pending.clone(),
            std::time::Duration::from_secs(60),
        )))
        .with_audit(Arc::new(srelens_mcp::audit::JsonlAuditLog::new(
            audit_path.to_path_buf(),
            5 * 1024 * 1024,
        )));
    let (tx, rx) = oneshot::channel();
    let handle = tokio::spawn(async move {
        let _ = srelens_mcp::http::serve_http_with_shutdown(
            server,
            listener,
            async {
                let _ = rx.await;
            },
            Some(token),
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

/// Start (or restart) the loopback MCP HTTP server on `port`. Returns its URL.
#[tauri::command]
pub async fn mcp_http_start(
    port: u16,
    app: tauri::AppHandle,
    manager: State<'_, McpHttpManager>,
    pending: State<'_, Arc<crate::mcp_confirm::Pending>>,
    token_store: State<'_, Arc<dyn srelens_mcp::auth::TokenStore>>,
    audit_path: State<'_, McpAuditPath>,
) -> Result<String, String> {
    // The HTTP transport must never serve unauthenticated: mint a token on
    // first use if one hasn't been generated yet.
    let token = match token_store.load() {
        Some(t) => t,
        None => {
            let t = srelens_mcp::auth::Token::generate();
            token_store.save(&t).map_err(|e| e.to_string())?;
            t
        }
    };

    start_server(port, &app, &manager, pending.inner(), token, &audit_path.0).await
}

/// Stop the MCP HTTP server if running.
#[tauri::command]
pub fn mcp_http_stop(
    manager: State<'_, McpHttpManager>,
    pending: State<'_, Arc<crate::mcp_confirm::Pending>>,
) -> Result<(), String> {
    stop_running(&manager, &pending);
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

/// Resolve a pending MCP confirm dialog. `approved` decides whether the
/// blocked tool call proceeds or is refused.
#[tauri::command]
pub fn mcp_confirm_respond(
    id: String,
    approved: bool,
    pending: State<'_, Arc<crate::mcp_confirm::Pending>>,
) -> Result<(), String> {
    if pending.resolve(&id, approved) {
        Ok(())
    } else {
        Err("that confirmation is no longer waiting (it timed out or was already answered)".into())
    }
}

/// The current MCP bearer token, if one has been generated.
#[tauri::command]
pub fn mcp_token_get(store: State<'_, Arc<dyn srelens_mcp::auth::TokenStore>>) -> Option<String> {
    store.load().map(|t| t.as_str().to_string())
}

/// Generate and persist a fresh MCP bearer token, replacing any existing one.
/// If the HTTP server is currently running, restarts it on the same port so
/// the new token takes effect immediately — the previously running listener
/// had the old token baked into its middleware state and would otherwise
/// keep accepting it until the app restarted. If the server is not running,
/// it is left stopped: rotating a token must never switch the server on.
#[tauri::command]
pub async fn mcp_token_rotate(
    app: tauri::AppHandle,
    store: State<'_, Arc<dyn srelens_mcp::auth::TokenStore>>,
    manager: State<'_, McpHttpManager>,
    pending: State<'_, Arc<crate::mcp_confirm::Pending>>,
    audit_path: State<'_, McpAuditPath>,
) -> Result<String, String> {
    let t = srelens_mcp::auth::Token::generate();
    store.save(&t).map_err(|e| e.to_string())?;

    // Read the running port (if any) and drop the lock before restarting —
    // `start_server` takes it again via `stop_running`.
    let running_port = manager.running.lock().unwrap().as_ref().map(|r| r.addr.port());
    if let Some(port) = running_port {
        start_server(port, &app, &manager, pending.inner(), t.clone(), &audit_path.0).await?;
    }

    Ok(t.as_str().to_string())
}

/// Where the MCP bearer token is actually stored right now: `"keychain"` when
/// the OS keychain is serving, `"file"` once it has fallen back to the 0600
/// file. Reads the live flag off the store (flipped the first time a keychain
/// operation genuinely failed) rather than a value guessed at startup, so
/// Settings reports observed reality even if the keychain looked available
/// when the app launched but failed on first use.
#[tauri::command]
pub fn mcp_token_storage(
    store: State<'_, Arc<crate::token_store::ResilientTokenStore>>,
) -> &'static str {
    store.current_backend()
}

/// Revoke the MCP bearer token and stop the HTTP transport — it must never
/// serve unauthenticated.
#[tauri::command]
pub fn mcp_token_revoke(
    store: State<'_, Arc<dyn srelens_mcp::auth::TokenStore>>,
    manager: State<'_, McpHttpManager>,
    pending: State<'_, Arc<crate::mcp_confirm::Pending>>,
) -> Result<(), String> {
    store.clear().map_err(|e| e.to_string())?;
    stop_running(&manager, &pending);
    Ok(())
}

/// The most recent `limit` MCP audit records, newest first.
#[tauri::command]
pub fn mcp_audit_tail(limit: usize, path: State<'_, McpAuditPath>) -> Vec<serde_json::Value> {
    let body = std::fs::read_to_string(&path.0).unwrap_or_default();
    let mut lines: Vec<serde_json::Value> = body
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    lines.reverse(); // newest first
    lines.truncate(limit);
    lines
}

/// Path to the audit log, managed so the command can read it without
/// reconstructing the app config dir.
pub struct McpAuditPath(pub std::path::PathBuf);

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
