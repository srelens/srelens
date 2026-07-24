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
pub mod config;
pub mod crypto;
pub mod db;

/// Shared handler state.
#[derive(Clone)]
pub struct AppState {
    pub registry: Arc<Registry>,
}

/// Server launch configuration.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub addr: SocketAddr,
    pub data_dir: std::path::PathBuf,
}

/// Build the full application router. Named routes win over the asset
/// fallback, so `/api/*` and the health probes are never shadowed.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(|| async { "ok" }))
        .route(
            "/api/capability/:id",
            axum::routing::post(api::invoke_capability),
        )
        .fallback(get(assets::serve_asset))
        .with_state(state)
}

/// Bind `config.addr` and serve until the process exits.
pub async fn serve(registry: Arc<Registry>, config: ServerConfig) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(config.addr).await?;
    serve_on(registry, listener).await
}

/// Serve on an already-bound listener (lets callers surface bind errors
/// synchronously and lets tests pick port 0).
pub async fn serve_on(
    registry: Arc<Registry>,
    listener: tokio::net::TcpListener,
) -> std::io::Result<()> {
    let state = AppState { registry };
    axum::serve(listener, router(state)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt; // oneshot

    fn state() -> AppState {
        AppState {
            registry: Arc::new(Registry::new()),
        }
    }

    #[tokio::test]
    async fn healthz_and_readyz_respond_ok() {
        for path in ["/healthz", "/readyz"] {
            let resp = router(state())
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
        let resp = router(state())
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
}
