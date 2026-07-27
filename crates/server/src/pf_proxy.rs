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
    let mut connector = HttpConnector::new();
    connector.set_connect_timeout(Some(std::time::Duration::from_secs(10)));
    Client::builder(TokioExecutor::new()).build(connector)
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

/// Strip the `srelens_session` pair from a `cookie` header value before it is
/// forwarded to a forwarded pod. The pod is untrusted (it's whatever the
/// port-forward target happens to be) — forwarding the session cookie
/// verbatim would let a hostile/compromised upstream replay it against the
/// srelens server itself. Any other cookies the client sent are preserved.
/// Returns an empty string when nothing is left, so callers can omit the
/// header entirely rather than sending an empty `Cookie:`.
fn strip_session_cookie(value: &str) -> String {
    value
        .split(';')
        .map(|pair| pair.trim())
        .filter(|pair| {
            let name = pair.split_once('=').map(|(n, _)| n).unwrap_or(pair);
            name != session::COOKIE_NAME
        })
        .collect::<Vec<_>>()
        .join("; ")
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
///
/// Takes the whole `axum::extract::Request` (rather than separate
/// method/uri/headers/body extractors) because the WebSocket-upgrade path
/// needs `hyper::upgrade::on(&mut req)`, which requires the original request
/// value — reconstructing an equivalent `hyper::Request` from extracted parts
/// does not carry the connection's upgrade extension.
pub async fn proxy(
    State(state): State<AppState>,
    Path(pf): Path<PfPath>,
    mut req: axum::extract::Request,
) -> Response {
    let headers = req.headers().clone();
    let method = req.method().clone();
    let uri = req.uri().clone();

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

    let is_ws_upgrade = headers
        .get(axum::http::header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false);
    if is_ws_upgrade {
        return ws_passthrough(port, method, uri, headers, &mut req).await;
    }
    let body = req.into_body();

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
        if is_hop_by_hop(name) || name.as_str() == "host" {
            continue;
        }
        if name.as_str().eq_ignore_ascii_case("cookie") {
            if let Ok(v) = value.to_str() {
                let stripped = strip_session_cookie(v);
                if !stripped.is_empty() {
                    builder = builder.header(name, stripped);
                }
            }
            continue;
        }
        builder = builder.header(name, value);
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

use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Relay a WebSocket upgrade to the forwarded loopback port. We send the raw
/// upgrade request upstream, confirm the 101 response, then pipe bytes both
/// ways between the client's upgraded connection and the upstream TCP socket.
///
/// Takes `req: &mut axum::extract::Request` (rather than reconstructing a
/// `hyper::Request` from separate parts) so `hyper::upgrade::on` sees the
/// original request value — that's what carries the connection's upgrade
/// extension set by the server that accepted this request.
async fn ws_passthrough(
    port: u16,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    req: &mut axum::extract::Request,
) -> Response {
    // Only GET can be a WS handshake.
    if method != Method::GET {
        return StatusCode::BAD_REQUEST.into_response();
    }
    // Connect to the upstream and send the handshake request line + headers,
    // rewriting the path to strip /pf/<id>.
    let path_and_query = uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/");
    let stripped = strip_pf_prefix(path_and_query);

    let mut handshake = format!("GET {stripped} HTTP/1.1\r\n");
    for (name, value) in headers.iter() {
        if name.as_str() == "host" {
            continue;
        }
        let Ok(v) = value.to_str() else { continue };
        if name.as_str().eq_ignore_ascii_case("cookie") {
            let stripped_cookie = strip_session_cookie(v);
            if !stripped_cookie.is_empty() {
                handshake.push_str(&format!("{}: {}\r\n", name.as_str(), stripped_cookie));
            }
            continue;
        }
        handshake.push_str(&format!("{}: {}\r\n", name.as_str(), v));
    }
    handshake.push_str(&format!("host: 127.0.0.1:{port}\r\n\r\n"));

    let (mut upstream, head) = match upstream_ws_handshake(
        port,
        handshake.as_bytes(),
        std::time::Duration::from_secs(10),
    )
    .await
    {
        Ok(pair) => pair,
        Err(status) => return status.into_response(),
    };

    let head_str = String::from_utf8_lossy(&head);
    if !head_str.starts_with("HTTP/1.1 101") {
        return StatusCode::BAD_GATEWAY.into_response();
    }

    // Build the 101 response back to the client, echoing the upstream's WS
    // accept headers, and take over the client connection on upgrade.
    let mut client_resp = Response::builder().status(StatusCode::SWITCHING_PROTOCOLS);
    for line in head_str.lines().skip(1) {
        if let Some((name, value)) = line.split_once(':') {
            let (name, value) = (name.trim(), value.trim());
            if !name.is_empty() && !value.is_empty() {
                client_resp = client_resp.header(name, value);
            }
        }
    }

    let on_upgrade = hyper::upgrade::on(req);

    tokio::spawn(async move {
        if let Ok(upgraded) = on_upgrade.await {
            let mut client_io = hyper_util::rt::TokioIo::new(upgraded);
            let _ = tokio::io::copy_bidirectional(&mut client_io, &mut upstream).await;
        }
    });

    match client_resp.body(Body::empty()) {
        Ok(r) => r,
        Err(_) => StatusCode::BAD_GATEWAY.into_response(),
    }
}

/// Connect to a forwarded loopback port, write the raw WS handshake request,
/// and read the upstream response head (until CRLFCRLF), all bounded by
/// `timeout`. Returns the connected socket plus the raw head bytes so the
/// caller can decide whether the response is actually a 101 — this fn only
/// owns connect/write/read-until-blank-line and timeout enforcement, not
/// status validation, so it stays trivially testable against any upstream
/// response (101, 401, or none at all).
async fn upstream_ws_handshake(
    port: u16,
    req_bytes: &[u8],
    timeout: std::time::Duration,
) -> Result<(tokio::net::TcpStream, Vec<u8>), StatusCode> {
    tokio::time::timeout(timeout, async move {
        let mut upstream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .map_err(|_| StatusCode::BAD_GATEWAY)?;

        upstream
            .write_all(req_bytes)
            .await
            .map_err(|_| StatusCode::BAD_GATEWAY)?;

        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            match upstream.read(&mut byte).await {
                Ok(0) => return Err(StatusCode::BAD_GATEWAY),
                Ok(_) => {
                    head.push(byte[0]);
                    if head.ends_with(b"\r\n\r\n") {
                        break;
                    }
                    if head.len() > 16 * 1024 {
                        return Err(StatusCode::BAD_GATEWAY);
                    }
                }
                Err(_) => return Err(StatusCode::BAD_GATEWAY),
            }
        }
        Ok((upstream, head))
    })
    .await
    .unwrap_or(Err(StatusCode::GATEWAY_TIMEOUT))
}

/// Strip the leading `/pf/<id>` (two path segments) from a path+query,
/// returning the remainder starting with `/`.
fn strip_pf_prefix(path_and_query: &str) -> String {
    // e.g. "/pf/3/socket?x=1" -> "/socket?x=1"; "/pf/3" -> "/"
    let mut segs = path_and_query.splitn(4, '/'); // ["", "pf", "<id>", "rest?query"]
    segs.next(); // leading ""
    segs.next(); // "pf"
    segs.next(); // "<id>"
    match segs.next() {
        Some(rest) if !rest.is_empty() => format!("/{rest}"),
        _ => "/".to_string(),
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

    #[tokio::test(flavor = "multi_thread")]
    async fn upstream_ws_handshake_times_out_on_black_hole_port() {
        // Bound but never accept: the connect completes (loopback backlog),
        // but nothing ever reads the request or writes a response head, so
        // the read-until-CRLFCRLF loop must hit the timeout, not hang forever.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        // Keep the listener alive for the duration of the test but never call
        // accept() on it.
        let _listener = listener;

        let start = std::time::Instant::now();
        let result = super::upstream_ws_handshake(
            port,
            b"GET / HTTP/1.1\r\n\r\n",
            std::time::Duration::from_millis(200),
        )
        .await;
        let elapsed = start.elapsed();

        assert_eq!(result.err(), Some(StatusCode::GATEWAY_TIMEOUT));
        assert!(elapsed < std::time::Duration::from_secs(5), "took {elapsed:?}");
    }

    #[tokio::test]
    async fn upstream_ws_handshake_connect_refused_is_bad_gateway() {
        // No listener bound on this port: connect fails fast (not the
        // timeout path), and the error must map to a gateway status rather
        // than panicking or hanging.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener); // free the port so nothing is listening on it

        let result = super::upstream_ws_handshake(
            port,
            b"GET / HTTP/1.1\r\n\r\n",
            std::time::Duration::from_secs(10),
        )
        .await;
        assert_eq!(result.err(), Some(StatusCode::BAD_GATEWAY));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn upstream_ws_handshake_returns_head_for_101_response() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                let _ = sock.read(&mut buf).await;
                let _ = sock
                    .write_all(b"HTTP/1.1 101 Switching Protocols\r\nupgrade: websocket\r\n\r\n")
                    .await;
            }
        });

        let result = super::upstream_ws_handshake(
            port,
            b"GET / HTTP/1.1\r\n\r\n",
            std::time::Duration::from_secs(10),
        )
        .await;
        let (_stream, head) = result.expect("expected Ok for a 101-responding upstream");
        assert!(String::from_utf8_lossy(&head).starts_with("HTTP/1.1 101"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn upstream_ws_handshake_returns_head_for_non_101_response() {
        // A non-101 (e.g. 401) response head is still returned as `Ok` — this
        // fn only owns connect/write/read-head/timeout; validating the status
        // line is the caller's job (ws_passthrough maps non-101 to 502).
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                let _ = sock.read(&mut buf).await;
                let _ = sock
                    .write_all(b"HTTP/1.1 401 Unauthorized\r\ncontent-length: 0\r\n\r\n")
                    .await;
            }
        });

        let result = super::upstream_ws_handshake(
            port,
            b"GET / HTTP/1.1\r\n\r\n",
            std::time::Duration::from_secs(10),
        )
        .await;
        let (_stream, head) = result.expect("expected Ok even for a 401 response");
        assert!(String::from_utf8_lossy(&head).starts_with("HTTP/1.1 401"));
    }

    #[test]
    fn strip_pf_prefix_removes_two_segments() {
        assert_eq!(super::strip_pf_prefix("/pf/3/socket?x=1"), "/socket?x=1");
        assert_eq!(super::strip_pf_prefix("/pf/3/"), "/");
        assert_eq!(super::strip_pf_prefix("/pf/3"), "/");
        assert_eq!(super::strip_pf_prefix("/pf/12/a/b/c"), "/a/b/c");
    }

    #[test]
    fn strip_session_cookie_drops_only_the_session_pair() {
        assert_eq!(
            super::strip_session_cookie("a=1; srelens_session=tok; b=2"),
            "a=1; b=2"
        );
        assert_eq!(super::strip_session_cookie("srelens_session=tok"), "");
        assert_eq!(super::strip_session_cookie("x=1"), "x=1");
    }
}
