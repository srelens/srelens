//! The macOS watchdog (#713) on a real process: `sandbox::watch`, what the
//! macOS backend puts its sandboxed sidecar under, here on the probe without
//! Seatbelt. The watchdog does not depend on Seatbelt, so this runs on
//! GitHub's macOS runners, which are older than macOS 27, the one version the
//! Seatbelt profile was checked on. The `macos-watchdog` CI job runs it; with
//! Seatbelt, the conformance suite's checks 5 and 6 are run by hand on a
//! macOS 27 Mac, release-built (that file's header says how).
#![cfg(target_os = "macos")]

use serde_json::{json, Value};
use srelens_plugin_host::sidecar::sandbox::watch;
use srelens_plugin_host::sidecar::{Exit, Launched, Limits, MemoryProbe, Process};
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, Lines};

const PROBE: &str = env!("CARGO_BIN_EXE_srelens-sidecar-probe");
const MIB: u64 = 1024 * 1024;

/// The conformance suite's limits.
fn limits() -> Limits {
    Limits {
        memory_bytes: 128 * MIB,
        cpus: 0.25,
        ..Limits::default()
    }
}

fn spawn_probe() -> tokio::process::Child {
    tokio::process::Command::new(PROBE)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("the probe starts")
}

/// A launched probe, and a way to call it.
struct Probe {
    stdin: Box<dyn AsyncWrite + Send + Unpin>,
    lines: Lines<BufReader<Box<dyn AsyncRead + Send + Unpin>>>,
    process: Process,
    next: u64,
}

impl Probe {
    fn new(launched: Launched) -> Probe {
        Probe {
            stdin: launched.stdin,
            lines: BufReader::new(launched.stdout).lines(),
            process: launched.process,
            next: 1,
        }
    }

    /// One request's result; `None` when the probe ended before answering.
    /// A probe that does neither within 30 s fails the test, rather than
    /// hanging it.
    async fn call(&mut self, method: &str, params: Value) -> Option<Value> {
        let id = self.next;
        self.next += 1;
        let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let exchange = async {
            self.stdin
                .write_all(format!("{request}\n").as_bytes())
                .await
                .ok()?;
            let line = self.lines.next_line().await.ok()??;
            let answer: Value = serde_json::from_str(&line).expect("a JSON-RPC answer");
            Some(answer["result"].clone())
        };
        tokio::time::timeout(Duration::from_secs(30), exchange)
            .await
            .unwrap_or_else(|_| {
                panic!("{method}: the probe neither answered nor ended within 30 s")
            })
    }

    async fn exit(&mut self) -> Exit {
        tokio::time::timeout(Duration::from_secs(5), self.process.exit())
            .await
            .expect("it ended within 5 s")
    }
}

/// The reader's value once `ok` holds, or its last after 2 s: it is the
/// watchdog's last reading, up to 50 ms old.
async fn reading(memory: &MemoryProbe, ok: impl Fn(u64) -> bool) -> Option<u64> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let now = memory();
        if now.is_some_and(&ok) || Instant::now() >= deadline {
            return now;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn memory_held_past_the_limit_stops_it_at_the_limit() {
    let mut probe = Probe::new(watch(spawn_probe(), &limits()).unwrap());
    let started = Instant::now();
    // Held, not only allocated: the watchdog bounds sustained use, and memory
    // freed again before the next reading can pass unseen.
    let _ = probe.call("hold", json!({"mib": 512})).await;
    let exit = probe.exit().await;
    // The measured figure is how far it overshot. `cargo test` builds the
    // probe unoptimised, which allocates slowly, so this understates it; the
    // figure the issue asks to record is a release-built probe's.
    eprintln!(
        "hold 512 MiB against 128 MiB: {} after {:?}",
        exit.description,
        started.elapsed()
    );
    assert!(exit.memory_limit, "{exit:?}");
    assert!(
        exit.description
            .starts_with("was stopped at its 128 MiB memory limit (srelens measured "),
        "{}",
        exit.description
    );
}

#[tokio::test]
async fn cpu_is_throttled_to_the_limit() {
    let burn = json!({"millis": 3000, "threads": 2});
    let cpus = |v: &Value| v["cpu_ms"].as_f64().unwrap() / v["wall_ms"].as_f64().unwrap();
    // The control: the same burn, unwatched.
    let mut control = Probe::new(Launched::from_child(spawn_probe(), Exit::from_status).unwrap());
    let used = control
        .call("burn_cpu", burn.clone())
        .await
        .expect("the control burned");
    assert!(
        cpus(&used) > 0.375,
        "INCONCLUSIVE: only {:.2} CPUs unwatched",
        cpus(&used)
    );
    let mut probe = Probe::new(watch(spawn_probe(), &limits()).unwrap());
    let used = probe
        .call("burn_cpu", burn)
        .await
        .expect("the burn completed");
    eprintln!("used {:.2} CPUs against 0.25", cpus(&used));
    assert!(
        cpus(&used) <= 0.375,
        "{:.2} CPUs is over 1.5 times the 0.25 limit",
        cpus(&used)
    );
}

#[tokio::test]
async fn memory_is_read_while_it_runs_and_not_after_it_exits() {
    let limit = limits().memory_bytes;
    let mut probe = Probe::new(watch(spawn_probe(), &limits()).unwrap());
    let memory = probe.process.memory().expect("the watchdog reads memory");
    let before = reading(&memory, |n| n > 0).await;
    assert!(
        matches!(before, Some(n) if n > 0 && n <= limit),
        "while it runs, within its limit: {before:?}"
    );
    let held = 32 * MIB;
    probe
        .call("hold", json!({"mib": held / MIB}))
        .await
        .expect("held");
    let after = reading(&memory, |n| matches!(before, Some(b) if n >= b + held / 2)).await;
    assert!(
        matches!((before, after), (Some(b), Some(a)) if a >= b + held / 2 && a <= limit),
        "holding {held} bytes more: {before:?} then {after:?}"
    );
    (probe.process.killer())();
    probe.exit().await;
    assert_eq!(memory(), None, "once it has exited");
}
