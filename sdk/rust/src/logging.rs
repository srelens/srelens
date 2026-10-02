//! A sidecar's log is its stderr, one line per record, which srelens keeps
//! in the app's log at the level the line starts with
//! (`docs/extensions/sidecar-protocol.md`, "Logs"). A line longer than 4 KiB
//! would be dropped whole, so each is cut to fit; a panic is written as
//! `panic: …`, which srelens reads as an error.

use log::{Level, LevelFilter, Log, Metadata, Record};
use std::io::Write;

/// The longest line srelens keeps.
pub(crate) const LOG_LINE_BYTES: usize = 4096;

/// The stderr lines for one record: `LEVEL target: text`, one per line of
/// `text`, each cut to [`LOG_LINE_BYTES`].
pub(crate) fn lines(level: Level, target: &str, text: &str) -> Vec<String> {
    let mut text_lines: Vec<&str> = text.lines().collect();
    if text_lines.is_empty() {
        text_lines.push("");
    }
    text_lines
        .into_iter()
        .map(|line| cut(format!("{} {target}: {line}", level.as_str())))
        .collect()
}

/// The stderr line for a panic.
pub(crate) fn panic_line(message: &str, location: Option<&str>) -> String {
    match location {
        Some(at) => cut(format!("panic: {message} at {at}")),
        None => cut(format!("panic: {message}")),
    }
}

fn cut(mut line: String) -> String {
    if line.len() > LOG_LINE_BYTES {
        let mut end = LOG_LINE_BYTES;
        while !line.is_char_boundary(end) {
            end -= 1;
        }
        line.truncate(end);
    }
    line
}

struct Stderr {
    level: LevelFilter,
}

impl Stderr {
    fn new(level: LevelFilter) -> Stderr {
        Stderr { level }
    }
}

impl Log for Stderr {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &Record) {
        let text = record.args().to_string();
        let mut stderr = std::io::stderr().lock();
        for line in lines(record.level(), record.target(), &text) {
            let _ = writeln!(stderr, "{line}");
        }
    }

    fn flush(&self) {
        let _ = std::io::stderr().flush();
    }
}

/// Log to stderr at `level` and above, and write panics as srelens reads
/// them. Once per process; a second call changes nothing.
pub(crate) fn install(level: LevelFilter) {
    if log::set_boxed_logger(Box::new(Stderr::new(level))).is_ok() {
        log::set_max_level(level);
    }
    std::panic::set_hook(Box::new(|info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "a panic with no message".to_owned());
        let location = info.location().map(|l| l.to_string());
        let _ = writeln!(
            std::io::stderr(),
            "{}",
            panic_line(&message, location.as_deref())
        );
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_is_a_line_srelens_reads_at_its_level() {
        assert_eq!(
            lines(Level::Info, "scanner", "3 images queued"),
            ["INFO scanner: 3 images queued"]
        );
        assert_eq!(
            lines(Level::Warn, "scanner", "slow"),
            ["WARN scanner: slow"]
        );
        assert_eq!(
            lines(Level::Error, "scanner", "failed"),
            ["ERROR scanner: failed"]
        );
        assert_eq!(lines(Level::Debug, "s", "d"), ["DEBUG s: d"]);
    }

    #[test]
    fn a_record_of_several_lines_is_written_one_line_at_a_time_each_with_its_level() {
        assert_eq!(
            lines(Level::Error, "s", "first\nsecond"),
            ["ERROR s: first", "ERROR s: second"]
        );
        assert_eq!(lines(Level::Info, "s", ""), ["INFO s: "]);
    }

    #[test]
    fn a_line_is_cut_to_what_srelens_keeps_on_a_character_boundary() {
        let long = lines(Level::Info, "s", &"é".repeat(3000));
        assert_eq!(long.len(), 1);
        assert!(long[0].len() <= LOG_LINE_BYTES, "{}", long[0].len());
        assert!(long[0].len() > LOG_LINE_BYTES - 4);
        assert!(long[0].is_char_boundary(long[0].len()));
    }

    #[test]
    fn a_panic_is_a_line_srelens_reads_as_an_error() {
        assert_eq!(
            panic_line("boom", Some("src/main.rs:3:5")),
            "panic: boom at src/main.rs:3:5"
        );
        assert_eq!(panic_line("boom", None), "panic: boom");
    }

    #[test]
    fn a_record_is_enabled_at_the_level_and_above_and_not_below_it() {
        let at = |level: Level| Metadata::builder().level(level).target("s").build();
        let info = Stderr::new(LevelFilter::Info);
        assert!(info.enabled(&at(Level::Error)));
        assert!(info.enabled(&at(Level::Info)));
        assert!(!info.enabled(&at(Level::Debug)));
        let debug = Stderr::new(LevelFilter::Debug);
        assert!(debug.enabled(&at(Level::Debug)));
        assert!(!debug.enabled(&at(Level::Trace)));
        let off = Stderr::new(LevelFilter::Off);
        assert!(!off.enabled(&at(Level::Error)));
    }
}
