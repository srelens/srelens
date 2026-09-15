//! Out-of-band SSH node diagnostic and repair capabilities (#ssh-node-tools).
//!
//! When node system services like `rke2-server`, `rke2-agent`, or `kubelet` fail,
//! Kubernetes API-based debugging (`kubectl debug`, debug pods) is impossible.
//! These capabilities execute out-of-band SSH diagnostics and controlled service restarts
//! using the system's `ssh` binary and configuration.

use std::sync::Arc;
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use srelens_capability::{Annotations, Capability, CapabilityError};

use crate::client_cache::ClientCache;

const DEFAULT_SSH_TIMEOUT_SECS: u64 = 15;

/// Validate that a host/node name does not attempt SSH flag injection or contain shell metacharacters.
pub fn validate_node(node: &str) -> Result<(), CapabilityError> {
    let trimmed = node.trim();
    if trimmed.is_empty() {
        return Err(CapabilityError::InvalidInput(
            "Node address cannot be empty".into(),
        ));
    }
    if trimmed.starts_with('-') {
        return Err(CapabilityError::InvalidInput(format!(
            "Node address '{node}' must not start with '-'"
        )));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':'))
    {
        return Err(CapabilityError::InvalidInput(format!(
            "Node address '{node}' contains invalid characters"
        )));
    }
    Ok(())
}

/// Validate that a systemd service name contains only safe identifiers.
pub fn validate_service(service: &str) -> Result<(), CapabilityError> {
    let trimmed = service.trim();
    if trimmed.is_empty() {
        return Err(CapabilityError::InvalidInput(
            "Service name cannot be empty".into(),
        ));
    }
    if trimmed.starts_with('-') {
        return Err(CapabilityError::InvalidInput(format!(
            "Service name '{service}' must not start with '-'"
        )));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '@'))
    {
        return Err(CapabilityError::InvalidInput(format!(
            "Service name '{service}' contains invalid characters (only alphanumeric, ., -, _, @ allowed)"
        )));
    }
    Ok(())
}

/// Validate SSH username.
pub fn validate_user(user: Option<&str>) -> Result<(), CapabilityError> {
    if let Some(u) = user {
        let trimmed = u.trim();
        if trimmed.is_empty() {
            return Err(CapabilityError::InvalidInput(
                "User cannot be empty if specified".into(),
            ));
        }
        if trimmed.starts_with('-') {
            return Err(CapabilityError::InvalidInput(format!(
                "User '{u}' must not start with '-'"
            )));
        }
        if !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
        {
            return Err(CapabilityError::InvalidInput(format!(
                "User '{u}' contains invalid characters"
            )));
        }
    }
    Ok(())
}

/// Validate identity file path.
pub fn validate_identity_file(path: Option<&str>) -> Result<(), CapabilityError> {
    if let Some(p) = path {
        let trimmed = p.trim();
        if trimmed.is_empty() {
            return Err(CapabilityError::InvalidInput(
                "Identity file path cannot be empty".into(),
            ));
        }
        if trimmed.starts_with('-') {
            return Err(CapabilityError::InvalidInput(format!(
                "Identity file path '{p}' must not start with '-'"
            )));
        }
    }
    Ok(())
}

/// Validate `since` parameter for journalctl.
pub fn validate_since(since: Option<&str>) -> Result<(), CapabilityError> {
    if let Some(s) = since {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Ok(());
        }
        if trimmed.starts_with('-') {
            return Err(CapabilityError::InvalidInput(format!(
                "Time window '{s}' must not start with '-'"
            )));
        }
        if !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | ':' | '+' | '/'))
        {
            return Err(CapabilityError::InvalidInput(format!(
                "Time window '{s}' contains invalid characters"
            )));
        }
    }
    Ok(())
}

/// Validate `grep` filter for journalctl.
pub fn validate_grep(grep: Option<&str>) -> Result<(), CapabilityError> {
    if let Some(g) = grep {
        if g.contains('\n')
            || g.contains('\r')
            || g.contains(';')
            || g.contains('&')
            || g.contains('|')
            || g.contains('`')
            || g.contains('$')
        {
            return Err(CapabilityError::InvalidInput(
                "Grep pattern contains disallowed shell metacharacters".into(),
            ));
        }
    }
    Ok(())
}

/// Resolves a node name to an IP address if available, falling back cleanly to the input name.
pub async fn resolve_node_target(
    cache: &Arc<ClientCache>,
    context: Option<&str>,
    node: &str,
) -> String {
    // If it is already an IP address, use it directly without K8s query
    if node.parse::<std::net::IpAddr>().is_ok() {
        return node.to_string();
    }
    if let Some(ctx) = context {
        if let Ok(client) = cache.get(ctx).await {
            let node_api: kube::Api<k8s_openapi::api::core::v1::Node> = kube::Api::all(client);
            if let Ok(Ok(node_obj)) =
                tokio::time::timeout(Duration::from_secs(3), node_api.get(node)).await
            {
                if let Some(addresses) = node_obj.status.as_ref().and_then(|s| s.addresses.as_ref())
                {
                    if let Some(internal) = addresses
                        .iter()
                        .find(|a| a.type_ == "InternalIP")
                        .map(|a| a.address.clone())
                    {
                        return internal;
                    }
                    if let Some(external) = addresses
                        .iter()
                        .find(|a| a.type_ == "ExternalIP")
                        .map(|a| a.address.clone())
                    {
                        return external;
                    }
                    if let Some(hostname) = addresses
                        .iter()
                        .find(|a| a.type_ == "Hostname")
                        .map(|a| a.address.clone())
                    {
                        return hostname;
                    }
                }
            }
        }
    }
    node.to_string()
}

/// Builds argument list for non-interactive `ssh` batch execution.
pub fn build_ssh_args(
    target_host: &str,
    user: Option<&str>,
    port: Option<u16>,
    identity_file: Option<&str>,
    remote_cmd: &str,
) -> Vec<String> {
    let mut args = vec![
        "-o".to_string(),
        "BatchMode=yes".to_string(),
        "-o".to_string(),
        "ConnectTimeout=10".to_string(),
        "-o".to_string(),
        "StrictHostKeyChecking=accept-new".to_string(),
    ];
    if let Some(p) = port {
        args.push("-p".to_string());
        args.push(p.to_string());
    }
    if let Some(id) = identity_file {
        args.push("-i".to_string());
        args.push(id.to_string());
    }
    let destination = if let Some(u) = user {
        format!("{u}@{target_host}")
    } else {
        target_host.to_string()
    };
    args.push(destination);
    args.push(remote_cmd.to_string());
    args
}

/// Asynchronously runs an SSH command with timeout, capturing stdout, stderr, and exit code.
pub async fn run_ssh_command(
    args: &[String],
    timeout_secs: u64,
) -> Result<(String, String, i32), CapabilityError> {
    let mut cmd = tokio::process::Command::new("ssh");
    cmd.args(args);
    cmd.kill_on_drop(true);

    let output = tokio::time::timeout(Duration::from_secs(timeout_secs), cmd.output())
        .await
        .map_err(|_| {
            CapabilityError::Handler(format!("SSH command timed out after {timeout_secs}s"))
        })?
        .map_err(|e| CapabilityError::Handler(format!("Failed to execute ssh: {e}")))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let code = output.status.code().unwrap_or(-1);

    Ok((stdout, stderr, code))
}

// ---------------------------------------------------------------------------
// 1. k8s.nodeServiceStatus
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct NodeServiceStatusIn {
    pub context: Option<String>,
    pub node: String,
    pub service: String,
    pub user: Option<String>,
    pub port: Option<u16>,
    #[serde(rename = "identityFile")]
    pub identity_file: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct NodeServiceStatusOut {
    pub active: bool,
    #[serde(rename = "loadState")]
    pub load_state: String,
    #[serde(rename = "activeState")]
    pub active_state: String,
    #[serde(rename = "subState")]
    pub sub_state: String,
    #[serde(rename = "rawOutput")]
    pub raw_output: String,
    #[serde(rename = "exitCode")]
    pub exit_code: i32,
}

/// Pure parser extracting service state details from `systemctl status` output.
pub fn parse_systemctl_status(output: &str, exit_code: i32) -> NodeServiceStatusOut {
    let mut load_state = "unknown".to_string();
    let mut active_state = "unknown".to_string();
    let mut sub_state = "unknown".to_string();
    let mut active = false;

    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Loaded:") {
            if let Some(rest) = trimmed.strip_prefix("Loaded:") {
                let parts: Vec<&str> = rest.trim().split_whitespace().collect();
                if let Some(first) = parts.first() {
                    load_state = first.trim_matches(|c| c == ';' || c == ',').to_string();
                }
            }
        } else if trimmed.starts_with("Active:") {
            if let Some(rest) = trimmed.strip_prefix("Active:") {
                let rest = rest.trim();
                let mut parts = rest.split_whitespace();
                if let Some(state) = parts.next() {
                    active_state = state.to_string();
                    if state == "active" {
                        active = true;
                    }
                }
                if let Some(start) = rest.find('(') {
                    if let Some(end) = rest.find(')') {
                        if start < end {
                            sub_state = rest[start + 1..end].trim().to_string();
                        }
                    }
                }
            }
        }
    }

    NodeServiceStatusOut {
        active,
        load_state,
        active_state,
        sub_state,
        raw_output: output.to_string(),
        exit_code,
    }
}

pub fn node_service_status_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<NodeServiceStatusIn, NodeServiceStatusOut, _, _>(
        "k8s.nodeServiceStatus",
        "check the status of a systemd service (e.g. rke2-server, kubelet) on a node via SSH",
        Annotations::SENSITIVE_READ,
        move |input: NodeServiceStatusIn| {
            let cache = cache.clone();
            async move {
                validate_node(&input.node)?;
                validate_service(&input.service)?;
                validate_user(input.user.as_deref())?;
                validate_identity_file(input.identity_file.as_deref())?;

                let target_host =
                    resolve_node_target(&cache, input.context.as_deref(), &input.node).await;
                let remote_cmd = format!("systemctl status {} --no-pager", input.service);
                let args = build_ssh_args(
                    &target_host,
                    input.user.as_deref(),
                    input.port,
                    input.identity_file.as_deref(),
                    &remote_cmd,
                );

                let (stdout, stderr, exit_code) =
                    run_ssh_command(&args, DEFAULT_SSH_TIMEOUT_SECS).await?;

                if exit_code == 255 && !stderr.is_empty() && stdout.is_empty() {
                    return Err(CapabilityError::Handler(format!(
                        "SSH connection to '{target_host}' failed: {}",
                        stderr.trim()
                    )));
                }

                let combined_output = if !stderr.is_empty() && !stdout.is_empty() {
                    format!("{stdout}\n{stderr}")
                } else if !stdout.is_empty() {
                    stdout
                } else {
                    stderr
                };

                Ok(parse_systemctl_status(&combined_output, exit_code))
            }
        },
    )
}

// ---------------------------------------------------------------------------
// 2. k8s.nodeJournalLogs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct NodeJournalLogsIn {
    pub context: Option<String>,
    pub node: String,
    pub service: String,
    pub lines: Option<u32>,
    pub since: Option<String>,
    pub grep: Option<String>,
    pub user: Option<String>,
    pub port: Option<u16>,
    #[serde(rename = "identityFile")]
    pub identity_file: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct NodeJournalLogsOut {
    pub logs: String,
    #[serde(rename = "linesReturned")]
    pub lines_returned: usize,
}

/// Quote `value` as one POSIX shell word. `ssh` hands the remote command to the
/// target's login shell as a single string, so a caller-supplied argument has to
/// survive that shell intact. Inside single quotes nothing is special — not `\`,
/// `"`, `>` or `#` — and a literal `'` closes the quote, adds an escaped one and
/// reopens. Escaping only `"` inside double quotes was not enough (#615).
pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

pub fn build_journalctl_command(
    service: &str,
    lines: u32,
    since: Option<&str>,
    grep: Option<&str>,
) -> String {
    let mut cmd = format!(
        "journalctl -u {} -n {} --no-pager",
        service,
        lines.min(2000)
    );
    if let Some(s) = since.filter(|s| !s.trim().is_empty()) {
        cmd.push_str(&format!(" --since {}", shell_quote(s.trim())));
    }
    if let Some(g) = grep.filter(|g| !g.trim().is_empty()) {
        cmd.push_str(&format!(" --grep {}", shell_quote(g.trim())));
    }
    cmd
}

pub fn node_journal_logs_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<NodeJournalLogsIn, NodeJournalLogsOut, _, _>(
        "k8s.nodeJournalLogs",
        "retrieve journalctl logs for a service on a node via SSH",
        Annotations::SENSITIVE_READ,
        move |input: NodeJournalLogsIn| {
            let cache = cache.clone();
            async move {
                validate_node(&input.node)?;
                validate_service(&input.service)?;
                validate_user(input.user.as_deref())?;
                validate_identity_file(input.identity_file.as_deref())?;
                validate_since(input.since.as_deref())?;
                validate_grep(input.grep.as_deref())?;

                let target_host =
                    resolve_node_target(&cache, input.context.as_deref(), &input.node).await;
                let lines = input.lines.unwrap_or(100);
                let remote_cmd = build_journalctl_command(
                    &input.service,
                    lines,
                    input.since.as_deref(),
                    input.grep.as_deref(),
                );

                let args = build_ssh_args(
                    &target_host,
                    input.user.as_deref(),
                    input.port,
                    input.identity_file.as_deref(),
                    &remote_cmd,
                );

                let (stdout, stderr, exit_code) =
                    run_ssh_command(&args, DEFAULT_SSH_TIMEOUT_SECS).await?;
                journal_logs_result(&target_host, stdout, &stderr, exit_code)
            }
        },
    )
}

/// Turn a remote `journalctl` run into logs or an error. A failed call and an
/// empty journal must not read as the same answer: `--since nonsense` exits 1
/// with "Failed to parse timestamp" on stderr, and returning that as zero lines
/// claims the node has no logs (#615). A `--grep` that matches nothing also
/// exits 1, but with nothing on stderr, so it stays an honest empty result.
pub fn journal_logs_result(
    target_host: &str,
    stdout: String,
    stderr: &str,
    exit_code: i32,
) -> Result<NodeJournalLogsOut, CapabilityError> {
    if exit_code == 255 && !stderr.is_empty() && stdout.is_empty() {
        return Err(CapabilityError::Handler(format!(
            "SSH connection to '{target_host}' failed: {}",
            stderr.trim()
        )));
    }
    // `StrictHostKeyChecking=accept-new` announces a first connection on
    // stderr; that notice is ssh talking, not journalctl failing.
    let journal_stderr = stderr
        .lines()
        .filter(|line| !line.starts_with("Warning: Permanently added"))
        .collect::<Vec<_>>()
        .join("\n");
    if exit_code != 0 && !journal_stderr.trim().is_empty() {
        return Err(CapabilityError::Handler(format!(
            "journalctl on '{target_host}' failed (exit {exit_code}): {}",
            journal_stderr.trim()
        )));
    }
    let line_count = stdout.lines().count();
    Ok(NodeJournalLogsOut {
        logs: stdout,
        lines_returned: line_count,
    })
}

// ---------------------------------------------------------------------------
// 3. k8s.nodeRuntimeDiagnostics
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct NodeRuntimeDiagnosticsIn {
    pub context: Option<String>,
    pub node: String,
    pub check: String,
    pub user: Option<String>,
    pub port: Option<u16>,
    #[serde(rename = "identityFile")]
    pub identity_file: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct NodeRuntimeDiagnosticsOut {
    pub check: String,
    pub command: String,
    pub output: String,
}

pub fn build_diagnostics_command(
    check: &str,
) -> Result<(&'static str, &'static str), CapabilityError> {
    match check.trim().to_ascii_lowercase().as_str() {
        "containers" => Ok((
            "containers",
            "crictl ps 2>/dev/null || nerdctl ps 2>/dev/null || docker ps 2>/dev/null || ctr containers list 2>/dev/null || echo 'No container runtime CLI (crictl/nerdctl/docker/ctr) found'",
        )),
        "dmesg" => Ok((
            "dmesg",
            "dmesg -T 2>/dev/null | tail -n 50 || dmesg | tail -n 50",
        )),
        "disk" => Ok(("disk", "df -h")),
        "memory" => Ok(("memory", "free -m")),
        "process" => Ok(("process", "ps aux --sort=-%mem 2>/dev/null | head -n 20 || ps aux | head -n 20")),
        other => Err(CapabilityError::InvalidInput(format!(
            "Unsupported diagnostic check '{other}'. Supported: containers, dmesg, disk, memory, process"
        ))),
    }
}

pub fn node_runtime_diagnostics_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<NodeRuntimeDiagnosticsIn, NodeRuntimeDiagnosticsOut, _, _>(
        "k8s.nodeRuntimeDiagnostics",
        "run non-invasive host diagnostics (containers, dmesg, disk, memory, process) on a node via SSH",
        Annotations::SENSITIVE_READ,
        move |input: NodeRuntimeDiagnosticsIn| {
            let cache = cache.clone();
            async move {
                validate_node(&input.node)?;
                validate_user(input.user.as_deref())?;
                validate_identity_file(input.identity_file.as_deref())?;

                let (check_name, remote_cmd) = build_diagnostics_command(&input.check)?;
                let target_host = resolve_node_target(&cache, input.context.as_deref(), &input.node).await;

                let args = build_ssh_args(
                    &target_host,
                    input.user.as_deref(),
                    input.port,
                    input.identity_file.as_deref(),
                    remote_cmd,
                );

                let (stdout, stderr, exit_code) = run_ssh_command(&args, DEFAULT_SSH_TIMEOUT_SECS).await?;

                if exit_code == 255 && !stderr.is_empty() && stdout.is_empty() {
                    return Err(CapabilityError::Handler(format!(
                        "SSH connection to '{target_host}' failed: {}",
                        stderr.trim()
                    )));
                }

                let output = if !stdout.is_empty() {
                    stdout
                } else {
                    stderr
                };

                Ok(NodeRuntimeDiagnosticsOut {
                    check: check_name.to_string(),
                    command: remote_cmd.to_string(),
                    output,
                })
            }
        },
    )
}

// ---------------------------------------------------------------------------
// 4. k8s.nodeServiceRestart (Confirm-Gated / Destructive)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct NodeServiceRestartIn {
    pub context: Option<String>,
    pub node: String,
    pub service: String,
    pub user: Option<String>,
    pub port: Option<u16>,
    #[serde(rename = "identityFile")]
    pub identity_file: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct NodeServiceRestartOut {
    pub success: bool,
    pub message: String,
    pub output: String,
}

pub fn node_service_restart_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<NodeServiceRestartIn, NodeServiceRestartOut, _, _>(
        "k8s.nodeServiceRestart",
        "restart a system service (e.g. rke2-server, kubelet) on a node via SSH (confirm-gated)",
        Annotations::DESTRUCTIVE,
        move |input: NodeServiceRestartIn| {
            let cache = cache.clone();
            async move {
                validate_node(&input.node)?;
                validate_service(&input.service)?;
                validate_user(input.user.as_deref())?;
                validate_identity_file(input.identity_file.as_deref())?;

                let target_host =
                    resolve_node_target(&cache, input.context.as_deref(), &input.node).await;
                let remote_cmd = format!("sudo systemctl restart {}", input.service);

                let args = build_ssh_args(
                    &target_host,
                    input.user.as_deref(),
                    input.port,
                    input.identity_file.as_deref(),
                    &remote_cmd,
                );

                let (stdout, stderr, exit_code) = run_ssh_command(&args, 30).await?;

                if exit_code == 255 && !stderr.is_empty() && stdout.is_empty() {
                    return Err(CapabilityError::Handler(format!(
                        "SSH connection to '{target_host}' failed: {}",
                        stderr.trim()
                    )));
                }

                let success = exit_code == 0;
                let message = if success {
                    format!(
                        "Successfully restarted service '{}' on node '{}'",
                        input.service, target_host
                    )
                } else {
                    format!(
                        "Failed to restart service '{}' on node '{}' (exit code: {})",
                        input.service, target_host, exit_code
                    )
                };

                let combined_output = if !stderr.is_empty() {
                    format!("{stdout}\n{stderr}").trim().to_string()
                } else {
                    stdout.trim().to_string()
                };

                Ok(NodeServiceRestartOut {
                    success,
                    message,
                    output: combined_output,
                })
            }
        },
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_node_addresses() {
        assert!(validate_node("10.0.0.1").is_ok());
        assert!(validate_node("node-worker-01").is_ok());
        assert!(validate_node("k8s.example.com").is_ok());
        assert!(validate_node("").is_err());
        assert!(validate_node("-oProxyCommand=bad").is_err());
        assert!(validate_node("worker; rm -rf /").is_err());
        assert!(validate_node("worker node").is_err());
    }

    #[test]
    fn validates_service_names() {
        assert!(validate_service("rke2-server").is_ok());
        assert!(validate_service("kubelet").is_ok());
        assert!(validate_service("containerd.service").is_ok());
        assert!(validate_service("getty@tty1").is_ok());
        assert!(validate_service("").is_err());
        assert!(validate_service("-oFlag").is_err());
        assert!(validate_service("rke2-server; reboot").is_err());
        assert!(validate_service("rke2 server").is_err());
    }

    #[test]
    fn validates_user_and_identity() {
        assert!(validate_user(None).is_ok());
        assert!(validate_user(Some("ubuntu")).is_ok());
        assert!(validate_user(Some("ec2-user")).is_ok());
        assert!(validate_user(Some("-oOption")).is_err());
        assert!(validate_user(Some("user; id")).is_err());

        assert!(validate_identity_file(None).is_ok());
        assert!(validate_identity_file(Some("/home/user/.ssh/id_rsa")).is_ok());
        assert!(validate_identity_file(Some("-oOption")).is_err());
    }

    #[test]
    fn builds_ssh_arguments_with_options() {
        let args = build_ssh_args("10.0.0.5", None, None, None, "uptime");
        assert_eq!(
            args,
            vec![
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=10",
                "-o",
                "StrictHostKeyChecking=accept-new",
                "10.0.0.5",
                "uptime"
            ]
        );

        let args_full = build_ssh_args(
            "worker-1",
            Some("root"),
            Some(2222),
            Some("/tmp/key.pem"),
            "systemctl status kubelet",
        );
        assert_eq!(
            args_full,
            vec![
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=10",
                "-o",
                "StrictHostKeyChecking=accept-new",
                "-p",
                "2222",
                "-i",
                "/tmp/key.pem",
                "root@worker-1",
                "systemctl status kubelet"
            ]
        );
    }

    #[test]
    fn builds_journalctl_commands() {
        let cmd = build_journalctl_command("rke2-server", 50, None, None);
        assert_eq!(cmd, "journalctl -u rke2-server -n 50 --no-pager");

        let cmd_filtered = build_journalctl_command("kubelet", 200, Some("10m ago"), Some("error"));
        assert_eq!(
            cmd_filtered,
            "journalctl -u kubelet -n 200 --no-pager --since '10m ago' --grep 'error'"
        );
    }

    #[test]
    fn journalctl_arguments_cannot_escape_their_quoting() {
        // The payload from #615: `validate_grep` lets it through, and under the
        // old `"`-escaping the backslash closed the quote and the rest became a
        // redirection on the node.
        let payload = r#"\" > /tmp/output #"#;
        assert!(validate_grep(Some(payload)).is_ok());
        assert_eq!(
            build_journalctl_command("kubelet", 10, None, Some(payload)),
            r#"journalctl -u kubelet -n 10 --no-pager --grep '\" > /tmp/output #'"#
        );
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }

    /// Quoting is only right if a real shell agrees. Run the built command with
    /// `journalctl` stubbed to print its arguments one per line: every argument
    /// must come back verbatim, and a redirection that escaped would swallow
    /// the output instead.
    #[cfg(unix)]
    #[test]
    fn a_posix_shell_reads_journalctl_arguments_back_verbatim() {
        for grep in [r#"\" > /dev/null #"#, r#"it's "quoted" \ $HOME `id`"#] {
            let cmd = build_journalctl_command("kubelet", 10, Some("10m ago"), Some(grep));
            let out = std::process::Command::new("sh")
                .arg("-c")
                .arg(format!("journalctl() {{ printf '%s\\n' \"$@\"; }}; {cmd}"))
                .output()
                .unwrap();
            let stdout = String::from_utf8(out.stdout).unwrap();
            assert_eq!(
                stdout.lines().collect::<Vec<_>>(),
                [
                    "-u",
                    "kubelet",
                    "-n",
                    "10",
                    "--no-pager",
                    "--since",
                    "10m ago",
                    "--grep",
                    grep
                ],
                "{cmd}"
            );
        }
    }

    #[test]
    fn builds_diagnostic_commands() {
        assert!(build_diagnostics_command("containers").is_ok());
        assert!(build_diagnostics_command("dmesg").is_ok());
        assert!(build_diagnostics_command("disk").is_ok());
        assert!(build_diagnostics_command("memory").is_ok());
        assert!(build_diagnostics_command("process").is_ok());
        assert!(build_diagnostics_command("invalid_check").is_err());
    }

    #[test]
    fn parses_systemctl_status_active() {
        let sample = r#"● rke2-server.service - Rancher Kubernetes Engine v2 (server)
     Loaded: loaded (/usr/local/lib/systemd/system/rke2-server.service; enabled; vendor preset: disabled)
     Active: active (running) since Mon 2026-09-14 18:22:01 UTC; 2h 45min ago
       Docs: https://docs.rke2.io
   Main PID: 1240 (rke2)
      Tasks: 120
     Memory: 2.1G
        "#;
        let parsed = parse_systemctl_status(sample, 0);
        assert!(parsed.active);
        assert_eq!(parsed.load_state, "loaded");
        assert_eq!(parsed.active_state, "active");
        assert_eq!(parsed.sub_state, "running");
        assert_eq!(parsed.exit_code, 0);
    }

    #[test]
    fn parses_systemctl_status_failed() {
        let sample = r#"× rke2-server.service - Rancher Kubernetes Engine v2 (server)
     Loaded: loaded (/usr/local/lib/systemd/system/rke2-server.service; enabled; vendor preset: disabled)
     Active: failed (Result: exit-code) since Mon 2026-09-14 20:01:05 UTC; 12min ago
    Process: 9912 ExecStart=/usr/local/bin/rke2 server (code=exited, status=1/FAILURE)
   Main PID: 9912 (code=exited, status=1/FAILURE)
        "#;
        let parsed = parse_systemctl_status(sample, 3);
        assert!(!parsed.active);
        assert_eq!(parsed.load_state, "loaded");
        assert_eq!(parsed.active_state, "failed");
        assert_eq!(parsed.sub_state, "Result: exit-code");
        assert_eq!(parsed.exit_code, 3);
    }

    #[test]
    fn parses_systemctl_status_inactive_dead() {
        let sample = r#"○ kubelet.service - kubelet: The Kubernetes Node Agent
     Loaded: loaded (/lib/systemd/system/kubelet.service; disabled; vendor preset: enabled)
     Active: inactive (dead)
        "#;
        let parsed = parse_systemctl_status(sample, 3);
        assert!(!parsed.active);
        assert_eq!(parsed.load_state, "loaded");
        assert_eq!(parsed.active_state, "inactive");
        assert_eq!(parsed.sub_state, "dead");
    }

    #[test]
    fn capability_annotations_match_safety_rules() {
        let cache = ClientCache::new(std::path::PathBuf::from("/dev/null"));
        // The reads change nothing, but service status, the journal and `ps`
        // output carry process arguments and log lines that can hold
        // credentials, so an MCP client must not get them unprompted (#615).
        for read in [
            node_service_status_capability(cache.clone()),
            node_journal_logs_capability(cache.clone()),
            node_runtime_diagnostics_capability(cache.clone()),
        ] {
            assert!(read.annotations.read_only, "{}", read.id);
            assert!(!read.annotations.destructive, "{}", read.id);
            assert!(read.annotations.requires_confirm, "{}", read.id);
            assert!(read.annotations.sensitive, "{}", read.id);
        }

        let restart = node_service_restart_capability(cache);
        assert!(!restart.annotations.read_only);
        assert!(restart.annotations.destructive);
        assert!(restart.annotations.requires_confirm);
    }

    /// Outputs captured from journalctl 259, the exit codes included.
    #[test]
    fn a_failed_journalctl_is_an_error_but_a_grep_with_no_match_is_not() {
        // An unparseable --since exits 1 and says why on stderr. Returning
        // that as zero lines would claim the node has no logs.
        let err =
            journal_logs_result("worker-1", String::new(), "Failed to parse timestamp: nonsense\n", 1)
                .unwrap_err()
                .to_string();
        assert!(err.contains("Failed to parse timestamp"), "{err}");
        assert!(err.contains("worker-1"), "{err}");

        // A --grep that matches nothing ALSO exits 1, with nothing on stderr.
        let no_match = journal_logs_result(
            "worker-1",
            "-- Boot 327c08b63df648a7a51850bde913f768 --\n".into(),
            "",
            1,
        );
        assert!(no_match.is_ok(), "{no_match:?}");

        // ssh's accept-new notice on a first connection is not journalctl failing.
        let first_contact = journal_logs_result(
            "worker-1",
            String::new(),
            "Warning: Permanently added 'worker-1' (ED25519) to the list of known hosts.\r\n",
            1,
        );
        assert!(first_contact.is_ok(), "{first_contact:?}");

        // A connection failure keeps its own message.
        let unreachable = journal_logs_result(
            "worker-1",
            String::new(),
            "ssh: connect to host worker-1 port 22: Connection refused\n",
            255,
        )
        .unwrap_err()
        .to_string();
        assert!(unreachable.contains("SSH connection to 'worker-1' failed"), "{unreachable}");

        let logs = journal_logs_result("worker-1", "one\ntwo\n".into(), "", 0).unwrap();
        assert_eq!(logs.lines_returned, 2);
    }

    #[test]
    fn deserializes_caller_wire_payloads_with_camel_case() {
        let json_payload = serde_json::json!({
            "context": "kind-dev",
            "node": "worker-1",
            "service": "rke2-server",
            "user": "ubuntu",
            "port": 22,
            "identityFile": "/path/to/key.pem"
        });
        let status_in: NodeServiceStatusIn = serde_json::from_value(json_payload.clone()).unwrap();
        assert_eq!(status_in.identity_file.as_deref(), Some("/path/to/key.pem"));

        let restart_in: NodeServiceRestartIn = serde_json::from_value(json_payload).unwrap();
        assert_eq!(
            restart_in.identity_file.as_deref(),
            Some("/path/to/key.pem")
        );
    }

    #[test]
    fn validates_since_window() {
        assert!(validate_since(None).is_ok());
        assert!(validate_since(Some("")).is_ok());
        assert!(validate_since(Some("  ")).is_ok());
        assert!(validate_since(Some("10m ago")).is_ok());
        assert!(validate_since(Some("2026-09-14 18:00:00")).is_ok());
        assert!(validate_since(Some("-1 hour")).is_err());
        assert!(validate_since(Some("10m; rm -rf /")).is_err());
    }

    #[test]
    fn validates_grep_pattern() {
        assert!(validate_grep(None).is_ok());
        assert!(validate_grep(Some("error")).is_ok());
        assert!(validate_grep(Some("connection refused")).is_ok());
        assert!(validate_grep(Some("error\nrm -rf /")).is_err());
        assert!(validate_grep(Some("error\r\n")).is_err());
        assert!(validate_grep(Some("error; reboot")).is_err());
        assert!(validate_grep(Some("error && reboot")).is_err());
        assert!(validate_grep(Some("error | cat")).is_err());
        assert!(validate_grep(Some("`whoami`")).is_err());
        assert!(validate_grep(Some("$HOME")).is_err());
    }

    #[tokio::test]
    async fn resolve_node_target_short_circuits_on_ip_without_kube_lookup() {
        let cache = ClientCache::new(std::path::PathBuf::from("/dev/null"));

        // An IP address is used as-is, with no context/K8s lookup attempted.
        assert_eq!(
            resolve_node_target(&cache, None, "10.0.0.5").await,
            "10.0.0.5"
        );
        assert_eq!(
            resolve_node_target(&cache, Some("some-ctx"), "192.168.1.20").await,
            "192.168.1.20"
        );

        // A non-IP node name with no context falls back to the name unchanged.
        assert_eq!(
            resolve_node_target(&cache, None, "worker-01").await,
            "worker-01"
        );
    }

    #[test]
    fn parses_systemctl_status_empty_and_malformed() {
        let parsed = parse_systemctl_status("", 1);
        assert!(!parsed.active);
        assert_eq!(parsed.load_state, "unknown");
        assert_eq!(parsed.active_state, "unknown");
        assert_eq!(parsed.sub_state, "unknown");
        assert_eq!(parsed.exit_code, 1);

        let parsed2 = parse_systemctl_status("Loaded:\nActive: unknown", 0);
        assert!(!parsed2.active);
        assert_eq!(parsed2.load_state, "unknown");
        assert_eq!(parsed2.active_state, "unknown");
        assert_eq!(parsed2.sub_state, "unknown");
    }

    #[tokio::test]
    async fn capabilities_reject_invalid_inputs() {
        let cache = ClientCache::new(std::path::PathBuf::from("/dev/null"));

        // 1. Status capability
        let status = node_service_status_capability(cache.clone());
        assert!(
            (status.handler)(serde_json::json!({ "node": "-bad", "service": "rke2" }))
                .await
                .is_err()
        );
        assert!(
            (status.handler)(serde_json::json!({ "node": "worker-1", "service": "-bad" }))
                .await
                .is_err()
        );
        assert!((status.handler)(
            serde_json::json!({ "node": "worker-1", "service": "rke2", "user": "-bad" })
        )
        .await
        .is_err());
        assert!((status.handler)(
            serde_json::json!({ "node": "worker-1", "service": "rke2", "identityFile": "-bad" })
        )
        .await
        .is_err());

        // 2. Logs capability
        let logs = node_journal_logs_capability(cache.clone());
        assert!(
            (logs.handler)(serde_json::json!({ "node": "-bad", "service": "kubelet" }))
                .await
                .is_err()
        );
        assert!((logs.handler)(
            serde_json::json!({ "node": "worker-1", "service": "kubelet", "since": "-bad" })
        )
        .await
        .is_err());
        assert!((logs.handler)(
            serde_json::json!({ "node": "worker-1", "service": "kubelet", "grep": "`bad`" })
        )
        .await
        .is_err());

        // 3. Diagnostics capability
        let diag = node_runtime_diagnostics_capability(cache.clone());
        assert!(
            (diag.handler)(serde_json::json!({ "node": "-bad", "check": "disk" }))
                .await
                .is_err()
        );
        assert!((diag.handler)(
            serde_json::json!({ "node": "worker-1", "check": "invalid_check" })
        )
        .await
        .is_err());

        // 4. Restart capability
        let restart = node_service_restart_capability(cache);
        assert!(
            (restart.handler)(serde_json::json!({ "node": "-bad", "service": "rke2" }))
                .await
                .is_err()
        );
        assert!(
            (restart.handler)(serde_json::json!({ "node": "worker-1", "service": "-bad" }))
                .await
                .is_err()
        );
    }
}
