//! The sandbox conformance suite (#572): the #571 spike's seven checks
//! (`spikes/sidecar-sandbox/tests/checks.rs`), run against the production
//! backend for this OS through the supervisor, plus two for what the spike
//! left open: the host's environment, and Unix sockets. #573 adds three for
//! the data directory: that it is the only path the sidecar may write, that a
//! link inside it reaches nothing outside, and that its size limit holds.
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
//! On macOS the supervisor refuses sidecars until the #713 watchdog has been
//! checked on a Mac. The checks run there anyway, through a launcher that
//! vouches for limits (see `IsolationOnly`): the isolation checks, and the
//! memory and CPU checks against the watchdog `launch` attaches. Run them by
//! hand on a macOS 27 Mac:
//!
//! ```text
//! cargo test -p srelens-plugin-host --test sandbox_conformance -- --ignored --test-threads=1 --nocapture
//! ```

use serde_json::{json, Value};
use srelens_plugin_host::sidecar::data::DataDir;
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

/// On macOS, the OS sandbox with its limits vouched for, as they will be once
/// the watchdog is checked on a Mac, so its isolation and its watchdog can be
/// checked. Anywhere else, the OS sandbox unchanged.
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
/// <root>/apps/<digest>/       its data directory, the one it is granted (#573)
/// <root>/apps/<digest>/       another app's data directory
/// <root>/home/.kube/config    a stand-in kubeconfig
/// <root>/outside/secret.txt   an ordinary file outside the grant
/// ```
struct Fixture {
    root: tempfile::TempDir,
    data: DataDir,
    other: DataDir,
}

impl Fixture {
    fn new() -> Fixture {
        let root = tempfile::tempdir().expect("a temporary directory");
        let apps = root.path().join("apps");
        let data = DataDir::for_app(&apps, APP_ID).expect("the app's data directory");
        let other = DataDir::for_app(&apps, "org.srelens.another-app").expect("another's");
        let fixture = Fixture { root, data, other };
        for dir in [fixture.path("bin"), fixture.path("outside")] {
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
        self.data.path().to_owned()
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
    sidecar_with(fixture, limits()).await
}

/// A running, sandboxed probe under `limits`.
async fn sidecar_with(fixture: &Fixture, limits: Limits) -> Supervisor {
    let config = SidecarConfig {
        command: SidecarCommand {
            app_id: APP_ID.into(),
            program: fixture.probe(),
            args: Vec::new(),
            env: vec![("PROBE_MARK".into(), "1".into())],
            data_dir: fixture.data(),
        },
        limits,
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

/// How the sidecar stopped, once it has: `Reply::Stopped` with the
/// supervisor's reason, or `Reply::Ok` if it still runs after 5 s.
async fn stopped(sidecar: &Supervisor) -> Reply {
    let mut status = sidecar.watch();
    let disabled = tokio::time::timeout(
        Duration::from_secs(5),
        status.wait_for(|s| matches!(s, SidecarStatus::Disabled { .. })),
    )
    .await;
    match disabled.ok().and_then(Result::ok).map(|s| s.clone()) {
        Some(SidecarStatus::Disabled { reason }) => Reply::Stopped(reason),
        _ => Reply::Ok(Value::Null),
    }
}

/// The supervisor's memory reading once `ok` holds, or its last after 2 s:
/// on macOS it is the watchdog's last reading, up to 50 ms old.
async fn memory_reading(sidecar: &Supervisor, ok: impl Fn(u64) -> bool) -> Option<u64> {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        let reading = sidecar.metrics().memory_bytes;
        if reading.is_some_and(&ok) || std::time::Instant::now() >= deadline {
            return reading;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Which refusal a check expects. The spike's `Denial::accepts`.
#[derive(Debug, Clone, Copy)]
enum Denial {
    File,
    /// A hard link to a file outside the grant: a filesystem refusal, or
    /// Landlock's `EXDEV` for a link it will not let cross into the grant.
    Link,
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
            Denial::Link => Denial::File.accepts(failure) || kind == "CrossesDevices",
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

/// The ACL `icacls` reports for `path`.
#[cfg(windows)]
fn acl(path: &Path) -> String {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    std::process::Command::new(Path::new(&root).join("System32").join("icacls.exe"))
        .arg(path)
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
        .unwrap_or_else(|e| format!("icacls failed: {e}"))
}

/// The host writes `path` and removes it again: the positive control for a
/// write the sidecar must be refused.
fn host_writes(path: &Path) -> Result<(), String> {
    std::fs::write(path, "control").map_err(|e| e.to_string())?;
    std::fs::remove_file(path).map_err(|e| e.to_string())
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
    // macOS's watchdog bounds sustained use: memory allocated and freed again
    // between two of its readings can pass unseen, so there the probe holds it.
    let method = if cfg!(target_os = "macos") {
        "hold"
    } else {
        "allocate"
    };
    let reply = call(&sidecar, method, json!({"mib": 512})).await;
    eprintln!("{method} 512 MiB (limit 128 MiB): {reply:?}");
    let reply = match reply {
        // macOS: the reading that sees the held memory can come just after
        // the answer.
        Reply::Ok(_) if cfg!(target_os = "macos") => stopped(&sidecar).await,
        other => other,
    };
    match reply {
        // Windows: the Job Object refuses the allocation; the sidecar lives on.
        Reply::Refused(failure) if Denial::Memory.accepts(&failure) => {
            assert!(matches!(
                call(&sidecar, "ping", json!({})).await,
                Reply::Ok(_)
            ));
        }
        // Linux: the cgroup OOM-kills it, with its counters as evidence. macOS: the watchdog kills it.
        Reply::Stopped(why) => assert!(
            why.contains("memory limit"),
            "stopped, but not by the limit: {why}"
        ),
        other => panic!("not limited: {other:?}"),
    }
}

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
        // Set by LLVM's coverage runtime inside an instrumented probe, as it
        // starts: not passed by the host.
        .filter(|n| *n != "__LLVM_PROFILE_RT_INIT_ONCE")
        .collect();
    names.sort_unstable();
    // On Windows, `SystemRoot` for Winsock, and the three variables Windows
    // reroutes into the AppContainer's own folder when it starts the process.
    // On Linux and macOS, `TMPDIR`: the data directory, the one place its
    // temporary files can go (#573).
    let expected: &[&str] = if cfg!(windows) {
        &["LOCALAPPDATA", "PROBE_MARK", "SystemRoot", "TEMP", "TMP"]
    } else {
        &["PROBE_MARK", "TMPDIR"]
    };
    assert_eq!(names, expected, "the host's own variables leaked");
    #[cfg(unix)]
    {
        let tmpdir = answer["values"]["TMPDIR"].as_str().unwrap_or_default();
        let data = std::fs::canonicalize(fixture.data()).unwrap();
        assert_eq!(
            std::fs::canonicalize(tmpdir).ok(),
            Some(data),
            "TMPDIR is {tmpdir}"
        );
    }
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

/// The sidecar may write its data directory and nowhere else (#573): not the
/// directory beside it that holds the other apps' data, not another app's,
/// not a temporary directory, not where its program is. On Windows its own
/// temporary directory is the AppContainer's profile folder, which the backend
/// denies it.
#[tokio::test]
#[ignore = "needs this OS's sandbox; run by the sandbox-conformance CI job"]
async fn the_data_directory_is_the_only_path_the_sidecar_may_write() {
    let fixture = Fixture::new();
    let sidecar = sidecar(&fixture).await;
    let written = fixture.data().join("cache.db");
    let reply = call(
        &sidecar,
        "write_file",
        json!({"path": written, "text": "ok"}),
    )
    .await;
    assert!(
        matches!(reply, Reply::Ok(_)),
        "write in its data directory: {reply:?}"
    );

    // Its own temporary directory: on Linux and macOS its data directory,
    // where a temporary file is written and counted against the limit; on
    // Windows the AppContainer's folder, which it may not write.
    let Reply::Ok(theirs) = call(&sidecar, "temp_dir", json!({})).await else {
        panic!("temp_dir did not answer");
    };
    let theirs = PathBuf::from(theirs["path"].as_str().expect("a path"));
    let canonical_data = std::fs::canonicalize(fixture.data()).unwrap();
    if std::fs::canonicalize(&theirs).is_ok_and(|t| t.starts_with(&canonical_data)) {
        let temporary = theirs.join("scratch");
        let reply = call(
            &sidecar,
            "write_file",
            json!({"path": temporary, "text": "ok"}),
        )
        .await;
        assert!(
            matches!(reply, Reply::Ok(_)),
            "write in its temporary directory: {reply:?}"
        );
    }
    let mut places: Vec<(&str, PathBuf)> = vec![
        ("the data root, beside its directory", fixture.path("apps")),
        (
            "another app's data directory",
            fixture.other.path().to_owned(),
        ),
        ("a directory outside the grant", fixture.path("outside")),
        ("the directory holding its program", fixture.path("bin")),
        ("the host's temporary directory", std::env::temp_dir()),
    ];
    if !std::fs::canonicalize(&theirs).is_ok_and(|t| t.starts_with(&canonical_data)) {
        places.push(("its own temporary directory", theirs.clone()));
    }
    if cfg!(unix) {
        places.push(("/tmp", PathBuf::from("/tmp")));
        places.push(("/var/tmp", PathBuf::from("/var/tmp")));
    }
    if cfg!(target_os = "linux") {
        places.push(("/dev/shm", PathBuf::from("/dev/shm")));
    }
    for (n, (what, dir)) in places.into_iter().enumerate() {
        // What Windows has on the folder, for the log: which entry lets a
        // write through, if one does.
        #[cfg(windows)]
        if what == "its own temporary directory" {
            eprintln!("{what}: {}", acl(&dir));
        }
        let name = format!("srelens-conformance-{}-{n}", std::process::id());
        let control = host_writes(&dir.join(format!("{name}-control")));
        let planted = dir.join(&name);
        let params = json!({"path": planted, "text": "planted"});
        assert_denied(
            &sidecar,
            &format!("write in {what} ({})", dir.display()),
            control,
            "write_file",
            params,
            Denial::File,
        )
        .await;
        let leaked = planted.exists();
        let _ = std::fs::remove_file(&planted);
        assert!(!leaked, "{what}: the file was written");
    }
}

/// A link made inside the data directory reaches nothing outside it: a hard
/// link to the kubeconfig cannot be made, and a symbolic link, where it can be
/// made at all, cannot be read through. The sandbox resolves the path; the
/// supervisor's measuring and clearing never follow one (`sidecar::data`).
#[tokio::test]
#[ignore = "needs this OS's sandbox; run by the sandbox-conformance CI job"]
async fn a_link_in_the_data_directory_reaches_nothing_outside_it() {
    let fixture = Fixture::new();
    let sidecar = sidecar(&fixture).await;
    let control_link = fixture.path("outside").join("control-link");
    let control = std::fs::hard_link(fixture.kubeconfig(), &control_link)
        .map_err(|e| e.to_string())
        .and_then(|()| std::fs::remove_file(&control_link).map_err(|e| e.to_string()));
    let hard = fixture.data().join("kubeconfig-hard");
    let params = json!({"from": fixture.kubeconfig(), "to": hard});
    assert_denied(
        &sidecar,
        "hard-link the kubeconfig into the data directory",
        control,
        "hard_link",
        params,
        Denial::Link,
    )
    .await;
    assert!(!hard.exists());

    let soft = fixture.data().join("kubeconfig-soft");
    let made = call(
        &sidecar,
        "symlink",
        json!({"target": fixture.kubeconfig(), "link": soft}),
    )
    .await;
    eprintln!("symlink to the kubeconfig: {made:?}");
    match made {
        // Made: reading through it is what must fail.
        Reply::Ok(_) => {
            assert!(std::fs::symlink_metadata(&soft).is_ok_and(|m| m.file_type().is_symlink()));
            assert_denied(
                &sidecar,
                "read the kubeconfig through a symbolic link in the data directory",
                host_reads(&fixture.kubeconfig()),
                "read_file",
                json!({"path": soft}),
                Denial::File,
            )
            .await;
        }
        // Not made, as on Windows, where it takes a privilege an AppContainer
        // lacks (ERROR_PRIVILEGE_NOT_HELD, 1314): then there is nothing to
        // read through, and the refusal must be that one.
        Reply::Refused(failure) => {
            let privilege = cfg!(windows) && failure.os == Some(1314);
            assert!(
                privilege || Denial::File.accepts(&failure),
                "the link was refused, but not as a sandbox refuses: {failure:?}"
            );
            assert!(std::fs::symlink_metadata(&soft).is_err());
        }
        Reply::Stopped(why) => panic!("the sidecar stopped: {why}"),
    }
}

/// Nor may it change what it cannot write: the mode of a file outside the data
/// directory, which would let it make a kubeconfig readable to every user on
/// the machine, or unreadable to srelens. Landlock has no right for this, so
/// on Linux the seccomp filter refuses changing a mode, owner or extended
/// attribute by path, everywhere, and a sidecar sets one on a file it has
/// open instead.
#[tokio::test]
#[ignore = "needs this OS's sandbox; run by the sandbox-conformance CI job"]
async fn a_file_outside_the_data_directory_cannot_have_its_mode_changed() {
    let fixture = Fixture::new();
    let sidecar = sidecar(&fixture).await;
    let target = fixture.kubeconfig();
    let before = std::fs::metadata(&target).unwrap().permissions();
    let control = fixture.path("outside").join("control-mode");
    let host_control = std::fs::write(&control, "x")
        .and_then(|()| {
            let mut p = std::fs::metadata(&control)?.permissions();
            p.set_readonly(true);
            std::fs::set_permissions(&control, p)
        })
        .map_err(|e| e.to_string());
    let params = json!({"path": target, "readonly": !before.readonly()});
    assert_denied(
        &sidecar,
        "change the kubeconfig's mode",
        host_control,
        "set_readonly",
        params,
        Denial::File,
    )
    .await;
    let after = std::fs::metadata(&target).unwrap().permissions();
    assert_eq!(after, before, "the kubeconfig's mode changed");
    // Within its own directory it may, through the file it has open.
    let own = fixture.data().join("cache.db");
    std::fs::write(&own, "x").unwrap();
    // On Linux, not by path even there: the filter cannot tell the data
    // directory's paths from any other, so it refuses the call itself.
    #[cfg(target_os = "linux")]
    match call(&sidecar, "set_readonly", json!({"path": own})).await {
        Reply::Refused(failure) if failure.os == Some(libc::EPERM as i64) => {}
        other => {
            panic!("chmod by path in its data directory was not refused by the filter: {other:?}")
        }
    }
    let reply = call(&sidecar, "set_readonly", json!({"path": own, "by": "file"})).await;
    assert!(
        matches!(reply, Reply::Ok(_)),
        "mode in its data directory: {reply:?}"
    );
    assert!(std::fs::metadata(&own).unwrap().permissions().readonly());
}

/// The data directory's size limit (#573). On Linux and macOS the kernel
/// refuses a file past it (`EFBIG`, from the launcher's `RLIMIT_FSIZE`) and the
/// sidecar lives on; everywhere, a directory that grows past it in several
/// files gets the sidecar stopped, and it stays stopped.
#[tokio::test]
#[ignore = "needs this OS's sandbox; run by the sandbox-conformance CI job"]
async fn the_data_directory_size_limit_holds() {
    let fixture = Fixture::new();
    let limit = 8u64 << 20;
    let sidecar = sidecar_with(
        &fixture,
        Limits {
            data_bytes: limit,
            ..limits()
        },
    )
    .await;
    let one = fixture.data().join("one.bin");
    let reply = call(
        &sidecar,
        "write_bytes",
        json!({"path": one, "bytes": 2 * limit}),
    )
    .await;
    eprintln!("write one file of twice the limit: {reply:?}");
    #[cfg(unix)]
    {
        match &reply {
            Reply::Refused(failure) => assert_eq!(
                failure.os,
                Some(libc::EFBIG as i64),
                "refused, but not by the file size limit: {failure:?}"
            ),
            other => panic!("a file past the limit was written: {other:?}"),
        }
        let held = std::fs::metadata(&one).map(|m| m.len()).unwrap_or(0);
        assert!(
            held <= limit,
            "the file holds {held} bytes, over the {limit}"
        );
        // It lived through the refusal: SIGXFSZ is ignored.
        assert!(matches!(
            call(&sidecar, "ping", json!({})).await,
            Reply::Ok(_)
        ));
    }
    // Under the per-file limit each, over the directory's between them.
    for name in ["two.bin", "three.bin"] {
        let path = fixture.data().join(name);
        let _ = call(
            &sidecar,
            "write_bytes",
            json!({"path": path, "bytes": limit / 2}),
        )
        .await;
    }
    let mut status = sidecar.watch();
    let stopped = tokio::time::timeout(
        Duration::from_secs(30),
        status.wait_for(|s| matches!(s, SidecarStatus::Refused { .. })),
    )
    .await
    .expect("the sidecar was stopped within 30 s")
    .expect("the supervisor is running")
    .clone();
    let SidecarStatus::Refused { reason } = stopped else {
        unreachable!()
    };
    eprintln!("stopped: {reason}");
    assert!(reason.contains("over its 8 MiB limit"), "{reason}");
}

/// The Inspector's memory reading (#575, #753): the backend measures a running
/// sidecar's memory, within its limit and growing when it holds more, and
/// reports none once it has stopped. Linux reads the cgroup's `memory.current`;
/// Windows reads the process's committed private memory, which is what the
/// Job Object's limit caps. macOS reads the process's physical footprint, in
/// the watchdog's samples (#713).
#[tokio::test]
#[ignore = "needs this OS's sandbox; run by the sandbox-conformance CI job"]
async fn the_sidecars_memory_is_measured_while_it_runs() {
    let fixture = Fixture::new();
    let sidecar = sidecar(&fixture).await;
    let limit = limits().memory_bytes;
    let before = memory_reading(&sidecar, |n| n > 0).await;
    assert!(
        matches!(before, Some(n) if n > 0 && n <= limit),
        "while it runs, within its {limit}-byte limit: {before:?}"
    );
    let held = 32u64 << 20;
    match call(&sidecar, "hold", json!({"mib": held >> 20})).await {
        Reply::Ok(_) => {}
        other => panic!("the probe could not hold {held} bytes: {other:?}"),
    }
    let after = memory_reading(&sidecar, |n| matches!(before, Some(b) if n >= b + held / 2)).await;
    assert!(
        matches!((before, after), (Some(b), Some(a)) if a >= b + held / 2 && a <= limit),
        "holding {held} bytes more: {before:?} then {after:?}"
    );
    sidecar.stop().await;
    assert_eq!(sidecar.metrics().memory_bytes, None, "once it has stopped");
}
