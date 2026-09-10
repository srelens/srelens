use std::path::PathBuf;
use serde::{Deserialize, Serialize};

pub const DEFAULT_COMMAND_POPUP_MAX_WIDTH: u16 = 65;
pub const MIN_COMMAND_POPUP_MAX_WIDTH: u16 = 40;
pub const MAX_COMMAND_POPUP_MAX_WIDTH: u16 = 200;

pub const DEFAULT_COMMAND_POPUP_MAX_VISIBLE: usize = 6;
pub const MIN_COMMAND_POPUP_MAX_VISIBLE: usize = 3;
pub const MAX_COMMAND_POPUP_MAX_VISIBLE: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TuiConfig {
    pub command_popup_max_width: u16,
    pub command_popup_max_visible: usize,
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self {
            command_popup_max_width: DEFAULT_COMMAND_POPUP_MAX_WIDTH,
            command_popup_max_visible: DEFAULT_COMMAND_POPUP_MAX_VISIBLE,
        }
    }
}

impl TuiConfig {
    pub fn clamp(&mut self) {
        self.command_popup_max_width = self
            .command_popup_max_width
            .clamp(MIN_COMMAND_POPUP_MAX_WIDTH, MAX_COMMAND_POPUP_MAX_WIDTH);
        self.command_popup_max_visible = self
            .command_popup_max_visible
            .clamp(MIN_COMMAND_POPUP_MAX_VISIBLE, MAX_COMMAND_POPUP_MAX_VISIBLE);
    }

    pub fn config_file_path() -> PathBuf {
        if let Ok(custom) = std::env::var("SRELENS_TUI_CONFIG_PATH") {
            if !custom.trim().is_empty() {
                return PathBuf::from(custom);
            }
        }
        if let Ok(custom_dir) = std::env::var("SRELENS_CONFIG_DIR") {
            if !custom_dir.trim().is_empty() {
                return PathBuf::from(custom_dir).join("tui.json");
            }
        }
        dirs::config_dir()
            .map(|p| p.join("srelens").join("tui.json"))
            .unwrap_or_else(|| PathBuf::from(".srelens-tui.json"))
    }

    pub fn load() -> Self {
        let path = Self::config_file_path();
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(mut config) = serde_json::from_str::<TuiConfig>(&content) {
                config.clamp();
                return config;
            }
        }
        Self::default()
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::config_file_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let mut to_save = self.clone();
        to_save.clamp();
        let json = serde_json::to_string_pretty(&to_save)
            .map_err(|e| format!("Failed to serialize TuiConfig: {}", e))?;
        std::fs::write(&path, json)
            .map_err(|e| format!("Failed to write {}: {}", path.display(), e))
    }
}
