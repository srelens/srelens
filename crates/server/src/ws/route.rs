//! `/api/ws` — the multiplexed streaming socket. Authenticated by the session
//! cookie; CSRF-protected by an Origin check (a browser can't set the custom
//! CSRF header on a WS handshake, but it always sends a truthful Origin, and
//! JS can't forge it). Carries `{op:"sub"|"unsub"}` control frames in and
//! `{channel,payload}` data frames out.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::header::ORIGIN;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
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
    upgrade.on_upgrade(move |socket| run_socket(socket, hub, user_envs, user_id))
}

#[derive(Deserialize)]
struct ClientFrame {
    op: String,
    channel: Option<String>,
}

async fn run_socket(
    socket: WebSocket,
    hub: std::sync::Arc<WsHub>,
    user_envs: std::sync::Arc<UserEnvs>,
    user_id: i64,
) {
    use futures::{SinkExt, StreamExt};
    let (mut ws_tx, mut ws_rx) = socket.split();
    let (conn_id, mut out_rx) = hub.register(user_id);

    // Write task: drain the hub's outgoing frames to the client, and send a
    // keepalive ping every 30s (browsers auto-reply with a matching pong). A
    // dropped `out_rx` sender — from an overflow-close in the hub — makes
    // `recv()` return `None`, ending the writer.
    let writer = tokio::spawn(async move {
        let mut ping = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            tokio::select! {
                maybe = out_rx.recv() => match maybe {
                    Some(frame) => {
                        if ws_tx.send(Message::Text(frame)).await.is_err() { break; }
                    }
                    None => break, // hub closed the connection (overflow) → end
                },
                _ = ping.tick() => {
                    if ws_tx.send(Message::Ping(Vec::new())).await.is_err() { break; }
                }
            }
        }
    });

    // Read loop: handle sub/unsub control frames.
    while let Some(Ok(msg)) = ws_rx.next().await {
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
