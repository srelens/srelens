//! POST /api/capability/:id — dispatches into the shared capability registry.
//! This is the web equivalent of the desktop's `invoke_capability` Tauri
//! command and the MCP server's tools/call: one registry, three surfaces.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde_json::{json, Value};
use srelens_capability::CapabilityError;

use crate::auth::session::UserCtx;
use crate::AppState;

/// Capabilities that are desktop-only in web mode: `k8s.deleteContext` mutates
/// the per-request materialized kubeconfig, which is rebuilt from the database
/// on every env refresh, so its effect would silently vanish. Blocked until a
/// database-backed context edit exists.
pub const WEB_DENIED_CAPABILITIES: &[&str] = &["k8s.deleteContext"];

/// Invoke a capability by id. The request body is the capability's input JSON;
/// an empty body means null input. Unknown id → 404, invalid input (or a body
/// that isn't JSON) → 400, handler failure (cluster unreachable, RBAC denial)
/// → 502. Error bodies are `{"error": "<message>"}`.
///
/// Non-empty bodies must carry `Content-Type: application/json` (415 otherwise).
/// This isn't just input validation: a cross-origin `fetch` with a `text/plain`
/// body is a CORS "simple request", which browsers send without a preflight —
/// so without this check any website could invoke capabilities (including
/// destructive ones) against this loopback server. Requiring the JSON content
/// type forces the browser to preflight, and a cross-origin preflight fails.
pub async fn invoke_capability(
    State(state): State<AppState>,
    Extension(user): Extension<UserCtx>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    body: Bytes,
) -> Response {
    if WEB_DENIED_CAPABILITIES.contains(&id.as_str()) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "capability not available in web mode",
        );
    }

    if !body.is_empty() {
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
            return error_response(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "content-type must be application/json",
            );
        }
    }

    let input: Value = if body.is_empty() {
        Value::Null
    } else {
        match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(e) => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    &format!("request body is not valid JSON: {e}"),
                );
            }
        }
    };

    let env = match state
        .user_envs
        .env_for(&state.db, &state.master_key, user.user_id)
        .await
    {
        Ok(env) => env,
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("failed to build user environment: {e}"),
            );
        }
    };

    // Destructive/mutating capabilities must not silently run with a null
    // ("no-op default") input — force the caller to supply an explicit body.
    if input.is_null() {
        match env.registry.get(&id) {
            Some(cap) if !cap.annotations.read_only => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    "input required for non-read-only capability",
                );
            }
            _ => {}
        }
    }

    match env.registry.invoke(&id, input).await {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(e) => {
            let status = match &e {
                CapabilityError::NotFound(_) => StatusCode::NOT_FOUND,
                CapabilityError::InvalidInput(_) => StatusCode::BAD_REQUEST,
                CapabilityError::Handler(_) => StatusCode::BAD_GATEWAY,
            };
            error_response(status, &e.to_string())
        }
    }
}

fn error_response(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

#[cfg(test)]
mod tests {
    use crate::{router, AppState};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};
    use serde_json::{json, Value};
    use srelens_capability::{Annotations, Capability, CapabilityError, Registry};
    use std::sync::Arc;
    use tower::ServiceExt; // oneshot

    #[derive(Deserialize, JsonSchema)]
    struct AddIn {
        a: i64,
        b: i64,
    }
    #[derive(Serialize, JsonSchema)]
    struct AddOut {
        sum: i64,
    }

    async fn state() -> AppState {
        let mut reg = Registry::new();
        reg.register(Capability::read_only(
            "echo",
            "echoes input",
            |v| async move { Ok(json!({ "echo": v })) },
        ));
        reg.register(Capability::typed::<AddIn, AddOut, _, _>(
            "math.add",
            "adds",
            Annotations::READ_ONLY,
            |i| async move { Ok(AddOut { sum: i.a + i.b }) },
        ));
        reg.register(Capability::read_only("boom", "always fails", |_| async {
            Err(CapabilityError::Handler("cluster unreachable".into()))
        }));
        reg.register(Capability::typed::<AddIn, AddOut, _, _>(
            "danger.add",
            "non-read-only add",
            Annotations::DESTRUCTIVE,
            |i| async move { Ok(AddOut { sum: i.a + i.b }) },
        ));
        AppState::for_tests(Arc::new(reg)).await
    }

    async fn authed_headers(state: &AppState) -> (String, String) {
        let user = state
            .db
            .upsert_user("test-iss", "test-sub", "t@x", "T", 1)
            .await
            .unwrap();
        let token = state
            .db
            .create_session(user.id, crate::unix_now())
            .await
            .unwrap();
        (format!("srelens_session={token}"), "1".to_string())
    }

    async fn post(path: &str, body: Body) -> (StatusCode, Value) {
        let state = state().await;
        let (cookie, csrf) = authed_headers(&state).await;
        let resp = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .header("content-type", "application/json")
                    .header("cookie", cookie)
                    .header("x-srelens-csrf", csrf)
                    .body(body)
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .unwrap();
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        (status, value)
    }

    #[tokio::test]
    async fn dispatches_capability_and_returns_output() {
        let (status, body) = post("/api/capability/echo", Body::from("\"hi\"")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!({ "echo": "hi" }));
    }

    #[tokio::test]
    async fn empty_body_means_null_input() {
        let (status, body) = post("/api/capability/echo", Body::empty()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!({ "echo": null }));
    }

    #[tokio::test]
    async fn unknown_capability_is_404() {
        let (status, body) = post("/api/capability/nope", Body::empty()).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"], json!("capability not found: nope"));
    }

    #[tokio::test]
    async fn invalid_input_is_400() {
        let (status, body) = post("/api/capability/math.add", Body::from("{\"a\":\"x\"}")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"]
            .as_str()
            .unwrap()
            .starts_with("invalid input:"));
    }

    #[tokio::test]
    async fn malformed_body_json_is_400() {
        let (status, body) = post("/api/capability/echo", Body::from("{not json")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"]
            .as_str()
            .unwrap()
            .starts_with("request body is not valid JSON"));
    }

    #[tokio::test]
    async fn handler_failure_is_502() {
        let (status, body) = post("/api/capability/boom", Body::empty()).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert_eq!(body["error"], json!("handler error: cluster unreachable"));
    }

    #[tokio::test]
    async fn non_json_content_type_is_415() {
        let state = state().await;
        let (cookie, csrf) = authed_headers(&state).await;
        let resp = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/capability/echo")
                    .header("content-type", "text/plain")
                    .header("cookie", cookie)
                    .header("x-srelens-csrf", csrf)
                    .body(Body::from("\"hi\""))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
        let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .unwrap();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            v["error"],
            serde_json::json!("content-type must be application/json")
        );
    }

    #[tokio::test]
    async fn missing_content_type_with_body_is_415() {
        let state = state().await;
        let (cookie, csrf) = authed_headers(&state).await;
        let resp = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/capability/echo")
                    .header("cookie", cookie)
                    .header("x-srelens-csrf", csrf)
                    .body(Body::from("\"hi\""))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    #[tokio::test]
    async fn empty_body_without_content_type_is_allowed() {
        let state = state().await;
        let (cookie, csrf) = authed_headers(&state).await;
        let resp = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/capability/echo")
                    .header("cookie", cookie)
                    .header("x-srelens-csrf", csrf)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn destructive_capability_rejects_null_input() {
        let (status, body) = post("/api/capability/danger.add", Body::empty()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body["error"],
            json!("input required for non-read-only capability")
        );
    }

    #[tokio::test]
    async fn read_only_capability_still_accepts_null_input() {
        let (status, _) = post("/api/capability/echo", Body::empty()).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn web_denied_capability_is_rejected() {
        // k8s.deleteContext isn't even registered in the test registry, but
        // the deny-list must short-circuit with 400 (not 404) before dispatch.
        let (status, body) = post("/api/capability/k8s.deleteContext", Body::empty()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], json!("capability not available in web mode"));
    }

    #[tokio::test]
    async fn capability_requires_session() {
        let resp = router(state().await)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/capability/echo")
                    .header("x-srelens-csrf", "1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
