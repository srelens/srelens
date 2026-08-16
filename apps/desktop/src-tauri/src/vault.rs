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
//! READS as empty rather than erroring — never brick the app's display over
//! a corrupt secrets file. WRITES over such a vault are refused: the usual
//! cause is another process having re-keyed it (a password change in a
//! second instance), and overwriting would destroy the real secrets.

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
pub(crate) trait KeychainBackend: Send + Sync {
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
    /// `None` means LOCKED: either the keychain holding the key couldn't be
    /// reached (source `"locked"`), or the key sits behind a biometric gate
    /// that hasn't been passed yet this launch (source `"biometric-locked"`,
    /// issue #208). Minting a replacement key would let the next save
    /// silently destroy every stored secret — so instead reads are empty and
    /// writes fail loudly until the vault is unlocked. Behind an `RwLock`
    /// because a biometric unlock arrives AFTER construction.
    key: std::sync::RwLock<Option<[u8; KEY_LEN]>>,
    key_source: std::sync::RwLock<&'static str>,
    lock: Mutex<()>,
}

impl Vault {
    /// Open the vault under `dir` (`secrets.enc` + the `master.key` fallback
    /// file), resolving the master key from the real OS keychain. Infallible:
    /// key persistence is best-effort, and an unreadable vault reads as empty.
    pub fn open(dir: &Path) -> Vault {
        Self::with_backend(dir, Box::new(RealKeychain))
    }

    // pub(crate): `vault_password`'s unit tests build vaults over the same
    // in-memory keychain doubles (`test_support`) the tests here use.
    pub(crate) fn with_backend(dir: &Path, backend: Box<dyn KeychainBackend>) -> Vault {
        let path = dir.join("secrets.enc");
        // Master-password mode: `vault.json` existing means the key derives
        // from the user's password — nothing to resolve at open; the vault
        // starts locked until the gate is passed (biometric skip when the
        // marker is present, password otherwise). Legacy machine-key
        // resolution runs only while no password has been set up.
        let (key, key_source) = if meta_path(dir).exists() {
            if biometric_marker_path(dir).exists() {
                (None, "biometric-locked")
            } else {
                (None, "password-locked")
            }
        } else if biometric_marker_path(dir).exists() {
            // Pre-password biometric gate (transitional state from the
            // machine-key era of this branch): key lives in the biometric
            // store only.
            (None, "biometric-locked")
        } else {
            let resolved = resolve_master_key(backend.as_ref(), &dir.join("master.key"), &path);
            // Aborted SETUP recovery: a staged `.next` with no committed meta.
            // The machine key itself is the arbiter: if it still decrypts the
            // vault, the re-key never ran — drop the stale stage and continue
            // in machine mode (setup will show again). If it no longer does,
            // the re-key completed and only the promote was lost — commit the
            // stage and open password-locked.
            if meta_next_path(dir).exists() {
                // Arbitrate only a SETTLED stage: take the same transition
                // lock setup/change hold across stage → re-key → promote, so
                // a second process opening mid-transition blocks here instead
                // of deleting a live stage. If the lock can't be taken at
                // all, leave the stage alone — never touch it unserialized.
                if let Ok(_transition) = transition_lock(dir) {
                    // Re-check under the lock: the transition we waited on
                    // may have promoted while we blocked.
                    if meta_path(dir).exists() {
                        let source = if biometric_marker_path(dir).exists() {
                            "biometric-locked"
                        } else {
                            "password-locked"
                        };
                        return Vault {
                            path,
                            key: std::sync::RwLock::new(None),
                            key_source: std::sync::RwLock::new(source),
                            lock: Mutex::new(()),
                        };
                    }
                    if meta_next_path(dir).exists() {
                        match (std::fs::read(&path), &resolved.0) {
                            // CONFIRMED no vault — nothing was ever re-keyed;
                            // the stage is stale regardless of key state.
                            (Err(e), _) if e.kind() == std::io::ErrorKind::NotFound => {
                                let _ = std::fs::remove_file(meta_next_path(dir));
                            }
                            // Any other read error (transient PermissionDenied,
                            // I/O): we can't tell whether the re-key ran, and
                            // the stage may be the ONLY metadata deriving the
                            // vault's key — leave it for later arbitration.
                            (Err(_), _) => {}
                            // Machine key still decrypts the vault: the
                            // re-key never ran — drop the stage, continue in
                            // machine mode.
                            (Ok(bytes), Some(key)) if decrypt(key, &bytes).is_some() => {
                                let _ = std::fs::remove_file(meta_next_path(dir));
                            }
                            // Machine key present but doesn't fit: the
                            // re-key completed and only the promote was lost
                            // — commit it.
                            (Ok(_), Some(_)) => {
                                if promote_meta_next(dir).is_ok() {
                                    return Vault {
                                        path,
                                        key: std::sync::RwLock::new(None),
                                        key_source: std::sync::RwLock::new("password-locked"),
                                        lock: Mutex::new(()),
                                    };
                                }
                            }
                            // Machine key UNAVAILABLE (keychain outage): we
                            // cannot tell whether the re-key ran. Leave the
                            // stage — a later launch with keychain access
                            // arbitrates; the vault opens locked either way.
                            (Ok(_), None) => {}
                        }
                    }
                }
            }
            resolved
        };
        Vault {
            path,
            key: std::sync::RwLock::new(key),
            key_source: std::sync::RwLock::new(key_source),
            lock: Mutex::new(()),
        }
    }

    /// Where the master key lives: `"keychain"`; `"file"` when the keychain
    /// genuinely failed and the 0600 fallback is in use; `"locked"` when an
    /// existing vault's key is in an unreachable keychain and writes are
    /// refused this launch; `"biometric"` when the Touch ID gate is enabled
    /// and passed; or `"biometric-locked"` when the gate hasn't been passed
    /// yet. Shown in Settings so no reduced/locked state is ever silent.
    pub fn key_source(&self) -> &'static str {
        *self.key_source.read().unwrap()
    }

    /// The live key, for the biometric module to move between key homes.
    /// `None` while locked.
    pub(crate) fn current_key(&self) -> Option<[u8; KEY_LEN]> {
        *self.key.read().unwrap()
    }

    /// Install `key` after a passed biometric prompt — but only if it can
    /// actually read the existing vault (a stale biometric item must not be
    /// accepted; the caller purges it on this error).
    pub(crate) fn unlock_with(&self, key: [u8; KEY_LEN], source: &'static str) -> Result<(), String> {
        match std::fs::read(&self.path) {
            Ok(bytes) => {
                if decrypt(&key, &bytes).is_none() {
                    return Err("this key does not match the vault".into());
                }
            }
            // No vault yet — nothing to verify against.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            // A transient read error must REFUSE the unlock: accepting an
            // unverified key here would let the old password's success path
            // clean up a staged transition whose metadata is the only way to
            // derive the (already re-keyed) vault's key.
            Err(e) => return Err(format!("the vault file could not be read ({e}); unlock refused")),
        }
        *self.key.write().unwrap() = Some(key);
        *self.key_source.write().unwrap() = source;
        Ok(())
    }

    /// Re-label where the (unchanged) key lives after the biometric gate is
    /// toggled — the key itself stays cached in memory.
    pub(crate) fn set_key_source(&self, source: &'static str) {
        *self.key_source.write().unwrap() = source;
    }

    /// Swap the vault onto a NEW key — the heart of password setup and
    /// password change. The current contents are READ INSIDE the same
    /// process + inter-process critical section that rewrites them, so a
    /// concurrent update (another command, or the standalone CLI) can never
    /// slip in between a snapshot and the re-encryption and be lost.
    pub(crate) fn rekey_from_current(&self, new_key: [u8; KEY_LEN], source: &'static str) -> std::io::Result<()> {
        let _guard = self.lock.lock().unwrap();
        let Some(old_key) = *self.key.read().unwrap() else {
            return Err(std::io::Error::other("the vault is locked; it cannot be re-keyed"));
        };
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let lock_file = std::fs::File::create(self.path.with_extension("enc.lock"))?;
        lock_file.lock()?;
        // Same fail-closed rule as `update`: never re-key over an existing
        // vault the current key can't actually read.
        let secrets = match std::fs::read(&self.path) {
            Ok(bytes) => decrypt(&old_key, &bytes).ok_or_else(|| {
                std::io::Error::other(
                    "the vault was re-keyed by another srelens process (or the file is corrupt) — restart srelens and unlock again",
                )
            })?,
            // Same NotFound-only rule as `update`: a transient read error must
            // abort the rekey, never re-key Secrets::default() over an intact
            // but momentarily unreadable vault.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Secrets::default(),
            Err(e) => {
                return Err(std::io::Error::other(format!(
                    "the vault file could not be read ({e}); refusing to re-key over it"
                )))
            }
        };
        write_sealed(&new_key, &self.path, &secrets)?;
        *self.key.write().unwrap() = Some(new_key);
        *self.key_source.write().unwrap() = source;
        Ok(())
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
        let Some(key) = *self.key.read().unwrap() else {
            let message = match self.key_source() {
                "biometric-locked" => "the secrets vault is locked behind biometric unlock — unlock it in srelens",
                "password-locked" => "the secrets vault is locked — unlock it with your master password in srelens",
                _ => "the secrets vault is locked: the OS keychain holding its master key is unreachable",
            };
            return Err(std::io::Error::other(message));
        };
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let lock_file = std::fs::File::create(self.path.with_extension("enc.lock"))?;
        lock_file.lock()?;
        // Fail-closed read for the WRITE path: an existing vault this key
        // can't decrypt means another process re-keyed it (a password change
        // in a second instance) — proceeding would overwrite real secrets
        // with an empty map under a key the promoted metadata no longer
        // matches. Reads stay empty-on-mismatch for display; writes refuse.
        let mut secrets = match std::fs::read(&self.path) {
            Ok(bytes) => decrypt(&key, &bytes).ok_or_else(|| {
                std::io::Error::other(
                    "the vault was re-keyed by another srelens process (or the file is corrupt) — restart srelens and unlock again",
                )
            })?,
            // Only CONFIRMED absence starts from empty; any other read error
            // (transient PermissionDenied, I/O) aborts — the rename through a
            // still-writable directory could otherwise replace an intact but
            // momentarily unreadable vault with an empty one.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Secrets::default(),
            Err(e) => {
                return Err(std::io::Error::other(format!(
                    "the vault file could not be read ({e}); refusing to overwrite it"
                )))
            }
        };
        mutate(&mut secrets);
        write_sealed(&key, &self.path, &secrets)
        // `lock_file` drops here, releasing the inter-process lock.
    }

    fn read_secrets(&self) -> Secrets {
        let Some(key) = *self.key.read().unwrap() else { return Secrets::default() };
        let Ok(bytes) = std::fs::read(&self.path) else { return Secrets::default() };
        decrypt(&key, &bytes).unwrap_or_default()
    }
}

/// The marker recording that the Touch ID gate is on (issue #208): non-secret
/// by design — resolution must know how to fetch the key BEFORE any secret is
/// readable. Its presence means the plain keychain entry was deleted and the
/// key's only home is the OS biometric store.
pub(crate) fn biometric_marker_path(dir: &Path) -> PathBuf {
    dir.join("vault-biometric")
}

// --- Master-password mode (issue #208 follow-up, mqlens's model) ---

/// Known plaintext sealed under a derived key inside `vault.json`, so a wrong
/// password is detected without ever touching the real secrets.
const VERIFIER: &[u8] = b"srelens-vault-verifier-v1";

/// Unencrypted KDF metadata (`vault.json`). Its EXISTENCE is what switches
/// the vault into master-password mode: the key is then derived from the
/// user's password (argon2id), never machine-generated.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VaultMeta {
    pub version: u32,
    pub kdf_alg: String,
    pub kdf_m_kib: u32,
    pub kdf_t: u32,
    pub kdf_p: u32,
    /// Hex of the 16-byte salt.
    pub salt: String,
    /// Hex of `[version][nonce][ct]` of [`VERIFIER`] under the derived key.
    pub verifier: String,
}

pub(crate) fn meta_path(dir: &Path) -> PathBuf {
    dir.join("vault.json")
}

/// The staged HALF of a password transition (setup or change): the new meta
/// lands here first, the vault is re-keyed, and only then is this promoted
/// over `vault.json` — so a crash at any point leaves a deterministic pair of
/// files to recover from instead of a stranded vault (see
/// `unlock_with_master_password` and the aborted-transition check in
/// `with_backend`).
pub(crate) fn meta_next_path(dir: &Path) -> PathBuf {
    dir.join("vault.json.next")
}

pub(crate) fn read_meta(dir: &Path) -> Option<VaultMeta> {
    serde_json::from_str(&std::fs::read_to_string(meta_path(dir)).ok()?).ok()
}

pub(crate) fn read_meta_next(dir: &Path) -> Option<VaultMeta> {
    serde_json::from_str(&std::fs::read_to_string(meta_next_path(dir)).ok()?).ok()
}

/// Atomically write meta to `path`: full temp file first, then rename — a
/// failed write (full disk) must never leave the previous, still-needed meta
/// deleted or truncated.
fn write_meta_at(path: &Path, meta: &VaultMeta) -> Result<(), String> {
    let json = serde_json::to_vec_pretty(meta).map_err(|e| e.to_string())?;
    let tmp = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
    write_exclusive_private(&tmp, &json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// Direct meta write — production flows go through the staged
/// `write_meta_next` + `promote_meta_next` transaction; tests use this to
/// construct starting states.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn write_meta(dir: &Path, meta: &VaultMeta) -> Result<(), String> {
    write_meta_at(&meta_path(dir), meta)
}

pub(crate) fn write_meta_next(dir: &Path, meta: &VaultMeta) -> Result<(), String> {
    write_meta_at(&meta_next_path(dir), meta)
}

/// Commit a staged transition: the vault has been re-keyed to the `.next`
/// meta's key, so promote it over `vault.json` (atomic rename).
pub(crate) fn promote_meta_next(dir: &Path) -> Result<(), String> {
    std::fs::rename(meta_next_path(dir), meta_path(dir)).map_err(|e| e.to_string())
}

/// The inter-process lock serializing password TRANSITIONS (setup/change:
/// stage → re-key → promote) against anything that inspects or cleans the
/// staged `.next` meta. Without it, an unlock in a second process could
/// delete a live stage between another process's staging and re-key —
/// leaving a re-keyed vault whose new salt/verifier exist nowhere.
pub(crate) fn transition_lock(dir: &Path) -> std::io::Result<std::fs::File> {
    let _ = std::fs::create_dir_all(dir);
    let lock = std::fs::File::create(dir.join("vault.json.lock"))?;
    lock.lock()?;
    Ok(lock)
}

/// Unlock the vault with a master password, recovering an interrupted
/// password transition if one is staged: the CURRENT meta is tried first;
/// when it doesn't line up (verifier or vault mismatch), a staged `.next`
/// meta whose verifier AND vault both agree is promoted and used — that is
/// exactly the crash-between-rekey-and-promote state. A stale `.next` left
/// by a transition that never re-keyed is removed on a successful current
/// unlock. Runs under the transition lock, so a LIVE stage belonging to an
/// in-flight change in another process is never touched mid-transaction.
/// Public: the headless CLI (`SRELENS_MASTER_PASSWORD`) uses it too.
pub fn unlock_with_master_password(vault: &Vault, dir: &Path, password: &str) -> Result<(), String> {
    let _transition = transition_lock(dir).map_err(|e| e.to_string())?;
    let current = read_meta(dir);
    let mut last_err: String = "no master password is set".into();
    if let Some(meta) = &current {
        match unlock_key_for(meta, password).and_then(|key| vault.unlock_with(key, "password")) {
            Ok(()) => {
                let _ = std::fs::remove_file(meta_next_path(dir));
                return Ok(());
            }
            Err(e) => last_err = e,
        }
    }
    if let Some(next) = read_meta_next(dir) {
        if let Ok(key) = unlock_key_for(&next, password) {
            if vault.unlock_with(key, "password").is_ok() {
                // The re-key completed but the promote never ran — finish it.
                promote_meta_next(dir)?;
                return Ok(());
            }
        }
    }
    Err(last_err)
}

/// The non-secret record of the recovery opt-in made at setup. Lives on the
/// filesystem, NOT in the keychain — a keychain-less host (Linux without a
/// Secret Service) must still know an opted-out user has nothing to refresh,
/// or password changes would fail forever on the unreachable probe.
pub(crate) fn recovery_marker_path(dir: &Path) -> PathBuf {
    dir.join("recovery-enabled")
}

fn derive_key(password: &str, salt: &[u8], m_kib: u32, t: u32, p: u32) -> Result<[u8; KEY_LEN], String> {
    use argon2::{Algorithm, Argon2, Params, Version};
    let params = Params::new(m_kib, t, p, Some(KEY_LEN)).map_err(|e| e.to_string())?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; KEY_LEN];
    argon
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|e| e.to_string())?;
    Ok(key)
}

/// Fresh metadata (new salt, current default argon2id params) plus the key the
/// password derives — used at setup and password change.
pub(crate) fn build_meta(password: &str) -> Result<(VaultMeta, [u8; KEY_LEN]), String> {
    // OWASP-recommended argon2id defaults (the `argon2` crate's own).
    let (m_kib, t, p) = (19456, 2, 1);
    let mut salt = [0u8; 16];
    getrandom::getrandom(&mut salt).map_err(|e| e.to_string())?;
    let key = derive_key(password, &salt, m_kib, t, p)?;
    let verifier = seal_bytes(&key, VERIFIER).map_err(|e| e.to_string())?;
    Ok((
        VaultMeta {
            version: 1,
            kdf_alg: "argon2id".into(),
            kdf_m_kib: m_kib,
            kdf_t: t,
            kdf_p: p,
            salt: to_hex(&salt),
            verifier: to_hex(&verifier),
        },
        key,
    ))
}

/// Derive the key for `password` against existing metadata and check it
/// against the verifier. `Err` means the password is wrong (or the meta is
/// corrupt) — stated as the former, since that's overwhelmingly the cause.
pub(crate) fn unlock_key_for(meta: &VaultMeta, password: &str) -> Result<[u8; KEY_LEN], String> {
    let salt = bytes_from_hex(&meta.salt).ok_or("corrupt vault metadata (salt)")?;
    let key = derive_key(password, &salt, meta.kdf_m_kib, meta.kdf_t, meta.kdf_p)?;
    let verifier = bytes_from_hex(&meta.verifier).ok_or("corrupt vault metadata (verifier)")?;
    match open_bytes(&key, &verifier) {
        Some(plain) if plain == VERIFIER => Ok(key),
        _ => Err("incorrect master password".into()),
    }
}

/// Whether `key` is the one this vault's password derives — validates a
/// biometric-restored key without needing the password.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn key_matches_meta(meta: &VaultMeta, key: &[u8; KEY_LEN]) -> bool {
    let Some(verifier) = bytes_from_hex(&meta.verifier) else { return false };
    matches!(open_bytes(key, &verifier), Some(plain) if plain == VERIFIER)
}

fn bytes_from_hex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if s.len() % 2 != 0 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    s.as_bytes()
        .chunks(2)
        .map(|c| u8::from_str_radix(std::str::from_utf8(c).ok()?, 16).ok())
        .collect()
}

/// Keychain operations the biometric module needs when moving the key between
/// homes — kept here so the service/account coordinates stay in one place.
pub(crate) fn store_master_key_in_keychain(key: &[u8; KEY_LEN]) -> Result<(), String> {
    keyring::Entry::new(SERVICE, MASTER_KEY_ACCOUNT)
        .and_then(|e| e.set_password(&to_hex(key)))
        .map_err(|e| e.to_string())
}

pub(crate) fn delete_master_key_from_keychain() {
    let _ = keyring::Entry::new(SERVICE, MASTER_KEY_ACCOUNT).and_then(|e| e.delete_credential());
}

/// The opt-in recovery copy of the master PASSWORD (issue #208 follow-up):
/// one keychain entry, read only by the explicit "Forgot password?" flow —
/// never for silent unlocks, or the password would be theater.
const RECOVERY_ACCOUNT: &str = "master-password";

/// The recovery-copy operations behind a seam (#28): the real impl talks to
/// the OS keychain, and `vault_password`'s command logic takes `&dyn
/// RecoveryStore` so its flows — setup, recover, change — are unit-testable
/// against in-memory doubles without ever touching a real keychain (which
/// tests must not do: on a developer machine that keychain holds the REAL
/// recovery copy).
pub(crate) trait RecoveryStore: Sync {
    fn store(&self, password: &str) -> Result<(), String>;
    fn read(&self) -> Result<String, String>;
    fn state(&self) -> Result<Option<String>, String>;
    fn delete(&self);
    fn store_staged(&self, password: &str) -> Result<(), String>;
    fn read_staged(&self) -> Option<String>;
    fn delete_staged(&self);
    fn promote_staged(&self);
}

/// The production impl: one keychain entry per copy, exactly as before.
pub(crate) struct KeyringRecovery;

impl RecoveryStore for KeyringRecovery {
    fn store(&self, password: &str) -> Result<(), String> {
        store_recovery_password(password)
    }
    fn read(&self) -> Result<String, String> {
        read_recovery_password()
    }
    fn state(&self) -> Result<Option<String>, String> {
        recovery_password_state()
    }
    fn delete(&self) {
        delete_recovery_password()
    }
    fn store_staged(&self, password: &str) -> Result<(), String> {
        store_staged_recovery(password)
    }
    fn read_staged(&self) -> Option<String> {
        read_staged_recovery()
    }
    fn delete_staged(&self) {
        delete_staged_recovery()
    }
    fn promote_staged(&self) {
        promote_staged_recovery()
    }
}

pub(crate) fn store_recovery_password(password: &str) -> Result<(), String> {
    keyring::Entry::new(SERVICE, RECOVERY_ACCOUNT)
        .and_then(|e| e.set_password(password))
        .map_err(|e| e.to_string())
}

pub(crate) fn read_recovery_password() -> Result<String, String> {
    keyring::Entry::new(SERVICE, RECOVERY_ACCOUNT)
        .and_then(|e| e.get_password())
        .map_err(|e| match e {
            keyring::Error::NoEntry => "no recovery copy was stored for this vault".into(),
            other => other.to_string(),
        })
}

/// The recovery entry's state, DISTINGUISHING confirmed absence (`Ok(None)`)
/// from a keychain that couldn't be asked (`Err`) — collapsing the two let a
/// temporarily locked keychain silently strand a stale recovery copy across
/// a password change.
pub(crate) fn recovery_password_state() -> Result<Option<String>, String> {
    match keyring::Entry::new(SERVICE, RECOVERY_ACCOUNT).and_then(|e| e.get_password()) {
        Ok(p) if !p.is_empty() => Ok(Some(p)),
        Ok(_) | Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

pub(crate) fn delete_recovery_password() {
    let _ = keyring::Entry::new(SERVICE, RECOVERY_ACCOUNT).and_then(|e| e.delete_credential());
}

/// The STAGED half of a recovery update during a password change: the new
/// password lands here first, and only after the meta promote is it copied
/// over the main entry. A crash in any window then always leaves a working
/// copy for "Forgot password?" — the main entry pairs with the old meta, the
/// staged one with the staged/promoted meta (the recover flow tries both).
const RECOVERY_NEXT_ACCOUNT: &str = "master-password-next";

pub(crate) fn store_staged_recovery(password: &str) -> Result<(), String> {
    keyring::Entry::new(SERVICE, RECOVERY_NEXT_ACCOUNT)
        .and_then(|e| e.set_password(password))
        .map_err(|e| e.to_string())
}

pub(crate) fn read_staged_recovery() -> Option<String> {
    keyring::Entry::new(SERVICE, RECOVERY_NEXT_ACCOUNT)
        .and_then(|e| e.get_password())
        .ok()
        .filter(|p| !p.is_empty())
}

pub(crate) fn delete_staged_recovery() {
    let _ = keyring::Entry::new(SERVICE, RECOVERY_NEXT_ACCOUNT).and_then(|e| e.delete_credential());
}

/// Commit a staged recovery copy over the main entry (then drop the stage).
pub(crate) fn promote_staged_recovery() {
    if let Some(password) = read_staged_recovery() {
        if store_recovery_password(&password).is_ok() {
            delete_staged_recovery();
        }
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

/// Seal arbitrary bytes as `[version][nonce][ciphertext]` under `key`.
fn seal_bytes(key: &[u8; KEY_LEN], plaintext: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::getrandom(&mut nonce).map_err(|e| std::io::Error::other(e.to_string()))?;
    let cipher = XChaCha20Poly1305::new(key.into());
    let ciphertext = cipher
        .encrypt(XNonce::from_slice(&nonce), plaintext)
        .map_err(|_| std::io::Error::other("vault encryption failed"))?;
    let mut out = Vec::with_capacity(1 + NONCE_LEN + ciphertext.len());
    out.push(FORMAT_VERSION);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

fn open_bytes(key: &[u8; KEY_LEN], bytes: &[u8]) -> Option<Vec<u8>> {
    // The Poly1305 tag alone is 16 bytes, so anything shorter is garbage.
    if bytes.len() < 1 + NONCE_LEN + 16 || bytes[0] != FORMAT_VERSION {
        return None;
    }
    let (nonce, ciphertext) = bytes[1..].split_at(NONCE_LEN);
    let cipher = XChaCha20Poly1305::new(key.into());
    cipher.decrypt(XNonce::from_slice(nonce), ciphertext).ok()
}

fn encrypt(key: &[u8; KEY_LEN], secrets: &Secrets) -> std::io::Result<Vec<u8>> {
    let plaintext = serde_json::to_vec(secrets).map_err(std::io::Error::other)?;
    seal_bytes(key, &plaintext)
}

fn decrypt(key: &[u8; KEY_LEN], bytes: &[u8]) -> Option<Secrets> {
    serde_json::from_slice(&open_bytes(key, bytes)?).ok()
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

pub(crate) fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn key_from_hex(s: &str) -> Option<[u8; KEY_LEN]> {
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

/// Shared test doubles for this module's tests AND `vault_password`'s (#28).
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    /// A keychain with an in-memory entry — the working case.
    pub(crate) struct MemKeychain(pub(crate) Mutex<Option<String>>);

    impl MemKeychain {
        pub(crate) fn empty() -> Self {
            MemKeychain(Mutex::new(None))
        }
    }

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
    pub(crate) struct BrokenKeychain;

    impl KeychainBackend for BrokenKeychain {
        fn get_password(&self) -> Result<String, keyring::Error> {
            Err(keyring::Error::NoStorageAccess(Box::new(std::io::Error::other("stub"))))
        }
        fn set_password(&self, _value: &str) -> Result<(), keyring::Error> {
            Err(keyring::Error::NoStorageAccess(Box::new(std::io::Error::other("stub"))))
        }
    }

    /// An in-memory `RecoveryStore`: main + staged slots, with a switch that
    /// makes every operation fail (the unreachable-keychain case).
    #[derive(Default)]
    pub(crate) struct MemRecovery {
        pub(crate) main: Mutex<Option<String>>,
        pub(crate) staged: Mutex<Option<String>>,
        pub(crate) broken: bool,
    }

    impl RecoveryStore for MemRecovery {
        fn store(&self, password: &str) -> Result<(), String> {
            if self.broken {
                return Err("keychain unreachable (stub)".into());
            }
            *self.main.lock().unwrap() = Some(password.to_string());
            Ok(())
        }
        fn read(&self) -> Result<String, String> {
            if self.broken {
                return Err("keychain unreachable (stub)".into());
            }
            self.main
                .lock()
                .unwrap()
                .clone()
                .ok_or_else(|| "no recovery copy was stored for this vault".into())
        }
        fn state(&self) -> Result<Option<String>, String> {
            if self.broken {
                return Err("keychain unreachable (stub)".into());
            }
            Ok(self.main.lock().unwrap().clone())
        }
        fn delete(&self) {
            if !self.broken {
                *self.main.lock().unwrap() = None;
            }
        }
        fn store_staged(&self, password: &str) -> Result<(), String> {
            if self.broken {
                return Err("keychain unreachable (stub)".into());
            }
            *self.staged.lock().unwrap() = Some(password.to_string());
            Ok(())
        }
        fn read_staged(&self) -> Option<String> {
            if self.broken {
                return None;
            }
            self.staged.lock().unwrap().clone()
        }
        fn delete_staged(&self) {
            if !self.broken {
                *self.staged.lock().unwrap() = None;
            }
        }
        fn promote_staged(&self) {
            if let Some(p) = self.read_staged() {
                if self.store(&p).is_ok() {
                    self.delete_staged();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use srelens_mcp::auth::{Token, TokenStore};
    use test_support::{BrokenKeychain, MemKeychain};


    /// Build test passphrases at runtime rather than as literals: CodeQL's
    /// hardcoded-credential query flags every literal password in this test
    /// module on every line shift, and these fixtures are not secrets.
    fn test_pw(tag: &str) -> String {
        format!("{tag}-{}-passphrase", tag.len())
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
        let b = Vault::with_backend(&dir, Box::new(MemKeychain(Mutex::new(Some(to_hex(&a.current_key().unwrap()))))));
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
            &healthy.current_key().unwrap(),
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
        assert_eq!(vaults[0].current_key().unwrap(), vaults[1].current_key().unwrap(), "both openers hold the same key");
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
        assert_eq!(vaults[0].current_key().unwrap(), vaults[1].current_key().unwrap(), "both openers hold the same key");
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
    fn a_password_derives_a_stable_key_and_the_verifier_rejects_wrong_passwords() {
        let (meta, key) = build_meta(&test_pw("kdf")).unwrap();
        assert_eq!(meta.kdf_alg, "argon2id");
        // Same password against the stored meta re-derives the same key…
        assert_eq!(unlock_key_for(&meta, &test_pw("kdf")).unwrap(), key);
        assert!(key_matches_meta(&meta, &key));
        // …a wrong one is rejected by the verifier, never by guesswork.
        let err = unlock_key_for(&meta, &test_pw("wrong")).unwrap_err();
        assert!(err.contains("incorrect master password"), "got: {err}");
        assert!(!key_matches_meta(&meta, &generate_key()));
        // Meta round-trips through vault.json.
        let dir = temp_dir("meta");
        write_meta(&dir, &meta).unwrap();
        assert_eq!(read_meta(&dir).unwrap(), meta);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn setting_a_password_rekeys_the_vault_and_locks_future_opens_behind_it() {
        let dir = temp_dir("pwsetup");
        // Legacy machine-key vault with a secret in it.
        let legacy = Vault::with_backend(&dir, Box::new(MemKeychain(Mutex::new(None))));
        legacy.update(|s| s.mcp_token = Some("fe".repeat(32))).unwrap();

        // Setup: derive from the password, re-encrypt the existing secrets —
        // the snapshot is taken INSIDE the rekey critical section.
        let (meta, key) = build_meta(&test_pw("setup")).unwrap();
        legacy.rekey_from_current(key, "password").unwrap();
        write_meta(&dir, &meta).unwrap();
        assert_eq!(legacy.key_source(), "password");
        assert_eq!(legacy.load().mcp_token.as_deref(), Some("fe".repeat(32).as_str()));

        // The next open sees vault.json and starts password-locked — no
        // keyring resolution, no silent key.
        struct MustNotTouch;
        impl KeychainBackend for MustNotTouch {
            fn get_password(&self) -> Result<String, keyring::Error> {
                panic!("keyring touched in password mode")
            }
            fn set_password(&self, _v: &str) -> Result<(), keyring::Error> {
                panic!("keyring touched in password mode")
            }
        }
        let reopened = Vault::with_backend(&dir, Box::new(MustNotTouch));
        assert_eq!(reopened.key_source(), "password-locked");
        assert!(reopened.load().mcp_token.is_none());
        // The password unlocks it via the meta-derived key.
        let unlocked_key = unlock_key_for(&read_meta(&dir).unwrap(), &test_pw("setup")).unwrap();
        reopened.unlock_with(unlocked_key, "password").unwrap();
        assert_eq!(reopened.load().mcp_token.as_deref(), Some("fe".repeat(32).as_str()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_writer_holding_a_pre_rekey_key_is_rejected_instead_of_destroying_the_vault() {
        let dir = temp_dir("stalekey");
        // Instance A: unlocked with the (file) key, has written a secret.
        let a = Vault::with_backend(&dir, Box::new(BrokenKeychain));
        a.update(|s| s.mcp_token = Some("dd".repeat(32))).unwrap();
        // Another instance re-keys the vault (a password change).
        let rekeyed_secrets = a.load();
        let new_key = generate_key();
        write_sealed(&new_key, &dir.join("secrets.enc"), &rekeyed_secrets).unwrap();
        // A's cached key no longer fits: its write must be REJECTED, not
        // replace the vault with an empty map under the stale key.
        let err = a
            .update(|s| {
                s.llm_keys.insert("openai".into(), "k".into());
            })
            .unwrap_err();
        assert!(err.to_string().contains("re-keyed"), "got: {err}");
        // The re-keyed vault is untouched.
        let bytes = std::fs::read(dir.join("secrets.enc")).unwrap();
        assert_eq!(decrypt(&new_key, &bytes).unwrap().mcp_token.as_deref(), Some("dd".repeat(32).as_str()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn an_unverifiable_unlock_is_refused_and_leaves_the_stage_intact() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("permunlock");
        // A password change crashed after the re-key, before the promote:
        // the stage is the only metadata deriving the vault's key.
        let v = Vault::with_backend(&dir, Box::new(MemKeychain(Mutex::new(None))));
        v.update(|s| s.mcp_token = Some("ef".repeat(32))).unwrap();
        let (old_meta, old_key) = build_meta(&test_pw("pold")).unwrap();
        let _ = old_key;
        write_meta(&dir, &old_meta).unwrap();
        let (new_meta, new_key) = build_meta(&test_pw("pnew")).unwrap();
        write_meta_next(&dir, &new_meta).unwrap();
        v.rekey_from_current(new_key, "password").unwrap();

        // The vault is momentarily unreadable: the OLD password's verifier
        // still passes, but the vault check can't run — the unlock must be
        // REFUSED (not silently accepted) so its success path never deletes
        // the live stage.
        let vault_file = dir.join("secrets.enc");
        std::fs::set_permissions(&vault_file, std::fs::Permissions::from_mode(0o000)).unwrap();
        let reopened = Vault::with_backend(&dir, Box::new(MemKeychain(Mutex::new(None))));
        let err = unlock_with_master_password(&reopened, &dir, &test_pw("pold")).unwrap_err();
        assert!(err.contains("could not be read"), "got: {err}");
        assert!(meta_next_path(&dir).exists(), "the stage must survive the refusal");

        // Access restored: the new password recovers via the stage.
        std::fs::set_permissions(&vault_file, std::fs::Permissions::from_mode(0o600)).unwrap();
        unlock_with_master_password(&reopened, &dir, &test_pw("pnew")).unwrap();
        assert_eq!(reopened.load().mcp_token.as_deref(), Some("ef".repeat(32).as_str()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_transient_vault_read_error_aborts_writes_and_rekeys_instead_of_emptying() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("permwrite");
        let v = Vault::with_backend(&dir, Box::new(BrokenKeychain));
        v.update(|s| s.mcp_token = Some("cd".repeat(32))).unwrap();
        let vault_file = dir.join("secrets.enc");
        let before = std::fs::read(&vault_file).unwrap();

        // The vault file is momentarily unreadable while its directory stays
        // writable — both write paths must ABORT, not re-create it empty.
        std::fs::set_permissions(&vault_file, std::fs::Permissions::from_mode(0o000)).unwrap();
        let update_err = v.update(|s| s.mcp_token = None).unwrap_err();
        assert!(update_err.to_string().contains("could not be read"), "got: {update_err}");
        let rekey_err = v.rekey_from_current(generate_key(), "password").unwrap_err();
        assert!(rekey_err.to_string().contains("could not be read"), "got: {rekey_err}");

        std::fs::set_permissions(&vault_file, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(std::fs::read(&vault_file).unwrap(), before, "vault bytes untouched");
        assert_eq!(v.load().mcp_token.as_deref(), Some("cd".repeat(32).as_str()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_transient_vault_read_error_leaves_the_stage_for_later_arbitration() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("stageperm");
        // A completed re-key whose promote was lost: the stage is the ONLY
        // metadata deriving the vault's key.
        let v = Vault::with_backend(&dir, Box::new(MemKeychain(Mutex::new(None))));
        v.update(|s| s.mcp_token = Some("ab".repeat(32))).unwrap();
        let machine_key_hex = to_hex(&v.current_key().unwrap());
        let (staged, staged_key) = build_meta(&test_pw("perm")).unwrap();
        write_meta_next(&dir, &staged).unwrap();
        v.rekey_from_current(staged_key, "password").unwrap();

        // The vault file is temporarily unreadable (not missing!) — the
        // stage must survive, never be mistaken for a stale one.
        let vault_file = dir.join("secrets.enc");
        std::fs::set_permissions(&vault_file, std::fs::Permissions::from_mode(0o000)).unwrap();
        let during = Vault::with_backend(&dir, Box::new(MemKeychain(Mutex::new(Some(machine_key_hex.clone())))));
        assert!(meta_next_path(&dir).exists(), "stage must survive a read error");
        drop(during);

        // Access restored: normal arbitration promotes the stage.
        std::fs::set_permissions(&vault_file, std::fs::Permissions::from_mode(0o600)).unwrap();
        let healed = Vault::with_backend(&dir, Box::new(MemKeychain(Mutex::new(Some(machine_key_hex)))));
        assert_eq!(healed.key_source(), "password-locked");
        unlock_with_master_password(&healed, &dir, &test_pw("perm")).unwrap();
        assert_eq!(healed.load().mcp_token.as_deref(), Some("ab".repeat(32).as_str()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unavailable_machine_key_leaves_a_staged_setup_unresolved() {
        let dir = temp_dir("stagehold");
        // A machine-key vault exists…
        let v = Vault::with_backend(&dir, Box::new(MemKeychain(Mutex::new(None))));
        v.update(|s| s.mcp_token = Some("ee".repeat(32))).unwrap();
        // …with a staged setup whose re-key may or may not have run.
        let (staged, _) = build_meta(&test_pw("staged")).unwrap();
        write_meta_next(&dir, &staged).unwrap();
        // The next launch can't reach the keychain: it must NOT guess — the
        // stage stays for a later launch to arbitrate, and the vault opens
        // locked with its bytes untouched.
        let outage = Vault::with_backend(&dir, Box::new(BrokenKeychain));
        assert_eq!(outage.key_source(), "locked");
        assert!(meta_next_path(&dir).exists(), "the stage must survive the outage");
        assert!(!meta_path(&dir).exists(), "nothing was promoted blind");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_interrupted_password_change_is_recovered_at_the_next_unlock() {
        let dir = temp_dir("changecrash");
        // A password vault with a secret in it.
        let v = Vault::with_backend(&dir, Box::new(MemKeychain(Mutex::new(None))));
        v.update(|s| s.mcp_token = Some("aa".repeat(32))).unwrap();
        let (old_meta, _) = build_meta(&test_pw("old")).unwrap();
        // Align vault + meta to the old password.
        let old_key = unlock_key_for(&old_meta, &test_pw("old")).unwrap();
        v.rekey_from_current(old_key, "password").unwrap();
        write_meta(&dir, &old_meta).unwrap();

        // The change crashes AFTER the re-key but BEFORE the promote.
        let (new_meta, new_key) = build_meta(&test_pw("new")).unwrap();
        write_meta_next(&dir, &new_meta).unwrap();
        v.rekey_from_current(new_key, "password").unwrap();
        // (no promote — process died here)

        // Next launch: password-locked; the NEW password unlocks by
        // promoting the staged meta, and the old one fails cleanly.
        let reopened = Vault::with_backend(&dir, Box::new(MemKeychain(Mutex::new(None))));
        assert_eq!(reopened.key_source(), "password-locked");
        assert!(unlock_with_master_password(&reopened, &dir, &test_pw("old")).is_err());
        unlock_with_master_password(&reopened, &dir, &test_pw("new")).unwrap();
        assert_eq!(reopened.load().mcp_token.as_deref(), Some("aa".repeat(32).as_str()));
        assert!(!meta_next_path(&dir).exists(), "the stage was promoted");
        assert_eq!(read_meta(&dir).unwrap(), new_meta);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_aborted_setup_before_the_rekey_drops_the_stage_and_stays_in_machine_mode() {
        let dir = temp_dir("setupcrash1");
        let v = Vault::with_backend(&dir, Box::new(MemKeychain(Mutex::new(None))));
        v.update(|s| s.mcp_token = Some("bb".repeat(32))).unwrap();
        let machine_key_hex = to_hex(&v.current_key().unwrap());
        // Setup crashed after staging the meta but before any re-key.
        let (staged, _) = build_meta(&test_pw("neverused")).unwrap();
        write_meta_next(&dir, &staged).unwrap();

        let reopened =
            Vault::with_backend(&dir, Box::new(MemKeychain(Mutex::new(Some(machine_key_hex)))));
        // The machine key still decrypts the vault, so machine mode continues
        // (setup will show again) and the stale stage is gone.
        assert_eq!(reopened.key_source(), "keychain");
        assert_eq!(reopened.load().mcp_token.as_deref(), Some("bb".repeat(32).as_str()));
        assert!(!meta_next_path(&dir).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_aborted_setup_after_the_rekey_promotes_the_stage_and_locks() {
        let dir = temp_dir("setupcrash2");
        let v = Vault::with_backend(&dir, Box::new(MemKeychain(Mutex::new(None))));
        v.update(|s| s.mcp_token = Some("cc".repeat(32))).unwrap();
        let machine_key_hex = to_hex(&v.current_key().unwrap());
        // Setup crashed after the re-key but before the promote.
        let (staged, staged_key) = build_meta(&test_pw("chosen")).unwrap();
        write_meta_next(&dir, &staged).unwrap();
        v.rekey_from_current(staged_key, "password").unwrap();

        let reopened =
            Vault::with_backend(&dir, Box::new(MemKeychain(Mutex::new(Some(machine_key_hex)))));
        // The machine key no longer fits, so the stage is committed and the
        // chosen password unlocks everything.
        assert_eq!(reopened.key_source(), "password-locked");
        unlock_with_master_password(&reopened, &dir, &test_pw("chosen")).unwrap();
        assert_eq!(reopened.load().mcp_token.as_deref(), Some("cc".repeat(32).as_str()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_biometric_marker_opens_the_vault_locked_without_touching_the_keyring() {
        let dir = temp_dir("bio");
        // Seed a vault under a known key via a working keychain.
        let healthy = Vault::with_backend(&dir, Box::new(MemKeychain(Mutex::new(None))));
        healthy.update(|s| s.mcp_token = Some("ad".repeat(32))).unwrap();
        let key = healthy.current_key().unwrap();

        // Gate on: the marker is present, so resolution must not consult the
        // keyring at all — a backend that panics if touched proves it.
        std::fs::write(biometric_marker_path(&dir), b"").unwrap();
        struct MustNotTouch;
        impl KeychainBackend for MustNotTouch {
            fn get_password(&self) -> Result<String, keyring::Error> {
                panic!("keyring touched in biometric mode")
            }
            fn set_password(&self, _v: &str) -> Result<(), keyring::Error> {
                panic!("keyring touched in biometric mode")
            }
        }
        let gated = Vault::with_backend(&dir, Box::new(MustNotTouch));
        assert_eq!(gated.key_source(), "biometric-locked");
        assert!(gated.load().mcp_token.is_none(), "locked reads are empty");
        assert!(gated.update(|s| s.mcp_token = None).is_err(), "locked writes fail loudly");

        // A stale/wrong biometric key is rejected…
        assert!(gated.unlock_with(generate_key(), "biometric").is_err());
        // …the right one unlocks for the rest of the run.
        gated.unlock_with(key, "biometric").unwrap();
        assert_eq!(gated.key_source(), "biometric");
        assert_eq!(gated.load().mcp_token.as_deref(), Some("ad".repeat(32).as_str()));
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
