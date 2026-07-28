//! srelens web server foundation: serves the built frontend and exposes the
//! capability registry over HTTP. Requests to /api require a session (OIDC or dev login);
//! streaming arrives in a later plan.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::routing::get;
use axum::Router;
use srelens_capability::Registry;
use srelens_kube::client_cache::ClientCache;

pub mod api;
pub mod api_clusters;
pub mod api_command;
pub mod api_kubeconfigs;
pub mod assets;
pub mod auth;
pub mod cluster_auth_resolver;
pub mod cluster_oidc;
pub mod cluster_registry;
pub mod cluster_tokens;
pub mod config;
pub mod crypto;
pub mod db;
pub mod oidc_provider;
pub mod pf_proxy;
pub mod stores;
pub mod streams;
pub mod users;
pub mod ws;

/// Builds a full capability registry for a (cache, kubeconfig-paths) pair.
/// The desktop binary supplies this from its capability assembly, keeping
/// this crate assembly-agnostic.
pub type RegistryFactory =
    Arc<dyn Fn(Arc<ClientCache>, Vec<std::path::PathBuf>) -> Registry + Send + Sync>;

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
    pub user_envs: Arc<users::UserEnvs>,
    pub db: db::Db,
    pub master_key: Arc<crypto::MasterKey>,
    pub auth: Arc<auth::AuthConfig>,
    pub idp: Arc<dyn auth::idp::IdentityProvider>,
    pub pending: Arc<auth::idp::PendingLogins>,
    pub pending_cluster: Arc<auth::cluster_routes::PendingClusterLogins>,
    pub ws_hub: Arc<ws::hub::WsHub>,
    pub pf_client: pf_proxy::HttpProxyClient,
}

impl AppState {
    /// Test-only convenience: in-memory database, fixed master key, dev auth.
    /// The factory ignores the (cache, paths) it's given and always returns a
    /// clone of `registry` — tests don't materialize real kubeconfigs.
    pub async fn for_tests(registry: Arc<Registry>) -> AppState {
        let factory: RegistryFactory = Arc::new(move |_cache, _paths| (*registry).clone());
        let mut bytes = [0u8; 8];
        getrandom::getrandom(&mut bytes).expect("random");
        let data_dir =
            std::env::temp_dir().join(format!("srelens-state-test-{}", hex::encode(bytes)));
        std::fs::create_dir_all(&data_dir).expect("test data dir");
        AppState {
            user_envs: Arc::new(users::UserEnvs::new(
                factory,
                data_dir,
                "http://127.0.0.1:8080".into(),
            )),
            db: db::Db::open_in_memory().await.expect("in-memory db"),
            master_key: Arc::new(crypto::MasterKey::from_hex(&"ab".repeat(32)).expect("test key")),
            auth: Arc::new(auth::AuthConfig {
                public_url: "http://127.0.0.1:8080".into(),
                allowed_email_domains: vec![],
                dev_login: Some("dev@example.com".into()),
                oidc: None,
            }),
            idp: Arc::new(auth::idp::FakeIdp),
            pending: Arc::new(auth::idp::PendingLogins::default()),
            pending_cluster: Arc::new(auth::cluster_routes::PendingClusterLogins::default()),
            ws_hub: Arc::new(ws::hub::WsHub::new()),
            pf_client: pf_proxy::client(),
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
        .route(
            "/api/capability/:id",
            axum::routing::post(api::invoke_capability),
        )
        .route("/api/me", get(auth::session::me))
        .route(
            "/api/kubeconfigs",
            get(api_kubeconfigs::list).post(api_kubeconfigs::put),
        )
        .route(
            "/api/kubeconfigs/:id",
            axum::routing::delete(api_kubeconfigs::delete),
        )
        .route(
            "/api/clusters",
            get(api_clusters::list).post(api_clusters::create),
        )
        .route(
            "/api/clusters/:key/logout",
            axum::routing::post(api_clusters::logout),
        )
        .route(
            "/api/command/:command",
            axum::routing::post(api_command::dispatch),
        )
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::session::require_session,
        ));
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(|| async { "ok" }))
        .route("/auth/login", get(auth::routes::login))
        .route("/auth/callback", get(auth::routes::callback))
        .route("/auth/logout", axum::routing::post(auth::routes::logout))
        .route(
            "/auth/dev-login",
            axum::routing::post(auth::routes::dev_login),
        )
        .route("/auth/cluster/login", get(auth::cluster_routes::login))
        .route(
            "/auth/cluster/callback",
            get(auth::cluster_routes::callback),
        )
        .route("/api/ws", get(ws::route::ws_handler))
        .route(
            "/pf/:id",
            get(pf_proxy::proxy)
                .post(pf_proxy::proxy)
                .put(pf_proxy::proxy)
                .delete(pf_proxy::proxy)
                .patch(pf_proxy::proxy),
        )
        // matchit treats "/pf/:id/" as distinct from both "/pf/:id" (exact,
        // no trailing slash) and "/pf/:id/*rest" (which requires at least an
        // empty-but-present wildcard segment and errors as
        // `ExtraTrailingSlash` on a bare trailing slash) — so the bare
        // trailing-slash form needs its own explicit route to avoid falling
        // through to the asset fallback.
        .route(
            "/pf/:id/",
            get(pf_proxy::proxy)
                .post(pf_proxy::proxy)
                .put(pf_proxy::proxy)
                .delete(pf_proxy::proxy)
                .patch(pf_proxy::proxy),
        )
        .route(
            "/pf/:id/*rest",
            get(pf_proxy::proxy)
                .post(pf_proxy::proxy)
                .put(pf_proxy::proxy)
                .delete(pf_proxy::proxy)
                .patch(pf_proxy::proxy),
        )
        .merge(api)
        .fallback(get(assets::serve_asset))
        .with_state(state)
}

/// Bind `config.addr` and serve until the process exits. Initializes the data
/// directory, master key, and database first, so startup fails fast with a
/// clear message instead of at first request.
pub async fn serve(factory: RegistryFactory, config: ServerConfig) -> Result<(), String> {
    let auth_config = auth::AuthConfig::from_env(|k| std::env::var(k).ok())?;
    config::ensure_data_dir(&config.data_dir)
        .map_err(|e| format!("create data dir {}: {e}", config.data_dir.display()))?;
    // Server mode requires the key from the environment — it is never written
    // to the data volume, so a stolen volume yields only sealed ciphertext.
    let env_key = std::env::var("SRELENS_MASTER_KEY").ok();
    let master_key = crypto::MasterKey::require_env(env_key.as_deref())?;
    let db = db::Db::open(&config.data_dir.join("srelens.db")).await?;
    let idp: Arc<dyn auth::idp::IdentityProvider> = match &auth_config.oidc {
        Some(settings) => {
            Arc::new(auth::oidc::OidcProvider::discover(settings, &auth_config.public_url).await?)
        }
        None => Arc::new(auth::idp::NullIdp),
    };
    // A crash may have left decrypted kubeconfigs on disk; wipe before we
    // start materializing anyone's fresh environment.
    users::UserEnvs::wipe_runtime(&config.data_dir);
    let state = AppState {
        user_envs: Arc::new(users::UserEnvs::new(
            factory,
            config.data_dir.clone(),
            auth_config.public_url.clone(),
        )),
        db,
        master_key: Arc::new(master_key),
        auth: Arc::new(auth_config),
        idp,
        pending: Arc::new(auth::idp::PendingLogins::default()),
        pending_cluster: Arc::new(auth::cluster_routes::PendingClusterLogins::default()),
        ws_hub: Arc::new(ws::hub::WsHub::new()),
        pf_client: pf_proxy::client(),
    };
    {
        let db = state.db.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(3600));
            loop {
                tick.tick().await;
                if let Err(e) = db.purge_expired_sessions(unix_now()).await {
                    eprintln!("session purge failed: {e}");
                }
            }
        });
    }
    {
        let envs = state.user_envs.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(300));
            loop {
                tick.tick().await;
                envs.evict_idle(std::time::Duration::from_secs(users::USER_ENV_IDLE_SECS));
            }
        });
    }
    let listener = tokio::net::TcpListener::bind(config.addr)
        .await
        .map_err(|e| format!("bind {}: {e}", config.addr))?;
    serve_on(state, listener).await.map_err(|e| e.to_string())
}

/// Serve on an already-bound listener with a fully-built state.
pub async fn serve_on(state: AppState, listener: tokio::net::TcpListener) -> std::io::Result<()> {
    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
}

/// Resolves on SIGTERM (docker stop) or Ctrl-C (SIGINT), for graceful shutdown.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut sig) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            sig.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
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
            .oneshot(
                Request::builder()
                    .uri("/api/me")
                    .body(Body::empty())
                    .unwrap(),
            )
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
        let token = state
            .db
            .create_session(user.id, crate::unix_now())
            .await
            .unwrap();
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

    #[tokio::test]
    async fn kubeconfig_crud_roundtrip() {
        let state = state().await;
        let user = state.db.upsert_user("i", "s", "u@x", "U", 1).await.unwrap();
        let token = state
            .db
            .create_session(user.id, crate::unix_now())
            .await
            .unwrap();
        let cookie = format!("srelens_session={token}");

        let send = |method: &'static str, uri: String, body: Option<serde_json::Value>| {
            let state = state.clone();
            let cookie = cookie.clone();
            async move {
                let mut builder = Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("cookie", cookie)
                    .header("x-srelens-csrf", "1");
                let body = match body {
                    Some(v) => {
                        builder = builder.header("content-type", "application/json");
                        Body::from(v.to_string())
                    }
                    None => Body::empty(),
                };
                router(state)
                    .oneshot(builder.body(body).unwrap())
                    .await
                    .unwrap()
            }
        };

        // Empty list.
        let resp = send("GET", "/api/kubeconfigs".into(), None).await;
        assert_eq!(resp.status(), StatusCode::OK);

        // Upload.
        let resp = send(
            "POST",
            "/api/kubeconfigs".into(),
            Some(serde_json::json!({ "name": "prod", "yaml": "contexts: []\n" })),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::CREATED);
        let bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let id = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["id"]
            .as_i64()
            .unwrap();

        // Bad yaml rejected.
        let resp = send(
            "POST",
            "/api/kubeconfigs".into(),
            Some(serde_json::json!({ "name": "bad", "yaml": "clusters: []" })),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        // Listed.
        let resp = send("GET", "/api/kubeconfigs".into(), None).await;
        let bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let list: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(list.as_array().unwrap().len(), 1);
        assert_eq!(list[0]["name"], serde_json::json!("prod"));

        // Delete; second delete 404.
        let resp = send("DELETE", format!("/api/kubeconfigs/{id}"), None).await;
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        let resp = send("DELETE", format!("/api/kubeconfigs/{id}"), None).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn kubeconfig_routes_are_gated() {
        let state = state().await;
        for (method, uri) in [
            ("GET", "/api/kubeconfigs"),
            ("POST", "/api/kubeconfigs"),
            ("DELETE", "/api/kubeconfigs/1"),
        ] {
            let resp = router(state.clone())
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(uri)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                resp.status(),
                StatusCode::FORBIDDEN,
                "{method} {uri} must be gated (csrf first)"
            );
        }
    }
}
