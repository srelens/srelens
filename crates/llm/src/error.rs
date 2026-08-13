//! Error type shared by the provider clients and the agentic loop.

use thiserror::Error;

/// Anything that can go wrong talking to a provider or a tool. The loop turns
/// these into an `AgentEvent::Error` (message text) before ending the turn, so
/// the drawer shows the failure instead of a blank reply.
#[derive(Debug, Error)]
pub enum LlmError {
    /// Transport failure reaching the provider or the MCP server.
    #[error("network error: {0}")]
    Http(String),
    /// The provider or tool returned an error payload (auth, quota, bad model…).
    #[error("{0}")]
    Api(String),
    /// A response we couldn't parse into the expected shape.
    #[error("could not parse provider response: {0}")]
    Decode(String),
    /// No API key configured for the selected provider.
    #[error("no API key configured for this provider")]
    MissingKey,
}
