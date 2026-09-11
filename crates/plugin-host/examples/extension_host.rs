//! Developer-only manifest runner. It never loads extension JavaScript/binaries.
use srelens_capability::Registry;
use srelens_plugin_host::{Manifest, PluginHost};
use std::{error::Error, sync::Arc};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.as_slice() == ["--schema"] {
        println!("{}", serde_json::to_string_pretty(&Manifest::schema())?);
        return Ok(());
    }
    let path = args
        .first()
        .ok_or("usage: extension_host <manifest.json> --grant=<host-capability> [--mcp]")?;
    let source = std::fs::read_to_string(path)?;
    let manifest = Manifest::parse(&source)?;
    let grants: Vec<String> = args
        .iter()
        .skip(1)
        .filter_map(|a| a.strip_prefix("--grant=").map(str::to_owned))
        .collect();
    let core = Arc::new(srelens_registry::build_registry());
    let host = PluginHost::new(core);
    let mut registry = Registry::new();
    let registration = host.register(&mut registry, manifest, &grants)?;
    let server = srelens_mcp::McpServer::new(Arc::new(registry));
    if args.iter().any(|a| a == "--mcp") {
        // Default MCP policy denies mutations and sensitive reads.
        srelens_mcp::stdio::serve(
            server,
            tokio::io::BufReader::new(tokio::io::stdin()),
            tokio::io::stdout(),
        )
        .await?;
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "manifest": registration.manifest(),
                "tools": server.list_tools().iter().map(|t| &t.name).collect::<Vec<_>>()
            }))?
        );
    }
    Ok(())
}
