//! Token storage preferring the OS keychain, falling back to a 0600 file.
//! Linux without a Secret Service (headless boxes, minimal WMs) has no
//! keychain at all; refusing to run there would strand those users, so we fall
//! back and tell them plainly in Settings.

use std::path::PathBuf;
use std::sync::Arc;

use srelens_mcp::auth::{FileTokenStore, Token, TokenStore};

const SERVICE: &str = "srelens";
const ACCOUNT: &str = "mcp-http-token";

pub struct KeychainTokenStore;

impl TokenStore for KeychainTokenStore {
    fn load(&self) -> Option<Token> {
        keyring::Entry::new(SERVICE, ACCOUNT)
            .ok()?
            .get_password()
            .ok()
            .and_then(|s| Token::from_hex(&s))
    }

    fn save(&self, t: &Token) -> std::io::Result<()> {
        keyring::Entry::new(SERVICE, ACCOUNT)
            .and_then(|e| e.set_password(t.as_str()))
            .map_err(|e| std::io::Error::other(e.to_string()))
    }

    fn clear(&self) -> std::io::Result<()> {
        match keyring::Entry::new(SERVICE, ACCOUNT).and_then(|e| e.delete_credential()) {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(std::io::Error::other(e.to_string())),
        }
    }
}

/// Tauri-managed marker recording which backend is actually in use, so
/// Settings can tell the user plainly when the token sits on disk unencrypted
/// rather than implying it's always keychain-protected.
pub struct TokenStorage(pub &'static str);

impl TokenStorage {
    pub fn for_fallback(file_fallback: bool) -> Self {
        Self(if file_fallback { "file" } else { "keychain" })
    }
}

/// Returns the store and whether the on-disk fallback is in use.
pub fn keychain_or_file(fallback: PathBuf) -> (Arc<dyn TokenStore>, bool) {
    let probe = keyring::Entry::new(SERVICE, ACCOUNT);
    match probe {
        Ok(_) => (Arc::new(KeychainTokenStore), false),
        Err(_) => (Arc::new(FileTokenStore::new(fallback)), true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falls_back_to_a_file_when_no_keychain_is_available() {
        // On CI Linux there is usually no Secret Service; either branch is
        // valid, but the store must always be usable.
        let dir = std::env::temp_dir().join(format!("srelens-ks-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (store, _fallback) = keychain_or_file(dir.join("token"));
        let t = Token::generate();
        store.save(&t).expect("a usable store must always be returned");
        assert_eq!(store.load().unwrap().as_str(), t.as_str());
        store.clear().unwrap();
    }
}
