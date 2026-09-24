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

/// What came back from one call.
#[derive(Debug)]
pub enum Reply {
    /// The probe ran the operation and it worked.
    Ok(Value),
    /// The probe answered that the operation failed, with the OS error.
    Refused(String),
    /// The probe did not answer: its stdout closed. Carries how the process ended.
    Stopped(String),
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
            Ok(0) | Err(_) => Reply::Stopped(self.ended()),
            Ok(_) => match serde_json::from_str::<Value>(&line) {
                Ok(v) if v["id"] != id => Reply::Garbled(format!("reply for the wrong id: {line}")),
                Ok(v) if v.get("error").is_some() => {
                    Reply::Refused(v["error"]["message"].as_str().unwrap_or("?").to_owned())
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
                Ok(0) | Err(_) => {
                    text.push_str(&format!("[then stdout closed; {}]", self.ended()));
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
    fn ended(&mut self) -> String {
        match &mut self.process {
            Process::Plain(child) => match child.wait() {
                Ok(status) => format!("exited: {status}"),
                Err(e) => format!("wait failed: {e}"),
            },
            #[cfg(windows)]
            Process::Windows(c) => c.ended(),
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
