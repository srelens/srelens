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
        if !self.enabled(record.metadata()) {
            return;
        }
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
/// them. Once per process; a second call changes nothing. If the program has
/// already installed a `log` logger of its own, ours is not installed and
/// `level` is not applied: that logger's own filtering applies. The panic
/// hook is set either way.
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

    // The logger and the `log` crate's max level are process-global, so a test
    // that installs them runs itself again as a child process, which sees this
    // variable, and reads what the child wrote to stderr.
    const CHILD: &str = "SRELENS_SDK_LOG_CHILD";

    fn stderr_of_child(test: &str, mode: &str) -> String {
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([test, "--exact", "--nocapture", "--test-threads=1"])
            .env(CHILD, mode)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&child.stderr).into_owned();
        assert!(child.status.success(), "the child failed: {stderr}");
        stderr
    }

    #[test]
    fn install_sets_the_level_the_log_macros_filter_at() {
        if let Ok(level) = std::env::var(CHILD) {
            install(level.parse().unwrap());
            log::trace!("TRACE-LINE");
            log::debug!("DEBUG-LINE");
            log::info!("INFO-LINE");
            return;
        }
        let test = "logging::tests::install_sets_the_level_the_log_macros_filter_at";
        let at_debug = stderr_of_child(test, "debug");
        let at_info = stderr_of_child(test, "info");
        // `INFO-LINE` in both shows the child ran this test at all.
        assert!(at_debug.contains("INFO-LINE"), "{at_debug}");
        assert!(at_debug.contains("DEBUG-LINE"), "{at_debug}");
        assert!(!at_debug.contains("TRACE-LINE"), "{at_debug}");
        assert!(at_info.contains("INFO-LINE"), "{at_info}");
        assert!(!at_info.contains("DEBUG-LINE"), "{at_info}");
    }

    #[test]
    fn log_drops_a_record_below_its_level_when_called_directly() {
        if std::env::var(CHILD).is_ok() {
            // Not installed, so the macros' own filter is not in play.
            let stderr = Stderr::new(LevelFilter::Info);
            for (level, text) in [(Level::Debug, "DEBUG-LINE"), (Level::Info, "INFO-LINE")] {
                let args = format_args!("{text}");
                stderr.log(
                    &Record::builder()
                        .level(level)
                        .target("s")
                        .args(args)
                        .build(),
                );
            }
            return;
        }
        let test = "logging::tests::log_drops_a_record_below_its_level_when_called_directly";
        let written = stderr_of_child(test, "direct");
        // `INFO-LINE` shows the child ran this test at all.
        assert!(written.contains("INFO-LINE"), "{written}");
        assert!(!written.contains("DEBUG-LINE"), "{written}");
    }
}
