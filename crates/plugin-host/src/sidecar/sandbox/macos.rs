//! macOS: Seatbelt through `/usr/bin/sandbox-exec`, the backend the #571
//! spike verified on macOS 27.0 arm64 for isolation (its checks 1 to 4 and 7).
//!
//! macOS has no kernel facility that limits a process's memory or CPU rate
//! (`setrlimit` refuses `RLIMIT_DATA` and `RLIMIT_AS`; `RLIMIT_CPU` is a
//! lifetime budget). The accepted decision is a host-side watchdog (#713):
//! [`launch`] puts the sidecar under one (`watchdog.rs`), which stops it past
//! its memory limit and pauses it past its CPU rate. It has not yet been
//! checked with Seatbelt on a macOS 27 Mac, so [`LIMITS_MISSING`] is still
//! what this backend reports and the supervisor refuses to start a sidecar.
//! [`launch`] still works, so the isolation and the watchdog can be checked
//! (`tests/sandbox_conformance.rs`, `tests/macos_watchdog.rs`).
//!
//! The spike's launcher also set `RLIMIT_DATA`, `RLIMIT_AS` and a 60-second
//! `RLIMIT_CPU`. These are left out: the first two were refused, and the
//! third would kill a sidecar after a minute of CPU, which is not a limit.

use std::io;
use std::process::Stdio;

use super::watchdog::{self, Sampler, Usage};
use super::{Exit, LaunchError, Launched, SandboxConfig, SidecarCommand};
use crate::sidecar::Limits;

pub(super) const LIMITS_MISSING: &str = "srelens's memory and CPU watchdog for macOS has not yet been checked with Seatbelt on a macOS 27 Mac (#713), so srelens does not run executable apps here";

pub(super) fn launch(
    config: &SandboxConfig,
    command: &SidecarCommand,
    limits: &Limits,
) -> Result<Launched, LaunchError> {
    let launcher = config.launcher.as_ref().ok_or_else(|| {
        LaunchError::Unavailable(
            "srelens has no sandbox launcher (srelens-sandbox-launch) to start the app with, so it does not run executable apps".into(),
        )
    })?;
    if !std::path::Path::new("/usr/bin/sandbox-exec").is_file() {
        return Err(LaunchError::Unavailable(
            "This Mac has no /usr/bin/sandbox-exec, so srelens cannot confine an app and does not run executable apps".into(),
        ));
    }
    // Seatbelt matches resolved paths; `/var` and `/tmp` are symlinks.
    let canonical = |path: &std::path::Path, what: &str| {
        std::fs::canonicalize(path).map_err(|e| {
            LaunchError::Failed(format!(
                "srelens could not find the app's {what} {}: {e}",
                path.display()
            ))
        })
    };
    let program = canonical(&command.program, "program")?;
    let data = canonical(&command.data_dir, "data directory")?;
    let child = tokio::process::Command::new(launcher)
        .arg("--data")
        .arg(&data)
        .arg("--max-file-bytes")
        .arg(limits.data_bytes.to_string())
        .arg("--")
        .arg(&program)
        .args(&command.args)
        .env_clear()
        .envs(super::temporary_directory(command, &data))
        .envs(command.env.iter().map(|(k, v)| (k, v)))
        .current_dir(&data)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| {
            LaunchError::Failed(format!(
                "srelens could not start its sandbox launcher {}: {e}",
                launcher.display()
            ))
        })?;
    watch(child, limits)
}

/// The watchdog on `child`: what [`launch`] puts the sandboxed sidecar
/// under, public so `tests/macos_watchdog.rs` can run it on a process without
/// Seatbelt. It takes a child already started, so it is not a way to run
/// anything unconfined. `child` must have all three stdio streams piped and
/// `kill_on_drop(true)`: on an error it is dropped, which kills it.
#[doc(hidden)]
pub fn watch(child: tokio::process::Child, limits: &Limits) -> Result<Launched, LaunchError> {
    let rusage = Rusage::new().map_err(|e| {
        LaunchError::Failed(format!(
            "srelens could not read the Mach timebase it needs to measure the app's CPU: {e}"
        ))
    })?;
    watchdog::watched(child, Exit::from_status, limits, rusage)
}

/// Reads a process's physical footprint and CPU time with
/// `proc_pid_rusage`.
struct Rusage {
    /// The Mach timebase, which converts its CPU ticks to nanoseconds.
    numer: u32,
    denom: u32,
}

impl Rusage {
    // libc deprecates its Mach functions in favour of the mach2 crate; one
    // call does not justify the dependency.
    #[allow(deprecated)]
    fn new() -> Result<Rusage, String> {
        let mut base = libc::mach_timebase_info { numer: 0, denom: 0 };
        // SAFETY: a valid out-pointer.
        let kern = unsafe { libc::mach_timebase_info(&mut base) };
        let (numer, denom) = watchdog::timebase(kern, base.numer, base.denom)?;
        Ok(Rusage { numer, denom })
    }
}

impl Sampler for Rusage {
    fn sample(&mut self, pid: u32) -> io::Result<Usage> {
        let pid = libc::c_int::try_from(pid).map_err(io::Error::other)?;
        // SAFETY: plain data, for which zero is a valid value.
        let mut info: libc::rusage_info_v2 = unsafe { std::mem::zeroed() };
        // SAFETY: RUSAGE_INFO_V2 names the struct passed, which outlives the call.
        let ret = unsafe {
            libc::proc_pid_rusage(
                pid,
                libc::RUSAGE_INFO_V2,
                (&mut info as *mut libc::rusage_info_v2).cast::<libc::rusage_info_t>(),
            )
        };
        if ret != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Usage {
            footprint: info.ri_phys_footprint,
            cpu: watchdog::ticks_to_cpu(
                info.ri_user_time.saturating_add(info.ri_system_time),
                self.numer,
                self.denom,
            ),
        })
    }
}
