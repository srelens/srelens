//! The active theme is one index shared by every test in a binary, and the
//! runner executes those tests on parallel threads. A test that switches it
//! without holding the binary's theme lock repaints a sibling mid-assertion:
//! `test_workloads_rebuild_aggregation_and_status_verdicts` compared two styles
//! read a moment apart, and the node-inspector test saw nord's grey where it
//! had set solarized-dark's (#676).

mod common;

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use srelens_tui::theme::Theme;

/// The palette every test expects when it holds the guard.
const DEFAULT: usize = 0;

#[test]
fn the_guard_restores_the_default_theme_when_dropped() {
    {
        let mut theme = common::theme::lock();
        assert!(theme.set_by_name("nord").is_some());
        assert_eq!(Theme::active_palette().name, "nord");
    }
    assert_eq!(Theme::active_index(), DEFAULT);
}

#[test]
fn a_test_that_panics_under_the_guard_restores_the_theme_and_frees_the_lock() {
    let result = std::panic::catch_unwind(|| {
        let mut theme = common::theme::lock();
        theme.set_by_name("dracula");
        panic!("a failed assertion while the theme is switched");
    });
    assert!(result.is_err());
    assert_eq!(Theme::active_index(), DEFAULT);
    // Poisoned by the panic; the next test still wants the lock.
    let _theme = common::theme::lock();
}

/// A test holding the guard calls helpers, `common::app_with` among them,
/// that take it again. The inner guard must neither wait on the outer one nor
/// undo the outer test's switch when it drops.
#[test]
fn a_test_already_holding_the_guard_can_take_it_again() {
    let (done, finished) = mpsc::channel();
    // On its own thread, so a deadlock fails this test instead of hanging it.
    std::thread::spawn(move || {
        let mut outer = common::theme::lock();
        outer.set_by_name("gruvbox-dark");
        drop(common::theme::lock());
        let after_inner = Theme::active_palette().name;
        drop(outer);
        done.send(after_inner).unwrap();
    });
    let after_inner = finished
        .recv_timeout(Duration::from_secs(5))
        .expect("taking the guard a second time on one thread deadlocked");
    assert_eq!(
        after_inner, "gruvbox-dark",
        "the inner guard reset the outer test's theme"
    );
}

#[test]
fn a_second_test_waits_until_the_first_has_restored_the_theme() {
    let mut theme = common::theme::lock();
    theme.set_by_name("solarized-dark");
    let (attempting, attempt_started) = mpsc::channel();
    let (acquired, acquisition) = mpsc::channel();
    let second = std::thread::spawn(move || {
        attempting.send(()).unwrap();
        let _theme = common::theme::lock();
        acquired.send(Theme::active_index()).unwrap();
    });
    // Wait for the second thread to reach the lock before dropping the
    // guard. If it only starts afterwards, the test proves nothing.
    attempt_started
        .recv_timeout(Duration::from_secs(5))
        .expect("the second thread never began its lock attempt");
    assert!(
        acquisition
            .recv_timeout(Duration::from_millis(200))
            .is_err(),
        "the second test took the theme while the first test still held it"
    );
    drop(theme);
    assert_eq!(
        acquisition
            .recv_timeout(Duration::from_secs(5))
            .expect("the second thread never took the theme after it was released"),
        DEFAULT,
        "the second test saw a palette the first test still owned"
    );
    second.join().unwrap();
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

/// A test can also switch the theme through the app, by pressing keys in the
/// theme picker or running `:theme`. No scan can see that, so those tests
/// hold `common::theme::lock()` by hand.
#[test]
fn every_theme_write_in_the_tui_tests_goes_through_the_shared_guard() {
    let tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let guard = tests.join("common").join("theme.rs");
    // Spelled in pieces so this file does not match itself. `App::new` is a
    // writer: it applies the theme its AI settings name.
    let writes = [format!("{}_by_", "set_theme"), format!("{}::new(", "App")];

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
        "switch the theme through common::theme::lock() and build apps with \
         common::theme::new_app, so the write holds the binary's theme lock \
         and is undone when the test ends:\n  {}",
        offenders.join("\n  ")
    );
}
