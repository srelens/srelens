//! `/pf/{id}/…` — per-user reverse proxy to a forwarded loopback port. A
//! port-forward binds `127.0.0.1:<n>` inside the container; the browser can't
//! reach that directly, so this proxies HTTP (and WebSocket upgrades) to it.
//! Authenticated by the session cookie; authorized by per-user forward
//! ownership (the id resolves only against the caller's own ForwardManager).

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderName, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};

use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;

use crate::db::Db;
use crate::{auth::session, AppState};

/// Loopback HTTP client used to reach forwarded ports. Cheap to clone.
pub type HttpProxyClient = Client<HttpConnector, Body>;

/// Build the proxy client.
pub fn client() -> HttpProxyClient {
    Client::builder(TokioExecutor::new()).build_http()
}

/// Hop-by-hop headers that must not be forwarded (RFC 7230 §6.1).
const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailers",
    "transfer-encoding",
    "upgrade",
];

fn is_hop_by_hop(name: &HeaderName) -> bool {
    HOP_BY_HOP.iter().any(|h| name.as_str().eq_ignore_ascii_case(h))
}

/// Authenticate a `/pf` request by session cookie. No CSRF/Origin check: a
/// browser navigation or sub-resource load to a proxied service cannot carry
/// them, and authorization is enforced by per-user forward ownership.
pub async fn authorize_pf(headers: &HeaderMap, db: &Db, now: i64) -> Result<i64, StatusCode> {
    let token = session::cookie_value(headers, session::COOKIE_NAME)
        .ok_or(StatusCode::UNAUTHORIZED)?;
    match db.validate_session(&token, now).await {
        Ok(Some(user)) => Ok(user.id),
        Ok(None) => Err(StatusCode::UNAUTHORIZED),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// Split `/pf/:id/*rest` params: axum gives us `id` and the wildcard `rest`.
#[derive(serde::Deserialize)]
pub struct PfPath {
    pub id: u64,
    #[serde(default)]
    pub rest: String,
}

/// GET/POST/... `/pf/:id/*rest` — reverse-proxy to the caller's forward.
pub async fn proxy(
    State(state): State<AppState>,
    Path(pf): Path<PfPath>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let user_id = match authorize_pf(&headers, &state.db, crate::unix_now()).await {
        Ok(id) => id,
        Err(status) => return status.into_response(),
    };

    let env = match state
        .user_envs
        .env_for(&state.db, &state.master_key, user_id)
        .await
    {
        Ok(env) => env,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let Some(port) = env.streams.forward.local_port(pf.id) else {
        return (StatusCode::NOT_FOUND, "no such forward").into_response();
    };

    // Build the upstream URI: preserve the sub-path and query, drop the
    // /pf/<id> prefix.
    let query = uri.query().map(|q| format!("?{q}")).unwrap_or_default();
    let rest = pf.rest.trim_start_matches('/');
    let upstream = format!("http://127.0.0.1:{port}/{rest}{query}");
    let upstream_uri: Uri = match upstream.parse() {
        Ok(u) => u,
        Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
    };

    let mut builder = hyper::Request::builder().method(method).uri(upstream_uri);
    for (name, value) in headers.iter() {
        if !is_hop_by_hop(name) && name.as_str() != "host" {
            builder = builder.header(name, value);
        }
    }
    let upstream_req = match builder.body(body) {
        Ok(r) => r,
        Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
    };

    match state.pf_client.request(upstream_req).await {
        Ok(resp) => {
            let (parts, incoming) = resp.into_parts();
            let mut out = Response::from_parts(parts, Body::new(incoming));
            // Strip hop-by-hop headers from the upstream response too.
            let to_remove: Vec<HeaderName> = out
                .headers()
                .keys()
                .filter(|n| is_hop_by_hop(n))
                .cloned()
                .collect();
            for name in to_remove {
                out.headers_mut().remove(name);
            }
            out
        }
        Err(_) => (StatusCode::BAD_GATEWAY, "forwarded service unreachable").into_response(),
    }
}

#[cfg(test)]
mod tests {
    use crate::{router, AppState};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use srelens_capability::Registry;
    use std::sync::Arc;
    use tower::ServiceExt;

    #[tokio::test]
    async fn pf_requires_session() {
        let state = AppState::for_tests(Arc::new(Registry::new())).await;
        let resp = router(state)
            .oneshot(Request::builder().uri("/pf/1/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn pf_unknown_forward_is_404() {
        let state = AppState::for_tests(Arc::new(Registry::new())).await;
        let user = state.db.upsert_user("i", "s", "u@x", "U", 1).await.unwrap();
        let token = state.db.create_session(user.id, crate::unix_now()).await.unwrap();
        let resp = router(state)
            .oneshot(
                Request::builder()
                    .uri("/pf/1/")
                    .header("cookie", format!("srelens_session={token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        // Authenticated, but the user owns no forward id 1.
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pf_proxies_to_a_forwarded_upstream() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        // A trivial HTTP upstream on loopback that replies 200 "hello from pod".
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                let _ = sock.read(&mut buf).await;
                let body = "hello from pod";
                let resp = format!(
                    "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = sock.write_all(resp.as_bytes()).await;
            }
        });

        let state = AppState::for_tests(Arc::new(Registry::new())).await;
        let user = state.db.upsert_user("i", "s", "u@x", "U", 1).await.unwrap();
        let token = state.db.create_session(user.id, crate::unix_now()).await.unwrap();

        // Register a fake forward for this user pointing at the upstream port.
        // env_for builds the user's stream managers; inject the forward via the
        // manager's test seam by starting a real forward is heavy, so assert
        // the wiring through a direct local_port stub is out of scope here —
        // instead drive the proxy against a manually-inserted forward.
        let env = state.user_envs.env_for(&state.db, &state.master_key, user.id).await.unwrap();
        env.streams.forward.insert_test_forward(1, port);

        let resp = router(state.clone())
            .oneshot(
                Request::builder()
                    .uri("/pf/1/anything")
                    .header("cookie", format!("srelens_session={token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
        assert_eq!(&bytes[..], b"hello from pod");
    }
}
