//! MCP over HTTP (JSON-RPC POST). A networked transport for MCP clients that
//! can't spawn the stdio binary. Binds loopback-only; destructive tools are
//! consent-gated in the shared request handler.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::stdio::handle_request;
use crate::McpServer;

async fn rpc(State(server): State<Arc<McpServer>>, Json(req): Json<Value>) -> Json<Value> {
    match handle_request(&server, &req, crate::Transport::Http).await {
        Some(resp) => Json(resp),
        None => Json(json!({})), // notification — no response body
    }
}

/// Build the MCP HTTP router (POST /mcp for JSON-RPC, GET /healthz).
pub fn router(server: McpServer) -> Router {
    Router::new()
        .route("/mcp", post(rpc))
        .route("/healthz", get(|| async { "ok" }))
        .with_state(Arc::new(server))
}

/// Serve the MCP HTTP transport on `addr` (use a loopback address).
pub async fn serve_http(server: McpServer, addr: SocketAddr) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router(server)).await
}

/// Serve on an already-bound `listener` until `shutdown` resolves. Lets a host
/// (e.g. the desktop app) bind the port up front — surfacing bind errors
/// synchronously — then run the server with graceful shutdown so it can be
/// toggled off from Settings and the port released cleanly.
pub async fn serve_http_with_shutdown<F>(
    server: McpServer,
    listener: tokio::net::TcpListener,
    shutdown: F,
) -> std::io::Result<()>
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    axum::serve(listener, router(server))
        .with_graceful_shutdown(shutdown)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::McpServer;
    use srelens_capability::{Capability, Registry};
    use tower::ServiceExt; // oneshot

    fn server() -> McpServer {
        let mut reg = Registry::new();
        reg.register(Capability::read_only("ping", "health", |v| async move {
            Ok(json!({ "echo": v }))
        }));
        McpServer::new(Arc::new(reg))
    }

    #[tokio::test]
    async fn http_handles_tools_call() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};

        let app = router(server());
        let body = json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{"name":"ping","arguments":"hi"}});
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["result"]["isError"], false);
        assert!(v["result"]["content"][0]["text"].as_str().unwrap().contains("echo"));
    }
}
