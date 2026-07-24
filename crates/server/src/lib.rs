//! srelens web server foundation: serves the built frontend and exposes the
//! capability registry over HTTP. Auth, sessions, and streaming arrive in
//! later plans — until auth lands, bind loopback only.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::routing::get;
use axum::Router;
use srelens_capability::Registry;

pub mod api;
pub mod assets;
pub mod auth;
pub mod config;
pub mod crypto;
pub mod db;
pub mod stores;

/// Current unix time in seconds — the single clock read for the HTTP edge.
pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Shared handler state.
#[derive(Clone)]
pub struct AppState {
    pub registry: Arc<Registry>,
    pub db: db::Db,
    pub master_key: Arc<crypto::MasterKey>,
    pub auth: Arc<auth::AuthConfig>,
}

impl AppState {
    /// Test-only convenience: in-memory database, fixed master key, dev auth.
    pub async fn for_tests(registry: Arc<Registry>) -> AppState {
        AppState {
            registry,
            db: db::Db::open_in_memory().await.expect("in-memory db"),
            master_key: Arc::new(
                crypto::MasterKey::from_hex(&"ab".repeat(32)).expect("test key"),
            ),
            auth: Arc::new(auth::AuthConfig {
                public_url: "http://127.0.0.1:8080".into(),
                allowed_email_domains: vec![],
                dev_login: Some("dev@example.com".into()),
                oidc: None,
            }),
        }
    }
}

/// Server launch configuration.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub addr: SocketAddr,
    pub data_dir: std::path::PathBuf,
}

/// Build the full application router. `/api/*` requires a session + CSRF
/// header; auth routes, health probes, and static assets are open. Named
/// routes win over the asset fallback.
pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/api/capability/:id", axum::routing::post(api::invoke_capability))
        .route("/api/me", get(auth::session::me))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::session::require_session,
        ));
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(|| async { "ok" }))
        .merge(api)
        .fallback(get(assets::serve_asset))
        .with_state(state)
}

/// Bind `config.addr` and serve until the process exits. Initializes the data
/// directory, master key, and database first, so startup fails fast with a
/// clear message instead of at first request.
pub async fn serve(registry: Arc<Registry>, config: ServerConfig) -> Result<(), String> {
    let auth_config = auth::AuthConfig::from_env(|k| std::env::var(k).ok())?;
    config::ensure_data_dir(&config.data_dir)
        .map_err(|e| format!("create data dir {}: {e}", config.data_dir.display()))?;
    let env_key = std::env::var("SRELENS_MASTER_KEY").ok();
    let master_key = crypto::MasterKey::load_or_generate(
        env_key.as_deref(),
        &config.data_dir.join("master.key"),
    )?;
    let db = db::Db::open(&config.data_dir.join("srelens.db")).await?;
    let state = AppState {
        registry,
        db,
        master_key: Arc::new(master_key),
        auth: Arc::new(auth_config),
    };
    let listener = tokio::net::TcpListener::bind(config.addr)
        .await
        .map_err(|e| format!("bind {}: {e}", config.addr))?;
    serve_on(state, listener).await.map_err(|e| e.to_string())
}

/// Serve on an already-bound listener with a fully-built state.
pub async fn serve_on(state: AppState, listener: tokio::net::TcpListener) -> std::io::Result<()> {
    axum::serve(listener, router(state)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt; // oneshot

    async fn state() -> AppState {
        AppState::for_tests(Arc::new(Registry::new())).await
    }

    #[tokio::test]
    async fn healthz_and_readyz_respond_ok() {
        for path in ["/healthz", "/readyz"] {
            let resp = router(state().await)
                .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK, "{path}");
            let bytes = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
            assert_eq!(&bytes[..], b"ok", "{path}");
        }
    }

    #[tokio::test]
    async fn health_route_wins_over_asset_fallback() {
        let resp = router(state().await)
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        assert_eq!(&bytes[..], b"ok");
    }

    #[tokio::test]
    async fn api_requires_csrf_and_session() {
        let state = state().await;
        // No CSRF header → 403.
        let resp = router(state.clone())
            .oneshot(Request::builder().uri("/api/me").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        // CSRF but no cookie → 401.
        let resp = router(state.clone())
            .oneshot(
                Request::builder()
                    .uri("/api/me")
                    .header("x-srelens-csrf", "1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        // Real session → 200 with identity.
        let user = state.db.upsert_user("i", "s", "u@x", "U", 1).await.unwrap();
        let token = state.db.create_session(user.id, crate::unix_now()).await.unwrap();
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
        let bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["email"], serde_json::json!("u@x"));
    }
}
