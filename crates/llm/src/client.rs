//! The concrete network `Provider`: one HTTP client that speaks all four
//! backends, delegating request construction and stream parsing to the pure
//! per-provider adapters. Only the transport lives here — the SSE framing
//! ([`SseDecoder`]) is pure and unit-tested; the HTTP calls are thin.

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::Value;

use crate::error::LlmError;
use crate::provider::Provider;
use crate::types::{ModelInfo, ProviderKind, StreamItem, ToolDef, Turn};
use crate::{anthropic, gemini, openai};

/// Everything needed to reach one provider: which backend, the key, the base URL
/// (defaulted per provider, overridable for OpenAI-compatible), and the model.
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    pub kind: ProviderKind,
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    /// Anthropic requires an explicit `max_tokens`; ignored by the others.
    pub max_tokens: u32,
}

impl ProviderConfig {
    /// The default base URL for a provider; `OpenAiCompatible` has none (the user
    /// must supply one), so this returns an empty string for it.
    pub fn default_base_url(kind: ProviderKind) -> &'static str {
        match kind {
            ProviderKind::Anthropic => "https://api.anthropic.com",
            ProviderKind::OpenAi => "https://api.openai.com/v1",
            ProviderKind::Gemini => "https://generativelanguage.googleapis.com",
            ProviderKind::OpenAiCompatible => "",
        }
    }
}

/// A `Provider` backed by real HTTP. Cheap to build; holds a shared reqwest
/// client so connections are pooled across turns.
pub struct HttpProvider {
    config: ProviderConfig,
    http: reqwest::Client,
    /// Counts Gemini `stream_turn` calls (tool rounds) on this provider.
    /// Gemini has no per-call ids, so its parser synthesizes them; the round
    /// namespaces those ids, which would otherwise repeat every round because
    /// each round gets a fresh parser.
    gemini_round: std::sync::atomic::AtomicU64,
}

impl HttpProvider {
    pub fn new(config: ProviderConfig) -> Self {
        Self { config, http: reqwest::Client::new(), gemini_round: std::sync::atomic::AtomicU64::new(0) }
    }

    fn base(&self) -> &str {
        self.config.base_url.trim_end_matches('/')
    }
}

/// Which pure adapter parses this provider's stream. Wraps the three concrete
/// parser state machines behind one `push`.
enum StreamParser {
    Anthropic(anthropic::Stream),
    OpenAi(openai::Stream),
    Gemini(gemini::Stream),
}

impl StreamParser {
    fn push(&mut self, data: &str) -> Vec<StreamItem> {
        match self {
            StreamParser::Anthropic(s) => s.push(data),
            StreamParser::OpenAi(s) => s.push(data),
            StreamParser::Gemini(s) => s.push(data),
        }
    }
}

#[async_trait]
impl Provider for HttpProvider {
    async fn stream_turn(
        &self,
        turns: &[Turn],
        tools: &[ToolDef],
        on_item: &mut (dyn FnMut(StreamItem) + Send),
    ) -> Result<(), LlmError> {
        // Same identity/scope the CLI agents get — operate the cluster only
        // through the srelens MCP tools, no local filesystem/shell.
        let system = srelens_agent::adapter::BASE_SYSTEM_PROMPT;
        let (url, body, mut parser) = match self.config.kind {
            ProviderKind::Anthropic => (
                format!("{}/v1/messages", self.base()),
                anthropic::build_request(&self.config.model, self.config.max_tokens, system, turns, tools),
                StreamParser::Anthropic(anthropic::Stream::new()),
            ),
            ProviderKind::OpenAi | ProviderKind::OpenAiCompatible => (
                format!("{}/chat/completions", self.base()),
                openai::build_request(&self.config.model, system, turns, tools),
                StreamParser::OpenAi(openai::Stream::new()),
            ),
            ProviderKind::Gemini => (
                format!("{}/v1beta/models/{}:streamGenerateContent?alt=sse", self.base(), self.config.model),
                gemini::build_request(system, turns, tools),
                StreamParser::Gemini(gemini::Stream::for_round(
                    self.gemini_round.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                )),
            ),
        };

        let resp = self
            .auth(self.http.post(&url))
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(LlmError::Api(error_message(resp).await));
        }

        let mut decoder = SseDecoder::new();
        let mut stream = resp.bytes_stream();
        // Every provider marks the end of a healthy stream (`message_stop`,
        // `finish_reason`/`[DONE]`, Gemini's `finishReason`) or reports an
        // error item. EOF without either means the connection died mid-reply —
        // that must be an error, or the loop would persist the fragment as a
        // normal completed turn and feed it to follow-ups as if it were whole.
        let mut saw_terminal = false;
        while let Some(chunk) = stream.next().await {
            let bytes = chunk.map_err(|e| LlmError::Http(e.to_string()))?;
            for data in decoder.push(&bytes) {
                for item in parser.push(&data) {
                    saw_terminal |= matches!(item, StreamItem::Done(_) | StreamItem::Error(_));
                    on_item(item);
                }
            }
        }
        if !saw_terminal {
            return Err(LlmError::Api(
                "the provider stream ended before signaling completion; the reply may be incomplete".into(),
            ));
        }
        Ok(())
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, LlmError> {
        let url = match self.config.kind {
            ProviderKind::Anthropic => format!("{}/v1/models", self.base()),
            ProviderKind::OpenAi | ProviderKind::OpenAiCompatible => format!("{}/models", self.base()),
            ProviderKind::Gemini => format!("{}/v1beta/models", self.base()),
        };
        let resp = self.auth(self.http.get(&url)).send().await.map_err(|e| LlmError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(LlmError::Api(error_message(resp).await));
        }
        let body: Value = resp.json().await.map_err(|e| LlmError::Decode(e.to_string()))?;
        Ok(parse_models(self.config.kind, &body))
    }
}

impl HttpProvider {
    /// Attach the provider's auth header to a request builder.
    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self.config.kind {
            ProviderKind::Anthropic => {
                req.header("x-api-key", &self.config.api_key).header("anthropic-version", "2023-06-01")
            }
            ProviderKind::OpenAi | ProviderKind::OpenAiCompatible => req.bearer_auth(&self.config.api_key),
            ProviderKind::Gemini => req.header("x-goog-api-key", &self.config.api_key),
        }
    }
}

/// Extract a human error message from a non-2xx response body, falling back to
/// the raw text. Providers wrap it under `error.message` (OpenAI/Gemini/Anthropic).
async fn error_message(resp: reqwest::Response) -> String {
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.get("message")).and_then(Value::as_str).map(str::to_string))
        .unwrap_or_else(|| if text.is_empty() { format!("HTTP {status}") } else { format!("HTTP {status}: {text}") })
}

/// Parse a provider's models-list response into `ModelInfo`s.
fn parse_models(kind: ProviderKind, body: &Value) -> Vec<ModelInfo> {
    let entries = match kind {
        // OpenAI/Anthropic/compatible: `{ "data": [{ "id": ... }] }`.
        ProviderKind::Anthropic | ProviderKind::OpenAi | ProviderKind::OpenAiCompatible => body.get("data"),
        // Gemini: `{ "models": [{ "name": "models/gemini-...", "displayName": ... }] }`.
        ProviderKind::Gemini => body.get("models"),
    };
    let Some(arr) = entries.and_then(Value::as_array) else { return Vec::new() };
    arr.iter()
        .filter_map(|m| match kind {
            ProviderKind::Gemini => {
                let name = m.get("name").and_then(Value::as_str)?;
                // Gemini ids are `models/<id>`; strip the prefix for display use.
                let id = name.strip_prefix("models/").unwrap_or(name).to_string();
                let display = m.get("displayName").and_then(Value::as_str).unwrap_or(&id).to_string();
                Some(ModelInfo { id, display_name: display })
            }
            _ => {
                let id = m.get("id").and_then(Value::as_str)?.to_string();
                let display = m.get("display_name").and_then(Value::as_str).unwrap_or(&id).to_string();
                Some(ModelInfo { id, display_name: display })
            }
        })
        .collect()
}

/// Splits a streamed HTTP body into SSE `data:` payloads. Buffers partial lines
/// across chunks (a chunk boundary can fall mid-line), emits the text after
/// `data:` for each complete line, and ignores `event:`/comment/blank lines.
#[derive(Default)]
pub struct SseDecoder {
    /// Raw bytes, NOT a String: a chunk boundary can fall in the middle of a
    /// multibyte UTF-8 character, and decoding each chunk independently would
    /// mangle both halves into U+FFFD before they ever reached the buffer —
    /// corrupting non-ASCII text and potentially the JSON event around it.
    /// Only complete lines (which always contain whole characters) are decoded.
    buf: Vec<u8>,
}

impl SseDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one received byte chunk; returns any newly-complete `data:` payloads.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        // Only consume up to the last newline; keep any trailing partial line.
        while let Some(nl) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=nl).collect();
            let line = String::from_utf8_lossy(&line);
            let line = line.trim_end_matches(['\r', '\n']);
            if let Some(data) = line.strip_prefix("data:") {
                out.push(data.trim_start().to_string());
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_base_urls_are_set_for_the_hosted_providers() {
        assert!(ProviderConfig::default_base_url(ProviderKind::Anthropic).starts_with("https://api.anthropic"));
        assert!(ProviderConfig::default_base_url(ProviderKind::OpenAi).ends_with("/v1"));
        assert!(ProviderConfig::default_base_url(ProviderKind::Gemini).contains("generativelanguage"));
        assert_eq!(ProviderConfig::default_base_url(ProviderKind::OpenAiCompatible), "");
    }

    #[test]
    fn the_decoder_extracts_data_lines_and_ignores_the_rest() {
        let mut d = SseDecoder::new();
        let out = d.push(b"event: message\ndata: {\"a\":1}\n\ndata: [DONE]\n");
        assert_eq!(out, vec!["{\"a\":1}".to_string(), "[DONE]".to_string()]);
    }

    #[test]
    fn the_decoder_buffers_a_line_split_across_chunks() {
        let mut d = SseDecoder::new();
        assert!(d.push(b"data: {\"par").is_empty());
        assert!(d.push(b"tial\":true}").is_empty()); // still no newline
        assert_eq!(d.push(b"\n"), vec!["{\"partial\":true}".to_string()]);
    }

    #[test]
    fn a_multibyte_character_split_across_chunks_survives_intact() {
        let mut d = SseDecoder::new();
        let payload = "data: {\"text\":\"héllo\"}\n".as_bytes();
        // Split INSIDE the two-byte 'é' (0xC3 0xA9) — the boundary a real
        // HTTP chunking can produce.
        let split = payload.iter().position(|&b| b == 0xC3).unwrap() + 1;
        let (a, b) = payload.split_at(split);
        assert!(d.push(a).is_empty());
        assert_eq!(d.push(b), vec!["{\"text\":\"héllo\"}".to_string()]);
    }

    #[test]
    fn openai_style_models_parse_from_the_data_array() {
        let body = serde_json::json!({ "data": [{ "id": "gpt-5" }, { "id": "gpt-4o" }] });
        let models = parse_models(ProviderKind::OpenAi, &body);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "gpt-5");
        assert_eq!(models[0].display_name, "gpt-5");
    }

    #[test]
    fn gemini_models_strip_the_models_prefix_and_use_the_display_name() {
        let body = serde_json::json!({
            "models": [{ "name": "models/gemini-2.5-pro", "displayName": "Gemini 2.5 Pro" }]
        });
        let models = parse_models(ProviderKind::Gemini, &body);
        assert_eq!(models[0].id, "gemini-2.5-pro");
        assert_eq!(models[0].display_name, "Gemini 2.5 Pro");
    }

    #[test]
    fn a_models_body_of_the_wrong_shape_yields_an_empty_list() {
        assert!(parse_models(ProviderKind::OpenAi, &serde_json::json!({})).is_empty());
        assert!(parse_models(ProviderKind::Gemini, &serde_json::json!({ "models": "nope" })).is_empty());
    }
}
