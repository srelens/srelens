//! The macOS memory and CPU watchdog (#713): what the host does where the
//! kernel gives it no limit (ADR, "Decision: macOS limits are host-enforced").
//!
//! [`Watchdog`] is the policy, with no system calls, so every OS tests it.
//! On Unix, `watched` applies it to a child process; only the macOS backend
//! runs it, with a sampler that reads `proc_pid_rusage`.
//!
//! The guarantee is weaker than a Job Object's or a cgroup's: the watchdog
//! sees the sidecar every [`SAMPLE_EVERY`], so it bounds sustained use, and a
//! burst between two readings can exceed the limit.

#[cfg(unix)]
use std::io;
#[cfg(unix)]
use std::process::ExitStatus;
#[cfg(unix)]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(unix)]
use std::sync::Arc;
use std::time::{Duration, Instant};

#[cfg(unix)]
use super::{Exit, LaunchError, Launched, Waiter};
use crate::sidecar::Limits;

/// How often the watchdog reads a sidecar. The host's, not the app's: it is
/// in neither [`Limits`] nor the manifest.
pub(crate) const SAMPLE_EVERY: Duration = Duration::from_millis(50);

pub(crate) const MIB: u64 = 1024 * 1024;

/// The CPU limit's floor and ceiling. The floor is the Linux backend's (a
/// 1 ms quota in a 100 ms period); a limit below it, or not a number, is held
/// to it rather than turning the throttle off.
const MIN_CPUS: f64 = 0.01;
const MAX_CPUS: f64 = 1024.0;

/// One reading of a sidecar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Usage {
    /// Its memory now, in bytes: on macOS its physical footprint, which
    /// counts compressed and swapped pages too.
    pub footprint: u64,
    /// Its CPU time so far, user and system.
    pub cpu: Duration,
}

/// What the watchdog does after a reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verdict {
    Run,
    /// Stop it for this long, then let it run again.
    Pause(Duration),
    /// Its memory is over the limit: stop it for good.
    Kill {
        measured: u64,
    },
}

/// The policy for one sidecar.
#[derive(Debug)]
pub(crate) struct Watchdog {
    memory_bytes: u64,
    cpus: f64,
    /// The previous reading's time and CPU.
    last: Option<(Instant, Duration)>,
    /// CPU used beyond the limit's rate and not yet paid back.
    debt: Duration,
}

impl Watchdog {
    pub(crate) fn new(limits: &Limits) -> Watchdog {
        let cpus = if limits.cpus.is_nan() {
            MIN_CPUS
        } else {
            limits.cpus.clamp(MIN_CPUS, MAX_CPUS)
        };
        Watchdog {
            memory_bytes: limits.memory_bytes,
            cpus,
            last: None,
            debt: Duration::ZERO,
        }
    }

    /// Judge one reading, taken at `at`.
    ///
    /// Memory first: over the limit is a kill. Then CPU, as a bandwidth
    /// controller like a cgroup's `cpu.max`: CPU used beyond `cpus` per
    /// second of wall time is a debt, and a pause of `debt / cpus` lets the
    /// limit's rate pay it back. The debt never goes below zero, so idle time
    /// banks no credit for a later burst.
    pub(crate) fn observe(&mut self, usage: &Usage, at: Instant) -> Verdict {
        if usage.footprint > self.memory_bytes {
            return Verdict::Kill {
                measured: usage.footprint,
            };
        }
        let Some((then, cpu_then)) = self.last.replace((at, usage.cpu)) else {
            return Verdict::Run;
        };
        let used = usage.cpu.saturating_sub(cpu_then);
        let allowed = at.saturating_duration_since(then).mul_f64(self.cpus);
        self.debt = self.debt.saturating_add(used).saturating_sub(allowed);
        if self.debt.is_zero() {
            Verdict::Run
        } else {
            Verdict::Pause(self.debt.div_f64(self.cpus))
        }
    }
}

/// CPU time from Mach absolute-time ticks, as `proc_pid_rusage` reports it.
/// They are nanoseconds only on Intel; Apple Silicon's timebase is 125/3. A
/// zero denominator, which the kernel does not give, is read as 1/1.
pub(crate) fn ticks_to_cpu(ticks: u64, numer: u32, denom: u32) -> Duration {
    let (numer, denom) = if denom == 0 { (1, 1) } else { (numer, denom) };
    let nanos = u128::from(ticks) * u128::from(numer) / u128::from(denom);
    Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
}

/// Reads a sidecar's [`Usage`] by its PID: the macOS backend's reads
/// `proc_pid_rusage`; the tests' are scripted.
#[cfg(unix)]
pub(crate) trait Sampler: Send + 'static {
    fn sample(&mut self, pid: u32) -> io::Result<Usage>;
}

/// The memory reader's value before the first reading and after the exit.
#[cfg(unix)]
const NO_READING: u64 = u64::MAX;

/// Why the watchdog stopped a sidecar.
#[cfg(unix)]
enum Reason {
    Memory { measured: u64, limit: u64 },
    Unmeasured(io::Error),
}

/// `child` under the watchdog: [`Launched::from_child`]'s wait task, plus a
/// reading every [`SAMPLE_EVERY`] that [`Watchdog`] judges.
///
/// The readings run in the task that waits on the child, so the child is
/// never reaped while it is being read or signalled, and its PID cannot be
/// another process's. `child` must be spawned with `kill_on_drop(true)`: if
/// the task panics, dropping the child kills the sidecar, so a failed
/// watchdog stops it rather than leaving it unwatched. A reading that fails
/// while the sidecar runs stops it too.
///
/// The [`super::Process`]'s memory reader answers the last reading, `None`
/// before the first and after the exit; it makes no system call itself.
#[cfg(unix)]
pub(crate) fn watched(
    mut child: tokio::process::Child,
    describe: impl FnOnce(io::Result<ExitStatus>) -> Exit + Send + 'static,
    limits: &Limits,
    mut sampler: impl Sampler,
) -> Result<Launched, LaunchError> {
    let (mut launched, Waiter { mut stopped, ended }) = Launched::piped(&mut child)?;
    let reading = Arc::new(AtomicU64::new(NO_READING));
    let reader = reading.clone();
    launched.process = launched
        .process
        .with_memory(move || match reader.load(Ordering::Relaxed) {
            NO_READING => None,
            bytes => Some(bytes),
        });
    let mut watchdog = Watchdog::new(limits);
    let limit = limits.memory_bytes;
    tokio::spawn(async move {
        let mut ticks = tokio::time::interval(SAMPLE_EVERY);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut reason = None;
        let status = loop {
            tokio::select! {
                status = child.wait() => break status,
                // A kill, or every handle to the process dropped.
                _ = stopped.recv() => break kill(&mut child).await,
                _ = ticks.tick() => {}
            }
            let Some(pid) = child.id() else { continue };
            let usage = match sampler.sample(pid) {
                Ok(usage) => usage,
                // It may have just exited: then that is how it ended.
                Err(e) => match child.try_wait() {
                    Ok(Some(status)) => break Ok(status),
                    _ => {
                        reason = Some(Reason::Unmeasured(e));
                        break kill(&mut child).await;
                    }
                },
            };
            reading.store(usage.footprint, Ordering::Relaxed);
            match watchdog.observe(&usage, Instant::now()) {
                Verdict::Run => {}
                Verdict::Kill { measured } => {
                    reason = Some(Reason::Memory { measured, limit });
                    break kill(&mut child).await;
                }
                Verdict::Pause(pause) => {
                    signal(pid, libc::SIGSTOP);
                    tokio::select! {
                        status = child.wait() => break status,
                        _ = stopped.recv() => break kill(&mut child).await,
                        () = tokio::time::sleep(pause) => {}
                    }
                    signal(pid, libc::SIGCONT);
                }
            }
        };
        reading.store(NO_READING, Ordering::Relaxed);
        let mut exit = describe(status);
        match reason {
            Some(Reason::Memory { measured, limit }) => {
                exit.memory_limit = true;
                exit.description = format!(
                    "was stopped at its {} MiB memory limit (srelens measured {} MiB)",
                    limit / MIB,
                    measured / MIB
                );
            }
            Some(Reason::Unmeasured(e)) => {
                exit.description = format!(
                    "was stopped because srelens could not measure its memory and CPU: {e}"
                );
            }
            None => {}
        }
        let _ = ended.send(exit);
    });
    Ok(launched)
}

#[cfg(unix)]
async fn kill(child: &mut tokio::process::Child) -> io::Result<ExitStatus> {
    let _ = child.start_kill();
    child.wait().await
}

/// Send `signal` to the sidecar: only ever to a child not yet reaped, so the
/// PID is still its own. A PID too large for `pid_t` is not sent to, since a
/// negative one would signal a process group.
#[cfg(unix)]
fn signal(pid: u32, signal: libc::c_int) {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return;
    };
    // SAFETY: kill(2) with a PID and a signal number; no memory is passed.
    unsafe { libc::kill(pid, signal) };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn watchdog(memory_mib: u64, cpus: f64) -> Watchdog {
        Watchdog::new(&Limits {
            memory_bytes: memory_mib * MIB,
            cpus,
            ..Limits::default()
        })
    }

    fn usage(footprint_mib: u64, cpu_ms: u64) -> Usage {
        Usage {
            footprint: footprint_mib * MIB,
            cpu: ms(cpu_ms),
        }
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// A pause within a microsecond of `expected`: the policy multiplies
    /// durations by floats.
    fn pause_of(verdict: Verdict, expected: Duration) -> bool {
        matches!(verdict, Verdict::Pause(d) if d.abs_diff(expected) < Duration::from_micros(1))
    }

    #[test]
    fn memory_over_the_limit_is_a_kill_with_what_was_measured() {
        let mut dog = watchdog(128, 1.0);
        let t = Instant::now();
        assert_eq!(dog.observe(&usage(128, 0), t), Verdict::Run, "at the limit");
        let over = Usage {
            footprint: 128 * MIB + 1,
            cpu: ms(0),
        };
        assert_eq!(
            dog.observe(&over, t + ms(50)),
            Verdict::Kill {
                measured: 128 * MIB + 1
            }
        );
    }

    #[test]
    fn the_first_reading_is_only_a_baseline() {
        let mut dog = watchdog(128, 0.25);
        // Ten seconds of CPU before the watchdog started is not a debt.
        assert_eq!(dog.observe(&usage(1, 10_000), Instant::now()), Verdict::Run);
    }

    #[test]
    fn cpu_within_the_rate_runs() {
        let mut dog = watchdog(128, 0.25);
        let t = Instant::now();
        dog.observe(&usage(1, 0), t);
        assert_eq!(dog.observe(&usage(1, 20), t + ms(100)), Verdict::Run);
    }

    #[test]
    fn cpu_over_the_rate_pauses_it_until_the_average_is_back_at_the_limit() {
        let mut dog = watchdog(128, 0.25);
        let t = Instant::now();
        dog.observe(&usage(1, 0), t);
        // 200 ms of CPU in 100 ms against 0.25: 25 ms allowed, 175 ms owed,
        // which 700 ms at 0.25 pays back.
        let verdict = dog.observe(&usage(1, 200), t + ms(100));
        assert!(pause_of(verdict, ms(700)), "{verdict:?}");
        // A little past the pause, having used nothing, the debt is paid.
        assert_eq!(dog.observe(&usage(1, 200), t + ms(900)), Verdict::Run);
    }

    #[test]
    fn idle_time_banks_no_credit_for_a_later_burst() {
        let mut dog = watchdog(128, 0.25);
        let t = Instant::now();
        dog.observe(&usage(1, 0), t);
        assert_eq!(dog.observe(&usage(1, 0), t + ms(10_000)), Verdict::Run);
        // Ten idle seconds earned nothing: this burst is owed in full.
        let verdict = dog.observe(&usage(1, 200), t + ms(10_100));
        assert!(pause_of(verdict, ms(700)), "{verdict:?}");
    }

    #[test]
    fn memory_is_judged_before_cpu() {
        let mut dog = watchdog(128, 0.25);
        let t = Instant::now();
        dog.observe(&usage(1, 0), t);
        assert_eq!(
            dog.observe(&usage(200, 200), t + ms(100)),
            Verdict::Kill {
                measured: 200 * MIB
            }
        );
    }

    #[test]
    fn a_cpu_limit_below_the_floor_or_not_a_number_is_held_to_the_floor() {
        for cpus in [0.0, -1.0, f64::NAN] {
            let mut dog = watchdog(128, cpus);
            let t = Instant::now();
            dog.observe(&usage(1, 0), t);
            // 11 ms of CPU in 100 ms against the 0.01 floor: 1 ms allowed,
            // 10 ms owed, which a second at 0.01 pays back.
            let verdict = dog.observe(&usage(1, 11), t + ms(100));
            assert!(pause_of(verdict, ms(1_000)), "{cpus}: {verdict:?}");
        }
    }

    #[test]
    fn mach_ticks_are_converted_by_the_timebase() {
        // Apple Silicon: 125/3, so 24 ticks are 1,000 ns.
        assert_eq!(ticks_to_cpu(24, 125, 3), Duration::from_nanos(1_000));
        // Intel: 1/1.
        assert_eq!(ticks_to_cpu(1_000, 1, 1), Duration::from_nanos(1_000));
        assert_eq!(
            ticks_to_cpu(1_000, 1, 0),
            Duration::from_nanos(1_000),
            "a zero denominator"
        );
        assert_eq!(
            ticks_to_cpu(u64::MAX, 125, 3),
            Duration::from_nanos(u64::MAX),
            "saturates"
        );
    }

    #[test]
    fn a_debt_still_owed_at_the_next_reading_carries_over() {
        let mut dog = watchdog(128, 0.25);
        let t = Instant::now();
        dog.observe(&usage(1, 0), t);
        // First reading: 200 ms of CPU in 100 ms against 0.25: 25 ms allowed, 175 ms owed.
        let verdict1 = dog.observe(&usage(1, 200), t + ms(100));
        assert!(pause_of(verdict1, ms(700)), "{verdict1:?}");
        // Second reading: 200 ms total CPU, so 0 ms new usage. Debt carries over.
        // 175 ms owed + 0 ms used = 175 ms owed, paying back takes 700 ms at 0.25.
        // But we're only 100 ms later, so debt is now 175 ms owed.
        let verdict2 = dog.observe(&usage(1, 400), t + ms(200));
        assert!(pause_of(verdict2, ms(1_400)), "debt carries: {verdict2:?}");
    }

    #[test]
    fn a_cpu_reading_lower_than_the_previous_one_becomes_the_new_baseline() {
        let mut dog = watchdog(128, 0.25);
        let t = Instant::now();
        dog.observe(&usage(1, 100), t);
        // CPU drops from 100 to 50: counts as 0 used, 50 becomes the new baseline.
        assert_eq!(dog.observe(&usage(1, 50), t + ms(100)), Verdict::Run);
        // From baseline 50, we're at 100 after 100 ms: 50 ms used, 25 allowed, 25 owed.
        let verdict = dog.observe(&usage(1, 100), t + ms(200));
        assert!(pause_of(verdict, ms(100)), "{verdict:?}");
    }

    #[test]
    fn a_cpu_limit_above_the_ceiling_or_infinite_is_held_to_the_ceiling() {
        for cpus in [2048.0, f64::INFINITY] {
            let mut dog = watchdog(128, cpus);
            let t = Instant::now();
            dog.observe(&usage(1, 0), t);
            // 102.5 s of CPU in 100 ms against ceiling 1024: 102.4 s allowed,
            // 100 ms owed, which 100 ms at 1024 CPUs pays back.
            let verdict = dog.observe(&usage(1, 102_500), t + ms(100));
            assert!(
                pause_of(verdict, Duration::from_nanos(97_656)),
                "{cpus}: {verdict:?}"
            );
        }
    }
}

#[cfg(all(test, unix))]
mod loop_tests {
    use super::*;
    use crate::sidecar::sandbox::{Exit, Launched};
    use std::io;
    use std::process::Stdio;

    /// A sampler from a closure.
    struct Fake<F>(F);

    impl<F: FnMut(u32) -> io::Result<Usage> + Send + 'static> Sampler for Fake<F> {
        fn sample(&mut self, pid: u32) -> io::Result<Usage> {
            (self.0)(pid)
        }
    }

    fn command(program: &str, args: &[&str]) -> tokio::process::Child {
        tokio::process::Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap()
    }

    fn limits() -> Limits {
        Limits {
            memory_bytes: 128 * MIB,
            cpus: 1.0,
            ..Limits::default()
        }
    }

    /// The process's state letter from `ps`: `T` stopped, `S` sleeping, `Z` a
    /// zombie; empty once it is gone.
    fn state(pid: u32) -> String {
        let out = std::process::Command::new("ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout)
            .trim()
            .chars()
            .take(1)
            .collect()
    }

    /// Its state once `ok` holds, or its last after `within`.
    async fn state_when(pid: u32, within: Duration, ok: impl Fn(&str) -> bool) -> String {
        let deadline = Instant::now() + within;
        loop {
            let now = state(pid);
            if ok(&now) || Instant::now() >= deadline {
                return now;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn exit_of(launched: &mut Launched) -> Exit {
        tokio::time::timeout(Duration::from_secs(5), launched.process.exit())
            .await
            .expect("it ended within 5 s")
    }

    /// `seconds` of CPU by the second reading and none after it: one pause of
    /// about that long at 1 CPU, then it runs again.
    fn burst_of(seconds: u64) -> Fake<impl FnMut(u32) -> io::Result<Usage> + Send + 'static> {
        let mut readings = 0u32;
        Fake(move |_| {
            readings += 1;
            let cpu = if readings < 2 {
                Duration::ZERO
            } else {
                Duration::from_secs(seconds)
            };
            Ok(Usage {
                footprint: MIB,
                cpu,
            })
        })
    }

    #[tokio::test]
    async fn memory_over_the_limit_stops_it_and_says_what_was_measured() {
        let sampler = Fake(|_| {
            Ok(Usage {
                footprint: 129 * MIB,
                cpu: Duration::ZERO,
            })
        });
        let mut launched = watched(
            command("sleep", &["30"]),
            Exit::from_status,
            &limits(),
            sampler,
        )
        .unwrap();
        let exit = exit_of(&mut launched).await;
        assert!(exit.memory_limit, "{exit:?}");
        assert_eq!(
            exit.description,
            "was stopped at its 128 MiB memory limit (srelens measured 129 MiB)"
        );
        assert_eq!(exit.signal, Some(libc::SIGKILL));
    }

    #[tokio::test]
    async fn cpu_over_the_rate_stops_it_and_then_lets_it_run_again() {
        let mut launched = watched(
            command("sleep", &["30"]),
            Exit::from_status,
            &limits(),
            burst_of(1),
        )
        .unwrap();
        let pid = launched.process.pid().unwrap();
        assert_eq!(
            state_when(pid, Duration::from_secs(2), |s| s == "T").await,
            "T",
            "stopped for its debt"
        );
        assert_eq!(
            state_when(pid, Duration::from_secs(3), |s| s != "T").await,
            "S",
            "running again once the debt is paid"
        );
        (launched.process.killer())();
        exit_of(&mut launched).await;
    }

    #[tokio::test]
    async fn a_kill_during_a_pause_stops_it() {
        // A hundred seconds of CPU: a pause far longer than the test.
        let mut launched = watched(
            command("sleep", &["30"]),
            Exit::from_status,
            &limits(),
            burst_of(100),
        )
        .unwrap();
        let pid = launched.process.pid().unwrap();
        assert_eq!(
            state_when(pid, Duration::from_secs(2), |s| s == "T").await,
            "T"
        );
        (launched.process.killer())();
        let exit = exit_of(&mut launched).await;
        assert_eq!(exit.signal, Some(libc::SIGKILL), "{exit:?}");
        assert!(!exit.memory_limit);
    }

    #[tokio::test]
    async fn a_reading_that_fails_while_it_runs_stops_it() {
        let sampler = Fake(|_| Err(io::Error::other("no reading")));
        let mut launched = watched(
            command("sleep", &["30"]),
            Exit::from_status,
            &limits(),
            sampler,
        )
        .unwrap();
        let exit = exit_of(&mut launched).await;
        assert_eq!(
            exit.description,
            "was stopped because srelens could not measure its memory and CPU: no reading"
        );
        assert_eq!(exit.signal, Some(libc::SIGKILL));
    }

    #[tokio::test]
    async fn a_reading_that_fails_because_it_has_exited_reports_its_own_exit() {
        // The shell outlives the wait task's first poll, so the first reading
        // is taken; that reading waits until the shell has exited, then fails,
        // as a reading of an ended process may.
        let sampler = Fake(|_| {
            std::thread::sleep(Duration::from_millis(300));
            Err(io::Error::other("no such process"))
        });
        let mut launched = watched(
            command("sh", &["-c", "sleep 0.1; exit 3"]),
            Exit::from_status,
            &limits(),
            sampler,
        )
        .unwrap();
        let exit = exit_of(&mut launched).await;
        assert_eq!(exit.description, "exited with status 3");
        assert_eq!(exit.code, Some(3));
    }

    #[tokio::test]
    async fn a_watchdog_that_panics_stops_the_sidecar() {
        let sampler = Fake(|_| -> io::Result<Usage> { panic!("the sampler failed") });
        let mut launched = watched(
            command("sleep", &["30"]),
            Exit::from_status,
            &limits(),
            sampler,
        )
        .unwrap();
        let pid = launched.process.pid().unwrap();
        let exit = exit_of(&mut launched).await;
        assert_eq!(exit.description, "ended, and srelens lost track of how");
        let gone = state_when(pid, Duration::from_secs(2), |s| s.is_empty() || s == "Z").await;
        assert!(gone.is_empty() || gone == "Z", "still {gone}");
    }

    #[tokio::test]
    async fn the_memory_reader_answers_the_last_reading_and_none_after_the_exit() {
        let sampler = Fake(|_| {
            Ok(Usage {
                footprint: 42 * MIB,
                cpu: Duration::ZERO,
            })
        });
        let mut launched = watched(
            command("sleep", &["30"]),
            Exit::from_status,
            &limits(),
            sampler,
        )
        .unwrap();
        let memory = launched.process.memory().expect("a reader");
        let deadline = Instant::now() + Duration::from_secs(2);
        while memory().is_none() && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(memory(), Some(42 * MIB));
        (launched.process.killer())();
        exit_of(&mut launched).await;
        assert_eq!(memory(), None, "after the exit");
    }
}
