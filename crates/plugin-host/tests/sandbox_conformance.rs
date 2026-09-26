//! The sandbox conformance suite (#572): the #571 spike's seven checks
//! (`spikes/sidecar-sandbox/tests/checks.rs`), run against the production
//! backend for this OS through the supervisor, plus two for what the spike
//! left open: the host's environment, and Unix sockets.
//!
//! Every test is `#[ignore]`: each needs an OS sandbox, network access for
//! its positive controls, and on Linux a delegated cgroup. The
//! `sandbox-conformance` CI job runs them on Linux and Windows:
//!
//! ```text
//! SRELENS_SANDBOX_CGROUP_ROOT=/sys/fs/cgroup/<delegated> \
//!   cargo test -p srelens-plugin-host --test sandbox_conformance -- --ignored --test-threads=1
//! ```
//!
//! A "must be denied" check passes only when, as in the spike:
//!
//! 1. the sidecar is alive and answers that the operation failed,
//! 2. the host itself could do the same operation (the positive control;
//!    without it the result says nothing about the sandbox: INCONCLUSIVE), and
//! 3. the failure is one a sandbox produces for that operation (`Denial`).
//!
//! On macOS the supervisor refuses sidecars, because nothing limits their
//! memory and CPU until #713; the isolation checks run there anyway, through a
//! launcher that vouches for limits (see `IsolationOnly`), and the memory and
//! CPU checks are skipped.

use serde_json::{json, Value};
use srelens_plugin_host::sidecar::{
    Enforcement, LaunchError, Launched, Launcher, Limits, NoBroker, OsSandbox, Policy,
    RequestError, SandboxConfig, SidecarCommand, SidecarConfig, SidecarStatus, Supervisor,
};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const PROBE: &str = env!("CARGO_BIN_EXE_srelens-sidecar-probe");
const LAUNCHER: &str = env!("CARGO_BIN_EXE_srelens-sandbox-launch");
const APP_ID: &str = "org.srelens.sandbox-conformance";

/// The spike's limits.
fn limits() -> Limits {
    Limits {
        memory_bytes: 128 * 1024 * 1024,
        cpus: 0.25,
        ..Limits::default()
    }
}

/// On macOS, the OS sandbox with its missing limits vouched for, so the
/// isolation it does provide can be checked. Anywhere else, the OS sandbox
/// unchanged.
struct IsolationOnly(OsSandbox);

impl Launcher for IsolationOnly {
    fn enforcement(&self) -> Enforcement {
        match self.0.enforcement() {
            Enforcement::Missing(_) if cfg!(target_os = "macos") => Enforcement::Host,
            other => other,
        }
    }

    fn launch(&self, command: &SidecarCommand, limits: &Limits) -> Result<Launched, LaunchError> {
        self.0.launch(command, limits)
    }
}

/// Throwaway directories for one run:
///
/// ```text
/// <root>/bin/probe[.exe]      the sidecar, copied here
/// <root>/data/                the one directory it is granted
/// <root>/home/.kube/config    a stand-in kubeconfig
/// <root>/outside/secret.txt   an ordinary file outside the grant
/// ```
struct Fixture {
    root: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Fixture {
        let root = tempfile::tempdir().expect("a temporary directory");
        let fixture = Fixture { root };
        for dir in [fixture.path("bin"), fixture.data(), fixture.path("outside")] {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::create_dir_all(fixture.kubeconfig().parent().unwrap()).unwrap();
        std::fs::write(fixture.kubeconfig(), "apiVersion: v1\nkind: Config\n").unwrap();
        std::fs::write(fixture.outside_file(), "not for the sidecar\n").unwrap();
        std::fs::copy(PROBE, fixture.probe()).unwrap();
        fixture
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.root.path().join(rel)
    }
    fn probe(&self) -> PathBuf {
        self.path("bin")
            .join(format!("probe{}", std::env::consts::EXE_SUFFIX))
    }
    fn data(&self) -> PathBuf {
        self.path("data")
    }
    fn kubeconfig(&self) -> PathBuf {
        self.path("home").join(".kube").join("config")
    }
    fn outside_file(&self) -> PathBuf {
        self.path("outside").join("secret.txt")
    }
}

fn sandbox() -> OsSandbox {
    OsSandbox::new(SandboxConfig {
        launcher: Some(LAUNCHER.into()),
        cgroup_root: std::env::var_os("SRELENS_SANDBOX_CGROUP_ROOT").map(PathBuf::from),
    })
}

/// A running, sandboxed probe.
async fn sidecar(fixture: &Fixture) -> Supervisor {
    let config = SidecarConfig {
        command: SidecarCommand {
            app_id: APP_ID.into(),
            program: fixture.probe(),
            args: Vec::new(),
            env: vec![("PROBE_MARK".into(), "1".into())],
            data_dir: fixture.data(),
        },
        limits: limits(),
        policy: Policy {
            // One start per test: a stop is what some checks look for.
            backoff: Vec::new(),
            ..Policy::default()
        },
    };
    let supervisor = Supervisor::start(
        config,
        Arc::new(IsolationOnly(sandbox())),
        Arc::new(NoBroker),
    );
    let mut status = supervisor.watch();
    let started = tokio::time::timeout(
        Duration::from_secs(60),
        status.wait_for(|s| !matches!(s, SidecarStatus::Starting)),
    )
    .await
    .expect("the sidecar started or failed within a minute")
    .expect("the supervisor is running")
    .clone();
    assert!(
        matches!(started, SidecarStatus::Running { .. }),
        "the probe did not start under the sandbox: {started:?}\nlog: {:#?}",
        supervisor.logs()
    );
    supervisor
}

/// A refusal the probe reported: its error kind and OS code.
#[derive(Debug)]
struct Failure {
    /// Shown through `Debug` when a check fails.
    #[allow(dead_code)]
    message: String,
    kind: String,
    os: Option<i64>,
}

/// What came back from one probe request.
#[derive(Debug)]
// The memory check, the one reader of `Stopped`'s text, does not run on macOS.
#[cfg_attr(target_os = "macos", allow(dead_code))]
enum Reply {
    Ok(Value),
    Refused(Failure),
    /// It did not answer: it stopped, or the request failed otherwise.
    Stopped(String),
}

async fn call(sidecar: &Supervisor, method: &str, params: Value) -> Reply {
    match sidecar.request(method, params).await {
        Ok(value) => Reply::Ok(value),
        Err(RequestError::Failed(error)) => {
            let data = error.data.unwrap_or(Value::Null);
            Reply::Refused(Failure {
                message: error.message,
                kind: data["kind"].as_str().unwrap_or("?").to_owned(),
                os: data["os"].as_i64(),
            })
        }
        Err(other) => Reply::Stopped(other.to_string()),
    }
}

/// Which refusal a check expects. The spike's `Denial::accepts`.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(target_os = "macos", allow(dead_code))]
enum Denial {
    File,
    Network,
    Dns,
    Process,
    Memory,
}

impl Denial {
    fn accepts(self, failure: &Failure) -> bool {
        let kind = failure.kind.as_str();
        match self {
            Denial::File => matches!(kind, "PermissionDenied" | "NotFound" | "ReadOnlyFilesystem"),
            Denial::Network => matches!(
                kind,
                "PermissionDenied"
                    | "TimedOut"
                    | "ConnectionRefused"
                    | "NetworkUnreachable"
                    | "HostUnreachable"
            ),
            Denial::Dns => matches!(kind, "ResolverFailed" | "PermissionDenied"),
            // seccomp's or Seatbelt's EPERM, not EACCES, which is a filesystem
            // refusal (of /dev/null, say).
            #[cfg(unix)]
            Denial::Process => failure.os == Some(libc::EPERM as i64),
            // The Job Object's active-process limit: ERROR_NOT_ENOUGH_QUOTA.
            #[cfg(windows)]
            Denial::Process => failure.os == Some(1816),
            Denial::Memory => kind == "OutOfMemory",
        }
    }
}

/// The sidecar must refuse `method` with a sandbox's refusal, after the host
/// has done the same thing itself.
async fn assert_denied(
    sidecar: &Supervisor,
    what: &str,
    control: Result<(), String>,
    method: &str,
    params: Value,
    denial: Denial,
) {
    if let Err(why) = control {
        panic!("{what}: INCONCLUSIVE, the host itself could not do it ({why}), so a refusal says nothing about the sandbox");
    }
    let reply = call(sidecar, method, params).await;
    eprintln!("{what}: {reply:?}");
    match reply {
        Reply::Refused(failure) if denial.accepts(&failure) => {}
        Reply::Refused(failure) => {
            panic!("{what}: failed, but not as a sandbox refuses ({failure:?})")
        }
        other => panic!("{what}: not denied: {other:?}"),
    }
}

fn host_reads(path: &Path) -> Result<(), String> {
    std::fs::read(path).map(|_| ()).map_err(|e| e.to_string())
}

#[tokio::test]
#[ignore = "needs this OS's sandbox; run by the sandbox-conformance CI job"]
async fn check_1_the_kubeconfig_and_files_outside_the_grant_cannot_be_read() {
    let fixture = Fixture::new();
    let sidecar = sidecar(&fixture).await;
    for (what, path) in [
        ("~/.kube/config", fixture.kubeconfig()),
        ("a file outside the grant", fixture.outside_file()),
    ] {
        let params = json!({"path": path});
        assert_denied(
            &sidecar,
            &format!("read {what}"),
            host_reads(&path),
            "read_file",
            params,
            Denial::File,
        )
        .await;
    }
}

#[tokio::test]
#[ignore = "needs this OS's sandbox; run by the sandbox-conformance CI job"]
async fn check_2_the_data_directory_is_writable_and_nothing_else_is() {
    let fixture = Fixture::new();
    let sidecar = sidecar(&fixture).await;
    let inside = fixture.data().join("cache.db");
    let reply = call(
        &sidecar,
        "write_file",
        json!({"path": inside, "text": "ok"}),
    )
    .await;
    assert!(
        matches!(reply, Reply::Ok(_)),
        "write inside the data directory: {reply:?}"
    );
    let outside = fixture.path("outside").join("planted.txt");
    let control =
        std::fs::write(fixture.path("outside").join("control.txt"), "x").map_err(|e| e.to_string());
    let params = json!({"path": outside, "text": "planted"});
    assert_denied(
        &sidecar,
        "write outside the grant",
        control,
        "write_file",
        params,
        Denial::File,
    )
    .await;
    assert!(!outside.exists());
}

#[tokio::test]
#[ignore = "needs this OS's sandbox and a network; run by the sandbox-conformance CI job"]
async fn check_3_no_tcp_and_no_dns() {
    let fixture = Fixture::new();
    let sidecar = sidecar(&fixture).await;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let local = listener.local_addr().unwrap();
    let connects = |addr: std::net::SocketAddr| {
        TcpStream::connect_timeout(&addr, Duration::from_secs(5))
            .map(|_| ())
            .map_err(|e| e.to_string())
    };
    let params = json!({"addr": local.to_string()});
    assert_denied(
        &sidecar,
        "TCP to the host's loopback",
        connects(local),
        "tcp_connect",
        params,
        Denial::Network,
    )
    .await;
    let internet: std::net::SocketAddr = "1.1.1.1:443".parse().unwrap();
    let params = json!({"addr": internet.to_string()});
    assert_denied(
        &sidecar,
        "TCP to 1.1.1.1:443",
        connects(internet),
        "tcp_connect",
        params,
        Denial::Network,
    )
    .await;
    let resolves = ("example.com", 443)
        .to_socket_addrs()
        .map(|_| ())
        .map_err(|e| e.to_string());
    let params = json!({"host": "example.com"});
    assert_denied(
        &sidecar,
        "resolve example.com",
        resolves,
        "resolve",
        params,
        Denial::Dns,
    )
    .await;
}

#[tokio::test]
#[ignore = "needs this OS's sandbox; run by the sandbox-conformance CI job"]
async fn check_4_no_child_process() {
    let fixture = Fixture::new();
    let sidecar = sidecar(&fixture).await;
    let control = std::process::Command::new(fixture.probe())
        .arg("--child")
        .status()
        .map_err(|e| e.to_string())
        .and_then(|s| {
            if s.success() {
                Ok(())
            } else {
                Err(s.to_string())
            }
        });
    assert_denied(
        &sidecar,
        "start a child process",
        control,
        "spawn_child",
        json!({}),
        Denial::Process,
    )
    .await;
}

/// The same workload with no sandbox, for the memory and CPU controls.
#[cfg(not(target_os = "macos"))]
async fn unconfined(fixture: &Fixture, method: &str, params: Value) -> Value {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    let mut child = tokio::process::Command::new(fixture.probe())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let request = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
    stdin
        .write_all(format!("{request}\n").as_bytes())
        .await
        .unwrap();
    let mut lines = tokio::io::BufReader::new(child.stdout.take().unwrap()).lines();
    let line = lines.next_line().await.unwrap().expect("an answer");
    let answer: Value = serde_json::from_str(&line).unwrap();
    answer["result"].clone()
}

#[cfg(not(target_os = "macos"))]
#[tokio::test]
#[ignore = "needs this OS's sandbox; run by the sandbox-conformance CI job"]
async fn check_5_memory_past_the_limit_is_refused_or_stops_the_sidecar_at_the_limit() {
    let fixture = Fixture::new();
    let control = unconfined(&fixture, "allocate", json!({"mib": 512})).await;
    assert_eq!(
        control["mib"], 512,
        "INCONCLUSIVE: 512 MiB could not be allocated unconfined: {control}"
    );
    let sidecar = sidecar(&fixture).await;
    let reply = call(&sidecar, "allocate", json!({"mib": 512})).await;
    eprintln!("allocate 512 MiB (limit 128 MiB): {reply:?}");
    match reply {
        // Windows: the Job Object refuses the allocation; the sidecar lives on.
        Reply::Refused(failure) if Denial::Memory.accepts(&failure) => {
            assert!(matches!(
                call(&sidecar, "ping", json!({})).await,
                Reply::Ok(_)
            ));
        }
        // Linux: the cgroup OOM-kills it, with its counters as evidence.
        Reply::Stopped(why) => assert!(
            why.contains("memory limit"),
            "stopped, but not by the limit: {why}"
        ),
        other => panic!("not limited: {other:?}"),
    }
}

#[cfg(not(target_os = "macos"))]
#[tokio::test]
#[ignore = "needs this OS's sandbox; run by the sandbox-conformance CI job"]
async fn check_6_cpu_is_throttled_to_the_limit() {
    let fixture = Fixture::new();
    let burn = json!({"millis": 3000, "threads": 2});
    let control = unconfined(&fixture, "burn_cpu", burn.clone()).await;
    let cpus = |v: &Value| v["cpu_ms"].as_f64().unwrap() / v["wall_ms"].as_f64().unwrap();
    assert!(
        cpus(&control) > 0.375,
        "INCONCLUSIVE: only {:.2} CPUs unconfined",
        cpus(&control)
    );
    let sidecar = sidecar(&fixture).await;
    let Reply::Ok(used) = call(&sidecar, "burn_cpu", burn).await else {
        panic!("the CPU burn did not complete");
    };
    eprintln!("used {:.2} CPUs against 0.25", cpus(&used));
    assert!(
        cpus(&used) <= 0.375,
        "{:.2} CPUs is over 1.5 times the 0.25 limit",
        cpus(&used)
    );
}

#[tokio::test]
#[ignore = "needs this OS's sandbox; run by the sandbox-conformance CI job"]
async fn check_7_fifty_json_rpc_round_trips_work() {
    let fixture = Fixture::new();
    let sidecar = sidecar(&fixture).await;
    for n in 0..50 {
        let reply = call(&sidecar, "echo", json!({"n": n})).await;
        assert!(
            matches!(&reply, Reply::Ok(v) if v["n"] == n),
            "round trip {n}: {reply:?}"
        );
    }
}

#[tokio::test]
#[ignore = "needs this OS's sandbox; run by the sandbox-conformance CI job"]
async fn the_host_environment_does_not_reach_the_sidecar() {
    let fixture = Fixture::new();
    let sidecar = sidecar(&fixture).await;
    let Reply::Ok(answer) = call(&sidecar, "env", json!({})).await else {
        panic!("env did not answer");
    };
    let mut names: Vec<&str> = answer["names"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    names.sort_unstable();
    // On Windows, `SystemRoot` for Winsock, and the three variables Windows
    // reroutes into the AppContainer's own folder when it starts the process.
    let expected: &[&str] = if cfg!(windows) {
        &["LOCALAPPDATA", "PROBE_MARK", "SystemRoot", "TEMP", "TMP"]
    } else {
        &["PROBE_MARK"]
    };
    assert_eq!(names, expected, "the host's own variables leaked");
    #[cfg(windows)]
    for name in ["LOCALAPPDATA", "TEMP", "TMP"] {
        let theirs = answer["values"][name].as_str().unwrap_or_default();
        let ours = std::env::var(name).unwrap_or_default();
        eprintln!("{name}: the host's {ours}, the sidecar's {theirs}");
        assert_ne!(
            theirs, ours,
            "{name} was not rerouted: the host's value reached the sidecar"
        );
        assert!(
            theirs.contains("Packages"),
            "{name} is not under the AppContainer's profile: {theirs}"
        );
    }
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "needs this OS's sandbox; run by the sandbox-conformance CI job"]
async fn a_unix_socket_outside_the_grant_cannot_be_reached() {
    let fixture = Fixture::new();
    let sidecar = sidecar(&fixture).await;
    let path = fixture.path("outside").join("host.sock");
    let _listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
    let control = std::os::unix::net::UnixStream::connect(&path)
        .map(|_| ())
        .map_err(|e| e.to_string());
    let params = json!({"path": path});
    assert_denied(
        &sidecar,
        "connect to a host Unix socket",
        control,
        "unix_connect",
        params,
        Denial::Network,
    )
    .await;
}

#[cfg(windows)]
#[test]
#[ignore = "run last by the sandbox-conformance CI job"]
fn zz_cleanup_deletes_the_appcontainer_profile() {
    srelens_plugin_host::sidecar::sandbox::delete_profile(APP_ID).expect("the profile is deleted");
}
