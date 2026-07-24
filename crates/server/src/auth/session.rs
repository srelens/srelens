//! Session-cookie middleware and helpers. Auth is a server-side session (an
//! opaque token in an HttpOnly SameSite=Lax cookie) plus a custom-header CSRF
//! requirement: browsers cannot attach `X-Srelens-Csrf` cross-origin without
//! CORS approval, which this server never grants.

use axum::extract::{Request, State};
use axum::http::header::{COOKIE, SET_COOKIE};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Serialize;

use crate::stores::SESSION_ABSOLUTE_TTL_SECS;
use crate::AppState;

pub const COOKIE_NAME: &str = "srelens_session";
/// Compared case-insensitively by the HTTP layer; axum lowercases names.
pub const CSRF_HEADER: &str = "x-srelens-csrf";

pub const LOGIN_COOKIE: &str = "srelens_login";

/// The authenticated caller, attached to the request by [`require_session`].
#[derive(Debug, Clone, Serialize)]
pub struct UserCtx {
    pub user_id: i64,
    pub email: String,
    pub display_name: String,
}

/// Extract a cookie's value from the `Cookie` header (there may be several
/// cookies, `; `-separated, and several Cookie headers).
pub fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    for header in headers.get_all(COOKIE) {
        let Ok(raw) = header.to_str() else { continue };
        for pair in raw.split(';') {
            let mut parts = pair.trim().splitn(2, '=');
            if parts.next() == Some(name) {
                return parts.next().map(str::to_string);
            }
        }
    }
    None
}

/// Build the `Set-Cookie` value for a fresh session.
pub fn set_cookie(token: &str, secure: bool) -> String {
    let secure_attr = if secure { "; Secure" } else { "" };
    format!(
        "{COOKIE_NAME}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={SESSION_ABSOLUTE_TTL_SECS}{secure_attr}"
    )
}

/// Build the `Set-Cookie` value that clears the session cookie.
pub fn clear_cookie(secure: bool) -> String {
    let secure_attr = if secure { "; Secure" } else { "" };
    format!("{COOKIE_NAME}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{secure_attr}")
}

/// Short-lived cookie binding an in-flight OIDC login to this browser: the
/// callback only completes when the cookie's hash matches the pending state's
/// stored hash, so a forwarded callback URL can't log a victim into an
/// attacker's account.
pub fn login_cookie(binder: &str, secure: bool) -> String {
    let secure_attr = if secure { "; Secure" } else { "" };
    format!("{LOGIN_COOKIE}={binder}; Path=/auth; HttpOnly; SameSite=Lax; Max-Age=600{secure_attr}")
}

pub fn clear_login_cookie(secure: bool) -> String {
    let secure_attr = if secure { "; Secure" } else { "" };
    format!("{LOGIN_COOKIE}=; Path=/auth; HttpOnly; SameSite=Lax; Max-Age=0{secure_attr}")
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(serde_json::json!({ "error": message }))).into_response()
}

/// Middleware for `/api/*`: requires the CSRF header and a valid session, and
/// attaches [`UserCtx`] for handlers.
pub async fn require_session(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Response {
    let has_csrf = req
        .headers()
        .get(CSRF_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false);
    if !has_csrf {
        return error(StatusCode::FORBIDDEN, "missing csrf header");
    }
    let Some(token) = cookie_value(req.headers(), COOKIE_NAME) else {
        return error(StatusCode::UNAUTHORIZED, "unauthenticated");
    };
    let user = match state.db.validate_session(&token, crate::unix_now()).await {
        Ok(Some(user)) => user,
        Ok(None) => return error(StatusCode::UNAUTHORIZED, "unauthenticated"),
        Err(e) => {
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("session lookup failed: {e}"),
            )
        }
    };
    req.extensions_mut().insert(UserCtx {
        user_id: user.id,
        email: user.email,
        display_name: user.display_name,
    });
    next.run(req).await
}

/// GET /api/me — the authenticated identity (also the frontend's session probe).
pub async fn me(Extension(user): Extension<UserCtx>) -> Json<UserCtx> {
    Json(user)
}

/// Helper used by auth routes: response headers carrying a fresh session cookie.
pub fn session_headers(token: &str, secure: bool) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        SET_COOKIE,
        set_cookie(token, secure).parse().expect("valid cookie"),
    );
    headers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_value_parses_multi_cookie_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(COOKIE, "a=1; srelens_session=tok123; b=2".parse().unwrap());
        assert_eq!(cookie_value(&headers, COOKIE_NAME), Some("tok123".into()));
        assert_eq!(cookie_value(&headers, "missing"), None);
    }

    #[test]
    fn set_and_clear_cookie_attributes() {
        let set = set_cookie("tok", false);
        assert!(set.starts_with("srelens_session=tok; "));
        assert!(set.contains("HttpOnly"));
        assert!(set.contains("SameSite=Lax"));
        assert!(set.contains("Max-Age=604800"));
        assert!(!set.contains("Secure"));
        assert!(set_cookie("tok", true).contains("; Secure"));
        let clear = clear_cookie(false);
        assert!(clear.contains("Max-Age=0"));
    }

    #[test]
    fn login_cookie_scoped_to_auth_path_and_short_lived() {
        let set = login_cookie("binder123", false);
        assert!(set.starts_with("srelens_login=binder123; "));
        assert!(set.contains("Path=/auth"));
        assert!(set.contains("HttpOnly"));
        assert!(set.contains("SameSite=Lax"));
        assert!(set.contains("Max-Age=600"));
        assert!(!set.contains("Secure"));
        assert!(login_cookie("b", true).contains("; Secure"));

        let clear = clear_login_cookie(false);
        assert!(clear.starts_with("srelens_login=; "));
        assert!(clear.contains("Path=/auth"));
        assert!(clear.contains("Max-Age=0"));
    }
}
