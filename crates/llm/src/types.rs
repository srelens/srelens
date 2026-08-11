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
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    /// Correlates the call with its result across the wire and in `AgentEvent`s.
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// The result of running a tool, fed back to the model on the next request.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutcome {
    pub id: String,
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
pub struct ModelInfo {
    pub id: String,
    /// A human label when the provider gives one; otherwise the adapter falls
    /// back to `id`.
    pub display_name: String,
}
