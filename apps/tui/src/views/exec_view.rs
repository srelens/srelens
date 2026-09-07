use std::process::Command;

pub struct ExecRunner;

impl ExecRunner {
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

    /// Launches an interactive pod exec shell via kubectl with the current context and namespace
    pub fn run_pod_shell(
        context: &str,
        namespace: &str,
        pod_name: &str,
        container: Option<&str>,
        shell_cmd: Option<&str>,
    ) -> Result<(), String> {
        let mut cmd = Self::build_pod_shell_command(context, namespace, pod_name, container, shell_cmd);
        let status = cmd
            .status()
            .map_err(|e| format!("Failed to spawn pod exec session: {}", e))?;

        if !status.success() {
            return Err(format!("Pod exec exited with status: {}", status));
        }

        Ok(())
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
        let mut cmd = Self::build_debug_shell_command(context, namespace, pod_name, target_container);
        let status = cmd
            .status()
            .map_err(|e| format!("Failed to launch debug container: {}", e))?;

        if !status.success() {
            return Err(format!("Debug container exited with status: {}", status));
        }

        Ok(())
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
        let mut cmd = Self::build_node_shell_command(context, node_name);
        let status = cmd
            .status()
            .map_err(|e| format!("Failed to spawn node debug container: {}", e))?;

        if !status.success() {
            return Err(format!("Node debug exited with status: {}", status));
        }

        Ok(())
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
}
