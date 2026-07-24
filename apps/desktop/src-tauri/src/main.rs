// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;

fn main() {
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
        run_serve(addr.as_deref().unwrap_or("127.0.0.1:8080"), data.as_deref());
        return;
    }
    // `--mcp-stdio` / `--mcp-http [addr]` run the MCP server instead of the GUI,
    // so external MCP clients/agents can drive every capability.
    if args.iter().any(|a| a == "--mcp-stdio") {
        run_mcp_stdio();
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--mcp-http") {
        let addr = args.get(i + 1).cloned().unwrap_or_else(|| "127.0.0.1:8765".into());
        run_mcp_http(&addr);
        return;
    }
    srelens_desktop_lib::run();
}

fn run_mcp_http(addr: &str) {
    let addr: std::net::SocketAddr = addr.parse().expect("invalid --mcp-http address");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");
    runtime.block_on(async {
        let registry = srelens_desktop_lib::build_registry();
        let server = srelens_mcp::McpServer::new(Arc::new(registry));
        eprintln!("MCP HTTP listening on http://{addr}/mcp (loopback; destructive tools need _confirm)");
        if let Err(e) = srelens_mcp::http::serve_http(server, addr).await {
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

fn run_mcp_stdio() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");
    runtime.block_on(async {
        let registry = srelens_desktop_lib::build_registry();
        let server = srelens_mcp::McpServer::new(Arc::new(registry));
        let reader = tokio::io::BufReader::new(tokio::io::stdin());
        let writer = tokio::io::stdout();
        if let Err(e) = srelens_mcp::stdio::serve(server, reader, writer).await {
            eprintln!("mcp stdio server error: {e}");
        }
    });
}
