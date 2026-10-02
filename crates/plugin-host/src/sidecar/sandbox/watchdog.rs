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

use std::time::{Duration, Instant};

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
}
