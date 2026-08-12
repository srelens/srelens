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
    /// `None` means LOCKED: a vault exists whose key lives in a keychain that
    /// couldn't be reached this launch, and no fallback key file exists.
    /// Minting a replacement key would let the next save silently destroy
    /// every stored secret — so instead reads are empty and writes fail
    /// loudly until a launch with a reachable keychain heals things.
    key: Option<[u8; KEY_LEN]>,
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
        let path = dir.join("secrets.enc");
        let (key, key_source) = resolve_master_key(backend.as_ref(), &dir.join("master.key"), &path);
        Vault { path, key, key_source, lock: Mutex::new(()) }
    }

    /// Where the master key lives: `"keychain"`; `"file"` when the keychain
    /// genuinely failed and the 0600 fallback is in use; or `"locked"` when
    /// an existing vault's key is in an unreachable keychain and writes are
    /// refused this launch. Shown in Settings so neither reduced trust model
    /// is ever silent.
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
    /// persist the result atomically. Two locks, because two kinds of
    /// concurrent writer exist: the process-local mutex for callers sharing
    /// this `Vault`, and an EXCLUSIVE file lock for the other process — the
    /// GUI and the standalone `--mcp-http` CLI deliberately open the same
    /// vault, and without the file lock their read-modify-writes could each
    /// read the old map and silently discard the other's mutation. Readers
    /// need no lock: writes replace the file atomically via rename, so a
    /// load sees either the old vault or the new one, never a torn write.
    pub fn update(&self, mutate: impl FnOnce(&mut Secrets)) -> std::io::Result<()> {
        let _guard = self.lock.lock().unwrap();
        let Some(key) = self.key else {
            return Err(std::io::Error::other(
                "the secrets vault is locked: the OS keychain holding its master key is unreachable",
            ));
        };
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let lock_file = std::fs::File::create(self.path.with_extension("enc.lock"))?;
        lock_file.lock()?;
        let mut secrets = self.read_secrets();
        mutate(&mut secrets);
        write_sealed(&key, &self.path, &secrets)
        // `lock_file` drops here, releasing the inter-process lock.
    }

    fn read_secrets(&self) -> Secrets {
        let Some(key) = &self.key else { return Secrets::default() };
        let Ok(bytes) = std::fs::read(&self.path) else { return Secrets::default() };
        decrypt(key, &bytes).unwrap_or_default()
    }
}

/// Encrypt and atomically replace the vault file: exclusive-create a private
/// temp file next to it (same directory, so the rename can't cross
/// filesystems), then rename over. A crash mid-write leaves the old vault
/// intact. The temp name is writer-unique so a competing process (see
/// [`Vault::update`]) can never remove or rename another writer's
/// half-written file.
fn write_sealed(key: &[u8; KEY_LEN], path: &Path, secrets: &Secrets) -> std::io::Result<()> {
    let sealed = encrypt(key, secrets)?;
    let tmp = path.with_extension(format!("enc.{}.tmp", uuid::Uuid::new_v4()));
    write_exclusive_private(&tmp, &sealed)?;
    std::fs::rename(&tmp, path)
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

/// Resolve the master key. This is the process's ONE keychain touch — and it
/// must RECONCILE the two possible key homes, not just pick a branch: a
/// headless launch (no keychain) leaves the key in `master.key`, and a later
/// keychain-capable launch seeing `NoEntry` must import that file key rather
/// than mint a fresh one — otherwise the existing vault silently reads as
/// empty and the next save destroys every stored secret. When BOTH homes
/// hold (different) keys — e.g. a transient keychain failure once minted a
/// file key alongside a healthy keychain entry — the vault file itself is
/// the arbiter: whichever key actually decrypts it wins, and is promoted
/// into the keychain so the split heals instead of persisting.
fn resolve_master_key(
    backend: &dyn KeychainBackend,
    key_file: &Path,
    vault_path: &Path,
) -> (Option<[u8; KEY_LEN]>, &'static str) {
    // One inter-process lock around the WHOLE resolution — keychain read,
    // mint/import, key-file read/create. Without it, two first-time openers
    // (the GUI and the standalone CLI starting together) can both observe an
    // empty keychain (or a missing key file), mint different keys, and split
    // the vault: the last `set_password` wins while each process keeps its
    // own key. Best-effort: if the lock can't be created, resolution simply
    // proceeds unserialized.
    if let Some(parent) = key_file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let resolution_lock = std::fs::File::create(key_file.with_extension("key.lock"));
    if let Ok(lock) = &resolution_lock {
        let _ = lock.lock();
    }

    let resolved = resolve_master_key_locked(backend, key_file, vault_path);

    // Claim the directory BEFORE the resolution lock releases: if no vault
    // exists yet, write an empty one under the resolved key. Without this, a
    // keychain-capable first opener that hasn't saved anything leaves the
    // directory bare, and a concurrently starting keychain-LESS opener would
    // find neither vault nor key file and mint an unrelated fallback key —
    // two live processes encrypting under different keys. With the marker,
    // that second opener finds `secrets.enc` and takes the locked path.
    // Best-effort: a failed marker write only leaves the pre-existing race
    // window rather than failing startup.
    if let (Some(key), _) = &resolved {
        if !vault_path.exists() {
            let _ = write_sealed(key, vault_path, &Secrets::default());
        }
    }
    resolved
}

/// The resolution decision tree proper — runs under `resolve_master_key`'s
/// inter-process lock.
fn resolve_master_key_locked(
    backend: &dyn KeychainBackend,
    key_file: &Path,
    vault_path: &Path,
) -> (Option<[u8; KEY_LEN]>, &'static str) {
    let keychain_key = match backend.get_password() {
        // A malformed entry can't decrypt anything anyway — treat it like an
        // absent one (a fresh or imported key will overwrite it).
        Ok(hex) => key_from_hex(&hex),
        Err(keyring::Error::NoEntry) => None,
        // Genuine keychain failure (no Secret Service, locked store). Use the
        // existing file key if there is one. Otherwise: if a vault already
        // EXISTS, its key is almost certainly sitting in the keychain we just
        // failed to reach — NEVER mint a replacement (a save under it would
        // permanently destroy every stored secret); open locked instead.
        // Only a first keychain-less run (no vault to lose) mints a file key.
        Err(_) => {
            if let Some(k) = read_key_file(key_file) {
                return (Some(k), "file");
            }
            if vault_path.exists() {
                return (None, "locked");
            }
            return file_key(key_file);
        }
    };
    let stored_file_key = read_key_file(key_file);

    match (keychain_key, stored_file_key) {
        (Some(k), None) => (Some(k), "keychain"),
        // The same key in both homes: the file copy is a redundant plaintext
        // liability — drop it.
        (Some(k), Some(f)) if k == f => {
            let _ = std::fs::remove_file(key_file);
            (Some(k), "keychain")
        }
        // Diverged keys: whichever decrypts the existing vault wins. The
        // winner is promoted to the keychain (when it was the file key) and
        // the file copy is dropped, healing the split.
        (Some(k), Some(f)) => {
            let vault_bytes = std::fs::read(vault_path).ok();
            let file_key_decrypts = vault_bytes
                .as_deref()
                .map(|b| decrypt(&f, b).is_some() && decrypt(&k, b).is_none())
                .unwrap_or(false);
            let (key, source) = if file_key_decrypts {
                adopt_file_key(backend, key_file, f)
            } else {
                let _ = std::fs::remove_file(key_file);
                (k, "keychain")
            };
            (Some(key), source)
        }
        // No keychain entry but a file key exists (a previous launch ran
        // keychain-less): import it instead of minting a divergent fresh key.
        (None, Some(f)) => {
            let (key, source) = adopt_file_key(backend, key_file, f);
            (Some(key), source)
        }
        (None, None) => {
            let key = generate_key();
            if backend.set_password(&to_hex(&key)).is_ok() {
                (Some(key), "keychain")
            } else {
                file_key(key_file)
            }
        }
    }
}

/// Promote an existing file key into the keychain; only once that write
/// SUCCEEDS is the plaintext file copy dropped. If the keychain refuses, the
/// file stays the (working) home.
fn adopt_file_key(
    backend: &dyn KeychainBackend,
    key_file: &Path,
    key: [u8; KEY_LEN],
) -> ([u8; KEY_LEN], &'static str) {
    if backend.set_password(&to_hex(&key)).is_ok() {
        let _ = std::fs::remove_file(key_file);
        (key, "keychain")
    } else {
        (key, "file")
    }
}

fn read_key_file(key_file: &Path) -> Option<[u8; KEY_LEN]> {
    std::fs::read_to_string(key_file).ok().and_then(|s| key_from_hex(&s))
}

/// Read the fallback key file, or create it (0600, exclusive) with a fresh
/// key. Runs under `resolve_master_key`'s inter-process resolution lock, so
/// two starters can't mint different keys. The key handed out MUST be the
/// PERSISTED one — read back after writing — because an in-memory-only key
/// would accept saves this run that no future launch could ever decrypt; if
/// the write cannot be read back (e.g. `master.key` is unexpectedly a
/// directory), the vault opens locked so saves fail loudly instead.
fn file_key(key_file: &Path) -> (Option<[u8; KEY_LEN]>, &'static str) {
    if let Some(key) = read_key_file(key_file) {
        return (Some(key), "file");
    }
    let key = generate_key();
    let _ = write_exclusive_private(key_file, to_hex(&key).as_bytes());
    match read_key_file(key_file) {
        Some(persisted) => (Some(persisted), "file"),
        None => (None, "locked"),
    }
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
        let b = Vault::with_backend(&dir, Box::new(MemKeychain(Mutex::new(Some(to_hex(&a.key.unwrap()))))));
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
    fn a_file_key_from_a_keychainless_launch_is_imported_not_shadowed() {
        let dir = temp_dir("import");
        // Launch 1: no keychain — secrets land under a file key.
        let headless = Vault::with_backend(&dir, Box::new(BrokenKeychain));
        headless.update(|s| s.mcp_token = Some("67".repeat(32))).unwrap();

        // Launch 2: keychain works but holds no entry. The file key must be
        // imported (NOT a fresh key minted), so the vault still decrypts…
        let mem = Mutex::new(None);
        let gui = Vault::with_backend(&dir, Box::new(MemKeychain(mem)));
        assert_eq!(gui.key_source(), "keychain");
        assert_eq!(gui.load().mcp_token.as_deref(), Some("67".repeat(32).as_str()));
        // …and the plaintext file copy is gone after a successful import.
        assert!(!dir.join("master.key").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn diverged_keys_are_arbitrated_by_whichever_decrypts_the_vault() {
        let dir = temp_dir("diverge");
        // The vault was written under a FILE key (a transient keychain outage)…
        let outage = Vault::with_backend(&dir, Box::new(BrokenKeychain));
        outage.update(|s| s.mcp_token = Some("89".repeat(32))).unwrap();
        // …while the keychain still holds an older, unrelated key.
        let stale_keychain_key = to_hex(&generate_key());
        let healed = Vault::with_backend(&dir, Box::new(MemKeychain(Mutex::new(Some(stale_keychain_key)))));
        // The file key decrypts the vault, so it wins and is promoted.
        assert_eq!(healed.key_source(), "keychain");
        assert_eq!(healed.load().mcp_token.as_deref(), Some("89".repeat(32).as_str()));
        assert!(!dir.join("master.key").exists(), "promoted file key's plaintext copy removed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_temporary_keychain_failure_reuses_the_existing_file_key() {
        let dir = temp_dir("transient");
        let a = Vault::with_backend(&dir, Box::new(BrokenKeychain));
        a.update(|s| s.mcp_token = Some("ba".repeat(32))).unwrap();
        // A later keychain-less launch must reuse the SAME file key, never
        // mint a second one over it.
        let b = Vault::with_backend(&dir, Box::new(BrokenKeychain));
        assert_eq!(b.load().mcp_token.as_deref(), Some("ba".repeat(32).as_str()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unreachable_keychain_with_an_existing_vault_locks_instead_of_rekeying() {
        let dir = temp_dir("locked");
        // The vault exists, keyed by a healthy keychain…
        let healthy = Vault::with_backend(&dir, Box::new(MemKeychain(Mutex::new(None))));
        healthy.update(|s| s.mcp_token = Some("dc".repeat(32))).unwrap();
        let vault_before = std::fs::read(dir.join("secrets.enc")).unwrap();

        // …and a later launch can't reach the keychain (locked store, dead
        // D-Bus). No replacement key may be minted: reads are empty, writes
        // fail loudly, and the vault bytes are untouched.
        let outage = Vault::with_backend(&dir, Box::new(BrokenKeychain));
        assert_eq!(outage.key_source(), "locked");
        assert_eq!(outage.load(), Secrets::default());
        let err = outage.update(|s| s.mcp_token = Some("ff".repeat(32))).unwrap_err();
        assert!(err.to_string().contains("locked"), "got: {err}");
        assert_eq!(std::fs::read(dir.join("secrets.enc")).unwrap(), vault_before);
        assert!(!dir.join("master.key").exists(), "no replacement key was minted");

        // The next healthy launch reads everything as before.
        let healed = Vault::with_backend(&dir, Box::new(MemKeychain(Mutex::new(Some(to_hex(
            &healthy.key.unwrap(),
        ))))));
        assert_eq!(healed.load().mcp_token.as_deref(), Some("dc".repeat(32).as_str()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A working keychain shared between two backends, the way the real one
    /// is shared between two srelens processes.
    struct SharedKeychain(std::sync::Arc<Mutex<Option<String>>>);

    impl KeychainBackend for SharedKeychain {
        fn get_password(&self) -> Result<String, keyring::Error> {
            self.0.lock().unwrap().clone().ok_or(keyring::Error::NoEntry)
        }
        fn set_password(&self, value: &str) -> Result<(), keyring::Error> {
            *self.0.lock().unwrap() = Some(value.to_string());
            Ok(())
        }
    }

    #[test]
    fn concurrent_first_time_openers_agree_on_one_keychain_key() {
        // Both see an empty (but working) keychain: without the resolution
        // lock each would mint its own key and the last set_password would
        // win, splitting the vault.
        let dir = temp_dir("mintrace");
        let entry = std::sync::Arc::new(Mutex::new(None));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let dir = dir.clone();
                let entry = entry.clone();
                std::thread::spawn(move || Vault::with_backend(&dir, Box::new(SharedKeychain(entry))))
            })
            .collect();
        let vaults: Vec<Vault> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(vaults[0].key.unwrap(), vaults[1].key.unwrap(), "both openers hold the same key");
        vaults[0].update(|s| s.mcp_token = Some("aa".repeat(32))).unwrap();
        assert_eq!(vaults[1].load().mcp_token.as_deref(), Some("aa".repeat(32).as_str()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_keychain_first_opener_claims_the_dir_so_a_keychainless_opener_locks() {
        let dir = temp_dir("mixed");
        // Opener A resolves via a working keychain and hasn't saved anything.
        let a = Vault::with_backend(&dir, Box::new(MemKeychain(Mutex::new(None))));
        // Opener B can't reach the keychain. The empty marker vault A wrote at
        // resolution means B must LOCK — not mint an unrelated file key that
        // would split the vault between two live processes.
        let b = Vault::with_backend(&dir, Box::new(BrokenKeychain));
        assert_eq!(b.key_source(), "locked");
        assert!(!dir.join("master.key").exists(), "no divergent fallback key was minted");
        // A remains fully functional and authoritative.
        a.update(|s| s.mcp_token = Some("cc".repeat(32))).unwrap();
        assert_eq!(a.load().mcp_token.as_deref(), Some("cc".repeat(32).as_str()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unpersistable_key_file_locks_the_vault_instead_of_accepting_doomed_saves() {
        let dir = temp_dir("unpersistable");
        // `master.key` is unexpectedly a DIRECTORY: the key write cannot land,
        // so no key handed out this run could ever be recovered next launch.
        std::fs::create_dir_all(dir.join("master.key")).unwrap();
        let v = Vault::with_backend(&dir, Box::new(BrokenKeychain));
        assert_eq!(v.key_source(), "locked");
        assert!(v.update(|s| s.mcp_token = Some("bb".repeat(32))).is_err(), "saves must fail loudly");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrent_first_time_openers_agree_on_one_file_key() {
        // Two processes starting together on a keychain-less host must not
        // mint different keys (the loser's writes would split the vault).
        let dir = temp_dir("keyrace");
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let dir = dir.clone();
                std::thread::spawn(move || Vault::with_backend(&dir, Box::new(BrokenKeychain)))
            })
            .collect();
        let vaults: Vec<Vault> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(vaults[0].key.unwrap(), vaults[1].key.unwrap(), "both openers hold the same key");
        // And a write through one is readable through the other.
        vaults[0].update(|s| s.mcp_token = Some("ee".repeat(32))).unwrap();
        assert_eq!(vaults[1].load().mcp_token.as_deref(), Some("ee".repeat(32).as_str()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrent_writers_from_separate_vault_instances_lose_no_fields() {
        // Two `Vault` instances over the same directory model the GUI and the
        // standalone CLI: the process-local mutex doesn't cover them, so this
        // exercises the inter-process file lock end to end.
        let dir = temp_dir("interproc");
        let a = std::sync::Arc::new(Vault::with_backend(&dir, Box::new(BrokenKeychain)));
        let b = std::sync::Arc::new(Vault::with_backend(&dir, Box::new(BrokenKeychain)));
        let ta = {
            let a = a.clone();
            std::thread::spawn(move || {
                for i in 0..50 {
                    a.update(|s| {
                        s.llm_keys.insert("anthropic".into(), format!("a{i}"));
                    })
                    .unwrap();
                }
            })
        };
        let tb = {
            let b = b.clone();
            std::thread::spawn(move || {
                for i in 0..50 {
                    b.update(|s| {
                        s.llm_keys.insert("openai".into(), format!("b{i}"));
                    })
                    .unwrap();
                }
            })
        };
        ta.join().unwrap();
        tb.join().unwrap();
        // Interleaved read-modify-writes must not have discarded either
        // writer's field — both final values survive.
        let s = a.load();
        assert_eq!(s.llm_keys.get("anthropic").map(String::as_str), Some("a49"));
        assert_eq!(s.llm_keys.get("openai").map(String::as_str), Some("b49"));
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
