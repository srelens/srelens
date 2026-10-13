//! GET /api/logs/export — a log stream's history as a download (#914). It is
//! streamed from the spool in chunks, so a 256 MB history never sits whole in
//! server memory.

use std::io::{self, Write};

use axum::body::{Body, Bytes};
use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::mpsc;

use crate::auth::session::UserCtx;
use crate::AppState;

/// How much of the export goes to the client at a time.
const CHUNK: usize = 64 * 1024;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportQuery {
    channel: String,
    #[serde(default)]
    since_seconds: Option<u64>,
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

pub async fn export(
    State(state): State<AppState>,
    Extension(user): Extension<UserCtx>,
    Query(query): Query<ExportQuery>,
) -> Response {
    let env = match state
        .user_envs
        .env_for(&state.db, &state.master_key, user.user_id)
        .await
    {
        Ok(env) => env,
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    };
    // Said before the 200: once the body streams, a status can no longer change.
    if !env.streams.logs.keeps_history(&query.channel) {
        return error(StatusCode::NOT_FOUND, "this log stream keeps no history");
    }
    let since = query.since_seconds.map(srelens_streams::log_spool::cutoff);
    let (tx, rx) = mpsc::channel::<Result<Bytes, io::Error>>(8);
    tokio::spawn(async move {
        let out = ChunkSender {
            tx: tx.clone(),
            buf: Vec::with_capacity(CHUNK),
        };
        if let Err(e) = env.streams.logs.export(&query.channel, out, since).await {
            // Ends the body with an error, so the browser's download fails
            // rather than saving a file that looks whole.
            let _ = tx.send(Err(io::Error::other(e))).await;
        }
    });
    let body = Body::from_stream(futures::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|item| (item, rx))
    }));
    ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], body).into_response()
}

/// The export's `Write`: what is written goes to the response body in
/// [`CHUNK`]s. It runs on the export's blocking thread, so it may block on a
/// full channel: that is the client's pace reaching the spool reader.
struct ChunkSender {
    tx: mpsc::Sender<Result<Bytes, io::Error>>,
    buf: Vec<u8>,
}

impl Write for ChunkSender {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(data);
        if self.buf.len() >= CHUNK {
            self.flush()?;
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        let chunk = Bytes::from(std::mem::take(&mut self.buf));
        self.tx
            .blocking_send(Ok(chunk))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "the download was closed"))
    }
}

#[cfg(test)]
mod tests {
    use super::{ChunkSender, CHUNK};
    use crate::{router, AppState};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use srelens_capability::Registry;
    use srelens_streams::logs::LogTarget;
    use std::io::{ErrorKind, Write};
    use std::sync::Arc;
    use tokio::sync::mpsc;
    use tower::ServiceExt;

    /// The body goes out in chunks once they fill, the rest on a flush, and a
    /// download the client closed stops the export instead of buffering for it.
    #[test]
    fn the_writer_sends_full_chunks_the_rest_on_flush_and_stops_when_the_client_leaves() {
        let (tx, mut rx) = mpsc::channel(8);
        let mut out = ChunkSender {
            tx,
            buf: Vec::new(),
        };
        out.write_all(&[b'a'; 100]).unwrap();
        assert!(rx.try_recv().is_err(), "a short write waits for the chunk");
        out.write_all(&vec![b'b'; CHUNK]).unwrap();
        assert_eq!(rx.try_recv().unwrap().unwrap().len(), CHUNK + 100);
        out.write_all(b"tail").unwrap();
        assert!(rx.try_recv().is_err());
        out.flush().unwrap();
        assert_eq!(&rx.try_recv().unwrap().unwrap()[..], b"tail");

        drop(rx);
        out.write_all(b"more").unwrap();
        assert_eq!(out.flush().unwrap_err().kind(), ErrorKind::BrokenPipe);
    }

    /// A signed-in user with a spooled stream on `logs:1` whose error line has landed.
    async fn user_with_history(state: &AppState) -> String {
        let user = state.db.upsert_user("i", "s", "u@x", "U", 1).await.unwrap();
        let token = state
            .db
            .create_session(user.id, crate::unix_now())
            .await
            .unwrap();
        let env = state
            .user_envs
            .env_for(&state.db, &state.master_key, user.id)
            .await
            .unwrap();
        env.streams
            .logs
            .start(
                Arc::new(srelens_streams::test_util::TestSink::default()),
                "c".into(),
                "n".into(),
                vec![LogTarget {
                    pod: "web-1".into(),
                    container: None,
                    label: String::new(),
                }],
                "logs:1".into(),
                Some(true),
                None,
                None,
                Some(64 << 20),
            )
            .await
            .unwrap();
        for _ in 0..300 {
            if !env
                .streams
                .logs
                .read("logs:1", u64::MAX, 1)
                .await
                .unwrap()
                .lines
                .is_empty()
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        token
    }

    async fn get(state: &AppState, uri: &str, token: &str, csrf: bool) -> (StatusCode, String) {
        let mut req = Request::builder()
            .uri(uri)
            .header("cookie", format!("srelens_session={token}"));
        if csrf {
            req = req.header("x-srelens-csrf", "1");
        }
        let resp = router(state.clone())
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
            .await
            .unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    #[tokio::test]
    async fn the_export_streams_the_history_as_text() {
        let state = AppState::for_tests(Arc::new(Registry::new())).await;
        let token = user_with_history(&state).await;
        let (status, body) = get(&state, "/api/logs/export?channel=logs:1", &token, true).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body.starts_with("web-1 | [error:"), "{body}");
    }

    /// The browser sends `sinceSeconds`, so no value of it may panic the handler: one
    /// reaching back past the clock's start means every line.
    #[tokio::test]
    async fn a_window_reaching_back_past_the_clocks_start_exports_every_line() {
        let state = AppState::for_tests(Arc::new(Registry::new())).await;
        let token = user_with_history(&state).await;
        // 1e12 overflows Windows' clock, 1e13 is past what chrono can hold on unix,
        // and u64::MAX overflows both.
        for seconds in ["1000000000000", "10000000000000", "18446744073709551615"] {
            let uri = format!("/api/logs/export?channel=logs:1&sinceSeconds={seconds}");
            let (status, body) = get(&state, &uri, &token, true).await;
            assert_eq!(status, StatusCode::OK, "{seconds}: {body}");
            assert!(body.starts_with("web-1 | [error:"), "{seconds}: {body}");
        }
    }

    #[tokio::test]
    async fn the_export_needs_the_csrf_header() {
        let state = AppState::for_tests(Arc::new(Registry::new())).await;
        let token = user_with_history(&state).await;
        let (status, _) = get(&state, "/api/logs/export?channel=logs:1", &token, false).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn a_stream_without_history_is_not_found() {
        let state = AppState::for_tests(Arc::new(Registry::new())).await;
        let token = user_with_history(&state).await;
        let (status, body) = get(&state, "/api/logs/export?channel=logs:none", &token, true).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(body.contains("keeps no history"), "{body}");
    }
}
