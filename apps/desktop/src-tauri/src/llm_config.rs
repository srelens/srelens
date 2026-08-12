//! Storage for the native LLM agent's per-provider API keys (in the encrypted
//! secrets vault — see `vault.rs`) and its non-secret settings (default
//! provider, chosen model + base URL per provider). Pure over the vault and
//! paths passed in, so everything is testable without touching the real app
//! config dir or any keychain.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use srelens_llm::types::ProviderKind;
use srelens_llm::ProviderConfig;

use crate::vault::Vault;
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

/// Where a pre-vault build kept this provider's plaintext fallback key. Only
/// used to delete that stale copy when the key is rewritten or cleared —
/// never read (the vault is the sole source of truth).
fn legacy_key_file(dir: &Path, kind: ProviderKind) -> PathBuf {
    dir.join(format!("llm-key-{}", slug(kind)))
}

/// Read a provider's API key from the vault.
pub fn get_key(vault: &Vault, kind: ProviderKind) -> Option<String> {
    vault.load().llm_keys.get(slug(kind)).cloned().filter(|s| !s.is_empty())
}

/// Store a provider's API key in the vault. Also best-effort deletes the
/// plaintext fallback file a pre-vault build may have left under `dir`, so a
/// rewritten key's stale copy doesn't linger on disk.
pub fn set_key(vault: &Vault, dir: &Path, kind: ProviderKind, key: &str) -> Result<(), String> {
    let value = key.to_string();
    vault
        .update(|s| {
            s.llm_keys.insert(slug(kind).to_string(), value);
        })
        .map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(legacy_key_file(dir, kind));
    Ok(())
}

/// Remove a provider's API key from the vault (and any stale pre-vault
/// plaintext copy under `dir`).
pub fn clear_key(vault: &Vault, dir: &Path, kind: ProviderKind) -> Result<(), String> {
    vault
        .update(|s| {
            s.llm_keys.remove(slug(kind));
        })
        .map_err(|e| e.to_string())?;
    match std::fs::remove_file(legacy_key_file(dir, kind)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

pub fn has_key(vault: &Vault, kind: ProviderKind) -> bool {
    get_key(vault, kind).is_some()
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
pub fn provider_config(vault: &Vault, settings: &LlmSettings, kind: ProviderKind) -> Option<ProviderConfig> {
    let api_key = get_key(vault, kind)?;
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

    #[test]
    fn keys_round_trip_through_the_vault_and_clear_removes_them() {
        let dir = temp();
        let vault = crate::vault::test_vault(&dir);
        assert!(get_key(&vault, ProviderKind::Anthropic).is_none());
        assert!(!has_key(&vault, ProviderKind::Anthropic));

        set_key(&vault, &dir, ProviderKind::Anthropic, "sk-ant-1").unwrap();
        assert_eq!(get_key(&vault, ProviderKind::Anthropic).as_deref(), Some("sk-ant-1"));
        assert!(has_key(&vault, ProviderKind::Anthropic));
        // Providers are isolated.
        assert!(get_key(&vault, ProviderKind::OpenAi).is_none());

        clear_key(&vault, &dir, ProviderKind::Anthropic).unwrap();
        assert!(get_key(&vault, ProviderKind::Anthropic).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn writing_a_key_deletes_the_stale_pre_vault_plaintext_copy() {
        let dir = temp();
        let vault = crate::vault::test_vault(&dir);
        // A pre-vault build left the key as a plaintext file.
        std::fs::write(legacy_key_file(&dir, ProviderKind::Gemini), "old-plaintext").unwrap();
        set_key(&vault, &dir, ProviderKind::Gemini, "g-new").unwrap();
        assert!(!legacy_key_file(&dir, ProviderKind::Gemini).exists(), "stale plaintext copy removed");
        assert_eq!(get_key(&vault, ProviderKind::Gemini).as_deref(), Some("g-new"));
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
    fn provider_config_is_none_without_a_key_and_uses_defaults_with_one() {
        let dir = temp();
        let vault = crate::vault::test_vault(&dir);
        let settings = LlmSettings::default();
        // No key configured.
        assert!(provider_config(&vault, &settings, ProviderKind::OpenAi).is_none());

        set_key(&vault, &dir, ProviderKind::OpenAi, "sk-test").unwrap();
        let cfg = provider_config(&vault, &settings, ProviderKind::OpenAi).unwrap();
        assert_eq!(cfg.api_key, "sk-test");
        assert_eq!(cfg.base_url, ProviderConfig::default_base_url(ProviderKind::OpenAi));
        assert_eq!(cfg.max_tokens, DEFAULT_MAX_TOKENS);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_custom_base_url_overrides_the_default() {
        let dir = temp();
        let vault = crate::vault::test_vault(&dir);
        let mut settings = LlmSettings::default();
        settings.base_urls.insert("openai-compatible".into(), "http://localhost:11434/v1".into());
        set_key(&vault, &dir, ProviderKind::OpenAiCompatible, "k").unwrap();
        let cfg = provider_config(&vault, &settings, ProviderKind::OpenAiCompatible).unwrap();
        assert_eq!(cfg.base_url, "http://localhost:11434/v1");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
