//! Starting a sidecar confined: the backends the #571 spike chose, one per OS
//! (`docs/design/plugin-architecture.md`, "Recommended implementation per
//! OS").
//!
//! | OS | Isolation | Memory and CPU |
//! |---|---|---|
//! | Linux | Landlock and a seccomp filter, applied by `srelens-sandbox-launch` before it `exec`s the sidecar | a cgroup v2 directory the host creates under a delegated root |
//! | macOS | Seatbelt, through `/usr/bin/sandbox-exec` | a host-side watchdog (`watchdog.rs`, #713), weaker than the kernel's |
//! | Windows | an AppContainer with no capabilities | the Job Object the process starts in |
//! | anything else | none | none |
//!
//! [`OsSandbox`] is the only [`Launcher`] srelens runs sidecars with. The trait
//! is public so the supervisor can be tested with a fake one; a launcher that
//! does not confine is never a fallback.

use std::ffi::OsString;
use std::fmt;
use std::future::Future;
use std::io;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::ExitStatus;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, oneshot};

use super::Limits;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "linux")]
mod systemd;
#[cfg(windows)]
mod windows;

// Only the macOS backend runs the watchdog; every OS tests its policy, and
// Linux its loop too.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod watchdog;

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[doc(hidden)]
pub mod launch;

#[cfg(target_os = "macos")]
#[doc(hidden)]
pub use macos::watch;
#[cfg(windows)]
pub use windows::delete_profile;

/// What to run, and the one directory it may write.
#[derive(Debug, Clone)]
pub struct SidecarCommand {
    /// The app the sidecar belongs to; names its cgroup and AppContainer.
    pub app_id: String,
    pub program: PathBuf,
    pub args: Vec<OsString>,
    /// The whole environment the sidecar gets. The host's own is never
    /// inherited: it may hold `KUBECONFIG`, cloud credentials or tokens.
    pub env: Vec<(OsString, OsString)>,
    /// The only path it may write, and its working directory: the app's
    /// [`super::data::DataDir`] (#573). The supervisor refuses one that is
    /// over its limit, not private, or a link, and passes it to the sidecar
    /// in `initialize`.
    pub data_dir: PathBuf,
}

/// `TMPDIR`, set to the data directory unless the command names its own
/// (#573): the one place a sidecar may write, so its temporary files go where
/// they can be written and are counted against its limit. Linux and macOS;
/// Windows points `TEMP` into the AppContainer's folder itself.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn temporary_directory<'a>(
    command: &SidecarCommand,
    data_dir: &'a std::path::Path,
) -> Option<(&'static str, &'a std::path::Path)> {
    let named = command
        .env
        .iter()
        .any(|(name, _)| name.as_os_str() == "TMPDIR");
    (!named).then_some(("TMPDIR", data_dir))
}

/// Who enforces a sidecar's memory and CPU limits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Enforcement {
    /// The kernel refuses or stops at the limit: a Job Object, a cgroup.
    Kernel,
    /// The host watches and stops the sidecar past the limit, which a burst
    /// between two samples can exceed. The macOS watchdog (#713).
    Host,
    /// Nothing does, for the reason given. The supervisor refuses to start a
    /// sidecar then: isolation without limits was considered for macOS and
    /// rejected (ADR, "Decision: macOS limits are host-enforced").
    Missing(String),
}

/// Why a sidecar could not be started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchError {
    /// This machine cannot run it confined: no backend for the OS, a sandbox
    /// layer the kernel lacks, no cgroup to put it in. Retrying will not help,
    /// so the supervisor does not.
    Unavailable(String),
    /// Starting it failed (a missing binary, say). Counts as a failed start.
    Failed(String),
}

impl fmt::Display for LaunchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LaunchError::Unavailable(why) | LaunchError::Failed(why) => f.write_str(why),
        }
    }
}

/// Starts sidecars.
pub trait Launcher: Send + Sync + 'static {
    /// Who enforces the memory and CPU limits of what this starts.
    fn enforcement(&self) -> Enforcement;

    /// Start `command` under `limits`. Called on one of the tokio runtime's
    /// blocking threads, so it may block, and may spawn tasks.
    fn launch(&self, command: &SidecarCommand, limits: &Limits) -> Result<Launched, LaunchError>;
}

/// A started sidecar: its three pipes and the process.
pub struct Launched {
    pub stdin: Box<dyn AsyncWrite + Send + Unpin>,
    pub stdout: Box<dyn AsyncRead + Send + Unpin>,
    pub stderr: Box<dyn AsyncRead + Send + Unpin>,
    pub process: Process,
}

/// How a sidecar process ended. `description` completes "The extension
/// process …".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exit {
    pub description: String,
    pub code: Option<i32>,
    pub signal: Option<i32>,
    /// It was stopped by its memory limit, with the backend's evidence (on
    /// Linux, the cgroup's OOM counters beside the `SIGKILL`).
    pub memory_limit: bool,
}

impl Exit {
    /// From what the OS reported.
    pub fn from_status(status: io::Result<ExitStatus>) -> Exit {
        let status = match status {
            Ok(status) => status,
            Err(e) => {
                return Exit {
                    description: format!("ended, and srelens could not learn how: {e}"),
                    code: None,
                    signal: None,
                    memory_limit: false,
                }
            }
        };
        #[cfg(unix)]
        let signal = std::os::unix::process::ExitStatusExt::signal(&status);
        #[cfg(not(unix))]
        let signal: Option<i32> = None;
        let description = match (status.code(), signal) {
            (Some(code), _) => format!("exited with status {code}"),
            (None, Some(signal)) => match signal_name(signal) {
                Some(name) => format!("was killed by signal {signal} ({name})"),
                None => format!("was killed by signal {signal}"),
            },
            (None, None) => format!("ended: {status}"),
        };
        Exit {
            description,
            code: status.code(),
            signal,
            memory_limit: false,
        }
    }
}

/// The name of a signal that commonly ends a process.
fn signal_name(signal: i32) -> Option<&'static str> {
    #[cfg(unix)]
    {
        let name = match signal {
            libc::SIGABRT => "SIGABRT",
            libc::SIGBUS => "SIGBUS",
            libc::SIGFPE => "SIGFPE",
            libc::SIGILL => "SIGILL",
            libc::SIGKILL => "SIGKILL",
            libc::SIGSEGV => "SIGSEGV",
            libc::SIGSYS => "SIGSYS",
            libc::SIGTERM => "SIGTERM",
            libc::SIGXCPU => "SIGXCPU",
            _ => return None,
        };
        Some(name)
    }
    #[cfg(not(unix))]
    {
        let _ = signal;
        None
    }
}

/// Reads a running sidecar's memory use now, in bytes; `None` once it cannot.
pub type MemoryProbe = Arc<dyn Fn() -> Option<u64> + Send + Sync>;

/// A running sidecar process: a way to stop it, and its exit.
pub struct Process {
    pid: Option<u32>,
    kill: Arc<dyn Fn() + Send + Sync>,
    exit: Pin<Box<dyn Future<Output = Exit> + Send>>,
    memory: Option<MemoryProbe>,
}

impl Process {
    /// `exit` resolves once the process has ended; `kill` stops it now, and
    /// may be called any number of times. Dropping the `Process` must stop
    /// it too: the backends arrange that through what `kill` captures.
    pub fn new(
        pid: Option<u32>,
        exit: impl Future<Output = Exit> + Send + 'static,
        kill: impl Fn() + Send + Sync + 'static,
    ) -> Process {
        Process {
            pid,
            kill: Arc::new(kill),
            exit: Box::pin(exit),
            memory: None,
        }
    }

    /// The same process, with a way to read its memory use for the Inspector
    /// (#575): the cgroup's `memory.current` on Linux, the process's committed
    /// private memory on Windows (#753), and the watchdog's last reading of the
    /// process's physical footprint on macOS (#713). A backend that cannot
    /// measure it leaves it out, and the Inspector says so rather than
    /// showing a number.
    pub fn with_memory(
        mut self,
        probe: impl Fn() -> Option<u64> + Send + Sync + 'static,
    ) -> Process {
        self.memory = Some(Arc::new(probe));
        self
    }

    /// The memory reader, when the backend has one.
    pub fn memory(&self) -> Option<MemoryProbe> {
        self.memory.clone()
    }

    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// A handle that stops the process from anywhere.
    pub fn killer(&self) -> Arc<dyn Fn() + Send + Sync> {
        self.kill.clone()
    }

    /// Wait for the process to end. Call it until it returns once; not after.
    pub fn exit(&mut self) -> &mut Pin<Box<dyn Future<Output = Exit> + Send>> {
        &mut self.exit
    }
}

/// What a backend's wait task holds: the requests to stop the process, and
/// where its exit goes.
struct Waiter {
    /// A kill, or `None` once every handle to the process is dropped, which
    /// must stop it too.
    stopped: mpsc::UnboundedReceiver<()>,
    ended: oneshot::Sender<Exit>,
}

impl Launched {
    /// A launched child with all three stdio streams piped. `describe` turns
    /// its exit status into an [`Exit`], after the child has been reaped, so a
    /// backend can add what it knows (and clean up) there. The child is killed
    /// when the returned [`Process`] is dropped.
    pub fn from_child(
        mut child: tokio::process::Child,
        describe: impl FnOnce(io::Result<ExitStatus>) -> Exit + Send + 'static,
    ) -> Result<Launched, LaunchError> {
        let (launched, Waiter { mut stopped, ended }) = Launched::piped(&mut child)?;
        tokio::spawn(async move {
            let status = tokio::select! {
                status = child.wait() => status,
                // A kill, or every handle to the process dropped.
                _ = stopped.recv() => {
                    let _ = child.start_kill();
                    child.wait().await
                }
            };
            let _ = ended.send(describe(status));
        });
        Ok(launched)
    }

    /// The pipes and the [`Process`] of a child spawned with all three stdio
    /// streams piped, and the [`Waiter`] for the task that then waits on it:
    /// [`Launched::from_child`]'s, or the watchdog's.
    fn piped(child: &mut tokio::process::Child) -> Result<(Launched, Waiter), LaunchError> {
        let missing =
            |what: &str| LaunchError::Failed(format!("the sidecar's {what} is not piped"));
        let stdin = child.stdin.take().ok_or_else(|| missing("stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| missing("stdout"))?;
        let stderr = child.stderr.take().ok_or_else(|| missing("stderr"))?;
        let pid = child.id();
        let (stop, stopped) = mpsc::unbounded_channel::<()>();
        let (ended, exit) = oneshot::channel();
        let exit = async move {
            exit.await.unwrap_or_else(|_| Exit {
                description: "ended, and srelens lost track of how".into(),
                code: None,
                signal: None,
                memory_limit: false,
            })
        };
        let launched = Launched {
            stdin: Box::new(stdin),
            stdout: Box::new(stdout),
            stderr: Box::new(stderr),
            process: Process::new(pid, exit, move || {
                let _ = stop.send(());
            }),
        };
        Ok((launched, Waiter { stopped, ended }))
    }
}

/// Where the sandbox finds what it needs on this machine.
#[derive(Debug, Clone, Default)]
pub struct SandboxConfig {
    /// `srelens-sandbox-launch`, the trusted launcher that applies the Linux
    /// layers and starts Seatbelt on macOS. Linux and macOS only.
    pub launcher: Option<PathBuf>,
    /// Where each sidecar's cgroup is made. Linux only.
    pub cgroup: CgroupRoot,
}

/// Where a Linux sidecar's cgroup is made: under a cgroup v2 directory
/// delegated to srelens, with the `memory` and `cpu` controllers enabled for
/// its children, and srelens itself in a leaf of it. cgroup v2 lets a process
/// move another only between cgroups under one it may write, and each
/// sidecar's launcher moves itself from srelens's leaf into its own.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum CgroupRoot {
    /// Nowhere: every sidecar is refused.
    #[default]
    Missing,
    /// A directory set up by hand, as `SRELENS_SANDBOX_CGROUP_ROOT` names one.
    Delegated(PathBuf),
    /// A delegated scope srelens asks the systemd user manager for at its
    /// first sidecar start (`systemd.rs`).
    SystemdScope,
}

/// The sandbox backend for the OS srelens runs on.
#[derive(Debug, Clone)]
pub struct OsSandbox {
    config: SandboxConfig,
}

impl OsSandbox {
    pub fn new(config: SandboxConfig) -> OsSandbox {
        OsSandbox { config }
    }
}

/// The refusal for an OS with no backend.
pub fn unsupported(os: &str) -> String {
    format!(
        "srelens has no sandbox for executable apps on {os}, so it does not run them here. They run on Linux, macOS and Windows"
    )
}

impl Launcher for OsSandbox {
    fn enforcement(&self) -> Enforcement {
        #[cfg(target_os = "linux")]
        return Enforcement::Kernel;
        #[cfg(windows)]
        return Enforcement::Kernel;
        #[cfg(target_os = "macos")]
        return Enforcement::Host;
        #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
        return Enforcement::Missing(unsupported(std::env::consts::OS));
    }

    fn launch(&self, command: &SidecarCommand, limits: &Limits) -> Result<Launched, LaunchError> {
        #[cfg(target_os = "linux")]
        return linux::launch(&self.config, command, limits);
        #[cfg(target_os = "macos")]
        return macos::launch(&self.config, command, limits);
        #[cfg(windows)]
        {
            let _ = &self.config;
            windows::launch(command, limits)
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
        {
            let _ = (command, limits, &self.config);
            Err(LaunchError::Unavailable(unsupported(std::env::consts::OS)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_os_without_a_backend_is_refused_by_name() {
        let why = unsupported("freebsd");
        assert!(why.contains("freebsd"), "{why}");
        assert!(why.contains("does not run them"), "{why}");
    }

    #[cfg(unix)]
    #[test]
    fn an_exit_says_how_the_process_ended() {
        use std::os::unix::process::ExitStatusExt;
        let exited = Exit::from_status(Ok(ExitStatus::from_raw(3 << 8)));
        assert_eq!(exited.description, "exited with status 3");
        assert_eq!(exited.code, Some(3));
        let killed = Exit::from_status(Ok(ExitStatus::from_raw(libc::SIGABRT)));
        assert_eq!(killed.description, "was killed by signal 6 (SIGABRT)");
        assert_eq!(killed.signal, Some(libc::SIGABRT));
        let unknown = Exit::from_status(Ok(ExitStatus::from_raw(60)));
        assert_eq!(unknown.description, "was killed by signal 60");
        let lost = Exit::from_status(Err(io::Error::other("no child")));
        assert!(
            lost.description.contains("no child"),
            "{}",
            lost.description
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_runs_apps_with_host_enforced_limits() {
        assert_eq!(
            OsSandbox::new(SandboxConfig::default()).enforcement(),
            Enforcement::Host
        );
    }
}
