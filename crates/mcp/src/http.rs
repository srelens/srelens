//! MCP over HTTP, Streamable-HTTP shaped (#193): `POST /mcp` for JSON-RPC,
//! `GET /mcp` for the server→client SSE stream that carries
//! `notifications/resources/updated` — the channel that makes
//! `resources/subscribe` real on this transport. A networked transport for
//! MCP clients that can't spawn the stdio binary. Binds loopback-only;
//! destructive tools are consent-gated in the shared request handler.

use std::collections::{BTreeSet, VecDeque};
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::task::{Context, Poll};

use axum::extract::{Request, State};
use axum::http::{HeaderName, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::stdio::{handle_request, handle_subscription, subscription_notification};
use crate::subscriptions::SubscriptionRegistry;
use crate::McpServer;

/// The push half of the transport. At most ONE live SSE stream exists per
/// server — this is a loopback, single-user endpoint, and one stream is what
/// the Streamable HTTP shape needs (the client opens GET /mcp once and every
/// notification flows down it). A new GET replaces the previous stream: the
/// old stream's wake sender drops, its poll sees the closed channel and ends
/// the stream, and its guard aborts the watches it owned — so a reconnecting
/// client starts clean and re-subscribes, and no watch ever outlives the
/// stream that requested it. Subscriptions arriving while NO stream is
/// connected are refused (see `rpc`): accepting one would promise
/// notifications with nowhere to send them, the exact failure shape #195
/// eliminated.
#[derive(Default)]
struct PushState {
    active: Mutex<Option<PushChannels>>,
    /// Distinguishes streams so a dying stream's guard can only clear ITS
    /// slot, never a successor's (same shape as the subscription registry's
    /// generations).
    next_id: AtomicU64,
    /// Set by `end_streams` at graceful shutdown. Checked under the `active`
    /// lock on install, so a GET that was already accepted when shutdown
    /// began cannot slip a fresh stream in after the teardown and stall the
    /// shutdown all over again.
    closed: std::sync::atomic::AtomicBool,
}

impl PushState {
    /// Install `chans` as THE stream, unless the server is shutting down.
    /// Returns whether it was installed. Dropping a previous occupant here is
    /// what ends a replaced stream: its wake sender closes, its poll sees the
    /// closed channel, and its guard aborts the watches it owned.
    fn install(&self, chans: PushChannels) -> bool {
        let mut active = self.active.lock().unwrap_or_else(|e| e.into_inner());
        if self.closed.load(Ordering::Relaxed) {
            return false;
        }
        *active = Some(chans);
        true
    }

    /// Graceful-shutdown teardown (#193 review): drop the active stream's
    /// channels so its SSE body ENDS. Without this the state and the stream
    /// keep each other alive — the guard holds this state, the state's slot
    /// holds the wake sender the stream waits on — and axum's graceful
    /// shutdown waits on that body forever; the desktop only escaped by
    /// timing out and aborting the whole server task, delaying stop and
    /// token rotation by its two-second fallback.
    fn end_streams(&self) {
        let mut active = self.active.lock().unwrap_or_else(|e| e.into_inner());
        self.closed.store(true, Ordering::Relaxed);
        active.take();
    }
}

/// What `resources/subscribe` needs from the active stream: the same
/// registry/dirty-set/wakeup trio the stdio serve loop owns per session.
#[derive(Clone)]
struct PushChannels {
    id: u64,
    subs: Arc<SubscriptionRegistry>,
    dirty: Arc<Mutex<BTreeSet<String>>>,
    wake: tokio::sync::mpsc::Sender<()>,
}

#[derive(Clone)]
struct AppState {
    server: Arc<McpServer>,
    push: Arc<PushState>,
}

/// Aborts the stream's watches when the stream itself is dropped — client
/// disconnect, replacement by a newer stream, or server shutdown all land
/// here, so the "no watch outlives its stream" contract has one enforcement
/// point. Clears the active slot only when it still holds this stream.
struct StreamGuard {
    push: Arc<PushState>,
    subs: Arc<SubscriptionRegistry>,
    id: u64,
}

impl Drop for StreamGuard {
    fn drop(&mut self) {
        self.subs.abort_all();
        let mut active = self.push.active.lock().unwrap_or_else(|e| e.into_inner());
        if active.as_ref().is_some_and(|a| a.id == self.id) {
            *active = None;
        }
    }
}

/// The SSE body: wakes on the subscription machinery's wakeup channel, drains
/// the dirty set, and yields one `notifications/resources/updated` per URI —
/// each SSE event carrying exactly one JSON-RPC message, per Streamable HTTP.
/// Ends when the wake sender is dropped (this stream was replaced). Implements
/// `Stream` by hand (`futures-core` is the only extra dependency, and it is
/// already in the tree under axum) rather than pulling in a stream-combinator
/// crate for one loop.
struct PushStream {
    wake: tokio::sync::mpsc::Receiver<()>,
    dirty: Arc<Mutex<BTreeSet<String>>>,
    pending: VecDeque<String>,
    _guard: StreamGuard,
}

impl futures_core::Stream for PushStream {
    type Item = Result<Event, std::convert::Infallible>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            if let Some(uri) = this.pending.pop_front() {
                let msg = subscription_notification(&uri);
                return Poll::Ready(Some(Ok(Event::default().data(msg.to_string()))));
            }
            match this.wake.poll_recv(cx) {
                Poll::Ready(Some(())) => {
                    let uris = {
                        let mut guard = this.dirty.lock().unwrap_or_else(|e| e.into_inner());
                        std::mem::take(&mut *guard)
                    };
                    // Loop: emit the first drained URI, or wait again on a
                    // spurious wake that raced an earlier drain.
                    this.pending.extend(uris);
                }
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

/// `GET /mcp`: open the server→client SSE stream. Replaces any previous
/// stream (see `PushState`). The keep-alive comment every 20s holds idle
/// connections open through proxies and lets a dead client surface as a
/// write error instead of a silent zombie.
async fn sse(State(st): State<AppState>, headers: axum::http::HeaderMap) -> Response {
    // The spec has clients send `Accept: text/event-stream`; a GET that
    // explicitly refuses it gets 406 rather than a stream it won't parse.
    // An absent Accept header is allowed (curl convenience).
    let accept = headers
        .get(axum::http::header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !accept.is_empty() && !accept.contains("text/event-stream") && !accept.contains("*/*") {
        return (StatusCode::NOT_ACCEPTABLE, "this endpoint streams text/event-stream").into_response();
    }

    let (wake_tx, wake_rx) = tokio::sync::mpsc::channel(1);
    let chans = PushChannels {
        id: st.push.next_id.fetch_add(1, Ordering::Relaxed),
        subs: Arc::new(SubscriptionRegistry::new()),
        dirty: Arc::default(),
        wake: wake_tx,
    };
    let stream = PushStream {
        wake: wake_rx,
        dirty: chans.dirty.clone(),
        pending: VecDeque::new(),
        _guard: StreamGuard { push: st.push.clone(), subs: chans.subs.clone(), id: chans.id },
    };
    // Install as THE stream (see `PushState::install` for replacement and
    // shutdown semantics). Refusal means graceful shutdown already began on
    // this in-flight connection's watch — a fresh stream now would stall it.
    if !st.push.install(chans) {
        return (StatusCode::SERVICE_UNAVAILABLE, "the server is shutting down").into_response();
    }

    (
        [(MCP_SESSION_ID.clone(), session_id())],
        Sse::new(stream).keep_alive(
            KeepAlive::new().interval(std::time::Duration::from_secs(20)).text("keep-alive"),
        ),
    )
        .into_response()
}

/// The `Mcp-Session-Id` header name, surfaced on every `/mcp` response.
static MCP_SESSION_ID: HeaderName = HeaderName::from_static("mcp-session-id");

/// A stable per-process MCP session id. Streamable-HTTP clients (e.g. Codex's
/// `streamable_http` transport) establish a session on `initialize` and refuse
/// to load tools unless the response carries this header — a minimal
/// POST/JSON-RPC endpoint without it hangs their handshake (verified: Codex
/// retries and reports "no tools"). srelens is stateless — the bearer token,
/// not this id, is what authorizes each request — so the value only has to be
/// present and stable across a server's lifetime, never validated on later
/// requests. Claude Code's more lenient HTTP client works with or without it,
/// so adding it is purely additive.
fn session_id() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| format!("srelens-{}", std::process::id()))
}

async fn rpc(State(st): State<AppState>, Json(req): Json<Value>) -> Response {
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
    // Subscription methods are handled here, not in `handle_request` — same
    // split as the stdio serve loop, because they need this transport's push
    // machinery: the registry, dirty set, and wakeup of the ACTIVE SSE
    // stream. With no stream connected they are refused outright; accepting
    // would promise notifications with nowhere to send them.
    let resp = if method == "resources/subscribe" || method == "resources/unsubscribe" {
        // The active-slot lock is held across `handle_subscription`, which is
        // entirely synchronous (no await), so registration is SERIALIZED with
        // stream replacement — `PushState::install` takes this same lock. The
        // race this closes: a reconnecting GET replacing the stream between a
        // clone of the channels and the registration completing would leave
        // the watch in the old registry (whose guard then aborts it) while
        // the client holds a success response for a subscription the new
        // stream will never deliver.
        let active = st.push.active.lock().unwrap_or_else(|e| e.into_inner());
        match active.as_ref() {
            Some(c) => handle_subscription(&st.server, &c.subs, &c.dirty, &c.wake, &req, method),
            None => req.get("id").cloned().map(|id| {
                json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32002, "message":
                    "no notification stream is connected: open GET /mcp with `Accept: \
                     text/event-stream` first, then subscribe — this subscription's \
                     notifications would otherwise have nowhere to go"}})
            }),
        }
    } else {
        handle_request(&st.server, &req, crate::Transport::Http).await
    };
    match resp {
        // `Json` sets `content-type: application/json`; we add the session-id
        // header the streamable-HTTP handshake needs.
        Some(resp) => ([(MCP_SESSION_ID.clone(), session_id())], Json(resp)).into_response(),
        // A JSON-RPC notification (no `id`) expects no body. Streamable-HTTP
        // wants `202 Accepted` here, not `200` with a `{}` body — the latter
        // is what stalled Codex's client.
        None => StatusCode::ACCEPTED.into_response(),
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

/// Build the MCP HTTP router (POST /mcp for JSON-RPC, GET /mcp for the SSE
/// push stream, GET /healthz). Requires a
/// token: there is no way to ask this crate for an unauthenticated production
/// server, because an `Option` here is an invitation to pass `None` by accident.
pub fn router_with_auth(server: McpServer, token: crate::auth::Token) -> Router {
    router_inner(server, Some(token))
}

/// Layering: the token check is a `route_layer` on `/mcp` alone, while the Host
/// check is an outer `layer` covering every route (and unmatched paths), so
/// nothing this server exposes answers a non-loopback caller.
fn router_inner(server: McpServer, token: Option<crate::auth::Token>) -> Router {
    router_inner_with_push(server, token).0
}

/// Like `router_inner`, also handing back the push state so a graceful
/// shutdown can end the live SSE stream (see `PushState::end_streams`).
fn router_inner_with_push(
    server: McpServer,
    token: Option<crate::auth::Token>,
) -> (Router, Arc<PushState>) {
    let push = Arc::new(PushState::default());
    let state = AppState { server: Arc::new(server), push: push.clone() };
    let router = Router::new()
        // POST and GET share the route, so the token `route_layer` below
        // covers the SSE stream exactly like the JSON-RPC endpoint — a
        // long-lived stream is established under the same bearer check.
        .route("/mcp", post(rpc).get(sse))
        .route_layer(middleware::from_fn_with_state(token, token_guard))
        .route("/healthz", get(|| async { "ok" }))
        .layer(middleware::from_fn(host_guard))
        .with_state(state);
    (router, push)
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
    let (router, push) = router_inner_with_push(server, Some(token));
    // End the live SSE stream BEFORE axum starts waiting on in-flight
    // bodies, or an idle connected client would hold the shutdown open
    // forever (see `PushState::end_streams`).
    let shutdown = async move {
        shutdown.await;
        push.end_streams();
    };
    axum::serve(listener, router).with_graceful_shutdown(shutdown).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::McpServer;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use serde_json::json;
    use srelens_capability::{Capability, Registry};
    use tower::ServiceExt; // oneshot

    fn test_server() -> McpServer {
        let mut reg = Registry::new();
        reg.register(Capability::read_only("ping", "health", |v| async move {
            Ok(json!({ "echo": v }))
        }));
        McpServer::new(Arc::new(reg))
    }

    /// A server whose watcher spawns a forever-pending task per subscription,
    /// firing `on_change` once synchronously (so the notification is already
    /// queued when subscribe answers) and parking each task's JoinHandle where
    /// the test can await its cancellation.
    fn subscribable_server(
        joins: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    ) -> McpServer {
        struct Kinds;
        impl crate::resources::KindResolver for Kinds {
            fn scope(&self, kind: &str) -> Option<crate::resources::KindScope> {
                (kind == "Pod").then_some(crate::resources::KindScope::Namespaced)
            }
        }
        struct FireOnceWatcher(Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>);
        impl crate::resources::ObjectWatcher for FireOnceWatcher {
            fn watch(
                &self,
                _uri: &crate::resources::ResourceUri,
                mut on_change: Box<dyn FnMut() + Send>,
                _on_dead: Box<dyn FnOnce(String) + Send>,
            ) -> Result<tokio::task::AbortHandle, String> {
                on_change();
                let join = tokio::spawn(async { std::future::pending::<()>().await });
                let abort = join.abort_handle();
                self.0.lock().unwrap().push(join);
                Ok(abort)
            }
        }
        test_server()
            .with_kind_resolver(Arc::new(Kinds))
            .with_watcher(Arc::new(FireOnceWatcher(joins)))
    }

    fn sse_get() -> Request<Body> {
        Request::builder()
            .method("GET")
            .uri("/mcp")
            .header("host", "127.0.0.1:8765")
            .header("accept", "text/event-stream")
            .body(Body::empty())
            .unwrap()
    }

    fn subscribe_post(uri: &str) -> Request<Body> {
        let body = json!({"jsonrpc":"2.0","id":7,"method":"resources/subscribe",
            "params":{"uri": uri}});
        Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("host", "127.0.0.1:8765")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    /// Next data chunk of a streaming body, under a bound so a regression
    /// hangs the test for two seconds instead of forever.
    async fn next_chunk(body: &mut axum::body::BodyDataStream) -> Option<String> {
        use futures_core::Stream as _;
        use std::future::poll_fn;
        let fut = poll_fn(|cx| Pin::new(&mut *body).poll_next(cx));
        match tokio::time::timeout(std::time::Duration::from_secs(2), fut).await {
            Ok(Some(Ok(bytes))) => Some(String::from_utf8_lossy(&bytes).to_string()),
            _ => None,
        }
    }

    #[tokio::test]
    async fn the_sse_route_sits_behind_the_same_token_check() {
        let app = router_inner(test_server(), Some(crate::auth::Token::generate()));
        let resp = app.oneshot(sse_get()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn the_sse_route_rejects_a_non_loopback_host() {
        let app = router(test_server());
        let req = Request::builder()
            .method("GET")
            .uri("/mcp")
            .header("host", "evil.example.com")
            .header("accept", "text/event-stream")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn subscribe_without_a_stream_is_refused() {
        let joins = Arc::new(Mutex::new(Vec::new()));
        let app = router(subscribable_server(joins.clone()));
        let resp = app.oneshot(subscribe_post("k8s://c/ns/Pod/web-0")).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["error"]["code"], json!(-32002));
        assert!(
            v["error"]["message"].as_str().unwrap().contains("GET /mcp"),
            "the refusal must say how to fix it: {v}"
        );
        assert!(joins.lock().unwrap().is_empty(), "no watch may be spawned for a refused subscribe");
    }

    #[tokio::test]
    async fn a_watch_notification_reaches_the_http_client() {
        let joins = Arc::new(Mutex::new(Vec::new()));
        let app = router(subscribable_server(joins));

        let stream_resp = app.clone().oneshot(sse_get()).await.unwrap();
        assert_eq!(stream_resp.status(), StatusCode::OK);
        assert!(
            stream_resp
                .headers()
                .get(axum::http::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|c| c.starts_with("text/event-stream")),
            "the GET stream must be SSE"
        );
        let mut body = stream_resp.into_body().into_data_stream();

        let sub_resp = app.oneshot(subscribe_post("k8s://c/ns/Pod/web-0")).await.unwrap();
        let sub_body = axum::body::to_bytes(sub_resp.into_body(), 64 * 1024).await.unwrap();
        let v: Value = serde_json::from_slice(&sub_body).unwrap();
        assert!(v.get("error").is_none(), "subscribe must succeed with a live stream: {v}");

        // The watcher fired on_change synchronously inside subscribe, so the
        // notification is queued; read frames until it arrives (skipping any
        // keep-alive comments).
        let mut seen = String::new();
        for _ in 0..5 {
            match next_chunk(&mut body).await {
                Some(chunk) => {
                    seen.push_str(&chunk);
                    if seen.contains("notifications/resources/updated") {
                        break;
                    }
                }
                None => break,
            }
        }
        assert!(
            seen.contains("notifications/resources/updated") && seen.contains("k8s://c/ns/Pod/web-0"),
            "the update notification must reach the HTTP client: got {seen:?}"
        );
    }

    #[tokio::test]
    async fn a_dropped_stream_releases_its_watch() {
        let joins = Arc::new(Mutex::new(Vec::new()));
        let app = router(subscribable_server(joins.clone()));

        let stream_resp = app.clone().oneshot(sse_get()).await.unwrap();
        let sub_resp = app.oneshot(subscribe_post("k8s://c/ns/Pod/web-0")).await.unwrap();
        assert_eq!(sub_resp.status(), StatusCode::OK);
        assert_eq!(joins.lock().unwrap().len(), 1, "the subscribe spawned a watch");

        // Client disconnect: hyper drops the response body; the stream guard
        // must abort the watch — no watch outlives the stream that asked.
        drop(stream_resp);
        let join = joins.lock().unwrap().pop().unwrap();
        match tokio::time::timeout(std::time::Duration::from_secs(2), join).await {
            Ok(Err(e)) if e.is_cancelled() => {}
            other => panic!("the watch must be cancelled when its stream drops: {other:?}"),
        }
    }

    /// The graceful-shutdown deadlock from review: the state and the stream
    /// kept each other alive, so axum's shutdown waited on the SSE body
    /// forever and the desktop escaped only via its 2s abort fallback.
    /// `end_streams` must END the live body (which is what lets the shutdown
    /// complete), abort its watches, and refuse a late stream.
    #[tokio::test]
    async fn graceful_shutdown_ends_the_live_stream_instead_of_waiting_on_it() {
        let joins = Arc::new(Mutex::new(Vec::new()));
        let (app, push) = router_inner_with_push(subscribable_server(joins.clone()), None);

        let stream_resp = app.clone().oneshot(sse_get()).await.unwrap();
        let sub = app.clone().oneshot(subscribe_post("k8s://c/ns/Pod/web-0")).await.unwrap();
        assert_eq!(sub.status(), StatusCode::OK);
        let mut body = stream_resp.into_body().into_data_stream();

        push.end_streams();
        let mut flushed = 0;
        loop {
            match next_chunk(&mut body).await {
                None => break,
                Some(_) if flushed < 3 => flushed += 1,
                Some(chunk) => panic!("the stream must end at shutdown, got: {chunk:?}"),
            }
        }
        // In production hyper drops the completed body, which is what runs
        // the guard; the drop is explicit here.
        drop(body);
        let join = joins.lock().unwrap().pop().unwrap();
        match tokio::time::timeout(std::time::Duration::from_secs(2), join).await {
            Ok(Err(e)) if e.is_cancelled() => {}
            other => panic!("the watch must die with the shut-down stream: {other:?}"),
        }

        // An already-accepted GET racing the shutdown cannot reopen a stream
        // and stall it all over again.
        let late = app.oneshot(sse_get()).await.unwrap();
        assert_eq!(late.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn a_new_stream_replaces_the_old_and_aborts_its_watches() {
        let joins = Arc::new(Mutex::new(Vec::new()));
        let app = router(subscribable_server(joins.clone()));

        let first = app.clone().oneshot(sse_get()).await.unwrap();
        let sub = app.clone().oneshot(subscribe_post("k8s://c/ns/Pod/web-0")).await.unwrap();
        assert_eq!(sub.status(), StatusCode::OK);
        let mut first_body = first.into_body().into_data_stream();

        // A reconnecting client opens a fresh stream: the old one ends. It
        // may first flush what it had already queued (the buffered wakeup
        // survives the sender dropping), so drain to end-of-stream rather
        // than expecting an instant close — but it must END within those
        // already-queued frames, not linger half-dead.
        let second = app.clone().oneshot(sse_get()).await.unwrap();
        assert_eq!(second.status(), StatusCode::OK);
        let mut flushed = 0;
        loop {
            match next_chunk(&mut first_body).await {
                None => break,
                Some(_) if flushed < 3 => flushed += 1,
                Some(chunk) => panic!("the replaced stream must end, got: {chunk:?}"),
            }
        }
        drop(first_body);
        // ...and its watch dies with it.
        let join = joins.lock().unwrap().pop().unwrap();
        match tokio::time::timeout(std::time::Duration::from_secs(2), join).await {
            Ok(Err(e)) if e.is_cancelled() => {}
            other => panic!("the replaced stream's watch must be cancelled: {other:?}"),
        }

        // The new stream subscribes cleanly — replacement leaves no residue.
        let resub = app.oneshot(subscribe_post("k8s://c/ns/Pod/web-0")).await.unwrap();
        let body = axum::body::to_bytes(resub.into_body(), 64 * 1024).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert!(v.get("error").is_none(), "resubscribe on the new stream must succeed: {v}");
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
        // Streamable-HTTP clients (Codex) require a session id on the response.
        assert!(
            resp.headers()
                .get("mcp-session-id")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|s| !s.is_empty()),
            "an /mcp JSON-RPC response must carry a non-empty Mcp-Session-Id header"
        );
        let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["result"]["isError"], false);
        assert!(v["result"]["content"][0]["text"].as_str().unwrap().contains("echo"));
    }

    #[tokio::test]
    async fn a_notification_returns_202_accepted_with_no_body() {
        // A JSON-RPC message with no `id` is a notification — streamable-HTTP
        // expects 202 Accepted and an empty body, not 200 with `{}` (which
        // stalled Codex's client).
        let app = router(test_server());
        let body = json!({"jsonrpc":"2.0","method":"notifications/initialized"});
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
        assert_eq!(resp.status(), StatusCode::ACCEPTED);
        let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
        assert!(bytes.is_empty(), "a 202 notification response must have no body");
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
