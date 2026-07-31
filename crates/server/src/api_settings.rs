//! Per-user settings API: `GET`/`PUT`/`DELETE /api/settings/:key`. Backs
//! saved port-forwards (and future per-user preferences) on the web; the
//! desktop app persists the equivalent state in `localStorage` instead.
//!
//! Values are opaque JSON, stored and returned verbatim in `value_json` —
//! this route never inspects the shape, it just round-trips whatever the
//! caller sent.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde_json::{json, Value};

use crate::auth::session::UserCtx;
use crate::AppState;

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

fn validate_key(key: &str) -> Result<(), &'static str> {
    if key.trim().is_empty() {
        return Err("key must not be empty");
    }
    Ok(())
}

/// GET /api/settings/:key → `{"value": <json | null>}`. The stored
/// `value_json` is parsed and re-emitted as JSON (not a JSON-encoded string),
/// so callers get back exactly the shape they PUT.
pub async fn get(
    State(state): State<AppState>,
    Extension(user): Extension<UserCtx>,
    Path(key): Path<String>,
) -> Response {
    if let Err(msg) = validate_key(&key) {
        return error(StatusCode::BAD_REQUEST, msg);
    }
    match state.db.get_setting(user.user_id, &key).await {
        Ok(Some(raw)) => match serde_json::from_str::<Value>(&raw) {
            Ok(value) => Json(json!({ "value": value })).into_response(),
            Err(e) => error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("stored value is not valid JSON: {e}"),
            ),
        },
        Ok(None) => Json(json!({ "value": null })).into_response(),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

/// PUT /api/settings/:key — stores the request body verbatim as the
/// setting's `value_json`. Requires `Content-Type: application/json`, for the
/// same CORS-preflight reason as `invoke_capability` (see `api.rs`): without
/// it, a cross-origin `fetch` with a `text/plain` body is a CORS "simple
/// request" the browser sends without preflight, which would let any website
/// write this loopback server's per-user settings.
pub async fn put(
    State(state): State<AppState>,
    Extension(user): Extension<UserCtx>,
    Path(key): Path<String>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if let Err(msg) = validate_key(&key) {
        return error(StatusCode::BAD_REQUEST, msg);
    }

    let is_json = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            v.trim()
                .to_ascii_lowercase()
                .starts_with("application/json")
        })
        .unwrap_or(false);
    if !is_json {
        return error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "content-type must be application/json",
        );
    }

    let value: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return error(
                StatusCode::BAD_REQUEST,
                &format!("request body is not valid JSON: {e}"),
            )
        }
    };
    let value_json = value.to_string();

    match state.db.set_setting(user.user_id, &key, &value_json).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

/// DELETE /api/settings/:key
pub async fn delete(
    State(state): State<AppState>,
    Extension(user): Extension<UserCtx>,
    Path(key): Path<String>,
) -> Response {
    if let Err(msg) = validate_key(&key) {
        return error(StatusCode::BAD_REQUEST, msg);
    }
    match state.db.delete_setting(user.user_id, &key).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

#[cfg(test)]
mod tests {
    use crate::{router, AppState};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use serde_json::{json, Value};
    use srelens_capability::Registry;
    use std::sync::Arc;
    use tower::ServiceExt;

    async fn state() -> AppState {
        AppState::for_tests(Arc::new(Registry::new())).await
    }

    async fn send(
        state: &AppState,
        method: &str,
        uri: &str,
        cookie: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("x-srelens-csrf", "1");
        if let Some(cookie) = cookie {
            builder = builder.header("cookie", cookie);
        }
        let body = match body {
            Some(v) => {
                builder = builder.header("content-type", "application/json");
                Body::from(v.to_string())
            }
            None => Body::empty(),
        };
        let resp = router(state.clone())
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .unwrap();
        let v = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        (status, v)
    }

    async fn login(state: &AppState) -> String {
        let user = state.db.upsert_user("i", "s", "u@x", "U", 1).await.unwrap();
        let token = state
            .db
            .create_session(user.id, crate::unix_now())
            .await
            .unwrap();
        format!("srelens_session={token}")
    }

    #[tokio::test]
    async fn settings_routes_require_auth() {
        let state = state().await;
        for (method, uri) in [
            ("GET", "/api/settings/theme"),
            ("PUT", "/api/settings/theme"),
            ("DELETE", "/api/settings/theme"),
        ] {
            let (status, _) = send(&state, method, uri, None, None).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {uri}");
        }
    }

    #[tokio::test]
    async fn value_roundtrips_through_put_and_get() {
        let state = state().await;
        let cookie = login(&state).await;

        // Absent key reads as null.
        let (status, body) = send(&state, "GET", "/api/settings/theme", Some(&cookie), None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!({ "value": null }));

        // PUT stores the body verbatim.
        let (status, _) = send(
            &state,
            "PUT",
            "/api/settings/theme",
            Some(&cookie),
            Some(json!({ "mode": "dark", "scale": 1.5 })),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        let (status, body) = send(&state, "GET", "/api/settings/theme", Some(&cookie), None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!({ "value": { "mode": "dark", "scale": 1.5 } }));

        // DELETE clears it.
        let (status, _) = send(&state, "DELETE", "/api/settings/theme", Some(&cookie), None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, body) = send(&state, "GET", "/api/settings/theme", Some(&cookie), None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!({ "value": null }));
    }

    #[tokio::test]
    async fn put_requires_json_content_type() {
        let state = state().await;
        let cookie = login(&state).await;
        let resp = router(state.clone())
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/settings/theme")
                    .header("cookie", &cookie)
                    .header("x-srelens-csrf", "1")
                    .header("content-type", "text/plain")
                    .body(Body::from("\"dark\""))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    #[tokio::test]
    async fn empty_key_is_rejected() {
        let state = state().await;
        let cookie = login(&state).await;
        // The router only matches non-empty path segments, so an empty key
        // segment can't reach the handler as `/api/settings/` — verify the
        // validator directly rejects it via a whitespace-only key instead.
        let (status, _) = send(
            &state,
            "PUT",
            "/api/settings/%20",
            Some(&cookie),
            Some(json!("x")),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn settings_are_isolated_per_user() {
        let state = state().await;
        let alice_cookie = login(&state).await;

        let bob = state
            .db
            .upsert_user("i", "bob", "b@x", "B", 1)
            .await
            .unwrap();
        let bob_token = state
            .db
            .create_session(bob.id, crate::unix_now())
            .await
            .unwrap();
        let bob_cookie = format!("srelens_session={bob_token}");

        let (status, _) = send(
            &state,
            "PUT",
            "/api/settings/theme",
            Some(&alice_cookie),
            Some(json!("dark")),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        let (status, body) = send(
            &state,
            "GET",
            "/api/settings/theme",
            Some(&bob_cookie),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!({ "value": null }));
    }
}
