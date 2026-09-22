//! The environment is one table shared by every test in a binary, and the
//! runner executes those tests on parallel threads. A test that writes a
//! variable without holding the binary's lock can repoint it under a sibling
//! mid-run. That is how `feature_banner_modal_interactive_navigation_toggle_and_jump`
//! came to save to a writable path when it expected a write failure — and, when
//! the variable had been cleared instead, to write the developer's real
//! `tui.json` (#671).

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

/// Always present, on every platform, so "the value from before" is real.
const PRESENT: &str = "PATH";
const FRESH: &str = "SRELENS_ENV_GUARD_TEST_FRESH";
const HELD: &str = "SRELENS_ENV_GUARD_TEST_HELD";

#[test]
fn the_guard_restores_every_variable_it_touched_when_dropped() {
    let before = std::env::var_os(PRESENT).expect("PATH is set");
    assert_eq!(std::env::var_os(FRESH), None);
    {
        let mut env = common::env::lock();
        env.set(PRESENT, "first");
        // A second write must not replace the saved original with "first".
        env.set(PRESENT, "second");
        env.set(FRESH, "value");
        assert!(std::env::var_os(PRESENT).unwrap() == "second");
        assert_eq!(std::env::var_os(FRESH).unwrap(), "value");
    }
    // `assert!`, not `assert_eq!`: a failure should not print all of PATH.
    assert!(
        std::env::var_os(PRESENT) == Some(before.clone()),
        "an overwritten variable gets its original value back"
    );
    assert_eq!(
        std::env::var_os(FRESH),
        None,
        "a new variable is removed again"
    );

    {
        let mut env = common::env::lock();
        env.remove(PRESENT);
        assert_eq!(std::env::var_os(PRESENT), None);
    }
    assert!(
        std::env::var_os(PRESENT) == Some(before),
        "a removed variable comes back"
    );
}

#[test]
fn a_second_test_waits_until_the_first_has_restored_the_environment() {
    let mut env = common::env::lock();
    env.set(HELD, "owned by the first test");
    let second = std::thread::spawn(|| {
        let _env = common::env::lock();
        std::env::var_os(HELD)
    });
    // Long enough for the second thread to reach the lock and, without one,
    // read the first test's value.
    std::thread::sleep(Duration::from_millis(200));
    drop(env);
    assert_eq!(
        second.join().unwrap(),
        None,
        "the second test saw a variable the first test still owned"
    );
}

/// Every `.rs` file under `dir`, recursively.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("the tests directory is readable") {
        let path = entry.expect("a directory entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn every_env_write_in_the_tui_tests_goes_through_the_shared_guard() {
    let tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let guard = tests.join("common").join("env.rs");
    // Spelled without the call parenthesis so this file does not match itself.
    let writes = ["set_var", "remove_var"].map(|name| format!("{name}("));

    let mut files = Vec::new();
    rust_files(&tests, &mut files);
    let mut offenders = Vec::new();
    for file in files.iter().filter(|f| **f != guard) {
        let source = std::fs::read_to_string(file).expect("a test source file");
        for (index, line) in source.lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            if writes.iter().any(|w| line.contains(w.as_str())) {
                let relative = file.strip_prefix(&tests).unwrap_or(file);
                offenders.push(format!("{}:{}", relative.display(), index + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "write environment variables through common::env::lock() so the \
         write holds the binary's lock and is undone when the test ends:\n  {}",
        offenders.join("\n  ")
    );
}
