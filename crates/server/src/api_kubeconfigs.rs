//! Kubeconfig management API: list, upload (paste), and delete the caller's
//! kubeconfigs. Mutations invalidate the user's cached environment so the
//! next capability call sees the new cluster set.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::{Deserialize, Serialize};

use crate::auth::session::UserCtx;
use crate::AppState;

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(serde_json::json!({ "error": message }))).into_response()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KubeconfigOut {
    pub id: i64,
    pub name: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// GET /api/kubeconfigs
pub async fn list(State(state): State<AppState>, Extension(user): Extension<UserCtx>) -> Response {
    match state.db.list_kubeconfigs(user.user_id).await {
        Ok(metas) => Json(
            metas
                .into_iter()
                .map(|m| KubeconfigOut {
                    id: m.id,
                    name: m.name,
                    created_at: m.created_at,
                    updated_at: m.updated_at,
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

#[derive(Deserialize)]
pub struct PutKubeconfig {
    pub name: String,
    pub yaml: String,
}

pub(crate) fn validate_name(name: &str) -> Result<(), &'static str> {
    if name.trim().is_empty() {
        return Err("name must not be empty");
    }
    if name.len() > 64 {
        return Err("name must be at most 64 characters");
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err("name must not contain path separators");
    }
    Ok(())
}

fn validate_yaml(yaml: &str) -> Result<(), &'static str> {
    let parsed: serde_yaml::Value = serde_yaml::from_str(yaml).map_err(|_| "not valid YAML")?;
    let Some(mapping) = parsed.as_mapping() else {
        return Err("kubeconfig must be a YAML mapping");
    };
    if !mapping.contains_key(serde_yaml::Value::String("contexts".into())) {
        return Err("kubeconfig has no contexts");
    }
    Ok(())
}

/// POST /api/kubeconfigs — upsert by name, sealed at rest.
pub async fn put(
    State(state): State<AppState>,
    Extension(user): Extension<UserCtx>,
    Json(body): Json<PutKubeconfig>,
) -> Response {
    if let Err(msg) = validate_name(&body.name) {
        return error(StatusCode::BAD_REQUEST, msg);
    }
    if let Err(msg) = validate_yaml(&body.yaml) {
        return error(StatusCode::BAD_REQUEST, msg);
    }
    match state
        .db
        .put_kubeconfig(
            user.user_id,
            body.name.trim(),
            &state.master_key,
            &body.yaml,
            crate::unix_now(),
        )
        .await
    {
        Ok(id) => {
            state.user_envs.invalidate(user.user_id);
            (StatusCode::CREATED, Json(serde_json::json!({ "id": id }))).into_response()
        }
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

/// DELETE /api/kubeconfigs/:id
pub async fn delete(
    State(state): State<AppState>,
    Extension(user): Extension<UserCtx>,
    Path(id): Path<i64>,
) -> Response {
    match state.db.delete_kubeconfig(user.user_id, id).await {
        Ok(true) => {
            state.user_envs.invalidate(user.user_id);
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => error(StatusCode::NOT_FOUND, "kubeconfig not found"),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

#[cfg(test)]
mod tests {
    use super::{validate_name, validate_yaml};

    #[test]
    fn name_validation() {
        assert!(validate_name("prod").is_ok());
        assert!(validate_name("").is_err());
        assert!(validate_name(" ").is_err());
        assert!(validate_name(&"x".repeat(65)).is_err());
        assert!(validate_name("a/b").is_err());
        assert!(validate_name("a\\b").is_err());
        assert!(validate_name("..").is_err());
    }

    #[test]
    fn yaml_validation() {
        assert!(validate_yaml("contexts: []\nclusters: []").is_ok());
        assert!(validate_yaml("{not yaml").is_err());
        assert!(validate_yaml("- just\n- a\n- list").is_err());
        assert!(validate_yaml("clusters: []").is_err());
    }
}
