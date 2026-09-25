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

/// Capabilities blocked on the web surface. UI gating is not a security
/// boundary — these must be denied at the API layer too. `k8s.deleteContext`
/// mutates the per-request materialized kubeconfig (silently reverts on env
/// rebuild); the `toolbox.*` mutators install/remove tools on the shared
/// container host, affecting every user. `k8s.helmRepoAdd`/`k8s.helmRepoUpdate`
/// run via `run_helm_local`, which inherits the process env, so helm's repo
/// config is shared across every web user in the container — one user could
/// add a repo whose name shadows another user's, pointing at an attacker URL
/// that a victim's next helmInstall would pull from. The other helm ops
/// (install/upgrade/rollback/uninstall/template/searchRepo/list/get) use
/// per-context temp kubeconfigs and stay allowed. Read-only toolbox
/// capabilities (status/diagnoseContext/searchPlugins) stay allowed. The
/// `k8s.node*` SSH capabilities run the server's own `ssh` against a
/// caller-named host, user, port and server-side `identityFile`: on the web that
/// hands every user the container's SSH identity against any reachable machine,
/// and `nodeServiceRestart` has no consent prompt there, so all four are denied.
///
/// Apps (`extensions.*`) are not here (#515). Each user's registry reads and
/// writes only that user's own inventory, a row of the server database
/// (`app_inventory.rs`), and reads an app catalog only the server writes
/// (`SharedCatalog`). Every call on this route is a signed-in user's own
/// request: the web host runs no MCP server and no agent, so nothing here
/// answers a consent prompt on anyone's behalf.
pub const WEB_DENIED_CAPABILITIES: &[&str] = &[
    // An app's secret settings (#543) are kept by the desktop vault; the web
    // host has no per-user secret store yet (#522), so a set is refused here
    // rather than kept anywhere else. A web user's registry is built with no
    // store, so `extensions.list` reports secrets as unavailable there too.
    "extension.secretStore",
    // The host action primitives (#549). What makes one safe is an installed
    // app's manifest fixing the kind and the template, and a person confirming
    // the request. A declared action reaches its primitive only through
    // `extensions.action`: the user's own installed app, the exact
    // group/kind/plural it binds, its revision and grants rechecked on the call,
    // UID/resourceVersion preconditions, and the host confirmation the app's
    // screen shows first. Called directly, a primitive would let the caller name
    // the kind and the patch itself, so they stay denied here.
    // `k8s.getCustomResource` stays allowed: it is a read under the user's own
    // kubeconfig and RBAC, like every other custom-resource read.
    "k8s.annotate",
    "k8s.setFields",
    "k8s.setStatusCondition",
    "k8s.mergePatch",
    "k8s.requestRolloutRestart",
    "k8s.requestCordonNode",
    "k8s.deleteContext",
    "k8s.helmRepoAdd",
    "k8s.helmRepoUpdate",
    "k8s.nodeServiceStatus",
    "k8s.nodeJournalLogs",
    "k8s.nodeRuntimeDiagnostics",
    "k8s.nodeServiceRestart",
    "toolbox.installKubectl",
    "toolbox.installHelm",
    "toolbox.installKrew",
    "toolbox.installPlugin",
    "toolbox.upgradePlugin",
    "toolbox.removePlugin",
];

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
            // A handler failure carrying the cluster-login marker means the
            // context is OIDC-protected and has no valid token → 401 so the
            // frontend can start the cluster sign-in flow.
            if let CapabilityError::Handler(msg) = &e {
                if let Some(resp) = maybe_cluster_login_response(msg) {
                    return resp;
                }
            }
            let status = match &e {
                CapabilityError::NotFound(_) => StatusCode::NOT_FOUND,
                CapabilityError::InvalidInput(_) => StatusCode::BAD_REQUEST,
                CapabilityError::Handler(_) => StatusCode::BAD_GATEWAY,
            };
            error_response(status, &e.to_string())
        }
    }
}

/// If `err_msg` is a cluster-login-required marker (from the OIDC auth
/// resolver), build the 401 that tells the frontend to start the cluster OIDC
/// flow: `{"error":"cluster_login_required","key","context","loginUrl"}`.
/// Returns `None` for any other error so normal error mapping proceeds.
pub fn maybe_cluster_login_response(err_msg: &str) -> Option<Response> {
    let (key, context) = srelens_kube::auth_resolver::parse_needs_login(err_msg)?;
    let login_url = format!("/auth/cluster/login?key={key}");
    Some(
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "error": "cluster_login_required",
                "key": key,
                "context": context,
                "loginUrl": login_url,
            })),
        )
            .into_response(),
    )
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
        reg.register(Capability::read_only(
            "needslogin",
            "context needs cluster oidc login",
            |_| async {
                Err(CapabilityError::Handler(
                    srelens_kube::auth_resolver::needs_login_marker("K", "ctx"),
                ))
            },
        ));
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
    async fn cluster_login_marker_becomes_401() {
        // A handler error carrying the OIDC needs-login marker maps to a 401
        // that tells the frontend where to start the cluster sign-in flow —
        // not the generic 502 a plain handler failure would produce.
        let (status, body) = post("/api/capability/needslogin", Body::empty()).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"], json!("cluster_login_required"));
        assert_eq!(body["key"], json!("K"));
        assert_eq!(body["context"], json!("ctx"));
        assert_eq!(body["loginUrl"], json!("/auth/cluster/login?key=K"));
    }

    #[tokio::test]
    async fn plain_handler_failure_stays_502() {
        // A non-marker handler failure is unaffected by the marker check.
        let (status, body) = post("/api/capability/boom", Body::empty()).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert_eq!(body["error"], json!("handler error: cluster unreachable"));
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

    /// Apps are per user on the web (#515), so none of their capabilities is
    /// refused before dispatch: each reaches it (404 in the test registry).
    #[tokio::test]
    async fn app_capabilities_reach_dispatch_on_web() {
        assert!(!super::WEB_DENIED_CAPABILITIES
            .iter()
            .any(|id| id.starts_with("extensions.")));
        for id in [
            "extensions.resource",
            "extensions.action",
            "extensions.catalog",
            "extensions.catalogManifest",
            "extensions.validate",
            "extensions.list",
            "extensions.configure",
            "extensions.read",
            "extensions.resolveColumns",
            "extensions.resolveCards",
            "extensions.resolvePanels",
            "extensions.resolveLinks",
            "extensions.streams",
        ] {
            let (status, _) = post(&format!("/api/capability/{id}"), Body::from("{}")).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{id}");
        }
    }

    /// A server whose users get the registry `srelens-server` builds for them (#515).
    async fn apps_state() -> AppState {
        AppState::for_tests_with(Arc::new(srelens_registry::build_registry_for_user)).await
    }

    async fn sign_in(state: &AppState, sub: &str) -> (i64, String) {
        let user = state
            .db
            .upsert_user("dev", sub, &format!("{sub}@example.com"), sub, 1)
            .await
            .unwrap();
        let token = state
            .db
            .create_session(user.id, crate::unix_now())
            .await
            .unwrap();
        (user.id, format!("srelens_session={token}"))
    }

    async fn call(state: &AppState, cookie: &str, id: &str, input: Value) -> (StatusCode, Value) {
        let resp = router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/capability/{id}"))
                    .header("content-type", "application/json")
                    .header("cookie", cookie)
                    .header("x-srelens-csrf", "1")
                    .body(Body::from(input.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), 4 * 1024 * 1024)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    /// A local, unsigned, read-only app: the registry's own test manifest, at the API
    /// its fixtures use (`settings` needs 0.4, #709).
    fn local_app() -> String {
        let source = include_str!("../../registry/tests/fixtures/argocd-manifest.json")
            .replace("\"org.srelens.argocd\"", "\"org.example.argocd\"")
            .replace("\"^0.1\"", "\"^0.4\"");
        let manifest: Value = serde_json::from_str(&source).unwrap();
        manifest.to_string()
    }

    fn apps(listed: &Value) -> Vec<&str> {
        listed["plugins"]
            .as_array()
            .unwrap()
            .iter()
            .map(|app| app["manifest"]["id"].as_str().unwrap())
            .collect()
    }

    /// Two users of one server keep separate app inventories (#515): what one
    /// installs the other neither lists, reads, inspects, nor changes.
    #[tokio::test(flavor = "multi_thread")]
    async fn two_users_see_only_their_own_apps() {
        let state = apps_state().await;
        let (alice_id, alice) = sign_in(&state, "alice").await;
        let (_, bob) = sign_in(&state, "bob").await;

        let install = json!({"action": "install", "manifest": local_app(),
            "grants": ["k8s.listCustomResource"]});
        let (status, installed) = call(&state, &alice, "extensions.configure", install).await;
        assert_eq!(status, StatusCode::OK, "{installed}");
        assert_eq!(apps(&installed), ["org.example.argocd"]);
        let app = &installed["plugins"][0];
        let (revision, reader) = (
            app["revision"].clone(),
            app["manifest"]["capabilities"][0]["name"].clone(),
        );

        let (status, listed) = call(&state, &bob, "extensions.list", json!({})).await;
        assert_eq!(status, StatusCode::OK);
        assert!(apps(&listed).is_empty(), "{listed}");

        // Naming alice's app and revision gets bob nothing: his inventory has no such app.
        let selection = json!({"id": "org.example.argocd", "revision": revision,
            "capability": reader, "context": "prod", "namespace": "team", "name": "app"});
        let (status, refused) = call(&state, &bob, "extensions.resource", selection.clone()).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert!(
            refused["error"]
                .as_str()
                .unwrap()
                .contains("disabled, removed or updated"),
            "{refused}"
        );
        let mut read = selection.clone();
        read.as_object_mut().unwrap().remove("name");
        let (status, refused) = call(&state, &bob, "extensions.read", read).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert!(
            refused["error"]
                .as_str()
                .unwrap()
                .contains("disabled, removed or updated"),
            "{refused}"
        );
        let action =
            json!({"resource": selection, "action": "sync", "uid": "u", "resourceVersion": "1"});
        let (status, refused) = call(&state, &bob, "extensions.action", action).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert!(
            refused["error"]
                .as_str()
                .unwrap()
                .contains("disabled, removed or updated"),
            "{refused}"
        );
        for change in [
            json!({"action": "remove", "id": "org.example.argocd"}),
            json!({"action": "enable", "id": "org.example.argocd", "enabled": false}),
        ] {
            let (status, refused) = call(&state, &bob, "extensions.configure", change).await;
            assert_eq!(status, StatusCode::BAD_GATEWAY);
            assert!(
                refused["error"].as_str().unwrap().contains("not installed"),
                "{refused}"
            );
        }
        // A settings row is not an inventory: /api/settings cannot place one.
        let resp = router(state.clone())
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/settings/extension_inventories")
                    .header("content-type", "application/json")
                    .header("cookie", &bob)
                    .header("x-srelens-csrf", "1")
                    .body(Body::from(installed.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        let (_, listed) = call(&state, &bob, "extensions.list", json!({})).await;
        assert!(apps(&listed).is_empty(), "{listed}");

        // Alice's app is untouched, and outlives her environment: rebuilding it wipes
        // her runtime files, not her inventory.
        state.user_envs.invalidate(alice_id);
        let (status, listed) = call(&state, &alice, "extensions.list", json!({})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(apps(&listed), ["org.example.argocd"]);
        assert_eq!(listed["plugins"][0]["enabled"], json!(true));
    }

    /// An app that declares a secret setting installs on the web, but the web host
    /// keeps no app secrets yet (#522): the user's registry has no secret store and
    /// no `extension.secretStore`, the list says so, and its other settings still save.
    #[tokio::test(flavor = "multi_thread")]
    async fn apps_with_secret_settings_install_on_the_web_and_keep_no_secret() {
        let state = apps_state().await;
        let (erin_id, erin) = sign_in(&state, "erin").await;
        let mut manifest: Value = serde_json::from_str(&local_app()).unwrap();
        manifest["settings"] =
            json!([{"id": "token", "type": "secret-reference", "title": "Token"}]);
        manifest["permissions"]
            .as_array_mut()
            .unwrap()
            .push(json!("extension.secretStore"));
        let grants = manifest["permissions"].clone();
        let install =
            json!({"action": "install", "manifest": manifest.to_string(), "grants": grants});
        let (status, installed) = call(&state, &erin, "extensions.configure", install).await;
        assert_eq!(status, StatusCode::OK, "{installed}");

        let (_, listed) = call(&state, &erin, "extensions.list", json!({})).await;
        assert_eq!(listed["secretStore"]["available"], json!(false), "{listed}");
        let env = state
            .user_envs
            .env_for(&state.db, &state.master_key, erin_id)
            .await
            .unwrap();
        assert!(env.registry.get("extension.secretStore").is_none());
        let (status, refused) = call(
            &state,
            &erin,
            "extension.secretStore",
            json!({"action": "set", "id": "org.example.argocd", "setting": "token", "secret": "s"}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(refused["error"], json!("capability not available in web mode"));

        let reset = json!({"action": "settings", "id": "org.example.argocd", "settings": {}});
        let (status, saved) = call(&state, &erin, "extensions.configure", reset).await;
        assert_eq!(status, StatusCode::OK, "{saved}");
    }

    /// The catalog is read without any kubeconfig, from the one cache the server
    /// shares between its users and refreshes itself (#515).
    #[tokio::test(flavor = "multi_thread")]
    async fn the_catalog_is_read_without_a_kubeconfig() {
        let state = apps_state().await;
        let (user_id, carol) = sign_in(&state, "carol").await;
        assert!(state.db.list_kubeconfigs(user_id).await.unwrap().is_empty());

        let (status, refused) = call(
            &state,
            &carol,
            "extensions.catalog",
            json!({"refresh": true}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert!(
            refused["error"]
                .as_str()
                .unwrap()
                .contains("has not refreshed the shared catalog"),
            "{refused}"
        );

        let catalog = state.user_envs.catalog().clone();
        tokio::task::spawn_blocking(move || {
            catalog.refresh_if_stale_with(|| {
                Ok(include_bytes!("../../registry/tests/fixtures/extension-catalog.json").to_vec())
            })
        })
        .await
        .unwrap()
        .unwrap();
        let (_, dave) = sign_in(&state, "dave").await;
        for cookie in [&carol, &dave] {
            let (status, snapshot) = call(
                &state,
                cookie,
                "extensions.catalog",
                json!({"refresh": true}),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{snapshot}");
            assert_eq!(snapshot["stale"], json!(false));
            assert_eq!(
                snapshot["catalog"]["extensions"].as_array().unwrap().len(),
                2
            );
        }
    }

    /// #543. Keeping an app's secret needs a store the web host does not have
    /// (per-user storage is #522's), so a set is refused before dispatch —
    /// never answered by something that would keep the value somewhere else —
    /// and the refusal does not repeat it.
    #[tokio::test]
    async fn app_secret_storage_is_refused_on_web_before_dispatch() {
        let secret = "web-must-not-keep-this-7f3a";
        let (status, body) = post(
            "/api/capability/extension.secretStore",
            Body::from(
                json!({"action":"set","id":"org.example.app","setting":"token","secret":secret})
                    .to_string(),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], json!("capability not available in web mode"));
        assert!(!body.to_string().contains(secret));
    }

    #[tokio::test]
    async fn removed_gitops_endpoint_is_absent_and_custom_resource_reads_are_not_denied() {
        // The retired endpoint has no handler and no compatibility shim.
        let (status, _) = post("/api/capability/k8s.gitOpsAction", Body::empty()).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        // A read under the user's own kubeconfig and RBAC stays available: it reaches
        // dispatch (404 in the test registry) instead of being denied.
        let (status, _) = post("/api/capability/k8s.getCustomResource", Body::empty()).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    /// The same reasoning covers #549's action primitives: on the web no
    /// installed app scopes one to a kind and there is no consent prompt, so a
    /// caller could name any kind and any template directly.
    #[tokio::test]
    async fn host_action_primitives_are_denied_on_web() {
        for id in srelens_kube::action_primitives::PRIMITIVES {
            let (status, body) = post(&format!("/api/capability/{id}"), Body::empty()).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{id}");
            assert_eq!(
                body["error"],
                json!("capability not available in web mode"),
                "{id}"
            );
        }
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

    #[tokio::test]
    async fn toolbox_mutators_are_denied_on_web() {
        for id in [
            "toolbox.installKubectl",
            "toolbox.installHelm",
            "toolbox.installKrew",
            "toolbox.installPlugin",
            "toolbox.upgradePlugin",
            "toolbox.removePlugin",
        ] {
            let (status, body) = post(&format!("/api/capability/{id}"), Body::empty()).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{id} must be denied");
            assert_eq!(body["error"], json!("capability not available in web mode"), "{id}");
        }
    }

    #[tokio::test]
    async fn helm_repo_mutators_are_denied_on_web() {
        for id in ["k8s.helmRepoAdd", "k8s.helmRepoUpdate"] {
            let (status, body) = post(&format!("/api/capability/{id}"), Body::empty()).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{id} must be denied");
            assert_eq!(body["error"], json!("capability not available in web mode"), "{id}");
        }
    }

    #[tokio::test]
    async fn node_ssh_capabilities_are_denied_on_web() {
        // Read-only or not, each one runs the server's `ssh` against a host the
        // caller names, so none may reach dispatch on the shared web server.
        for id in [
            "k8s.nodeServiceStatus",
            "k8s.nodeJournalLogs",
            "k8s.nodeRuntimeDiagnostics",
            "k8s.nodeServiceRestart",
        ] {
            let (status, body) = post(&format!("/api/capability/{id}"), Body::empty()).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{id} must be denied");
            assert_eq!(body["error"], json!("capability not available in web mode"), "{id}");
        }
    }

    #[tokio::test]
    async fn toolbox_read_capabilities_are_not_denied() {
        // A read-only toolbox cap is not in the deny-list, so it reaches
        // dispatch and 404s (unregistered in the test registry) rather than
        // being 400-denied.
        let (status, _) = post("/api/capability/toolbox.status", Body::empty()).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
