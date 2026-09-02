use std::process::Command;

pub struct ExecRunner;

impl ExecRunner {
    /// Launches an interactive pod exec shell via kubectl with the current context and namespace
    pub fn run_pod_shell(
        context: &str,
        namespace: &str,
        pod_name: &str,
        container: Option<&str>,
        shell_cmd: Option<&str>,
    ) -> Result<(), String> {
        let shell = shell_cmd.unwrap_or("/bin/sh");
        let mut cmd = Command::new("kubectl");
        cmd.args(["--context", context, "-n", namespace, "exec", "-i", "-t", pod_name]);

        if let Some(c) = container {
            cmd.args(["-c", c]);
        }

        cmd.args(["--", shell]);

        let status = cmd
            .status()
            .map_err(|e| format!("Failed to spawn pod exec session: {}", e))?;

        if !status.success() {
            return Err(format!("Pod exec exited with status: {}", status));
        }

        Ok(())
    }

    /// Launches an ephemeral debug container on a distroless/target pod
    pub fn run_debug_shell(
        context: &str,
        namespace: &str,
        pod_name: &str,
        target_container: Option<&str>,
    ) -> Result<(), String> {
        let mut cmd = Command::new("kubectl");
        cmd.args([
            "--context",
            context,
            "-n",
            namespace,
            "debug",
            "-i",
            "-t",
            pod_name,
            "--image=busybox:latest",
        ]);

        if let Some(c) = target_container {
            cmd.args([&format!("--target={}", c)]);
        }

        let status = cmd
            .status()
            .map_err(|e| format!("Failed to launch debug container: {}", e))?;

        if !status.success() {
            return Err(format!("Debug container exited with status: {}", status));
        }

        Ok(())
    }

    /// Launches a privileged node shell
    pub fn run_node_shell(context: &str, node_name: &str) -> Result<(), String> {
        let mut cmd = Command::new("kubectl");
        cmd.args(["--context", context, "node-shell", node_name]);

        let status = cmd
            .status()
            .map_err(|e| format!("Failed to spawn node shell: {}", e))?;

        if !status.success() {
            return Err(format!("Node shell exited with status: {}", status));
        }

        Ok(())
    }
}
