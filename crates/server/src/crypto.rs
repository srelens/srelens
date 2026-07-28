//! Master-key crypto for secrets at rest (per-user kubeconfigs). AES-256-GCM
//! with a fresh random nonce per seal; the 32-byte master key comes from
//! `SRELENS_MASTER_KEY` (64 hex chars) or an auto-generated key file
//! (`<data>/master.key`, raw 32 bytes, mode 0600).

use std::io::Write;
use std::path::Path;

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, Key, KeyInit, Nonce};

/// Ciphertext plus the nonce it was sealed with. Both are stored per row;
/// the nonce is not secret, but it must never be reused with the same key —
/// hence a fresh random nonce on every seal.
pub struct Sealed {
    pub ciphertext: Vec<u8>,
    pub nonce: Vec<u8>,
}

pub struct MasterKey {
    cipher: Aes256Gcm,
}

impl MasterKey {
    /// Build from a 64-hex-char string (the `SRELENS_MASTER_KEY` format).
    pub fn from_hex(hex_str: &str) -> Result<Self, String> {
        let bytes = hex::decode(hex_str.trim())
            .map_err(|e| format!("SRELENS_MASTER_KEY is not valid hex: {e}"))?;
        Self::from_bytes(&bytes)
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() != 32 {
            return Err(format!("master key must be 32 bytes, got {}", bytes.len()));
        }
        let key = Key::<Aes256Gcm>::from_slice(bytes);
        Ok(Self {
            cipher: Aes256Gcm::new(key),
        })
    }

    /// Server mode: the key MUST come from `SRELENS_MASTER_KEY`, never a file on
    /// disk. Writing the key next to the sealed database would defeat at-rest
    /// encryption — the persistent volume would hold both the ciphertext and the
    /// key that opens it — so a missing or blank value is a hard startup error.
    pub fn require_env(env_value: Option<&str>) -> Result<Self, String> {
        match env_value {
            Some(hex) if !hex.trim().is_empty() => Self::from_hex(hex),
            _ => Err(
                "SRELENS_MASTER_KEY must be set in server mode (64 hex chars = 32 bytes). \
                 Refusing to persist a generated key next to the encrypted database. \
                 Generate one with: openssl rand -hex 32"
                    .into(),
            ),
        }
    }

    /// Load the key: env value wins; else read `key_file`; else generate a new
    /// key into `key_file` (0600, created atomically with O_EXCL semantics).
    /// Used by the single-user desktop app; the server uses [`require_env`].
    pub fn load_or_generate(env_value: Option<&str>, key_file: &Path) -> Result<Self, String> {
        if let Some(hex_key) = env_value {
            if !hex_key.trim().is_empty() {
                return Self::from_hex(hex_key);
            }
        }
        match std::fs::read(key_file) {
            Ok(bytes) => Self::from_bytes(&bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let mut bytes = [0u8; 32];
                getrandom::getrandom(&mut bytes).map_err(|e| e.to_string())?;
                let mut opts = std::fs::OpenOptions::new();
                opts.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    opts.mode(0o600);
                }
                let mut file = opts.open(key_file).map_err(|e| e.to_string())?;
                file.write_all(&bytes).map_err(|e| e.to_string())?;
                Self::from_bytes(&bytes)
            }
            Err(e) => Err(format!("cannot read {}: {e}", key_file.display())),
        }
    }

    /// Encrypt `plaintext` under a fresh random 96-bit nonce.
    pub fn seal(&self, plaintext: &[u8]) -> Result<Sealed, String> {
        let mut nonce_bytes = [0u8; 12];
        getrandom::getrandom(&mut nonce_bytes).map_err(|e| e.to_string())?;
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = self
            .cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| format!("encrypt failed: {e}"))?;
        Ok(Sealed {
            ciphertext,
            nonce: nonce_bytes.to_vec(),
        })
    }

    /// Decrypt; fails on tampered ciphertext, wrong nonce, or a rotated key.
    pub fn open(&self, sealed: &Sealed) -> Result<Vec<u8>, String> {
        if sealed.nonce.len() != 12 {
            return Err("invalid nonce length".into());
        }
        let nonce = Nonce::from_slice(&sealed.nonce);
        self.cipher
            .decrypt(nonce, sealed.ciphertext.as_slice())
            .map_err(|_| "decryption failed (tampered data or rotated master key)".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> MasterKey {
        MasterKey::from_hex(&"ab".repeat(32)).unwrap()
    }

    #[test]
    fn seal_open_round_trips() {
        let k = key();
        let sealed = k.seal(b"apiVersion: v1").unwrap();
        assert_ne!(sealed.ciphertext, b"apiVersion: v1");
        assert_eq!(k.open(&sealed).unwrap(), b"apiVersion: v1");
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let k = key();
        let mut sealed = k.seal(b"secret").unwrap();
        sealed.ciphertext[0] ^= 0xff;
        assert!(k.open(&sealed).is_err());
    }

    #[test]
    fn require_env_rejects_missing_or_blank_and_accepts_hex() {
        // Server mode must refuse to run without an env-provided key (so the key
        // is never written to disk), and must not fall back to a file.
        assert!(MasterKey::require_env(None).is_err());
        assert!(MasterKey::require_env(Some("   ")).is_err());
        assert!(MasterKey::require_env(Some("nothex")).is_err());
        assert!(MasterKey::require_env(Some(&"ab".repeat(32))).is_ok());
    }

    #[test]
    fn different_key_fails_to_open() {
        let sealed = key().seal(b"secret").unwrap();
        let other = MasterKey::from_hex(&"cd".repeat(32)).unwrap();
        assert!(other.open(&sealed).is_err());
    }

    #[test]
    fn nonces_are_unique_per_seal() {
        let k = key();
        let a = k.seal(b"x").unwrap();
        let b = k.seal(b"x").unwrap();
        assert_ne!(a.nonce, b.nonce);
        assert_ne!(a.ciphertext, b.ciphertext);
    }

    #[test]
    fn from_hex_rejects_wrong_length() {
        assert!(MasterKey::from_hex("abcd").is_err());
        assert!(MasterKey::from_hex("zz").is_err());
    }

    #[test]
    fn load_or_generate_creates_0600_file_and_is_stable() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("master.key");
        let k1 = MasterKey::load_or_generate(None, &path).unwrap();
        assert!(path.is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // A second load reads the same key: what k1 sealed, k2 opens.
        let sealed = k1.seal(b"stable").unwrap();
        let k2 = MasterKey::load_or_generate(None, &path).unwrap();
        assert_eq!(k2.open(&sealed).unwrap(), b"stable");
    }

    #[test]
    fn env_value_wins_over_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("master.key");
        let hex_key = "ef".repeat(32);
        let k = MasterKey::load_or_generate(Some(&hex_key), &path).unwrap();
        // No file is created when the env key is used.
        assert!(!path.exists());
        let sealed = k.seal(b"via-env").unwrap();
        assert_eq!(
            MasterKey::from_hex(&hex_key)
                .unwrap()
                .open(&sealed)
                .unwrap(),
            b"via-env"
        );
    }
}
