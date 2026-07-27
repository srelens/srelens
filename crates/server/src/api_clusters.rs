//! Cluster CRUD API: define a cluster from the "Add cluster" UI form (synthesizes
//! a one-context kubeconfig, stored exactly like an uploaded one) and forget a
//! cluster's stored OIDC token. Mutations invalidate the user's cached
//! environment so the next capability call sees the change.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;

use crate::auth::session::UserCtx;
use crate::AppState;

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(serde_json::json!({ "error": message }))).into_response()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OidcForm {
    pub issuer: String,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub extra_scopes: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterCreate {
    pub name: String,
    pub server: String,
    pub ca_cert_pem: Option<String>,
    #[serde(default)]
    pub insecure_skip_tls_verify: bool,
    pub oidc: Option<OidcForm>,
}

/// POST /api/clusters — synthesize a one-context kubeconfig from the form and
/// store it under the caller's own kubeconfigs.
pub async fn create(
    State(state): State<AppState>,
    Extension(user): Extension<UserCtx>,
    Json(body): Json<ClusterCreate>,
) -> Response {
    // Same name rules as an uploaded kubeconfig (bounded length, no path
    // separators), applied to the trimmed name that will be stored.
    let name = body.name.trim().to_string();
    if let Err(e) = crate::api_kubeconfigs::validate_name(&name) {
        return error(StatusCode::BAD_REQUEST, e);
    }
    let form = srelens_kube::cluster_synth::ClusterForm {
        name: name.clone(),
        server: body.server,
        ca_cert_pem: body.ca_cert_pem,
        insecure_skip_tls_verify: body.insecure_skip_tls_verify,
        oidc: body
            .oidc
            .map(|o| srelens_kube::oidc_detect::OidcClusterConfig {
                issuer: o.issuer,
                client_id: o.client_id,
                client_secret: o.client_secret,
                extra_scopes: o.extra_scopes.unwrap_or_default(),
            }),
    };
    let yaml = match srelens_kube::cluster_synth::synthesize_kubeconfig(&form) {
        Ok(yaml) => yaml,
        Err(e) => return error(StatusCode::BAD_REQUEST, &format!("invalid cluster: {e}")),
    };
    if let Err(e) = state
        .db
        .put_kubeconfig(
            user.user_id,
            &name,
            &state.master_key,
            &yaml,
            crate::unix_now(),
        )
        .await
    {
        return error(StatusCode::INTERNAL_SERVER_ERROR, &e);
    }
    state.user_envs.invalidate(user.user_id);
    Json(serde_json::json!({ "name": name })).into_response()
}

/// GET /api/clusters — list the caller's OIDC clusters with sign-in status.
pub async fn list(
    State(state): State<AppState>,
    Extension(user): Extension<UserCtx>,
) -> Response {
    let env = match state
        .user_envs
        .env_for(&state.db, &state.master_key, user.user_id)
        .await
    {
        Ok(env) => env,
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    };
    let now = crate::unix_now();
    let mut clusters = Vec::new();
    for (key, cfg, contexts) in env.oidc_registry.oidc_clusters() {
        let token = state
            .db
            .get_cluster_token(user.user_id, &key, &state.master_key)
            .await
            .ok()
            .flatten();
        let (signed_in, expires_at) = match &token {
            // "signed in" = a token exists and isn't already past expiry.
            Some(t) => (t.expires_at > now, Some(t.expires_at)),
            None => (false, None),
        };
        clusters.push(serde_json::json!({
            "key": key,
            "issuer": cfg.issuer,
            "clientId": cfg.client_id,
            "contexts": contexts,
            "signedIn": signed_in,
            "expiresAt": expires_at,
        }));
    }
    Json(serde_json::json!({ "clusters": clusters })).into_response()
}

/// POST /api/clusters/:key/logout — forget the caller's stored OIDC token for
/// this cluster (does not remove the cluster's kubeconfig itself).
pub async fn logout(
    State(state): State<AppState>,
    Extension(user): Extension<UserCtx>,
    Path(key): Path<String>,
) -> Response {
    if let Err(e) = state.db.delete_cluster_token(user.user_id, &key).await {
        return error(StatusCode::INTERNAL_SERVER_ERROR, &e);
    }
    state.user_envs.invalidate(user.user_id);
    Json(serde_json::json!({ "ok": true })).into_response()
}

#[cfg(test)]
mod tests {
    use crate::{router, AppState};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use srelens_capability::Registry;
    use std::sync::Arc;
    use tower::ServiceExt; // oneshot

    async fn session(state: &AppState) -> (i64, String) {
        let user = state.db.upsert_user("i", "s", "u@x", "U", 1).await.unwrap();
        let token = state
            .db
            .create_session(user.id, crate::unix_now())
            .await
            .unwrap();
        (user.id, token)
    }

    async fn send(
        state: AppState,
        method: &'static str,
        uri: String,
        cookie: &str,
        body: Option<serde_json::Value>,
    ) -> axum::response::Response {
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

    #[tokio::test]
    async fn create_with_oidc_body_is_stored_and_detected_as_oidc() {
        let state = AppState::for_tests(Arc::new(Registry::new())).await;
        let (user_id, token) = session(&state).await;
        let cookie = format!("srelens_session={token}");

        let resp = send(
            state.clone(),
            "POST",
            "/api/clusters".into(),
            &cookie,
            Some(serde_json::json!({
                "name": "prod",
                "server": "https://api.prod:6443",
                "insecureSkipTlsVerify": true,
                "oidc": {
                    "issuer": "https://dex.example.com",
                    "clientId": "k8s",
                    "extraScopes": ["groups"],
                }
            })),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["name"], serde_json::json!("prod"));

        let metas = state.db.list_kubeconfigs(user_id).await.unwrap();
        assert_eq!(metas.len(), 1);
        let yaml = state
            .db
            .get_kubeconfig_yaml(user_id, metas[0].id, &state.master_key)
            .await
            .unwrap()
            .unwrap();
        let reg = crate::cluster_registry::ClusterOidcRegistry::from_kubeconfig_yamls(&[yaml]);
        let key = reg.key_for_context("prod").expect("context is OIDC");
        assert!(reg.config_for_key(&key).is_some());
    }

    #[tokio::test]
    async fn list_returns_created_oidc_cluster_not_signed_in() {
        let state = AppState::for_tests(Arc::new(Registry::new())).await;
        let (_user_id, token) = session(&state).await;
        let cookie = format!("srelens_session={token}");

        let resp = send(
            state.clone(),
            "POST",
            "/api/clusters".into(),
            &cookie,
            Some(serde_json::json!({
                "name": "prod",
                "server": "https://api.prod:6443",
                "insecureSkipTlsVerify": true,
                "oidc": {
                    "issuer": "https://dex.example.com",
                    "clientId": "k8s",
                    "extraScopes": ["groups"],
                }
            })),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);

        let resp = send(state.clone(), "GET", "/api/clusters".into(), &cookie, None).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let clusters = v["clusters"].as_array().unwrap();
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0]["signedIn"], serde_json::json!(false));
        assert_eq!(clusters[0]["issuer"], serde_json::json!("https://dex.example.com"));
        assert_eq!(clusters[0]["clientId"], serde_json::json!("k8s"));
        assert!(!clusters[0]["contexts"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn create_with_invalid_body_is_bad_request() {
        let state = AppState::for_tests(Arc::new(Registry::new())).await;
        let (_user_id, token) = session(&state).await;
        let cookie = format!("srelens_session={token}");

        let resp = send(
            state.clone(),
            "POST",
            "/api/clusters".into(),
            &cookie,
            Some(serde_json::json!({
                "name": "",
                "server": "https://x",
                "insecureSkipTlsVerify": false,
            })),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn logout_deletes_the_stored_token() {
        let state = AppState::for_tests(Arc::new(Registry::new())).await;
        let (user_id, token) = session(&state).await;
        let cookie = format!("srelens_session={token}");

        state
            .db
            .put_cluster_token(
                user_id,
                "k1",
                &state.master_key,
                &crate::cluster_tokens::StoredToken {
                    id_token: "idtok".into(),
                    refresh_token: Some("reftok".into()),
                    expires_at: 5000,
                },
            )
            .await
            .unwrap();
        assert!(state
            .db
            .get_cluster_token(user_id, "k1", &state.master_key)
            .await
            .unwrap()
            .is_some());

        let resp = send(
            state.clone(),
            "POST",
            "/api/clusters/k1/logout".into(),
            &cookie,
            None,
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);

        assert!(state
            .db
            .get_cluster_token(user_id, "k1", &state.master_key)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn cluster_routes_are_gated() {
        let state = AppState::for_tests(Arc::new(Registry::new())).await;
        for (method, uri) in [
            ("GET", "/api/clusters"),
            ("POST", "/api/clusters"),
            ("POST", "/api/clusters/k/logout"),
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
