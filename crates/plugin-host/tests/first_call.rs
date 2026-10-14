//! A call made the moment a sidecar is reported running is answered. The
//! registry waits for a starting sidecar to leave `Starting` and then calls it
//! (`crates/registry/src/extensions/sidecars.rs`), on the desktop's
//! multi-threaded runtime, so the status must not say `Running` before the
//! session is there to take the call.
//!
//! A test binary of its own, so that its process is fresh: the app log's
//! redaction patterns are compiled on first use, and the first line srelens
//! writes about a sidecar is "The extension is running". An installed desktop
//! package showed it: the first calls to a newly started app were refused with
//! "The extension is running" until those patterns had compiled.

use serde_json::json;
use srelens_plugin_host::sidecar::data::DataDir;
use srelens_plugin_host::sidecar::{
    Enforcement, Exit, LaunchError, Launched, Launcher, Limits, NoBroker, Policy, SidecarCommand,
    SidecarConfig, SidecarStatus, Supervisor,
};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

const PROBE: &str = env!("CARGO_BIN_EXE_srelens-sidecar-probe");

/// Starts the probe with no sandbox. Test support only: it claims the kernel
/// enforces limits it does not set, so that the supervisor will run it.
struct Unconfined;

impl Launcher for Unconfined {
    fn enforcement(&self) -> Enforcement {
        Enforcement::Kernel
    }

    fn launch(&self, command: &SidecarCommand, _limits: &Limits) -> Result<Launched, LaunchError> {
        let child = tokio::process::Command::new(&command.program)
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_call_made_as_soon_as_the_sidecar_is_running_is_answered() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let data = DataDir::for_app(root.path(), "org.example.probe").expect("a data directory");
    let config = SidecarConfig {
        command: SidecarCommand {
            app_id: "org.example.probe".into(),
            program: PROBE.into(),
            args: Vec::new(),
            env: Vec::new(),
            data_dir: data.path().to_owned(),
        },
        limits: Limits::default(),
        policy: Policy::default(),
    };
    let supervisor = Supervisor::start(config, Arc::new(Unconfined), Arc::new(NoBroker));
    let mut status = supervisor.watch();
    tokio::time::timeout(
        Duration::from_secs(30),
        status.wait_for(|status| !matches!(status, SidecarStatus::Starting)),
    )
    .await
    .expect("the sidecar started within 30 s")
    .expect("the supervisor is running");
    drop(status);
    let answer = supervisor.request("echo", json!({"n": 1})).await;
    assert_eq!(
        answer,
        Ok(json!({"n": 1})),
        "status: {:?}",
        supervisor.status()
    );
    supervisor.stop().await;
}
