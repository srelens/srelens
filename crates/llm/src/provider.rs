//! The two async boundaries the agentic loop is written against: a `Provider`
//! (one LLM backend) and a `ToolInvoker` (the srelens MCP tool surface). Keeping
//! the loop generic over these traits is what lets it be driven by stubs in
//! tests, and by real network clients in the desktop, with the same code.

use async_trait::async_trait;
use serde_json::Value;

use crate::error::LlmError;
use crate::types::{ModelInfo, StreamItem, ToolDef, Turn};

/// One LLM backend. `stream_turn` streams a single assistant turn, invoking
/// `on_item` for each `StreamItem` as it arrives (text, thinking, tool calls,
/// the terminal `Done`, or an `Error`). `list_models` backs the Settings picker.
#[async_trait]
pub trait Provider: Send + Sync {
    async fn stream_turn(
        &self,
        turns: &[Turn],
        tools: &[ToolDef],
        on_item: &mut (dyn FnMut(StreamItem) + Send),
    ) -> Result<(), LlmError>;

    async fn list_models(&self) -> Result<Vec<ModelInfo>, LlmError>;
}

/// The srelens MCP tool surface, as the loop sees it. Real implementations call
/// the local MCP server over loopback with the session bearer token, so consent
/// gating and audit are identical to the CLI agents.
#[async_trait]
pub trait ToolInvoker: Send + Sync {
    /// The tools the model may call, with their `read_only` hint.
    async fn list_tools(&self) -> Result<Vec<ToolDef>, LlmError>;

    /// Run one tool call. Returns the textual result and whether it failed
    /// (a denied destructive call included — the loop feeds that back to the
    /// model so it can react rather than aborting the turn).
    async fn call_tool(&self, name: &str, args: &Value) -> Result<ToolCallResult, LlmError>;
}

/// The outcome of a single tool invocation.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCallResult {
    pub content: String,
    pub is_error: bool,
    /// True when the call was refused at the consent boundary (destructive tool,
    /// user declined). Surfaced to the drawer as `ToolStatus::Denied`.
    pub denied: bool,
}
