//! The `k8s.podLogs` capability — fetch recent logs for a pod via kube-rs.

use std::sync::Arc;

use futures::{AsyncBufRead, AsyncBufReadExt};
use k8s_openapi::api::core::v1::Pod;
use kube::api::LogParams;
use kube::Api;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use srelens_capability::{Annotations, Capability, CapabilityError};

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;

const DEFAULT_TAIL_LINES: i64 = 200;

/// Default number of trailing lines returned by the one-shot `k8s.podLogs` capability.
/// Lowered from 200 to 80 to reduce token consumption during agentic investigations.
pub const DEFAULT_POD_LOGS_TAIL_LINES: i64 = 80;

/// The most of one log line a follow keeps, in bytes (#747). The rest of a
/// longer line is read up to its newline and dropped, and the line says it was
/// cut, so a container that writes a very long line, or no newline at all,
/// cannot grow what the host holds past this.
pub const MAX_LOG_LINE_BYTES: usize = 64 * 1024;

/// Per-stream options beyond the target itself: how much history to tail, an
/// optional `sinceSeconds` window, whether to prefix each line with an RFC
/// 3339 timestamp, and where to cut a long line. `Copy` so the resilient loop
/// can tweak it per reconnect.
#[derive(Debug, Clone, Copy)]
pub struct StreamOpts {
    pub tail_lines: i64,
    pub since_seconds: Option<i64>,
    pub timestamps: bool,
    /// The most of one line kept, in bytes; see [`MAX_LOG_LINE_BYTES`].
    pub max_line_bytes: usize,
}

impl Default for StreamOpts {
    fn default() -> Self {
        Self {
            tail_lines: DEFAULT_TAIL_LINES,
            since_seconds: None,
            timestamps: false,
            max_line_bytes: MAX_LOG_LINE_BYTES,
        }
    }
}

/// One line of a log, as a follow hands it on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    /// The line without its line ending. Bytes that are not UTF-8 read as
    /// U+FFFD rather than ending the stream.
    pub text: String,
    /// The line was longer than the follow keeps, and `text` is its start.
    pub truncated: bool,
}

impl Line {
    /// A line kept whole: within the limit, or one the host writes itself,
    /// such as why a follow failed.
    pub fn whole(text: String) -> Self {
        Self {
            text,
            truncated: false,
        }
    }
}

/// Read `reader` a line at a time, handing each to `on_line`, holding at most
/// `max` bytes of any one line (#747).
///
/// This replaces `AsyncBufReadExt::lines`, which keeps everything up to the
/// next newline in one `String` however long it grows, and ends the stream
/// with an error at the first line that is not UTF-8. Here the bytes past
/// `max` are read and dropped until the newline, the line is cut on a
/// character boundary and marked, and bytes that are not UTF-8 are decoded
/// lossily. A last line with no newline is still a line.
pub async fn read_lines<R, F>(mut reader: R, max: usize, mut on_line: F) -> std::io::Result<()>
where
    R: AsyncBufRead + Unpin,
    F: FnMut(Line),
{
    // One byte over `max`, so a `\r` just past it is a line ending and not a cut.
    let keep = max.saturating_add(1);
    let mut line = Vec::new();
    let mut dropped = false;
    loop {
        let chunk = reader.fill_buf().await?;
        if chunk.is_empty() {
            if !line.is_empty() || dropped {
                on_line(finish(&mut line, dropped, false, max));
            }
            return Ok(());
        }
        let (part, used, ends) = match chunk.iter().position(|&b| b == b'\n') {
            Some(at) => (&chunk[..at], at + 1, true),
            None => (chunk, chunk.len(), false),
        };
        let room = keep.saturating_sub(line.len()).min(part.len());
        line.extend_from_slice(&part[..room]);
        dropped |= room < part.len();
        reader.consume_unpin(used);
        if ends {
            on_line(finish(&mut line, dropped, true, max));
            dropped = false;
        }
    }
}

/// The line `bytes` holds, emptying it: its `\r\n` ending stripped, cut to
/// `max` bytes on a character boundary, and decoded.
fn finish(bytes: &mut Vec<u8>, dropped: bool, newline: bool, max: usize) -> Line {
    if newline && !dropped && bytes.last() == Some(&b'\r') {
        bytes.pop();
    }
    let truncated = dropped || bytes.len() > max;
    if truncated {
        bytes.truncate(max);
        drop_partial_char(bytes);
    }
    let text = String::from_utf8_lossy(bytes).into_owned();
    bytes.clear();
    Line { text, truncated }
}

/// Drop the start of a character that a cut left at the end of `bytes`, so a
/// cut line does not end in U+FFFD for a character it did not break.
fn drop_partial_char(bytes: &mut Vec<u8>) {
    let from = bytes.len().saturating_sub(3);
    for at in (from..bytes.len()).rev() {
        let width = match bytes[at] {
            // A continuation byte: its character starts further back.
            0x80..=0xBF => continue,
            0xC0..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF7 => 4,
            _ => 1,
        };
        if at + width > bytes.len() {
            bytes.truncate(at);
        }
        return;
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
        Some(tail_lines.unwrap_or(DEFAULT_POD_LOGS_TAIL_LINES))
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
/// arrives, each held to `opts.max_line_bytes` ([`read_lines`]). Runs until
/// the stream closes (pod exits) or the task is aborted. Tauri-agnostic so the
/// streaming logic stays reusable.
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
    F: FnMut(Line) + Send,
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
    read_lines(reader, opts.max_line_bytes, &mut on_line)
        .await
        .map_err(|e| e.to_string())
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
    F: FnMut(Line) + Send,
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
                ..opts
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
            on_line(Line::whole(format!("[error: {}]", e)));
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
    /// Optional substring filter (case-insensitive) to retain only matching log lines.
    #[serde(default)]
    pub filter: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PodLogsOut {
    pub logs: String,
}

/// Maximum characters retained per single log line before truncation.
pub const MAX_LOG_LINE_CHARS: usize = 400;
/// Maximum total bytes returned by `k8s.podLogs` to prevent token exhaustion.
pub const MAX_POD_LOGS_BYTES: usize = 12 * 1024;
/// Number of head lines preserved on log truncation (e.g. startup / config).
pub const LOG_HEAD_LINES_PRESERVED: usize = 10;
/// Maximum tail lines preserved when truncating.
pub const LOG_TAIL_LINES_PRESERVED: usize = 60;

/// Compact and budget a pod's logs for token-efficient consumption by LLMs and API callers.
///
/// 1. If `filter` is specified, only lines containing `filter` (case-insensitive) are kept.
/// 2. Long individual lines are clipped to [`MAX_LOG_LINE_CHARS`].
/// 3. If the total log size exceeds [`MAX_POD_LOGS_BYTES`] or [`LOG_HEAD_LINES_PRESERVED`] + [`LOG_TAIL_LINES_PRESERVED`],
///    the output is budgeted to preserve the initial startup lines and the most recent crash/error lines,
///    with an explicit skipped count marker.
pub fn compact_pod_logs(raw: String, filter: Option<&str>, all_lines: bool) -> String {
    let mut lines: Vec<String> = raw
        .lines()
        .map(|l| {
            if l.chars().count() > MAX_LOG_LINE_CHARS {
                let clipped: String = l
                    .chars()
                    .take(MAX_LOG_LINE_CHARS.saturating_sub(15))
                    .collect();
                format!("{}...[clipped]", clipped)
            } else {
                l.to_string()
            }
        })
        .collect();

    if let Some(f) = filter.map(str::trim).filter(|s| !s.is_empty()) {
        let needle = f.to_lowercase();
        lines.retain(|line| line.to_lowercase().contains(&needle));
        if lines.is_empty() {
            return format!("[No log lines matched filter: \"{}\"]", f);
        }
    }

    if all_lines {
        return lines.join("\n");
    }

    let total_bytes: usize = lines.iter().map(|l| l.len() + 1).sum();
    let max_lines = LOG_HEAD_LINES_PRESERVED + LOG_TAIL_LINES_PRESERVED;
    if (total_bytes > MAX_POD_LOGS_BYTES || lines.len() > max_lines) && lines.len() > max_lines {
        let head = &lines[..LOG_HEAD_LINES_PRESERVED];
        let tail = &lines[lines.len() - LOG_TAIL_LINES_PRESERVED..];
        let skipped_lines = lines.len() - max_lines;
        let skipped_bytes: usize = lines
            [LOG_HEAD_LINES_PRESERVED..lines.len() - LOG_TAIL_LINES_PRESERVED]
            .iter()
            .map(|l| l.len() + 1)
            .sum();
        let mut out = head.join("\n");
        out.push_str(&format!(
            "\n[... skipped {} lines ({} bytes) for token efficiency ...]\n",
            skipped_lines, skipped_bytes
        ));
        out.push_str(&tail.join("\n"));
        out
    } else {
        lines.join("\n")
    }
}

/// `k8s.podLogs` — return the last N lines of a pod's logs, or all of them.
pub fn pod_logs_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<PodLogsIn, PodLogsOut, _, _>(
        "k8s.podLogs",
        "fetch logs for a pod in a connected kube context: the last 80 lines by default (tail_lines to change), or set all_lines to get everything the runtime still retains (can be large)",
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
                let logs = compact_pod_logs(logs, input.filter.as_deref(), input.all_lines);
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
                |line| lines.push(line.text),
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

    /// `bytes` through [`read_lines`], handed over `chunk` bytes at a time as
    /// network reads hand them.
    async fn read_all(bytes: &[u8], chunk: usize, max: usize) -> Vec<Line> {
        let reader =
            futures::io::BufReader::with_capacity(chunk, futures::io::Cursor::new(bytes.to_vec()));
        let mut lines = Vec::new();
        read_lines(reader, max, |line| lines.push(line))
            .await
            .unwrap();
        lines
    }

    fn whole(text: &str) -> Line {
        Line {
            text: text.into(),
            truncated: false,
        }
    }

    fn cut(text: &str) -> Line {
        Line {
            text: text.into(),
            truncated: true,
        }
    }

    /// A line past the limit is cut and marked, the rest of it up to its
    /// newline is dropped, and the next line is whole — however the bytes
    /// were split across reads.
    #[tokio::test]
    async fn a_long_line_is_cut_and_the_next_one_is_whole() {
        let mut bytes = b"short\n".to_vec();
        bytes.extend(std::iter::repeat_n(b'x', 100));
        bytes.extend(b"\nafter\n");
        for chunk in [1, 3, 7, 64, 4096] {
            assert_eq!(
                read_all(&bytes, chunk, 10).await,
                [whole("short"), cut("xxxxxxxxxx"), whole("after")],
                "read {chunk} bytes at a time"
            );
        }
        // No newline at all: what is kept is the limit, not the stream.
        let endless = vec![b'y'; 1024 * 1024];
        let lines = read_all(&endless, 8192, MAX_LOG_LINE_BYTES).await;
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text.len(), MAX_LOG_LINE_BYTES);
        assert!(lines[0].truncated);
    }

    /// `\r\n` is one line ending, even split across reads; a lone `\r` is
    /// text; a last line needs no newline. A line exactly at the limit is
    /// whole, with or without its `\r`.
    #[tokio::test]
    async fn line_endings_and_a_last_line_without_one() {
        for chunk in [1, 2, 4096] {
            assert_eq!(
                read_all(b"one\r\ntwo\rthree\nlast", chunk, 64).await,
                [whole("one"), whole("two\rthree"), whole("last")],
                "read {chunk} bytes at a time"
            );
        }
        assert_eq!(read_all(b"abcd\r\n", 1, 4).await, [whole("abcd")]);
        assert_eq!(read_all(b"abcd\n", 1, 4).await, [whole("abcd")]);
        assert_eq!(read_all(b"abcde\r\n", 1, 4).await, [cut("abcd")]);
        assert!(read_all(b"", 1, 4).await.is_empty());
        assert_eq!(read_all(b"\n\n", 1, 4).await, [whole(""), whole("")]);
    }

    /// A line that is not UTF-8 is shown, lossily, and the lines after it
    /// still arrive; `lines()` ended the stream there.
    #[tokio::test]
    async fn bytes_that_are_not_utf8_are_shown_and_the_stream_goes_on() {
        assert_eq!(
            read_all(b"before\n\xff\xfe\nafter\n", 2, 64).await,
            [whole("before"), whole("\u{fffd}\u{fffd}"), whole("after")]
        );
    }

    /// A cut through a character drops the whole character rather than
    /// leaving half of it to read as U+FFFD.
    #[tokio::test]
    async fn a_cut_does_not_break_a_character() {
        assert_eq!(read_all("abé\n".as_bytes(), 1, 3).await, [cut("ab")]);
        assert_eq!(read_all("a日x\n".as_bytes(), 1, 3).await, [cut("a")]);
        assert_eq!(read_all("a日x\n".as_bytes(), 1, 4).await, [cut("a日")]);
    }

    /// Through the API: a follow of a container that writes a long line and
    /// bytes that are not UTF-8 hands on every line, cut where it is long, and
    /// ends cleanly rather than with "stream did not contain valid UTF-8".
    #[tokio::test]
    async fn a_follow_reads_past_a_long_line_and_bytes_that_are_not_utf8() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config");
        std::fs::write(&config, format!(
            "apiVersion: v1\nkind: Config\ncurrent-context: test\nclusters:\n- name: c\n  cluster:\n    server: http://{address}\nusers:\n- name: u\n  user: {{}}\ncontexts:\n- name: test\n  context: {{cluster: c, user: u}}\n"
        )).unwrap();
        let mut body = b"before\n".to_vec();
        body.extend(std::iter::repeat_n(b'x', 200_000));
        body.extend(b"\n\xffbad\nafter\n");
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 8192];
            let _ = socket.read(&mut request).await.unwrap();
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(head.as_bytes()).await.unwrap();
            socket.write_all(&body).await.unwrap();
        });
        let mut lines = Vec::new();
        let result = stream_pod_logs(
            ClientCache::new(config),
            "test".into(),
            "default".into(),
            "web".into(),
            Some("app".into()),
            StreamOpts {
                max_line_bytes: 16,
                ..StreamOpts::default()
            },
            |line| lines.push(line),
            || {},
        )
        .await;
        server.await.unwrap();
        assert_eq!(result, Ok(()));
        assert_eq!(
            lines,
            [
                whole("before"),
                cut("xxxxxxxxxxxxxxxx"),
                whole("\u{fffd}bad"),
                whole("after")
            ]
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
    fn without_all_lines_the_tail_defaults_to_80_or_the_given_count() {
        // The one-shot podLogs default is 80 to prevent excessive token burn in assistant queries.
        assert_eq!(
            effective_tail_lines(false, None),
            Some(DEFAULT_POD_LOGS_TAIL_LINES)
        );
        assert_eq!(effective_tail_lines(false, Some(50)), Some(50));
    }

    #[test]
    fn pod_logs_input_defaults_all_lines_to_false() {
        let input: PodLogsIn =
            serde_json::from_str(r#"{"context":"c","namespace":"n","pod":"p"}"#).unwrap();
        assert!(!input.all_lines);
        assert_eq!(input.tail_lines, None);
        assert_eq!(input.filter, None);
        let input: PodLogsIn = serde_json::from_str(
            r#"{"context":"c","namespace":"n","pod":"p","all_lines":true,"tail_lines":50,"filter":"panic"}"#,
        )
        .unwrap();
        assert!(input.all_lines);
        assert_eq!(input.filter.as_deref(), Some("panic"));
    }

    #[test]
    fn compact_pod_logs_clips_excessively_long_single_lines() {
        let long_line = "A".repeat(1000);
        let output = compact_pod_logs(long_line, None, false);
        assert!(output.contains("...[clipped]"));
        assert!(output.chars().count() <= MAX_LOG_LINE_CHARS);
    }

    #[test]
    fn compact_pod_logs_filters_lines_case_insensitively() {
        let logs =
            "INFO starting app\nWARN high memory\nERROR database connection timeout\nINFO ready";
        let out = compact_pod_logs(logs.to_string(), Some("error"), false);
        assert_eq!(out, "ERROR database connection timeout");

        let none_matched = compact_pod_logs(logs.to_string(), Some("critical"), false);
        assert_eq!(none_matched, "[No log lines matched filter: \"critical\"]");
    }

    #[test]
    fn compact_pod_logs_budgets_large_streams_preserving_head_and_tail() {
        let mut lines = Vec::new();
        for i in 1..=200 {
            lines.push(format!("line {:03}: application event log", i));
        }
        let raw = lines.join("\n");
        let compacted = compact_pod_logs(raw, None, false);

        // Head 10 preserved
        assert!(compacted.contains("line 001: application event log"));
        assert!(compacted.contains("line 010: application event log"));
        // Skip marker present
        assert!(compacted.contains("[... skipped 130 lines"));
        // Tail 60 preserved (lines 141 to 200)
        assert!(compacted.contains("line 141: application event log"));
        assert!(compacted.contains("line 200: application event log"));
        // Middle skipped
        assert!(!compacted.contains("line 050: application event log"));
    }

    #[test]
    fn stream_opts_default_tails_and_omits_extras() {
        let o = StreamOpts::default();
        assert_eq!(o.tail_lines, DEFAULT_TAIL_LINES);
        assert_eq!(o.since_seconds, None);
        assert!(!o.timestamps);
        assert_eq!(o.max_line_bytes, MAX_LOG_LINE_BYTES);
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
