#![allow(dead_code, unused_imports)]

pub mod agent;
pub mod ai_config;
pub mod ai_skills;
pub mod app;
pub mod cli;
pub mod commands;
pub mod consent;
pub mod deep_link;
pub mod event;
pub mod mcp_server;
pub mod self_update;
pub mod sink;
pub mod theme;
pub mod tui_config;
pub mod ui;
pub mod update_signature;
pub mod quick_rca;
pub mod rebrand;
pub mod views;

pub use ai_config::{AiProvider, AiSettings};
pub use ai_skills::{Skill, SkillDef, SkillMeta};
pub use app::App;
pub use cli::{Cli, CliCommand, SkillSubcommand};
pub use deep_link::DeepLink;
pub use tui_config::{CommandPopupDensity, TuiConfig};
