//! The secrets vault (issue #206): every srelens secret — the MCP HTTP token
//! and the native agent's provider API keys — lives in ONE encrypted file
//! (`secrets.enc`), keyed by ONE master key held in a single OS keychain
//! entry. The app therefore touches the keychain once per launch, for one
//! item, instead of once per secret per access — which is what made macOS
//! re-prompt constantly under dev builds (each rebuild changes the ad-hoc
//! signature, and every keychain item access re-asks).
//!
//! File format: `[version: 1 byte][XChaCha20-Poly1305 nonce: 24 bytes][ciphertext]`.
//! The plaintext is a small JSON [`Secrets`] map. Writes are atomic
//! (exclusive-create a 0600 temp file, then rename) so a crash mid-save can't
//! leave a torn vault.
//!
//! Master key: 32 random bytes, hex-encoded in the keychain (service
//! `srelens`, account `master-key`). Where the keychain genuinely fails
//! (headless Linux without a Secret Service), the key falls back to a 0600
//! file — the vault is then only obfuscation, the same trust model the old
//! per-secret fallback files had, and Settings says so plainly.
//!
//! A vault that can't be decrypted (wrong key, tampered or truncated file)
//! reads as EMPTY rather than erroring: secrets here are all re-obtainable
//! (the MCP token regenerates; API keys are re-pasted), and a later save
//! simply overwrites. That is deliberate — never brick the app over a
//! corrupt secrets file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};

const SERVICE: &str = "srelens";
const MASTER_KEY_ACCOUNT: &str = "master-key";
const FORMAT_VERSION: u8 = 1;
const NONCE_LEN: usize = 24;
const KEY_LEN: usize = 32;

/// Everything the vault holds. New fields must be `#[serde(default)]` so a
/// vault written by an older build still decodes.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Secrets {
    /// The MCP HTTP bearer token (64 hex chars), if one has been minted.
    #[serde(default)]
    pub mcp_token: Option<String>,
    /// Provider API keys, keyed by provider slug (`llm_config::slug`).
    #[serde(default)]
    pub llm_keys: BTreeMap<String, String>,
}

/// The raw master-key keychain operations, factored out so tests can inject a
/// stub that deterministically fails — the only way to exercise the file
/// fallback without a real (or deliberately broken) OS keychain.
trait KeychainBackend: Send + Sync {
    fn get_password(&self) -> Result<String, keyring::Error>;
    fn set_password(&self, value: &str) -> Result<(), keyring::Error>;
}

struct RealKeychain;

impl KeychainBackend for RealKeychain {
    fn get_password(&self) -> Result<String, keyring::Error> {
        keyring::Entry::new(SERVICE, MASTER_KEY_ACCOUNT)?.get_password()
    }

    fn set_password(&self, value: &str) -> Result<(), keyring::Error> {
        keyring::Entry::new(SERVICE, MASTER_KEY_ACCOUNT)?.set_password(value)
    }
}

/// The open vault: master key resolved (and cached — the keychain is never
/// touched again for this process), file path fixed. All access goes through
/// [`load`](Self::load) / [`update`](Self::update); `update` is a locked
/// read-modify-write so two commands can't lose each other's field.
pub struct Vault {
    path: PathBuf,
    key: [u8; KEY_LEN],
    key_source: &'static str,
    lock: Mutex<()>,
}

impl Vault {
    /// Open the vault under `dir` (`secrets.enc` + the `master.key` fallback
    /// file), resolving the master key from the real OS keychain. Infallible:
    /// key persistence is best-effort, and an unreadable vault reads as empty.
    pub fn open(dir: &Path) -> Vault {
        Self::with_backend(dir, Box::new(RealKeychain))
    }

    fn with_backend(dir: &Path, backend: Box<dyn KeychainBackend>) -> Vault {
        let (key, key_source) = load_master_key(backend.as_ref(), &dir.join("master.key"));
        Vault { path: dir.join("secrets.enc"), key, key_source, lock: Mutex::new(()) }
    }

    /// Where the master key lives: `"keychain"`, or `"file"` when the
    /// keychain genuinely failed and the 0600 fallback is in use. Shown in
    /// Settings so the reduced trust model is never silent.
    pub fn key_source(&self) -> &'static str {
        self.key_source
    }

    /// Read the current secrets. Missing, tampered, or wrong-key vaults read
    /// as empty (see the module docs for why that's deliberate).
    pub fn load(&self) -> Secrets {
        let _guard = self.lock.lock().unwrap();
        self.read_secrets()
    }

    /// Locked read-modify-write: apply `mutate` to the current secrets and
    /// persist the result atomically.
    pub fn update(&self, mutate: impl FnOnce(&mut Secrets)) -> std::io::Result<()> {
        let _guard = self.lock.lock().unwrap();
        let mut secrets = self.read_secrets();
        mutate(&mut secrets);
        self.write_secrets(&secrets)
    }

    fn read_secrets(&self) -> Secrets {
        let Ok(bytes) = std::fs::read(&self.path) else { return Secrets::default() };
        decrypt(&self.key, &bytes).unwrap_or_default()
    }

    fn write_secrets(&self, secrets: &Secrets) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let sealed = encrypt(&self.key, secrets)?;
        // Atomic replace: exclusive-create a private temp file next to the
        // vault (same directory, so the rename can't cross filesystems), then
        // rename over. A crash mid-write leaves the old vault intact.
        let tmp = self.path.with_extension("enc.tmp");
        write_exclusive_private(&tmp, &sealed)?;
        std::fs::rename(&tmp, &self.path)
    }
}

fn encrypt(key: &[u8; KEY_LEN], secrets: &Secrets) -> std::io::Result<Vec<u8>> {
    let plaintext = serde_json::to_vec(secrets).map_err(std::io::Error::other)?;
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::getrandom(&mut nonce).map_err(|e| std::io::Error::other(e.to_string()))?;
    let cipher = XChaCha20Poly1305::new(key.into());
    let ciphertext = cipher
        .encrypt(XNonce::from_slice(&nonce), plaintext.as_slice())
        .map_err(|_| std::io::Error::other("vault encryption failed"))?;
    let mut out = Vec::with_capacity(1 + NONCE_LEN + ciphertext.len());
    out.push(FORMAT_VERSION);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

fn decrypt(key: &[u8; KEY_LEN], bytes: &[u8]) -> Option<Secrets> {
    // The Poly1305 tag alone is 16 bytes, so anything shorter is garbage.
    if bytes.len() < 1 + NONCE_LEN + 16 || bytes[0] != FORMAT_VERSION {
        return None;
    }
    let (nonce, ciphertext) = bytes[1..].split_at(NONCE_LEN);
    let cipher = XChaCha20Poly1305::new(key.into());
    let plaintext = cipher.decrypt(XNonce::from_slice(nonce), ciphertext).ok()?;
    serde_json::from_slice(&plaintext).ok()
}

/// Resolve the master key, in order: a valid keychain entry; a fresh key
/// stored to the keychain; the 0600 fallback file (read or created) when the
/// keychain genuinely fails. This is the process's ONE keychain touch.
fn load_master_key(backend: &dyn KeychainBackend, key_file: &Path) -> ([u8; KEY_LEN], &'static str) {
    match backend.get_password() {
        Ok(hex) => {
            if let Some(key) = key_from_hex(&hex) {
                return (key, "keychain");
            }
            // A malformed entry can't decrypt anything anyway — mint a fresh
            // key over it (the unreadable-vault-reads-empty rule absorbs the
            // consequence).
            store_fresh_key(backend, key_file)
        }
        Err(keyring::Error::NoEntry) => store_fresh_key(backend, key_file),
        // Genuine keychain failure (no Secret Service, locked store): the key
        // lives in a private file instead, and Settings says so.
        Err(_) => file_key(key_file),
    }
}

fn store_fresh_key(backend: &dyn KeychainBackend, key_file: &Path) -> ([u8; KEY_LEN], &'static str) {
    let key = generate_key();
    if backend.set_password(&to_hex(&key)).is_ok() {
        (key, "keychain")
    } else {
        file_key(key_file)
    }
}

/// Read the fallback key file, or create it (0600, exclusive) with a fresh
/// key. If even that write fails the key is memory-only for this run — the
/// vault still works, it just won't decrypt next launch, which the
/// reads-as-empty rule absorbs.
fn file_key(key_file: &Path) -> ([u8; KEY_LEN], &'static str) {
    if let Ok(existing) = std::fs::read_to_string(key_file) {
        if let Some(key) = key_from_hex(&existing) {
            return (key, "file");
        }
    }
    let key = generate_key();
    if let Some(parent) = key_file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = write_exclusive_private(key_file, to_hex(&key).as_bytes());
    (key, "file")
}

fn generate_key() -> [u8; KEY_LEN] {
    let mut key = [0u8; KEY_LEN];
    getrandom::getrandom(&mut key).expect("system randomness");
    key
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn key_from_hex(s: &str) -> Option<[u8; KEY_LEN]> {
    let s = s.trim();
    if s.len() != KEY_LEN * 2 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut key = [0u8; KEY_LEN];
    for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
        key[i] = u8::from_str_radix(std::str::from_utf8(chunk).ok()?, 16).ok()?;
    }
    Some(key)
}

/// Remove-then-exclusively-create with 0600 — the same fail-closed pattern as
/// `assistant::write_private_file`: a pre-existing path (including a planted
/// symlink) is never followed or inherited.
fn write_exclusive_private(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let _ = std::fs::remove_file(path);
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?;
        f.write_all(contents)
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, contents)
    }
}

/// Test-only vault for OTHER modules' tests: never touches the real OS
/// keychain (a deliberately broken backend routes the key to the file
/// fallback under `dir`).
#[cfg(test)]
pub fn test_vault(dir: &Path) -> Vault {
    struct NoKeychain;
    impl KeychainBackend for NoKeychain {
        fn get_password(&self) -> Result<String, keyring::Error> {
            Err(keyring::Error::NoStorageAccess(Box::new(std::io::Error::other("test: no keychain"))))
        }
        fn set_password(&self, _value: &str) -> Result<(), keyring::Error> {
            Err(keyring::Error::NoStorageAccess(Box::new(std::io::Error::other("test: no keychain"))))
        }
    }
    Vault::with_backend(dir, Box::new(NoKeychain))
}

/// The MCP token, served from the vault. Drop-in [`TokenStore`] replacement
/// for the old keychain-per-token `ResilientTokenStore`.
pub struct VaultTokenStore(pub std::sync::Arc<Vault>);

impl srelens_mcp::auth::TokenStore for VaultTokenStore {
    fn load(&self) -> Option<srelens_mcp::auth::Token> {
        self.0.load().mcp_token.as_deref().and_then(srelens_mcp::auth::Token::from_hex)
    }

    fn save(&self, t: &srelens_mcp::auth::Token) -> std::io::Result<()> {
        let value = t.as_str().to_string();
        self.0.update(|s| s.mcp_token = Some(value))
    }

    fn clear(&self) -> std::io::Result<()> {
        self.0.update(|s| s.mcp_token = None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use srelens_mcp::auth::{Token, TokenStore};

    /// A keychain with an in-memory entry — the working case.
    struct MemKeychain(Mutex<Option<String>>);

    impl KeychainBackend for MemKeychain {
        fn get_password(&self) -> Result<String, keyring::Error> {
            self.0.lock().unwrap().clone().ok_or(keyring::Error::NoEntry)
        }
        fn set_password(&self, value: &str) -> Result<(), keyring::Error> {
            *self.0.lock().unwrap() = Some(value.to_string());
            Ok(())
        }
    }

    /// A keychain that genuinely fails every operation — the headless case.
    struct BrokenKeychain;

    impl KeychainBackend for BrokenKeychain {
        fn get_password(&self) -> Result<String, keyring::Error> {
            Err(keyring::Error::NoStorageAccess(Box::new(std::io::Error::other("stub"))))
        }
        fn set_password(&self, _value: &str) -> Result<(), keyring::Error> {
            Err(keyring::Error::NoStorageAccess(Box::new(std::io::Error::other("stub"))))
        }
    }

    fn temp_dir(label: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "srelens-vault-{label}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn secrets_round_trip_across_vault_reopens() {
        let dir = temp_dir("roundtrip");
        let a = Vault::with_backend(&dir, Box::new(MemKeychain(Mutex::new(None))));
        a.update(|s| {
            s.mcp_token = Some("ab".repeat(32));
            s.llm_keys.insert("anthropic".into(), "sk-ant-123".into());
        })
        .unwrap();
        assert_eq!(a.key_source(), "keychain");
        // Reopen "next launch": a keychain already holding the key `a` minted.
        let b = Vault::with_backend(&dir, Box::new(MemKeychain(Mutex::new(Some(to_hex(&a.key))))));
        let s = b.load();
        assert_eq!(s.mcp_token.as_deref(), Some("ab".repeat(32).as_str()));
        assert_eq!(s.llm_keys.get("anthropic").map(String::as_str), Some("sk-ant-123"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn update_preserves_fields_it_does_not_touch() {
        let dir = temp_dir("preserve");
        let v = Vault::with_backend(&dir, Box::new(MemKeychain(Mutex::new(None))));
        v.update(|s| s.mcp_token = Some("cd".repeat(32))).unwrap();
        v.update(|s| {
            s.llm_keys.insert("openai".into(), "sk-oai".into());
        })
        .unwrap();
        let s = v.load();
        assert_eq!(s.mcp_token.as_deref(), Some("cd".repeat(32).as_str()), "token survived the key write");
        assert_eq!(s.llm_keys.get("openai").map(String::as_str), Some("sk-oai"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_wrong_key_or_tampered_vault_reads_as_empty_not_an_error() {
        let dir = temp_dir("tamper");
        let v = Vault::with_backend(&dir, Box::new(MemKeychain(Mutex::new(None))));
        v.update(|s| s.mcp_token = Some("ef".repeat(32))).unwrap();

        // Wrong key: a different vault instance with a fresh random key.
        let wrong = Vault::with_backend(&dir, Box::new(MemKeychain(Mutex::new(Some(to_hex(&generate_key()))))));
        assert_eq!(wrong.load(), Secrets::default());

        // Tampered ciphertext: flip one byte — the AEAD tag must reject it.
        let mut bytes = std::fs::read(dir.join("secrets.enc")).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        std::fs::write(dir.join("secrets.enc"), &bytes).unwrap();
        assert_eq!(v.load(), Secrets::default());

        // Truncated/garbage file likewise.
        std::fs::write(dir.join("secrets.enc"), b"nope").unwrap();
        assert_eq!(v.load(), Secrets::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_vault_file_is_versioned_nonced_and_owner_only() {
        let dir = temp_dir("format");
        let v = Vault::with_backend(&dir, Box::new(MemKeychain(Mutex::new(None))));
        v.update(|s| s.mcp_token = Some("01".repeat(32))).unwrap();
        let bytes = std::fs::read(dir.join("secrets.enc")).unwrap();
        assert_eq!(bytes[0], FORMAT_VERSION);
        assert!(bytes.len() > 1 + NONCE_LEN + 16, "nonce + tag + ciphertext present");
        // The plaintext must not appear in the file.
        let hex_token = "01".repeat(32);
        assert!(!bytes.windows(hex_token.len()).any(|w| w == hex_token.as_bytes()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join("secrets.enc")).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_keychain_entry_mints_and_stores_the_master_key() {
        let dir = temp_dir("mint");
        let entry = MemKeychain(Mutex::new(None));
        let v = Vault::with_backend(&dir, Box::new(entry));
        assert_eq!(v.key_source(), "keychain");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_broken_keychain_falls_back_to_a_stable_owner_only_key_file() {
        let dir = temp_dir("fallback");
        let a = Vault::with_backend(&dir, Box::new(BrokenKeychain));
        assert_eq!(a.key_source(), "file");
        a.update(|s| s.mcp_token = Some("23".repeat(32))).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join("master.key")).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        // A second open (fresh process) reads the SAME key from the file, so
        // the vault written above still decrypts.
        let b = Vault::with_backend(&dir, Box::new(BrokenKeychain));
        assert_eq!(b.load().mcp_token.as_deref(), Some("23".repeat(32).as_str()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_malformed_keychain_entry_is_replaced_with_a_fresh_key() {
        let dir = temp_dir("malformed");
        let v = Vault::with_backend(&dir, Box::new(MemKeychain(Mutex::new(Some("not-hex".into())))));
        assert_eq!(v.key_source(), "keychain");
        // And the vault is usable with the replacement key.
        v.update(|s| s.mcp_token = Some("45".repeat(32))).unwrap();
        assert_eq!(v.load().mcp_token.as_deref(), Some("45".repeat(32).as_str()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_vault_token_store_round_trips_and_clears() {
        let dir = temp_dir("tokenstore");
        let vault = std::sync::Arc::new(Vault::with_backend(&dir, Box::new(MemKeychain(Mutex::new(None)))));
        let store = VaultTokenStore(vault.clone());
        assert!(store.load().is_none());
        let t = Token::generate();
        store.save(&t).unwrap();
        assert_eq!(store.load().unwrap().as_str(), t.as_str());
        // The token coexists with other secrets rather than clobbering them.
        vault.update(|s| {
            s.llm_keys.insert("gemini".into(), "g-key".into());
        })
        .unwrap();
        store.clear().unwrap();
        assert!(store.load().is_none());
        assert_eq!(vault.load().llm_keys.get("gemini").map(String::as_str), Some("g-key"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
