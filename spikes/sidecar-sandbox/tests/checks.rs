//! The seven checks from #571, asserted against whichever backend `SPIKE_BACKEND` names.
//!
//! Every check first proves the sidecar is alive (a `ping` round trip), so a backend that
//! stops the sidecar from starting at all cannot pass a "must be denied" check by accident:
//! a denial has to be the probe answering that the operation failed.
//!
//! Run with `SPIKE_BACKEND=none` to see the baseline: every "must be denied" check fails
//! because the operation succeeds. See `src/lib.rs` for the backends.

use serde_json::json;
use sidecar_sandbox_spike::{Backend, Fixture, Limits, Reply, Sidecar};

const LIMITS: Limits = Limits::SPIKE;

struct Run {
    backend: Backend,
    fixture: Fixture,
    sidecar: Sidecar,
}

fn start() -> Run {
    let backend = Backend::from_env().expect("SPIKE_BACKEND names a backend this OS supports");
    let fixture = Fixture::new().expect("fixture directories are created");
    let mut sidecar = Sidecar::launch(backend, &fixture, &LIMITS).expect("the sidecar starts");
    let pong = sidecar.call("ping", json!({}));
    assert!(
        matches!(pong, Reply::Ok(_)),
        "[{backend}] the sidecar must be alive before any check; ping got {pong:?}"
    );
    Run { backend, fixture, sidecar }
}

/// The probe answered, and the answer is that the operation failed.
#[track_caller]
fn assert_refused(run: &Run, check: &str, reply: &Reply) {
    println!("[{}] {check}: {reply:?}", run.backend);
    assert!(
        matches!(reply, Reply::Refused(_)),
        "[{}] {check} must be denied, got {reply:?}",
        run.backend
    );
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
    let reply = run.sidecar.call("read_file", json!({ "path": path }));
    assert_refused(&run, "read ~/.kube/config", &reply);
}

#[test]
fn c1_read_outside_scratch_is_denied() {
    let mut run = start();
    let path = run.fixture.outside_file();
    let reply = run.sidecar.call("read_file", json!({ "path": path }));
    assert_refused(&run, "read a file outside the scratch directory", &reply);
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
    let path = run.fixture.outside_dir().join("written-by-sidecar.txt");
    let reply = run.sidecar.call("write_file", json!({ "path": path, "text": "hello" }));
    assert_refused(&run, "write outside the scratch directory", &reply);
    assert!(!path.exists(), "[{}] nothing may be left outside the scratch directory", run.backend);
}

// 3. Outbound TCP and DNS resolution are denied.

#[test]
fn c3_tcp_connect_to_host_loopback_is_denied() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("host listener binds");
    let addr = listener.local_addr().expect("listener address").to_string();
    let mut run = start();
    let reply = run.sidecar.call("tcp_connect", json!({ "addr": addr }));
    assert_refused(&run, &format!("TCP connect to the host's listener {addr}"), &reply);
}

#[test]
fn c3_tcp_connect_to_internet_is_denied() {
    let mut run = start();
    let reply = run.sidecar.call("tcp_connect", json!({ "addr": "1.1.1.1:443" }));
    assert_refused(&run, "TCP connect to 1.1.1.1:443", &reply);
}

#[test]
fn c3_dns_resolution_is_denied() {
    let mut run = start();
    let reply = run.sidecar.call("resolve", json!({ "host": "example.com" }));
    assert_refused(&run, "resolve example.com", &reply);
}

// 4. Starting a child process is denied.

#[test]
fn c4_spawn_child_process_is_denied() {
    let mut run = start();
    let reply = run.sidecar.call("spawn_child", json!({}));
    assert_refused(&run, "start a child process", &reply);
}

// 5. Allocating past the memory limit stops or refuses the sidecar; the host keeps running.

#[test]
fn c5_memory_past_limit_is_refused_or_stopped_and_host_survives() {
    let mut run = start();
    let mib = LIMITS.memory_mib * 4;
    let reply = run.sidecar.call("allocate", json!({ "mib": mib }));
    println!("[{}] allocate {mib} MiB (limit {} MiB): {reply:?}", run.backend, LIMITS.memory_mib);
    assert!(
        matches!(reply, Reply::Refused(_) | Reply::Stopped(_)),
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
        Reply::Refused(e) | Reply::Garbled(e) => {
            panic!("[{}] burn_cpu failed unexpectedly: {e}", run.backend)
        }
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
