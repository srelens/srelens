use std::collections::HashMap;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use srelens_llm::types::ProviderKind;
use srelens_llm::ProviderConfig;

/// All supported AI providers in order of display.
pub const ALL_PROVIDERS: [ProviderKind; 4] = [
    ProviderKind::Anthropic,
    ProviderKind::OpenAi,
    ProviderKind::Gemini,
    ProviderKind::OpenAiCompatible,
];

pub fn provider_slug(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::Anthropic => "anthropic",
        ProviderKind::OpenAi => "openai",
        ProviderKind::Gemini => "gemini",
        ProviderKind::OpenAiCompatible => "openai-compatible",
    }
}

pub fn provider_display_name(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::Anthropic => "Anthropic (Claude)",
        ProviderKind::OpenAi => "OpenAI (GPT-4o)",
        ProviderKind::Gemini => "Google Gemini",
        ProviderKind::OpenAiCompatible => "OpenAI-Compatible / Ollama (Local)",
    }
}

pub fn default_model_for_provider(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::Anthropic => "claude-3-7-sonnet-20250219",
        ProviderKind::OpenAi => "gpt-4o",
        ProviderKind::Gemini => "gemini-2.5-flash",
        ProviderKind::OpenAiCompatible => "llama3.2",
    }
}

pub fn default_base_url_for_provider(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::Anthropic => "https://api.anthropic.com",
        ProviderKind::OpenAi => "https://api.openai.com/v1",
        ProviderKind::Gemini => "https://generativelanguage.googleapis.com",
        ProviderKind::OpenAiCompatible => "http://localhost:11434/v1",
    }
}

pub fn env_var_for_provider(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::Anthropic => "ANTHROPIC_API_KEY",
        ProviderKind::OpenAi => "OPENAI_API_KEY",
        ProviderKind::Gemini => "GEMINI_API_KEY",
        ProviderKind::OpenAiCompatible => "OLLAMA_HOST",
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSettings {
    pub default_provider: ProviderKind,
    #[serde(default)]
    pub api_keys: HashMap<String, String>,
    #[serde(default)]
    pub models: HashMap<String, String>,
    #[serde(default)]
    pub base_urls: HashMap<String, String>,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
}

fn default_max_tokens() -> u32 {
    4096
}

impl Default for AiSettings {
    fn default() -> Self {
        let mut models = HashMap::new();
        let mut base_urls = HashMap::new();
        for provider in ALL_PROVIDERS {
            let slug = provider_slug(provider).to_string();
            models.insert(slug.clone(), default_model_for_provider(provider).to_string());
            base_urls.insert(slug, default_base_url_for_provider(provider).to_string());
        }

        Self {
            default_provider: ProviderKind::Anthropic,
            api_keys: HashMap::new(),
            models,
            base_urls,
            max_tokens: 4096,
        }
    }
}

impl AiSettings {
    pub fn config_path() -> PathBuf {
        dirs::config_dir()
            .map(|p| p.join("srelens").join("ai_settings.json"))
            .unwrap_or_else(|| PathBuf::from(".srelens-ai.json"))
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(settings) = serde_json::from_str::<AiSettings>(&content) {
                return settings;
            }
        }

        // Fallback: check desktop app's settings if present
        if let Some(data_dir) = dirs::data_dir() {
            let desktop_settings_path = data_dir
                .join("app.srelens.desktop")
                .join("llm")
                .join("settings.json");
            if let Ok(content) = std::fs::read_to_string(&desktop_settings_path) {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                    let mut settings = Self::default();
                    if let Some(models) = val.get("models").and_then(|v| v.as_object()) {
                        for (k, v) in models {
                            if let Some(s) = v.as_str() {
                                settings.models.insert(k.clone(), s.to_string());
                            }
                        }
                    }
                    if let Some(base_urls) = val.get("baseUrls").and_then(|v| v.as_object()) {
                        for (k, v) in base_urls {
                            if let Some(s) = v.as_str() {
                                settings.base_urls.insert(k.clone(), s.to_string());
                            }
                        }
                    }
                    return settings;
                }
            }
        }

        Self::default()
    }

    pub fn save(&self) -> Result<PathBuf, String> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&path, json).map_err(|e| e.to_string())?;
        Ok(path)
    }

    /// Resolve API key: explicitly stored in settings or from environment variable.
    pub fn get_api_key(&self, kind: ProviderKind) -> Option<String> {
        let slug = provider_slug(kind);
        if let Some(key) = self.api_keys.get(slug).filter(|k| !k.trim().is_empty()) {
            return Some(key.clone());
        }
        // Fallback to environment variable
        let env_var = env_var_for_provider(kind);
        if let Ok(val) = std::env::var(env_var) {
            if !val.trim().is_empty() {
                return Some(val);
            }
        }
        None
    }

    pub fn get_model(&self, kind: ProviderKind) -> String {
        let slug = provider_slug(kind);
        self.models
            .get(slug)
            .filter(|m| !m.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| default_model_for_provider(kind).to_string())
    }

    pub fn get_base_url(&self, kind: ProviderKind) -> String {
        let slug = provider_slug(kind);
        self.base_urls
            .get(slug)
            .filter(|u| !u.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| default_base_url_for_provider(kind).to_string())
    }

    pub fn resolve_provider_config(&self, kind: ProviderKind) -> Option<ProviderConfig> {
        let api_key = self.get_api_key(kind).unwrap_or_else(|| {
            if kind == ProviderKind::OpenAiCompatible {
                "ollama".to_string()
            } else {
                String::new()
            }
        });

        if api_key.is_empty() && kind != ProviderKind::OpenAiCompatible {
            return None;
        }

        Some(ProviderConfig {
            kind,
            api_key,
            base_url: self.get_base_url(kind),
            model: self.get_model(kind),
            max_tokens: self.max_tokens,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_ai_settings() {
        let s = AiSettings::default();
        assert_eq!(s.default_provider, ProviderKind::Anthropic);
        assert_eq!(s.get_model(ProviderKind::Anthropic), "claude-3-7-sonnet-20250219");
        assert_eq!(s.get_model(ProviderKind::OpenAi), "gpt-4o");
        assert_eq!(s.get_base_url(ProviderKind::OpenAiCompatible), "http://localhost:11434/v1");
    }

    #[test]
    fn test_serialization_round_trip() {
        let mut s = AiSettings::default();
        s.default_provider = ProviderKind::OpenAi;
        s.api_keys.insert("openai".to_string(), "sk-test-12345".to_string());
        s.models.insert("openai".to_string(), "gpt-4.5-preview".to_string());

        let json = serde_json::to_string(&s).unwrap();
        let deserialized: AiSettings = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.default_provider, ProviderKind::OpenAi);
        assert_eq!(deserialized.get_api_key(ProviderKind::OpenAi).as_deref(), Some("sk-test-12345"));
        assert_eq!(deserialized.get_model(ProviderKind::OpenAi), "gpt-4.5-preview");
    }
}
