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

pub const DEFAULT_SHOW_FEATURE_BANNER: bool = true;
pub const DEFAULT_CHECK_UPDATES: bool = true;

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

/// The ArgoCD fetch-timeout choices `:config` steps through with h/l.
/// `None` follows the request timeout.
pub const ARGO_TIMEOUT_STEPS: &[Option<u64>] = &[
    None,
    Some(5),
    Some(10),
    Some(15),
    Some(20),
    Some(30),
    Some(45),
    Some(60),
    Some(90),
    Some(120),
];

/// Validate and tidy an ArgoCD UI base URL as typed into `:config`. Empty
/// clears it; anything else must be an http(s) URL, and trailing slashes are
/// dropped so the app link never reads `//applications`.
pub fn normalize_argo_ui_url(input: &str) -> Result<Option<String>, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let lower = trimmed.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"));
    match rest {
        Some(host) if !host.trim_end_matches('/').is_empty() => {
            Ok(Some(trimmed.trim_end_matches('/').to_string()))
        }
        Some(_) => Err("ArgoCD UI URL needs a host after the scheme".to_string()),
        None => Err(format!(
            "ArgoCD UI URL must start with https:// or http:// (got '{trimmed}')"
        )),
    }
}

/// The container-level `default` is load-bearing, not decoration: every field
/// here is required by serde unless something supplies one, and a `tui.json`
/// written by an earlier build has none of the fields added since. Without it,
/// adding a single field would make every older file fail to deserialize, and
/// `load()` — which falls back to `Self::default()` on any error — would answer
/// with defaults, silently discarding popup width, visible rows, density, the
/// banner flag and the Argo hub settings on upgrade. `field_added_later_does_not_discard_the_rest`
/// below is the regression test; do not drop this attribute.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TuiConfig {
    pub command_popup_max_width: u16,
    pub command_popup_max_visible: usize,
    #[serde(alias = "commandPopupTextScale")]
    pub command_popup_density: CommandPopupDensity,
    pub show_feature_banner: bool,
    pub check_updates: bool,
    pub argo_hub_context: Option<String>,
    pub argo_hub_kubeconfig: Option<PathBuf>,
    /// Base URL of the ArgoCD web UI, e.g. `https://argocd.example.com`.
    /// When set, `a` in an Argo app's detail view opens the app there.
    pub argo_ui_url: Option<String>,
    /// Timeout for ArgoCD reads (`:argo`), in seconds. `None` follows the
    /// process-wide request timeout.
    pub argo_timeout_secs: Option<u64>,
    #[serde(skip)]
    pub update_available: Option<String>,
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self {
            command_popup_max_width: DEFAULT_COMMAND_POPUP_MAX_WIDTH,
            command_popup_max_visible: DEFAULT_COMMAND_POPUP_MAX_VISIBLE,
            command_popup_density: CommandPopupDensity::default(),
            show_feature_banner: DEFAULT_SHOW_FEATURE_BANNER,
            check_updates: DEFAULT_CHECK_UPDATES,
            argo_hub_context: None,
            argo_hub_kubeconfig: None,
            argo_ui_url: None,
            argo_timeout_secs: None,
            update_available: None,
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

    /// The ArgoCD web UI link for Application `app`, or `None` when no UI
    /// URL is configured.
    pub fn argo_app_url(&self, app: &str) -> Option<String> {
        let base = self.argo_ui_url.as_deref()?.trim().trim_end_matches('/');
        if base.is_empty() {
            return None;
        }
        Some(format!("{base}/applications/{app}"))
    }

    /// Push `argo_timeout_secs` to the kube crate, which every ArgoCD read
    /// consults. Called at startup and after each `:config` change, so the
    /// next `:argo` fetch uses it without a restart.
    pub fn apply_argo_timeout(&self) -> Option<u64> {
        srelens_kube::argo::set_argo_timeout_secs(self.argo_timeout_secs)
    }

    /// The ArgoCD timeout as a reader should see it: "30s", or "inherit (8s)"
    /// when it follows the request timeout.
    pub fn argo_timeout_label(&self) -> String {
        match self.argo_timeout_secs {
            Some(secs) => format!("{secs}s"),
            None => format!(
                "inherit ({}s request timeout)",
                srelens_kube::connect::request_timeout_secs()
            ),
        }
    }

    /// Step the ArgoCD timeout through [`ARGO_TIMEOUT_STEPS`] by `delta`,
    /// stopping at either end. A value not in the list (hand-edited file)
    /// steps from the nearest choice below it.
    pub fn step_argo_timeout(&mut self, delta: i32) {
        let pos = ARGO_TIMEOUT_STEPS
            .iter()
            .rposition(|step| match (step, self.argo_timeout_secs) {
                (None, _) => true,
                (Some(s), Some(cur)) => *s <= cur,
                (Some(_), None) => false,
            })
            .unwrap_or(0) as i32;
        let next = (pos + delta).clamp(0, ARGO_TIMEOUT_STEPS.len() as i32 - 1);
        self.argo_timeout_secs = ARGO_TIMEOUT_STEPS[next as usize];
    }

    pub fn resolved_argo_hub_context(&self) -> Option<String> {
        if let Ok(val) = std::env::var("SRELENS_ARGO_HUB_CONTEXT") {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
        if let Some(ref ctx) = self.argo_hub_context {
            let trimmed = ctx.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
        if let Some(kc_path) = self.resolved_argo_hub_kubeconfig() {
            if let Ok(kc) = srelens_kube::kube::config::Kubeconfig::read_from(&kc_path) {
                if let Some(curr) = kc.current_context {
                    let trimmed = curr.trim();
                    if !trimmed.is_empty() {
                        return Some(trimmed.to_string());
                    }
                }
            }
        }
        None
    }

    pub fn resolved_argo_hub_kubeconfig(&self) -> Option<PathBuf> {
        let raw = if let Ok(val) = std::env::var("SRELENS_ARGO_HUB_KUBECONFIG") {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                Some(PathBuf::from(trimmed))
            } else {
                None
            }
        } else {
            self.argo_hub_kubeconfig.clone()
        };
        raw.map(|p| {
            if let Ok(stripped) = p.strip_prefix("~") {
                dirs::home_dir().map(|h| h.join(stripped)).unwrap_or(p)
            } else {
                p
            }
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolved_argo_hub_context_fallback_to_kubeconfig() {
        let temp_dir = tempfile::tempdir().unwrap();
        let kc_path = temp_dir.path().join("hub.kubeconfig");
        let kc_yaml = "apiVersion: v1\nclusters: []\ncontexts:\n- context:\n    cluster: my-hub\n    user: me\n  name: hub-cluster-ctx\ncurrent-context: hub-cluster-ctx\nusers: []\n";
        std::fs::write(&kc_path, kc_yaml).unwrap();

        let mut cfg = TuiConfig::default();
        cfg.argo_hub_context = None;
        cfg.argo_hub_kubeconfig = Some(kc_path);

        assert_eq!(cfg.resolved_argo_hub_context(), Some("hub-cluster-ctx".to_string()));

        // Explicit context overrides kubeconfig current-context
        cfg.argo_hub_context = Some("explicit-ctx".to_string());
        assert_eq!(cfg.resolved_argo_hub_context(), Some("explicit-ctx".to_string()));
    }

    /// A `tui.json` written before a field existed still loads, and keeps every
    /// setting it DOES carry.
    ///
    /// This is the whole reason `TuiConfig` is `#[serde(default)]` at the
    /// container. `load()` treats any deserialize error as "no config" and
    /// answers `Self::default()`, so one field without a default would not
    /// reset that field — it would reset the file. The payload below is what
    /// the build before `checkUpdates` and the Argo hub settings wrote.
    #[test]
    fn field_added_later_does_not_discard_the_rest() {
        let older = r#"{
            "commandPopupMaxWidth": 120,
            "commandPopupMaxVisible": 14,
            "commandPopupDensity": "large",
            "showFeatureBanner": false
        }"#;

        let config: TuiConfig =
            serde_json::from_str(older).expect("a config written without the newer fields loads");

        // The settings the file does carry survive.
        assert_eq!(config.command_popup_max_width, 120);
        assert_eq!(config.command_popup_max_visible, 14);
        assert_eq!(config.command_popup_density, CommandPopupDensity::Large);
        assert!(!config.show_feature_banner);
        // And the ones it does not come back as the documented defaults.
        assert_eq!(config.check_updates, DEFAULT_CHECK_UPDATES);
        assert_eq!(config.argo_hub_context, None);
        assert_eq!(config.argo_hub_kubeconfig, None);
        assert_eq!(config.argo_ui_url, None);
        assert_eq!(config.argo_timeout_secs, None);
    }

    #[test]
    fn argo_settings_round_trip_under_their_camel_case_names() {
        let config = TuiConfig {
            argo_ui_url: Some("https://argocd.example.com".to_string()),
            argo_timeout_secs: Some(30),
            ..TuiConfig::default()
        };
        let raw = serde_json::to_value(&config).unwrap();
        assert_eq!(raw["argoUiUrl"], "https://argocd.example.com");
        assert_eq!(raw["argoTimeoutSecs"], 30);
        let back: TuiConfig = serde_json::from_value(raw).unwrap();
        assert_eq!(back, config);
    }

    #[test]
    fn argo_app_url_joins_the_base_and_the_application_name() {
        let mut config = TuiConfig::default();
        assert_eq!(config.argo_app_url("payments"), None, "off until configured");

        config.argo_ui_url = Some("   ".to_string());
        assert_eq!(config.argo_app_url("payments"), None, "blank is off");

        config.argo_ui_url = Some("https://argocd.example.com/".to_string());
        assert_eq!(
            config.argo_app_url("payments").as_deref(),
            Some("https://argocd.example.com/applications/payments"),
            "no double slash from a trailing one"
        );
    }

    #[test]
    fn normalize_argo_ui_url_accepts_http_urls_and_rejects_the_rest() {
        assert_eq!(normalize_argo_ui_url(""), Ok(None));
        assert_eq!(normalize_argo_ui_url("  "), Ok(None));
        assert_eq!(
            normalize_argo_ui_url(" https://argocd.example.com/// "),
            Ok(Some("https://argocd.example.com".to_string()))
        );
        assert_eq!(
            normalize_argo_ui_url("http://localhost:8080/argo"),
            Ok(Some("http://localhost:8080/argo".to_string()))
        );
        assert_eq!(
            normalize_argo_ui_url("HTTPS://Argo.Example.com"),
            Ok(Some("HTTPS://Argo.Example.com".to_string()))
        );
        assert!(normalize_argo_ui_url("argo.local").unwrap_err().contains("https://"));
        assert!(normalize_argo_ui_url("https://").is_err());
        assert!(normalize_argo_ui_url("ftp://argo").is_err());
    }

    #[test]
    fn step_argo_timeout_walks_the_choices_and_stops_at_the_ends() {
        let mut config = TuiConfig::default();
        config.step_argo_timeout(-1);
        assert_eq!(config.argo_timeout_secs, None, "no step below inherit");
        config.step_argo_timeout(1);
        assert_eq!(config.argo_timeout_secs, Some(5));
        config.step_argo_timeout(1);
        assert_eq!(config.argo_timeout_secs, Some(10));
        config.step_argo_timeout(-2);
        assert_eq!(config.argo_timeout_secs, None);

        config.argo_timeout_secs = Some(120);
        config.step_argo_timeout(1);
        assert_eq!(config.argo_timeout_secs, Some(120), "no step above the maximum");

        // A hand-edited value between choices steps from the one below it.
        config.argo_timeout_secs = Some(25);
        config.step_argo_timeout(1);
        assert_eq!(config.argo_timeout_secs, Some(30));
    }

    /// The same for a file from before ANY of these fields existed: an empty
    /// object is a valid config, not a parse failure.
    #[test]
    fn an_empty_config_object_is_the_defaults_rather_than_an_error() {
        let config: TuiConfig = serde_json::from_str("{}").expect("an empty object loads");
        assert_eq!(config, TuiConfig::default());
    }
}
