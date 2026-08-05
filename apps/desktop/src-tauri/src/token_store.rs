//! Token storage preferring the OS keychain, falling back to a 0600 file.
//! Linux without a Secret Service (headless boxes, minimal WMs) has no
//! keychain at all; refusing to run there would strand those users, so we fall
//! back and tell them plainly in Settings.
//!
//! The store does **not** decide once at startup by probing whether a
//! keychain "looks" available. On the sync-secret-service backend,
//! `keyring::Entry::new` only builds an in-memory attribute map — it never
//! dials D-Bus, so on a headless host it happily returns `Ok`, and the first
//! real failure only surfaces later, inside `get_password`/`set_password`.
//! Deciding at construction time would report "keychain" right up until an
//! operation panics. Instead, [`ResilientTokenStore`] attempts the keychain
//! on every call and falls back to the file the moment an operation genuinely
//! fails, remembering that for the rest of the process so Settings reports
//! what's actually serving rather than a one-time guess.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use srelens_mcp::auth::{FileTokenStore, Token, TokenStore};

const SERVICE: &str = "srelens";
const ACCOUNT: &str = "mcp-http-token";

/// The raw keychain operations, factored out so tests can inject a stub that
/// deterministically fails — the only way to exercise the fallback branch
/// without a real (or deliberately broken) OS keychain.
trait KeychainBackend: Send + Sync {
    fn get_password(&self) -> Result<String, keyring::Error>;
    fn set_password(&self, value: &str) -> Result<(), keyring::Error>;
    fn delete_credential(&self) -> Result<(), keyring::Error>;
}

/// The real backend: a fresh `keyring::Entry` per call (entries are cheap,
/// stateless handles — the actual connection happens inside each operation).
struct RealKeychain;

impl KeychainBackend for RealKeychain {
    fn get_password(&self) -> Result<String, keyring::Error> {
        keyring::Entry::new(SERVICE, ACCOUNT)?.get_password()
    }

    fn set_password(&self, value: &str) -> Result<(), keyring::Error> {
        keyring::Entry::new(SERVICE, ACCOUNT)?.set_password(value)
    }

    fn delete_credential(&self) -> Result<(), keyring::Error> {
        keyring::Entry::new(SERVICE, ACCOUNT)?.delete_credential()
    }
}

/// Prefers the OS keychain; falls back to a 0600 file the first time a
/// keychain operation genuinely fails, and stays on the file for the rest of
/// this process's lifetime (retrying per-call would re-pay a slow/failing
/// D-Bus dial on every load/save, and would let a load silently miss a
/// file-stored token by checking an empty keychain instead).
pub struct ResilientTokenStore {
    keychain: Box<dyn KeychainBackend>,
    file: FileTokenStore,
    file_fallback: AtomicBool,
}

impl ResilientTokenStore {
    fn new(fallback: PathBuf) -> Self {
        Self::with_backend(Box::new(RealKeychain), fallback)
    }

    fn with_backend(keychain: Box<dyn KeychainBackend>, fallback: PathBuf) -> Self {
        Self {
            keychain,
            file: FileTokenStore::new(fallback),
            file_fallback: AtomicBool::new(false),
        }
    }

    /// The backend actually serving right now: `"keychain"` or `"file"`.
    /// Reflects observed reality (flips permanently to `"file"` the first
    /// time a keychain operation fails), not a value guessed at construction.
    pub fn current_backend(&self) -> &'static str {
        if self.file_fallback.load(Ordering::Relaxed) {
            "file"
        } else {
            "keychain"
        }
    }
}

impl TokenStore for ResilientTokenStore {
    fn load(&self) -> Option<Token> {
        if self.file_fallback.load(Ordering::Relaxed) {
            return self.file.load();
        }
        match self.keychain.get_password() {
            Ok(s) => Token::from_hex(&s),
            // The keychain works; there's just nothing stored yet. Must NOT
            // fall back here — that would silently prefer a stale file token
            // over a genuinely empty keychain.
            Err(keyring::Error::NoEntry) => None,
            Err(_) => {
                self.file_fallback.store(true, Ordering::Relaxed);
                self.file.load()
            }
        }
    }

    fn save(&self, t: &Token) -> std::io::Result<()> {
        if self.file_fallback.load(Ordering::Relaxed) {
            return self.file.save(t);
        }
        match self.keychain.set_password(t.as_str()) {
            Ok(()) => Ok(()),
            // Never propagate the keychain error up as a hard failure when
            // the file write succeeds — that's what turned an unavailable
            // D-Bus session into a process-killing panic in the caller.
            Err(_) => {
                self.file_fallback.store(true, Ordering::Relaxed);
                // Best-effort: drop whatever entry the keychain still holds.
                // `load()` prefers the keychain, so leaving a stale entry
                // behind would let a later process (with a working keychain)
                // read the OLD token in preference to the file written just
                // below — silently reinstating a token this save replaced.
                let _ = self.keychain.delete_credential();
                self.file.save(t)
            }
        }
    }

    fn clear(&self) -> std::io::Result<()> {
        // Clear both backends: a revoke must not leave a stale token behind
        // in whichever one wasn't the active one this run (e.g. an earlier
        // process saved to the keychain; this one has been in file fallback
        // since startup, or vice versa). The keychain half is best-effort —
        // an entry that's already absent, or a keychain that's unreachable,
        // isn't a reason to fail the revoke when the file half succeeds.
        let _ = self.keychain.delete_credential();
        self.file.clear()
    }
}

/// Returns a store that prefers the OS keychain, falling back to `fallback`
/// (a 0600 file) the first time a keychain operation genuinely fails. Always
/// usable — construction itself cannot fail.
pub fn keychain_or_file(fallback: PathBuf) -> Arc<ResilientTokenStore> {
    Arc::new(ResilientTokenStore::new(fallback))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Always fails with a non-`NoEntry` error, so tests can exercise the
    /// fallback path deterministically without a broken real keychain.
    struct AlwaysFails;

    impl KeychainBackend for AlwaysFails {
        fn get_password(&self) -> Result<String, keyring::Error> {
            Err(keyring::Error::NoStorageAccess(Box::new(std::io::Error::other(
                "stub: no keychain in this test",
            ))))
        }
        fn set_password(&self, _value: &str) -> Result<(), keyring::Error> {
            Err(keyring::Error::NoStorageAccess(Box::new(std::io::Error::other(
                "stub: no keychain in this test",
            ))))
        }
        fn delete_credential(&self) -> Result<(), keyring::Error> {
            Err(keyring::Error::NoStorageAccess(Box::new(std::io::Error::other(
                "stub: no keychain in this test",
            ))))
        }
    }

    /// Always reports "nothing stored", the way a *working* keychain does
    /// before any token has been saved.
    struct EmptyButWorking;

    impl KeychainBackend for EmptyButWorking {
        fn get_password(&self) -> Result<String, keyring::Error> {
            Err(keyring::Error::NoEntry)
        }
        fn set_password(&self, _value: &str) -> Result<(), keyring::Error> {
            Ok(())
        }
        fn delete_credential(&self) -> Result<(), keyring::Error> {
            Err(keyring::Error::NoEntry)
        }
    }

    /// Records whether it was ever asked to store/load a token, so tests can
    /// exercise a genuinely working (in-memory) keychain path.
    struct RecordingWorking {
        stored: Mutex<Option<String>>,
    }

    impl KeychainBackend for RecordingWorking {
        fn get_password(&self) -> Result<String, keyring::Error> {
            self.stored.lock().unwrap().clone().ok_or(keyring::Error::NoEntry)
        }
        fn set_password(&self, value: &str) -> Result<(), keyring::Error> {
            *self.stored.lock().unwrap() = Some(value.to_string());
            Ok(())
        }
        fn delete_credential(&self) -> Result<(), keyring::Error> {
            *self.stored.lock().unwrap() = None;
            Ok(())
        }
    }

    /// A keychain that is readable-but-empty and fails every write, recording
    /// whether it was asked to delete its entry.
    struct FailingWrites(Arc<Mutex<bool>>);

    impl KeychainBackend for FailingWrites {
        fn get_password(&self) -> Result<String, keyring::Error> {
            Err(keyring::Error::NoEntry)
        }
        fn set_password(&self, _value: &str) -> Result<(), keyring::Error> {
            Err(keyring::Error::NoStorageAccess(Box::new(std::io::Error::other(
                "stub: keychain write failed",
            ))))
        }
        fn delete_credential(&self) -> Result<(), keyring::Error> {
            *self.0.lock().unwrap() = true;
            Ok(())
        }
    }

    /// The stale-token hazard: a keychain that fails *this* save (a transient
    /// D-Bus hiccup, a locked login keychain) sends the token to the file, but
    /// any entry already in the keychain survives. The next process starts
    /// with a healthy keychain, `load()` prefers it, and the OLD token comes
    /// back — so a rotate looks undone and a token the user believed replaced
    /// still authenticates. `clear()` already clears both backends; `save()`
    /// has to as well.
    #[test]
    fn falling_back_on_save_clears_the_stale_keychain_entry() {
        let deleted = Arc::new(Mutex::new(false));
        let store = ResilientTokenStore::with_backend(
            Box::new(FailingWrites(deleted.clone())),
            temp_dir("stale").join("token"),
        );

        store.save(&Token::generate()).expect("save falls back to the file");

        assert_eq!(store.current_backend(), "file");
        assert!(
            *deleted.lock().unwrap(),
            "a save that fell back to the file must clear the keychain entry it could not update"
        );
    }

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("srelens-ks-{label}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_working_keychain_is_used_and_reported_as_such() {
        let store = ResilientTokenStore::with_backend(
            Box::new(RecordingWorking { stored: Mutex::new(None) }),
            temp_dir("working").join("token"),
        );
        assert_eq!(store.current_backend(), "keychain");
        let t = Token::generate();
        store.save(&t).unwrap();
        assert_eq!(store.load().unwrap().as_str(), t.as_str());
        assert_eq!(store.current_backend(), "keychain");
        store.clear().unwrap();
        assert!(store.load().is_none());
    }

    #[test]
    fn a_keychain_that_genuinely_fails_falls_back_to_the_file_without_panicking() {
        let store = ResilientTokenStore::with_backend(
            Box::new(AlwaysFails),
            temp_dir("broken").join("token"),
        );
        let t = Token::generate();
        // This is the exact call that used to panic when the "probe once"
        // design reported `Ok` up front and the real dial failed here.
        store.save(&t).expect("a usable store must always be returned");
        assert_eq!(store.load().unwrap().as_str(), t.as_str());
        assert_eq!(store.current_backend(), "file");
        store.clear().unwrap();
        assert!(store.load().is_none());
    }

    #[test]
    fn no_entry_from_a_working_keychain_does_not_trigger_fallback() {
        let store = ResilientTokenStore::with_backend(
            Box::new(EmptyButWorking),
            temp_dir("empty").join("token"),
        );
        // Nothing saved yet: a working-but-empty keychain must read as "no
        // token", not silently fall through to whatever (stale) file might
        // be sitting at `fallback`.
        assert!(store.load().is_none());
        assert_eq!(store.current_backend(), "keychain");
    }

    #[test]
    fn once_fallen_back_a_store_stays_on_the_file_for_subsequent_calls() {
        let store = ResilientTokenStore::with_backend(
            Box::new(AlwaysFails),
            temp_dir("sticky").join("token"),
        );
        let a = Token::generate();
        store.save(&a).unwrap();
        assert_eq!(store.current_backend(), "file");
        // A later load must find the file-stored token via the same path,
        // not re-probe the (still-broken) keychain and come back empty.
        assert_eq!(store.load().unwrap().as_str(), a.as_str());
    }

    #[test]
    fn falls_back_to_a_file_when_no_keychain_is_available() {
        // The real backend, exercised end-to-end. On this developer's
        // machine (macOS) this goes through the actual Keychain; on a
        // headless Linux CI runner without a D-Bus session it goes through
        // the file fallback. Either way the store must always be usable —
        // that's what's asserted, not which branch was taken.
        let store = keychain_or_file(temp_dir("real").join("token"));
        let t = Token::generate();
        store.save(&t).expect("a usable store must always be returned");
        assert_eq!(store.load().unwrap().as_str(), t.as_str());
        store.clear().unwrap();
    }
}
