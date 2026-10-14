//! The probe sidecar: newline-delimited JSON-RPC 2.0 on stdin/stdout.
//!
//! Each method performs one ordinary operation and reports whether it worked. The probe
//! never returns file contents, only sizes, so pointing it at a real kubeconfig reveals
//! nothing but whether the open succeeded. It makes no attempt to evade a sandbox.

use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--child") {
        // Started by `spawn_child`: exit at once.
        return;
    }
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(req) => {
                let id = req["id"].clone();
                match handle(req["method"].as_str().unwrap_or(""), &req["params"]) {
                    Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                    Err(Fail { message, kind, os }) => json!({
                        "jsonrpc": "2.0", "id": id,
                        "error": {
                            "code": -32000,
                            "message": message,
                            "data": { "kind": kind, "os": os }
                        }
                    }),
                }
            }
            Err(e) => json!({
                "jsonrpc": "2.0", "id": null,
                "error": { "code": -32700, "message": format!("parse error: {e}") }
            }),
        };
        if writeln!(stdout, "{reply}").and_then(|()| stdout.flush()).is_err() {
            break;
        }
    }
}

/// A failed operation: the message, the `std::io::ErrorKind` name (or a kind the probe
/// names itself: `ResolverFailed`, `OutOfMemory`, `InvalidInput`), and the raw OS code.
struct Fail {
    message: String,
    kind: String,
    os: Option<i32>,
}

impl Fail {
    fn new(kind: &str, message: impl Into<String>) -> Fail {
        Fail { message: message.into(), kind: kind.into(), os: None }
    }
}

impl From<std::io::Error> for Fail {
    fn from(e: std::io::Error) -> Fail {
        Fail { message: e.to_string(), kind: format!("{:?}", e.kind()), os: e.raw_os_error() }
    }
}

/// A bad request, never a sandbox's doing.
impl From<&str> for Fail {
    fn from(message: &str) -> Fail {
        Fail::new("InvalidInput", message)
    }
}

impl From<String> for Fail {
    fn from(message: String) -> Fail {
        Fail::new("InvalidInput", message)
    }
}

fn handle(method: &str, params: &Value) -> Result<Value, Fail> {
    match method {
        "ping" => Ok(json!({ "pid": std::process::id() })),
        "echo" => Ok(params.clone()),
        "read_file" => {
            let bytes = std::fs::read(str_param(params, "path")?)?;
            Ok(json!({ "bytes": bytes.len() }))
        }
        "write_file" => {
            std::fs::write(str_param(params, "path")?, str_param(params, "text")?)?;
            Ok(json!({}))
        }
        "tcp_connect" => {
            let addr: SocketAddr =
                str_param(params, "addr")?.parse().map_err(|e| format!("bad addr: {e}"))?;
            let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5))?;
            Ok(json!({ "local": stream.local_addr().map(|a| a.to_string()).ok() }))
        }
        "resolve" => {
            let host = str_param(params, "host")?;
            // Resolver errors carry no distinctive ErrorKind, so name the failure here.
            let addrs: Vec<String> = (host, 443)
                .to_socket_addrs()
                .map_err(|e| Fail { kind: "ResolverFailed".into(), ..Fail::from(e) })?
                .map(|a| a.ip().to_string())
                .collect();
            if addrs.is_empty() {
                return Err(Fail::new("ResolverFailed", format!("{host} resolved to no addresses")));
            }
            Ok(json!({ "addrs": addrs }))
        }
        "spawn_child" => {
            // The child inherits stdio and exits at once without touching it. Not
            // `Stdio::null()`: opening /dev/null is a filesystem access, and a sandbox that
            // refuses it would look as if it had refused the process.
            let exe = std::env::current_exe()?;
            let status = std::process::Command::new(exe).arg("--child").status()?;
            Ok(json!({ "status": status.to_string() }))
        }
        "allocate" => {
            let mib = params["mib"].as_u64().ok_or("missing mib")? as usize;
            let len = mib * 1024 * 1024;
            let mut buf: Vec<u8> = Vec::new();
            buf.try_reserve_exact(len)
                .map_err(|e| Fail::new("OutOfMemory", format!("allocation of {mib} MiB refused: {e}")))?;
            // Touch every byte so the pages are really committed, not just reserved.
            buf.resize(len, 0xA5);
            let sum = buf.iter().step_by(4096).map(|b| *b as u64).sum::<u64>();
            Ok(json!({ "mib": mib, "checksum": sum }))
        }
        "burn_cpu" => {
            let millis = params["millis"].as_u64().ok_or("missing millis")?;
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
            for w in workers {
                w.join().map_err(|_| Fail::new("Other", "worker panicked"))?;
            }
            let wall_ms = start.elapsed().as_secs_f64() * 1000.0;
            let cpu_ms = process_cpu_ms()? - cpu_before;
            Ok(json!({ "cpu_ms": cpu_ms, "wall_ms": wall_ms, "threads": threads }))
        }
        other => Err(format!("unknown method {other}").into()),
    }
}

fn str_param<'a>(params: &'a Value, key: &str) -> Result<&'a str, String> {
    params[key].as_str().ok_or_else(|| format!("missing {key}"))
}

/// User plus kernel CPU time of this whole process, in milliseconds.
#[cfg(windows)]
fn process_cpu_ms() -> Result<f64, Fail> {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    let zero = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
    let (mut c, mut e, mut k, mut u) = (zero, zero, zero, zero);
    // SAFETY: GetCurrentProcess is a pseudo-handle; the out-pointers are valid locals.
    let ok = unsafe { GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) };
    let t = |f: FILETIME| ((f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64) as f64;
    cpu_clock(ok != 0, "GetProcessTimes", (t(k) + t(u)) / 10_000.0)
}

#[cfg(unix)]
fn process_cpu_ms() -> Result<f64, Fail> {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: valid out-pointer.
    let ret = unsafe { libc::clock_gettime(libc::CLOCK_PROCESS_CPUTIME_ID, &mut ts) };
    cpu_clock(ret == 0, "clock_gettime", ts.tv_sec as f64 * 1000.0 + ts.tv_nsec as f64 / 1e6)
}

/// A CPU-time reading, or the OS error of the call that failed to take it. A failed call
/// leaves the zeroed locals behind, and a zero must not be reported as a measurement:
/// check 6 would read it as throttling.
fn cpu_clock(succeeded: bool, call: &str, ms: f64) -> Result<f64, Fail> {
    if succeeded {
        return Ok(ms);
    }
    let e = std::io::Error::last_os_error();
    Err(Fail { message: format!("{call} failed: {e}"), ..Fail::from(e) })
}

#[cfg(test)]
mod tests {
    use super::cpu_clock;

    #[test]
    fn a_failed_clock_read_is_an_error_not_zero() {
        // Check 6 would read a zero as "throttled".
        let reading = cpu_clock(false, "clock_gettime", 0.0);
        assert!(reading.is_err(), "got {:?}", reading.ok());
        let fail = reading.err().expect("an error");
        assert!(fail.message.starts_with("clock_gettime failed"), "{}", fail.message);
    }

    #[test]
    fn a_successful_clock_read_is_the_reading() {
        assert_eq!(cpu_clock(true, "clock_gettime", 12.5).ok(), Some(12.5));
    }
}
