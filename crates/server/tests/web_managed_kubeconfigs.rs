//! The server host's managed kubeconfig folder must never reach a web user.
//!
//! On the desktop that folder is the person's own — where a pasted config is
//! saved — and `k8s.listContexts` merges it on every call. On the web server it
//! belongs to whoever runs the host, and every user's registry would read it.
//!
//! This is its own test binary because the folder is found through
//! `SRELENS_KUBECONFIG_DIR`, which is process-wide. Set here, before anything
//! else in the process runs, it cannot race another test reading the
//! environment; set inside the library's suite, it could.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use srelens_server::{router, AppState};
use tower::ServiceExt; // oneshot

fn kubeconfig(name: &str) -> String {
    format!(
        "apiVersion: v1\nkind: Config\nclusters:\n- name: {name}\n  cluster: {{server: https://127.0.0.1:1}}\nusers:\n- name: {name}\n  user: {{token: t}}\ncontexts:\n- name: {name}\n  context: {{cluster: {name}, user: {name}}}\n"
    )
}

#[tokio::test]
async fn web_list_contexts_never_reads_the_host_managed_folder() {
    let managed = tempfile::tempdir().unwrap();
    std::fs::write(managed.path().join("host.yaml"), kubeconfig("host")).unwrap();
    std::env::set_var("SRELENS_KUBECONFIG_DIR", managed.path());

    let state =
        AppState::for_tests_with(Arc::new(srelens_registry::build_registry_for_user)).await;
    let user = state
        .db
        .upsert_user("test-iss", "test-sub", "t@x", "T", 1)
        .await
        .unwrap();
    state
        .db
        .put_kubeconfig(user.id, "mine", &state.master_key, &kubeconfig("mine"), 1)
        .await
        .unwrap();
    let token = state
        .db
        .create_session(user.id, srelens_server::unix_now())
        .await
        .unwrap();

    for input in [json!({ "paths": [] }), json!({})] {
        let resp = router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/capability/k8s.listContexts")
                    .header("content-type", "application/json")
                    .header("cookie", format!("srelens_session={token}"))
                    .header("x-srelens-csrf", "1")
                    .body(Body::from(input.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "{input}");
        let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        let names: Vec<&str> = body["contexts"]
            .as_array()
            .expect("contexts")
            .iter()
            .map(|c| c["name"].as_str().expect("name"))
            .collect();
        assert_eq!(names, ["mine"], "{input}");
    }

    let env = state
        .user_envs
        .env_for(&state.db, &state.master_key, user.id)
        .await
        .unwrap();
    assert_eq!(env.cache.paths().await, env.paths);
}
