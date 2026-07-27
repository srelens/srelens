//! Headless srelens web server — the container entry point. Wires the
//! Tauri-free capability registry into the axum server. No GUI, no Tauri.

use std::sync::Arc;

fn main() {
    // Honor SRELENS_TIMEOUT_SECS for kube request timeouts, like the desktop.
    srelens_kube::connect::init_timeout_from_env();

    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) != Some("serve") {
        eprintln!("usage: srelens-server serve [addr] [--data DIR]");
        std::process::exit(2);
    }

    // Parse `serve [addr] [--data DIR]` (same shape as the desktop `serve`).
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

    let addr: std::net::SocketAddr = addr
        .as_deref()
        .unwrap_or("0.0.0.0:8080")
        .parse()
        .expect("invalid serve address");
    let env_data = std::env::var("SRELENS_DATA").ok();
    let data_dir = srelens_server::config::resolve_data_dir(data.as_deref(), env_data.as_deref());

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");
    runtime.block_on(async {
        let factory: srelens_server::RegistryFactory =
            Arc::new(|cache, paths| srelens_registry::build_registry_with_paths(cache, paths));
        eprintln!("srelens web server listening on http://{addr}");
        eprintln!("srelens data directory: {}", data_dir.display());
        if let Err(e) =
            srelens_server::serve(factory, srelens_server::ServerConfig { addr, data_dir }).await
        {
            eprintln!("web server error: {e}");
            std::process::exit(1);
        }
    });
}
