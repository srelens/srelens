//! Harness for the #571 sandbox feasibility spike.
//!
//! The host (this library, driven by `tests/checks.rs`) starts the probe sidecar
//! (`src/bin/probe.rs`) under one sandbox backend and talks newline-delimited JSON-RPC to
//! it over stdin/stdout. `SPIKE_BACKEND` picks the backend.

use serde_json::{json, Value};
use std::fmt;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

/// The limits every backend is asked to enforce.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub memory_mib: u64,
    /// CPU time as a number of CPUs (0.25 = a quarter of one CPU).
    pub cpus: f64,
}

impl Limits {
    pub const SPIKE: Limits = Limits { memory_mib: 128, cpus: 0.25 };
}

/// One sandbox configuration. Single-facility backends exist so each guarantee in the
/// matrix can be traced to the facility that provides it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// No sandbox: the baseline every "must be denied" check fails against.
    None,
    /// Windows: AppContainer with no capabilities, no Job Object.
    AppContainer,
    /// Windows: Job Object limits only, ordinary user token.
    Job,
    /// Windows: AppContainer inside a Job Object. The recommended Windows backend.
    AppContainerJob,
    /// Windows: less-privileged AppContainer (no ALL APPLICATION PACKAGES) inside a Job.
    LpacJob,
    /// Linux: Landlock only (filesystem, and TCP where the kernel's ABI has it).
    Landlock,
    /// Linux: the seccomp filter only (non-Unix sockets, io_uring, new processes).
    Seccomp,
    /// Linux: a cgroup v2 directory only (memory and CPU).
    Cgroup,
    /// Linux: all three layers. The recommended Linux backend.
    LandlockSeccompCgroup,
    /// Linux: bubblewrap with every namespace unshared, nothing else.
    Bwrap,
    /// macOS: `sandbox-exec` with `src/seatbelt.sb`, plus rlimits. Verified on macOS 27.0
    /// arm64 only.
    Seatbelt,
}

const BACKENDS: &[(Backend, &str)] = &[
    (Backend::None, "none"),
    (Backend::AppContainer, "appcontainer"),
    (Backend::Job, "job"),
    (Backend::AppContainerJob, "appcontainer+job"),
    (Backend::LpacJob, "lpac+job"),
    (Backend::Landlock, "landlock"),
    (Backend::Seccomp, "seccomp"),
    (Backend::Cgroup, "cgroup"),
    (Backend::LandlockSeccompCgroup, "landlock+seccomp+cgroup"),
    (Backend::Bwrap, "bwrap"),
    (Backend::Seatbelt, "seatbelt"),
];

impl Backend {
    /// `SPIKE_BACKEND`, or this OS's recommended backend when it is unset.
    pub fn from_env() -> io::Result<Backend> {
        let name = match std::env::var("SPIKE_BACKEND") {
            Ok(name) => name,
            Err(_) => Self::recommended()
                .ok_or_else(|| io::Error::other("no sandbox backend on this OS"))?
                .to_string(),
        };
        BACKENDS
            .iter()
            .find(|(_, n)| *n == name)
            .map(|(b, _)| *b)
            .ok_or_else(|| io::Error::other(format!("unknown SPIKE_BACKEND {name}")))
    }

    /// The backend this spike recommends for the OS it runs on, if there is one.
    pub fn recommended() -> Option<Backend> {
        if cfg!(windows) {
            Some(Backend::AppContainerJob)
        } else if cfg!(target_os = "linux") {
            Some(Backend::LandlockSeccompCgroup)
        } else if cfg!(target_os = "macos") {
            // The candidate under test, not a recommendation: see the ADR.
            Some(Backend::Seatbelt)
        } else {
            None
        }
    }
}

impl fmt::Display for Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = BACKENDS.iter().find(|(b, _)| b == self).map(|(_, n)| *n).unwrap_or("?");
        f.write_str(name)
    }
}

#[cfg(test)]
mod denial_tests {
    use super::{Denial, Failure};

    fn failure(kind: &str, os: Option<i64>) -> Failure {
        Failure { message: String::new(), kind: kind.into(), os }
    }

    #[test]
    fn invalid_input_is_never_a_denial() {
        for denial in [Denial::File, Denial::Network, Denial::Dns, Denial::Process, Denial::Memory] {
            assert!(!denial.accepts(&failure("InvalidInput", None)), "{denial:?}");
        }
    }

    #[test]
    fn file_denials_are_permission_not_found_or_read_only() {
        for kind in ["PermissionDenied", "NotFound", "ReadOnlyFilesystem"] {
            assert!(Denial::File.accepts(&failure(kind, None)), "{kind}");
        }
        assert!(!Denial::File.accepts(&failure("TimedOut", None)));
    }

    #[test]
    fn network_denials_are_the_ways_a_connect_is_refused() {
        for kind in [
            "PermissionDenied",
            "TimedOut",
            "ConnectionRefused",
            "NetworkUnreachable",
            "HostUnreachable",
        ] {
            assert!(Denial::Network.accepts(&failure(kind, None)), "{kind}");
        }
        assert!(!Denial::Network.accepts(&failure("NotFound", None)));
    }

    #[test]
    fn dns_denial_is_a_resolver_failure() {
        assert!(Denial::Dns.accepts(&failure("ResolverFailed", None)));
        assert!(!Denial::Dns.accepts(&failure("Uncategorized", None)));
    }

    #[cfg(unix)]
    #[test]
    fn process_denial_is_eperm_not_a_filesystem_refusal() {
        assert!(Denial::Process.accepts(&failure("PermissionDenied", Some(1))));
        // EACCES is what a filesystem layer returns (for example on /dev/null): not a
        // refusal to create the process.
        assert!(!Denial::Process.accepts(&failure("PermissionDenied", Some(13))));
    }

    #[cfg(windows)]
    #[test]
    fn process_denial_is_the_job_process_quota_not_access_denied() {
        assert!(Denial::Process.accepts(&failure("Uncategorized", Some(1816))));
        assert!(!Denial::Process.accepts(&failure("PermissionDenied", Some(5))));
    }

    #[test]
    fn memory_denial_is_a_refused_allocation() {
        assert!(Denial::Memory.accepts(&failure("OutOfMemory", None)));
        assert!(!Denial::Memory.accepts(&failure("Uncategorized", None)));
    }
}

#[cfg(test)]
mod stop_tests {
    use super::{Ended, MemoryEvents, Stop};

    fn ended(text: &str, signal: Option<i32>, memory_events: Option<MemoryEvents>) -> Ended {
        Ended { text: text.into(), signal, memory_events }
    }

    fn events(oom: u64, oom_kill: u64) -> Option<MemoryEvents> {
        Some(MemoryEvents { oom, oom_kill })
    }

    #[test]
    fn an_ordinary_exit_is_never_a_limit() {
        // A panic in the probe: exit status 101, no signal.
        let panicked = ended("exited: exit status: 101", None, None);
        for stop in [Stop::Memory, Stop::Cpu] {
            assert!(!stop.accepts(&panicked), "{stop:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn an_abort_is_never_a_limit() {
        let aborted = ended("exited: signal: 6 (SIGABRT)", Some(libc::SIGABRT), None);
        for stop in [Stop::Memory, Stop::Cpu] {
            assert!(!stop.accepts(&aborted), "{stop:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_memory_stop_is_the_cgroup_limits_oom_kill() {
        let oom = "exited: signal: 9 (SIGKILL); cgroup memory.events oom 1, oom_kill 1";
        let killed = ended(oom, Some(libc::SIGKILL), events(1, 1));
        assert!(Stop::Memory.accepts(&killed));
        let xcpu = ended("exited: signal: 24 (SIGXCPU)", Some(libc::SIGXCPU), events(1, 1));
        assert!(!Stop::Memory.accepts(&xcpu));
    }

    #[cfg(unix)]
    #[test]
    fn a_sigkill_without_a_recorded_oom_kill_is_not_the_memory_limit() {
        // Killed from outside the cgroup's memory limit (`kill -9`, say): no OOM kill.
        let other = "exited: signal: 9 (SIGKILL); cgroup memory.events oom 0, oom_kill 0";
        assert!(!Stop::Memory.accepts(&ended(other, Some(libc::SIGKILL), events(0, 0))));
        // No cgroup at all, so no counters: the system OOM killer, or anything else.
        let bare = ended("exited: signal: 9 (SIGKILL)", Some(libc::SIGKILL), None);
        assert!(!Stop::Memory.accepts(&bare));
    }

    #[cfg(unix)]
    #[test]
    fn a_system_oom_kill_is_not_the_cgroup_limit() {
        // oom_kill counts kills by any OOM killer, the system's included. Without the
        // cgroup's own oom event, its memory.max was never reached.
        let global = "exited: signal: 9 (SIGKILL); cgroup memory.events oom 0, oom_kill 1";
        assert!(!Stop::Memory.accepts(&ended(global, Some(libc::SIGKILL), events(0, 1))));
    }

    #[cfg(unix)]
    #[test]
    fn a_cpu_stop_is_rlimit_cpus_sigxcpu() {
        let xcpu = ended("exited: signal: 24 (SIGXCPU)", Some(libc::SIGXCPU), None);
        assert!(Stop::Cpu.accepts(&xcpu));
    }

    #[cfg(unix)]
    #[test]
    fn a_sigkill_is_not_a_cpu_stop() {
        // Nothing tells RLIMIT_CPU's SIGKILL (Linux, soft = hard) from an OOM or outside kill.
        let killed = ended("exited: signal: 9 (SIGKILL)", Some(libc::SIGKILL), None);
        assert!(!Stop::Cpu.accepts(&killed));
    }
}

#[cfg(test)]
mod reply_tests {
    use super::{Process, Reply, Sidecar};
    use serde_json::json;
    use std::io;
    use std::process::{Command, Stdio};

    /// A sidecar whose stdout is `bytes`, attached to a real process that has already
    /// exited, so a call that wrongly waits for it returns at once instead of hanging.
    fn reading(bytes: &[u8]) -> Sidecar {
        let mut child = Command::new(env!("CARGO"))
            .arg("--version")
            .stdout(Stdio::null())
            .spawn()
            .expect("cargo starts");
        child.wait().expect("cargo exits");
        let stdout = Box::new(io::Cursor::new(bytes.to_vec()));
        Sidecar::new(Box::new(io::sink()), stdout, Process::Plain(child))
    }

    #[test]
    fn an_unreadable_reply_is_garbled_not_stopped() {
        // A line that is not UTF-8 fails the read; the process may still be running.
        let mut sidecar = reading(b"\xff\xfe not UTF-8\n");
        match sidecar.call("ping", json!({})) {
            Reply::Garbled(text) => assert!(text.contains("unreadable"), "{text}"),
            other => panic!("expected Garbled, got {other:?}"),
        }
    }

    #[test]
    fn an_unreadable_line_in_a_garble_is_not_a_closed_stdout() {
        let mut sidecar = reading(b"thread 'main' panicked\n\xff\n");
        match sidecar.call("ping", json!({})) {
            Reply::Garbled(text) => {
                assert!(text.contains("unreadable"), "{text}");
                assert!(!text.contains("stdout closed"), "{text}");
            }
            other => panic!("expected Garbled, got {other:?}"),
        }
    }
}

/// What the OS can say about a sidecar that never answered its first ping. On macOS under
/// `seatbelt`, the sandbox's recent log entries for the probe; elsewhere, nothing more.
pub fn start_failure_context(backend: Backend) -> String {
    #[cfg(target_os = "macos")]
    if backend == Backend::Seatbelt {
        return format!("\nrecent sandbox log entries for the probe:\n{}", macos::recent_denials());
    }
    let _ = backend;
    String::new()
}

/// Remove per-user state a run registered (the Windows AppContainer profile).
pub fn cleanup() -> io::Result<()> {
    #[cfg(windows)]
    windows::delete_profile()?;
    Ok(())
}

/// Throwaway directories for one run:
///
/// ```text
/// <root>/bin/probe[.exe]        the sidecar binary, copied here
/// <root>/scratch/               the one directory the sidecar is granted
/// <root>/home/.kube/config      a stand-in kubeconfig
/// <root>/outside/secret.txt     an ordinary file outside the grant
/// ```
pub struct Fixture {
    root: PathBuf,
}

impl Fixture {
    pub fn new() -> io::Result<Fixture> {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "srelens-sandbox-spike-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        let fixture = Fixture { root };
        std::fs::create_dir_all(fixture.root.join("bin"))?;
        std::fs::create_dir_all(fixture.scratch())?;
        std::fs::create_dir_all(fixture.kubeconfig().parent().expect("parent"))?;
        std::fs::create_dir_all(fixture.outside_dir())?;
        std::fs::write(fixture.kubeconfig(), "apiVersion: v1\nkind: Config\n")?;
        std::fs::write(fixture.outside_file(), "not for the sidecar\n")?;
        std::fs::copy(built_binary("probe")?, fixture.probe())?;
        Ok(fixture)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn probe(&self) -> PathBuf {
        self.root.join("bin").join(format!("probe{}", std::env::consts::EXE_SUFFIX))
    }
    pub fn scratch(&self) -> PathBuf {
        self.root.join("scratch")
    }
    pub fn home(&self) -> PathBuf {
        self.root.join("home")
    }
    pub fn kubeconfig(&self) -> PathBuf {
        self.home().join(".kube").join("config")
    }
    pub fn outside_dir(&self) -> PathBuf {
        self.root.join("outside")
    }
    pub fn outside_file(&self) -> PathBuf {
        self.outside_dir().join("secret.txt")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// `target/<profile>/<name>[.exe]`, next to the `deps/` directory the test binary runs from.
fn built_binary(name: &str) -> io::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let name = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    exe.ancestors()
        .skip(1)
        .take(3)
        .map(|dir| dir.join(&name))
        .find(|p| p.is_file())
        .ok_or_else(|| io::Error::other(format!("no built {name} near {}", exe.display())))
}

/// An operation the probe reports as failed: the message, the `std::io::ErrorKind` name
/// (or `ResolverFailed` / `OutOfMemory` / `InvalidInput`, which the probe sets itself), and
/// the raw OS error code when there is one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub message: String,
    pub kind: String,
    pub os: Option<i64>,
}

/// Which kind of denial a check expects. A failure counts as the sandbox's refusal only if
/// its kind is one a sandbox produces for that operation. For the network, DNS and
/// filesystem, several kinds qualify (a dropped connection times out, a hidden file is not
/// found), so those checks also run a host-side positive control first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Denial {
    File,
    Network,
    Dns,
    Process,
    Memory,
}

impl Denial {
    pub fn accepts(self, failure: &Failure) -> bool {
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
            // Resolver errors carry no distinctive code on either OS, so this accepts any
            // failure of the resolver; the host control is what makes it meaningful.
            Denial::Dns => matches!(kind, "ResolverFailed" | "PermissionDenied"),
            // seccomp's EPERM, not EACCES, which is a filesystem refusal.
            #[cfg(unix)]
            Denial::Process => failure.os == Some(libc::EPERM as i64),
            // The Job Object's active-process limit: ERROR_NOT_ENOUGH_QUOTA.
            #[cfg(windows)]
            Denial::Process => failure.os == Some(1816),
            #[cfg(not(any(unix, windows)))]
            Denial::Process => false,
            Denial::Memory => kind == "OutOfMemory",
        }
    }
}

/// How a sidecar ended, once its stdout closed.
pub struct Ended {
    /// What the OS reported: the exit status and, under a cgroup, its OOM kills.
    pub text: String,
    /// The signal that ended it, on Unix.
    pub signal: Option<i32>,
    /// The sidecar cgroup's OOM counters, when it had a cgroup and they could be read.
    pub memory_events: Option<MemoryEvents>,
}

/// The two counters in a cgroup's `memory.events` that show its own limit killed a process.
/// The sidecar's cgroup is a fresh leaf, so both start at 0 and count only its own events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryEvents {
    /// Times the cgroup's usage reached `memory.max` and an allocation was about to fail.
    pub oom: u64,
    /// Processes in the cgroup killed by any OOM killer, the system's included.
    pub oom_kill: u64,
}

/// Prints as the text alone, so a recorded reply reads `Stopped("exited: …")`.
impl fmt::Debug for Ended {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.text, f)
    }
}

/// The signal that ended a process, on Unix; `None` for an ordinary exit and on Windows.
fn signal_of(status: &std::process::ExitStatus) -> Option<i32> {
    #[cfg(unix)]
    return std::os::unix::process::ExitStatusExt::signal(status);
    #[cfg(not(unix))]
    {
        let _ = status;
        None
    }
}

/// Which resource limit a check expects to have stopped the sidecar. A stop counts only if
/// it ended with the signal that limit sends (for memory, with the cgroup recording that
/// its own limit was reached and a process OOM-killed): a panic (exit status 101) or an
/// abort is the sidecar breaking, never a limit. No Windows backend here stops the sidecar
/// for a limit (the Job Object refuses the allocation and throttles the CPU), so none
/// counts there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    Memory,
    Cpu,
}

impl Stop {
    pub fn accepts(self, ended: &Ended) -> bool {
        #[cfg(unix)]
        return match self {
            // The OOM killer's SIGKILL, with the sidecar's cgroup recording both that it hit
            // its memory.max (oom) and that a process was OOM-killed (oom_kill). oom_kill
            // alone also counts the system's OOM killer; a SIGKILL with no cgroup to count
            // it, or from anything else, is not the limit.
            Stop::Memory => {
                ended.signal == Some(libc::SIGKILL)
                    && matches!(ended.memory_events, Some(e) if e.oom > 0 && e.oom_kill > 0)
            }
            // RLIMIT_CPU's SIGXCPU, the one signal only it sends. The launcher sets soft =
            // hard: macOS then sends SIGXCPU, and Linux sends SIGKILL, which nothing tells
            // apart from any other kill (no Linux backend here uses RLIMIT_CPU).
            Stop::Cpu => ended.signal == Some(libc::SIGXCPU),
        };
        #[cfg(not(unix))]
        {
            let _ = (self, ended);
            false
        }
    }
}

/// What came back from one call.
#[derive(Debug)]
pub enum Reply {
    /// The probe ran the operation and it worked.
    Ok(Value),
    /// The probe answered that the operation failed.
    Refused(Failure),
    /// The probe did not answer: its stdout closed. Carries how the process ended.
    Stopped(Ended),
    /// Something other than a well-formed reply to this request came back (for example a
    /// panic message). Never a denial: the exchange itself broke.
    Garbled(String),
}

pub struct Sidecar {
    stdin: Box<dyn Write + Send>,
    stdout: BufReader<Box<dyn Read + Send>>,
    process: Process,
    next_id: u64,
}

enum Process {
    Plain(Child),
    #[cfg(windows)]
    Windows(windows::Contained),
    #[cfg(target_os = "linux")]
    Linux(linux::Confined),
}

impl Sidecar {
    pub fn launch(backend: Backend, fixture: &Fixture, limits: &Limits) -> io::Result<Sidecar> {
        match backend {
            Backend::None => {
                let mut child = Command::new(fixture.probe())
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::inherit())
                    .spawn()?;
                let stdin = Box::new(child.stdin.take().expect("piped stdin"));
                let stdout: Box<dyn Read + Send> = Box::new(child.stdout.take().expect("piped"));
                Ok(Sidecar::new(stdin, stdout, Process::Plain(child)))
            }
            #[cfg(windows)]
            Backend::AppContainer | Backend::Job | Backend::AppContainerJob | Backend::LpacJob => {
                let layers = windows::Layers {
                    appcontainer: backend != Backend::Job,
                    lpac: backend == Backend::LpacJob,
                    job: backend != Backend::AppContainer,
                };
                let (stdin, stdout, contained) = windows::launch(fixture, limits, layers)?;
                Ok(Sidecar::new(Box::new(stdin), Box::new(stdout), Process::Windows(contained)))
            }
            #[cfg(target_os = "linux")]
            Backend::Landlock
            | Backend::Seccomp
            | Backend::Cgroup
            | Backend::LandlockSeccompCgroup
            | Backend::Bwrap => {
                let all = backend == Backend::LandlockSeccompCgroup;
                let mut confined = if backend == Backend::Bwrap {
                    linux::launch_bwrap(fixture)?
                } else {
                    let layers = linux::Layers {
                        landlock: all || backend == Backend::Landlock,
                        seccomp: all || backend == Backend::Seccomp,
                        cgroup: all || backend == Backend::Cgroup,
                    };
                    linux::launch_layered(fixture, limits, layers)?
                };
                let stdin = Box::new(confined.child.stdin.take().expect("piped stdin"));
                let stdout: Box<dyn Read + Send> =
                    Box::new(confined.child.stdout.take().expect("piped stdout"));
                Ok(Sidecar::new(stdin, stdout, Process::Linux(confined)))
            }
            #[cfg(target_os = "macos")]
            Backend::Seatbelt => {
                let mut child = macos::launch_seatbelt(fixture, limits)?;
                let stdin = Box::new(child.stdin.take().expect("piped stdin"));
                let stdout: Box<dyn Read + Send> = Box::new(child.stdout.take().expect("piped"));
                Ok(Sidecar::new(stdin, stdout, Process::Plain(child)))
            }
            #[allow(unreachable_patterns)]
            other => {
                let _ = limits;
                Err(io::Error::other(format!("{other} is not available on this OS")))
            }
        }
    }

    fn new(stdin: Box<dyn Write + Send>, stdout: Box<dyn Read + Send>, process: Process) -> Sidecar {
        Sidecar { stdin, stdout: BufReader::new(stdout), process, next_id: 1 }
    }

    pub fn call(&mut self, method: &str, params: Value) -> Reply {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        if writeln!(self.stdin, "{request}").and_then(|()| self.stdin.flush()).is_err() {
            return Reply::Stopped(self.ended());
        }
        let mut line = String::new();
        match self.stdout.read_line(&mut line) {
            Ok(0) => Reply::Stopped(self.ended()),
            // A failed read (bytes that are not UTF-8, say) is a broken exchange, not a
            // closed stdout: the process may still be running, and waiting for it would hang.
            Err(e) => Reply::Garbled(format!("unreadable reply: {e}")),
            Ok(_) => match serde_json::from_str::<Value>(&line) {
                Ok(v) if v["id"] != id => Reply::Garbled(format!("reply for the wrong id: {line}")),
                Ok(v) if v.get("error").is_some() => {
                    let error = &v["error"];
                    Reply::Refused(Failure {
                        message: error["message"].as_str().unwrap_or("?").to_owned(),
                        kind: error["data"]["kind"].as_str().unwrap_or("?").to_owned(),
                        os: error["data"]["os"].as_i64(),
                    })
                }
                Ok(v) => Reply::Ok(v["result"].clone()),
                Err(_) => Reply::Garbled(self.rest_of_garble(line)),
            },
        }
    }

    /// Collect the lines after an unparseable one, up to the next JSON line or EOF, so the
    /// report shows what the probe actually wrote (typically a panic message).
    fn rest_of_garble(&mut self, first: String) -> String {
        let mut text = first;
        for _ in 0..20 {
            let mut line = String::new();
            match self.stdout.read_line(&mut line) {
                Ok(0) => {
                    text.push_str(&format!("[then stdout closed; {}]", self.ended().text));
                    break;
                }
                Err(e) => {
                    text.push_str(&format!("[then an unreadable line: {e}]"));
                    break;
                }
                Ok(_) => {
                    let json = line.trim_start().starts_with('{');
                    text.push_str(&line);
                    if json {
                        break;
                    }
                }
            }
        }
        text
    }

    /// How the process ended, once its stdout has closed.
    fn ended(&mut self) -> Ended {
        match &mut self.process {
            Process::Plain(child) => match child.wait() {
                Ok(status) => {
                    let signal = signal_of(&status);
                    Ended { text: format!("exited: {status}"), signal, memory_events: None }
                }
                Err(e) => {
                    Ended { text: format!("wait failed: {e}"), signal: None, memory_events: None }
                }
            },
            #[cfg(windows)]
            Process::Windows(c) => Ended { text: c.ended(), signal: None, memory_events: None },
            #[cfg(target_os = "linux")]
            Process::Linux(c) => c.ended(),
        }
    }
}

impl Drop for Sidecar {
    fn drop(&mut self) {
        match &mut self.process {
            Process::Plain(child) => {
                let _ = child.kill();
                let _ = child.wait();
            }
            #[cfg(windows)]
            Process::Windows(_) => {} // Contained's own Drop terminates it.
            #[cfg(target_os = "linux")]
            Process::Linux(_) => {} // Confined's own Drop kills it and removes its cgroup.
        }
    }
}
