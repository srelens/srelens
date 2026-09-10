//! The `k8s.podLogs` capability — fetch recent logs for a pod via kube-rs.

use std::sync::Arc;

use futures::{AsyncBufReadExt, StreamExt};
use k8s_openapi::api::core::v1::Pod;
use kube::api::LogParams;
use kube::Api;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use srelens_capability::{Annotations, Capability, CapabilityError};

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;

const DEFAULT_TAIL_LINES: i64 = 200;

/// Per-stream options beyond the target itself: how much history to tail, an
/// optional `sinceSeconds` window, and whether to prefix each line with an RFC
/// 3339 timestamp. `Copy` so the resilient loop can tweak it per reconnect.
#[derive(Debug, Clone, Copy)]
pub struct StreamOpts {
    pub tail_lines: i64,
    pub since_seconds: Option<i64>,
    pub timestamps: bool,
}

impl Default for StreamOpts {
    fn default() -> Self {
        Self {
            tail_lines: DEFAULT_TAIL_LINES,
            since_seconds: None,
            timestamps: false,
        }
    }
}

/// How many trailing lines a one-shot `k8s.podLogs` fetch asks for. `None`
/// means "no bound": the API then returns every line the container runtime
/// still retains, which is the only way to get a pod's complete history and
/// is why `all_lines` overrides `tail_lines` rather than combining with it.
/// Pure, so the precedence is unit-tested.
pub fn effective_tail_lines(all_lines: bool, tail_lines: Option<i64>) -> Option<i64> {
    if all_lines {
        None
    } else {
        Some(tail_lines.unwrap_or(DEFAULT_TAIL_LINES))
    }
}

/// Build the kube `LogParams` for a target + options. Pure, so the mapping
/// (follow, tail, since, timestamps, previous) is unit-tested.
pub fn build_log_params(
    container: Option<String>,
    follow: bool,
    tail_lines: Option<i64>,
    since_seconds: Option<i64>,
    timestamps: bool,
    previous: bool,
) -> LogParams {
    LogParams {
        container,
        follow,
        tail_lines,
        since_seconds,
        timestamps,
        previous,
        ..Default::default()
    }
}

/// Follow a pod/container's logs, invoking `on_line` for each line as it
/// arrives. Runs until the stream closes (pod exits) or the task is aborted.
/// Tauri-agnostic so the streaming logic stays reusable.
pub async fn stream_pod_logs<F, G>(
    cache: Arc<ClientCache>,
    context: String,
    namespace: String,
    pod: String,
    container: Option<String>,
    opts: StreamOpts,
    mut on_line: F,
    mut on_connected: G,
) -> Result<(), String>
where
    F: FnMut(String) + Send,
    G: FnMut() + Send,
{
    let client = cache.get(&context).await?;
    let api: Api<Pod> = Api::namespaced(client, &namespace);
    let params = build_log_params(
        container,
        true,
        Some(opts.tail_lines),
        opts.since_seconds,
        opts.timestamps,
        false,
    );
    let reader = tokio::time::timeout(request_timeout(), api.log_stream(&pod, &params))
        .await
        .map_err(|_| "open log stream timed out".to_string())?
        .map_err(|e| e.to_string())?;
    on_connected();
    let mut lines = reader.lines();
    while let Some(line) = lines.next().await {
        on_line(line.map_err(|e| e.to_string())?);
    }
    Ok(())
}

/// Backoff between log reconnect attempts.
const LOG_RECONNECT_SECS: u64 = 2;

/// Ordinary init containers are finite. Native sidecars and init containers
/// still eligible for a retry must keep following after an EOF.
fn init_logs_complete(pod: &Pod, container: &str) -> bool {
    let Some(spec) = pod.spec.as_ref() else {
        return false;
    };
    let Some(init) = spec
        .init_containers
        .as_ref()
        .and_then(|cs| cs.iter().find(|c| c.name == container))
    else {
        return false;
    };
    if init.restart_policy.as_deref() == Some("Always") {
        return false;
    }
    let Some(status) = pod.status.as_ref() else {
        return false;
    };
    let Some(terminated) = status
        .init_container_statuses
        .as_ref()
        .and_then(|cs| cs.iter().find(|c| c.name == container))
        .and_then(|c| c.state.as_ref())
        .and_then(|s| s.terminated.as_ref())
    else {
        return false;
    };
    terminated.exit_code == 0
        || spec.restart_policy.as_deref() == Some("Never")
        || matches!(status.phase.as_deref(), Some("Failed" | "Succeeded"))
}

async fn init_logs_finished(
    cache: &ClientCache,
    context: &str,
    namespace: &str,
    pod: &str,
    container: Option<&str>,
) -> bool {
    let Some(container) = container else {
        return false;
    };
    let Ok(client) = cache.get(context).await else {
        return false;
    };
    let api: Api<Pod> = Api::namespaced(client, namespace);
    match tokio::time::timeout(request_timeout(), api.get(pod)).await {
        Ok(Ok(pod)) => init_logs_complete(&pod, container),
        // A failed read is not proof of completion. Preserve retry behavior.
        _ => false,
    }
}

/// Follow a pod/container's logs, transparently reconnecting when the stream
/// ends (pod restart, network blip). Unlike a resource watch, a log stream is
/// one-shot, so we loop: the first connect tails `tail_lines`; reconnects tail
/// `0` (only new lines) to avoid re-printing history. `on_status` fires
/// "reconnecting"/"live" on transitions. A successful EOF for a terminal
/// ordinary init container emits "completed" and ends without retrying.
pub async fn stream_pod_logs_resilient<F, G>(
    cache: Arc<ClientCache>,
    context: String,
    namespace: String,
    pod: String,
    container: Option<String>,
    opts: StreamOpts,
    mut on_line: F,
    mut on_status: G,
) where
    F: FnMut(String) + Send,
    G: FnMut(&'static str) + Send,
{
    let mut first = true;
    loop {
        // First connect honors tail + since; reconnects tail 0 with no `since`
        // so we don't re-print history, but keep the timestamps preference.
        let connect_opts = if first {
            opts
        } else {
            StreamOpts {
                tail_lines: 0,
                since_seconds: None,
                timestamps: opts.timestamps,
            }
        };
        let res = stream_pod_logs(
            cache.clone(),
            context.clone(),
            namespace.clone(),
            pod.clone(),
            container.clone(),
            connect_opts,
            |line| on_line(line),
            || on_status("live"),
        )
        .await;
        if res.is_ok()
            && init_logs_finished(&cache, &context, &namespace, &pod, container.as_deref()).await
        {
            on_status("completed");
            return;
        }
        if let Err(e) = res {
            on_line(format!("[error: {}]", e));
        }
        first = false;
        // Stream ended or errored — signal the outage, back off, then retry.
        on_status("reconnecting");
        tokio::time::sleep(std::time::Duration::from_secs(LOG_RECONNECT_SECS)).await;
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PodLogsIn {
    pub context: String,
    pub namespace: String,
    pub pod: String,
    /// Container name. May be omitted only when the pod has exactly one
    /// container — the Kubernetes log API rejects an omitted container name
    /// for any pod with more than one (it does not pick one for you).
    #[serde(default)]
    pub container: Option<String>,
    /// Number of trailing lines to return (default 200). Ignored when
    /// `all_lines` is set.
    #[serde(default)]
    pub tail_lines: Option<i64>,
    /// Return every line the container runtime still retains instead of a
    /// tail. Wins over `tail_lines` when both are given. Kubernetes keeps only
    /// what log rotation has not yet discarded, and the response can be very
    /// large — prefer `tail_lines`/`since_seconds` unless the whole history is
    /// needed (e.g. to save it to a file). Default false.
    #[serde(default)]
    pub all_lines: bool,
    /// Logs from the previous, terminated instance (post-crash triage).
    #[serde(default)]
    pub previous: bool,
    /// Prefix each line with an RFC 3339 timestamp.
    #[serde(default)]
    pub timestamps: bool,
    /// Only logs newer than this many seconds ago.
    #[serde(default)]
    pub since_seconds: Option<i64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PodLogsOut {
    pub logs: String,
}

/// `k8s.podLogs` — return the last N lines of a pod's logs, or all of them.
pub fn pod_logs_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<PodLogsIn, PodLogsOut, _, _>(
        "k8s.podLogs",
        "fetch logs for a pod in a connected kube context: the last 200 lines by default (tail_lines to change), or set all_lines to get everything the runtime still retains (can be large)",
        Annotations::READ_ONLY,
        move |input: PodLogsIn| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let api: Api<Pod> = Api::namespaced(client, &input.namespace);
                let params = build_log_params(
                    input.container.clone(),
                    false,
                    effective_tail_lines(input.all_lines, input.tail_lines),
                    input.since_seconds,
                    input.timestamps,
                    input.previous,
                );
                let logs = tokio::time::timeout(request_timeout(), api.logs(&input.pod, &params))
                    .await
                    .map_err(|_| CapabilityError::Handler("fetch logs timed out".into()))?
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                Ok(PodLogsOut { logs })
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn init_completion_preserves_retries_and_native_sidecars() {
        let mut value = serde_json::json!({
            "spec":{"containers":[{"name":"app"}],"initContainers":[{"name":"setup"}]},
            "status":{"initContainerStatuses":[{"name":"setup","image":"init","imageID":"init","ready":false,"restartCount":0,"state":{"terminated":{"exitCode":0}}}]}
        });
        let complete = |v: &serde_json::Value, name| {
            init_logs_complete(&serde_json::from_value(v.clone()).unwrap(), name)
        };
        assert!(complete(&value, "setup"));
        assert!(!complete(&value, "app"));
        value["spec"]["initContainers"][0]["restartPolicy"] = "Always".into();
        assert!(!complete(&value, "setup"));
        value["spec"]["initContainers"][0]
            .as_object_mut()
            .unwrap()
            .remove("restartPolicy");
        value["status"]["initContainerStatuses"][0]["state"]["terminated"]["exitCode"] = 1.into();
        assert!(!complete(&value, "setup"));
        value["spec"]["restartPolicy"] = "Never".into();
        assert!(complete(&value, "setup"));
        value["spec"]["restartPolicy"] = "OnFailure".into();
        value["status"]["phase"] = "Failed".into();
        assert!(complete(&value, "setup"));
        value["status"]["initContainerStatuses"][0]["state"] = serde_json::json!({"waiting":{}});
        assert!(!complete(&value, "setup"));
        assert!(!init_logs_complete(&Pod::default(), "setup"));
    }

    #[tokio::test]
    async fn completed_init_logs_finish_without_reconnecting() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config");
        std::fs::write(&config, format!(
            "apiVersion: v1\nkind: Config\ncurrent-context: test\nclusters:\n- name: c\n  cluster:\n    server: http://{address}\nusers:\n- name: u\n  user: {{}}\ncontexts:\n- name: test\n  context: {{cluster: c, user: u}}\n"
        )).unwrap();
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            for _ in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = vec![0; 8192];
                let count = socket.read(&mut bytes).await.unwrap();
                let request = String::from_utf8_lossy(&bytes[..count]).to_string();
                let body = if request.contains("/log?") {
                    "setup complete\n".to_string()
                } else {
                    serde_json::json!({
                        "apiVersion":"v1", "kind":"Pod", "metadata":{"name":"web"},
                        "spec":{"containers":[{"name":"app"}],"initContainers":[{"name":"setup"}]},
                        "status":{"initContainerStatuses":[{"name":"setup","image":"init","imageID":"init","ready":false,"restartCount":0,"state":{"terminated":{"exitCode":0}}}]}
                    }).to_string()
                };
                requests.push(request);
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
            requests
        });
        let mut lines = Vec::new();
        let mut statuses = Vec::new();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            stream_pod_logs_resilient(
                ClientCache::new(config),
                "test".into(),
                "default".into(),
                "web".into(),
                Some("setup".into()),
                StreamOpts::default(),
                |line| lines.push(line),
                |status| statuses.push(status),
            ),
        )
        .await;
        if result.is_err() {
            server.abort();
        }
        assert!(
            result.is_ok(),
            "a completed init container must not enter the retry loop"
        );
        assert_eq!(lines, ["setup complete"]);
        assert_eq!(statuses, ["live", "completed"]);
        assert_eq!(server.await.unwrap().len(), 2);
    }

    #[test]
    fn capability_has_expected_id_and_annotations() {
        let cap = pod_logs_capability(ClientCache::new(PathBuf::from("/x")));
        assert_eq!(cap.id, "k8s.podLogs");
        assert!(cap.annotations.read_only);
    }

    #[test]
    fn build_log_params_maps_all_options() {
        let p = build_log_params(Some("app".into()), false, Some(500), Some(300), true, true);
        assert_eq!(p.container.as_deref(), Some("app"));
        assert!(!p.follow);
        assert_eq!(p.tail_lines, Some(500));
        assert_eq!(p.since_seconds, Some(300));
        assert!(p.timestamps);
        assert!(p.previous);
    }

    #[test]
    fn all_lines_drops_the_tail_bound_and_wins_over_tail_lines() {
        // `all_lines` is the only way to get `tail_lines: None`, which is what
        // makes the API return everything the runtime still retains.
        assert_eq!(effective_tail_lines(true, None), None);
        assert_eq!(effective_tail_lines(true, Some(50)), None);
    }

    #[test]
    fn without_all_lines_the_tail_defaults_to_200_or_the_given_count() {
        // The default path — every existing caller, MCP agents included — is
        // unchanged: omit both and you still get the last 200 lines.
        assert_eq!(effective_tail_lines(false, None), Some(DEFAULT_TAIL_LINES));
        assert_eq!(effective_tail_lines(false, Some(50)), Some(50));
    }

    #[test]
    fn pod_logs_input_defaults_all_lines_to_false() {
        let input: PodLogsIn =
            serde_json::from_str(r#"{"context":"c","namespace":"n","pod":"p"}"#).unwrap();
        assert!(!input.all_lines);
        assert_eq!(input.tail_lines, None);
        let input: PodLogsIn = serde_json::from_str(
            r#"{"context":"c","namespace":"n","pod":"p","all_lines":true,"tail_lines":50}"#,
        )
        .unwrap();
        assert!(input.all_lines);
    }

    #[test]
    fn stream_opts_default_tails_and_omits_extras() {
        let o = StreamOpts::default();
        assert_eq!(o.tail_lines, DEFAULT_TAIL_LINES);
        assert_eq!(o.since_seconds, None);
        assert!(!o.timestamps);
        // The streaming params always follow and are never `previous`.
        let p = build_log_params(
            None,
            true,
            Some(o.tail_lines),
            o.since_seconds,
            o.timestamps,
            false,
        );
        assert!(p.follow && !p.previous);
    }
}
