//! Developer-only manifest runner. It never loads extension JavaScript/binaries.
use srelens_capability::Registry;
use srelens_plugin_host::{Manifest, PluginHost};
use std::{error::Error, process::ExitCode, sync::Arc};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<ExitCode, Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.as_slice() == ["--schema"] {
        println!("{}", serde_json::to_string_pretty(&Manifest::schema())?);
        return Ok(ExitCode::SUCCESS);
    }
    let path = args
        .first()
        .ok_or("usage: extension_host <manifest.json> --grant=<host-capability> [--mcp]")?;
    let source = std::fs::read_to_string(path)?;
    // Every problem, one per line as `path: message (CODE)`, so an author can fix them in
    // one pass.
    let manifest = match Manifest::parse(&source) {
        Ok(manifest) => manifest,
        Err(errors) => {
            eprintln!("{errors}");
            return Ok(ExitCode::FAILURE);
        }
    };
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
    Ok(ExitCode::SUCCESS)
}
