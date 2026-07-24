//! Login/logout HTTP routes. All flow logic runs against the
//! `IdentityProvider` trait; see auth/oidc.rs for the real OIDC adapter.

use axum::extract::{Query, State};
use axum::http::header::LOCATION;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;

use super::session;
use crate::AppState;

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(serde_json::json!({ "error": message }))).into_response()
}

/// GET /auth/login — stash the per-login secrets and bounce to the IdP.
pub async fn login(State(state): State<AppState>) -> Response {
    let begin = match state.idp.begin_login() {
        Ok(b) => b,
        Err(e) => return error(StatusCode::SERVICE_UNAVAILABLE, &e),
    };
    if !state
        .pending
        .insert(begin.state.clone(), begin.nonce, begin.pkce_verifier)
    {
        return error(StatusCode::TOO_MANY_REQUESTS, "too many logins in flight");
    }
    let mut headers = HeaderMap::new();
    headers.insert(LOCATION, begin.auth_url.parse().expect("valid auth url"));
    (StatusCode::FOUND, headers).into_response()
}

#[derive(Deserialize)]
pub struct CallbackParams {
    pub code: Option<String>,
    pub state: Option<String>,
}

/// GET /auth/callback — verify the code, gate by email domain, mint a session.
pub async fn callback(
    State(state): State<AppState>,
    Query(params): Query<CallbackParams>,
) -> Response {
    let (Some(code), Some(oidc_state)) = (params.code, params.state) else {
        return error(StatusCode::BAD_REQUEST, "missing code or state");
    };
    let Some((nonce, verifier)) = state.pending.take(&oidc_state) else {
        return error(StatusCode::BAD_REQUEST, "unknown or expired login state");
    };
    let claims = match state.idp.complete_login(&code, &nonce, &verifier).await {
        Ok(c) => c,
        Err(e) => return error(StatusCode::UNAUTHORIZED, &format!("login failed: {e}")),
    };
    if !state.auth.email_domain_allowed(&claims.email) {
        return error(StatusCode::FORBIDDEN, "email domain not allowed");
    }
    finish_login(
        &state,
        &claims.iss,
        &claims.sub,
        &claims.email,
        &claims.display_name,
    )
    .await
}

/// POST /auth/logout — revoke the session (if any) and clear the cookie.
/// Requires the CSRF header (it is a state-changing endpoint outside /api).
pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let has_csrf = headers
        .get(session::CSRF_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false);
    if !has_csrf {
        return error(StatusCode::FORBIDDEN, "missing csrf header");
    }
    if let Some(token) = session::cookie_value(&headers, session::COOKIE_NAME) {
        if let Err(e) = state.db.revoke_session(&token).await {
            return error(StatusCode::INTERNAL_SERVER_ERROR, &e);
        }
    }
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::SET_COOKIE,
        session::clear_cookie(state.auth.cookie_secure())
            .parse()
            .expect("valid cookie"),
    );
    (StatusCode::NO_CONTENT, headers).into_response()
}

/// POST /auth/dev-login — local-development login, only when
/// `SRELENS_DEV_LOGIN` is configured. Creates/refreshes the dev user and
/// mints a session, no IdP involved.
pub async fn dev_login(State(state): State<AppState>) -> Response {
    let Some(email) = state.auth.dev_login.clone() else {
        return error(StatusCode::FORBIDDEN, "dev login is not enabled");
    };
    finish_login(&state, "dev", &email, &email, "dev user").await
}

async fn finish_login(
    state: &AppState,
    iss: &str,
    sub: &str,
    email: &str,
    display_name: &str,
) -> Response {
    let now = crate::unix_now();
    let user = match state
        .db
        .upsert_user(iss, sub, email, display_name, now)
        .await
    {
        Ok(u) => u,
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    };
    let token = match state.db.create_session(user.id, now).await {
        Ok(t) => t,
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    };
    let mut headers = session::session_headers(&token, state.auth.cookie_secure());
    headers.insert(LOCATION, "/".parse().expect("valid location"));
    (StatusCode::FOUND, headers).into_response()
}

#[cfg(test)]
mod tests {
    use crate::{router, AppState};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use srelens_capability::Registry;
    use std::sync::Arc;
    use tower::ServiceExt; // oneshot

    async fn get(state: AppState, uri: &str) -> axum::response::Response {
        router(state)
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    fn location(resp: &axum::response::Response) -> String {
        resp.headers()["location"].to_str().unwrap().to_string()
    }

    fn set_cookie_token(resp: &axum::response::Response) -> String {
        let sc = resp.headers()["set-cookie"].to_str().unwrap();
        sc.split(';')
            .next()
            .unwrap()
            .split('=')
            .nth(1)
            .unwrap()
            .to_string()
    }

    #[tokio::test]
    async fn full_login_flow_with_fake_idp() {
        let state = AppState::for_tests(Arc::new(Registry::new())).await;

        // /auth/login redirects to the IdP with a state we can extract.
        let resp = get(state.clone(), "/auth/login").await;
        assert_eq!(resp.status(), StatusCode::FOUND);
        let auth_url = location(&resp);
        let oidc_state = auth_url.split("state=").nth(1).unwrap().to_string();

        // Callback with a FakeIdp-accepted code mints a session cookie.
        let resp = get(
            state.clone(),
            &format!("/auth/callback?code=ok:alice:alice@example.com&state={oidc_state}"),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::FOUND);
        assert_eq!(location(&resp), "/");
        let token = set_cookie_token(&resp);
        assert!(!token.is_empty());

        // The cookie works against /api/me.
        let resp = router(state.clone())
            .oneshot(
                Request::builder()
                    .uri("/api/me")
                    .header("x-srelens-csrf", "1")
                    .header("cookie", format!("srelens_session={token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Replaying the same state fails (one-shot).
        let resp = get(
            state.clone(),
            &format!("/auth/callback?code=ok:alice:alice@example.com&state={oidc_state}"),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn callback_rejects_unknown_state_and_bad_code() {
        let state = AppState::for_tests(Arc::new(Registry::new())).await;
        let resp = get(state.clone(), "/auth/callback?code=ok:a:a@x&state=nope").await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let resp = get(state.clone(), "/auth/login").await;
        let oidc_state = location(&resp).split("state=").nth(1).unwrap().to_string();
        let resp = get(
            state.clone(),
            &format!("/auth/callback?code=garbage&state={oidc_state}"),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn domain_gate_blocks_disallowed_email() {
        let mut state = AppState::for_tests(Arc::new(Registry::new())).await;
        let mut cfg = (*state.auth).clone();
        cfg.allowed_email_domains = vec!["corp.io".into()];
        state.auth = Arc::new(cfg);

        let resp = get(state.clone(), "/auth/login").await;
        let oidc_state = location(&resp).split("state=").nth(1).unwrap().to_string();
        let resp = get(
            state.clone(),
            &format!("/auth/callback?code=ok:eve:eve@evil.com&state={oidc_state}"),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn logout_requires_csrf_and_revokes() {
        let state = AppState::for_tests(Arc::new(Registry::new())).await;
        let user = state.db.upsert_user("i", "s", "u@x", "U", 1).await.unwrap();
        let token = state
            .db
            .create_session(user.id, crate::unix_now())
            .await
            .unwrap();

        // Without CSRF header → 403.
        let resp = router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/auth/logout")
                    .header("cookie", format!("srelens_session={token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);

        // With CSRF → 204 and the session no longer validates.
        let resp = router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/auth/logout")
                    .header("x-srelens-csrf", "1")
                    .header("cookie", format!("srelens_session={token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        assert!(state
            .db
            .validate_session(&token, crate::unix_now())
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn dev_login_gated_by_config() {
        // Enabled in for_tests → mints a session.
        let state = AppState::for_tests(Arc::new(Registry::new())).await;
        let resp = router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/auth/dev-login")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FOUND);

        // Disabled → 403.
        let mut state2 = AppState::for_tests(Arc::new(Registry::new())).await;
        let mut cfg = (*state2.auth).clone();
        cfg.dev_login = None;
        cfg.oidc = Some(crate::auth::OidcSettings {
            issuer: "https://idp".into(),
            client_id: "c".into(),
            client_secret: "s".into(),
        });
        state2.auth = Arc::new(cfg);
        let resp = router(state2)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/auth/dev-login")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }
}
