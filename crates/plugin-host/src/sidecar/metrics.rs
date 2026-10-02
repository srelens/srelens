//! What the Inspector reads from a supervised sidecar (#575): its state, its
//! process, its memory, and how its requests have gone.
//!
//! The one way in is [`Inspect::metrics`], a snapshot. A host holds a
//! supervisor as `dyn Inspect` and reads nothing else, so the supervisor's
//! internals stay its own while other work (#573's broker; the macOS
//! watchdog's last reading, #713) changes them.
//!
//! Everything here is kept in memory and bounded: counters, the last
//! [`LATENCY_SAMPLES`] round trips, and the state. Nothing is written to disk
//! or sent anywhere.

use std::collections::VecDeque;
use std::time::{Duration, SystemTime};

use super::connection::RequestError;
use super::sandbox::Enforcement;
use super::supervisor::SidecarStatus;
use super::Limits;

/// Round trips kept for the latency figures. Older ones are dropped.
pub const LATENCY_SAMPLES: usize = 256;

/// A supervised sidecar, as the Inspector reads it.
pub trait Inspect: Send + Sync {
    fn metrics(&self) -> SidecarMetrics;
}

/// One sidecar now.
#[derive(Debug, Clone, PartialEq)]
pub struct SidecarMetrics {
    /// Where it is. Every reason in it has been redacted like its app's log.
    pub status: SidecarStatus,
    /// When the process now running came up.
    pub started_at: Option<SystemTime>,
    /// Processes launched, over the supervisor's life.
    pub launches: u64,
    /// Unexpected exits, over the supervisor's life; a restart by a person
    /// does not reset it.
    pub unexpected_exits: u64,
    /// Memory the running process uses now, when its sandbox backend can say
    /// (a cgroup on Linux). `None` when nothing is running, or on an OS whose
    /// backend does not measure it.
    pub memory_bytes: Option<u64>,
    pub limits: Limits,
    /// Who enforces [`Limits::memory_bytes`] and [`Limits::cpus`].
    pub enforcement: Enforcement,
    pub requests: RequestMetrics,
    /// The sidecar's own streams open now, and opened over its life.
    pub open_streams: usize,
    pub streams_opened: u64,
}

/// How the requests sent to a sidecar went, over the supervisor's life.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RequestMetrics {
    /// Answered with a result.
    pub answered: u64,
    /// Answered with an error, or ended with the process.
    pub failed: u64,
    /// Not answered within the request timeout, and cancelled.
    pub timed_out: u64,
    /// Refused by srelens before it was sent: at the request or stream limit,
    /// a reserved method, or the sidecar not running.
    pub refused: u64,
    /// App requests waiting for an answer now.
    pub in_flight: usize,
    /// Of the last [`LATENCY_SAMPLES`] answers, with a result or an error.
    pub latency: Latency,
}

/// Round-trip times.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Latency {
    pub samples: usize,
    pub p50: Option<Duration>,
    pub p95: Option<Duration>,
    pub max: Option<Duration>,
}

impl Latency {
    fn of(samples: &VecDeque<Duration>) -> Latency {
        let mut sorted: Vec<Duration> = samples.iter().copied().collect();
        sorted.sort_unstable();
        // The nearest-rank percentile: the smallest sample at or above that
        // share of them.
        let rank = |p: usize| {
            (!sorted.is_empty()).then(|| sorted[(sorted.len() * p).div_ceil(100).max(1) - 1])
        };
        Latency {
            samples: sorted.len(),
            p50: rank(50),
            p95: rank(95),
            max: sorted.last().copied(),
        }
    }
}

/// The counters a supervisor keeps, and the samples behind [`Latency`].
#[derive(Debug, Default)]
pub(crate) struct Recorder {
    pub(crate) launches: u64,
    pub(crate) unexpected_exits: u64,
    pub(crate) started_at: Option<SystemTime>,
    answered: u64,
    failed: u64,
    timed_out: u64,
    refused: u64,
    streams_opened: u64,
    latency: VecDeque<Duration>,
}

impl Recorder {
    /// One request's outcome, after `took`.
    pub(crate) fn request<T>(&mut self, outcome: &Result<T, RequestError>, took: Duration) {
        match outcome {
            Ok(_) => self.answered += 1,
            Err(RequestError::Failed(_) | RequestError::Ended(_)) => self.failed += 1,
            Err(RequestError::TimedOut { .. }) => self.timed_out += 1,
            Err(
                RequestError::Busy { .. }
                | RequestError::TooManyStreams { .. }
                | RequestError::Reserved { .. }
                | RequestError::Unavailable(_),
            ) => self.refused += 1,
        }
        // A round trip is an answer, with a result or an error.
        if matches!(outcome, Ok(_) | Err(RequestError::Failed(_))) {
            if self.latency.len() == LATENCY_SAMPLES {
                self.latency.pop_front();
            }
            self.latency.push_back(took);
        }
    }

    pub(crate) fn stream_opened(&mut self) {
        self.streams_opened += 1;
    }

    pub(crate) fn requests(&self, in_flight: usize) -> RequestMetrics {
        RequestMetrics {
            answered: self.answered,
            failed: self.failed,
            timed_out: self.timed_out,
            refused: self.refused,
            in_flight,
            latency: Latency::of(&self.latency),
        }
    }

    pub(crate) fn streams_opened(&self) -> u64 {
        self.streams_opened
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn latency_is_read_from_answers_only() {
        let mut recorder = Recorder::default();
        recorder.request(&Ok::<(), _>(()), ms(10));
        recorder.request::<()>(
            &Err(RequestError::Failed(super::super::protocol::RpcError::new(
                1, "no",
            ))),
            ms(30),
        );
        recorder.request::<()>(
            &Err(RequestError::TimedOut {
                method: "scan".into(),
                after: ms(30_000),
            }),
            ms(30_000),
        );
        recorder.request::<()>(&Err(RequestError::Busy { limit: 8 }), ms(0));
        recorder.request::<()>(&Err(RequestError::Unavailable("stopped".into())), ms(0));
        recorder.request::<()>(&Err(RequestError::Ended("it exited".into())), ms(5));
        let requests = recorder.requests(2);
        assert_eq!(
            (
                requests.answered,
                requests.failed,
                requests.timed_out,
                requests.refused
            ),
            (1, 2, 1, 2)
        );
        assert_eq!(requests.in_flight, 2);
        assert_eq!(requests.latency.samples, 2);
        assert_eq!(requests.latency.max, Some(ms(30)));
    }

    #[test]
    fn the_percentiles_are_nearest_rank() {
        let samples: VecDeque<Duration> = (1..=100).map(ms).collect();
        let latency = Latency::of(&samples);
        assert_eq!(latency.p50, Some(ms(50)));
        assert_eq!(latency.p95, Some(ms(95)));
        assert_eq!(latency.max, Some(ms(100)));
        let one = Latency::of(&VecDeque::from([ms(7)]));
        assert_eq!(
            (one.p50, one.p95, one.max),
            (Some(ms(7)), Some(ms(7)), Some(ms(7)))
        );
        let none = Latency::of(&VecDeque::new());
        assert_eq!((none.samples, none.p50, none.max), (0, None, None));
    }

    #[test]
    fn only_the_last_samples_are_kept() {
        let mut recorder = Recorder::default();
        for n in 0..LATENCY_SAMPLES as u64 + 10 {
            recorder.request(&Ok::<(), _>(()), ms(1000 + n));
        }
        let latency = recorder.requests(0).latency;
        assert_eq!(latency.samples, LATENCY_SAMPLES);
        assert_eq!(latency.max, Some(ms(1000 + LATENCY_SAMPLES as u64 + 9)));
        assert_eq!(recorder.requests(0).answered, LATENCY_SAMPLES as u64 + 10);
    }
}
