//! A stderr logger for headless run modes.
//!
//! The GUI installs `tauri_plugin_log`. `--mcp-stdio` and `--mcp-http` never
//! boot Tauri, so without this the `log` facade drops every record — including
//! the durability warnings from `crates/registry/src/durable.rs` that say a
//! successful settings or extension save may not survive a power loss.
//!
//! Stderr only: on `--mcp-stdio`, stdout is the JSON-RPC channel and must stay
//! parseable.

use log::{Level, LevelFilter, Log, Metadata, Record, SetLoggerError};

struct StderrLogger;

impl Log for StderrLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= Level::Warn
    }

    fn log(&self, record: &Record<'_>) {
        if self.enabled(record.metadata()) {
            eprintln!("{}: {}", record.level(), record.args());
        }
    }

    fn flush(&self) {}
}

static LOGGER: StderrLogger = StderrLogger;

/// Install the stderr logger for this process. Safe to call more than once;
/// later calls are ignored once a logger is already set.
pub fn init() {
    let _ = try_init();
}

fn try_init() -> Result<(), SetLoggerError> {
    log::set_logger(&LOGGER).map(|()| log::set_max_level(LevelFilter::Warn))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warns_and_errors_are_enabled_info_is_not() {
        let warn = Metadata::builder().level(Level::Warn).target("t").build();
        let info = Metadata::builder().level(Level::Info).target("t").build();
        assert!(LOGGER.enabled(&warn));
        assert!(!LOGGER.enabled(&info));
    }
}
