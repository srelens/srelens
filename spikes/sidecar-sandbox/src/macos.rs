//! macOS backend: the probe runs under `sandbox-exec` with the Seatbelt profile in
//! `src/seatbelt.sb`, started by `sandbox-launch`, which first sets rlimits for memory and
//! CPU and records whether macOS accepted them.
//!
//! UNVERIFIED: this compiles from Windows (`cargo check --target aarch64-apple-darwin`)
//! but has never run. `run-macos.sh` is the first run.
//!
//! What it can and cannot claim, from the research in the ADR:
//!
//! - Checks 1 to 4 and 7 are the profile's job.
//! - Check 5 (memory): `RLIMIT_DATA` and `RLIMIT_AS` are set to the limit, and the launcher
//!   prints whether each was accepted. They are reported not to be enforced on macOS, and
//!   the check records what actually happens.
//! - Check 6 (CPU): macOS has no rate limit. `RLIMIT_CPU` is a lifetime budget of CPU
//!   seconds, which kills the process with `SIGXCPU` once spent: a different guarantee from
//!   "at most 0.25 of a CPU". The spike sets a generous budget ([`CPU_BUDGET_SECONDS`]) so
//!   that check 6 measures rate limiting, which is expected to be "not provided", instead of
//!   passing because a small budget ran out.

use crate::{built_binary, Fixture, Limits};
use std::io;
use std::process::{Child, Command, Stdio};

/// The Seatbelt profile, kept in its own file so it can be read and reviewed as SBPL.
pub const PROFILE: &str = include_str!("seatbelt.sb");

/// `RLIMIT_CPU` for the sidecar: long enough that the 6 CPU-seconds check 6 burns cannot
/// exhaust it.
pub const CPU_BUDGET_SECONDS: u64 = 60;

pub fn launch_seatbelt(fixture: &Fixture, limits: &Limits) -> io::Result<Child> {
    let profile = fixture.root().join("seatbelt.sb");
    std::fs::write(&profile, PROFILE)?;
    // Seatbelt matches resolved paths; $TMPDIR is under /var, a symlink to /private/var.
    let probe = std::fs::canonicalize(fixture.probe())?;
    let scratch = std::fs::canonicalize(fixture.scratch())?;
    Command::new(built_binary("sandbox-launch")?)
        .arg("--rlimit-mem")
        .arg(limits.memory_mib.to_string())
        .arg("--rlimit-cpu")
        .arg(CPU_BUDGET_SECONDS.to_string())
        .arg("--")
        .arg("/usr/bin/sandbox-exec")
        .arg("-f")
        .arg(&profile)
        .arg("-D")
        .arg(format!("PROBE={}", probe.display()))
        .arg("-D")
        .arg(format!("SCRATCH={}", scratch.display()))
        .arg(&probe)
        .current_dir(&scratch)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
}
