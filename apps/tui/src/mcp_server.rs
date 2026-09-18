//! Headless Model Context Protocol (MCP) server for `srelens-tui`.
//!
//! Exposes all SRElens capabilities over standard input/output (the MCP stdio transport),
//! enabling external AI coding agents (Antigravity/AGY, Claude Code, Cursor, Codex) to
//! inspect and manage Kubernetes clusters using the TUI's configured kubeconfig paths.

use std::path::PathBuf;
use std::sync::Arc;

use srelens_kube::client_cache::ClientCache;
use srelens_mcp::McpServer;

/// Assemble an `McpServer` instance wired to the capability registry and gated by policy flags.
pub fn build_stdio_mcp_server(
    kubeconfig_paths: Vec<PathBuf>,
    allow_destructive: bool,
    allow_sensitive_reads: bool,
) -> McpServer {
    let cache = ClientCache::new_many(kubeconfig_paths.clone());
    let registry = srelens_registry::build_registry_with_paths(cache, kubeconfig_paths);
    let policy = Arc::new(srelens_mcp::policy::FlagGated::new(
        allow_destructive,
        allow_sensitive_reads,
    ));
    McpServer::new(Arc::new(registry))
        .with_policy(policy)
        .with_kind_resolver(srelens_registry::kind_resolver())
}

/// Serve newline-delimited JSON-RPC 2.0 requests over arbitrary async reader/writer streams.
pub async fn run_mcp_stdio<R, W>(server: McpServer, reader: R, writer: W) -> std::io::Result<()>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    srelens_mcp::stdio::serve(server, tokio::io::BufReader::new(reader), writer).await
}
