//! The recent lines of one sidecar's log: what it wrote to stderr, and what
//! the supervisor did to it. What "View logs" shows when a sidecar has been
//! disabled. Per-app log storage, levels and the Inspector are #575; this is
//! the seam it replaces.

use std::collections::VecDeque;
use std::time::SystemTime;

/// Lines kept per sidecar. Older ones are dropped.
pub const LOG_LINES: usize = 1000;

/// The longest log line kept, in bytes. A longer one is cut at a character
/// boundary and marked.
pub const LOG_LINE_BYTES: usize = 4096;

/// Who wrote a log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogSource {
    /// The sidecar, on stderr.
    Sidecar,
    /// The supervisor: a start, an exit, a restart, a refusal.
    Host,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    pub at: SystemTime,
    pub source: LogSource,
    pub text: String,
}

#[derive(Debug, Default)]
pub(crate) struct LogTail {
    lines: VecDeque<LogLine>,
}

impl LogTail {
    /// Keep one line. Bytes that are not UTF-8 are replaced, not refused: a
    /// log is for reading, and the reason a sidecar failed may be in them.
    pub(crate) fn push(&mut self, source: LogSource, bytes: &[u8]) {
        let mut text = String::from_utf8_lossy(bytes).into_owned();
        if text.len() > LOG_LINE_BYTES {
            let mut cut = LOG_LINE_BYTES;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            text.truncate(cut);
            text.push_str(" […]");
        }
        if self.lines.len() == LOG_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(LogLine {
            at: SystemTime::now(),
            source,
            text,
        });
    }

    pub(crate) fn lines(&self) -> Vec<LogLine> {
        self.lines.iter().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_oldest_line_goes_when_the_tail_is_full() {
        let mut tail = LogTail::default();
        for n in 0..LOG_LINES + 2 {
            tail.push(LogSource::Sidecar, n.to_string().as_bytes());
        }
        let lines = tail.lines();
        assert_eq!(lines.len(), LOG_LINES);
        assert_eq!(lines[0].text, "2");
        assert_eq!(lines[LOG_LINES - 1].text, (LOG_LINES + 1).to_string());
    }

    #[test]
    fn a_long_line_is_cut_at_a_character_boundary_and_marked() {
        let mut tail = LogTail::default();
        // Three-byte characters, so the limit falls inside one.
        let long = "€".repeat(LOG_LINE_BYTES);
        tail.push(LogSource::Sidecar, long.as_bytes());
        let text = &tail.lines()[0].text;
        assert!(text.ends_with(" […]"), "{text}");
        assert!(text.len() <= LOG_LINE_BYTES + " […]".len());
    }

    #[test]
    fn bytes_that_are_not_utf8_are_kept_readable() {
        let mut tail = LogTail::default();
        tail.push(LogSource::Host, b"bad \xff byte");
        let line = &tail.lines()[0];
        assert_eq!(line.text, "bad \u{fffd} byte");
        assert_eq!(line.source, LogSource::Host);
    }
}
