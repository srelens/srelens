//! The audit trail, re-exported.
//!
//! It moved to `srelens-capability` (`crates/capability/src/audit.rs`) for
//! #555: an audit sink that lives in the MCP crate can only ever record what
//! came in over MCP, and a Flux reconcile clicked in the desktop UI goes
//! nowhere near this crate. The registry is where the UI bridge and the MCP
//! server meet, so the sink lives beside it and
//! `Registry::invoke_audited` writes every invocation record.
//!
//! This module stays as the name the MCP server and its host already reach
//! for — `srelens_mcp::audit::JsonlAuditLog`, `::tail` — so the transport code
//! is not littered with a second crate path for one file it writes to.

pub use srelens_capability::audit::*;
