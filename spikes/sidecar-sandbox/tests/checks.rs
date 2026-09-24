//! The seven checks from #571, asserted against whichever backend `SPIKE_BACKEND` names.
//!
//! Three rules keep a "must be denied" check from passing by accident:
//!
//! 1. The sidecar is alive first (a `ping` round trip), so a backend that stops it from
//!    starting cannot pass. A denial has to be the probe answering that the operation
//!    failed, never a crash or a garbled reply.
//! 2. A host-side **positive control** runs first: the host itself reads the same file,
//!    writes to the same directory, connects to the same address, resolves the same name
//!    or starts the same program. If the control fails, the environment cannot do the
//!    operation at all, so the sidecar failing it says nothing about the sandbox: the
//!    check fails as INCONCLUSIVE, never passes.
//! 3. The probe's error must be of a kind a sandbox produces for that operation
//!    ([`Denial::accepts`]), not any error at all (a bad argument, say).
//!
//! Run with `SPIKE_BACKEND=none` to see the baseline: every "must be denied" check fails
//! because the operation succeeds. See `src/lib.rs` for the backends.

use serde_json::json;
use sidecar_sandbox_spike::{Backend, Denial, Fixture, Limits, Reply, Sidecar};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

const LIMITS: Limits = Limits::SPIKE;
const INTERNET: &str = "1.1.1.1:443";
const NAME: &str = "example.com";

struct Run {
    backend: Backend,
    fixture: Fixture,
    sidecar: Sidecar,
}

fn start() -> Run {
    let backend = Backend::from_env().unwrap_or_else(|e| panic!("{e}"));
    let fixture = Fixture::new().expect("fixture directories are created");
    let mut sidecar = Sidecar::launch(backend, &fixture, &LIMITS)
        .unwrap_or_else(|e| panic!("[{backend}] the sidecar could not start: {e}"));
    let pong = sidecar.call("ping", json!({}));
    assert!(
        matches!(pong, Reply::Ok(_)),
        "[{backend}] the sidecar must be alive before any check; ping got {pong:?}"
    );
    Run { backend, fixture, sidecar }
}

/// The host-side positive control for a check. A failure makes the check inconclusive.
#[track_caller]
fn control<T, E: std::fmt::Display>(check: &str, what: &str, result: Result<T, E>) -> T {
    result.unwrap_or_else(|e| {
        panic!("INCONCLUSIVE: {check}: the host itself could not {what} ({e}), so a failure inside the sandbox would say nothing about the sandbox")
    })
}

/// The probe answered that the operation failed, with an error a sandbox produces for it.
#[track_caller]
fn assert_denied(run: &Run, check: &str, denial: Denial, reply: &Reply) {
    println!("[{}] {check}: {reply:?}", run.backend);
    match reply {
        Reply::Refused(failure) => assert!(
            denial.accepts(failure),
            "[{}] {check} failed, but with {failure:?}, which is not a {denial:?} denial",
            run.backend
        ),
        other => panic!("[{}] {check} must be denied, got {other:?}", run.backend),
    }
}

#[track_caller]
fn assert_allowed(run: &Run, check: &str, reply: &Reply) {
    println!("[{}] {check}: {reply:?}", run.backend);
    assert!(
        matches!(reply, Reply::Ok(_)),
        "[{}] {check} must be allowed, got {reply:?}",
        run.backend
    );
}

// 1. Reading the kubeconfig, or any file outside the granted scratch directory, is denied.

#[test]
fn c1_read_kubeconfig_is_denied() {
    let mut run = start();
    let path = run.fixture.kubeconfig();
    let check = "read ~/.kube/config";
    control(check, "read it", std::fs::read(&path));
    let reply = run.sidecar.call("read_file", json!({ "path": path }));
    assert_denied(&run, check, Denial::File, &reply);
}

#[test]
fn c1_read_outside_scratch_is_denied() {
    let mut run = start();
    let path = run.fixture.outside_file();
    let check = "read a file outside the scratch directory";
    control(check, "read it", std::fs::read(&path));
    let reply = run.sidecar.call("read_file", json!({ "path": path }));
    assert_denied(&run, check, Denial::File, &reply);
}

// 2. Writing inside the scratch directory is allowed; writing outside it is denied.

#[test]
fn c2_write_inside_scratch_is_allowed() {
    let mut run = start();
    let path = run.fixture.scratch().join("written-by-sidecar.txt");
    let write = run.sidecar.call("write_file", json!({ "path": path, "text": "hello" }));
    assert_allowed(&run, "write inside the scratch directory", &write);
    let read = run.sidecar.call("read_file", json!({ "path": path }));
    assert_allowed(&run, "read back inside the scratch directory", &read);
    assert_eq!(
        std::fs::read_to_string(&path).expect("the host can read what the sidecar wrote"),
        "hello"
    );
}

#[test]
fn c2_write_outside_scratch_is_denied() {
    let mut run = start();
    let check = "write outside the scratch directory";
    let probe_file = run.fixture.outside_dir().join("written-by-host.txt");
    control(check, "write there", std::fs::write(&probe_file, "control"));
    control(check, "clean up its own write", std::fs::remove_file(&probe_file));
    let path = run.fixture.outside_dir().join("written-by-sidecar.txt");
    let reply = run.sidecar.call("write_file", json!({ "path": path, "text": "hello" }));
    assert_denied(&run, check, Denial::File, &reply);
    assert!(!path.exists(), "[{}] nothing may be left outside the scratch directory", run.backend);
}

// 3. Outbound TCP and DNS resolution are denied.

fn host_connect(addr: &str) -> std::io::Result<TcpStream> {
    let addr: SocketAddr = addr.parse().expect("a literal socket address");
    TcpStream::connect_timeout(&addr, Duration::from_secs(5))
}

#[test]
fn c3_tcp_connect_to_host_loopback_is_denied() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("host listener binds");
    let addr = listener.local_addr().expect("listener address").to_string();
    let check = format!("TCP connect to the host's listener {addr}");
    control(&check, "connect to its own listener", host_connect(&addr));
    let mut run = start();
    let reply = run.sidecar.call("tcp_connect", json!({ "addr": addr }));
    assert_denied(&run, &check, Denial::Network, &reply);
}

#[test]
fn c3_tcp_connect_to_internet_is_denied() {
    let check = format!("TCP connect to {INTERNET}");
    control(&check, &format!("connect to {INTERNET} (no network?)"), host_connect(INTERNET));
    let mut run = start();
    let reply = run.sidecar.call("tcp_connect", json!({ "addr": INTERNET }));
    assert_denied(&run, &check, Denial::Network, &reply);
}

#[test]
fn c3_dns_resolution_is_denied() {
    let check = format!("resolve {NAME}");
    let resolved = (NAME, 443).to_socket_addrs().map(|a| a.count()).and_then(|n| {
        if n == 0 {
            Err(std::io::Error::other("no addresses"))
        } else {
            Ok(n)
        }
    });
    control(&check, &format!("resolve {NAME} (no DNS?)"), resolved);
    let mut run = start();
    let reply = run.sidecar.call("resolve", json!({ "host": NAME }));
    assert_denied(&run, &check, Denial::Dns, &reply);
}

// 4. Starting a child process is denied.

#[test]
fn c4_spawn_child_process_is_denied() {
    let mut run = start();
    let check = "start a child process";
    let ran = match std::process::Command::new(run.fixture.probe()).arg("--child").status() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(status.to_string()),
        Err(e) => Err(e.to_string()),
    };
    control(check, "start the same program", ran);
    let reply = run.sidecar.call("spawn_child", json!({}));
    assert_denied(&run, check, Denial::Process, &reply);
}

// 5. Allocating past the memory limit stops or refuses the sidecar; the host keeps running.

#[test]
fn c5_memory_past_limit_is_refused_or_stopped_and_host_survives() {
    let mut run = start();
    let mib = LIMITS.memory_mib * 4;
    let reply = run.sidecar.call("allocate", json!({ "mib": mib }));
    println!("[{}] allocate {mib} MiB (limit {} MiB): {reply:?}", run.backend, LIMITS.memory_mib);
    let refused = matches!(&reply, Reply::Refused(f) if Denial::Memory.accepts(f));
    assert!(
        refused || matches!(reply, Reply::Stopped(_)),
        "[{}] allocating {mib} MiB past a {} MiB limit must be refused or stop the sidecar, got {reply:?}",
        run.backend,
        LIMITS.memory_mib
    );
    // The host is this process. It is still here, and it can still supervise a sidecar.
    let mut again = Sidecar::launch(run.backend, &run.fixture, &LIMITS).expect("host relaunches");
    assert!(matches!(again.call("ping", json!({})), Reply::Ok(_)));
}

// 6. Using CPU past the limit is throttled or stopped.

#[test]
fn c6_cpu_past_limit_is_throttled_or_stopped() {
    let mut run = start();
    let reply = run.sidecar.call("burn_cpu", json!({ "millis": 3000, "threads": 2 }));
    println!("[{}] burn 2 threads for 3 s (limit {} CPU): {reply:?}", run.backend, LIMITS.cpus);
    match &reply {
        Reply::Stopped(_) => {}
        Reply::Ok(result) => {
            let cpu = result["cpu_ms"].as_f64().expect("cpu_ms");
            let wall = result["wall_ms"].as_f64().expect("wall_ms");
            let used = cpu / wall;
            println!("[{}] used {used:.2} CPUs", run.backend);
            assert!(
                used <= LIMITS.cpus * 1.5,
                "[{}] the sidecar used {used:.2} CPUs against a {} CPU limit",
                run.backend,
                LIMITS.cpus
            );
        }
        other => panic!("[{}] burn_cpu failed unexpectedly: {other:?}", run.backend),
    }
}

// 7. The JSON-RPC exchange over stdin/stdout with the host still works.

#[test]
fn c7_json_rpc_over_stdio_works() {
    let mut run = start();
    for n in 0..50 {
        let reply = run.sidecar.call("echo", json!({ "n": n }));
        match reply {
            Reply::Ok(v) => assert_eq!(v["n"], n, "[{}] echo {n}", run.backend),
            other => panic!("[{}] echo {n} failed: {other:?}", run.backend),
        }
    }
    println!("[{}] 50 JSON-RPC round trips over stdio", run.backend);
}
