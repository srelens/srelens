//! The `k8s.podLogs` capability — fetch recent logs for a pod via kube-rs.

use std::collections::BTreeMap;
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
use crate::workloads::LabelSelectorRequirement;

const DEFAULT_TAIL_LINES: i64 = 200;

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

/// [`read_lines`] one line at a time, for a reader that stops between lines:
/// a page of `k8s.podLogs` stops when it is full and reads on later.
/// Cancel-safe — everything read so far lives here, not in the future `next`
/// returns.
pub struct LineReader<R> {
    reader: R,
    max: usize,
    line: Vec<u8>,
    dropped: bool,
    done: bool,
}

impl<R: AsyncBufRead + Unpin> LineReader<R> {
    pub fn new(reader: R, max: usize) -> Self {
        Self {
            reader,
            max,
            line: Vec::new(),
            dropped: false,
            done: false,
        }
    }

    /// The next line, held to `max` bytes as [`read_lines`] holds it, or
    /// `None` at the end of the stream.
    pub async fn next(&mut self) -> std::io::Result<Option<Line>> {
        if self.done {
            return Ok(None);
        }
        // One byte over `max`, so a `\r` just past it is a line ending and not a cut.
        let keep = self.max.saturating_add(1);
        loop {
            let chunk = self.reader.fill_buf().await?;
            if chunk.is_empty() {
                self.done = true;
                if self.line.is_empty() && !self.dropped {
                    return Ok(None);
                }
                let dropped = std::mem::take(&mut self.dropped);
                return Ok(Some(finish(&mut self.line, dropped, false, self.max)));
            }
            let (part, used, ends) = match chunk.iter().position(|&b| b == b'\n') {
                Some(at) => (&chunk[..at], at + 1, true),
                None => (chunk, chunk.len(), false),
            };
            let room = keep.saturating_sub(self.line.len()).min(part.len());
            self.line.extend_from_slice(&part[..room]);
            self.dropped |= room < part.len();
            self.reader.consume_unpin(used);
            if ends {
                let dropped = std::mem::take(&mut self.dropped);
                return Ok(Some(finish(&mut self.line, dropped, true, self.max)));
            }
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
pub async fn read_lines<R, F>(reader: R, max: usize, mut on_line: F) -> std::io::Result<()>
where
    R: AsyncBufRead + Unpin,
    F: FnMut(Line),
{
    let mut lines = LineReader::new(reader, max);
    while let Some(line) = lines.next().await? {
        on_line(line);
    }
    Ok(())
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
    /// The pod. Give exactly one of `pod`, `selector` (with
    /// `matchExpressions`) or `workload`; a read without page mode needs `pod`.
    #[serde(default)]
    pub pod: Option<String>,
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
    /// Page mode: start at this RFC 3339 time. Not with `since_seconds`.
    #[serde(default)]
    pub since_time: Option<String>,
    /// Page mode: every pod this equality label selector matches, e.g.
    /// `{ "app": "cmi" }` — at most 20, merged in time order, each line
    /// prefixed `[pod] `.
    #[serde(default)]
    pub selector: Option<BTreeMap<String, String>>,
    /// Page mode: the set-based half of the selector, as `k8s.podsForSelector`
    /// takes it.
    #[serde(default, rename = "matchExpressions")]
    pub match_expressions: Option<Vec<LabelSelectorRequirement>>,
    /// Page mode: every pod of this Deployment, StatefulSet, DaemonSet,
    /// ReplicaSet or Job, read as `selector` would read them.
    #[serde(default)]
    pub workload: Option<WorkloadRef>,
    /// Page mode: read on from the page this came with (its `next_cursor`).
    /// With one, `tail_lines`, `all_lines`, `since_seconds` and `since_time`
    /// are ignored.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Page mode: keep only lines matching this regex (Rust `regex` syntax;
    /// `(?i)` ignores case), searched on the server across the whole log.
    #[serde(default)]
    pub grep: Option<String>,
    /// Page mode: lines kept before and after each `grep` match, 0–10.
    #[serde(default)]
    pub context_lines: Option<u32>,
    /// Page mode: the most bytes of `logs` one page holds, 1,000–40,000
    /// (default 12,000). Every page-mode answer fits srelens' 50,000-byte cap.
    #[serde(default)]
    pub max_bytes: Option<u32>,
    /// Page mode: once the end of the log is reached and the page has room,
    /// wait up to this many seconds (0–60) for new lines — a bounded follow.
    #[serde(default)]
    pub wait_seconds: Option<u32>,
}

impl PodLogsIn {
    /// Any field page mode adds turns it on; without one, the call is the
    /// blob it always was.
    pub fn page_mode(&self) -> bool {
        self.since_time.is_some()
            || self.selector.is_some()
            || self.match_expressions.is_some()
            || self.workload.is_some()
            || self.cursor.is_some()
            || self.grep.is_some()
            || self.context_lines.is_some()
            || self.max_bytes.is_some()
            || self.wait_seconds.is_some()
    }
}

/// A workload whose pods a page-mode read covers.
#[derive(Debug, Clone, PartialEq, Deserialize, JsonSchema)]
pub struct WorkloadRef {
    /// `Deployment`, `StatefulSet`, `DaemonSet`, `ReplicaSet` or `Job`.
    pub kind: String,
    pub name: String,
}

/// A pod's log: all of what was asked for, or — in page mode — one page of
/// it and where the page stopped.
#[derive(Debug, Serialize, JsonSchema)]
pub struct PodLogsOut {
    pub logs: String,
    /// Page mode only: where the page stopped, and what happened on the way.
    #[serde(flatten)]
    pub page: Option<crate::log_pages::Page>,
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
                if input.page_mode() {
                    return crate::log_pages::read_page(&cache, input).await;
                }
                let pod = input.pod.clone().ok_or_else(|| {
                    CapabilityError::InvalidInput(
                        "`pod` is required unless a page-mode field (`selector`, `workload`, `cursor`, `grep`, `max_bytes`, …) is given".into(),
                    )
                })?;
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
                let logs = tokio::time::timeout(request_timeout(), api.logs(&pod, &params))
                    .await
                    .map_err(|_| CapabilityError::Handler("fetch logs timed out".into()))?
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                Ok(PodLogsOut { logs, page: None })
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

    /// A page stops between lines and reads on later: `next` hands out one
    /// line per call, however the bytes arrive, and `None` from the end on.
    #[tokio::test]
    async fn a_line_reader_hands_out_one_line_a_call() {
        let reader = futures::io::BufReader::with_capacity(
            3,
            futures::io::Cursor::new(b"one\ntwo\nthree".to_vec()),
        );
        let mut lines = LineReader::new(reader, 64);
        assert_eq!(lines.next().await.unwrap(), Some(whole("one")));
        assert_eq!(lines.next().await.unwrap(), Some(whole("two")));
        assert_eq!(lines.next().await.unwrap(), Some(whole("three")));
        assert_eq!(lines.next().await.unwrap(), None);
        assert_eq!(lines.next().await.unwrap(), None);
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
    fn without_all_lines_the_tail_defaults_to_200_or_the_given_count() {
        // The default path — every existing caller, MCP agents included — is
        // unchanged: omit both and you still get the last 200 lines.
        assert_eq!(effective_tail_lines(false, None), Some(DEFAULT_TAIL_LINES));
        assert_eq!(effective_tail_lines(false, Some(50)), Some(50));
    }

    /// A blob read's answer is what it always was: `logs` and nothing else.
    #[test]
    fn a_blob_answer_carries_logs_alone() {
        let out = PodLogsOut {
            logs: "a\n".into(),
            page: None,
        };
        assert_eq!(
            serde_json::to_value(&out).unwrap(),
            serde_json::json!({ "logs": "a\n" })
        );
    }

    /// Any field page mode adds turns it on; the blob fields alone never do.
    #[test]
    fn every_field_page_mode_adds_turns_it_on_and_the_blob_fields_do_not() {
        use serde_json::json;
        let with = |extra: serde_json::Value| {
            let mut v = json!({ "context": "c", "namespace": "n", "pod": "p" });
            v.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            serde_json::from_value::<PodLogsIn>(v).unwrap()
        };
        for blob in [
            json!({}),
            json!({ "container": "app" }),
            json!({ "tail_lines": 5 }),
            json!({ "all_lines": true }),
            json!({ "previous": true }),
            json!({ "timestamps": true }),
            json!({ "since_seconds": 60 }),
        ] {
            assert!(!with(blob.clone()).page_mode(), "{blob}");
        }
        for page in [
            json!({ "cursor": "x" }),
            json!({ "grep": "x" }),
            json!({ "context_lines": 1 }),
            json!({ "max_bytes": 2000 }),
            json!({ "wait_seconds": 1 }),
            json!({ "since_time": "2026-10-16T00:00:00Z" }),
            json!({ "selector": { "app": "x" } }),
            json!({ "matchExpressions": [] }),
            json!({ "workload": { "kind": "Deployment", "name": "x" } }),
        ] {
            assert!(with(page.clone()).page_mode(), "{page}");
        }
    }

    /// `pod` became optional for page mode; a blob read without one is
    /// refused naming what to give, as serde's "missing field" used to.
    #[tokio::test]
    async fn a_blob_read_without_a_pod_is_refused() {
        let cap = pod_logs_capability(ClientCache::new(PathBuf::from("/x")));
        let err = (cap.handler)(serde_json::json!({ "context": "c", "namespace": "n" }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("`pod`"), "{err}");
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
