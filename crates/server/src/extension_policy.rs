//! The extension policy this server holds every user's apps to (#578), from its
//! deployment config: a JSON file named by `SRELENS_EXTENSION_POLICY`.
//!
//! A file rather than environment variables, because a policy is lists (app IDs,
//! publishers, capabilities, hosts) that do not fit one variable each, and a file can be
//! mounted read-only beside the compose file and reviewed like one. It is read once, at
//! startup, and checked whole: a policy that does not parse, or names an app, publisher,
//! capability or host that could never match, stops the server with every reason, rather
//! than starting with rules the operator did not write. With no file, the server holds
//! apps to [`AppPolicy::default`], which allows what the web host did before this policy
//! existed.
//!
//! Every signed-in user can read the policy (`GET /api/extension-policy`), and their
//! apps report it too (`extensions.list`), so the UI can say why an app is refused.
//! Nothing writes it over the API. The one way to change it while the server runs is
//! [`srelens_registry::SharedPolicy::replace`]; a write path behind an administrator
//! check (#739) goes through that.

use std::env::VarError;
use std::io::Read as _;

use axum::extract::State;
use axum::Json;
use srelens_registry::{AppPolicy, MAX_POLICY_BYTES};

use crate::AppState;

/// Names the policy file.
pub const POLICY_ENV: &str = "SRELENS_EXTENSION_POLICY";

/// The policy the server starts with, from the value of [`POLICY_ENV`] as the process
/// read it, and where it came from, for the startup log. Unset or empty, the default;
/// a value that is not Unicode names some file this cannot open, so it stops the server
/// rather than start without the policy the operator wrote. Takes the value, so tests
/// never set the process environment.
pub fn from_env(value: Result<String, VarError>) -> Result<(AppPolicy, String), String> {
    match value {
        Err(VarError::NotPresent) => Ok((
            AppPolicy::default(),
            format!("the default ({POLICY_ENV} is not set)"),
        )),
        Err(VarError::NotUnicode(_)) => Err(format!(
            "{POLICY_ENV} is not valid UTF-8; name the policy file with a UTF-8 path"
        )),
        Ok(path) if path.trim().is_empty() => Ok((
            AppPolicy::default(),
            format!("the default ({POLICY_ENV} is empty)"),
        )),
        Ok(path) => load(&path).map(|policy| (policy, path)),
    }
}

/// The policy in the file at `path`, checked whole.
pub fn load(path: &str) -> Result<AppPolicy, String> {
    let refused = |why: String| format!("{POLICY_ENV}={path}: {why}");
    let mut text = String::new();
    std::fs::File::open(path)
        .and_then(|file| {
            file.take(MAX_POLICY_BYTES as u64 + 1)
                .read_to_string(&mut text)
        })
        .map_err(|e| refused(e.to_string()))?;
    AppPolicy::parse(&text).map_err(refused)
}

/// GET /api/extension-policy — the policy in force, for any signed-in user to read.
pub async fn get(State(state): State<AppState>) -> Json<AppPolicy> {
    Json((*state.user_envs.policy().current()).clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::tests::{apps_state, call, local_app, sign_in};
    use crate::router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use serde_json::{json, Value};
    use tower::ServiceExt; // oneshot

    /// Put `policy` in force on `state`'s server, as a restart with that file would.
    fn enforce(state: &AppState, policy: Value) {
        state
            .user_envs
            .policy()
            .replace(AppPolicy::parse(&policy.to_string()).unwrap());
    }

    async fn request(
        state: &AppState,
        method: &str,
        cookie: Option<&str>,
        body: Value,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method(method)
            .uri("/api/extension-policy")
            .header("content-type", "application/json")
            .header("x-srelens-csrf", "1");
        if let Some(cookie) = cookie {
            builder = builder.header("cookie", cookie);
        }
        let resp = router(state.clone())
            .oneshot(builder.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    fn says(answer: &Value, reason: &str) -> bool {
        answer["error"]
            .as_str()
            .is_some_and(|error| error.contains(reason))
    }

    /// Any signed-in user reads the policy in force, from the route and from their own
    /// apps' list; nothing writes it over the API.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_policy_is_read_over_the_api_and_written_nowhere() {
        let state = apps_state().await;
        let (_, alice) = sign_in(&state, "alice").await;
        enforce(&state, json!({"blockedApps": ["org.example.argocd"]}));

        let (status, policy) = request(&state, "GET", Some(&alice), Value::Null).await;
        assert_eq!(status, StatusCode::OK, "{policy}");
        assert_eq!(policy["blockedApps"], json!(["org.example.argocd"]));
        assert_eq!(policy["allowExecutableApps"], json!(false));
        let (_, listed) = call(&state, &alice, "extensions.list", json!({})).await;
        assert_eq!(listed["policy"], policy);

        let (status, _) = request(&state, "GET", None, Value::Null).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let open = json!({"blockedApps": []});
        for method in ["PUT", "POST", "PATCH", "DELETE"] {
            let (status, _) = request(&state, method, Some(&alice), open.clone()).await;
            assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED, "{method}");
        }
        let (_, unchanged) = request(&state, "GET", Some(&alice), Value::Null).await;
        assert_eq!(unchanged, policy);
    }

    /// A blocked app can't be installed on the web, and one installed before the block
    /// is refused on every call from then on, with nothing rebuilt.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_blocked_app_can_be_neither_installed_nor_called() {
        let state = apps_state().await;
        let (_, alice) = sign_in(&state, "alice").await;
        let (_, bob) = sign_in(&state, "bob").await;
        let install = json!({"action": "install", "manifest": local_app(),
            "grants": ["k8s.listCustomResource"]});
        let (status, installed) =
            call(&state, &alice, "extensions.configure", install.clone()).await;
        assert_eq!(status, StatusCode::OK, "{installed}");
        let app = &installed["plugins"][0];
        let selection = json!({"id": "org.example.argocd", "revision": app["revision"],
            "capability": app["manifest"]["capabilities"][0]["name"], "context": "prod",
            "namespace": "team", "name": "app"});

        enforce(&state, json!({"blockedApps": ["org.example.argocd"]}));
        let blocked = "The administrator's policy blocks org.example.argocd";
        let (status, refused) = call(&state, &bob, "extensions.configure", install).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert!(says(&refused, blocked), "{refused}");
        let (_, listed) = call(&state, &bob, "extensions.list", json!({})).await;
        assert_eq!(listed["plugins"], json!([]));

        let (_, listed) = call(&state, &alice, "extensions.list", json!({})).await;
        assert_eq!(listed["plugins"][0]["policyBlocked"], json!(blocked));
        assert_eq!(listed["plugins"][0]["enabled"], json!(false));
        let mut read = selection.clone();
        read.as_object_mut().unwrap().remove("name");
        let action =
            json!({"resource": selection, "action": "sync", "uid": "u", "resourceVersion": "1"});
        for (id, payload) in [
            ("extensions.read", read),
            ("extensions.resource", selection.clone()),
            ("extensions.action", action),
            (
                "extensions.configure",
                json!({"action": "enable", "id": "org.example.argocd", "enabled": true}),
            ),
        ] {
            let (status, refused) = call(&state, &alice, id, payload).await;
            assert_eq!(status, StatusCode::BAD_GATEWAY, "{id}");
            assert!(says(&refused, blocked), "{id}: {refused}");
        }
    }

    /// A capability the policy stops allowing is refused at call time for an app
    /// installed before the change, and still after the user's environment is rebuilt.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_capability_outside_the_policy_is_refused_at_call_time() {
        let state = apps_state().await;
        let (alice_id, alice) = sign_in(&state, "alice").await;
        let install = json!({"action": "install", "manifest": local_app(),
            "grants": ["k8s.listCustomResource"]});
        let (status, installed) = call(&state, &alice, "extensions.configure", install).await;
        assert_eq!(status, StatusCode::OK, "{installed}");
        let app = &installed["plugins"][0];
        let read = json!({"id": "org.example.argocd", "revision": app["revision"],
            "capability": app["manifest"]["capabilities"][0]["name"], "context": "prod"});

        enforce(&state, json!({"allowedCapabilities": ["k8s.listEvents"]}));
        let outside = "The administrator's policy does not allow k8s.listCustomResource";
        let (status, refused) = call(&state, &alice, "extensions.read", read.clone()).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert!(says(&refused, outside), "{refused}");
        state.user_envs.invalidate(alice_id);
        let (status, refused) = call(&state, &alice, "extensions.read", read).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert!(says(&refused, outside), "{refused}");
    }

    /// With a ceiling, a web user's apps may use network.http, and a request to a host
    /// the app lists but the ceiling does not is refused before anything is sent.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_network_host_outside_the_ceiling_is_refused() {
        // A port nothing listens on: a request the policy lets through fails to connect.
        let closed = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let inside = format!("127.0.0.1:{}", closed.port());
        let state = apps_state().await;
        enforce(&state, json!({"networkCeiling": [inside]}));
        let (_, alice) = sign_in(&state, "alice").await;
        let manifest = json!({
            "id": "org.example.metrics", "name": "Metrics", "version": "0.1.0",
            "srelensApiVersion": "^0.4", "kind": "declarative",
            "permissions": [{"capability": "network.http",
                "hosts": [inside, "metrics.example.invalid"]}],
            "capabilities": [
                {"name": "inside", "title": "Inside", "target": "network.http",
                    "arguments": {"url": format!("https://{inside}/api")}, "inputs": []},
                {"name": "outside", "title": "Outside", "target": "network.http",
                    "arguments": {"url": "https://metrics.example.invalid/api"}, "inputs": []}],
            "contributions": {"pages": [], "detailTabs": [], "detailLinks": []}
        });
        let install = json!({"action": "install", "manifest": manifest.to_string(),
            "grants": ["network.http"]});
        let (status, installed) = call(&state, &alice, "extensions.configure", install).await;
        assert_eq!(status, StatusCode::OK, "{installed}");
        let read = |name: &str| {
            json!({"id": "org.example.metrics", "revision": installed["plugins"][0]["revision"],
                "capability": name, "context": "prod"})
        };
        let (status, refused) = call(&state, &alice, "extensions.read", read("outside")).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert!(
            says(
                &refused,
                "outside the hosts the administrator's policy lets network.http reach"
            ),
            "{refused}"
        );
        let (status, failed) = call(&state, &alice, "extensions.read", read("inside")).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert!(!says(&failed, "administrator's policy"), "{failed}");
    }

    #[test]
    fn no_file_is_the_default_policy_and_the_log_says_why() {
        use std::env::VarError;
        let (policy, source) = from_env(Err(VarError::NotPresent)).unwrap();
        assert_eq!(policy, AppPolicy::default());
        assert_eq!(source, format!("the default ({POLICY_ENV} is not set)"));
        // Set but empty, as compose's `${VAR:-}` writes an unset one: the default too,
        // and the log says it was set.
        let (policy, source) = from_env(Ok("  ".into())).unwrap();
        assert_eq!(policy, AppPolicy::default());
        assert_eq!(source, format!("the default ({POLICY_ENV} is empty)"));
    }

    /// A value the process cannot read as a path names some file, so starting on the
    /// default instead would run without the policy the operator wrote.
    #[cfg(unix)]
    #[test]
    fn a_value_that_is_not_unicode_stops_the_server() {
        use std::os::unix::ffi::OsStringExt;
        let value = std::ffi::OsString::from_vec(vec![b'/', 0xff]);
        let error = from_env(Err(std::env::VarError::NotUnicode(value))).unwrap_err();
        assert!(
            error.contains(POLICY_ENV) && error.contains("UTF-8"),
            "{error}"
        );
    }

    #[test]
    fn a_file_is_read_and_checked_whole() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("policy.json");
        std::fs::write(&path, r#"{"blockedApps":["org.example.app"]}"#).unwrap();
        let name = path.to_str().unwrap();
        assert_eq!(
            load(name).unwrap(),
            AppPolicy::parse(r#"{"blockedApps":["org.example.app"]}"#).unwrap()
        );

        std::fs::write(&path, r#"{"blockedApps":["*"],"allowExecutableApps":true}"#).unwrap();
        let error = load(name).unwrap_err();
        assert!(
            error.starts_with(&format!("{POLICY_ENV}={name}: ")),
            "{error}"
        );
        assert!(
            error.contains("blockedApps") && error.contains("#521"),
            "{error}"
        );

        std::fs::write(&path, " ".repeat(MAX_POLICY_BYTES + 1)).unwrap();
        assert!(load(name).unwrap_err().contains("1 MiB"));

        let missing = dir.path().join("missing.json");
        let error = load(missing.to_str().unwrap()).unwrap_err();
        assert!(error.contains("missing.json"), "{error}");
    }
}
