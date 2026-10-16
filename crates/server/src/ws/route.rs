//! `/api/ws` — the multiplexed streaming socket. Authenticated by the session
//! cookie; CSRF-protected by an Origin check (a browser can't set the custom
//! CSRF header on a WS handshake, but it always sends a truthful Origin, and
//! JS can't forge it). Carries `{op:"sub"|"unsub"}` control frames in and
//! `{channel,payload}` data frames out.

use axum::extract::ws::{Message, WebSocketUpgrade};
use axum::extract::State;
use axum::http::header::ORIGIN;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;

use crate::auth::session;
use crate::auth::AuthConfig;
use crate::db::Db;
use crate::users::{teardown_streams_if_disconnected, UserEnvs};
use crate::ws::hub::{ack_frame, WsHub};
use crate::AppState;

/// Grace period after a user's last WS connection drops before their
/// server-side stream tasks (watches/logs/exec/terminal/helm/forward) are
/// torn down. Long enough to survive a page refresh / brief reconnect
/// (wsClient's backoff caps at 10s), short enough to bound web-session
/// memory when a tab is actually closed.
const WS_DISCONNECT_GRACE_SECS: u64 = 60;

/// Authenticate a WS upgrade: valid session cookie required; if an `Origin`
/// header is present it must equal the configured public URL. Returns the
/// authenticated user id.
pub async fn authorize_ws(
    headers: &HeaderMap,
    auth: &AuthConfig,
    db: &Db,
    now: i64,
) -> Result<i64, (StatusCode, &'static str)> {
    if let Some(origin) = headers.get(ORIGIN) {
        let ok = origin
            .to_str()
            .map(|o| o == auth.public_url)
            .unwrap_or(false);
        if !ok {
            return Err((StatusCode::FORBIDDEN, "origin not allowed"));
        }
    }
    let token = session::cookie_value(headers, session::COOKIE_NAME)
        .ok_or((StatusCode::UNAUTHORIZED, "unauthenticated"))?;
    match db.validate_session(&token, now).await {
        Ok(Some(user)) => Ok(user.id),
        Ok(None) => Err((StatusCode::UNAUTHORIZED, "unauthenticated")),
        Err(_) => Err((StatusCode::INTERNAL_SERVER_ERROR, "session lookup failed")),
    }
}

/// GET /api/ws
pub async fn ws_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    let user_id = match authorize_ws(&headers, &state.auth, &state.db, crate::unix_now()).await {
        Ok(id) => id,
        Err((status, msg)) => {
            return (status, axum::Json(serde_json::json!({ "error": msg }))).into_response()
        }
    };
    let hub = state.ws_hub.clone();
    let user_envs = state.user_envs.clone();
    upgrade.on_upgrade(move |socket| {
        let (ws_tx, ws_rx) = socket.split();
        run_socket(ws_tx, ws_rx, hub, user_envs, user_id)
    })
}

#[derive(Deserialize)]
struct ClientFrame {
    op: String,
    channel: Option<String>,
}

/// Generic over the socket's two halves so a test can drive it in memory;
/// `ws_handler` passes the halves of the upgraded `WebSocket`.
async fn run_socket<Tx, Rx>(
    mut ws_tx: Tx,
    mut ws_rx: Rx,
    hub: std::sync::Arc<WsHub>,
    user_envs: std::sync::Arc<UserEnvs>,
    user_id: i64,
) where
    Tx: futures::Sink<Message> + Unpin + Send + 'static,
    Rx: futures::Stream<Item = Result<Message, axum::Error>> + Unpin,
{
    let (conn_id, mut out_rx) = hub.register(user_id);

    // Write task: drain the hub's outgoing frames to the client, and send a
    // keepalive ping every 30s (browsers auto-reply with a matching pong).
    //
    // When the hub drops this connection (its queue overflowed) the channel
    // still holds up to `WS_CONN_QUEUE` frames, and `subscribe`/`deliver_direct`
    // on it are now no-ops: draining them would keep a dead socket open at the
    // slow browser's pace, with new subscriptions hanging and live streams
    // frozen. So close at once — the client reconnects and resubscribes, log
    // frames carry `seq` so it tolerates the gap, and the history is on disk.
    let writer_hub = hub.clone();
    let mut writer = tokio::spawn(async move {
        let mut ping = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            tokio::select! {
                maybe = out_rx.recv() => match maybe {
                    Some(frame) if writer_hub.has_connection(conn_id) => {
                        if ws_tx.send(Message::Text(frame)).await.is_err() { break; }
                    }
                    _ => {
                        let _ = ws_tx.send(Message::Close(None)).await;
                        break;
                    }
                },
                _ = ping.tick() => {
                    if ws_tx.send(Message::Ping(Vec::new())).await.is_err() { break; }
                }
            }
        }
    });

    // Read loop: handle sub/unsub control frames. It also ends when the writer
    // does: dropping `ws_rx` is what actually closes the socket, and while it
    // stayed open a `sub` would be accepted and never acknowledged.
    loop {
        let msg = tokio::select! {
            next = ws_rx.next() => match next {
                Some(Ok(msg)) => msg,
                _ => break,
            },
            _ = &mut writer => break,
        };
        if let Message::Text(text) = msg {
            if let Ok(frame) = serde_json::from_str::<ClientFrame>(&text) {
                match (frame.op.as_str(), frame.channel) {
                    ("sub", Some(channel)) => {
                        hub.subscribe(conn_id, &channel);
                        // Ack via the same outgoing path (ordering preserved).
                        hub.deliver_direct(conn_id, ack_frame(&channel));
                    }
                    ("unsub", Some(channel)) => hub.unsubscribe(conn_id, &channel),
                    _ => {}
                }
            }
        }
        // Ping/Close/Binary are handled by axum or ignored.
    }

    hub.unregister(conn_id);
    writer.abort();

    // If that was the user's last live connection, schedule teardown of their
    // stream tasks after a grace period (a reconnect within the grace, e.g.
    // on refresh, cancels it).
    if hub.user_connection_count(user_id) == 0 {
        tokio::spawn(teardown_streams_if_disconnected(
            hub.clone(),
            user_envs,
            user_id,
            std::time::Duration::from_secs(WS_DISCONNECT_GRACE_SECS),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AppState;
    use srelens_capability::Registry;
    use std::sync::Arc;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                v.parse().unwrap(),
            );
        }
        h
    }

    #[tokio::test]
    async fn rejects_mismatched_origin() {
        let state = AppState::for_tests(Arc::new(Registry::new())).await;
        let h = headers(&[("origin", "https://evil.example")]);
        let err = authorize_ws(&h, &state.auth, &state.db, 1)
            .await
            .unwrap_err();
        assert_eq!(err.0, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn rejects_missing_session() {
        let state = AppState::for_tests(Arc::new(Registry::new())).await;
        // Matching origin, but no cookie.
        let h = headers(&[("origin", "http://127.0.0.1:8080")]);
        let err = authorize_ws(&h, &state.auth, &state.db, 1)
            .await
            .unwrap_err();
        assert_eq!(err.0, StatusCode::UNAUTHORIZED);
    }

    /// Run `fut`, failing the test if it takes more than a few seconds.
    async fn within<T>(what: &str, fut: impl std::future::Future<Output = T>) -> T {
        tokio::time::timeout(std::time::Duration::from_secs(5), fut)
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
    }

    /// #914: the hub drops a connection whose queue overflowed, and with it the
    /// only sender, but the channel still holds the frames already buffered.
    /// The route must close the socket at once, so the browser reconnects and
    /// resubscribes, instead of forwarding those stale frames at a slow
    /// browser's pace while a new `sub` on this socket silently does nothing.
    #[tokio::test]
    async fn closes_the_socket_at_once_when_the_hub_dropped_the_connection() {
        use futures::channel::mpsc::unbounded;
        use srelens_streams::EventSink;

        let state = AppState::for_tests(Arc::new(Registry::new())).await;
        let hub = state.ws_hub.clone();
        // The two halves of the browser's socket: what the server writes, and
        // what the browser sends.
        let (ws_tx, mut from_server) = unbounded::<Message>();
        let (to_server, ws_rx) = unbounded::<Result<Message, axum::Error>>();
        let socket = tokio::spawn(run_socket(
            ws_tx,
            ws_rx,
            hub.clone(),
            state.user_envs.clone(),
            1,
        ));

        // Subscribe, and wait for the ack so the subscription is registered.
        let sub = Message::Text(r#"{"op":"sub","channel":"ch"}"#.into());
        to_server.unbounded_send(Ok(sub)).unwrap();
        let ack = loop {
            // The writer's keepalive fires its first tick at once; skip it.
            match within("the sub ack", from_server.next()).await {
                Some(Message::Ping(_)) => continue,
                other => break other,
            }
        };
        assert!(
            matches!(&ack, Some(Message::Text(t)) if t.contains("subbed")),
            "{ack:?}"
        );

        // Overflow the queue with no await in between: the writer can't run on
        // this single-threaded runtime, so the hub drops the connection with
        // WS_CONN_QUEUE frames still buffered.
        let sink = crate::ws::hub::WsSink {
            hub: hub.clone(),
            user_id: 1,
        };
        for i in 0..(crate::ws::hub::WS_CONN_QUEUE + 5) {
            sink.emit("ch", serde_json::json!(i));
        }
        assert_eq!(
            hub.user_connection_count(1),
            0,
            "the hub should have dropped it"
        );

        within("the socket to close after the hub dropped it", socket)
            .await
            .unwrap();
        let frames: Vec<Message> = from_server.collect().await;
        let stale = frames
            .iter()
            .filter(|m| matches!(m, Message::Text(_)))
            .count();
        assert_eq!(stale, 0, "forwarded {stale} stale frames to the browser");
        assert!(
            matches!(frames.last(), Some(Message::Close(_))),
            "the browser was never sent a Close frame: {frames:?}"
        );
    }

    #[tokio::test]
    async fn accepts_valid_session_and_matching_or_absent_origin() {
        let state = AppState::for_tests(Arc::new(Registry::new())).await;
        let user = state.db.upsert_user("i", "s", "u@x", "U", 1).await.unwrap();
        let token = state.db.create_session(user.id, 1).await.unwrap();

        // Matching origin + cookie.
        let h = headers(&[
            ("origin", "http://127.0.0.1:8080"),
            ("cookie", &format!("srelens_session={token}")),
        ]);
        assert_eq!(
            authorize_ws(&h, &state.auth, &state.db, 1).await.unwrap(),
            user.id
        );

        // Absent origin (non-browser client) + cookie is allowed.
        let h = headers(&[("cookie", &format!("srelens_session={token}"))]);
        assert_eq!(
            authorize_ws(&h, &state.auth, &state.db, 1).await.unwrap(),
            user.id
        );
    }
}
