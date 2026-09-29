//! The example under srelens's own supervisor, without a sandbox: every call
//! a sidecar gets, over real pipes, to a real process.

mod common;

use srelens_plugin_host::sidecar::{
    Enforcement, Exit, LaunchError, Launched, Launcher, Limits, SidecarCommand,
};
use std::process::Stdio;
use std::sync::Arc;

/// Starts the sidecar with no sandbox. Test support only: it claims the
/// kernel enforces limits it does not set, so that the supervisor runs it.
struct Unconfined;

impl Launcher for Unconfined {
    fn enforcement(&self) -> Enforcement {
        Enforcement::Kernel
    }

    fn launch(&self, command: &SidecarCommand, _limits: &Limits) -> Result<Launched, LaunchError> {
        let child = tokio::process::Command::new(&command.program)
            .args(&command.args)
            .env_clear()
            .envs(command.env.iter().map(|(k, v)| (k, v)))
            .current_dir(&command.data_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| LaunchError::Failed(e.to_string()))?;
        Launched::from_child(child, Exit::from_status)
    }
}

#[tokio::test]
async fn the_hello_world_sidecar_serves_srelens_through_its_whole_life() {
    common::whole_life(Arc::new(Unconfined), Limits::default()).await;
}
