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

/// Ordinary init containers are finite. Native sidecars finish with their
/// Pod; containers still eligible for a retry keep following after an EOF.
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
    matches!(status.phase.as_deref(), Some("Failed" | "Succeeded"))
        || (init.restart_policy.as_deref() != Some("Always")
            && (terminated.exit_code == 0 || spec.restart_policy.as_deref() == Some("Never")))
}

/// Completion must describe the same already-terminal attempt on both sides
/// of the read. A running attempt can finish after an early clean EOF, so an
/// after-only check (even with the same restart count) is insufficient.
fn same_completed_init(before: &Pod, after: &Pod, container: &str) -> bool {
    if before.metadata.uid.is_none()
        || before.metadata.uid != after.metadata.uid
        || !init_logs_complete(before, container)
        || !init_logs_complete(after, container)
    {
        return false;
    }
    let status = |pod: &Pod| {
        pod.status
            .as_ref()?
            .init_container_statuses
            .as_ref()?
            .iter()
            .find(|s| s.name == container)
            .cloned()
    };
    match (status(before), status(after)) {
        (Some(a), Some(b)) => {
            a.restart_count == b.restart_count
                && a.container_id == b.container_id
                && a.state == b.state
        }
        _ => false,
    }
}

async fn read_log_pod(
    cache: &ClientCache,
    context: &str,
    namespace: &str,
    pod: &str,
    container: Option<&str>,
) -> Option<Pod> {
    container?;
    let Ok(client) = cache.get(context).await else {
        return None;
    };
    let api: Api<Pod> = Api::namespaced(client, namespace);
    match tokio::time::timeout(request_timeout(), api.get(pod)).await {
        Ok(Ok(pod)) => Some(pod),
        // A failed read is not proof of completion. Preserve retry behavior.
        _ => None,
    }
}

/// Follow a pod/container's logs, transparently reconnecting when the stream
/// ends (pod restart, network blip). Unlike a resource watch, a log stream is
/// one-shot, so we loop: the first connect tails `tail_lines`; ordinary
/// reconnects tail `0`. A terminal init attempt is read with the requested
/// history again, which may replay output but cannot discard its final lines
/// after an early EOF. Matching terminal states around that read emit
/// "completed"; other transitions emit "reconnecting"/"live".
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
        let before = read_log_pod(&cache, &context, &namespace, &pod, container.as_deref()).await;
        let terminal = before
            .as_ref()
            .zip(container.as_deref())
            .is_some_and(|(p, c)| init_logs_complete(p, c));
        // First connect honors tail + since; reconnects tail 0 with no `since`
        // so we don't re-print history, but keep the timestamps preference.
        // Read the requested history again once terminal. A prior EOF might
        // have preceded completion or belonged to a different attempt; tail 0
        // would silently discard the final output in either case.
        let connect_opts = if first || terminal {
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
        if res.is_ok() {
            let after =
                read_log_pod(&cache, &context, &namespace, &pod, container.as_deref()).await;
            if before
                .as_ref()
                .zip(after.as_ref())
                .zip(container.as_deref())
                .is_some_and(|((a, b), c)| same_completed_init(a, b, c))
            {
                on_status("completed");
                return;
            }
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
    fn completion_requires_the_same_terminal_attempt_and_pod() {
        let before: Pod = serde_json::from_value(serde_json::json!({
            "metadata":{"uid":"pod-1"},
            "spec":{"containers":[{"name":"app"}],"initContainers":[{"name":"setup"}]},
            "status":{"initContainerStatuses":[{"name":"setup","containerID":"container-1","image":"init","imageID":"init","ready":false,"restartCount":0,"state":{"terminated":{"exitCode":0}}}]}
        })).unwrap();
        assert!(same_completed_init(&before, &before, "setup"));
        let mut after = before.clone();
        after
            .status
            .as_mut()
            .unwrap()
            .init_container_statuses
            .as_mut()
            .unwrap()[0]
            .restart_count = 1;
        assert!(!same_completed_init(&before, &after, "setup"));
        after = before.clone();
        after
            .status
            .as_mut()
            .unwrap()
            .init_container_statuses
            .as_mut()
            .unwrap()[0]
            .container_id = Some("container-2".into());
        assert!(!same_completed_init(&before, &after, "setup"));
        after = before.clone();
        after.metadata.uid = Some("pod-2".into());
        assert!(!same_completed_init(&before, &after, "setup"));
        after.metadata.uid = None;
        assert!(!same_completed_init(&after, &after, "setup"));
    }

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
        assert_init_completion(0, false).await;
    }

    #[tokio::test]
    async fn clean_disconnect_before_init_completion_reads_the_final_snapshot() {
        assert_init_completion(1, false).await;
    }

    #[tokio::test]
    async fn a_later_successful_init_attempt_is_read_before_completion() {
        assert_init_completion(2, false).await;
    }

    #[tokio::test]
    async fn native_sidecar_logs_complete_when_the_pod_is_terminal() {
        assert_init_completion(0, true).await;
    }

    async fn assert_init_completion(race: u8, sidecar: bool) {
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
            for index in 0..if race > 0 { 6 } else { 3 } {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = vec![0; 8192];
                let count = socket.read(&mut bytes).await.unwrap();
                let request = String::from_utf8_lossy(&bytes[..count]).to_string();
                let body = if request.contains("/log?") {
                    if race > 0 && index < 3 {
                        "partial output\n".to_string()
                    } else {
                        "setup complete\n".to_string()
                    }
                } else {
                    let mut value = serde_json::json!({
                        "apiVersion":"v1", "kind":"Pod", "metadata":{"name":"web","uid":"pod-1"},
                        "spec":{"containers":[{"name":"app"}],"initContainers":[{"name":"setup"}]},
                        "status":{"initContainerStatuses":[{"name":"setup","containerID":"container-1","image":"init","imageID":"init","ready":false,"restartCount":0,"state":{"terminated":{"exitCode":0}}}]}
                    });
                    if sidecar {
                        value["spec"]["initContainers"][0]["restartPolicy"] = "Always".into();
                        value["status"]["phase"] = "Succeeded".into();
                    }
                    if race > 0 && index == 0 {
                        value["status"]["initContainerStatuses"][0]["state"] =
                            serde_json::json!({"running":{}});
                    } else if race == 2 {
                        value["status"]["initContainerStatuses"][0]["restartCount"] = 1.into();
                        value["status"]["initContainerStatuses"][0]["containerID"] =
                            "container-2".into();
                    }
                    value.to_string()
                };
                requests.push(request);
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
            requests
        });
        let mut lines = Vec::new();
        let mut statuses = Vec::new();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(10),
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
        assert!(
            lines.contains(&"setup complete".to_string()),
            "must read the final attempt, got {lines:?}"
        );
        assert_eq!(statuses.last(), Some(&"completed"));
        if race == 0 {
            assert_eq!(statuses, ["live", "completed"]);
        }
        let requests = tokio::time::timeout(std::time::Duration::from_secs(1), server)
            .await
            .expect("all expected API reads must happen")
            .unwrap();
        let final_read = requests.iter().rev().find(|r| r.contains("/log?")).unwrap();
        assert!(
            final_read.contains("tailLines=200"),
            "must not discard the final attempt with tailLines=0"
        );
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
