//! The sidecar supervisor (#572) against an in-process fake sidecar, on a
//! paused clock: every timeout and backoff is asserted to its exact length.
//! `sidecar_process.rs` runs the same lifecycle against a real process.

mod fake;

use fake::{FakeLauncher, Reply};
use serde_json::{json, Value};
use srelens_plugin_host::sidecar::data::DataDir;
use srelens_plugin_host::sidecar::{
    Action, AppLog, Enforcement, Limits, LogLevel, LogSource, NoBroker, Policy, RequestError,
    RequestMetrics, SidecarCommand, SidecarConfig, SidecarStatus, StreamEvent, Supervisor,
    SIDECAR_API_VERSIONS, UNEXPECTED_EXIT,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::time::{sleep, Instant};

/// A fresh, private data directory (#573) for one supervisor: the supervisor
/// refuses a shared or world-readable one such as the system's temporary
/// directory. Under one root per test binary, which outlives every test.
fn data_dir() -> PathBuf {
    static ROOT: OnceLock<tempfile::TempDir> = OnceLock::new();
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let root = ROOT.get_or_init(|| tempfile::tempdir().expect("a temporary directory"));
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    DataDir::for_app(&root.path().join(n.to_string()), "org.example.scanner")
        .expect("a data directory")
        .path()
        .to_owned()
}

fn config() -> SidecarConfig {
    SidecarConfig {
        command: SidecarCommand {
            app_id: "org.example.scanner".into(),
            program: "/opt/example/scanner".into(),
            args: Vec::new(),
            env: Vec::new(),
            data_dir: data_dir(),
        },
        limits: Limits::default(),
        policy: Policy::default(),
    }
}

fn start(launcher: &FakeLauncher) -> Supervisor {
    Supervisor::start(config(), Arc::new(launcher.clone()), Arc::new(NoBroker))
}

/// Wait until the status satisfies `done`, and return it. On the paused
/// clock a wait that can never end fails at once, at its virtual deadline,
/// instead of hanging the suite.
async fn until(supervisor: &Supervisor, done: impl Fn(&SidecarStatus) -> bool) -> SidecarStatus {
    let mut status = supervisor.watch();
    let waited = tokio::time::timeout(secs(24 * 3600), status.wait_for(|s| done(s))).await;
    match waited {
        Ok(Ok(found)) => found.clone(),
        Ok(Err(_)) => panic!("the supervisor is gone"),
        Err(_) => panic!("the status never came; it is {:?}", supervisor.status()),
    }
}

async fn running(supervisor: &Supervisor) -> SidecarStatus {
    until(supervisor, |s| matches!(s, SidecarStatus::Running { .. })).await
}

/// The stream's next event, failing at a virtual deadline like `until`.
async fn next(stream: &mut srelens_plugin_host::sidecar::SidecarStream) -> Option<StreamEvent> {
    tokio::time::timeout(secs(24 * 3600), stream.next())
        .await
        .expect("the stream's next event never came")
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

#[tokio::test(start_paused = true)]
async fn the_handshake_offers_every_version_then_activates() {
    let launcher = FakeLauncher::well_behaved();
    let supervisor = start(&launcher);
    let SidecarStatus::Running { api_version, .. } = running(&supervisor).await else {
        unreachable!()
    };
    assert_eq!(api_version, semver::Version::new(0, 1, 0));
    assert_eq!(launcher.methods(), ["initialize", "activate"]);
    let (_, initialize) = &launcher.received()[0];
    assert_eq!(
        initialize["params"]["apiVersions"],
        json!(SIDECAR_API_VERSIONS)
    );
    assert_eq!(initialize["params"]["limits"]["maxConcurrentRequests"], 8);
    assert_eq!(initialize["params"]["limits"]["maxStreams"], 5);
    assert_eq!(initialize["params"]["limits"]["requestTimeoutMs"], 30_000);
    assert_eq!(
        initialize["params"]["limits"]["memoryBytes"],
        268_435_456u64
    );
    assert_eq!(initialize["params"]["limits"]["dataBytes"], 1u64 << 30);
    assert!(initialize["params"]["dataDirectory"].is_string());
}

#[tokio::test(start_paused = true)]
async fn a_running_sidecar_answers_requests() {
    let launcher = FakeLauncher::well_behaved();
    let supervisor = start(&launcher);
    running(&supervisor).await;
    let answer = supervisor.request("echo", json!({"n": 1})).await;
    assert_eq!(answer, Ok(json!({"n": 1})));
}

#[tokio::test(start_paused = true)]
async fn a_version_srelens_did_not_offer_is_refused_and_never_retried() {
    let launcher = FakeLauncher::new(|call| {
        (call.method == "initialize").then(|| Reply::Result(json!({"apiVersion": "9.0.0"})))
    });
    let supervisor = start(&launcher);
    let status = until(&supervisor, |s| matches!(s, SidecarStatus::Refused { .. })).await;
    let SidecarStatus::Refused { reason } = status else {
        unreachable!()
    };
    assert!(
        reason.contains("9.0.0") && reason.contains("0.1.0"),
        "{reason}"
    );
    sleep(secs(3600)).await;
    assert_eq!(launcher.launches(), 1);
    assert_eq!(
        launcher.killed(),
        [1],
        "the incompatible process is stopped"
    );
    let error = supervisor.request("echo", json!({})).await.unwrap_err();
    assert_eq!(error, RequestError::Unavailable(reason));
}

#[tokio::test(start_paused = true)]
async fn a_sidecar_that_speaks_no_offered_version_says_which_it_speaks() {
    let launcher = FakeLauncher::new(|call| {
        (call.method == "initialize").then_some(Reply::Raw(
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32001,"message":"no common version","data":{"supported":["2.0.0"]}}}"#,
        ))
    });
    let supervisor = start(&launcher);
    let SidecarStatus::Refused { reason } =
        until(&supervisor, |s| matches!(s, SidecarStatus::Refused { .. })).await
    else {
        unreachable!()
    };
    assert!(
        reason.contains("2.0.0") && reason.contains("0.1.0"),
        "{reason}"
    );
    sleep(secs(3600)).await;
    assert_eq!(launcher.launches(), 1);
}

#[tokio::test(start_paused = true)]
async fn unexpected_exits_restart_after_1_then_5_then_30_seconds_then_disable() {
    let launcher = FakeLauncher::new(|call| (call.method == "activate").then_some(Reply::Crash));
    let supervisor = start(&launcher);
    let mut seen = Vec::new();
    for attempt in 1..=3 {
        let status = until(
            &supervisor,
            |s| matches!(s, SidecarStatus::Restarting { attempt: a, .. } if *a == attempt),
        )
        .await;
        seen.push(status);
    }
    let disabled = until(&supervisor, |s| matches!(s, SidecarStatus::Disabled { .. })).await;

    let delays: Vec<Duration> = seen
        .iter()
        .map(|s| match s {
            SidecarStatus::Restarting { delay, of, .. } => {
                assert_eq!(*of, 3);
                *delay
            }
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(delays, [secs(1), secs(5), secs(30)]);
    let at = launcher.launched_at();
    let gaps: Vec<Duration> = at.windows(2).map(|w| w[1] - w[0]).collect();
    assert_eq!(
        gaps,
        [secs(1), secs(5), secs(30)],
        "each restart waits exactly its delay"
    );

    let SidecarStatus::Disabled { reason } = &disabled else {
        unreachable!()
    };
    assert!(reason.contains("SIGABRT"), "{reason}");
    assert_eq!(disabled.message(), Some(UNEXPECTED_EXIT));
    assert_eq!(UNEXPECTED_EXIT, "Extension process exited unexpectedly");
    assert_eq!(
        disabled.actions(),
        [Action::Restart, Action::ViewLogs, Action::Disable]
    );
    let labels: Vec<&str> = disabled.actions().iter().map(|a| a.label()).collect();
    assert_eq!(labels, ["Restart", "View logs", "Disable"]);

    sleep(secs(3600)).await;
    assert_eq!(
        launcher.launches(),
        4,
        "a disabled sidecar is not started again on its own"
    );
}

#[tokio::test(start_paused = true)]
async fn a_crash_while_running_is_restarted_like_a_failed_start() {
    let launcher = FakeLauncher::new(|call| (call.method == "boom").then_some(Reply::Crash));
    let supervisor = start(&launcher);
    running(&supervisor).await;
    let error = supervisor.request("boom", json!({})).await.unwrap_err();
    let RequestError::Ended(why) = &error else {
        panic!("{error:?}")
    };
    assert!(
        why.contains("exited unexpectedly") && why.contains("SIGABRT"),
        "{why}"
    );
    let status = until(&supervisor, |s| {
        matches!(s, SidecarStatus::Restarting { .. })
    })
    .await;
    assert!(
        matches!(status, SidecarStatus::Restarting { attempt: 1, delay, .. } if delay == secs(1))
    );
    running(&supervisor).await;
    assert_eq!(launcher.launches(), 2);
}

#[tokio::test(start_paused = true)]
async fn a_sidecar_that_ran_long_enough_starts_the_sequence_over() {
    // Launch 1 fails at once; launch 2 runs for eleven minutes, then crashes.
    let launcher = FakeLauncher::new(|call| match (call.method, call.launch) {
        ("activate", 1) => Some(Reply::Crash),
        ("boom", _) => Some(Reply::Crash),
        _ => None,
    });
    let supervisor = start(&launcher);
    until(&supervisor, |s| {
        matches!(s, SidecarStatus::Restarting { attempt: 1, .. })
    })
    .await;
    running(&supervisor).await;
    sleep(secs(11 * 60)).await;
    let _ = supervisor.request("boom", json!({})).await;
    let status = until(&supervisor, |s| {
        matches!(s, SidecarStatus::Restarting { .. })
    })
    .await;
    let SidecarStatus::Restarting { attempt, delay, .. } = status else {
        unreachable!()
    };
    assert_eq!((attempt, delay), (1, secs(1)), "not the second step, 5 s");
}

#[tokio::test(start_paused = true)]
async fn restart_after_disabled_starts_a_fresh_sequence() {
    let launcher = FakeLauncher::new(|call| {
        (call.method == "activate" && call.launch <= 4).then_some(Reply::Crash)
    });
    let supervisor = start(&launcher);
    until(&supervisor, |s| matches!(s, SidecarStatus::Disabled { .. })).await;
    supervisor.restart();
    running(&supervisor).await;
    assert_eq!(launcher.launches(), 5);
}

#[tokio::test(start_paused = true)]
async fn a_hung_sidecar_fails_its_health_check_and_is_restarted() {
    let launcher = FakeLauncher::new(|call| {
        (call.method == "health" && call.launch == 1).then_some(Reply::Silent)
    });
    let supervisor = start(&launcher);
    running(&supervisor).await;
    let started = Instant::now();
    let status = until(&supervisor, |s| {
        matches!(s, SidecarStatus::Restarting { .. })
    })
    .await;
    // The first check is due after 30 s, and has 10 s to be answered.
    assert_eq!(started.elapsed(), secs(40));
    let SidecarStatus::Restarting { reason, .. } = status else {
        unreachable!()
    };
    assert!(
        reason.contains("health") && reason.contains("10 s"),
        "{reason}"
    );
    assert_eq!(launcher.killed(), [1]);
    running(&supervisor).await;
}

#[tokio::test(start_paused = true)]
async fn a_sidecar_that_breaks_the_protocol_is_stopped_and_restarted() {
    let launcher = FakeLauncher::new(|call| {
        (call.method == "garble" && call.launch == 1)
            .then_some(Reply::Raw("thread 'main' panicked"))
    });
    let supervisor = start(&launcher);
    running(&supervisor).await;
    let error = supervisor.request("garble", json!({})).await.unwrap_err();
    assert!(
        matches!(&error, RequestError::Ended(why) if why.contains("not JSON")),
        "{error:?}"
    );
    let status = until(&supervisor, |s| {
        matches!(s, SidecarStatus::Restarting { .. })
    })
    .await;
    let SidecarStatus::Restarting { reason, .. } = status else {
        unreachable!()
    };
    assert!(reason.contains("not JSON"), "{reason}");
    assert_eq!(launcher.killed(), [1]);
}

#[tokio::test(start_paused = true)]
async fn an_initialize_that_is_never_answered_is_a_failed_start() {
    let launcher = FakeLauncher::new(|call| {
        (call.method == "initialize" && call.launch == 1).then_some(Reply::Silent)
    });
    let supervisor = start(&launcher);
    let started = Instant::now();
    let status = until(&supervisor, |s| {
        matches!(s, SidecarStatus::Restarting { .. })
    })
    .await;
    assert_eq!(started.elapsed(), secs(30));
    let SidecarStatus::Restarting { reason, .. } = status else {
        unreachable!()
    };
    assert!(
        reason.contains("initialize") && reason.contains("30 s"),
        "{reason}"
    );
    running(&supervisor).await;
}

#[tokio::test(start_paused = true)]
async fn an_activate_the_sidecar_refuses_is_a_failed_start() {
    let launcher = FakeLauncher::new(|call| {
        (call.method == "activate" && call.launch == 1)
            .then_some(Reply::Error(-32000, "no database"))
    });
    let supervisor = start(&launcher);
    let status = until(&supervisor, |s| {
        matches!(s, SidecarStatus::Restarting { .. })
    })
    .await;
    let SidecarStatus::Restarting { reason, .. } = status else {
        unreachable!()
    };
    assert!(
        reason.contains("activate") && reason.contains("no database"),
        "{reason}"
    );
}

#[tokio::test(start_paused = true)]
async fn a_request_past_the_timeout_is_cancelled_at_the_sidecar() {
    let launcher = FakeLauncher::new(|call| (call.method == "slow").then_some(Reply::Silent));
    let supervisor = start(&launcher);
    running(&supervisor).await;
    let started = Instant::now();
    let error = supervisor.request("slow", json!({})).await.unwrap_err();
    assert_eq!(started.elapsed(), secs(30));
    assert!(matches!(error, RequestError::TimedOut { .. }), "{error:?}");
    sleep(Duration::from_millis(1)).await;
    let received = launcher.received();
    let slow = received
        .iter()
        .find(|(_, m)| m["method"] == "slow")
        .unwrap();
    let cancel = received
        .iter()
        .find(|(_, m)| m["method"] == "$/cancelRequest")
        .expect("the sidecar is told");
    assert_eq!(cancel.1["params"]["id"], slow.1["id"]);
}

#[tokio::test(start_paused = true)]
async fn a_dropped_request_is_cancelled_at_the_sidecar() {
    let launcher = FakeLauncher::new(|call| (call.method == "slow").then_some(Reply::Silent));
    let supervisor = Arc::new(start(&launcher));
    running(&supervisor).await;
    let asking = tokio::spawn({
        let supervisor = supervisor.clone();
        async move { supervisor.request("slow", json!({})).await }
    });
    sleep(secs(1)).await;
    asking.abort();
    sleep(secs(1)).await;
    assert!(launcher.methods().contains(&"$/cancelRequest".to_owned()));
}

#[tokio::test(start_paused = true)]
async fn requests_and_streams_are_capped_through_the_supervisor() {
    let launcher = FakeLauncher::new(|call| (call.method == "slow").then_some(Reply::Silent));
    let supervisor = Arc::new(start(&launcher));
    running(&supervisor).await;
    let mut streams = Vec::new();
    for _ in 0..5 {
        streams.push(
            supervisor
                .open_stream("watch", json!({}))
                .await
                .expect("opens"),
        );
    }
    let sixth = supervisor
        .open_stream("watch", json!({}))
        .await
        .unwrap_err();
    assert_eq!(sixth, RequestError::TooManyStreams { limit: 5 });
    let mut waiting = Vec::new();
    for _ in 0..8 {
        let supervisor = supervisor.clone();
        waiting.push(tokio::spawn(async move {
            supervisor.request("slow", json!({})).await
        }));
    }
    sleep(secs(1)).await;
    let ninth = supervisor.request("slow", json!({})).await.unwrap_err();
    assert_eq!(ninth, RequestError::Busy { limit: 8 });
    // The health check still gets through.
    assert_eq!(supervisor.health().await, Ok(()));
}

#[tokio::test(start_paused = true)]
async fn a_stream_carries_the_sidecars_frames_and_fails_when_it_crashes() {
    let launcher = FakeLauncher::new(|call| match call.method {
        "stream/open" => {
            let stream = call.params["stream"].clone();
            call.notify(
                "stream/data",
                json!({"stream": stream, "data": {"progress": 50}}),
            );
            Some(Reply::Result(json!({})))
        }
        "boom" => Some(Reply::Crash),
        _ => None,
    });
    let supervisor = start(&launcher);
    running(&supervisor).await;
    let mut stream = supervisor
        .open_stream("scan", json!({"image": "x"}))
        .await
        .unwrap();
    assert_eq!(
        next(&mut stream).await,
        Some(StreamEvent::Data(json!({"progress": 50})))
    );
    let _ = supervisor.request("boom", json!({})).await;
    let Some(StreamEvent::Failed(why)) = next(&mut stream).await else {
        panic!("the stream did not fail");
    };
    assert!(why.contains("exited unexpectedly"), "{why}");
    assert_eq!(next(&mut stream).await, None);
}

#[tokio::test(start_paused = true)]
async fn stop_deactivates_then_shuts_down_and_lets_the_sidecar_exit() {
    let launcher = FakeLauncher::well_behaved();
    let supervisor = start(&launcher);
    running(&supervisor).await;
    supervisor.stop().await;
    assert_eq!(supervisor.status(), SidecarStatus::Stopped);
    assert_eq!(
        launcher.methods(),
        ["initialize", "activate", "deactivate", "shutdown"]
    );
    assert!(launcher.killed().is_empty(), "it exited on its own");
    let error = supervisor.request("echo", json!({})).await.unwrap_err();
    assert!(matches!(error, RequestError::Unavailable(_)), "{error:?}");
    sleep(secs(3600)).await;
    assert_eq!(launcher.launches(), 1, "a stopped sidecar stays stopped");
}

#[tokio::test(start_paused = true)]
async fn stop_kills_a_sidecar_that_will_not_exit() {
    let launcher = FakeLauncher::new(|call| (call.method == "shutdown").then_some(Reply::Hang));
    let supervisor = start(&launcher);
    running(&supervisor).await;
    let started = Instant::now();
    supervisor.stop().await;
    // 5 s for shutdown to be answered, then 5 s to exit.
    assert_eq!(started.elapsed(), secs(10));
    assert_eq!(launcher.killed(), [1]);
    assert_eq!(supervisor.status(), SidecarStatus::Stopped);
}

#[tokio::test(start_paused = true)]
async fn a_stopped_sidecar_can_be_started_again() {
    let launcher = FakeLauncher::well_behaved();
    let supervisor = start(&launcher);
    running(&supervisor).await;
    supervisor.stop().await;
    supervisor.restart();
    running(&supervisor).await;
    assert_eq!(launcher.launches(), 2);
}

#[tokio::test(start_paused = true)]
async fn a_sandbox_this_machine_cannot_provide_is_refused_without_retrying() {
    let launcher =
        FakeLauncher::well_behaved().unavailable("Landlock is not enabled on this kernel");
    let supervisor = start(&launcher);
    let SidecarStatus::Refused { reason } =
        until(&supervisor, |s| matches!(s, SidecarStatus::Refused { .. })).await
    else {
        unreachable!()
    };
    assert_eq!(reason, "Landlock is not enabled on this kernel");
    sleep(secs(3600)).await;
    assert_eq!(launcher.launches(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_backend_that_enforces_no_limits_is_refused_before_anything_starts() {
    let launcher = FakeLauncher::well_behaved().enforcing(Enforcement::Missing(
        "macOS has no memory limit yet (#713)".into(),
    ));
    let supervisor = start(&launcher);
    let SidecarStatus::Refused { reason } =
        until(&supervisor, |s| matches!(s, SidecarStatus::Refused { .. })).await
    else {
        unreachable!()
    };
    assert!(reason.contains("#713"), "{reason}");
    assert_eq!(launcher.launches(), 0);
}

#[tokio::test(start_paused = true)]
async fn host_limits_are_enough_to_start() {
    let launcher = FakeLauncher::well_behaved().enforcing(Enforcement::Host);
    let supervisor = start(&launcher);
    running(&supervisor).await;
}

#[tokio::test(start_paused = true)]
async fn the_log_keeps_what_the_sidecar_wrote_and_what_happened_to_it() {
    let launcher = FakeLauncher::new(|call| {
        (call.method == "activate" && call.launch == 1).then_some(Reply::Crash)
    })
    .logging("scanner: opening database");
    let supervisor = start(&launcher);
    running(&supervisor).await;
    let logs = supervisor.logs();
    assert!(
        logs.iter()
            .any(|l| l.source == LogSource::Sidecar && l.text == "scanner: opening database"),
        "{logs:?}"
    );
    assert!(
        logs.iter()
            .any(|l| l.source == LogSource::Host && l.text.contains("SIGABRT")),
        "{logs:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn dropping_the_supervisor_stops_the_sidecar() {
    let launcher = FakeLauncher::well_behaved();
    let supervisor = start(&launcher);
    running(&supervisor).await;
    drop(supervisor);
    sleep(secs(1)).await;
    assert_eq!(launcher.killed(), [1]);
}

#[tokio::test(start_paused = true)]
async fn a_request_before_the_sidecar_is_running_says_so() {
    let launcher = FakeLauncher::new(|call| (call.method == "initialize").then_some(Reply::Silent));
    let supervisor = start(&launcher);
    until(&supervisor, |s| matches!(s, SidecarStatus::Starting)).await;
    let error = supervisor.request("echo", json!({})).await.unwrap_err();
    let RequestError::Unavailable(why) = error else {
        panic!("{error:?}")
    };
    assert!(why.contains("starting"), "{why}");
}

#[tokio::test(start_paused = true)]
async fn an_app_request_cannot_stop_the_sidecar() {
    let launcher = FakeLauncher::well_behaved();
    let supervisor = start(&launcher);
    running(&supervisor).await;
    let error = supervisor
        .request("shutdown", Value::Null)
        .await
        .unwrap_err();
    assert_eq!(
        error,
        RequestError::Reserved {
            method: "shutdown".into()
        }
    );
    assert!(!launcher.methods().contains(&"shutdown".to_owned()));
}

/// A supervisor for `launcher` whose data directory may hold `bytes`.
fn with_data_limit(launcher: &FakeLauncher, bytes: u64) -> (Supervisor, PathBuf) {
    let mut config = config();
    config.limits.data_bytes = bytes;
    let data = config.command.data_dir.clone();
    let supervisor = Supervisor::start(config, Arc::new(launcher.clone()), Arc::new(NoBroker));
    (supervisor, data)
}

async fn refused(supervisor: &Supervisor) -> String {
    let status = until(supervisor, |s| matches!(s, SidecarStatus::Refused { .. })).await;
    let SidecarStatus::Refused { reason } = status else {
        unreachable!()
    };
    reason
}

#[tokio::test(start_paused = true)]
async fn a_data_directory_over_its_limit_is_refused_before_anything_starts() {
    let launcher = FakeLauncher::well_behaved();
    let mut config = config();
    config.limits.data_bytes = 1024 * 1024;
    std::fs::write(config.command.data_dir.join("cache.db"), vec![0u8; 2 << 20]).unwrap();
    let supervisor = Supervisor::start(config, Arc::new(launcher.clone()), Arc::new(NoBroker));
    let reason = refused(&supervisor).await;
    assert!(reason.contains("over its 1 MiB limit"), "{reason}");
    assert!(reason.contains("did not start it"), "{reason}");
    sleep(secs(3600)).await;
    assert_eq!(launcher.launches(), 0, "a start would fail the same way");
}

#[tokio::test(start_paused = true)]
async fn a_sidecar_that_fills_its_data_directory_past_the_limit_is_stopped_and_not_restarted() {
    let written = Arc::new(OnceLock::<PathBuf>::new());
    let target = written.clone();
    let launcher = FakeLauncher::new(move |call| {
        (call.method == "fill").then(|| {
            let path = target.get().expect("the data directory").join("trivy.db");
            std::fs::write(path, vec![0u8; 3 << 20]).unwrap();
            Reply::Result(json!({}))
        })
    });
    let (supervisor, data) = with_data_limit(&launcher, 2 << 20);
    written.set(data).unwrap();
    running(&supervisor).await;
    let started = Instant::now();
    supervisor.request("fill", json!({})).await.unwrap();
    let reason = refused(&supervisor).await;
    assert!(reason.contains("holds 3 MiB, over its 2 MiB limit"), "{reason}");
    assert!(reason.contains("so srelens stopped it"), "{reason}");
    assert!(
        started.elapsed() <= Policy::default().data_check_interval,
        "measured only after {:?}",
        started.elapsed()
    );
    assert_eq!(launcher.killed(), [1]);
    sleep(secs(3600)).await;
    assert_eq!(launcher.launches(), 1, "restarting it would only refill it");
    let error = supervisor.request("echo", json!({})).await.unwrap_err();
    assert_eq!(error, RequestError::Unavailable(reason));
}

#[tokio::test(start_paused = true)]
async fn a_sidecar_within_its_data_limit_keeps_running() {
    let launcher = FakeLauncher::well_behaved();
    let (supervisor, data) = with_data_limit(&launcher, 2 << 20);
    std::fs::write(data.join("small.db"), vec![0u8; 1 << 20]).unwrap();
    running(&supervisor).await;
    sleep(secs(60)).await;
    assert!(matches!(supervisor.status(), SidecarStatus::Running { .. }));
    assert!(launcher.killed().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_restart_after_clearing_the_data_directory_runs_again() {
    let launcher = FakeLauncher::well_behaved();
    let (supervisor, data) = with_data_limit(&launcher, 1024 * 1024);
    running(&supervisor).await;
    std::fs::write(data.join("big.db"), vec![0u8; 2 << 20]).unwrap();
    refused(&supervisor).await;
    std::fs::remove_file(data.join("big.db")).unwrap();
    supervisor.restart();
    running(&supervisor).await;
    assert_eq!(launcher.launches(), 2);
}

#[cfg(unix)]
#[tokio::test(start_paused = true)]
async fn a_data_directory_other_users_can_read_is_refused_and_a_running_one_stopped() {
    use std::os::unix::fs::PermissionsExt;
    let open = |path: &std::path::Path| {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap()
    };
    // Before the start.
    let launcher = FakeLauncher::well_behaved();
    let config = config();
    open(&config.command.data_dir);
    let supervisor = Supervisor::start(config, Arc::new(launcher.clone()), Arc::new(NoBroker));
    let reason = refused(&supervisor).await;
    assert!(reason.contains("other users"), "{reason}");
    assert_eq!(launcher.launches(), 0);
    // While running: a sidecar can change its own directory's mode.
    let launcher = FakeLauncher::well_behaved();
    let (supervisor, data) = with_data_limit(&launcher, 1 << 30);
    running(&supervisor).await;
    open(&data);
    let reason = refused(&supervisor).await;
    assert!(reason.contains("other users"), "{reason}");
    assert_eq!(launcher.killed(), [1]);
}

/// A broker whose calls wait for ever, as one waiting on a person's
/// confirmation does, and which says when one starts and when one is dropped.
struct Waiting {
    started: tokio::sync::mpsc::UnboundedSender<()>,
    dropped: Arc<std::sync::atomic::AtomicBool>,
}

struct Flag(Arc<std::sync::atomic::AtomicBool>);

impl Drop for Flag {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

impl srelens_plugin_host::sidecar::Broker for Waiting {
    fn call<'a>(
        &'a self,
        _method: &'a str,
        _params: Value,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<Value, srelens_plugin_host::sidecar::protocol::RpcError>,
                > + Send
                + 'a,
        >,
    > {
        let flag = Flag(self.dropped.clone());
        let _ = self.started.send(());
        Box::pin(async move {
            let _flag = flag;
            std::future::pending::<()>().await;
            Ok(Value::Null)
        })
    }
}

/// The same while it is still starting: a sidecar may call the host before
/// it has answered `initialize`, and its session is not yet the running one.
#[tokio::test(start_paused = true)]
async fn dropping_the_supervisor_while_it_starts_drops_the_calls_the_sidecar_made() {
    let launcher = FakeLauncher::new(|call| {
        (call.method == "initialize").then(|| {
            call.call_host("c-1", "host/action", json!({}));
            Reply::Silent
        })
    });
    let (started, mut calls) = tokio::sync::mpsc::unbounded_channel();
    let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let broker = Waiting {
        started,
        dropped: dropped.clone(),
    };
    let supervisor = Supervisor::start(config(), Arc::new(launcher.clone()), Arc::new(broker));
    tokio::time::timeout(secs(3600), calls.recv())
        .await
        .expect("the sidecar's call reached the broker");
    assert_eq!(supervisor.status(), SidecarStatus::Starting);
    drop(supervisor);
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    assert!(
        dropped.load(std::sync::atomic::Ordering::SeqCst),
        "the call outlived the supervisor"
    );
}

/// A confirmation must not outlive the process that asked for it: dropping
/// the supervisor ends the session, and with it every call the sidecar made.
#[tokio::test(start_paused = true)]
async fn dropping_the_supervisor_drops_every_call_the_sidecar_is_waiting_on() {
    let launcher = FakeLauncher::new(|call| {
        (call.method == "ask").then(|| {
            call.call_host("c-1", "host/action", json!({}));
            Reply::Result(json!({}))
        })
    });
    let (started, mut calls) = tokio::sync::mpsc::unbounded_channel();
    let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let broker = Waiting {
        started,
        dropped: dropped.clone(),
    };
    let supervisor = Supervisor::start(config(), Arc::new(launcher.clone()), Arc::new(broker));
    running(&supervisor).await;
    supervisor.request("ask", json!({})).await.unwrap();
    tokio::time::timeout(secs(3600), calls.recv())
        .await
        .expect("the sidecar's call reached the broker");
    drop(supervisor);
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    assert!(
        dropped.load(std::sync::atomic::Ordering::SeqCst),
        "the call outlived the supervisor"
    );
}

// The Inspector's view of a sidecar (#575): `Supervisor::metrics` and the
// app's log.

#[tokio::test(start_paused = true)]
async fn the_inspector_reads_a_running_sidecars_process_and_memory() {
    let launcher = FakeLauncher::well_behaved().using_memory(48 * 1024 * 1024);
    let supervisor = start(&launcher);
    running(&supervisor).await;
    let metrics = supervisor.metrics();
    assert!(matches!(
        &metrics.status,
        SidecarStatus::Running { api_version, pid: Some(1) } if *api_version == semver::Version::new(0, 1, 0)
    ));
    assert!(metrics.started_at.is_some());
    assert_eq!((metrics.launches, metrics.unexpected_exits), (1, 0));
    assert_eq!(metrics.memory_bytes, Some(48 * 1024 * 1024));
    assert_eq!(metrics.limits, Limits::default());
    assert_eq!(metrics.enforcement, Enforcement::Kernel);
    assert_eq!(metrics.requests, RequestMetrics::default());
    assert_eq!((metrics.open_streams, metrics.streams_opened), (0, 0));
}

#[tokio::test(start_paused = true)]
async fn a_backend_that_cannot_measure_memory_says_nothing_rather_than_zero() {
    let launcher = FakeLauncher::well_behaved();
    let supervisor = start(&launcher);
    running(&supervisor).await;
    assert_eq!(supervisor.metrics().memory_bytes, None);
}

#[tokio::test(start_paused = true)]
async fn request_latency_and_failures_are_counted_and_failures_logged() {
    let launcher = FakeLauncher::new(|call| match call.method {
        "slow" => Some(Reply::Late(Duration::from_millis(250), json!({}))),
        "broken" => Some(Reply::Error(-1, "the database is locked")),
        "hang" => Some(Reply::Silent),
        _ => None,
    });
    let supervisor = start(&launcher);
    running(&supervisor).await;
    supervisor.request("fast", json!({})).await.unwrap();
    supervisor.request("slow", json!({})).await.unwrap();
    supervisor.request("broken", json!({})).await.unwrap_err();
    supervisor.request("hang", json!({})).await.unwrap_err();
    assert_eq!(
        supervisor.request("stream/data", json!({})).await,
        Err(RequestError::Reserved {
            method: "stream/data".into()
        })
    );

    let requests = supervisor.metrics().requests;
    assert_eq!(
        (
            requests.answered,
            requests.failed,
            requests.timed_out,
            requests.refused
        ),
        (2, 1, 1, 1)
    );
    assert_eq!(requests.in_flight, 0);
    // Answers with a result or an error: the timeout and the refusal are not round trips.
    assert_eq!(requests.latency.samples, 3);
    assert_eq!(requests.latency.max, Some(Duration::from_millis(250)));
    assert_eq!(requests.latency.p50, Some(Duration::ZERO));

    let errors: Vec<String> = supervisor
        .log()
        .recent_errors()
        .into_iter()
        .map(|l| l.text)
        .collect();
    assert_eq!(
        errors,
        [
            "`broken` failed: The extension answered with an error: the database is locked",
            "`hang` failed: The extension did not answer `hang` within 30 s",
        ]
    );
    let refused = supervisor
        .logs()
        .into_iter()
        .find(|l| l.text.starts_with("`stream/data`"))
        .expect("the refusal is logged");
    assert_eq!(
        (refused.level, refused.source),
        (LogLevel::Warn, LogSource::Host)
    );
}

#[tokio::test(start_paused = true)]
async fn requests_to_a_stopped_sidecar_are_counted_but_not_logged_each_time() {
    let launcher = FakeLauncher::well_behaved();
    let supervisor = start(&launcher);
    running(&supervisor).await;
    supervisor.stop().await;
    let before = supervisor.logs().len();
    for _ in 0..3 {
        supervisor.request("scan", json!({})).await.unwrap_err();
    }
    assert_eq!(supervisor.metrics().requests.refused, 3);
    assert_eq!(supervisor.logs().len(), before);
}

#[tokio::test(start_paused = true)]
async fn the_inspector_counts_the_sidecars_open_streams_and_requests_in_flight() {
    let launcher = FakeLauncher::new(|call| (call.method == "slow").then_some(Reply::Silent));
    let supervisor = Arc::new(start(&launcher));
    running(&supervisor).await;
    let first = supervisor.open_stream("watch", json!({})).await.unwrap();
    let second = supervisor.open_stream("watch", json!({})).await.unwrap();
    let waiting = tokio::spawn({
        let supervisor = supervisor.clone();
        async move { supervisor.request("slow", json!({})).await }
    });
    sleep(secs(1)).await;
    let metrics = supervisor.metrics();
    assert_eq!((metrics.open_streams, metrics.streams_opened), (2, 2));
    assert_eq!(metrics.requests.in_flight, 1);
    drop(first);
    assert_eq!(supervisor.metrics().open_streams, 1);
    drop(second);
    waiting.abort();
    sleep(secs(1)).await;
    let metrics = supervisor.metrics();
    assert_eq!((metrics.open_streams, metrics.streams_opened), (0, 2));
    assert_eq!(metrics.requests.in_flight, 0);
}

#[tokio::test(start_paused = true)]
async fn a_crashed_sidecar_reads_as_disabled_with_its_reason_and_view_logs() {
    let launcher = FakeLauncher::new(|call| (call.method == "activate").then_some(Reply::Crash))
        .using_memory(1024);
    let supervisor = start(&launcher);
    until(&supervisor, |s| matches!(s, SidecarStatus::Disabled { .. })).await;
    let metrics = supervisor.metrics();
    let SidecarStatus::Disabled { reason } = &metrics.status else {
        panic!("{:?}", metrics.status)
    };
    assert!(reason.contains("SIGABRT"), "{reason}");
    assert_eq!(metrics.status.message(), Some(UNEXPECTED_EXIT));
    assert!(metrics.status.actions().contains(&Action::ViewLogs));
    assert_eq!((metrics.launches, metrics.unexpected_exits), (4, 4));
    assert_eq!(metrics.started_at, None);
    assert_eq!(metrics.memory_bytes, None, "nothing is running to measure");
    assert_eq!(metrics.open_streams, 0);

    // What View logs opens: each exit, each restart, and the disable.
    let log = supervisor.log();
    let restarts: Vec<String> = log
        .lines()
        .into_iter()
        .filter(|l| l.level == LogLevel::Warn)
        .map(|l| l.text)
        .collect();
    assert_eq!(
        restarts,
        [
            "Restarting the extension in 1 s (attempt 1 of 3)",
            "Restarting the extension in 5 s (attempt 2 of 3)",
            "Restarting the extension in 30 s (attempt 3 of 3)",
        ]
    );
    let errors = log.recent_errors();
    assert_eq!(errors.len(), 5);
    assert!(
        errors[..4].iter().all(|l| l.text.contains("SIGABRT")),
        "{errors:?}"
    );
    assert!(errors[4].text.starts_with(UNEXPECTED_EXIT), "{errors:?}");
}

#[tokio::test(start_paused = true)]
async fn the_sidecars_stderr_is_kept_at_its_level_and_redacted() {
    let launcher = FakeLauncher::well_behaved()
        .logging("WARN registry is slow")
        .logging("DEBUG pulling with token=ghp_0123456789abcdefghijABCDEFGHIJ012345")
        .logging("opening database");
    let supervisor = start(&launcher);
    running(&supervisor).await;
    // stderr is read beside the protocol; give it a moment.
    sleep(secs(1)).await;
    let sidecar: Vec<(LogLevel, String)> = supervisor
        .logs()
        .into_iter()
        .filter(|l| l.source == LogSource::Sidecar)
        .map(|l| (l.level, l.text))
        .collect();
    assert_eq!(
        sidecar,
        [
            (LogLevel::Warn, "registry is slow".to_owned()),
            (LogLevel::Debug, "pulling with token=<redacted>".to_owned()),
            (LogLevel::Info, "opening database".to_owned()),
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn a_reason_that_quotes_the_sidecar_is_redacted_for_the_inspector() {
    let launcher = FakeLauncher::new(|call| {
        (call.method == "health").then_some(Reply::Error(-1, "cannot log in with password=hunter2"))
    });
    let supervisor = start(&launcher);
    running(&supervisor).await;
    until(&supervisor, |s| {
        matches!(s, SidecarStatus::Restarting { .. })
    })
    .await;
    let SidecarStatus::Restarting { reason, .. } = supervisor.metrics().status else {
        panic!("not restarting")
    };
    assert!(reason.contains("unhealthy"), "{reason}");
    assert!(!reason.contains("hunter2"), "{reason}");
    let everything = format!("{:?}", supervisor.logs());
    assert!(!everything.contains("hunter2"), "{everything}");
}

#[tokio::test(start_paused = true)]
async fn a_value_the_host_scrubs_never_reaches_the_log() {
    let log = AppLog::new();
    log.scrub("registry-robot-s3cret");
    let launcher = FakeLauncher::well_behaved().logging("INFO logging in as registry-robot-s3cret");
    let supervisor = Supervisor::start_with_log(
        config(),
        Arc::new(launcher.clone()),
        Arc::new(NoBroker),
        log.clone(),
    );
    running(&supervisor).await;
    sleep(secs(1)).await;
    let texts: Vec<String> = log.lines().into_iter().map(|l| l.text).collect();
    assert!(
        texts.contains(&"logging in as <redacted>".to_owned()),
        "{texts:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn the_hosts_log_outlives_the_supervisor_that_wrote_to_it() {
    let log = AppLog::new();
    let launcher = FakeLauncher::well_behaved();
    let supervisor = Supervisor::start_with_log(
        config(),
        Arc::new(launcher.clone()),
        Arc::new(NoBroker),
        log.clone(),
    );
    running(&supervisor).await;
    drop(supervisor);
    let restarted = Supervisor::start_with_log(
        config(),
        Arc::new(launcher.clone()),
        Arc::new(NoBroker),
        log.clone(),
    );
    running(&restarted).await;
    let running_lines = log
        .lines()
        .into_iter()
        .filter(|l| l.text == "The extension is running")
        .count();
    assert_eq!(running_lines, 2);
}
