//! One app's log (#575): what its sidecar wrote to stderr, and what srelens
//! did about the app, at a level from trace to error. What Settings → Apps →
//! app → Logs shows, and what the supervisor's **View logs** opens.
//!
//! It is the app's own and nothing else's: srelens's core log never receives
//! a line of it, and it never receives srelens's. It is bounded — the last
//! [`LOG_LINES`] lines, each at most [`LOG_LINE_BYTES`], plus the last
//! [`RECENT_ERRORS`] errors kept apart so that a burst of chatter cannot push
//! them out — and it lives in memory only: nothing writes it to disk, and
//! nothing sends it anywhere (`docs/extensions/inspector.md`).
//!
//! **Every line is redacted on its way in** ([`redact`]): [`AppLog::push`] is
//! the only way into the buffer, so no reader can see a line that did not go
//! through it. A value the host has handed the app is scrubbed exactly; a
//! token-shaped string is scrubbed by pattern.

mod redact;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::SystemTime;

/// Lines kept per app. Older ones are dropped, and counted.
pub const LOG_LINES: usize = 1000;

/// The longest log line kept, in bytes. The supervisor drops a longer stderr
/// line before it gets here, with a warning; [`AppLog::push`] cuts any other
/// line at a character boundary, and marks it.
pub const LOG_LINE_BYTES: usize = 4096;

/// Errors kept apart from the lines, for the Inspector's "Recent errors".
pub const RECENT_ERRORS: usize = 20;

/// Values scrubbed from one app's log. The newest are kept: they are the ones
/// the host is using.
const SCRUBBED_VALUES: usize = 64;

/// How much a line matters, from trace to error. Written in lowercase on the
/// wire, as `@srelens/core` sends and reads it.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub const ALL: [LogLevel; 5] = [
        LogLevel::Trace,
        LogLevel::Debug,
        LogLevel::Info,
        LogLevel::Warn,
        LogLevel::Error,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            LogLevel::Trace => "trace",
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        }
    }
}

/// Who wrote a log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum LogSource {
    /// The app's sidecar, on stderr.
    Sidecar,
    /// srelens: a start, an exit, a restart, a refusal, a failed request.
    Host,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    /// Counts from 1 over the life of the log, so a reader can ask for what
    /// came after the last line it has.
    pub seq: u64,
    pub at: SystemTime,
    pub level: LogLevel,
    pub source: LogSource,
    /// Redacted, and at most [`LOG_LINE_BYTES`] plus the cut marker.
    pub text: String,
}

/// The level a sidecar's stderr line is written at, and its text.
///
/// A line that starts with a level — `TRACE`, `DEBUG`, `INFO`, `WARN`,
/// `WARNING` or `ERROR`, in any case, optionally in square brackets and
/// followed by a colon — is kept at that level, without it. A line that
/// starts with `FATAL` or `panic:` is an error and is kept whole, since those
/// words are part of what it says. Anything else is `info`: stderr is a
/// sidecar's log, not only its errors (`docs/extensions/sidecar-protocol.md`).
pub fn sidecar_level(line: &str) -> (LogLevel, &str) {
    let trimmed = line.trim_start();
    let (word, rest) = match trimmed.strip_prefix('[') {
        Some(inner) => match inner.split_once(']') {
            Some((word, rest)) => (word.trim(), rest),
            None => return (LogLevel::Info, line),
        },
        None => {
            let end = trimmed
                .find(|c: char| !c.is_ascii_alphabetic())
                .unwrap_or(trimmed.len());
            let (word, rest) = trimmed.split_at(end);
            // "information", "errors" and "warned" are words, not levels.
            if !(rest.is_empty() || rest.starts_with(':') || rest.starts_with(char::is_whitespace))
            {
                return (LogLevel::Info, line);
            }
            (word, rest)
        }
    };
    let level = match word.to_ascii_lowercase().as_str() {
        "trace" => LogLevel::Trace,
        "debug" => LogLevel::Debug,
        "info" => LogLevel::Info,
        "warn" | "warning" => LogLevel::Warn,
        "error" => LogLevel::Error,
        "fatal" | "panic" => return (LogLevel::Error, line),
        _ => return (LogLevel::Info, line),
    };
    let rest = rest.strip_prefix(':').unwrap_or(rest);
    (level, rest.trim_start())
}

#[derive(Default)]
struct Ring {
    lines: VecDeque<LogLine>,
    errors: VecDeque<LogLine>,
    last_seq: u64,
    dropped: u64,
    scrub: redact::Scrubber,
}

/// One app's log. Cloning it shares it: the supervisor writes to the same
/// buffer the Inspector reads, and a restarted sidecar keeps its app's lines.
#[derive(Clone, Default)]
pub struct AppLog {
    ring: Arc<Mutex<Ring>>,
}

impl std::fmt::Debug for AppLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ring = self.ring();
        f.debug_struct("AppLog")
            .field("lines", &ring.lines.len())
            .field("dropped", &ring.dropped)
            .finish()
    }
}

impl AppLog {
    pub fn new() -> AppLog {
        AppLog::default()
    }

    fn ring(&self) -> MutexGuard<'_, Ring> {
        // Nothing panics while holding it; a poisoned lock still holds a
        // consistent ring.
        self.ring.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Keep one line the sidecar wrote on stderr, at the level it starts with
    /// ([`sidecar_level`]). Bytes that are not UTF-8 are replaced, not
    /// refused: a log is for reading, and the reason a sidecar failed may be
    /// in them.
    pub fn sidecar(&self, bytes: &[u8]) {
        let text = String::from_utf8_lossy(bytes);
        let (level, text) = sidecar_level(&text);
        self.push(level, LogSource::Sidecar, text);
    }

    /// Keep one line srelens writes about the app.
    pub fn host(&self, level: LogLevel, text: &str) {
        self.push(level, LogSource::Host, text);
    }

    /// Keep one line: redacted, cut to [`LOG_LINE_BYTES`], and the oldest line
    /// dropped when the log is full. The only way into the buffer.
    pub fn push(&self, level: LogLevel, source: LogSource, text: &str) {
        let mut ring = self.ring();
        // Redacted before it is cut, so a token that straddles the cut is
        // still whole when the patterns look for it. Only the sidecar's own
        // lines follow a private key across lines: a host line can land in
        // the middle of one, and must not end its redaction.
        let mut text = match source {
            LogSource::Sidecar => ring.scrub.line(text),
            LogSource::Host => ring.scrub.text(text),
        };
        if text.len() > LOG_LINE_BYTES {
            let mut cut = LOG_LINE_BYTES;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            text.truncate(cut);
            text.push_str(" […]");
        }
        ring.last_seq += 1;
        let line = LogLine {
            seq: ring.last_seq,
            at: SystemTime::now(),
            level,
            source,
            text,
        };
        if level == LogLevel::Error {
            if ring.errors.len() == RECENT_ERRORS {
                ring.errors.pop_front();
            }
            ring.errors.push_back(line.clone());
        }
        if ring.lines.len() == LOG_LINES {
            ring.lines.pop_front();
            ring.dropped += 1;
        }
        ring.lines.push_back(line);
    }

    /// Scrub `value` from every line kept from now on: a secret, token or
    /// password the host has handed the app, or read on its behalf.
    pub fn scrub(&self, value: &str) {
        self.ring().scrub.remember(value, SCRUBBED_VALUES);
    }

    /// `text` redacted as a line would be, for text shown beside the log: a
    /// sidecar's state, whose reason can quote what the sidecar said.
    pub fn redact(&self, text: &str) -> String {
        self.ring().scrub.text(text)
    }

    /// Every line kept, oldest first.
    pub fn lines(&self) -> Vec<LogLine> {
        self.ring().lines.iter().cloned().collect()
    }

    /// The lines after `after` (a [`LogLine::seq`]) at `min` or above, oldest
    /// first.
    pub fn lines_since(&self, after: u64, min: LogLevel) -> Vec<LogLine> {
        self.ring()
            .lines
            .iter()
            .filter(|line| line.seq > after && line.level >= min)
            .cloned()
            .collect()
    }

    /// The last [`RECENT_ERRORS`] error lines, oldest first, even those the
    /// log itself has since dropped.
    pub fn recent_errors(&self) -> Vec<LogLine> {
        self.ring().errors.iter().cloned().collect()
    }

    /// Lines dropped to keep the log at [`LOG_LINES`].
    pub fn dropped(&self) -> u64 {
        self.ring().dropped
    }

    /// Lines kept now.
    pub fn len(&self) -> usize {
        self.ring().lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ring().lines.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(log: &AppLog) -> Vec<String> {
        log.lines().into_iter().map(|line| line.text).collect()
    }

    #[test]
    fn a_sidecar_line_is_kept_at_the_level_it_starts_with() {
        let cases = [
            (
                "TRACE opening the cache",
                LogLevel::Trace,
                "opening the cache",
            ),
            ("debug: 3 images queued", LogLevel::Debug, "3 images queued"),
            ("[INFO] ready", LogLevel::Info, "ready"),
            ("Warn: slow registry", LogLevel::Warn, "slow registry"),
            ("[warning] slow registry", LogLevel::Warn, "slow registry"),
            ("ERROR scan failed", LogLevel::Error, "scan failed"),
            ("  error:scan failed", LogLevel::Error, "scan failed"),
            ("ERROR", LogLevel::Error, ""),
        ];
        for (line, level, text) in cases {
            assert_eq!(sidecar_level(line), (level, text), "{line}");
        }
    }

    #[test]
    fn a_fatal_line_or_a_panic_is_an_error_and_kept_whole() {
        for line in [
            "FATAL: no database",
            "panic: runtime error: index out of range",
        ] {
            assert_eq!(sidecar_level(line), (LogLevel::Error, line));
        }
    }

    #[test]
    fn a_line_without_a_level_is_info_and_kept_whole() {
        for line in [
            "opening database",
            "information follows",
            "errors: 0",
            "warned once",
            "[scanner] ready",
            "[unclosed",
            "",
        ] {
            assert_eq!(sidecar_level(line), (LogLevel::Info, line), "{line}");
        }
    }

    #[test]
    fn lines_can_be_read_from_a_level_up_and_after_the_last_one_read() {
        let log = AppLog::new();
        log.sidecar(b"TRACE t");
        log.sidecar(b"DEBUG d");
        log.sidecar(b"INFO i");
        log.host(LogLevel::Warn, "w");
        log.sidecar(b"ERROR e");
        let warn: Vec<String> = log
            .lines_since(0, LogLevel::Warn)
            .into_iter()
            .map(|line| line.text)
            .collect();
        assert_eq!(warn, ["w", "e"]);
        let all = log.lines_since(0, LogLevel::Trace);
        assert_eq!(all.len(), 5);
        let seqs: Vec<u64> = all.iter().map(|line| line.seq).collect();
        assert_eq!(seqs, [1, 2, 3, 4, 5]);
        let newer: Vec<String> = log
            .lines_since(3, LogLevel::Trace)
            .into_iter()
            .map(|line| line.text)
            .collect();
        assert_eq!(newer, ["w", "e"]);
        assert_eq!(all[3].source, LogSource::Host);
        assert_eq!(all[4].source, LogSource::Sidecar);
    }

    #[test]
    fn the_oldest_line_goes_when_the_log_is_full_and_is_counted() {
        let log = AppLog::new();
        for n in 0..LOG_LINES + 2 {
            log.sidecar(n.to_string().as_bytes());
        }
        let lines = texts(&log);
        assert_eq!(lines.len(), LOG_LINES);
        assert_eq!(lines[0], "2");
        assert_eq!(lines[LOG_LINES - 1], (LOG_LINES + 1).to_string());
        assert_eq!(log.dropped(), 2);
        // The sequence keeps counting across what was dropped.
        assert_eq!(log.lines()[0].seq, 3);
    }

    #[test]
    fn recent_errors_outlive_the_chatter_that_pushes_them_out_of_the_log() {
        let log = AppLog::new();
        for n in 0..RECENT_ERRORS + 3 {
            log.sidecar(format!("ERROR failure {n}").as_bytes());
        }
        for _ in 0..LOG_LINES {
            log.sidecar(b"TRACE tick");
        }
        assert!(log.lines().iter().all(|line| line.level == LogLevel::Trace));
        let errors: Vec<String> = log.recent_errors().into_iter().map(|l| l.text).collect();
        assert_eq!(errors.len(), RECENT_ERRORS);
        assert_eq!(errors[0], "failure 3");
        assert_eq!(
            errors[RECENT_ERRORS - 1],
            format!("failure {}", RECENT_ERRORS + 2)
        );
    }

    #[test]
    fn a_long_line_is_cut_at_a_character_boundary_and_marked() {
        let log = AppLog::new();
        // Three-byte characters, so the limit falls inside one.
        log.sidecar("€".repeat(LOG_LINE_BYTES).as_bytes());
        let text = &texts(&log)[0];
        assert!(text.ends_with(" […]"), "{text}");
        assert!(text.len() <= LOG_LINE_BYTES + " […]".len());
    }

    #[test]
    fn bytes_that_are_not_utf8_are_kept_readable() {
        let log = AppLog::new();
        log.host(LogLevel::Info, "fine");
        log.sidecar(b"bad \xff byte");
        let line = &log.lines()[1];
        assert_eq!(line.text, "bad \u{fffd} byte");
        assert_eq!(line.source, LogSource::Sidecar);
    }

    #[test]
    fn a_clone_shares_the_log() {
        let log = AppLog::new();
        log.clone().host(LogLevel::Info, "from the supervisor");
        assert_eq!(texts(&log), ["from the supervisor"]);
    }

    #[test]
    fn no_line_reaches_the_buffer_unredacted() {
        let log = AppLog::new();
        log.scrub("hunter2-registry-password");
        log.sidecar(b"WARN login with hunter2-registry-password refused");
        log.sidecar(b"ERROR GET https://scanner:s3cr3t@registry.example/v2 -> 401");
        log.host(
            LogLevel::Error,
            "The extension answered with an error: Authorization: Bearer abc.def.ghi-123456 rejected",
        );
        log.sidecar(b"DEBUG token=ghp_0123456789abcdefghijABCDEFGHIJ012345");
        let everything = format!("{:?} {:?}", log.lines(), log.recent_errors());
        // Named by position, not printed: a test that fails must not write
        // what it planted to the CI log either.
        let planted = [
            "hunter2-registry-password",
            "s3cr3t",
            "abc.def.ghi-123456",
            "ghp_0123456789abcdefghijABCDEFGHIJ012345",
        ];
        let leaked: Vec<usize> = (0..planted.len())
            .filter(|&n| everything.contains(planted[n]))
            .collect();
        assert!(
            leaked.is_empty(),
            "planted values {leaked:?} reached the log"
        );
        // What is not secret is still there to read.
        assert!(
            everything.contains("registry.example/v2 -> 401"),
            "{everything}"
        );
        assert!(everything.contains("login with"), "{everything}");
    }

    #[test]
    fn a_private_key_written_line_by_line_never_reaches_the_buffer() {
        let log = AppLog::new();
        for line in [
            "INFO loading key",
            "-----BEGIN RSA PRIVATE KEY-----",
            "MIIEowIBAAKCAQEAu1SU1LfVLPHCozMxH2Mo4lgOEePzNm0tRgeLezV6ffAt0gun",
            "VTLw7onLRnrq0/IzW7yWR7QkrmBL7jTKEn5u+qKhbwKfBstIs+bMY2Zkp18gnTxK",
            "-----END RSA PRIVATE KEY-----",
            "INFO key loaded",
        ] {
            log.sidecar(line.as_bytes());
        }
        let lines = texts(&log);
        assert_eq!(lines[0], "loading key");
        assert_eq!(lines[5], "key loaded");
        let everything = lines.join("\n");
        assert!(!everything.contains("MIIEow"), "{everything}");
        assert!(!everything.contains("VTLw7on"), "{everything}");
    }

    /// The supervisor writes its lines while the sidecar's stderr is read on
    /// another task, so one can land in the middle of a key. It must not end
    /// the key's redaction: only the sidecar's own lines decide that.
    #[test]
    fn a_host_line_in_the_middle_of_a_private_key_does_not_end_its_redaction() {
        let log = AppLog::new();
        log.sidecar(b"-----BEGIN RSA PRIVATE KEY-----");
        log.host(
            LogLevel::Error,
            "`scan` failed: The extension did not answer `scan` within 30 s",
        );
        log.sidecar(b"MIIEowIBAAKCAQEAu1SU1LfVLPHCozMxH2Mo4lgOEePzNm0tRgeLezV6ffAt0gun");
        log.sidecar(b"-----END RSA PRIVATE KEY-----");
        log.sidecar(b"INFO key loaded");
        let lines = texts(&log);
        assert!(!lines.join("\n").contains("MIIEow"), "{lines:?}");
        assert_eq!(
            lines[1],
            "`scan` failed: The extension did not answer `scan` within 30 s"
        );
        assert_eq!(lines[4], "key loaded");
    }

    #[test]
    fn text_shown_beside_the_log_is_redacted_the_same_way() {
        let log = AppLog::new();
        log.scrub("p4ssw0rd-value");
        let reason = log.redact(
            "The extension reported itself unhealthy (login p4ssw0rd-value, password=other), so srelens stopped it",
        );
        assert!(!reason.contains("p4ssw0rd-value"), "{reason}");
        assert!(!reason.contains("other"), "{reason}");
        assert!(reason.contains("so srelens stopped it"), "{reason}");
    }
}
