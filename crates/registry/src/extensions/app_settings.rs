//! An installed app's typed settings (#542) as the inventory keeps them.
//!
//! Three rules, each at the one place a path has to pass:
//!
//! - a save is held to the manifest's declarations and to every binding that
//!   interpolates a value (`checked_settings`, for `extensions.configure`);
//! - a secret setting holds only its reference, never a value: refused when
//!   saved (`saved_form`) and dropped when loaded (`drop_secret_values`);
//! - an update or rollback keeps only what the new manifest still accepts
//!   (`Manifest::retain_settings`, called from `mutate`).
use super::Installed;
use serde_json::{Map, Value};
use srelens_capability::settings::SettingType;
use srelens_capability::Registry;
use srelens_plugin_host::{secret_reference, Manifest, PluginHost, ValidationErrors};
use std::sync::Arc;

/// The settings to store when a person saves `values`, or every reason not.
///
/// A cleared field is left out rather than stored empty, so the setting falls
/// back to its default. Secret references already stored are kept: a caller
/// never sends one, and the secret store (#543) is what writes them.
pub(super) fn checked_settings(
    manifest: &Manifest,
    stored: &Map<String, Value>,
    values: Map<String, Value>,
    core: Arc<Registry>,
) -> Result<Map<String, Value>, ValidationErrors> {
    manifest.check_setting_values(&values)?;
    let mut saved: Map<String, Value> = values
        .into_iter()
        .filter(|(_, value)| {
            !matches!(value, Value::Null) && value != "" && value != &Value::Array(vec![])
        })
        .collect();
    for setting in &manifest.settings {
        if setting.setting_type == SettingType::SecretReference {
            if let Some(reference) = stored.get(&setting.id) {
                saved.insert(setting.id.clone(), reference.clone());
            }
        }
    }
    // The values in place, as a request would send them: a value its
    // declaration allows but a capability refuses is refused now, not on the
    // next click.
    let problems = PluginHost::new(core).settings_problems(manifest, &saved);
    if problems.is_empty() {
        Ok(saved)
    } else {
        Err(ValidationErrors(problems))
    }
}

/// Drops whatever a secret setting holds other than its own reference: a
/// hand-edited inventory, or one written by a host with a bug. The next save
/// then removes it from disk too, and nothing reads it in between.
pub(super) fn drop_secret_values(plugin: &mut Installed) {
    for setting in &plugin.manifest.settings {
        if setting.setting_type == SettingType::SecretReference
            && plugin
                .settings
                .get(&setting.id)
                .is_some_and(|value| *value != secret_reference(&plugin.manifest.id, &setting.id))
        {
            plugin.settings.remove(&setting.id);
        }
    }
}

/// What the settings `arguments` interpolate can make a binding send, for the
/// access review: each one's declaration, so an update that lets a setting
/// write another value shows as changed access. Empty when none is used.
pub(super) fn setting_scope(manifest: &Manifest, arguments: &Map<String, Value>) -> String {
    let mut used: Vec<String> = arguments
        .values()
        .filter_map(|value| srelens_capability::settings::reference(value)?.ok())
        .filter_map(|id| manifest.setting(id))
        .map(|setting| {
            let mut declared = serde_json::to_value(setting).unwrap_or(Value::Null);
            // How the field is labelled is not what it can send.
            if let Some(fields) = declared.as_object_mut() {
                fields.remove("title");
                fields.remove("description");
                if let Some(Value::Array(options)) = fields.get_mut("options") {
                    for option in options.iter_mut().filter_map(Value::as_object_mut) {
                        option.remove("label");
                    }
                }
            }
            super::canonical(&declared)
        })
        .collect();
    if used.is_empty() {
        return String::new();
    }
    used.sort();
    format!(" from settings [{}]", used.join(","))
}
