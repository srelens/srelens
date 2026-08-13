// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;

use srelens_mcp::auth::TokenStore as _;

fn main() {
    // `SRELENS_MASTER_PASSWORD`: captured into a LOCAL and immediately
    // scrubbed from the environment, FIRST, before any run-mode dispatch —
    // every run mode (GUI, `--serve`, `--mcp-stdio`, `--mcp-http`) can spawn
    // Helm binaries and kubectl exec credential plugins, and none of them
    // may inherit the vault password. Only `--mcp-http` consumes the value
    // (by move); every other mode drops it before running, so the plaintext
    // does not sit in process memory for the whole session.
    let master_password = std::env::var("SRELENS_MASTER_PASSWORD").ok().filter(|p| !p.is_empty());
    std::env::remove_var("SRELENS_MASTER_PASSWORD");
    // GUI launches (Finder/Dock) inherit launchd's minimal PATH, not the
    // user's shell PATH — kubeconfig exec plugins (kubectl, kubectl-oidc_login,
    // cloud CLIs) then fail to spawn with "No such file or directory". Resolve
    // the login-shell environment before anything creates a kube client.
    if let Err(e) = fix_path_env::fix() {
        eprintln!("warning: could not resolve login-shell PATH: {e}");
    }

    // Apply the SRELENS_TIMEOUT_SECS override up front so every mode — GUI, MCP
    // stdio, and MCP HTTP — honors it (the GUI can adjust it further at runtime).
    srelens_kube::connect::init_timeout_from_env();

    // Running from a Linux AppImage, keep the bundled GLib from scanning the
    // host's GIO modules (its gvfs modules use symbols the bundled GLib lacks,
    // spamming "undefined symbol" on startup). Must happen before anything
    // touches GLib/GTK; we're still single-threaded here. No-op off AppImage.
    #[cfg(target_os = "linux")]
    {
        let extra = std::env::var("GIO_EXTRA_MODULES").ok();
        let existing = std::env::var("GIO_MODULE_DIR").ok();
        if let Some(dir) = srelens_desktop_lib::gio_module_dir_for_appimage(
            extra.as_deref(),
            existing.as_deref(),
        ) {
            std::env::set_var("GIO_MODULE_DIR", dir);
        }
    }

    let args: Vec<String> = std::env::args().collect();
    // `serve [addr] [--data DIR]` runs the web server (frontend + capability
    // API) instead of the GUI. Sessions + OIDC/dev-login auth are required; default bind is loopback.
    if args.get(1).map(String::as_str) == Some("serve") {
        let mut addr: Option<String> = None;
        let mut data: Option<String> = None;
        let mut rest = args[2..].iter();
        while let Some(a) = rest.next() {
            if a == "--data" {
                data = rest.next().cloned();
                if data.is_none() {
                    eprintln!("--data requires a directory argument");
                    std::process::exit(2);
                }
            } else if addr.is_none() {
                addr = Some(a.clone());
            } else {
                eprintln!("unexpected argument: {a}");
                std::process::exit(2);
            }
        }
        drop(master_password);
        run_serve(addr.as_deref().unwrap_or("127.0.0.1:8080"), data.as_deref());
        return;
    }
    // `--mcp-stdio` / `--mcp-http [addr]` run the MCP server instead of the GUI,
    // so external MCP clients/agents can drive every capability. Headless runs
    // get no GUI to prompt for consent, so gated tools need an explicit
    // process-level opt-in rather than the GUI's per-call dialog.
    //
    // Two independent flags, because they are two different risks: mutating the
    // cluster and reading out its secrets. Granting one must not grant the
    // other, so an agent allowed to read a Secret still cannot drain a node.
    let allow_destructive = args.iter().any(|a| a == "--mcp-allow-destructive");
    let allow_sensitive_reads = args.iter().any(|a| a == "--mcp-allow-sensitive-reads");
    if args.iter().any(|a| a == "--mcp-stdio") {
        drop(master_password);
        run_mcp_stdio(allow_destructive, allow_sensitive_reads);
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--mcp-http") {
        // The next arg is the address unless it's itself a flag (e.g.
        // `--mcp-http --mcp-allow-destructive` with no address given).
        let addr = args
            .get(i + 1)
            .filter(|a| !a.starts_with("--"))
            .cloned()
            .unwrap_or_else(|| "127.0.0.1:8765".into());
        run_mcp_http(&addr, allow_destructive, allow_sensitive_reads, master_password);
        return;
    }
    drop(master_password);
    srelens_desktop_lib::run();
}

/// Environment variable carrying a caller-supplied MCP bearer token.
///
/// Deliberately NOT a `--mcp-token` flag: argv is world-readable through `ps`
/// on both Linux and macOS, so a flag would hand the token to every other
/// account on the machine — the exact local-process threat the token exists to
/// stop. A process's environment is readable only by its own user.
const TOKEN_ENV: &str = "SRELENS_MCP_TOKEN";

/// Where the desktop app keeps its MCP bearer token's file fallback, under
/// the app config dir. Headless mode never boots a Tauri `App`, so
/// `app.path().app_config_dir()` (used in `lib.rs`'s setup) isn't callable
/// here. This reproduces that resolver's formula — `dirs::config_dir()/<bundle
/// identifier>` — directly, so the CLI and the GUI open the same secrets
/// vault (`secrets.enc` under this dir, keyed by the same keychain-held
/// master key — see `vault.rs`), and a token provisioned in one is usable
/// from the other.
fn mcp_dir() -> std::path::PathBuf {
    dirs::config_dir()
        .expect("could not resolve the platform config directory")
        .join("app.srelens.desktop") // tauri.conf.json "identifier"
        .join("mcp")
}

/// Where a headless run (`--mcp-stdio` / `--mcp-http`) writes its audit log —
/// `audit.jsonl` in the same `mcp/` directory as the vault.
fn mcp_audit_path() -> std::path::PathBuf {
    mcp_dir().join("audit.jsonl")
}

/// Where a headless run reads user prompt files — `prompts/` in the same `mcp/`
/// directory as the vault, so a prompt authored while the GUI is running is
/// visible to `--mcp-stdio` and `--mcp-http` too.
fn mcp_prompts_dir() -> std::path::PathBuf {
    mcp_dir().join("prompts")
}

/// Same rotation cap the desktop app's in-process MCP server uses (see
/// `McpAuditPath` wiring in `mcp.rs`), so a headless run and the GUI behave
/// identically.
const MCP_AUDIT_CAP_BYTES: u64 = 5 * 1024 * 1024;

fn run_mcp_http(
    addr: &str,
    allow_destructive: bool,
    allow_sensitive_reads: bool,
    master_password: Option<String>,
) {
    let addr: std::net::SocketAddr = addr.parse().expect("invalid --mcp-http address");
    let policy = Arc::new(srelens_mcp::policy::FlagGated::new(
        allow_destructive,
        allow_sensitive_reads,
    ));

    // The HTTP transport must never serve unauthenticated: resolve a token
    // from the environment, then the vault, then generate and persist one.
    // Same vault (and master key) as the GUI, so a token provisioned in one
    // is usable from the other. The vault absorbs a genuinely failed keychain
    // by holding its master key in a 0600 file instead — `save` only errors
    // on a real file-write failure, so `.expect` below can't panic just
    // because there's no D-Bus session on this host.
    let vault = Arc::new(srelens_desktop_lib::vault::Vault::open(&mcp_dir()));
    // A password-protected vault has no GUI here to unlock it: accept the
    // master password from the environment (never argv — it would show in
    // `ps`). Without it, `SRELENS_MCP_TOKEN` still works, and a store-token
    // path that needs to mint exits below with guidance naming both.
    // Both password-derived locked sources unlock with the master password:
    // `biometric-locked` only means the GUI would normally skip the password
    // via Touch ID / Hello — the password (and vault.json) still exist.
    if matches!(vault.key_source(), "password-locked" | "biometric-locked") {
        match master_password {
            Some(password) => {
                if let Err(e) =
                    srelens_desktop_lib::vault::unlock_with_master_password(&vault, &mcp_dir(), &password)
                {
                    eprintln!("srelens: SRELENS_MASTER_PASSWORD did not unlock the vault: {e}");
                    std::process::exit(2);
                }
            }
            None => eprintln!(
                "srelens: the secrets vault is password-locked; set SRELENS_MASTER_PASSWORD to unlock it, or pass the token via {TOKEN_ENV}"
            ),
        }
    }
    let store = srelens_desktop_lib::vault::VaultTokenStore(vault.clone());
    let token = match std::env::var(TOKEN_ENV).ok().filter(|v| !v.trim().is_empty()) {
        Some(hex) => srelens_mcp::auth::Token::from_hex(&hex).unwrap_or_else(|| {
            // The value itself is never echoed — that's the whole point of
            // keeping it out of argv.
            eprintln!("{TOKEN_ENV} must be 64 hex characters");
            std::process::exit(2);
        }),
        None => match store.load() {
            Some(t) => t,
            None => {
                let t = srelens_mcp::auth::Token::generate();
                if let Err(e) = store.save(&t) {
                    // Covers a locked vault (keychain unreachable while a
                    // vault exists) and real write failures — either way the
                    // HTTP transport must not serve with a token nobody can
                    // reuse next run.
                    eprintln!("srelens: could not persist the MCP token: {e}");
                    std::process::exit(2);
                }
                // stderr, not stdout: stdout is the JSON-RPC channel on the
                // stdio transport and must stay parseable. HTTP has no such
                // constraint but stays consistent with stdio here.
                eprintln!("srelens: generated MCP token: {}", t.as_str());
                t
            }
        },
    };
    if vault.key_source() == "file" {
        eprintln!("srelens: no OS keychain available, holding the vault master key in a 0600 file");
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");
    runtime.block_on(async {
        // Built explicitly (rather than via `build_registry`, which creates
        // its own cache internally and does not expose it) so the same cache
        // backs both the registry's reads and the watcher's subscriptions —
        // a second cache would authenticate independently, double the
        // connections, and could diverge from the read path on credential
        // refresh.
        let cache = srelens_kube::client_cache::ClientCache::new_many(
            srelens_registry::default_kubeconfig_paths(),
        );
        let registry = srelens_desktop_lib::build_registry_with_paths(
            cache.clone(),
            srelens_registry::default_kubeconfig_paths(),
        );
        let server = srelens_mcp::McpServer::new(Arc::new(registry))
            .with_policy(policy)
            .with_audit(Arc::new(srelens_mcp::audit::JsonlAuditLog::new(
                mcp_audit_path(),
                MCP_AUDIT_CAP_BYTES,
            )))
            .with_prompts(srelens_mcp::prompts::PromptLibrary::new(Some(
                mcp_prompts_dir(),
            )))
            .with_kind_resolver(srelens_registry::kind_resolver())
            .with_watcher(Arc::new(srelens_desktop_lib::mcp_watch::CacheWatcher::new(
                cache,
            )));
        eprintln!(
            "MCP HTTP listening on http://{addr}/mcp (loopback; gated tools need _confirm plus \
             --mcp-allow-destructive to mutate or --mcp-allow-sensitive-reads to read secrets)"
        );
        if let Err(e) = srelens_mcp::http::serve_http(server, addr, token).await {
            eprintln!("mcp http server error: {e}");
        }
    });
}

fn run_serve(addr: &str, data_flag: Option<&str>) {
    let addr: std::net::SocketAddr = addr.parse().expect("invalid serve address");
    let env_data = std::env::var("SRELENS_DATA").ok();
    let data_dir = srelens_server::config::resolve_data_dir(data_flag, env_data.as_deref());
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");
    runtime.block_on(async {
        let factory: srelens_server::RegistryFactory =
            Arc::new(|cache, paths| srelens_desktop_lib::build_registry_with_paths(cache, paths));
        eprintln!("srelens web server listening on http://{addr}");
        eprintln!("srelens data directory: {}", data_dir.display());
        if let Err(e) =
            srelens_server::serve(factory, srelens_server::ServerConfig { addr, data_dir }).await
        {
            eprintln!("web server error: {e}");
        }
    });
}

fn run_mcp_stdio(allow_destructive: bool, allow_sensitive_reads: bool) {
    let policy = Arc::new(srelens_mcp::policy::FlagGated::new(
        allow_destructive,
        allow_sensitive_reads,
    ));
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");
    runtime.block_on(async {
        // Same reasoning as `run_mcp_http`: build the cache explicitly and
        // share it between the registry and the watcher rather than letting
        // `build_registry` create one it doesn't expose.
        let cache = srelens_kube::client_cache::ClientCache::new_many(
            srelens_registry::default_kubeconfig_paths(),
        );
        let registry = srelens_desktop_lib::build_registry_with_paths(
            cache.clone(),
            srelens_registry::default_kubeconfig_paths(),
        );
        let server = srelens_mcp::McpServer::new(Arc::new(registry))
            .with_policy(policy)
            .with_audit(Arc::new(srelens_mcp::audit::JsonlAuditLog::new(
                mcp_audit_path(),
                MCP_AUDIT_CAP_BYTES,
            )))
            .with_prompts(srelens_mcp::prompts::PromptLibrary::new(Some(
                mcp_prompts_dir(),
            )))
            .with_kind_resolver(srelens_registry::kind_resolver())
            .with_watcher(Arc::new(srelens_desktop_lib::mcp_watch::CacheWatcher::new(
                cache,
            )));
        let reader = tokio::io::BufReader::new(tokio::io::stdin());
        let writer = tokio::io::stdout();
        if let Err(e) = srelens_mcp::stdio::serve(server, reader, writer).await {
            eprintln!("mcp stdio server error: {e}");
        }
    });
}
