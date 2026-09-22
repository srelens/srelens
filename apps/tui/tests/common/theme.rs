//! The one way a TUI test changes the active theme, and the lock a test holds
//! while it asserts on anything the theme decides.
//!
//! The active theme is one index shared by every test in a binary, and the
//! runner executes those tests on parallel threads. `lock()` takes this
//! binary's theme lock for as long as the guard lives, and the guard puts the
//! default theme back before it releases the lock. So a test holding the
//! guard sees the default theme until it switches it, and its switch never
//! reaches a sibling (#676).
//!
//! `theme_guard_tests.rs` fails the build of any test that writes the theme
//! some other way.
//!
//! Take this lock last. `new_app` takes it, so a test that holds it and then
//! waits on another lock — the environment, `isolate_ai_settings` — deadlocks
//! against a test that holds that lock and builds an app.

use std::cell::Cell;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use srelens_tui::app::App;
use srelens_tui::commands::ResourceKind;
use srelens_tui::event::AppEvent;
use srelens_tui::theme::{Theme, ThemePalette};
use tokio::sync::mpsc::UnboundedSender;

/// Index of the theme a test sees when nobody has switched it.
const DEFAULT: usize = 0;

static LOCK: Mutex<()> = Mutex::new(());

thread_local! {
    /// Whether this thread already holds `LOCK`, so a helper that takes the
    /// guard inside a test that holds it does not wait on itself.
    static HELD: Cell<bool> = const { Cell::new(false) };
}

/// Holds the theme lock; restores the default theme on drop.
///
/// It is a plain `std` lock, so it works in `#[test]` and `#[tokio::test]`
/// alike: each test runs on its own thread, and waiting blocks only that test.
/// Only the outermost guard on a thread owns the lock; an inner one is a
/// no-op, so dropping it keeps the outer test's theme.
pub struct ThemeGuard {
    lock: Option<MutexGuard<'static, ()>>,
}

/// Wait for the theme, then hold it until the guard drops.
pub fn lock() -> ThemeGuard {
    if HELD.with(Cell::get) {
        return ThemeGuard { lock: None };
    }
    // A test that panics while holding the lock poisons it; the next test
    // still wants the lock, not the panic.
    let lock = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    HELD.with(|held| held.set(true));
    ThemeGuard { lock: Some(lock) }
}

impl ThemeGuard {
    pub fn set_by_name(&mut self, name: &str) -> Option<&'static ThemePalette> {
        Theme::set_theme_by_name(name)
    }

    pub fn set_by_index(&mut self, index: usize) -> bool {
        Theme::set_theme_by_index(index)
    }
}

impl Drop for ThemeGuard {
    fn drop(&mut self) {
        // Runs before the fields drop, so the lock is still held here.
        if self.lock.is_some() {
            Theme::set_theme_by_index(DEFAULT);
            HELD.with(|held| held.set(false));
        }
    }
}

/// `App::new`, holding the guard. `App::new` applies the theme named in the
/// AI settings it loads, so it writes the theme too; this puts back whatever
/// was active before, which inside a test that switched the theme is that
/// test's choice.
pub async fn new_app(
    initial_context: Option<String>,
    initial_namespace: Option<String>,
    all_namespaces: bool,
    initial_resource: Option<ResourceKind>,
    kubeconfig_paths: Vec<PathBuf>,
    event_tx: UnboundedSender<AppEvent>,
) -> Result<App, String> {
    let _theme = lock();
    let before = Theme::active_index();
    let app = App::new(
        initial_context,
        initial_namespace,
        all_namespaces,
        initial_resource,
        kubeconfig_paths,
        event_tx,
    )
    .await;
    Theme::set_theme_by_index(before);
    app
}
