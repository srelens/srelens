//! Apps' secret settings (#543), kept in the desktop's existing vault.
//!
//! Not a second store: an app secret is one more entry in `secrets.enc`,
//! under the ONE master key the OS keychain holds (`vault.rs`), beside the
//! MCP token and the provider API keys. No per-secret keychain item, no other
//! file. The registry (`srelens-registry`, Tauri-free) sees only the
//! [`SecretStore`] trait this implements.
//!
//! **Fails closed.** A secret is stored only while the vault's key is
//! protected by something other than the file beside it — the OS keychain,
//! the biometric gate, or the master password — and the vault is unlocked.
//! With no usable keychain (`file`, the headless fallback) or a locked vault
//! it refuses and says why; nothing is ever written in plain text instead.
//! Deleting (`retain`) needs only an unlocked vault, since removing a secret
//! can never expose one.
use crate::vault::Vault;
use srelens_registry::{SecretStore, SecretValue};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

/// The desktop's [`SecretStore`]: the vault, once it is open.
pub struct VaultSecretStore {
    vault: OnceLock<Arc<Vault>>,
    /// Where to open the vault on first use, for a headless process that has
    /// not opened one itself. `None`: the GUI attaches the vault it manages.
    open_at: Option<PathBuf>,
}

impl VaultSecretStore {
    /// Over a vault already open (headless `--mcp-http`, tests).
    pub fn with(vault: Arc<Vault>) -> Self {
        let store = Self::attached_later();
        store.attach(vault);
        store
    }

    /// The GUI's: the registry is built before `setup` opens the vault, which
    /// is then handed over with [`attach`](Self::attach). Until then the store
    /// is unavailable, and says so.
    pub fn attached_later() -> Self {
        Self {
            vault: OnceLock::new(),
            open_at: None,
        }
    }

    /// Opens the vault under `dir` the first time a secret is asked for, so a
    /// headless run that never touches one never touches the keychain.
    pub fn opening(dir: PathBuf) -> Self {
        Self {
            vault: OnceLock::new(),
            open_at: Some(dir),
        }
    }

    /// The vault the GUI manages: the SAME instance the password and
    /// biometric unlock act on, so unlocking it makes the store available.
    pub fn attach(&self, vault: Arc<Vault>) {
        let _ = self.vault.set(vault);
    }

    fn vault(&self) -> Result<&Arc<Vault>, String> {
        if let Some(vault) = self.vault.get() {
            return Ok(vault);
        }
        match &self.open_at {
            Some(dir) => Ok(self.vault.get_or_init(|| Arc::new(Vault::open(dir)))),
            None => Err("The secrets vault is not open yet".into()),
        }
    }

    /// The vault, unlocked, whatever protects its key.
    fn unlocked(&self) -> Result<&Arc<Vault>, String> {
        let vault = self.vault()?;
        if vault.is_unlocked() {
            Ok(vault)
        } else {
            Err(unavailable(vault.key_source()).into())
        }
    }
}

/// Why a vault whose key comes from `source` cannot keep an app secret.
fn unavailable(source: &str) -> &'static str {
    match source {
        "file" => "This system has no usable keychain, so srelens cannot protect an app's secret",
        "password-locked" => "The secrets vault is locked. Unlock it with your master password",
        "biometric-locked" => {
            "The secrets vault is locked behind biometric unlock. Unlock it in srelens"
        }
        "locked" => "The system keychain holding the secrets vault's key can't be reached",
        _ => "The secrets vault cannot keep a secret right now",
    }
}

impl SecretStore for VaultSecretStore {
    fn status(&self) -> Result<(), String> {
        let vault = self.unlocked()?;
        match vault.key_source() {
            "keychain" | "biometric" | "password" => Ok(()),
            other => Err(unavailable(other).into()),
        }
    }

    fn put(&self, key: &str, value: &SecretValue) -> Result<(), String> {
        self.status()?;
        let (key, value) = (key.to_owned(), value.expose().to_owned());
        self.unlocked()?
            .update(move |secrets| {
                secrets.extension_secrets.insert(key, value);
            })
            .map_err(|e| e.to_string())
    }

    fn contains(&self, key: &str) -> Result<bool, String> {
        Ok(self.unlocked()?.load().extension_secrets.contains_key(key))
    }

    fn retain(&self, keep: &BTreeSet<String>) -> Result<(), String> {
        let vault = self.unlocked()?;
        // Rewrite the vault only when something goes: most changes delete
        // nothing, and each write is a keychain-keyed re-encryption.
        if vault
            .load()
            .extension_secrets
            .keys()
            .all(|key| keep.contains(key))
        {
            return Ok(());
        }
        vault
            .update(|secrets| {
                secrets
                    .extension_secrets
                    .retain(|key, _| keep.contains(key))
            })
            .map_err(|e| e.to_string())
    }

    fn reveal(&self, key: &str) -> Result<Option<SecretValue>, String> {
        self.status()?;
        Ok(self
            .unlocked()?
            .load()
            .extension_secrets
            .get(key)
            .cloned()
            .map(SecretValue::new))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::test_support::{BrokenKeychain, MemKeychain};
    use crate::vault::Vault;
    use srelens_registry::{SecretStore, SecretValue};
    use std::collections::BTreeSet;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    const SECRET: &str = "grafana-cloud-glc_9f2e-secret";
    const KEY: &str = "org.example.metrics/token";

    fn temp_dir(label: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "srelens-ext-secrets-{label}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn keychain_vault(dir: &std::path::Path) -> Arc<Vault> {
        Arc::new(Vault::with_backend(dir, Box::new(MemKeychain::empty())))
    }

    #[test]
    fn a_keychain_backed_vault_keeps_an_app_secret_encrypted_in_its_one_file() {
        let dir = temp_dir("keychain");
        let vault = keychain_vault(&dir);
        let store = VaultSecretStore::with(vault.clone());
        assert_eq!(store.status(), Ok(()));
        store.put(KEY, &SecretValue::new(SECRET.into())).unwrap();

        assert_eq!(store.reveal(KEY).unwrap().unwrap().expose(), SECRET);
        assert_eq!(store.contains(KEY), Ok(true));
        assert_eq!(
            vault.load().extension_secrets.get(KEY).map(String::as_str),
            Some(SECRET)
        );
        // The same `secrets.enc` the MCP token lives in, and never in the clear.
        let raw = std::fs::read(dir.join("secrets.enc")).unwrap();
        assert!(!raw.windows(SECRET.len()).any(|w| w == SECRET.as_bytes()));
        // No second storage scheme: nothing else was written beside it.
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| !name.ends_with(".lock"))
            .collect();
        names.sort();
        assert_eq!(names, vec!["secrets.enc".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn with_no_keychain_it_refuses_rather_than_keep_a_secret_beside_its_key() {
        let dir = temp_dir("no-keychain");
        // The headless case: the master key falls back to a file next to the
        // vault, which protects nothing an app token needs protecting from.
        let vault = Arc::new(Vault::with_backend(&dir, Box::new(BrokenKeychain)));
        assert_eq!(vault.key_source(), "file");
        let store = VaultSecretStore::with(vault.clone());
        let why = store.status().unwrap_err();
        assert!(why.contains("keychain"), "{why}");
        assert!(store.put(KEY, &SecretValue::new(SECRET.into())).is_err());
        assert!(vault.load().extension_secrets.is_empty());
        let raw = std::fs::read(dir.join("secrets.enc")).unwrap_or_default();
        assert!(!raw.windows(SECRET.len()).any(|w| w == SECRET.as_bytes()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_locked_vault_refuses_and_says_so() {
        let dir = temp_dir("locked");
        let keychain = Arc::new(Mutex::new(None));
        struct Shared(Arc<Mutex<Option<String>>>);
        impl crate::vault::KeychainBackend for Shared {
            fn get_password(&self) -> Result<String, keyring::Error> {
                self.0
                    .lock()
                    .unwrap()
                    .clone()
                    .ok_or(keyring::Error::NoEntry)
            }
            fn set_password(&self, value: &str) -> Result<(), keyring::Error> {
                *self.0.lock().unwrap() = Some(value.into());
                Ok(())
            }
        }
        let first = Arc::new(Vault::with_backend(
            &dir,
            Box::new(Shared(keychain.clone())),
        ));
        VaultSecretStore::with(first)
            .put(KEY, &SecretValue::new(SECRET.into()))
            .unwrap();
        // Next launch, the keychain is unreachable: the vault opens locked.
        let locked = Arc::new(Vault::with_backend(&dir, Box::new(BrokenKeychain)));
        assert_eq!(locked.key_source(), "locked");
        let store = VaultSecretStore::with(locked);
        assert!(store.status().unwrap_err().contains("can't be reached"));
        assert!(store.put(KEY, &SecretValue::new("other".into())).is_err());
        assert!(store.reveal(KEY).is_err(), "a locked store reveals nothing");
        assert!(
            store.contains(KEY).is_err(),
            "and does not claim it is gone"
        );
        assert!(store.retain(&BTreeSet::new()).is_err(), "nor deletes blind");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_store_whose_vault_is_not_open_yet_is_unavailable() {
        let store = VaultSecretStore::attached_later();
        assert!(store.status().unwrap_err().contains("not open"));
        assert!(store.put(KEY, &SecretValue::new(SECRET.into())).is_err());
        let dir = temp_dir("later");
        store.attach(keychain_vault(&dir));
        assert_eq!(store.status(), Ok(()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn retain_deletes_only_app_secrets_it_is_not_told_to_keep() {
        let dir = temp_dir("retain");
        let vault = keychain_vault(&dir);
        vault
            .update(|s| {
                s.mcp_token = Some("ab".repeat(32));
                s.llm_keys.insert("anthropic".into(), "sk-ant".into());
            })
            .unwrap();
        let store = VaultSecretStore::with(vault.clone());
        store.put(KEY, &SecretValue::new(SECRET.into())).unwrap();
        store
            .put("org.example.gone/token", &SecretValue::new("old".into()))
            .unwrap();

        store.retain(&BTreeSet::from([KEY.to_string()])).unwrap();
        let secrets = vault.load();
        assert_eq!(
            secrets.extension_secrets.keys().collect::<Vec<_>>(),
            vec![KEY]
        );
        assert!(secrets.mcp_token.is_some(), "the MCP token is not an app's");
        assert_eq!(secrets.llm_keys.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_vault_contents_never_print() {
        let mut secrets = crate::vault::Secrets::default();
        secrets.extension_secrets.insert(KEY.into(), SECRET.into());
        secrets.mcp_token = Some("cd".repeat(32));
        let printed = format!("{secrets:?}");
        assert!(!printed.contains(SECRET), "{printed}");
        assert!(!printed.contains(&"cd".repeat(32)), "{printed}");
        assert!(
            printed.contains(KEY),
            "which app secrets are held is not secret: {printed}"
        );
    }
}
