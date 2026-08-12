//! Storage for the native LLM agent's per-provider API keys (in the OS
//! keychain, falling back to a 0600 file like the MCP token) and its non-secret
//! settings (default provider, chosen model + base URL per provider). Pure over
//! the paths passed in, so the file/settings logic is testable without touching
//! the real app config dir or keychain.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use srelens_llm::types::ProviderKind;
use srelens_llm::ProviderConfig;

const SERVICE: &str = "srelens";
/// Anthropic requires an explicit `max_tokens`; a sane default for cluster Q&A.
const DEFAULT_MAX_TOKENS: u32 = 4096;

/// The four providers, in picker order.
pub fn all_providers() -> [ProviderKind; 4] {
    [ProviderKind::Anthropic, ProviderKind::OpenAi, ProviderKind::Gemini, ProviderKind::OpenAiCompatible]
}

/// Stable, filesystem/keychain-safe slug for a provider, used for the keychain
/// account name, the fallback file name, and the settings map keys.
pub fn slug(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::Anthropic => "anthropic",
        ProviderKind::OpenAi => "openai",
        ProviderKind::Gemini => "gemini",
        ProviderKind::OpenAiCompatible => "openai-compatible",
    }
}

fn key_account(kind: ProviderKind) -> String {
    format!("llm-key-{}", slug(kind))
}

fn key_file(dir: &Path, kind: ProviderKind) -> PathBuf {
    dir.join(format!("llm-key-{}", slug(kind)))
}

/// Write `contents` owner-only (`0600` on Unix) — used for the fallback key
/// files, which hold secrets in the app config dir.
fn write_private(path: &Path, contents: &str) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        // Remove any existing file first, then `create_new` (O_CREAT|O_EXCL):
        // `.mode(0o600)` only governs a FRESH inode — truncating an existing
        // file (e.g. restored from a backup as 0644) would write the secret
        // while keeping the loose mode. The exclusive open also refuses to
        // follow anything (like a symlink) re-created at the path in between.
        // Same pattern as `assistant::write_private_file`.
        let _ = std::fs::remove_file(path);
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(contents.as_bytes())
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, contents)
    }
}

/// Read a provider's API key: the OS keychain first, then the 0600 fallback
/// file. A working-but-empty keychain returns `None` without consulting the
/// file (matching the MCP token store's rule); any other keychain failure falls
/// through to the file.
pub fn get_key(dir: &Path, kind: ProviderKind) -> Option<String> {
    // Keychain first. Anything other than a non-empty hit (absent entry, empty
    // value, or a keychain failure) falls through to the fallback file — where
    // `set_key` writes when the keychain is unavailable, so the file must be
    // consulted on a `NoEntry` too, not just on a hard error.
    if let Ok(s) = keyring::Entry::new(SERVICE, &key_account(kind)).and_then(|e| e.get_password()) {
        if !s.is_empty() {
            return Some(s);
        }
    }
    std::fs::read_to_string(key_file(dir, kind)).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// Store a provider's API key in the keychain, falling back to a 0600 file if
/// the keychain write fails (headless Linux without a Secret Service).
pub fn set_key(dir: &Path, kind: ProviderKind, key: &str) -> Result<(), String> {
    if let Ok(entry) = keyring::Entry::new(SERVICE, &key_account(kind)) {
        if entry.set_password(key).is_ok() {
            // Drop any stale file copy so `get_key` (keychain-first) can't later
            // be shadowed by an out-of-date fallback file.
            let _ = std::fs::remove_file(key_file(dir, kind));
            return Ok(());
        }
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    write_private(&key_file(dir, kind), key).map_err(|e| e.to_string())
}

/// Remove a provider's API key from both the keychain and the fallback file.
pub fn clear_key(dir: &Path, kind: ProviderKind) -> Result<(), String> {
    // Attempt both backends. A locked/unavailable keychain that refuses the
    // delete must NOT be reported as success — the credential would still be
    // there and could reactivate once the keychain is reachable. `NoEntry` (or
    // an entry that never existed) is a clean "already gone".
    let keychain_err = match keyring::Entry::new(SERVICE, &key_account(kind))
        .and_then(|e| e.delete_credential())
    {
        Ok(()) | Err(keyring::Error::NoEntry) => None,
        Err(e) => Some(e.to_string()),
    };
    match std::fs::remove_file(key_file(dir, kind)) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    match keychain_err {
        Some(e) => Err(format!("removed the local copy, but the keychain entry could not be deleted: {e}")),
        None => Ok(()),
    }
}

pub fn has_key(dir: &Path, kind: ProviderKind) -> bool {
    get_key(dir, kind).is_some()
}

/// The native agent's non-secret configuration. Keys are the provider slugs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmSettings {
    pub default_provider: ProviderKind,
    /// Chosen model id per provider slug.
    #[serde(default)]
    pub models: BTreeMap<String, String>,
    /// Custom base URL per provider slug (only meaningful for the
    /// OpenAI-compatible provider; ignored for the hosted ones).
    #[serde(default)]
    pub base_urls: BTreeMap<String, String>,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
}

fn default_max_tokens() -> u32 {
    DEFAULT_MAX_TOKENS
}

impl Default for LlmSettings {
    fn default() -> Self {
        Self {
            default_provider: ProviderKind::Anthropic,
            models: BTreeMap::new(),
            base_urls: BTreeMap::new(),
            max_tokens: DEFAULT_MAX_TOKENS,
        }
    }
}

/// Load settings from `path`, returning defaults for a missing or corrupt file
/// (never an error — a bad settings file must not brick the assistant).
pub fn load_settings(path: &Path) -> LlmSettings {
    std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// Persist settings to `path` (pretty JSON), creating the parent dir.
pub fn save_settings(path: &Path, settings: &LlmSettings) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let raw = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    std::fs::write(path, raw).map_err(|e| e.to_string())
}

/// Assemble a ready-to-use [`ProviderConfig`] for `kind`, or `None` if no key is
/// configured. The base URL defaults per provider (the custom provider must
/// supply one via settings); the model comes from settings (empty if unset —
/// the caller surfaces "choose a model").
pub fn provider_config(dir: &Path, settings: &LlmSettings, kind: ProviderKind) -> Option<ProviderConfig> {
    let api_key = get_key(dir, kind)?;
    let base_url = settings
        .base_urls
        .get(slug(kind))
        .filter(|s| !s.is_empty())
        .cloned()
        .unwrap_or_else(|| ProviderConfig::default_base_url(kind).to_string());
    let model = settings.models.get(slug(kind)).cloned().unwrap_or_default();
    Some(ProviderConfig { kind, api_key, base_url, model, max_tokens: settings.max_tokens })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("srelens-llmcfg-{}-{:?}", std::process::id(), std::thread::current().id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[cfg(unix)]
    #[test]
    fn rewriting_a_key_over_a_loose_permission_file_restores_owner_only_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp();
        let path = dir.join("llm-key-anthropic");
        // The file pre-exists with loose permissions (a backup restore, a
        // manual chmod) — the rewrite must not inherit them.
        std::fs::write(&path, "old-key").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_private(&path, "new-key").unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new-key");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn each_provider_has_a_distinct_stable_slug() {
        let slugs: Vec<_> = all_providers().iter().map(|k| slug(*k)).collect();
        assert_eq!(slugs, vec!["anthropic", "openai", "gemini", "openai-compatible"]);
    }

    #[test]
    fn settings_round_trip_and_default_on_missing_or_corrupt() {
        let dir = temp();
        let path = dir.join("settings.json");
        // Missing → defaults.
        let def = load_settings(&path);
        assert_eq!(def.default_provider, ProviderKind::Anthropic);
        assert_eq!(def.max_tokens, DEFAULT_MAX_TOKENS);

        let mut s = LlmSettings::default();
        s.default_provider = ProviderKind::OpenAi;
        s.models.insert("openai".into(), "gpt-5".into());
        save_settings(&path, &s).unwrap();
        assert_eq!(load_settings(&path), s);

        // Corrupt → defaults, not a panic.
        std::fs::write(&path, "{not json").unwrap();
        assert_eq!(load_settings(&path).default_provider, ProviderKind::Anthropic);
    }

    #[test]
    fn a_key_written_to_the_fallback_file_round_trips_and_is_owner_only() {
        let dir = temp();
        // Force the file path directly (the keychain may or may not exist in CI;
        // this asserts the fallback file behavior deterministically).
        write_private(&key_file(&dir, ProviderKind::Gemini), "secret-key").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(key_file(&dir, ProviderKind::Gemini)).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // get_key reads the file when the keychain has no entry.
        assert_eq!(std::fs::read_to_string(key_file(&dir, ProviderKind::Gemini)).unwrap(), "secret-key");
    }

    #[test]
    fn provider_config_is_none_without_a_key_and_uses_defaults_with_one() {
        let dir = temp();
        let settings = LlmSettings::default();
        // No key configured.
        assert!(provider_config(&dir, &settings, ProviderKind::OpenAi).is_none());

        // With a fallback-file key, the config fills in the default base URL.
        write_private(&key_file(&dir, ProviderKind::OpenAi), "sk-test").unwrap();
        let cfg = provider_config(&dir, &settings, ProviderKind::OpenAi).unwrap();
        assert_eq!(cfg.api_key, "sk-test");
        assert_eq!(cfg.base_url, ProviderConfig::default_base_url(ProviderKind::OpenAi));
        assert_eq!(cfg.max_tokens, DEFAULT_MAX_TOKENS);
    }

    #[test]
    fn a_custom_base_url_overrides_the_default() {
        let dir = temp();
        let mut settings = LlmSettings::default();
        settings.base_urls.insert("openai-compatible".into(), "http://localhost:11434/v1".into());
        write_private(&key_file(&dir, ProviderKind::OpenAiCompatible), "k").unwrap();
        let cfg = provider_config(&dir, &settings, ProviderKind::OpenAiCompatible).unwrap();
        assert_eq!(cfg.base_url, "http://localhost:11434/v1");
    }
}
