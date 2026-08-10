//! Agent-CLI bridge: normalize each supported agent CLI's streaming output
//! into a common `AgentEvent` stream. Pure — no Tauri, no cluster I/O.

pub mod adapter;
pub mod claude;
pub mod codex;
pub mod cursor;
pub mod event;
