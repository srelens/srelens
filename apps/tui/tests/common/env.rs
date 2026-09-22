//! The one way a TUI test writes a process environment variable.
//!
//! The environment is shared by every test in a binary, and the runner
//! executes those tests on parallel threads. `lock()` takes this binary's
//! environment lock for as long as the guard lives. Every variable written
//! through the guard is put back to its previous value when the guard drops,
//! and that happens before the lock is released. A sibling test never sees a
//! half-finished write, and a developer's own exported value survives the run.
//!
//! `env_guard_tests.rs` fails the build of any test that writes the
//! environment some other way (#671).

use std::ffi::{OsStr, OsString};
use std::sync::{Mutex, MutexGuard};

static LOCK: Mutex<()> = Mutex::new(());

/// Holds the environment lock; restores what it changed on drop.
///
/// It is a plain `std` lock, so it works in `#[test]` and `#[tokio::test]`
/// alike: each test runs on its own thread, and waiting blocks only that test.
pub struct EnvGuard {
    /// First value of each variable this guard touched, in touch order.
    saved: Vec<(&'static str, Option<OsString>)>,
    _lock: MutexGuard<'static, ()>,
}

/// Wait for the environment, then hold it until the guard drops.
pub fn lock() -> EnvGuard {
    // A test that panics while holding the lock poisons it; the next test
    // still wants the lock, not the panic.
    let lock = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    EnvGuard {
        saved: Vec::new(),
        _lock: lock,
    }
}

impl EnvGuard {
    pub fn set(&mut self, name: &'static str, value: impl AsRef<OsStr>) {
        self.save(name);
        std::env::set_var(name, value);
    }

    pub fn remove(&mut self, name: &'static str) {
        self.save(name);
        std::env::remove_var(name);
    }

    fn save(&mut self, name: &'static str) {
        if !self.saved.iter().any(|(saved, _)| *saved == name) {
            self.saved.push((name, std::env::var_os(name)));
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        // Runs before the fields drop, so the lock is still held here.
        for (name, previous) in self.saved.drain(..).rev() {
            match previous {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }
}

/// Keep both configuration files private for the full lifetime of a test's App.
/// Construct this before the App so it is dropped after settings-sensitive work.
pub fn isolate_settings() -> SettingsGuard {
    let mut env = lock();
    let dir = tempfile::tempdir().expect("scratch configuration directory");
    env.set("SRELENS_AI_SETTINGS_PATH", dir.path().join("ai_settings.json"));
    env.set("SRELENS_TUI_CONFIG_PATH", dir.path().join("tui.json"));
    SettingsGuard {
        _env: env,
        _dir: dir,
    }
}

pub struct SettingsGuard {
    // Restore the environment before deleting the files it pointed at.
    _env: EnvGuard,
    _dir: tempfile::TempDir,
}
