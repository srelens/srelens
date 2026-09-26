//! macOS: Seatbelt through `/usr/bin/sandbox-exec`, the backend the #571
//! spike verified on macOS 27.0 arm64 for isolation (its checks 1 to 4 and 7).
//!
//! macOS has no kernel facility that limits a process's memory or CPU rate
//! (`setrlimit` refuses `RLIMIT_DATA` and `RLIMIT_AS`; `RLIMIT_CPU` is a
//! lifetime budget). The accepted decision is a host-side watchdog, #713,
//! which is not built, so [`LIMITS_MISSING`] is what this backend reports and
//! the supervisor refuses to start a sidecar. [`launch`] still works, so the
//! isolation can be checked (`tests/sandbox_conformance.rs`).
//!
//! The spike's launcher also set `RLIMIT_DATA`, `RLIMIT_AS` and a 60-second
//! `RLIMIT_CPU`. These are left out: the first two were refused, and the
//! third would kill a sidecar after a minute of CPU, which is not a limit.

use std::process::Stdio;

use super::{Exit, LaunchError, Launched, SandboxConfig, SidecarCommand};
use crate::sidecar::Limits;

pub(super) const LIMITS_MISSING: &str = "srelens cannot limit an app's memory and CPU on macOS yet (#713), so it does not run executable apps here";

pub(super) fn launch(
    config: &SandboxConfig,
    command: &SidecarCommand,
    _limits: &Limits,
) -> Result<Launched, LaunchError> {
    let launcher = config.launcher.as_ref().ok_or_else(|| {
        LaunchError::Unavailable(
            "srelens has no sandbox launcher (srelens-sandbox-launch) to start the app with, so it does not run executable apps".into(),
        )
    })?;
    if !std::path::Path::new("/usr/bin/sandbox-exec").is_file() {
        return Err(LaunchError::Unavailable(
            "This Mac has no /usr/bin/sandbox-exec, so srelens cannot confine an app and does not run executable apps".into(),
        ));
    }
    // Seatbelt matches resolved paths; `/var` and `/tmp` are symlinks.
    let canonical = |path: &std::path::Path, what: &str| {
        std::fs::canonicalize(path).map_err(|e| {
            LaunchError::Failed(format!(
                "srelens could not find the app's {what} {}: {e}",
                path.display()
            ))
        })
    };
    let program = canonical(&command.program, "program")?;
    let data = canonical(&command.data_dir, "data directory")?;
    let child = tokio::process::Command::new(launcher)
        .arg("--data")
        .arg(&data)
        .arg("--")
        .arg(&program)
        .args(&command.args)
        .env_clear()
        .envs(command.env.iter().map(|(k, v)| (k, v)))
        .current_dir(&data)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| {
            LaunchError::Failed(format!(
                "srelens could not start its sandbox launcher {}: {e}",
                launcher.display()
            ))
        })?;
    Launched::from_child(child, Exit::from_status)
}
