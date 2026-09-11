use std::path::PathBuf;
use serde::{Deserialize, Serialize};

pub const DEFAULT_COMMAND_POPUP_MAX_WIDTH: u16 = 65;
pub const MIN_COMMAND_POPUP_MAX_WIDTH: u16 = 40;
pub const MAX_COMMAND_POPUP_MAX_WIDTH: u16 = 200;

pub const DEFAULT_COMMAND_POPUP_MAX_VISIBLE: usize = 6;
pub const MIN_COMMAND_POPUP_MAX_VISIBLE: usize = 3;
pub const MAX_COMMAND_POPUP_MAX_VISIBLE: usize = 20;

pub const DEFAULT_COMMAND_POPUP_TEXT_SCALE: u8 = 1;
pub const MIN_COMMAND_POPUP_TEXT_SCALE: u8 = 1;
pub const MAX_COMMAND_POPUP_TEXT_SCALE: u8 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CommandPopupDensity {
    #[default]
    Compact,
    Standard,
    Large,
    ExtraLarge,
}

impl Serialize for CommandPopupDensity {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Compact => serializer.serialize_str("compact"),
            Self::Standard => serializer.serialize_str("standard"),
            Self::Large => serializer.serialize_str("large"),
            Self::ExtraLarge => serializer.serialize_str("extralarge"),
        }
    }
}

impl<'de> Deserialize<'de> for CommandPopupDensity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = CommandPopupDensity;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str(
                    "a density string ('compact', 'standard', 'large', 'extralarge') or scale number (1..=4)",
                )
            }

            fn visit_u64<E>(self, v: u64) -> Result<CommandPopupDensity, E>
            where
                E: serde::de::Error,
            {
                Ok(CommandPopupDensity::from_scale(v.clamp(1, 4) as u8))
            }

            fn visit_i64<E>(self, v: i64) -> Result<CommandPopupDensity, E>
            where
                E: serde::de::Error,
            {
                Ok(CommandPopupDensity::from_scale(v.clamp(1, 4) as u8))
            }

            fn visit_str<E>(self, v: &str) -> Result<CommandPopupDensity, E>
            where
                E: serde::de::Error,
            {
                match v.to_ascii_lowercase().as_str() {
                    "compact" | "small" | "1" => Ok(CommandPopupDensity::Compact),
                    "standard" | "normal" | "medium" | "2" => Ok(CommandPopupDensity::Standard),
                    "large" | "3" => Ok(CommandPopupDensity::Large),
                    "extralarge" | "extra_large" | "extra-large" | "huge" | "4" => {
                        Ok(CommandPopupDensity::ExtraLarge)
                    }
                    _ => Ok(CommandPopupDensity::default()),
                }
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

impl CommandPopupDensity {
    pub fn scale(&self) -> u8 {
        match self {
            Self::Compact => 1,
            Self::Standard => 2,
            Self::Large => 3,
            Self::ExtraLarge => 4,
        }
    }

    pub fn from_scale(scale: u8) -> Self {
        match scale {
            1 => Self::Compact,
            2 => Self::Standard,
            3 => Self::Large,
            _ => Self::ExtraLarge,
        }
    }

    pub fn is_large(&self) -> bool {
        matches!(self, Self::Large | Self::ExtraLarge)
    }

    pub fn item_height(&self) -> u16 {
        match self {
            Self::Compact | Self::Standard => 1,
            Self::Large => 2,
            Self::ExtraLarge => 3,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Compact => "Compact (1-line dense)",
            Self::Standard => "Standard (1-line enriched)",
            Self::Large => "Large (2-line cards)",
            Self::ExtraLarge => "Extra Large (3-line cards)",
        }
    }

    pub fn toggle(&self) -> Self {
        match self {
            Self::Compact => Self::Standard,
            Self::Standard => Self::Large,
            Self::Large => Self::ExtraLarge,
            Self::ExtraLarge => Self::Compact,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TuiConfig {
    pub command_popup_max_width: u16,
    pub command_popup_max_visible: usize,
    #[serde(alias = "commandPopupTextScale")]
    pub command_popup_density: CommandPopupDensity,
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self {
            command_popup_max_width: DEFAULT_COMMAND_POPUP_MAX_WIDTH,
            command_popup_max_visible: DEFAULT_COMMAND_POPUP_MAX_VISIBLE,
            command_popup_density: CommandPopupDensity::default(),
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

    pub fn text_scale(&self) -> u8 {
        self.command_popup_density.scale()
    }

    pub fn set_text_scale(&mut self, scale: u8) {
        self.command_popup_density = CommandPopupDensity::from_scale(
            scale.clamp(MIN_COMMAND_POPUP_TEXT_SCALE, MAX_COMMAND_POPUP_TEXT_SCALE),
        );
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
