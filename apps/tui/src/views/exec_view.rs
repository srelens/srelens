use std::io::Read;
use std::process::{Command, Stdio};

pub struct ExecRunner;

impl ExecRunner {
    /// Cleans and extracts the human-actionable error message from raw exec/debug stderr.
    pub fn clean_exec_stderr(raw: &str) -> String {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return String::new();
        }

        // If it contains OCI runtime exec failure or container process startup failure, extract innermost cause
        if let Some(pos) = trimmed.find("unable to start container process:") {
            let tail = trimmed[pos + "unable to start container process:".len()..].trim();
            if !tail.is_empty() {
                return tail.to_string();
            }
        }
        if let Some(pos) = trimmed.find("OCI runtime exec failed:") {
            let tail = trimmed[pos + "OCI runtime exec failed:".len()..].trim();
            if !tail.is_empty() {
                return tail.to_string();
            }
        }

        // Strip redundant nested "error: Internal error occurred:" prefixes
        let mut msg = trimmed;
        while let Some(stripped) = msg
            .strip_prefix("error: Internal error occurred:")
            .or_else(|| msg.strip_prefix("Internal error occurred:"))
            .or_else(|| msg.strip_prefix("error executing command in container:"))
            .or_else(|| msg.strip_prefix("failed to exec in container:"))
            .or_else(|| msg.strip_prefix("error: "))
        {
            msg = stripped.trim();
        }

        let first_line = msg.lines().next().unwrap_or(msg).trim();
        if first_line.is_empty() {
            trimmed.to_string()
        } else {
            first_line.to_string()
        }
    }

    /// Checks if an error indicates that the shell binary is missing in the container.
    pub fn is_missing_shell_err(err: &str) -> bool {
        let lower = err.to_lowercase();
        (lower.contains("stat ") || lower.contains("not found in $path") || lower.contains("no such file or directory"))
            && (lower.contains("exec:")
                || lower.contains("exec failed")
                || lower.contains("container process")
                || lower.contains("oci runtime")
                || lower.contains("executable file not found"))
    }

    /// Runs an interactive command with stdin/stdout inherited and stderr captured in memory.
    /// This prevents raw error text from corrupting the terminal primary screen while capturing
    /// any startup failures for helpful user toast feedback.
    pub fn run_interactive_command(mut cmd: Command) -> Result<(), String> {
        cmd.stdin(Stdio::inherit());
        cmd.stdout(Stdio::inherit());
        cmd.stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("Failed to spawn command: {}", e))?;

        let stderr_handle = child.stderr.take().map(|mut stderr| {
            std::thread::spawn(move || {
                let mut buf = Vec::new();
                let _ = stderr.read_to_end(&mut buf);
                buf
            })
        });

        let status = child
            .wait()
            .map_err(|e| format!("Command execution error: {}", e))?;

        let stderr_bytes = stderr_handle.and_then(|h| h.join().ok()).unwrap_or_default();
        let stderr_str = String::from_utf8_lossy(&stderr_bytes);

        if !status.success() {
            let clean_err = Self::clean_exec_stderr(&stderr_str);
            if clean_err.is_empty() {
                // Exit code 130 is normal user interrupt (Ctrl+C)
                if status.code() == Some(130) {
                    return Ok(());
                }
                #[cfg(unix)]
                {
                    use std::os::unix::process::ExitStatusExt;
                    if status.signal() == Some(2) {
                        return Ok(());
                    }
                }
                return Err(format!("Command exited with status: {}", status));
            } else {
                return Err(clean_err);
            }
        }

        Ok(())
    }

    /// Builds the Command for an interactive pod exec shell
    pub fn build_pod_shell_command(
        context: &str,
        namespace: &str,
        pod_name: &str,
        container: Option<&str>,
        shell_cmd: Option<&str>,
    ) -> Command {
        let shell = shell_cmd.unwrap_or("/bin/sh");
        let mut cmd = Command::new("kubectl");
        if !context.is_empty() {
            cmd.args(["--context", context]);
        }
        if !namespace.is_empty() {
            cmd.args(["-n", namespace]);
        }
        cmd.args(["exec", "-i", "-t", pod_name]);

        if let Some(c) = container {
            cmd.args(["-c", c]);
        }

        cmd.args(["--", shell]);
        cmd
    }

    /// Launches an interactive pod exec shell via kubectl with the current context and namespace.
    /// When shell_cmd is None, automatically tries fallback shells (/bin/sh -> sh -> bash -> /bin/bash)
    /// if the container does not have /bin/sh.
    pub fn run_pod_shell(
        context: &str,
        namespace: &str,
        pod_name: &str,
        container: Option<&str>,
        shell_cmd: Option<&str>,
    ) -> Result<(), String> {
        if let Some(explicit_shell) = shell_cmd {
            let cmd = Self::build_pod_shell_command(context, namespace, pod_name, container, Some(explicit_shell));
            return Self::run_interactive_command(cmd);
        }

        let candidate_shells = ["/bin/sh", "sh", "bash", "/bin/bash"];
        let mut last_err = String::new();

        for shell in candidate_shells {
            let cmd = Self::build_pod_shell_command(context, namespace, pod_name, container, Some(shell));
            match Self::run_interactive_command(cmd) {
                Ok(()) => return Ok(()),
                Err(err) => {
                    if Self::is_missing_shell_err(&err) {
                        last_err = err;
                        continue;
                    } else {
                        // Non-shell error (e.g. pod not found, connection refused, forbidden)
                        return Err(err);
                    }
                }
            }
        }

        if last_err.is_empty() {
            Err("Container has no shell (/bin/sh, sh, bash not found; distroless?). Try Debug container ('S')".to_string())
        } else {
            Err(format!(
                "Container has no shell (/bin/sh, sh, bash not found; distroless? [{}]. Try Debug container ('S'))",
                last_err
            ))
        }
    }

    /// Builds the Command for an ephemeral debug container on a pod
    pub fn build_debug_shell_command(
        context: &str,
        namespace: &str,
        pod_name: &str,
        target_container: Option<&str>,
    ) -> Command {
        let mut cmd = Command::new("kubectl");
        if !context.is_empty() {
            cmd.args(["--context", context]);
        }
        if !namespace.is_empty() {
            cmd.args(["-n", namespace]);
        }
        cmd.args([
            "debug",
            "-i",
            "-t",
            pod_name,
            "--image=busybox:latest",
        ]);

        if let Some(c) = target_container {
            cmd.args([&format!("--target={}", c)]);
        }
        cmd
    }

    /// Launches an ephemeral debug container on a distroless/target pod
    pub fn run_debug_shell(
        context: &str,
        namespace: &str,
        pod_name: &str,
        target_container: Option<&str>,
    ) -> Result<(), String> {
        let cmd = Self::build_debug_shell_command(context, namespace, pod_name, target_container);
        Self::run_interactive_command(cmd)
    }

    /// Builds the Command for an interactive debug container on a node
    pub fn build_node_shell_command(context: &str, node_name: &str) -> Command {
        let mut cmd = Command::new("kubectl");
        if !context.is_empty() {
            cmd.args(["--context", context]);
        }
        cmd.args([
            "debug",
            &format!("node/{}", node_name),
            "-i",
            "-t",
            "--image=busybox",
        ]);
        cmd
    }

    /// Launches an interactive debug container on a node
    pub fn run_node_shell(context: &str, node_name: &str) -> Result<(), String> {
        let cmd = Self::build_node_shell_command(context, node_name);
        Self::run_interactive_command(cmd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_pod_shell_command_minimal() {
        let cmd = ExecRunner::build_pod_shell_command("", "", "my-pod", None, None);
        let args: Vec<String> = cmd.get_args().map(|s| s.to_string_lossy().to_string()).collect();
        assert_eq!(args, vec!["exec", "-i", "-t", "my-pod", "--", "/bin/sh"]);
    }

    #[test]
    fn test_build_pod_shell_command_full() {
        let cmd = ExecRunner::build_pod_shell_command("my-ctx", "my-ns", "my-pod", Some("app"), Some("/bin/bash"));
        let args: Vec<String> = cmd.get_args().map(|s| s.to_string_lossy().to_string()).collect();
        assert_eq!(args, vec!["--context", "my-ctx", "-n", "my-ns", "exec", "-i", "-t", "my-pod", "-c", "app", "--", "/bin/bash"]);
    }

    #[test]
    fn test_build_debug_shell_command_minimal() {
        let cmd = ExecRunner::build_debug_shell_command("", "", "my-pod", None);
        let args: Vec<String> = cmd.get_args().map(|s| s.to_string_lossy().to_string()).collect();
        assert_eq!(args, vec!["debug", "-i", "-t", "my-pod", "--image=busybox:latest"]);
    }

    #[test]
    fn test_build_debug_shell_command_full() {
        let cmd = ExecRunner::build_debug_shell_command("my-ctx", "my-ns", "my-pod", Some("sidecar"));
        let args: Vec<String> = cmd.get_args().map(|s| s.to_string_lossy().to_string()).collect();
        assert_eq!(args, vec!["--context", "my-ctx", "-n", "my-ns", "debug", "-i", "-t", "my-pod", "--image=busybox:latest", "--target=sidecar"]);
    }

    #[test]
    fn test_build_node_shell_command_minimal() {
        let cmd = ExecRunner::build_node_shell_command("", "node-1");
        let args: Vec<String> = cmd.get_args().map(|s| s.to_string_lossy().to_string()).collect();
        assert_eq!(args, vec!["debug", "node/node-1", "-i", "-t", "--image=busybox"]);
    }

    #[test]
    fn test_build_node_shell_command_full() {
        let cmd = ExecRunner::build_node_shell_command("prod-ctx", "node-gpu");
        let args: Vec<String> = cmd.get_args().map(|s| s.to_string_lossy().to_string()).collect();
        assert_eq!(args, vec!["--context", "prod-ctx", "debug", "node/node-gpu", "-i", "-t", "--image=busybox"]);
    }

    #[test]
    fn test_clean_exec_stderr_oci_runtime_failure() {
        let raw = "error: Internal error occurred: Internal error occurred: error executing command in container: failed to exec in container: failed to start exec \"1f767395a0e43807b5a76ad8c469b9eccf49c89a4b42b65ead0aef91135e97ce\": OCI runtime exec failed: exec failed: unable to start container process: exec: \"/bin/sh\": stat /bin/sh: no such file or directory";
        let cleaned = ExecRunner::clean_exec_stderr(raw);
        assert_eq!(cleaned, "exec: \"/bin/sh\": stat /bin/sh: no such file or directory");
    }

    #[test]
    fn test_clean_exec_stderr_oci_runtime_without_unable_prefix() {
        let raw = "OCI runtime exec failed: container process exited prematurely";
        let cleaned = ExecRunner::clean_exec_stderr(raw);
        assert_eq!(cleaned, "container process exited prematurely");
    }

    #[test]
    fn test_clean_exec_stderr_not_found() {
        let raw = "error: pods \"my-pod\" not found\n";
        let cleaned = ExecRunner::clean_exec_stderr(raw);
        assert_eq!(cleaned, "pods \"my-pod\" not found");
    }

    #[test]
    fn test_clean_exec_stderr_forbidden() {
        let raw = "Error from server (Forbidden): pods \"web\" is forbidden: User \"system:anonymous\" cannot get resource \"pods/exec\"";
        let cleaned = ExecRunner::clean_exec_stderr(raw);
        assert_eq!(cleaned, "Error from server (Forbidden): pods \"web\" is forbidden: User \"system:anonymous\" cannot get resource \"pods/exec\"");
    }

    #[test]
    fn test_clean_exec_stderr_empty_and_whitespace() {
        assert_eq!(ExecRunner::clean_exec_stderr(""), "");
        assert_eq!(ExecRunner::clean_exec_stderr("   \n\t  "), "");
    }

    #[test]
    fn test_is_missing_shell_err() {
        assert!(ExecRunner::is_missing_shell_err("exec: \"/bin/sh\": stat /bin/sh: no such file or directory"));
        assert!(ExecRunner::is_missing_shell_err("exec: \"sh\": executable file not found in $PATH"));
        assert!(ExecRunner::is_missing_shell_err("OCI runtime exec failed: exec failed: unable to start container process: exec: \"bash\": stat /bin/bash: no such file or directory"));

        assert!(!ExecRunner::is_missing_shell_err("pods \"my-pod\" not found"));
        assert!(!ExecRunner::is_missing_shell_err("Error from server (Forbidden): cannot exec"));
        assert!(!ExecRunner::is_missing_shell_err("Failed to spawn kubectl: No such file or directory"));
        assert!(!ExecRunner::is_missing_shell_err(""));
    }

    #[test]
    fn test_run_interactive_command_success() {
        #[cfg(unix)]
        {
            let mut cmd = Command::new("sh");
            cmd.args(["-c", "exit 0"]);
            let res = ExecRunner::run_interactive_command(cmd);
            assert!(res.is_ok());
        }
    }

    #[test]
    fn test_run_interactive_command_exit_code_130() {
        #[cfg(unix)]
        {
            let mut cmd = Command::new("sh");
            cmd.args(["-c", "exit 130"]);
            let res = ExecRunner::run_interactive_command(cmd);
            assert!(res.is_ok());
        }
    }

    #[test]
    fn test_run_interactive_command_failure_captured() {
        #[cfg(unix)]
        {
            let mut cmd = Command::new("sh");
            cmd.args(["-c", "echo 'error: test failure message' >&2; exit 1"]);
            let res = ExecRunner::run_interactive_command(cmd);
            assert_eq!(res.unwrap_err(), "test failure message");
        }
    }

    #[test]
    fn test_run_interactive_command_exit_failure_without_stderr() {
        #[cfg(unix)]
        {
            let mut cmd = Command::new("sh");
            cmd.args(["-c", "exit 42"]);
            let res = ExecRunner::run_interactive_command(cmd);
            assert!(res.is_err());
            assert!(res.unwrap_err().contains("42"));
        }
    }

    #[test]
    fn test_clean_exec_stderr_nested_prefixes() {
        let raw = "error: Internal error occurred: failed to exec in container: error executing command in container: something broke";
        assert_eq!(ExecRunner::clean_exec_stderr(raw), "something broke");
    }

    #[test]
    fn test_run_interactive_command_nonexistent_binary() {
        let cmd = Command::new("/nonexistent/binary/that/does/not/exist");
        let res = ExecRunner::run_interactive_command(cmd);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("Failed to spawn command"));
    }

    #[test]
    #[ignore = "spawns external kubectl; run manually as an integration test"]
    fn test_run_pod_shell_with_explicit_command_failure() {
        let res = ExecRunner::run_pod_shell("ctx", "ns", "pod", None, Some("/nonexistent/sh"));
        assert!(res.is_err());
    }

    #[test]
    #[ignore = "spawns external kubectl; run manually as an integration test"]
    fn test_run_pod_shell_fallback_chain_all_missing() {
        // When running against non-existent cluster/context with no kubectl or missing pod,
        // it fails cleanly with captured error.
        let res = ExecRunner::run_pod_shell("nonexistent-ctx", "ns", "pod", None, None);
        assert!(res.is_err());
    }

    #[test]
    #[ignore = "spawns external kubectl; run manually as an integration test"]
    fn test_run_debug_shell_failure() {
        let res = ExecRunner::run_debug_shell("nonexistent-ctx", "ns", "pod", None);
        assert!(res.is_err());
    }

    #[test]
    #[ignore = "spawns external kubectl; run manually as an integration test"]
    fn test_run_node_shell_failure() {
        let res = ExecRunner::run_node_shell("nonexistent-ctx", "nonexistent-node");
        assert!(res.is_err());
    }
}
