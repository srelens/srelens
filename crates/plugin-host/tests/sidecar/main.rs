//! The sidecar supervisor (#572) against an in-process fake sidecar, on a
//! paused clock: every timeout and backoff is asserted to its exact length.
//! `sidecar_process.rs` runs the same lifecycle against a real process.

mod fake;

use fake::{FakeLauncher, Reply};
use serde_json::{json, Value};
use srelens_plugin_host::sidecar::{
    Action, Enforcement, Limits, LogSource, NoBroker, Policy, RequestError, SidecarCommand,
    SidecarConfig, SidecarStatus, StreamEvent, Supervisor, SIDECAR_API_VERSIONS, UNEXPECTED_EXIT,
};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::{sleep, Instant};

fn config() -> SidecarConfig {
    SidecarConfig {
        command: SidecarCommand {
            app_id: "org.example.scanner".into(),
            program: "/opt/example/scanner".into(),
            args: Vec::new(),
            env: Vec::new(),
            data_dir: std::env::temp_dir(),
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
