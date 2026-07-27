//! Cluster login-flow HTTP routes: `GET /auth/cluster/login` and
//! `GET /auth/cluster/callback`. Mirrors the app-login flow in
//! `auth/routes.rs` (state/binder-cookie/one-shot pending-store pattern), but
//! requires an already-authenticated srelens session (this signs an
//! EXISTING user into one of THEIR clusters, it doesn't establish identity)
//! and stores the resulting tokens per (user, oidc_key) rather than minting a
//! session.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::{Query, State};
use axum::http::header::{LOCATION, SET_COOKIE};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::session;
use crate::AppState;

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(serde_json::json!({ "error": message }))).into_response()
}

fn sha256_hex(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

const PENDING_TTL: Duration = Duration::from_secs(600);
const PENDING_CAP: usize = 1000;

/// A cluster login in flight, held between `/auth/cluster/login` and
/// `/auth/cluster/callback`, keyed by the OIDC `state`.
pub struct PendingClusterLogin {
    pub user_id: i64,
    pub oidc_key: String,
    pub pkce_verifier: String,
    pub binder_hash: String,
    created_at: Instant,
}

/// In-flight cluster logins keyed by OIDC `state`. Entries expire after 10
/// minutes; the map is capped so a login-spam loop can't grow memory
/// unboundedly. Mirrors `auth::idp::PendingLogins`.
#[derive(Default)]
pub struct PendingClusterLogins {
    inner: Mutex<HashMap<String, PendingClusterLogin>>,
}

impl PendingClusterLogins {
    /// Returns false (and stores nothing) when the cap is reached.
    pub fn insert(&self, state: String, pending: PendingClusterLogin) -> bool {
        let mut map = self.inner.lock().unwrap();
        map.retain(|_, p| p.created_at.elapsed() < PENDING_TTL);
        if map.len() >= PENDING_CAP {
            return false;
        }
        map.insert(state, pending);
        true
    }

    /// One-shot: a state can only be redeemed once, and only within the TTL.
    pub fn take(&self, state: &str) -> Option<PendingClusterLogin> {
        let mut map = self.inner.lock().unwrap();
        let pending = map.remove(state)?;
        if pending.created_at.elapsed() >= PENDING_TTL {
            return None;
        }
        Some(pending)
    }
}

#[derive(Deserialize)]
pub struct LoginParams {
    pub key: String,
}

/// GET /auth/cluster/login?key=<oidc_key> — resolve the caller's cluster OIDC
/// config for `key`, begin the authorization-code + PKCE flow against that
/// cluster's IdP, stash the pending flow, bind it to this browser with a
/// short-lived cookie, and bounce to the IdP. Gated by the session cookie
/// (browser navigation, no CSRF header — like `/pf`).
pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<LoginParams>,
) -> Response {
    let user_id = match crate::pf_proxy::authorize_pf(&headers, &state.db, crate::unix_now()).await
    {
        Ok(id) => id,
        Err(status) => return status.into_response(),
    };

    let env = match state
        .user_envs
        .env_for(&state.db, &state.master_key, user_id)
        .await
    {
        Ok(env) => env,
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    };
    let Some(cfg) = env.oidc_registry.config_for_key(&params.key) else {
        return error(StatusCode::NOT_FOUND, "unknown cluster");
    };

    let redirect_uri = format!("{}/auth/cluster/callback", state.auth.public_url);
    let begin = match crate::cluster_oidc::begin_login(&cfg, &redirect_uri).await {
        Ok(b) => b,
        Err(e) => return error(StatusCode::BAD_GATEWAY, &e),
    };

    let mut binder_bytes = [0u8; 32];
    if let Err(e) = getrandom::getrandom(&mut binder_bytes) {
        return error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
    }
    let binder = hex::encode(binder_bytes);
    let binder_hash = sha256_hex(&binder);

    if !state.pending_cluster.insert(
        begin.state.clone(),
        PendingClusterLogin {
            user_id,
            oidc_key: params.key,
            pkce_verifier: begin.pkce_verifier,
            binder_hash,
            created_at: Instant::now(),
        },
    ) {
        return error(StatusCode::TOO_MANY_REQUESTS, "too many logins in flight");
    }

    // The auth URL comes from a user-supplied issuer's discovery document, so
    // an unparseable value must not panic the handler — map it to 502.
    let location = match begin.auth_url.parse() {
        Ok(loc) => loc,
        Err(_) => return error(StatusCode::BAD_GATEWAY, "invalid IdP authorization URL"),
    };
    let mut headers = HeaderMap::new();
    headers.insert(LOCATION, location);
    headers.insert(
        SET_COOKIE,
        session::login_cookie(&binder, state.auth.cookie_secure())
            .parse()
            .expect("valid cookie"),
    );
    (StatusCode::FOUND, headers).into_response()
}

#[derive(Deserialize)]
pub struct CallbackParams {
    pub code: Option<String>,
    pub state: Option<String>,
}

/// Append the clear-binder-cookie header to a response. Called on every
/// /auth/cluster/callback exit path (once the pending state has been taken)
/// so a stale binder cookie never lingers in the browser.
fn with_cleared_binder(resp: Response, secure: bool) -> Response {
    let mut resp = resp;
    resp.headers_mut().append(
        SET_COOKIE,
        session::clear_login_cookie(secure)
            .parse()
            .expect("valid cookie"),
    );
    resp
}

/// GET /auth/cluster/callback?code&state — verify the caller, the binder
/// cookie, and pending-state ownership, exchange the code, persist the
/// tokens, invalidate the user's env (so the resolver picks up the fresh
/// token), and bounce back to `/`.
pub async fn callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<CallbackParams>,
) -> Response {
    let user_id = match crate::pf_proxy::authorize_pf(&headers, &state.db, crate::unix_now()).await
    {
        Ok(id) => id,
        Err(status) => return status.into_response(),
    };

    let (Some(code), Some(oidc_state)) = (params.code, params.state) else {
        return error(StatusCode::BAD_REQUEST, "missing code or state");
    };
    let Some(pending) = state.pending_cluster.take(&oidc_state) else {
        return error(StatusCode::BAD_REQUEST, "unknown or expired state");
    };

    let secure = state.auth.cookie_secure();

    // Ownership: a taken pending state must belong to the caller, not
    // whoever's browser happens to hit the callback URL.
    if pending.user_id != user_id {
        return with_cleared_binder(
            error(
                StatusCode::FORBIDDEN,
                "login was started by a different user",
            ),
            secure,
        );
    }

    let binder_ok = session::cookie_value(&headers, session::LOGIN_COOKIE)
        .map(|cookie| sha256_hex(&cookie) == pending.binder_hash)
        .unwrap_or(false);
    if !binder_ok {
        return with_cleared_binder(
            error(
                StatusCode::BAD_REQUEST,
                "login was started in a different browser",
            ),
            secure,
        );
    }

    let env = match state
        .user_envs
        .env_for(&state.db, &state.master_key, user_id)
        .await
    {
        Ok(env) => env,
        Err(e) => return with_cleared_binder(error(StatusCode::INTERNAL_SERVER_ERROR, &e), secure),
    };
    let Some(cfg) = env.oidc_registry.config_for_key(&pending.oidc_key) else {
        return with_cleared_binder(error(StatusCode::NOT_FOUND, "unknown cluster"), secure);
    };

    let redirect_uri = format!("{}/auth/cluster/callback", state.auth.public_url);
    let refreshed = match crate::cluster_oidc::exchange_code(
        &cfg,
        &redirect_uri,
        &code,
        &pending.pkce_verifier,
        crate::unix_now(),
    )
    .await
    {
        Ok(r) => r,
        Err(e) => return with_cleared_binder(error(StatusCode::BAD_GATEWAY, &e), secure),
    };

    if let Err(e) = state
        .db
        .put_cluster_token(
            user_id,
            &pending.oidc_key,
            &state.master_key,
            &crate::cluster_tokens::StoredToken {
                id_token: refreshed.id_token,
                refresh_token: refreshed.refresh_token,
                expires_at: refreshed.expires_at,
            },
        )
        .await
    {
        return with_cleared_binder(error(StatusCode::INTERNAL_SERVER_ERROR, &e), secure);
    }
    state.user_envs.invalidate(user_id);

    let mut resp_headers = HeaderMap::new();
    resp_headers.insert(LOCATION, "/".parse().expect("valid location"));
    let resp = (StatusCode::FOUND, resp_headers).into_response();
    with_cleared_binder(resp, secure)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{router, AppState};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use srelens_capability::Registry;
    use std::sync::Arc;
    use tower::ServiceExt; // oneshot

    #[test]
    fn pending_cluster_logins_round_trip_and_one_shot() {
        let p = PendingClusterLogins::default();
        assert!(p.insert(
            "s1".into(),
            PendingClusterLogin {
                user_id: 42,
                oidc_key: "k".into(),
                pkce_verifier: "v".into(),
                binder_hash: "h".into(),
                created_at: Instant::now(),
            }
        ));
        let taken = p.take("s1").expect("first take succeeds");
        assert_eq!(taken.user_id, 42);
        assert_eq!(taken.oidc_key, "k");
        assert_eq!(taken.pkce_verifier, "v");
        assert_eq!(taken.binder_hash, "h");

        // One-shot: a second take of the same state fails.
        assert!(p.take("s1").is_none());
        assert!(p.take("unknown").is_none());
    }

    #[test]
    fn pending_cluster_logins_cap_returns_false() {
        let p = PendingClusterLogins::default();
        for i in 0..PENDING_CAP {
            assert!(p.insert(
                format!("s{i}"),
                PendingClusterLogin {
                    user_id: 1,
                    oidc_key: "k".into(),
                    pkce_verifier: "v".into(),
                    binder_hash: "h".into(),
                    created_at: Instant::now(),
                }
            ));
        }
        assert!(!p.insert(
            "over-cap".into(),
            PendingClusterLogin {
                user_id: 1,
                oidc_key: "k".into(),
                pkce_verifier: "v".into(),
                binder_hash: "h".into(),
                created_at: Instant::now(),
            }
        ));
    }

    #[test]
    fn pending_cluster_logins_evicts_expired_entries() {
        let p = PendingClusterLogins::default();
        // Insert one entry whose created_at is already past the TTL.
        p.inner.lock().unwrap().insert(
            "stale".into(),
            PendingClusterLogin {
                user_id: 1,
                oidc_key: "k".into(),
                pkce_verifier: "v".into(),
                binder_hash: "h".into(),
                created_at: Instant::now() - PENDING_TTL - Duration::from_secs(1),
            },
        );
        // take() itself refuses an expired entry...
        assert!(p.take("stale").is_none());
        // ...and a fresh insert's TTL-evict sweep drops expired rows so the
        // map doesn't grow unboundedly with dead entries.
        p.inner.lock().unwrap().insert(
            "stale2".into(),
            PendingClusterLogin {
                user_id: 1,
                oidc_key: "k".into(),
                pkce_verifier: "v".into(),
                binder_hash: "h".into(),
                created_at: Instant::now() - PENDING_TTL - Duration::from_secs(1),
            },
        );
        assert!(p.insert(
            "fresh".into(),
            PendingClusterLogin {
                user_id: 1,
                oidc_key: "k".into(),
                pkce_verifier: "v".into(),
                binder_hash: "h".into(),
                created_at: Instant::now(),
            }
        ));
        assert_eq!(p.inner.lock().unwrap().len(), 1);
    }

    async fn cookie_session(state: &AppState) -> String {
        session_for(state).await.1
    }

    /// Create a user + session, returning `(user_id, session_token)`.
    async fn session_for(state: &AppState) -> (i64, String) {
        let user = state.db.upsert_user("i", "s", "u@x", "U", 1).await.unwrap();
        let token = state
            .db
            .create_session(user.id, crate::unix_now())
            .await
            .unwrap();
        (user.id, token)
    }

    #[tokio::test]
    async fn login_with_unknown_key_is_404() {
        let state = AppState::for_tests(Arc::new(Registry::new())).await;
        let token = cookie_session(&state).await;
        let resp = router(state)
            .oneshot(
                Request::builder()
                    .uri("/auth/cluster/login?key=nope")
                    .header("cookie", format!("srelens_session={token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn login_without_session_cookie_is_unauthorized() {
        let state = AppState::for_tests(Arc::new(Registry::new())).await;
        let resp = router(state)
            .oneshot(
                Request::builder()
                    .uri("/auth/cluster/login?key=nope")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn callback_rejects_unknown_state() {
        let state = AppState::for_tests(Arc::new(Registry::new())).await;
        let token = cookie_session(&state).await;
        let resp = router(state)
            .oneshot(
                Request::builder()
                    .uri("/auth/cluster/callback?code=c&state=nope")
                    .header("cookie", format!("srelens_session={token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn callback_rejects_state_owned_by_a_different_user() {
        let state = AppState::for_tests(Arc::new(Registry::new())).await;
        let token = cookie_session(&state).await;
        // A pending flow that belongs to some other user id.
        assert!(state.pending_cluster.insert(
            "st".into(),
            PendingClusterLogin {
                user_id: 999_999,
                oidc_key: "k".into(),
                pkce_verifier: "v".into(),
                binder_hash: sha256_hex("binder"),
                created_at: Instant::now(),
            }
        ));
        let resp = router(state)
            .oneshot(
                Request::builder()
                    .uri("/auth/cluster/callback?code=c&state=st")
                    .header(
                        "cookie",
                        format!("srelens_session={token}; srelens_login=binder"),
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn callback_rejects_wrong_or_missing_binder_cookie() {
        // The pending flow belongs to the CALLER (ownership passes), but the
        // browser presents a binder cookie that doesn't match the stored hash
        // (or none at all) → 400, so an attacker who fixated a state can't
        // complete the flow from a different browser.
        for binder_cookie in ["; srelens_login=wrong", ""] {
            let state = AppState::for_tests(Arc::new(Registry::new())).await;
            let (user_id, token) = session_for(&state).await;
            assert!(state.pending_cluster.insert(
                "st".into(),
                PendingClusterLogin {
                    user_id,
                    oidc_key: "k".into(),
                    pkce_verifier: "v".into(),
                    binder_hash: sha256_hex("the-real-binder"),
                    created_at: Instant::now(),
                }
            ));
            let resp = router(state)
                .oneshot(
                    Request::builder()
                        .uri("/auth/cluster/callback?code=c&state=st")
                        .header("cookie", format!("srelens_session={token}{binder_cookie}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                resp.status(),
                StatusCode::BAD_REQUEST,
                "binder cookie {binder_cookie:?} must be rejected",
            );
        }
    }
}
