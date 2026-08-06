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
    /// Serializes whole lifecycle transitions. `running` is a std mutex that
    /// cannot be held across an await, so on its own it only makes each
    /// individual read/write atomic — not the stop-then-rebind sequence around
    /// them. Two overlapping transitions (the Settings toggle and a token
    /// rotate are separate controls) could therefore both pass teardown and
    /// then both bind the same port, or the later one could overwrite a live
    /// `Running` and orphan a task still holding the listener.
    lifecycle: tokio::sync::Mutex<()>,
}

impl McpHttpManager {
    pub fn new(cache: Arc<ClientCache>) -> Self {
        Self {
            cache,
            running: Mutex::new(None),
            lifecycle: tokio::sync::Mutex::new(()),
        }
    }

    /// Take the lifecycle lock for the duration of a start/stop/rotate. The
    /// resulting guard is threaded into [`stop_running`] and [`start_server`]
    /// as a witness, so neither can be called without it being held.
    async fn lifecycle(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.lifecycle.lock().await
    }
}

/// Proof that the caller holds `McpHttpManager::lifecycle`. Only exists to make
/// "you must hold the lock" a compile-time requirement rather than a comment.
type Lifecycle<'a> = tokio::sync::MutexGuard<'a, ()>;

/// Stop whatever server is currently registered, if any, and wait for its
/// task to actually finish before returning. Signals graceful shutdown first
/// (axum's graceful shutdown waits on in-flight keep-alive connections, which
/// an idle MCP client may be holding open) and only falls back to `abort()`
/// if that doesn't finish promptly — aborting alone does not synchronously
/// drop the task's future, so the old listener fd can still be open when the
/// caller immediately tries to rebind the same port.
async fn stop_running(
    manager: &McpHttpManager,
    pending: &crate::mcp_confirm::Pending,
    _lifecycle: &Lifecycle<'_>,
) {
    // Release every in-flight confirm as denied first, so a call blocked on a
    // dialog fails fast instead of hanging until its timeout once the
    // transport it belongs to is gone.
    pending.deny_all();
    let running = manager.running.lock().unwrap().take();
    if let Some(mut running) = running {
        if let Some(tx) = running.shutdown.take() {
            let _ = tx.send(());
        }
        let mut handle = running.handle;
        if tokio::time::timeout(std::time::Duration::from_secs(2), &mut handle)
            .await
            .is_err()
        {
            handle.abort();
            let _ = handle.await;
        }
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
    prompts_dir: &std::path::Path,
    lifecycle: &Lifecycle<'_>,
) -> Result<String, String> {
    stop_running(manager, pending, lifecycle).await;

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
        )))
        .with_prompts(srelens_mcp::prompts::PromptLibrary::new(Some(
            prompts_dir.to_path_buf(),
        )));
    let (tx, rx) = oneshot::channel();
    let handle = tokio::spawn(async move {
        let _ = srelens_mcp::http::serve_http_with_shutdown(
            server,
            listener,
            async {
                let _ = rx.await;
            },
            token,
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
    prompts_dir: State<'_, McpPromptsDir>,
) -> Result<String, String> {
    // Taken BEFORE the token is read, not just around the restart: a rotate
    // that lands in between would persist a new token and restart the server
    // with it, and then this start would rebind with the token it had already
    // loaded — leaving the transport serving a value the store no longer holds,
    // so every client configured from Settings gets a 401.
    let lifecycle = manager.lifecycle().await;

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

    start_server(
        port,
        &app,
        &manager,
        pending.inner(),
        token,
        &audit_path.0,
        &prompts_dir.0,
        &lifecycle,
    )
    .await
}

/// Stop the MCP HTTP server if running.
#[tauri::command]
pub async fn mcp_http_stop(
    manager: State<'_, McpHttpManager>,
    pending: State<'_, Arc<crate::mcp_confirm::Pending>>,
) -> Result<(), String> {
    let lifecycle = manager.lifecycle().await;
    stop_running(&manager, &pending, &lifecycle).await;
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
    prompts_dir: State<'_, McpPromptsDir>,
) -> Result<String, String> {
    // Held across persist-then-restart, not just the restart: a revoke landing
    // between the two would clear the store and stop the server, and this
    // rotate would then leave a freshly persisted token behind it — Settings
    // showing a live token for a server that was deliberately switched off.
    let lifecycle = manager.lifecycle().await;

    let t = srelens_mcp::auth::Token::generate();
    store.save(&t).map_err(|e| e.to_string())?;

    let running_port = manager.running.lock().unwrap().as_ref().map(|r| r.addr.port());
    if let Some(port) = running_port {
        start_server(
            port,
            &app,
            &manager,
            pending.inner(),
            t.clone(),
            &audit_path.0,
            &prompts_dir.0,
            &lifecycle,
        )
        .await?;
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
pub async fn mcp_token_revoke(
    store: State<'_, Arc<dyn srelens_mcp::auth::TokenStore>>,
    manager: State<'_, McpHttpManager>,
    pending: State<'_, Arc<crate::mcp_confirm::Pending>>,
) -> Result<(), String> {
    // Same ordering rule as rotate: clear and stop are one transition, so a
    // concurrent start can't load the token between them and bring the server
    // back up on a value that has just been revoked.
    let lifecycle = manager.lifecycle().await;
    store.clear().map_err(|e| e.to_string())?;
    stop_running(&manager, &pending, &lifecycle).await;
    Ok(())
}

/// The most recent `limit` MCP audit records, newest first. Reading is owned by
/// `srelens_mcp::audit`, which also writes the format and so knows how to tail
/// it without parsing the whole 5 MB file.
#[tauri::command]
pub fn mcp_audit_tail(limit: usize, path: State<'_, McpAuditPath>) -> Vec<serde_json::Value> {
    srelens_mcp::audit::tail(&path.0, limit)
}

/// Path to the audit log, managed so the command can read it without
/// reconstructing the app config dir.
pub struct McpAuditPath(pub std::path::PathBuf);

/// Where user-authored prompt files live, managed so the commands and the
/// server builder agree on one path without recomputing the config dir.
pub struct McpPromptsDir(pub std::path::PathBuf);

/// Prompt files that could not be loaded, so Settings can say which file was
/// skipped and why — a silently-ignored file is a miserable authoring
/// experience. Re-reads the directory, so it reflects the files on disk now.
#[tauri::command]
pub fn mcp_prompt_issues(
    dir: State<'_, McpPromptsDir>,
) -> Vec<srelens_mcp::prompts::LoadIssue> {
    srelens_mcp::prompts::PromptLibrary::new(Some(dir.0.clone())).issues()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_library_pointed_at_a_user_dir_reports_that_dirs_issues() {
        let dir = std::env::temp_dir().join(format!("srelens-pd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("broken.md"), "not a prompt\n").unwrap();

        let lib = srelens_mcp::prompts::PromptLibrary::new(Some(dir));
        let issues = lib.issues();
        assert!(
            issues.iter().any(|i| i.file == "broken.md"),
            "Settings needs the reason a file was skipped, got: {issues:?}"
        );
    }

    /// The bug this closes: `stop_running` used to send the shutdown signal
    /// and `abort()` the task without waiting for either to actually finish,
    /// so `start_server`'s immediate rebind on the same port raced the old
    /// listener's fd closing — the reviewer reproduced 38/50 failures. This
    /// drives the exact shape of that race (bind, register as `Running`,
    /// `stop_running`, rebind the identical port) 50 times in a row; with the
    /// fix (`await` the handle, with a bounded `abort()` fallback) every
    /// rebind must succeed.
    /// Mirrors `start_server`'s lifecycle shape (take the lock, tear the old
    /// server down, rebind, register) without the Tauri bits `start_server`
    /// needs — an `AppHandle` can't be built in a unit test. The listener-owning
    /// task stands in for axum holding the socket.
    async fn restart_on(
        manager: &McpHttpManager,
        pending: &crate::mcp_confirm::Pending,
        addr: SocketAddr,
    ) -> std::io::Result<()> {
        let guard = manager.lifecycle().await;
        stop_running(manager, pending, &guard).await;
        let listener = tokio::net::TcpListener::bind(addr).await?;
        let (tx, rx) = oneshot::channel::<()>();
        let handle = tokio::spawn(async move {
            let _ = rx.await;
            drop(listener);
        });
        *manager.running.lock().unwrap() = Some(Running {
            addr,
            shutdown: Some(tx),
            handle,
        });
        Ok(())
    }

    /// Rotating a token restarts the server, and the Settings toggle starts and
    /// stops it — two separate controls that can overlap. Without a lock around
    /// the whole transition, both can complete teardown (each seeing no running
    /// server) and then race to bind the same port, so one loses with
    /// `Address already in use` and the user sees a spurious failure.
    #[tokio::test]
    async fn concurrent_lifecycle_transitions_do_not_race_the_bind() {
        let manager = Arc::new(McpHttpManager::new(ClientCache::new(
            std::path::PathBuf::from("/dev/null"),
        )));
        let pending = Arc::new(crate::mcp_confirm::Pending::default());

        // Start with a server ALREADY running on the port. This is what opens
        // the window: the first transition's teardown awaits the old task's
        // JoinHandle, and that await is where the second transition gets in —
        // finding `running` already taken, so it sails past teardown and binds
        // the port the first one is about to rebind. With no server running
        // there is no await in teardown, the two never interleave, and the test
        // would pass against the racy code.
        let addr = {
            let listener = tokio::net::TcpListener::bind(SocketAddr::from((
                Ipv4Addr::LOCALHOST,
                0,
            )))
            .await
            .unwrap();
            let addr = listener.local_addr().unwrap();
            let (tx, rx) = oneshot::channel::<()>();
            let handle = tokio::spawn(async move {
                let _ = rx.await;
                // Mirror axum's unhurried release of the socket.
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                drop(listener);
            });
            *manager.running.lock().unwrap() = Some(Running {
                addr,
                shutdown: Some(tx),
                handle,
            });
            addr
        };

        let a = tokio::spawn({
            let (m, p) = (manager.clone(), pending.clone());
            async move { restart_on(&m, &p, addr).await }
        });
        let b = tokio::spawn({
            let (m, p) = (manager.clone(), pending.clone());
            async move { restart_on(&m, &p, addr).await }
        });

        a.await.unwrap().expect("first transition must bind");
        b.await.unwrap().expect("second transition must bind after the first tore down");
    }

    #[tokio::test]
    async fn stop_running_releases_the_port_before_returning() {
        let cache = ClientCache::new(std::path::PathBuf::from("/dev/null"));
        let manager = McpHttpManager::new(cache);
        let pending = crate::mcp_confirm::Pending::default();

        for i in 0..50 {
            let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, 0)); // OS-assigned port
            let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
            let bound_addr = listener.local_addr().unwrap();
            let (tx, rx) = oneshot::channel::<()>();
            // Mirrors `serve_http_with_shutdown`: the task owns the listener
            // and only lets go of it once told to shut down, just like axum
            // holding the socket open for an idle keep-alive connection.
            let handle = tokio::spawn(async move {
                let _ = rx.await;
                drop(listener);
            });
            *manager.running.lock().unwrap() = Some(Running {
                addr: bound_addr,
                shutdown: Some(tx),
                handle,
            });

            {
                let guard = manager.lifecycle().await;
                stop_running(&manager, &pending, &guard).await;
            }

            tokio::net::TcpListener::bind(bound_addr).await.unwrap_or_else(|e| {
                panic!("rebind attempt {i} on {bound_addr} failed: {e}")
            });
        }
    }
}
