//! Interactive in-pod exec via kube-rs. Opens a TTY exec session to a
//! container and pumps stdout to a callback while forwarding stdin from a
//! channel — Tauri-agnostic so the streaming logic stays reusable.

use std::sync::Arc;
use std::time::Duration;

use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Status;
use kube::api::{AttachParams, TerminalSize};
use kube::Api;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc::Receiver;

/// Map an xterm `(cols, rows)` to a kube [`TerminalSize`]. Columns are the
/// width and rows are the height — a pure helper so the mapping can't silently
/// get swapped.
pub fn terminal_size(cols: u16, rows: u16) -> TerminalSize {
    TerminalSize { width: cols, height: rows }
}

use crate::client_cache::ClientCache;

/// Candidate shells to try, in order — busybox images often lack bash.
pub fn shell_command(requested: Option<&str>) -> Vec<String> {
    match requested {
        Some(s) if !s.is_empty() => vec![s.to_string()],
        _ => vec!["/bin/sh".to_string()],
    }
}

/// Whether `container` (or any container, if `None`) is in the Running state —
/// checks both regular and ephemeral container statuses. Exec into a container
/// that isn't running yet fails the WebSocket upgrade with a 500, so we wait for
/// this before attaching.
pub fn container_running(pod: &Pod, container: Option<&str>) -> bool {
    let Some(status) = pod.status.as_ref() else {
        return false;
    };
    let running = |statuses: &[k8s_openapi::api::core::v1::ContainerStatus]| {
        statuses.iter().any(|cs| {
            (container.is_none() || container == Some(cs.name.as_str()))
                && cs.state.as_ref().and_then(|s| s.running.as_ref()).is_some()
        })
    };
    status.container_statuses.as_deref().is_some_and(running)
        || status.ephemeral_container_statuses.as_deref().is_some_and(running)
}

/// Poll until `container` is running, or a deadline passes. Gives debug
/// containers / node debug pods time to start before we exec into them.
async fn wait_for_running(
    api: &Api<Pod>,
    pod: &str,
    container: Option<&str>,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let p = api.get(pod).await.map_err(|e| e.to_string())?;
        if container_running(&p, container) {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "container did not start within {}s",
                timeout.as_secs()
            ));
        }
        tokio::time::sleep(Duration::from_millis(700)).await;
    }
}

/// The exec protocol's status channel, read as an error or as nothing.
///
/// A container without the shell we asked for still *opens* the connection —
/// the API call succeeds, the command fails inside the container, and stdout
/// closes at once. Only this channel distinguishes that from a reader typing
/// `exit`, so it is the sole place a dead shell can be named.
///
/// Silent unless the cluster said `Failure`: a clean exit is not an error, and
/// a status that never arrived (older server, connection dropped during
/// teardown) is an absence of evidence rather than evidence of a failure.
/// The wording returned is the cluster's own, unwrapped, because the frontend
/// classifies the sentence and a prefix of ours would hide it.
pub fn status_error(status: Option<&Status>) -> Option<String> {
    let status = status?;
    if !status.status.as_deref().is_some_and(|s| s.eq_ignore_ascii_case("Failure")) {
        return None;
    }
    let said = |field: &Option<String>| field.as_deref().filter(|s| !s.is_empty()).map(str::to_string);
    // A failure the cluster worded poorly is still a failure, so fall through
    // its own words before settling for ours — going silent here would restore
    // the blank pane this exists to prevent.
    said(&status.message)
        .or_else(|| said(&status.reason))
        .or_else(|| Some("exec: command failed".to_string()))
}

/// Open an interactive exec session. `on_output` receives stdout chunks
/// (lossy UTF-8); `input_rx` yields stdin keystrokes. Runs until either side
/// closes or the task is aborted.
#[allow(clippy::too_many_arguments)]
pub async fn exec_shell<F>(
    cache: Arc<ClientCache>,
    context: String,
    namespace: String,
    pod: String,
    container: Option<String>,
    shell: Option<String>,
    command: Option<Vec<String>>,
    initial_size: Option<(u16, u16)>,
    mut resize_rx: Receiver<(u16, u16)>,
    mut on_output: F,
    mut input_rx: Receiver<String>,
) -> Result<(), String>
where
    F: FnMut(String) + Send,
{
    let client = cache.get(&context).await?;
    let api: Api<Pod> = Api::namespaced(client, &namespace);
    let target = container.filter(|c| !c.is_empty());

    // Debug containers and node debug pods take a moment to start; exec before
    // that 500s the WebSocket upgrade. Wait briefly for the target to run.
    wait_for_running(&api, &pod, target.as_deref(), Duration::from_secs(30)).await?;

    let mut params = AttachParams::default()
        .stdin(true)
        .stdout(true)
        .stderr(false)
        .tty(true);
    // Target a specific container when asked (multi-container / sidecar pods);
    // otherwise the API defaults to the pod's first container.
    if let Some(container) = target {
        params = params.container(container);
    }

    // An explicit `command` (e.g. the node shell's `nsenter …`) overrides the
    // default login shell.
    let command = command.filter(|c| !c.is_empty()).unwrap_or_else(|| shell_command(shell.as_deref()));
    let mut attached = api
        .exec(&pod, command, &params)
        .await
        .map_err(|e| e.to_string())?;

    // Resize channel (present because `tty(true)` above). Send the initial size
    // so the remote PTY starts matching the panel, then forward later resizes.
    // `terminal_size()` yields a futures `Sender`; `try_send` keeps resizes
    // non-blocking and best-effort.
    let mut size_tx = attached.terminal_size();
    if let (Some(tx), Some((cols, rows))) = (size_tx.as_mut(), initial_size) {
        let _ = tx.try_send(terminal_size(cols, rows));
    }

    // Taken before the loop because `join()` below drops the receiver: the
    // status frame arrives as the remote command ends, and awaiting it after
    // the join is what turns a shell that never started into a reason.
    let status = attached.take_status();

    let mut stdout = attached.stdout().ok_or_else(|| "exec: no stdout".to_string())?;
    let mut stdin = attached.stdin().ok_or_else(|| "exec: no stdin".to_string())?;
    let mut buf = vec![0u8; 8192];
    // Once the resize channel closes we stop polling it (a closed `recv()`
    // returns immediately, which would otherwise spin the `select!`). A closed
    // resize channel must NOT end the session, unlike stdin closing.
    let mut resize_open = true;

    loop {
        tokio::select! {
            read = stdout.read(&mut buf) => match read {
                Ok(0) => break,
                Ok(n) => on_output(String::from_utf8_lossy(&buf[..n]).to_string()),
                Err(_) => break,
            },
            msg = input_rx.recv() => match msg {
                Some(data) => {
                    if stdin.write_all(data.as_bytes()).await.is_err() {
                        break;
                    }
                    let _ = stdin.flush().await;
                }
                None => break,
            },
            size = resize_rx.recv(), if resize_open => match size {
                // Resize the remote PTY; best-effort, never fatal.
                Some((cols, rows)) => {
                    if let Some(tx) = size_tx.as_mut() {
                        let _ = tx.try_send(terminal_size(cols, rows));
                    }
                }
                None => resize_open = false,
            },
        }
    }

    // Join first: it drops the streams the background task may still be
    // writing to, so the task can finish and post its status. Awaiting the
    // status before that would deadlock against a full stdout buffer.
    let _ = attached.join().await;
    let status = match status {
        Some(status) => status.await,
        None => None,
    };
    match status_error(status.as_ref()) {
        Some(reason) => Err(reason),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_command_defaults_to_sh() {
        assert_eq!(shell_command(None), vec!["/bin/sh".to_string()]);
        assert_eq!(shell_command(Some("")), vec!["/bin/sh".to_string()]);
        assert_eq!(shell_command(Some("/bin/bash")), vec!["/bin/bash".to_string()]);
    }

    #[test]
    fn terminal_size_maps_cols_to_width_and_rows_to_height() {
        // xterm reports (cols, rows); the k8s TerminalSize is (width, height).
        // Columns are width and rows are height — guard against swapping them.
        let size = terminal_size(120, 40);
        assert_eq!(size.width, 120);
        assert_eq!(size.height, 40);
    }

    fn pod_with(json: serde_json::Value) -> Pod {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn container_running_checks_regular_and_ephemeral_statuses() {
        let running = pod_with(serde_json::json!({
            "status": { "containerStatuses": [{ "name": "app", "image": "x", "imageID": "", "ready": true, "restartCount": 0, "state": { "running": { "startedAt": "2020-01-01T00:00:00Z" } } }] }
        }));
        assert!(container_running(&running, Some("app")));
        assert!(container_running(&running, None));
        assert!(!container_running(&running, Some("other")));

        let ephemeral = pod_with(serde_json::json!({
            "status": { "ephemeralContainerStatuses": [{ "name": "debugger-1", "image": "x", "imageID": "", "ready": false, "restartCount": 0, "state": { "running": { "startedAt": "2020-01-01T00:00:00Z" } } }] }
        }));
        assert!(container_running(&ephemeral, Some("debugger-1")));

        let waiting = pod_with(serde_json::json!({
            "status": { "containerStatuses": [{ "name": "app", "image": "x", "imageID": "", "ready": false, "restartCount": 0, "state": { "waiting": { "reason": "ContainerCreating" } } }] }
        }));
        assert!(!container_running(&waiting, Some("app")));

        assert!(!container_running(&pod_with(serde_json::json!({})), None));
    }

    fn status_with(json: serde_json::Value) -> Status {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn status_error_reports_the_clusters_own_sentence_on_failure() {
        // The coredns case: no /bin/sh in the image. The exec API call itself
        // succeeded, so only this channel says why the shell never started.
        let status = status_with(serde_json::json!({
            "status": "Failure",
            "message": "command terminated with exit code 126",
            "reason": "NonZeroExitCode",
        }));
        // Verbatim: `describeError` on the frontend classifies this wording,
        // so a prefix here would hide the sentence it needs to read.
        assert_eq!(
            status_error(Some(&status)).as_deref(),
            Some("command terminated with exit code 126")
        );
    }

    #[test]
    fn status_error_stays_silent_on_a_clean_exit() {
        // A reader who typed `exit` has not hit an error, and an alert on the
        // commonest possible action would be pure noise.
        let status = status_with(serde_json::json!({ "status": "Success" }));
        assert_eq!(status_error(Some(&status)), None);
    }

    #[test]
    fn status_error_treats_a_missing_status_as_no_evidence() {
        // An older server, or a connection dropped mid-teardown, yields nothing
        // on the channel. An absence is not a failure — same rule that makes a
        // missing metric read "no reading" rather than zero.
        assert_eq!(status_error(None), None);
        // Likewise a status object that never says which way it went.
        let unsaid = status_with(serde_json::json!({ "message": "" }));
        assert_eq!(status_error(Some(&unsaid)), None);
    }

    #[test]
    fn status_error_falls_back_to_the_reason_when_the_message_is_empty() {
        // A failure the cluster worded poorly is still a failure; reporting it
        // silently would put us right back to the blank pane. `reason` is still
        // the cluster's own word, so prefer it before anything we invent.
        let no_message = status_with(serde_json::json!({
            "status": "Failure",
            "reason": "InternalError",
        }));
        assert_eq!(status_error(Some(&no_message)).as_deref(), Some("InternalError"));

        let wordless = status_with(serde_json::json!({ "status": "Failure" }));
        assert_eq!(status_error(Some(&wordless)).as_deref(), Some("exec: command failed"));
    }
}
