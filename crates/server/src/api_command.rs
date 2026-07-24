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
                .map_err(|e| error(StatusCode::BAD_GATEWAY, &e))?;
            json!({ "channel": channel })
        }
        "stop_watch" => {
            let a: ChannelArg = parse(body)?;
            env.streams.watch.stop(&a.channel);
            json!({})
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
                .map_err(|e| error(StatusCode::BAD_GATEWAY, &e))?;
            json!({})
        }
        "stop_log_stream" => {
            let a: ChannelArg = parse(body)?;
            env.streams.logs.stop(&a.channel);
            json!({})
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
                .map_err(|e| error(StatusCode::BAD_GATEWAY, &e))?;
            json!({ "id": id })
        }
        "exec_input" => {
            let a: InputArg = parse(body)?;
            env.streams.exec.input(a.session, a.data).await;
            json!({})
        }
        "exec_resize" => {
            let a: ResizeArg = parse(body)?;
            env.streams.exec.resize(a.session, a.cols, a.rows).await;
            json!({})
        }
        "exec_close" => {
            let a: SessionArg = parse(body)?;
            env.streams.exec.close(a.session);
            json!({})
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
                .map_err(|e| error(StatusCode::BAD_GATEWAY, &e))?;
            json!({ "id": info.id, "localPort": info.local_port })
        }
        "stop_port_forward" => {
            let a: IdArg = parse(body)?;
            env.streams.forward.stop(a.id);
            json!({})
        }
        "start_terminal" => {
            let a: TerminalStart = parse(body)?;
            let id = env
                .streams
                .terminal
                .start(sink(), a.context, paths, a.channel, a.cols, a.rows)
                .await
                .map_err(|e| error(StatusCode::BAD_GATEWAY, &e))?;
            json!({ "id": id })
        }
        "terminal_input" => {
            let a: InputArg = parse(body)?;
            env.streams.terminal.input(a.session, &a.data);
            json!({})
        }
        "terminal_resize" => {
            let a: ResizeArg = parse(body)?;
            env.streams.terminal.resize(a.session, a.cols, a.rows);
            json!({})
        }
        "terminal_close" => {
            let a: SessionArg = parse(body)?;
            env.streams.terminal.close(a.session);
            json!({})
        }
        "start_helm_op" => {
            let a: HelmStart = parse(body)?;
            let id = env
                .streams
                .helm
                .start(sink(), a.context, paths, a.args, a.values, a.channel)
                .await
                .map_err(|e| error(StatusCode::BAD_GATEWAY, &e))?;
            json!({ "id": id })
        }
        "helm_op_close" => {
            let a: SessionArg = parse(body)?;
            env.streams.helm.close(a.session);
            json!({})
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
        assert_eq!(body, json!({ "channel": "watch:1" }));
    }

    #[tokio::test]
    async fn void_command_returns_empty_object() {
        let state = AppState::for_tests(Arc::new(Registry::new())).await;
        let (status, body) = authed_post(&state, "stop_watch", json!({ "channel": "x" })).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!({}));
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
