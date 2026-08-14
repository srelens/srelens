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
use std::sync::{Arc, Mutex};
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

/// More concurrent sessions than this and new push streams are refused: this
/// is a loopback single-user server, and its realistic clients are one IDE
/// plus an assistant CLI or two — a runaway client must not mint streams
/// (each carrying up to `MAX_SUBSCRIPTIONS` watches) without bound.
const MAX_PUSH_SESSIONS: usize = 8;

/// The push half of the transport: one SSE stream PER SESSION, keyed by the
/// `Mcp-Session-Id` this server minted at `initialize` (see `rpc`). Distinct
/// clients — a configured IDE and an assistant CLI, say — get distinct
/// sessions, so one client's GET can never hijack or tear down another's
/// stream; a GET carrying a session that already has a stream REPLACES that
/// stream (the reconnect case), ending the old one and aborting the watches
/// it owned — no watch ever outlives the stream that requested it.
/// Subscriptions arriving for a session with no connected stream are refused
/// (see `rpc`): accepting one would promise notifications with nowhere to
/// send them, the exact failure shape #195 eliminated.
#[derive(Default)]
struct PushState {
    sessions: Mutex<std::collections::BTreeMap<String, PushChannels>>,
    /// Distinguishes streams so a dying stream's guard can only clear ITS
    /// slot, never a successor's (same shape as the subscription registry's
    /// generations).
    next_id: AtomicU64,
    /// Set by `end_streams` at graceful shutdown. Checked under the
    /// `sessions` lock on install, so a GET that was already accepted when
    /// shutdown began cannot slip a fresh stream in after the teardown and
    /// stall the shutdown all over again.
    closed: std::sync::atomic::AtomicBool,
}

impl PushState {
    /// Install `chans` as `session`'s stream, unless the server is shutting
    /// down or the session cap is hit by OTHER sessions. Returns whether it
    /// was installed. Dropping a previous same-session occupant here is what
    /// ends a replaced stream (the reconnect case): its close signal fires
    /// and its guard aborts the watches it owned.
    fn install(&self, session: &str, chans: PushChannels) -> Result<(), StatusCode> {
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        if self.closed.load(Ordering::Relaxed) {
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
        if !sessions.contains_key(session) && sessions.len() >= MAX_PUSH_SESSIONS {
            // Reap sessions whose stream already died before refusing —
            // mirrors the subscription registry's cap-time reaping.
            sessions.retain(|_, c| !c.wake.is_closed());
            if sessions.len() >= MAX_PUSH_SESSIONS {
                return Err(StatusCode::TOO_MANY_REQUESTS);
            }
        }
        sessions.insert(session.to_string(), chans);
        Ok(())
    }

    /// Graceful-shutdown teardown (#193 review): drop every session's
    /// channels so their SSE bodies END. Ending the body is what lets axum's
    /// graceful shutdown complete instead of waiting on connected clients
    /// forever (the desktop's only escape was its two-second abort
    /// fallback, delaying stop and token rotation).
    fn end_streams(&self) {
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        self.closed.store(true, Ordering::Relaxed);
        sessions.clear();
    }
}

/// What `resources/subscribe` needs from a session's stream: the same
/// registry/dirty-set/wakeup trio the stdio serve loop owns per session.
/// Deliberately NOT `Clone`: `_close` is the one non-cloneable handle whose
/// drop tells the stream to end. The wake sender CANNOT serve that purpose —
/// `handle_subscription` clones it into `on_change`/`on_dead`, and the real
/// `CacheWatcher` moves those into its long-lived watch task, so with any
/// real subscription active the wake channel stays open long after this
/// struct is dropped (the first shutdown fix relied on exactly that and
/// deadlocked: stream waits for watches to drop, watches drop when the
/// guard runs, the guard runs when the stream ends).
struct PushChannels {
    id: u64,
    subs: Arc<SubscriptionRegistry>,
    dirty: Arc<Mutex<BTreeSet<String>>>,
    wake: tokio::sync::mpsc::Sender<()>,
    _close: tokio::sync::oneshot::Sender<()>,
}

#[derive(Clone)]
struct AppState {
    server: Arc<McpServer>,
    push: Arc<PushState>,
}

/// Aborts the stream's watches when the stream itself is dropped — client
/// disconnect, replacement by a newer stream, or server shutdown all land
/// here, so the "no watch outlives its stream" contract has one enforcement
/// point. Clears its session's slot only when it still holds this stream.
struct StreamGuard {
    push: Arc<PushState>,
    subs: Arc<SubscriptionRegistry>,
    session: String,
    id: u64,
}

impl Drop for StreamGuard {
    fn drop(&mut self) {
        // Slot removal FIRST, abort SECOND — and the removal takes the same
        // lock every subscribe POST holds across its registration. After the
        // slot is gone no POST can reach this registry (it finds no session
        // and refuses), so the abort that follows closes the set for good.
        // The reverse order had a window: a POST already holding the lock
        // could insert its watch AFTER an early abort_all saw an empty
        // registry, and the slot removal wouldn't abort again — an orphan
        // watch surviving its stream. (A POST that completed before removal
        // still answers success for a watch this abort kills moments later —
        // but its stream is the one that just died, so the client sees the
        // disconnect and resubscribes; no watch outlives the stream.)
        {
            let mut sessions = self.push.sessions.lock().unwrap_or_else(|e| e.into_inner());
            if sessions.get(&self.session).is_some_and(|a| a.id == self.id) {
                sessions.remove(&self.session);
            }
        }
        self.subs.abort_all();
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
    /// Resolves (with `Err`, from the `_close` sender dropping) when this
    /// stream's `PushChannels` leaves the session map — replacement or
    /// shutdown. THE end-of-stream signal: the wake channel cannot be it,
    /// because live watches hold wake-sender clones (see `PushChannels`).
    closed: tokio::sync::oneshot::Receiver<()>,
    dirty: Arc<Mutex<BTreeSet<String>>>,
    pending: VecDeque<String>,
    _guard: StreamGuard,
}

impl futures_core::Stream for PushStream {
    type Item = Result<Event, std::convert::Infallible>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            // The close signal is checked FIRST — before queued frames — so
            // it ends the stream even when a slow client is backpressuring
            // `pending`: replacement and shutdown must terminate promptly,
            // not after the client deigns to read its backlog. Dropping the
            // queued notifications is fine — the watches are being torn
            // down, and the client's reconnect re-reads anyway (the
            // initial-list notification fires on every fresh watch).
            if std::future::Future::poll(Pin::new(&mut this.closed), cx).is_ready() {
                return Poll::Ready(None);
            }
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

/// Whether an `Accept` header value admits `text/event-stream`. Media-type
/// tokens are case-insensitive (`Text/Event-Stream` is valid) and ranges
/// carry optional quality values (`text/event-stream;q=0` is an explicit
/// refusal, not an acceptance) — a bare substring check gets both wrong.
/// The caller treats an ABSENT header as accepting; this only judges a
/// header that is present.
fn accepts_event_stream(accept: &str) -> bool {
    // Per RFC 9110 §12.5.1, the MOST SPECIFIC matching range decides the
    // quality: `text/event-stream;q=0, */*;q=1` refuses SSE (the exact range
    // zeroes it; the wildcard covers everything ELSE), so ranges cannot be
    // judged independently. Specificity: exact > text/* > */*.
    let mut best: Option<(u8, f32)> = None;
    for range in accept.split(',') {
        let mut parts = range.split(';');
        let media = parts.next().unwrap_or("").trim().to_ascii_lowercase();
        let specificity = match media.as_str() {
            "text/event-stream" => 2u8,
            "text/*" => 1,
            "*/*" => 0,
            _ => continue,
        };
        let q = parts
            .filter_map(|p| {
                let p = p.trim().to_ascii_lowercase();
                p.strip_prefix("q=").map(str::to_string)
            })
            .next_back()
            .and_then(|v| v.trim().parse::<f32>().ok())
            .unwrap_or(1.0);
        best = match best {
            Some((s, _)) if specificity > s => Some((specificity, q)),
            // Duplicate ranges at equal specificity: keep the acceptance
            // (lenient — a client contradicting itself gets the stream it
            // half-asked for rather than a 406).
            Some((s, bq)) if specificity == s => Some((s, bq.max(q))),
            Some(kept) => Some(kept),
            None => Some((specificity, q)),
        };
    }
    best.is_some_and(|(_, q)| q > 0.0)
}

/// `GET /mcp`: open the server→client SSE stream. Replaces any previous
/// stream (see `PushState`). The keep-alive comment every 20s holds idle
/// connections open through proxies and lets a dead client surface as a
/// write error instead of a silent zombie.
async fn sse(State(st): State<AppState>, headers: axum::http::HeaderMap) -> Response {
    // The spec has clients send `Accept: text/event-stream`; a GET that
    // refuses it gets 406 rather than a stream it won't parse. An absent
    // Accept header is allowed (curl convenience).
    // ALL Accept field lines, combined: HTTP defines repeated field lines as
    // one comma-joined list, and `HeaderMap::get` sees only one of them.
    let accept = headers
        .get_all(axum::http::header::ACCEPT)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .collect::<Vec<_>>()
        .join(",");
    if !accept.is_empty() && !accepts_event_stream(&accept) {
        return (StatusCode::NOT_ACCEPTABLE, "this endpoint streams text/event-stream").into_response();
    }

    let session = presented_session(&headers).unwrap_or_else(|| DEFAULT_SESSION.to_string());
    let (wake_tx, wake_rx) = tokio::sync::mpsc::channel(1);
    let (close_tx, close_rx) = tokio::sync::oneshot::channel();
    let chans = PushChannels {
        id: st.push.next_id.fetch_add(1, Ordering::Relaxed),
        subs: Arc::new(SubscriptionRegistry::new()),
        dirty: Arc::default(),
        wake: wake_tx,
        _close: close_tx,
    };
    let stream = PushStream {
        wake: wake_rx,
        closed: close_rx,
        dirty: chans.dirty.clone(),
        pending: VecDeque::new(),
        _guard: StreamGuard {
            push: st.push.clone(),
            subs: chans.subs.clone(),
            session: session.clone(),
            id: chans.id,
        },
    };
    // Install as this session's stream (see `PushState::install` for
    // replacement, cap, and shutdown semantics).
    if let Err(status) = st.push.install(&session, chans) {
        let message = match status {
            StatusCode::TOO_MANY_REQUESTS => "too many concurrent push sessions",
            _ => "the server is shutting down",
        };
        return (status, message).into_response();
    }

    (
        [(MCP_SESSION_ID.clone(), session)],
        Sse::new(stream).keep_alive(
            KeepAlive::new().interval(std::time::Duration::from_secs(20)).text("keep-alive"),
        ),
    )
        .into_response()
}

/// The `Mcp-Session-Id` header name, surfaced on every `/mcp` response.
static MCP_SESSION_ID: HeaderName = HeaderName::from_static("mcp-session-id");

/// Mint a fresh MCP session id for one `initialize`. Streamable-HTTP clients
/// (e.g. Codex's `streamable_http` transport) establish a session on
/// `initialize` and refuse to load tools unless the response carries this
/// header — a minimal POST/JSON-RPC endpoint without it hangs their handshake
/// (verified: Codex retries and reports "no tools"). Each client keeps the id
/// IT was given and presents it on later requests, which is what routes its
/// GET stream and its subscriptions to its own session slot — DISTINCT
/// clients (an IDE and an assistant CLI) must not share one id, or one's
/// reconnect would tear down the other's stream. The bearer token, not this
/// id, is what authorizes a request; the id is routing, never auth.
fn mint_session_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    format!("srelens-{}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed))
}

/// The session id a request presents, if any. Absent for `initialize` (the
/// client has none yet) and for hand-rolled callers (curl); those fall back
/// to a shared default slot so single-client use needs no header discipline.
fn presented_session(headers: &axum::http::HeaderMap) -> Option<String> {
    headers.get(&MCP_SESSION_ID).and_then(|v| v.to_str().ok()).map(str::to_string)
}

/// The slot key for requests that present no session id.
const DEFAULT_SESSION: &str = "default";

async fn rpc(
    State(st): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<Value>,
) -> Response {
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
    // Subscription methods are handled here, not in `handle_request` — same
    // split as the stdio serve loop, because they need this transport's push
    // machinery: the registry, dirty set, and wakeup of the caller's OWN
    // session stream. With no stream connected for that session they are
    // refused outright; accepting would promise notifications with nowhere
    // to send them.
    let resp = if method == "resources/subscribe" || method == "resources/unsubscribe" {
        let session = presented_session(&headers).unwrap_or_else(|| DEFAULT_SESSION.to_string());
        // The sessions lock is held across `handle_subscription`, which is
        // entirely synchronous (no await), so registration is SERIALIZED
        // with stream replacement — `PushState::install` takes this same
        // lock. The race this closes: a reconnecting GET replacing the
        // stream between lookup and registration would leave the watch in
        // the old registry (whose guard then aborts it) while the client
        // holds a success response for a subscription the new stream will
        // never deliver.
        let sessions = st.push.sessions.lock().unwrap_or_else(|e| e.into_inner());
        match sessions.get(&session) {
            Some(c) => handle_subscription(&st.server, &c.subs, &c.dirty, &c.wake, &req, method),
            None => req.get("id").cloned().map(|id| {
                json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32002, "message":
                    "no notification stream is connected for this session: open GET /mcp \
                     with `Accept: text/event-stream` (presenting your Mcp-Session-Id) \
                     first, then subscribe — this subscription's notifications would \
                     otherwise have nowhere to go"}})
            }),
        }
    } else {
        handle_request(&st.server, &req, crate::Transport::Http).await
    };
    // Streamable-HTTP session id: `initialize` MINTS a fresh one (each client
    // then presents the id it was given, which is what keeps clients'
    // sessions apart); every other response echoes the presented id back.
    let sid = match presented_session(&headers) {
        Some(sid) => sid,
        None => mint_session_id(),
    };
    match resp {
        // `Json` sets `content-type: application/json`; we add the session-id
        // header the streamable-HTTP handshake needs.
        Some(resp) => ([(MCP_SESSION_ID.clone(), sid)], Json(resp)).into_response(),
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

    #[test]
    fn accept_header_parsing_is_case_insensitive_and_honors_q_values() {
        assert!(accepts_event_stream("text/event-stream"));
        assert!(accepts_event_stream("Text/Event-Stream"), "media types are case-insensitive");
        assert!(accepts_event_stream("application/json, text/event-stream;q=0.5"));
        assert!(accepts_event_stream("*/*"));
        assert!(accepts_event_stream("text/*"));
        assert!(!accepts_event_stream("application/json"));
        assert!(!accepts_event_stream("text/event-stream;q=0"), "q=0 is an explicit refusal");
        assert!(!accepts_event_stream("text/event-stream; q=0.0"));
        assert!(accepts_event_stream("text/event-stream;charset=utf-8"), "non-q params ignored");
        // The most specific matching range decides — a wildcard cannot
        // resurrect an explicitly refused exact range.
        assert!(!accepts_event_stream("text/event-stream;q=0, */*;q=1"));
        assert!(!accepts_event_stream("text/*;q=0, */*"));
        assert!(accepts_event_stream("text/event-stream, text/*;q=0"));
    }

    fn sse_get_with_session(session: &str) -> Request<Body> {
        Request::builder()
            .method("GET")
            .uri("/mcp")
            .header("host", "127.0.0.1:8765")
            .header("accept", "text/event-stream")
            .header("mcp-session-id", session)
            .body(Body::empty())
            .unwrap()
    }

    fn subscribe_post_with_session(uri: &str, session: &str) -> Request<Body> {
        let body = json!({"jsonrpc":"2.0","id":7,"method":"resources/subscribe",
            "params":{"uri": uri}});
        Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("host", "127.0.0.1:8765")
            .header("content-type", "application/json")
            .header("mcp-session-id", session)
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    /// Repeated `Accept` field lines are one combined list per HTTP — a
    /// client sending `application/json` and `text/event-stream` as separate
    /// lines is a valid SSE client whichever line the header map yields
    /// first.
    #[tokio::test]
    async fn multiple_accept_field_lines_are_combined() {
        let app = router(test_server());
        let req = Request::builder()
            .method("GET")
            .uri("/mcp")
            .header("host", "127.0.0.1:8765")
            .header("accept", "application/json")
            .header("accept", "text/event-stream")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    /// Two clients (an IDE and an assistant CLI, say) present distinct
    /// session ids: one's GET must not hijack or tear down the other's
    /// stream, and each subscribe routes to its own session's registry.
    #[tokio::test]
    async fn two_sessions_keep_separate_streams_and_subscriptions() {
        let joins = Arc::new(Mutex::new(Vec::new()));
        let app = router(subscribable_server(joins.clone()));

        let a = app.clone().oneshot(sse_get_with_session("sess-a")).await.unwrap();
        let b = app.clone().oneshot(sse_get_with_session("sess-b")).await.unwrap();
        assert_eq!(a.status(), StatusCode::OK);
        assert_eq!(b.status(), StatusCode::OK);
        let mut a_body = a.into_body().into_data_stream();
        let mut b_body = b.into_body().into_data_stream();

        // A subscribes; the notification reaches A's stream...
        let sub = app
            .clone()
            .oneshot(subscribe_post_with_session("k8s://c/ns/Pod/web-0", "sess-a"))
            .await
            .unwrap();
        let sub_body = axum::body::to_bytes(sub.into_body(), 64 * 1024).await.unwrap();
        let v: Value = serde_json::from_slice(&sub_body).unwrap();
        assert!(v.get("error").is_none(), "A's subscribe must succeed: {v}");
        let mut seen = String::new();
        for _ in 0..5 {
            match next_chunk(&mut a_body).await {
                Some(chunk) => {
                    seen.push_str(&chunk);
                    if seen.contains("notifications/resources/updated") {
                        break;
                    }
                }
                None => break,
            }
        }
        assert!(seen.contains("k8s://c/ns/Pod/web-0"), "A gets its notification: {seen:?}");

        // ...and NOT B's, whose stream stays open and quiet (the bounded read
        // times out with nothing).
        assert!(next_chunk(&mut b_body).await.is_none(), "B must not receive A's notification");

        // B's own GET reconnect replaces only B's stream: A's watch survives.
        let b2 = app.clone().oneshot(sse_get_with_session("sess-b")).await.unwrap();
        assert_eq!(b2.status(), StatusCode::OK);
        assert_eq!(joins.lock().unwrap().len(), 1, "A's watch still exists");
        let join = joins.lock().unwrap().pop().unwrap();
        assert!(!join.is_finished(), "A's watch must survive B's reconnect");
        join.abort();
    }

    /// Each `initialize` mints a fresh session id — two clients handed the
    /// same id would collide in the session map, one reconnect tearing down
    /// the other's stream.
    #[tokio::test]
    async fn initialize_mints_a_unique_session_id_per_client() {
        let app = router(test_server());
        let init = |app: Router| async move {
            let body = json!({"jsonrpc":"2.0","id":1,"method":"initialize"});
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
            resp.headers().get("mcp-session-id").unwrap().to_str().unwrap().to_string()
        };
        let first = init(app.clone()).await;
        let second = init(app).await;
        assert_ne!(first, second, "each client must get its own session id");
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
