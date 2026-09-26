//! macOS backend: the probe runs under `sandbox-exec` with the Seatbelt profile in
//! `src/seatbelt.sb`, started by `sandbox-launch`, which first sets rlimits for memory and
//! CPU and records whether macOS accepted them.
//!
//! Verified on macOS 27.0 arm64 only, by the maintainer's run at 02190671: checks 1 to 4
//! and 7 passed, and 5 and 6 failed as expected (below). Intel Macs and older macOS
//! versions have not been run. `run-macos.sh` drives it.
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
use std::process::{Child, Command, Output, Stdio};

/// The Seatbelt profile, kept in its own file so it can be read and reviewed as SBPL.
pub const PROFILE: &str = include_str!("seatbelt.sb");

/// `RLIMIT_CPU` for the sidecar: long enough that the 6 CPU-seconds check 6 burns cannot
/// exhaust it.
pub const CPU_BUDGET_SECONDS: u64 = 60;

/// The sandbox's recent log entries for the probe, for a sidecar that never answered: on the
/// first Mac run, the only clue to why the probe aborted was a `deny(1) file-read-data /`
/// entry in the unified log.
pub fn recent_denials() -> String {
    // The unified log lags the event slightly.
    std::thread::sleep(std::time::Duration::from_secs(1));
    let out = Command::new("/usr/bin/log")
        .args(["show", "--style", "compact", "--last", "1m", "--predicate"])
        .arg(r#"sender == "Sandbox" AND eventMessage CONTAINS "probe(""#)
        .output();
    denials_from(out)
}

/// The probe's entries in a `log show` result, at most the last 40. "No entries" is said
/// only of a query that succeeded: a failed one (no log access, a rejected predicate) has
/// empty stdout too, and must not read as Seatbelt having denied nothing.
fn denials_from(out: io::Result<Output>) -> String {
    match out {
        Ok(out) if !out.status.success() => format!(
            "could not read the sandbox log: log show {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ),
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            let lines: Vec<&str> = text.lines().filter(|l| l.contains("probe(")).collect();
            let tail = &lines[lines.len().saturating_sub(40)..];
            if tail.is_empty() {
                "no sandbox log entries for the probe in the last minute".into()
            } else {
                tail.join("\n")
            }
        }
        Err(e) => format!("could not read the sandbox log: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::denials_from;
    use std::os::unix::process::ExitStatusExt;
    use std::process::{ExitStatus, Output};

    fn output(code: i32, stdout: &str, stderr: &str) -> Output {
        let status = ExitStatus::from_raw(code << 8);
        Output { status, stdout: stdout.into(), stderr: stderr.into() }
    }

    #[test]
    fn a_failed_log_query_is_a_failure_not_an_empty_log() {
        let text = denials_from(Ok(output(1, "", "log: predicate rejected")));
        assert!(!text.contains("no sandbox log entries"), "{text}");
        assert!(text.contains("exit status: 1"), "{text}");
        assert!(text.contains("log: predicate rejected"), "{text}");
    }

    #[test]
    fn a_successful_query_with_no_entries_says_so() {
        let text = denials_from(Ok(output(0, "Timestamp               Ty Process[PID:TID]\n", "")));
        assert_eq!(text, "no sandbox log entries for the probe in the last minute");
    }

    #[test]
    fn the_probes_entries_are_listed() {
        let entry = "12:00:00.000 E  kernel[0:1] Sandbox: probe(12) deny(1) file-read-data /";
        let text = denials_from(Ok(output(0, &format!("Timestamp\n{entry}\n"), "")));
        assert_eq!(text, entry);
    }
}

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
