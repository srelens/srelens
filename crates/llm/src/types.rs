//! Provider-agnostic conversation and tool types shared by every adapter.
//!
//! These model the conversation once; each provider adapter maps `[Turn]` and
//! `[ToolDef]` into its own request wire format, and maps its own streaming wire
//! format back into `[StreamItem]`. Keeping this in one place is what lets the
//! agentic loop (`loop.rs`) stay provider-independent.

use serde::{Deserialize, Serialize};

/// Which provider a native turn targets. `OpenAiCompatible` reuses the OpenAI
/// wire format against a user-supplied base URL (OpenRouter, Azure, Ollama, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderKind {
    Anthropic,
    OpenAi,
    Gemini,
    OpenAiCompatible,
}

/// A tool the model may call: its srelens MCP name, description, and JSON Schema
/// for arguments. `read_only` drives the consent policy (read-only auto-runs;
/// destructive calls route through srelens's confirm dialog), mirroring the
/// `readOnlyHint` the MCP `tools/list` advertises.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub read_only: bool,
}

/// A tool call the model requested, with fully-assembled arguments (adapters
/// accumulate any streamed partial-JSON argument deltas before emitting this).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ToolCall {
    /// Correlates the call with its result across the wire and in `AgentEvent`s.
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
    /// Gemini thinking models attach an opaque `thoughtSignature` to a
    /// `functionCall` part; it must be replayed verbatim with that call in
    /// the next request or the follow-up is rejected. Gemini-only — the
    /// other adapters leave it `None` and ignore it when serializing.
    pub thought_signature: Option<String>,
}

/// The result of running a tool, fed back to the model on the next request.
/// Carries both the call `id` (Anthropic/OpenAI correlate by id) and the tool
/// `name` (Gemini's `functionResponse` correlates by name) so every adapter has
/// what it needs.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutcome {
    pub id: String,
    pub name: String,
    pub content: String,
    pub is_error: bool,
}

/// One turn of the conversation, in the shape the loop accumulates and each
/// adapter serializes. A provider request is built from `system` plus a slice
/// of these.
#[derive(Debug, Clone, PartialEq)]
pub enum Turn {
    /// A user message (the visible prompt, already prefaced/guided upstream).
    User(String),
    /// An assistant turn: any text it produced plus the tool calls it requested.
    Assistant { text: String, tool_calls: Vec<ToolCall> },
    /// The results of the tool calls from the immediately preceding assistant
    /// turn, in the same order.
    ToolResults(Vec<ToolOutcome>),
}

/// Why the provider ended a streamed turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// The model finished its reply and wants no tool calls — the turn is over.
    EndTurn,
    /// The model requested one or more tool calls; the loop must run them and
    /// send another request with their results.
    ToolUse,
    /// The provider cut the reply off at its output-token limit — whatever
    /// streamed is incomplete, and the loop must say so rather than present
    /// the fragment as a finished answer.
    MaxTokens,
}

/// One item a provider's streaming parser yields as SSE chunks arrive. The loop
/// turns these into `AgentEvent`s and accumulates the assistant `Turn`.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamItem {
    /// A chunk of streamed reply text.
    Text(String),
    /// A chunk of streamed reasoning/thinking.
    Thinking(String),
    /// A fully-assembled tool call the model requested this turn.
    ToolCall(ToolCall),
    /// The turn ended; carries why.
    Done(StopReason),
    /// A provider-reported error (auth/quota/rate-limit/overloaded/…).
    Error(String),
}

/// A model offered by a provider, as returned by its models endpoint and shown
/// in the Settings picker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    /// A human label when the provider gives one; otherwise the adapter falls
    /// back to `id`. Serialized as `displayName` — this crosses the tauri IPC
    /// boundary into the Settings UI's `ModelInfo` type.
    pub display_name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Settings UI reads `displayName` over IPC — a snake_case
    /// serialization renders every model option with a blank label.
    #[test]
    fn model_info_crosses_the_ipc_boundary_in_camel_case() {
        let m = ModelInfo { id: "deepseek-chat".into(), display_name: "DeepSeek Chat".into() };
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["id"], "deepseek-chat");
        assert_eq!(v["displayName"], "DeepSeek Chat");
    }
}
