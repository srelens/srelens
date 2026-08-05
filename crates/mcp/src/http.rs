//! MCP over HTTP (JSON-RPC POST). A networked transport for MCP clients that
//! can't spawn the stdio binary. Binds loopback-only; destructive tools are
//! consent-gated in the shared request handler.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
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

/// True for `127.0.0.1[:port]`, `[::1]`, `[::1]:port`, bare `::1`, and
/// `localhost[:port]` (case-insensitively). Host header hostnames are
/// case-insensitive per HTTP semantics, and IPv6 needs care: a bracketed
/// address may carry a `:port` suffix outside the brackets, but the colons
/// *inside* the brackets are part of the address, not port separators, and
/// an unbracketed literal like `::1` has no unambiguous port syntax at all.
fn host_is_loopback(host: &str) -> bool {
    let h: &str = if let Some(rest) = host.strip_prefix('[') {
        // Bracketed IPv6, with or without a trailing `:port`. Split on `]`,
        // never on `:`, so the address's own colons are never touched. The
        // bracket must actually close, and whatever follows the `]` must be
        // either nothing or a numeric `:port` — anything else (a bare
        // trailing hostname, a stray `.evil.com`, a non-numeric "port") is
        // not a valid authority and must reject, not fall through as if the
        // bracketed address were the whole story.
        let Some((inside, tail)) = rest.split_once(']') else {
            return false;
        };
        let tail_ok = tail.is_empty()
            || tail
                .strip_prefix(':')
                .map(|port| !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()))
                .unwrap_or(false);
        if !tail_ok {
            return false;
        }
        inside
    } else if host.matches(':').count() > 1 {
        // Unbracketed IPv6 (e.g. bare "::1"): more than one colon means
        // there's no unambiguous port suffix, so treat it all as the host.
        host
    } else {
        // "host:port" or a bare host — strip a numeric trailing port only.
        match host.rsplit_once(':') {
            Some((h, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => h,
            _ => host,
        }
    };
    h.eq_ignore_ascii_case("127.0.0.1") || h == "::1" || h.eq_ignore_ascii_case("localhost")
}

/// Strip a leading `Bearer ` auth scheme from an `Authorization` header value.
/// RFC 7235 §2.1 makes the scheme name case-insensitive ("bearer", "BEARER",
/// "Bearer" all name the same scheme) — only the scheme, not the credentials
/// that follow, so this does a case-insensitive match on `"Bearer"` alone and
/// leaves the token half untouched for `Token::matches`'s constant-time
/// comparison.
fn strip_bearer_prefix(header: &str) -> Option<&str> {
    let (scheme, rest) = header.split_once(' ')?;
    scheme.eq_ignore_ascii_case("Bearer").then_some(rest)
}

/// Applied to EVERY route, including `/healthz`. DNS rebinding: a page on
/// evil.com can resolve to 127.0.0.1 and reach this port. Binding loopback does
/// not stop it; checking Host does. An unauthenticated route is still a signal
/// — "something is listening here" — so the Host check has to cover it too.
async fn host_guard(req: Request, next: Next) -> Response {
    let host_ok = req
        .headers()
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(host_is_loopback)
        .unwrap_or(false);
    if !host_ok {
        return (StatusCode::FORBIDDEN, "non-loopback Host rejected").into_response();
    }
    next.run(req).await
}

/// Applied to `/mcp` only: `/healthz` is deliberately reachable without a
/// token so a client can probe liveness before it has been configured.
async fn token_guard(
    State(token): State<Option<crate::auth::Token>>,
    req: Request,
    next: Next,
) -> Response {
    if let Some(expected) = token {
        let presented = req
            .headers()
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(strip_bearer_prefix)
            .unwrap_or("");
        if !expected.matches(presented) {
            // No detail in the body: do not reveal whether a token is set.
            return (
                StatusCode::UNAUTHORIZED,
                [(axum::http::header::WWW_AUTHENTICATE, "Bearer")],
                "unauthorized",
            )
                .into_response();
        }
    }
    next.run(req).await
}

/// Build the MCP HTTP router (POST /mcp for JSON-RPC, GET /healthz). Requires a
/// token: there is no way to ask this crate for an unauthenticated production
/// server, because an `Option` here is an invitation to pass `None` by accident.
pub fn router_with_auth(server: McpServer, token: crate::auth::Token) -> Router {
    router_inner(server, Some(token))
}

/// Layering: the token check is a `route_layer` on `/mcp` alone, while the Host
/// check is an outer `layer` covering every route (and unmatched paths), so
/// nothing this server exposes answers a non-loopback caller.
fn router_inner(server: McpServer, token: Option<crate::auth::Token>) -> Router {
    Router::new()
        .route("/mcp", post(rpc))
        .route_layer(middleware::from_fn_with_state(token, token_guard))
        .route("/healthz", get(|| async { "ok" }))
        .layer(middleware::from_fn(host_guard))
        .with_state(Arc::new(server))
}

/// Test-only convenience: a router with authentication disabled. Never
/// available to production code — a public no-auth constructor is exactly
/// the kind of thing that gets called by accident later.
#[cfg(test)]
pub(crate) fn router(server: McpServer) -> Router {
    router_inner(server, None)
}

/// Serve the MCP HTTP transport on `addr` (use a loopback address).
pub async fn serve_http(
    server: McpServer,
    addr: SocketAddr,
    token: crate::auth::Token,
) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router_with_auth(server, token)).await
}

/// Serve on an already-bound `listener` until `shutdown` resolves. Lets a host
/// (e.g. the desktop app) bind the port up front — surfacing bind errors
/// synchronously — then run the server with graceful shutdown so it can be
/// toggled off from Settings and the port released cleanly.
pub async fn serve_http_with_shutdown<F>(
    server: McpServer,
    listener: tokio::net::TcpListener,
    shutdown: F,
    token: crate::auth::Token,
) -> std::io::Result<()>
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    axum::serve(listener, router_with_auth(server, token))
        .with_graceful_shutdown(shutdown)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::McpServer;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use srelens_capability::{Capability, Registry};
    use tower::ServiceExt; // oneshot

    fn test_server() -> McpServer {
        let mut reg = Registry::new();
        reg.register(Capability::read_only("ping", "health", |v| async move {
            Ok(json!({ "echo": v }))
        }));
        McpServer::new(Arc::new(reg))
    }

    #[tokio::test]
    async fn http_handles_tools_call() {
        let app = router(test_server());
        let body = json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{"name":"ping","arguments":"hi"}});
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header("host", "127.0.0.1:8765")
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

    fn call_body() -> Body {
        Body::from(
            serde_json::to_vec(&json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/list"
            }))
            .unwrap(),
        )
    }

    #[tokio::test]
    async fn rejects_a_request_with_no_token() {
        let token = crate::auth::Token::generate();
        let app = router_with_auth(test_server(), token);
        let resp = app
            .oneshot(
                Request::post("/mcp")
                    .header("host", "127.0.0.1:8765")
                    .header("content-type", "application/json")
                    .body(call_body())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        assert!(resp.headers().get("www-authenticate").is_some());
    }

    #[tokio::test]
    async fn rejects_a_wrong_token() {
        let app = router_with_auth(test_server(), crate::auth::Token::generate());
        let resp = app
            .oneshot(
                Request::post("/mcp")
                    .header("host", "127.0.0.1:8765")
                    .header("authorization", format!("Bearer {}", "a".repeat(64)))
                    .header("content-type", "application/json")
                    .body(call_body())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    /// RFC 7235 §2.1: the auth scheme name is case-insensitive, so a client
    /// sending `bearer <token>` (lowercase) must still authenticate — not get
    /// a 401 that reads as a wrong token.
    #[tokio::test]
    async fn accepts_a_lowercase_bearer_scheme() {
        let token = crate::auth::Token::generate();
        let app = router_with_auth(test_server(), token.clone());
        let resp = app
            .oneshot(
                Request::post("/mcp")
                    .header("host", "127.0.0.1:8765")
                    .header("authorization", format!("bearer {}", token.as_str()))
                    .header("content-type", "application/json")
                    .body(call_body())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn accepts_the_right_token() {
        let token = crate::auth::Token::generate();
        let app = router_with_auth(test_server(), token.clone());
        let resp = app
            .oneshot(
                Request::post("/mcp")
                    .header("host", "127.0.0.1:8765")
                    .header("authorization", format!("Bearer {}", token.as_str()))
                    .header("content-type", "application/json")
                    .body(call_body())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    /// Loopback binding does NOT stop a page on evil.com that resolves to
    /// 127.0.0.1 from posting here. A Host check does.
    #[tokio::test]
    async fn rejects_a_non_loopback_host_header() {
        let token = crate::auth::Token::generate();
        let app = router_with_auth(test_server(), token.clone());
        let resp = app
            .oneshot(
                Request::post("/mcp")
                    .header("host", "evil.com")
                    .header("authorization", format!("Bearer {}", token.as_str()))
                    .header("content-type", "application/json")
                    .body(call_body())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn healthz_needs_no_token() {
        let app = router_with_auth(test_server(), crate::auth::Token::generate());
        let resp = app
            .oneshot(
                Request::get("/healthz")
                    .header("host", "127.0.0.1:8765")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    /// `/healthz` deliberately needs no token, but that must not make it a
    /// free "is srelens listening on this port?" oracle for any process or web
    /// page sweeping loopback ports. The Host check is what makes the answer
    /// unavailable to a caller that isn't genuinely local, so it has to cover
    /// every route, not just `/mcp`.
    #[tokio::test]
    async fn healthz_rejects_a_non_loopback_host() {
        let app = router_with_auth(test_server(), crate::auth::Token::generate());
        let resp = app
            .oneshot(
                Request::get("/healthz")
                    .header("host", "evil.com")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn host_is_loopback_accepts_loopback_forms() {
        for host in [
            "[::1]",
            "[::1]:8765",
            "LOCALHOST",
            "LocalHost:8765",
            "127.0.0.1",
            "127.0.0.1:8765",
            "::1",
        ] {
            assert!(host_is_loopback(host), "expected {host:?} to be accepted");
        }
    }

    #[test]
    fn host_is_loopback_rejects_non_loopback_forms() {
        for host in [
            "evil.com",
            "evil.com:80",
            "127.0.0.1.evil.com",
            "localhost.evil.com",
            "",
            // A closed bracket doesn't end the authority: anything after
            // it other than a numeric `:port` must still reject, or a
            // spoofed Host slips through as if `[::1]` were the whole
            // story.
            "[::1]evil.com",
            "[::1].evil.com",
            "[::1]:notaport",
            "[::1]:",
        ] {
            assert!(!host_is_loopback(host), "expected {host:?} to be rejected");
        }
    }

    #[tokio::test]
    async fn rejects_a_missing_host_header() {
        let token = crate::auth::Token::generate();
        let app = router_with_auth(test_server(), token.clone());
        let resp = app
            .oneshot(
                Request::post("/mcp")
                    .header("authorization", format!("Bearer {}", token.as_str()))
                    .header("content-type", "application/json")
                    .body(call_body())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }
}
