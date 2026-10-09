//! Test support, not shipped: a sidecar that speaks the #572 protocol, and
//! performs one ordinary operation per request so the sandbox conformance
//! suite can check what its backend denies. The #571 spike's probe
//! (`spikes/sidecar-sandbox/src/bin/probe.rs`) with the lifecycle methods
//! added, and ways to misbehave for the supervisor's crash tests.
//!
//! It never returns file contents, only sizes, so pointing it at a real
//! kubeconfig reveals nothing but whether the open succeeded. It makes no
//! attempt to evade a sandbox.
//!
//! `PROBE_ON_START` makes it fail at once: `abort`, or `exit:N`.
//! `PROBE_API_VERSION` is the version it answers `initialize` with; `none`
//! answers that it speaks none of those offered.

use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--child") {
        // Started by `spawn_child`: exit at once.
        return;
    }
    match std::env::var("PROBE_ON_START").as_deref() {
        Ok("abort") => std::process::abort(),
        Ok(other) if other.starts_with("exit:") => {
            std::process::exit(other[5..].parse().unwrap_or(1));
        }
        _ => {}
    }
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            break;
        };
        // Notifications (a cancellation, a stream cancellation) need nothing:
        // every operation here finishes before the next line is read.
        let Some(id) = message.get("id").cloned() else {
            continue;
        };
        let method = message["method"].as_str().unwrap_or("");
        let reply = match handle(method, &message["params"]) {
            Ok(Action::Answer(result)) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Ok(Action::Hang) => loop {
                std::thread::sleep(Duration::from_secs(3600));
            },
            Ok(Action::AnswerThenExit(result)) => {
                let reply = json!({"jsonrpc": "2.0", "id": id, "result": result});
                let _ = writeln!(stdout, "{reply}").and_then(|()| stdout.flush());
                std::process::exit(0);
            }
            Err(Fail {
                message,
                kind,
                os,
                code,
            }) => json!({
                "jsonrpc": "2.0", "id": id,
                "error": {"code": code, "message": message, "data": {"kind": kind, "os": os}}
            }),
        };
        if writeln!(stdout, "{reply}")
            .and_then(|()| stdout.flush())
            .is_err()
        {
            break;
        }
    }
}

enum Action {
    Answer(Value),
    AnswerThenExit(Value),
    Hang,
}

/// A failed operation: the message, the `std::io::ErrorKind` name (or a kind
/// the probe names itself: `ResolverFailed`, `OutOfMemory`, `InvalidInput`),
/// and the raw OS code.
struct Fail {
    message: String,
    kind: String,
    os: Option<i32>,
    code: i64,
}

impl Fail {
    fn new(kind: &str, message: impl Into<String>) -> Fail {
        Fail {
            message: message.into(),
            kind: kind.into(),
            os: None,
            code: -32000,
        }
    }
}

impl From<std::io::Error> for Fail {
    fn from(e: std::io::Error) -> Fail {
        Fail {
            message: e.to_string(),
            kind: format!("{:?}", e.kind()),
            os: e.raw_os_error(),
            code: -32000,
        }
    }
}

impl From<String> for Fail {
    fn from(message: String) -> Fail {
        Fail::new("InvalidInput", message)
    }
}

fn answer(value: Value) -> Result<Action, Fail> {
    Ok(Action::Answer(value))
}

fn handle(method: &str, params: &Value) -> Result<Action, Fail> {
    match method {
        "initialize" => {
            let version = match std::env::var("PROBE_API_VERSION") {
                Ok(v) if v == "none" => {
                    return Err(Fail {
                        code: -32001,
                        ..Fail::new("Unsupported", "no common sidecar API version")
                    })
                }
                Ok(v) => v,
                Err(_) => params["apiVersions"][0]
                    .as_str()
                    .unwrap_or("0.0.0")
                    .to_owned(),
            };
            answer(json!({"apiVersion": version, "sidecar": {"name": "srelens-sidecar-probe"}}))
        }
        "activate" | "deactivate" | "health" => answer(json!({})),
        "shutdown" => Ok(Action::AnswerThenExit(json!({}))),
        "ping" => answer(json!({"pid": std::process::id()})),
        "echo" => answer(params.clone()),
        "env" => {
            let vars: Vec<(String, String)> = std::env::vars_os()
                .map(|(k, v)| {
                    (
                        k.to_string_lossy().into_owned(),
                        v.to_string_lossy().into_owned(),
                    )
                })
                .collect();
            let names: Vec<&String> = vars.iter().map(|(k, _)| k).collect();
            let values: serde_json::Map<String, Value> =
                vars.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
            answer(json!({"names": names, "values": values}))
        }
        "log" => {
            eprintln!("{}", str_param(params, "text")?);
            answer(json!({}))
        }
        "abort" => std::process::abort(),
        "exit" => std::process::exit(params["code"].as_i64().unwrap_or(1) as i32),
        "hang" => Ok(Action::Hang),
        "read_file" => {
            let bytes = std::fs::read(str_param(params, "path")?)?;
            answer(json!({"bytes": bytes.len()}))
        }
        "write_file" => {
            std::fs::write(str_param(params, "path")?, str_param(params, "text")?)?;
            answer(json!({}))
        }
        "write_bytes" => {
            // `bytes` zeros, a MiB at a time, for the data directory's limit.
            let len = params["bytes"].as_u64().ok_or("missing bytes".to_owned())?;
            let mut file = std::fs::File::create(str_param(params, "path")?)?;
            let chunk = vec![0u8; 1 << 20];
            let mut left = len;
            while left > 0 {
                let n = left.min(chunk.len() as u64) as usize;
                file.write_all(&chunk[..n])?;
                left -= n as u64;
            }
            answer(json!({"bytes": len}))
        }
        "temp_dir" => answer(json!({"path": std::env::temp_dir()})),
        "set_readonly" => {
            // A metadata write: the mode on Linux and macOS, the read-only
            // attribute on Windows. Nothing of the file's contents. By path
            // (`chmod`), or with `by: "file"` through the open file (`fchmod`).
            let path = str_param(params, "path")?;
            let mut permissions = std::fs::metadata(path)?.permissions();
            permissions.set_readonly(params["readonly"].as_bool().unwrap_or(true));
            if params["by"] == "file" {
                let file = std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(path)?;
                file.set_permissions(permissions)?;
            } else {
                std::fs::set_permissions(path, permissions)?;
            }
            answer(json!({}))
        }
        "hard_link" => {
            std::fs::hard_link(str_param(params, "from")?, str_param(params, "to")?)?;
            answer(json!({}))
        }
        "symlink" => {
            let (target, link) = (str_param(params, "target")?, str_param(params, "link")?);
            #[cfg(unix)]
            std::os::unix::fs::symlink(target, link)?;
            #[cfg(windows)]
            std::os::windows::fs::symlink_file(target, link)?;
            answer(json!({}))
        }
        "tcp_connect" => {
            let addr: SocketAddr = str_param(params, "addr")?
                .parse()
                .map_err(|e| format!("bad addr: {e}"))?;
            let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5))?;
            answer(json!({"local": stream.local_addr().map(|a| a.to_string()).ok()}))
        }
        #[cfg(unix)]
        "unix_connect" => {
            std::os::unix::net::UnixStream::connect(str_param(params, "path")?)?;
            answer(json!({}))
        }
        "resolve" => {
            let host = str_param(params, "host")?;
            // Resolver errors carry no distinctive ErrorKind, so name it here.
            let addrs: Vec<String> = (host, 443)
                .to_socket_addrs()
                .map_err(|e| Fail {
                    kind: "ResolverFailed".into(),
                    ..Fail::from(e)
                })?
                .map(|a| a.ip().to_string())
                .collect();
            if addrs.is_empty() {
                return Err(Fail::new(
                    "ResolverFailed",
                    format!("{host} resolved to no addresses"),
                ));
            }
            answer(json!({"addrs": addrs}))
        }
        "spawn_child" => {
            // The child inherits stdio and exits at once without touching it.
            // Not `Stdio::null()`: opening /dev/null is a filesystem access,
            // and a sandbox that refuses it would look as if it had refused
            // the process.
            let exe = std::env::current_exe()?;
            let status = std::process::Command::new(exe).arg("--child").status()?;
            answer(json!({"status": status.to_string()}))
        }
        "allocate" => {
            let mib = params["mib"].as_u64().ok_or("missing mib".to_owned())?;
            // Checked: a wrapped length would be a small allocation reported
            // as a large one, a false pass for the memory check.
            let len = usize::try_from(mib)
                .ok()
                .and_then(|mib| mib.checked_mul(1024 * 1024))
                .ok_or_else(|| Fail::new("InvalidInput", format!("{mib} MiB is too large")))?;
            let mut buf: Vec<u8> = Vec::new();
            buf.try_reserve_exact(len).map_err(|e| {
                Fail::new(
                    "OutOfMemory",
                    format!("allocation of {mib} MiB refused: {e}"),
                )
            })?;
            // Touch every page so it is really committed, not just reserved.
            buf.resize(len, 0xA5);
            let sum = buf.iter().step_by(4096).map(|b| *b as u64).sum::<u64>();
            answer(json!({"mib": mib, "checksum": sum}))
        }
        "hold" => {
            // `mib` MiB, touched and kept for the rest of the probe's life, so
            // the Inspector's memory reading has something to show (#753).
            let mib = params["mib"].as_u64().ok_or("missing mib".to_owned())?;
            let len = usize::try_from(mib)
                .ok()
                .and_then(|mib| mib.checked_mul(1024 * 1024))
                .ok_or_else(|| Fail::new("InvalidInput", format!("{mib} MiB is too large")))?;
            let mut buf: Vec<u8> = Vec::new();
            buf.try_reserve_exact(len).map_err(|e| {
                Fail::new(
                    "OutOfMemory",
                    format!("allocation of {mib} MiB refused: {e}"),
                )
            })?;
            buf.resize(len, 0x5A);
            buf.leak();
            answer(json!({"mib": mib}))
        }
        "burn_cpu" => {
            let millis = params["millis"]
                .as_u64()
                .ok_or("missing millis".to_owned())?;
            let threads = params["threads"].as_u64().unwrap_or(1);
            let cpu_before = process_cpu_ms()?;
            let start = Instant::now();
            let deadline = start + Duration::from_millis(millis);
            let workers: Vec<_> = (0..threads)
                .map(|_| {
                    std::thread::spawn(move || {
                        let mut x = 0u64;
                        while Instant::now() < deadline {
                            for i in 0..10_000u64 {
                                x = std::hint::black_box(x.wrapping_mul(31).wrapping_add(i));
                            }
                        }
                        x
                    })
                })
                .collect();
            for worker in workers {
                worker
                    .join()
                    .map_err(|_| Fail::new("Other", "worker panicked"))?;
            }
            let wall_ms = start.elapsed().as_secs_f64() * 1000.0;
            let cpu_ms = process_cpu_ms()? - cpu_before;
            answer(json!({"cpu_ms": cpu_ms, "wall_ms": wall_ms, "threads": threads}))
        }
        other => Err(Fail {
            code: -32601,
            ..Fail::new("InvalidInput", format!("unknown method {other}"))
        }),
    }
}

fn str_param<'a>(params: &'a Value, key: &str) -> Result<&'a str, Fail> {
    params[key]
        .as_str()
        .ok_or_else(|| Fail::new("InvalidInput", format!("missing {key}")))
}

/// User plus kernel CPU time of this whole process, in milliseconds.
#[cfg(windows)]
fn process_cpu_ms() -> Result<f64, Fail> {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    let zero = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut c, mut e, mut k, mut u) = (zero, zero, zero, zero);
    // SAFETY: GetCurrentProcess is a pseudo-handle; the out-pointers are valid.
    let ok = unsafe { GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) };
    let t = |f: FILETIME| ((f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64) as f64;
    cpu_clock(ok != 0, "GetProcessTimes", (t(k) + t(u)) / 10_000.0)
}

#[cfg(unix)]
fn process_cpu_ms() -> Result<f64, Fail> {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: a valid out-pointer.
    let ret = unsafe { libc::clock_gettime(libc::CLOCK_PROCESS_CPUTIME_ID, &mut ts) };
    cpu_clock(
        ret == 0,
        "clock_gettime",
        ts.tv_sec as f64 * 1000.0 + ts.tv_nsec as f64 / 1e6,
    )
}

/// A CPU-time reading, or the error of the call that failed to take it. A
/// zero must not be reported as a measurement: the CPU check would read it
/// as throttling.
fn cpu_clock(succeeded: bool, call: &str, ms: f64) -> Result<f64, Fail> {
    if succeeded {
        return Ok(ms);
    }
    let e = std::io::Error::last_os_error();
    Err(Fail {
        message: format!("{call} failed: {e}"),
        ..Fail::from(e)
    })
}
