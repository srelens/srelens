//! What a sidecar may use, and how the supervisor paces its restarts. Host
//! policy: nothing here comes from the app, and an app cannot raise any of it.

use std::time::Duration;

/// The limits one sidecar runs under.
#[derive(Debug, Clone, PartialEq)]
pub struct Limits {
    /// How long the host waits for the answer to one request. The request is
    /// cancelled when it runs out. Default 30 s (#572).
    pub request_timeout: Duration,
    /// Requests in flight at once, streams being opened included; the next one
    /// is refused, not queued. Default 8 (#572).
    pub max_concurrent_requests: usize,
    /// Streams open at once; the next open is refused. Default 5 (#572).
    pub max_streams: usize,
    /// The sidecar's memory, enforced by the sandbox backend: a Job Object on
    /// Windows, a cgroup on Linux. Default 256 MiB (#572).
    pub memory_bytes: u64,
    /// CPU time, as a number of CPUs, enforced like the memory. Default 1.
    /// #572 names no default; this one is the supervisor's.
    pub cpus: f64,
    /// What the app's data directory, the one path it may write (#573), may
    /// hold: files, directories and links, by the larger of each file's length
    /// and what the filesystem allocated to it. Default 1 GiB, room for
    /// Trivy's vulnerability database, which is what #573 names.
    pub data_bytes: u64,
    /// How many files, directories and links it may hold. Default 100,000.
    /// It bounds what one measurement costs the host, and a directory of tiny
    /// files uses disk its byte count does not show.
    pub data_entries: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            request_timeout: Duration::from_secs(30),
            max_concurrent_requests: 8,
            max_streams: 5,
            memory_bytes: 256 * 1024 * 1024,
            cpus: 1.0,
            data_bytes: 1024 * 1024 * 1024,
            data_entries: 100_000,
        }
    }
}

/// How the supervisor keeps a sidecar running.
#[derive(Debug, Clone, PartialEq)]
pub struct Policy {
    /// The wait before each restart after an unexpected exit, in order. One
    /// exit past the last is the end: the sidecar is disabled until a person
    /// restarts it. Default 1 s, 5 s, 30 s (#572).
    pub backoff: Vec<Duration>,
    /// How long a sidecar must have run for its next exit to start the
    /// sequence again at the first wait, instead of taking the next one.
    /// Default 10 minutes, chosen here: without one, a sidecar that ran for a
    /// week and then crashed three times in a month would be disabled.
    pub backoff_reset_after: Duration,
    /// How often a running sidecar is asked `health`. Default 30 s.
    pub health_interval: Duration,
    /// How long it has to answer before it is taken as hung, stopped and
    /// restarted. Default 10 s.
    pub health_timeout: Duration,
    /// On a stop: how long `deactivate` and `shutdown` each get, and then how
    /// long the process gets to exit before it is killed. Default 5 s.
    pub shutdown_grace: Duration,
    /// How often a running sidecar's data directory is measured against
    /// [`Limits::data_bytes`] and [`Limits::data_entries`]. Default 2 s. What it
    /// writes between two measurements can overshoot the limit; see
    /// `docs/extensions/sidecar-protocol.md`, "Data directory".
    pub data_check_interval: Duration,
}

impl Default for Policy {
    fn default() -> Self {
        Policy {
            backoff: vec![
                Duration::from_secs(1),
                Duration::from_secs(5),
                Duration::from_secs(30),
            ],
            backoff_reset_after: Duration::from_secs(10 * 60),
            health_interval: Duration::from_secs(30),
            health_timeout: Duration::from_secs(10),
            shutdown_grace: Duration::from_secs(5),
            data_check_interval: Duration::from_secs(2),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_the_ones_572_names() {
        let limits = Limits::default();
        assert_eq!(limits.request_timeout, Duration::from_secs(30));
        assert_eq!(limits.max_concurrent_requests, 8);
        assert_eq!(limits.max_streams, 5);
        assert_eq!(limits.memory_bytes, 268_435_456);
        // #573's.
        assert_eq!(limits.data_bytes, 1 << 30);
        assert_eq!(limits.data_entries, 100_000);
        assert_eq!(
            Policy::default().data_check_interval,
            Duration::from_secs(2)
        );
        let backoff: Vec<u64> = Policy::default()
            .backoff
            .iter()
            .map(Duration::as_secs)
            .collect();
        assert_eq!(backoff, [1, 5, 30]);
    }
}
