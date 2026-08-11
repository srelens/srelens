//! srelens native LLM agent.
//!
//! Talks directly to provider APIs (Anthropic, OpenAI, Gemini, and any
//! OpenAI-compatible endpoint) with the user's own key, and drives srelens's
//! MCP tools through the same loopback-HTTP + bearer-token boundary the CLI
//! agents use — so the sandbox, the destructive-tool confirm dialog, and the
//! audit log are all identical.
//!
//! Request construction and SSE parsing are pure (per-provider modules); only
//! the thin client wrappers touch the network, and the agentic loop is driven
//! against those pure pieces so its contract is unit-tested without a live API.

pub mod agent_loop;
pub mod anthropic;
pub mod client;
pub mod error;
pub mod gemini;
pub mod openai;
pub mod provider;
pub mod types;

pub use client::{HttpProvider, ProviderConfig};
pub use error::LlmError;
pub use provider::{Provider, ToolCallResult, ToolInvoker};

pub use types::{
    ModelInfo, ProviderKind, StopReason, StreamItem, ToolCall, ToolDef, ToolOutcome, Turn,
};
