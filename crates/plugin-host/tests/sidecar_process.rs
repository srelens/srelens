//! The supervisor (#572) against a real sidecar process: the probe binary
//! (`src/bin/srelens-sidecar-probe.rs`) over real pipes. Crash isolation is
//! the point here: a sidecar that aborts, exits or hangs is a real process
//! doing so, and the test process, which is the host, carries on.
//!
//! These start the probe without a sandbox, on the wall clock, with short
//! waits. The sandbox itself is checked by `sandbox_conformance.rs`.

use serde_json::json;
use srelens_plugin_host::sidecar::data::DataDir;
use srelens_plugin_host::sidecar::{
    Enforcement, Exit, LaunchError, Launched, Launcher, Limits, LogSource, NoBroker, OsSandbox,
    Policy, RequestError, SandboxConfig, SidecarCommand, SidecarConfig, SidecarStatus, Supervisor,
};
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

const PROBE: &str = env!("CARGO_BIN_EXE_srelens-sidecar-probe");

/// Variables a process's own runtime sets in its environment as it starts,
/// which no host passed it. LLVM's coverage runtime sets this one in every
/// instrumented binary, so the probe has it under `cargo llvm-cov`.
const SET_BY_THE_RUNTIME: &[&str] = &["__LLVM_PROFILE_RT_INIT_ONCE"];

/// Starts the probe with no sandbox. Test support only: it claims the kernel
/// enforces limits it does not set, so that the supervisor will run it.
#[derive(Default)]
struct Unconfined {
    launches: AtomicUsize,
}

impl Launcher for Unconfined {
    fn enforcement(&self) -> Enforcement {
        Enforcement::Kernel
    }

    fn launch(&self, command: &SidecarCommand, _limits: &Limits) -> Result<Launched, LaunchError> {
        self.launches.fetch_add(1, Ordering::SeqCst);
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

/// A fresh, private data directory (#573) for one supervisor, under one root
/// per test binary.
fn data_dir() -> PathBuf {
    static ROOT: OnceLock<tempfile::TempDir> = OnceLock::new();
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let root = ROOT.get_or_init(|| tempfile::tempdir().expect("a temporary directory"));
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    DataDir::for_app(&root.path().join(n.to_string()), "org.example.probe")
        .expect("a data directory")
        .path()
        .to_owned()
}

fn config(env: &[(&str, &str)]) -> SidecarConfig {
    SidecarConfig {
        command: SidecarCommand {
            app_id: "org.example.probe".into(),
            program: PROBE.into(),
            args: Vec::new(),
            env: env
                .iter()
                .map(|(k, v)| (OsString::from(k), OsString::from(v)))
                .collect(),
            data_dir: data_dir(),
        },
        limits: Limits::default(),
        policy: Policy {
            backoff: vec![
                Duration::from_millis(20),
                Duration::from_millis(40),
                Duration::from_millis(60),
            ],
            ..Policy::default()
        },
    }
}

fn start(config: SidecarConfig) -> (Supervisor, Arc<Unconfined>) {
    let launcher = Arc::new(Unconfined::default());
    let supervisor = Supervisor::start(config, launcher.clone(), Arc::new(NoBroker));
    (supervisor, launcher)
}

async fn until(supervisor: &Supervisor, done: impl Fn(&SidecarStatus) -> bool) -> SidecarStatus {
    let mut status = supervisor.watch();
    let waited = tokio::time::timeout(Duration::from_secs(30), status.wait_for(|s| done(s))).await;
    match waited {
        Ok(Ok(found)) => found.clone(),
        Ok(Err(_)) => panic!("the supervisor is gone"),
        Err(_) => panic!("the status never came; it is {:?}", supervisor.status()),
    }
}

async fn running(supervisor: &Supervisor) -> Option<u32> {
    match until(supervisor, |s| matches!(s, SidecarStatus::Running { .. })).await {
        SidecarStatus::Running { pid, .. } => pid,
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn a_real_sidecar_completes_the_handshake_and_answers_over_its_pipes() {
    let (supervisor, _) = start(config(&[]));
    running(&supervisor).await;
    for n in 0..50 {
        let answer = supervisor.request("echo", json!({"n": n})).await;
        assert_eq!(answer, Ok(json!({"n": n})));
    }
    supervisor.stop().await;
}

#[tokio::test]
async fn a_sidecar_that_aborts_mid_request_does_not_take_the_host_with_it() {
    let (supervisor, launcher) = start(config(&[]));
    let first = running(&supervisor).await;
    let error = supervisor.request("abort", json!({})).await.unwrap_err();
    let RequestError::Ended(why) = &error else {
        panic!("{error:?}")
    };
    assert!(why.contains("exited unexpectedly"), "{why}");
    #[cfg(unix)]
    assert!(why.contains("SIGABRT"), "{why}");
    // The host is still here, and so, restarted, is the sidecar.
    let second = running(&supervisor).await;
    assert_ne!(first, second, "a new process");
    assert_eq!(supervisor.request("echo", json!(1)).await, Ok(json!(1)));
    assert_eq!(launcher.launches.load(Ordering::SeqCst), 2);
    supervisor.stop().await;
}

#[tokio::test]
async fn a_sidecar_that_dies_at_every_start_is_disabled_after_three_restarts() {
    let (supervisor, launcher) = start(config(&[("PROBE_ON_START", "exit:3")]));
    let status = until(&supervisor, |s| matches!(s, SidecarStatus::Disabled { .. })).await;
    let SidecarStatus::Disabled { reason } = &status else {
        unreachable!()
    };
    assert!(reason.contains("exited with status 3"), "{reason}");
    assert_eq!(
        status.message(),
        Some("Extension process exited unexpectedly")
    );
    assert_eq!(launcher.launches.load(Ordering::SeqCst), 4);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        launcher.launches.load(Ordering::SeqCst),
        4,
        "nothing starts it on its own"
    );
}

#[tokio::test]
async fn a_sidecar_that_stops_answering_is_killed_and_restarted() {
    let mut config = config(&[]);
    // Short, for a quick test, but with room for a loaded CI runner: a healthy
    // probe answers in microseconds.
    config.policy.health_interval = Duration::from_millis(100);
    config.policy.health_timeout = Duration::from_secs(1);
    let (supervisor, _) = start(config);
    running(&supervisor).await;
    // The probe blocks its only thread: nothing more is answered.
    let hung = supervisor.request("hang", json!({}));
    let restarting = until(&supervisor, |s| {
        matches!(s, SidecarStatus::Restarting { .. })
    });
    let (hung, status) = tokio::join!(hung, restarting);
    assert!(matches!(hung, Err(RequestError::Ended(_))), "{hung:?}");
    let SidecarStatus::Restarting { reason, .. } = status else {
        unreachable!()
    };
    assert!(reason.contains("health check"), "{reason}");
    running(&supervisor).await;
    supervisor.stop().await;
}

#[tokio::test]
async fn an_incompatible_sidecar_is_refused_with_both_versions() {
    let (supervisor, launcher) = start(config(&[("PROBE_API_VERSION", "none")]));
    let SidecarStatus::Refused { reason } =
        until(&supervisor, |s| matches!(s, SidecarStatus::Refused { .. })).await
    else {
        unreachable!()
    };
    assert!(reason.contains("0.1.0"), "{reason}");
    assert_eq!(launcher.launches.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn the_sidecar_gets_only_the_environment_it_is_given() {
    let (supervisor, _) = start(config(&[("PROBE_MARK", "1")]));
    running(&supervisor).await;
    let answer = supervisor.request("env", json!({})).await.unwrap();
    let names: Vec<&str> = answer["names"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|n| n.as_str())
        .filter(|n| !SET_BY_THE_RUNTIME.contains(n))
        .collect();
    assert_eq!(names, ["PROBE_MARK"]);
    supervisor.stop().await;
}

#[tokio::test]
async fn an_allocation_too_large_to_count_is_refused_not_wrapped() {
    // 2^45 MiB is past what a 64-bit byte count holds: multiplied unchecked,
    // it would panic in a debug build and wrap to a small size in a release
    // build, which the memory check would read as a successful allocation.
    let (supervisor, _) = start(config(&[]));
    running(&supervisor).await;
    let error = supervisor
        .request("allocate", json!({"mib": 1u64 << 45}))
        .await
        .unwrap_err();
    let RequestError::Failed(error) = &error else {
        panic!("{error:?}")
    };
    assert_eq!(
        error.data.as_ref().unwrap()["kind"],
        "InvalidInput",
        "{error:?}"
    );
    supervisor.stop().await;
}

#[tokio::test]
async fn what_the_sidecar_writes_to_stderr_is_in_its_log() {
    let (supervisor, _) = start(config(&[]));
    running(&supervisor).await;
    supervisor
        .request("log", json!({"text": "probe: cache warmed"}))
        .await
        .unwrap();
    // stderr is read on its own task; give it a moment.
    for _ in 0..100 {
        if supervisor
            .logs()
            .iter()
            .any(|l| l.source == LogSource::Sidecar && l.text == "probe: cache warmed")
        {
            supervisor.stop().await;
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("not in the log: {:?}", supervisor.logs());
}

#[cfg(unix)]
#[tokio::test]
async fn a_stopped_sidecar_leaves_no_process_behind() {
    let (supervisor, _) = start(config(&[]));
    let pid = running(&supervisor).await.expect("a pid") as i32;
    supervisor.stop().await;
    assert_eq!(supervisor.status(), SidecarStatus::Stopped);
    // SAFETY: signal 0 only asks whether the process exists.
    let exists = unsafe { libc::kill(pid, 0) } == 0;
    assert!(!exists, "process {pid} is still there");
}

#[cfg(unix)]
#[tokio::test]
async fn dropping_the_supervisor_kills_a_real_sidecar() {
    let (supervisor, _) = start(config(&[]));
    let pid = running(&supervisor).await.expect("a pid") as i32;
    drop(supervisor);
    for _ in 0..200 {
        // SAFETY: signal 0 only asks whether the process exists.
        if unsafe { libc::kill(pid, 0) } != 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("process {pid} outlived its supervisor");
}

/// The one refusal that holds on this machine, through the real backend:
/// macOS until #713, or an OS with no backend at all.
#[cfg(not(any(target_os = "linux", windows)))]
#[tokio::test]
async fn the_os_sandbox_refuses_to_run_sidecars_here_and_says_why() {
    let sandbox = OsSandbox::new(SandboxConfig {
        launcher: Some(env!("CARGO_BIN_EXE_srelens-sandbox-launch").into()),
        cgroup: srelens_plugin_host::sidecar::CgroupRoot::Missing,
    });
    let supervisor = Supervisor::start(config(&[]), Arc::new(sandbox), Arc::new(NoBroker));
    let SidecarStatus::Refused { reason } =
        until(&supervisor, |s| matches!(s, SidecarStatus::Refused { .. })).await
    else {
        unreachable!()
    };
    let expected = if cfg!(target_os = "macos") {
        "#713"
    } else {
        "no sandbox for executable apps"
    };
    assert!(reason.contains(expected), "{reason}");
}

/// On Linux without a delegated cgroup, as on an ordinary test machine, the
/// real backend refuses rather than run the sidecar without limits.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn without_a_delegated_cgroup_linux_refuses_to_run_sidecars() {
    let sandbox = OsSandbox::new(SandboxConfig {
        launcher: Some(env!("CARGO_BIN_EXE_srelens-sandbox-launch").into()),
        cgroup: srelens_plugin_host::sidecar::CgroupRoot::Missing,
    });
    let supervisor = Supervisor::start(config(&[]), Arc::new(sandbox), Arc::new(NoBroker));
    let SidecarStatus::Refused { reason } =
        until(&supervisor, |s| matches!(s, SidecarStatus::Refused { .. })).await
    else {
        unreachable!()
    };
    assert!(
        reason.contains("cgroup") || reason.contains("Landlock"),
        "{reason}"
    );
}
