use std::collections::HashMap;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use srelens_llm::types::ProviderKind;
use srelens_llm::ProviderConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AiProvider {
    Anthropic,
    OpenAi,
    Gemini,
    OpenAiCompatible,
    Cursor,
}

impl AiProvider {
    pub fn to_llm_kind(self) -> Option<ProviderKind> {
        match self {
            AiProvider::Anthropic => Some(ProviderKind::Anthropic),
            AiProvider::OpenAi => Some(ProviderKind::OpenAi),
            AiProvider::Gemini => Some(ProviderKind::Gemini),
            AiProvider::OpenAiCompatible => Some(ProviderKind::OpenAiCompatible),
            AiProvider::Cursor => None,
        }
    }
}

/// All supported AI providers in order of display.
pub const ALL_PROVIDERS: [AiProvider; 5] = [
    AiProvider::Anthropic,
    AiProvider::OpenAi,
    AiProvider::Gemini,
    AiProvider::OpenAiCompatible,
    AiProvider::Cursor,
];

pub fn provider_slug(kind: AiProvider) -> &'static str {
    match kind {
        AiProvider::Anthropic => "anthropic",
        AiProvider::OpenAi => "openai",
        AiProvider::Gemini => "gemini",
        AiProvider::OpenAiCompatible => "openai-compatible",
        AiProvider::Cursor => "cursor",
    }
}

pub fn provider_display_name(kind: AiProvider) -> &'static str {
    match kind {
        AiProvider::Anthropic => "Anthropic (Claude)",
        AiProvider::OpenAi => "OpenAI (GPT-4o)",
        AiProvider::Gemini => "Google Gemini",
        AiProvider::OpenAiCompatible => "OpenAI-Compatible / Ollama (Local)",
        AiProvider::Cursor => "Cursor Agent (cursor-agent)",
    }
}

pub fn default_model_for_provider(kind: AiProvider) -> &'static str {
    match kind {
        AiProvider::Anthropic => "claude-3-7-sonnet-20250219",
        AiProvider::OpenAi => "gpt-4o",
        AiProvider::Gemini => "gemini-2.5-flash",
        AiProvider::OpenAiCompatible => "llama3.2",
        AiProvider::Cursor => "default",
    }
}

pub fn default_base_url_for_provider(kind: AiProvider) -> &'static str {
    match kind {
        AiProvider::Anthropic => "https://api.anthropic.com",
        AiProvider::OpenAi => "https://api.openai.com/v1",
        AiProvider::Gemini => "https://generativelanguage.googleapis.com",
        AiProvider::OpenAiCompatible => "http://localhost:11434/v1",
        AiProvider::Cursor => "",
    }
}

pub fn env_var_for_provider(kind: AiProvider) -> &'static str {
    match kind {
        AiProvider::Anthropic => "ANTHROPIC_API_KEY",
        AiProvider::OpenAi => "OPENAI_API_KEY",
        AiProvider::Gemini => "GEMINI_API_KEY",
        AiProvider::OpenAiCompatible => "OPENAI_COMPATIBLE_API_KEY",
        AiProvider::Cursor => "CURSOR_API_KEY",
    }
}

pub fn find_cursor_binary() -> Option<String> {
    if let Ok(output) = std::process::Command::new("which").arg("cursor-agent").output() {
        if output.status.success() {
            let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !s.is_empty() {
                return Some(s);
            }
        }
    }
    let home = std::env::var("HOME").unwrap_or_default();
    let candidates = [
        format!("{}/.local/bin/cursor-agent", home),
        "/usr/local/bin/cursor-agent".to_string(),
        "/opt/homebrew/bin/cursor-agent".to_string(),
    ];
    for c in candidates {
        if Path::new(&c).exists() {
            return Some(c);
        }
    }
    None
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSettings {
    pub default_provider: AiProvider,
    #[serde(default)]
    pub api_keys: HashMap<String, String>,
    #[serde(default)]
    pub models: HashMap<String, String>,
    #[serde(default)]
    pub base_urls: HashMap<String, String>,
    #[serde(default)]
    pub timeouts: HashMap<String, u32>,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u32,
}

fn default_max_tokens() -> u32 {
    4096
}

fn default_timeout_seconds() -> u32 {
    120
}

impl Default for AiSettings {
    fn default() -> Self {
        let mut models = HashMap::new();
        let mut base_urls = HashMap::new();
        let mut timeouts = HashMap::new();
        for provider in ALL_PROVIDERS {
            let slug = provider_slug(provider).to_string();
            models.insert(slug.clone(), default_model_for_provider(provider).to_string());
            base_urls.insert(slug.clone(), default_base_url_for_provider(provider).to_string());
            timeouts.insert(slug, 120);
        }

        Self {
            default_provider: AiProvider::Anthropic,
            api_keys: HashMap::new(),
            models,
            base_urls,
            timeouts,
            max_tokens: 4096,
            timeout_seconds: 120,
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
    pub fn get_api_key(&self, kind: AiProvider) -> Option<String> {
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

    pub fn get_model(&self, kind: AiProvider) -> String {
        let slug = provider_slug(kind);
        self.models
            .get(slug)
            .filter(|m| !m.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| default_model_for_provider(kind).to_string())
    }

    pub fn get_base_url(&self, kind: AiProvider) -> String {
        let slug = provider_slug(kind);
        self.base_urls
            .get(slug)
            .filter(|u| !u.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| default_base_url_for_provider(kind).to_string())
    }

    pub fn get_timeout_seconds(&self, kind: AiProvider) -> u32 {
        let slug = provider_slug(kind);
        self.timeouts
            .get(slug)
            .copied()
            .unwrap_or(if self.timeout_seconds == 0 { 120 } else { self.timeout_seconds })
    }

    pub fn set_timeout_seconds(&mut self, kind: AiProvider, seconds: u32) {
        let slug = provider_slug(kind).to_string();
        self.timeouts.insert(slug, seconds);
        self.timeout_seconds = seconds;
    }

    pub fn resolve_provider_config(&self, kind: AiProvider) -> Option<ProviderConfig> {
        let llm_kind = kind.to_llm_kind()?;
        let api_key = self.get_api_key(kind).unwrap_or_else(|| {
            if kind == AiProvider::OpenAiCompatible {
                "ollama".to_string()
            } else {
                String::new()
            }
        });

        if api_key.is_empty() && kind != AiProvider::OpenAiCompatible {
            return None;
        }

        Some(ProviderConfig {
            kind: llm_kind,
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
        assert_eq!(s.default_provider, AiProvider::Anthropic);
        assert_eq!(s.get_model(AiProvider::Anthropic), "claude-3-7-sonnet-20250219");
        assert_eq!(s.get_model(AiProvider::OpenAi), "gpt-4o");
        assert_eq!(s.get_base_url(AiProvider::OpenAiCompatible), "http://localhost:11434/v1");
        assert_eq!(s.get_model(AiProvider::Cursor), "default");
        assert_eq!(s.get_timeout_seconds(AiProvider::Anthropic), 120);
        assert_eq!(s.get_timeout_seconds(AiProvider::Cursor), 120);
    }

    #[test]
    fn test_serialization_round_trip() {
        let mut s = AiSettings::default();
        s.default_provider = AiProvider::Cursor;
        s.api_keys.insert("cursor".to_string(), "cur-test-12345".to_string());
        s.models.insert("cursor".to_string(), "claude-3.5-sonnet".to_string());
        s.set_timeout_seconds(AiProvider::Cursor, 180);
        s.set_timeout_seconds(AiProvider::Anthropic, 60);

        let json = serde_json::to_string(&s).unwrap();
        let deserialized: AiSettings = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.default_provider, AiProvider::Cursor);
        assert_eq!(deserialized.get_api_key(AiProvider::Cursor).as_deref(), Some("cur-test-12345"));
        assert_eq!(deserialized.get_model(AiProvider::Cursor), "claude-3.5-sonnet");
        assert_eq!(deserialized.get_timeout_seconds(AiProvider::Cursor), 180);
        assert_eq!(deserialized.get_timeout_seconds(AiProvider::Anthropic), 60);
        assert_eq!(deserialized.get_timeout_seconds(AiProvider::OpenAi), 120);
    }
}
