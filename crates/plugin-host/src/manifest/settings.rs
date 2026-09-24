//! Typed settings (#542): what an app declares, what a person may save into
//! it, and where a setting may be interpolated.
//!
//! The host draws the form from these declarations and holds every saved
//! value to them, whatever the form allowed. A setting reaches a cluster
//! request only through a binding argument the capability behind it marks as
//! settable (`srelens_capability::settings`), checked by
//! [`crate::PluginHost::interpolate`] at install, on save and on every
//! request.
//!
//! A `secret-reference` setting's value never enters the inventory: the
//! inventory may hold only [`secret_reference`], which the host's secret
//! store (#543) writes once it has the value, and no caller can save one.
use super::{identifier, is_false, label, unique, Manifest};
use crate::{ValidationCode as Code, ValidationErrors};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Number, Value};
use srelens_capability::is_format_character;
use srelens_capability::settings::{self, SettingType};

/// Most settings one manifest may declare.
pub const MAX_SETTINGS: usize = 32;
/// Most options a `select` or `multi-select` setting may list.
pub const MAX_SETTING_OPTIONS: usize = 64;
/// How long a `string` setting's value may be when it names no `maxLength`.
pub const DEFAULT_SETTING_MAX_LENGTH: u32 = 1024;
/// The largest `maxLength` a `string` setting may name.
pub const MAX_SETTING_MAX_LENGTH: u32 = 4096;
/// The longest a setting's `description` may be, in characters.
pub const MAX_SETTING_DESCRIPTION: usize = 500;
/// The longest a `url` or `cluster-selector` value may be, in characters.
const MAX_REFERENCE_CHARS: usize = 2048;

/// One setting an app declares. The host renders it as a form field and
/// checks every value saved into it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Setting {
    pub id: String,
    #[serde(rename = "type")]
    pub setting_type: SettingType,
    /// Untrusted display text, drawn as plain text.
    pub title: String,
    /// Untrusted help text under the field, drawn as plain text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Whether a save must give it a value. Refused beside `default`, which
    /// would make it never missing.
    #[serde(default, skip_serializing_if = "is_false")]
    pub required: bool,
    /// The value used while none is saved. Held to the setting's own rules;
    /// refused on a `secret-reference`, since a manifest is public.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    /// What a `select` or `multi-select` offers; refused on other types.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<SettingOption>,
    /// The smallest value a `number` setting takes; refused on other types.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum: Option<Number>,
    /// The largest value a `number` setting takes; refused on other types.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum: Option<Number>,
    /// Whether a `number` setting takes whole numbers only; refused on other types.
    #[serde(default, skip_serializing_if = "is_false")]
    pub integer: bool,
    /// The most characters a `string` setting takes, 1–4096 (1024 when
    /// absent); refused on other types.
    #[serde(default, rename = "maxLength", skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u32>,
}

/// One choice of a `select` or `multi-select` setting.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SettingOption {
    /// What is saved, and interpolated.
    pub value: String,
    /// Untrusted display text, drawn as plain text.
    pub label: String,
}

/// The only thing the inventory may hold for `setting` of app `app` when the
/// setting is a `secret-reference`: where the host's secret store (#543)
/// keeps the value, never the value. Host-minted and fixed by the two ids, so
/// a caller cannot point one app's setting at another's secret.
pub fn secret_reference(app: &str, setting: &str) -> Value {
    json!({ "secretRef": format!("{app}/{setting}") })
}

/// Whether a value counts as not given: absent, an empty string or an empty
/// list, which is what a cleared form field sends.
fn blank(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => true,
        Some(Value::String(text)) => text.is_empty(),
        Some(Value::Array(items)) => items.is_empty(),
        Some(_) => false,
    }
}

/// Text a person typed, on one line: no control or format characters.
fn visible(text: &str) -> bool {
    !text
        .chars()
        .any(|c| c.is_control() || is_format_character(c))
}

/// A Kubernetes namespace name (an RFC 1123 label).
fn namespace_name(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 63
        && text
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !text.starts_with('-')
        && !text.ends_with('-')
}

impl Setting {
    /// Whether `value` is one this setting takes, or why not. The reason never
    /// repeats the value: it is written to errors a log, an audit record or an
    /// MCP client may keep, and a value may be something its owner did not
    /// mean to share.
    pub fn check_value(&self, value: &Value) -> Result<(), String> {
        let text = || value.as_str().ok_or("Must be text");
        match self.setting_type {
            SettingType::String => {
                let text = text()?;
                let most = self.max_length.unwrap_or(DEFAULT_SETTING_MAX_LENGTH) as usize;
                if text.chars().count() > most {
                    return Err(format!("Must be at most {most} characters"));
                }
                if !visible(text) {
                    return Err(
                        "Must be one line with no control or invisible format characters".into(),
                    );
                }
                Ok(())
            }
            SettingType::Number => {
                let number = value.as_f64().ok_or("Must be a number")?;
                if self.integer && !(value.is_i64() || value.is_u64()) {
                    return Err("Must be a whole number".into());
                }
                if let Some(minimum) = &self.minimum {
                    if minimum.as_f64().is_some_and(|least| number < least) {
                        return Err(format!("Must be at least {minimum}"));
                    }
                }
                if let Some(maximum) = &self.maximum {
                    if maximum.as_f64().is_some_and(|most| number > most) {
                        return Err(format!("Must be at most {maximum}"));
                    }
                }
                Ok(())
            }
            SettingType::Boolean => value
                .is_boolean()
                .then_some(())
                .ok_or_else(|| "Must be true or false".into()),
            SettingType::Select => {
                let text = text()?;
                self.options
                    .iter()
                    .any(|option| option.value == text)
                    .then_some(())
                    .ok_or_else(|| "Must be one of the listed options".into())
            }
            SettingType::MultiSelect => {
                let items = value.as_array().ok_or("Must be a list of options")?;
                let mut seen = std::collections::BTreeSet::new();
                for item in items {
                    let item = item.as_str().ok_or("Must be a list of options")?;
                    if !self.options.iter().any(|option| option.value == item) {
                        return Err("Must list only the listed options".into());
                    }
                    if !seen.insert(item) {
                        return Err("Must list each option at most once".into());
                    }
                }
                Ok(())
            }
            SettingType::Url => {
                let text = text()?;
                const URL: &str = "Must be an http or https URL with a host";
                if text.chars().count() > MAX_REFERENCE_CHARS || !visible(text) {
                    return Err(URL.into());
                }
                let url = url::Url::parse(text).map_err(|_| URL)?;
                if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
                    return Err(URL.into());
                }
                // Stored in plain text and sent to the audit log's shape, so
                // credentials go in a secret-reference setting instead.
                if !url.username().is_empty() || url.password().is_some() {
                    return Err("Must not hold a user name or password; use a secret-reference setting for credentials".into());
                }
                Ok(())
            }
            SettingType::NamespaceSelector => namespace_name(text()?)
                .then_some(())
                .ok_or_else(|| "Must be a Kubernetes namespace name: at most 63 lowercase letters, digits and -".into()),
            SettingType::ClusterSelector => {
                let text = text()?;
                (!text.is_empty() && text.chars().count() <= MAX_REFERENCE_CHARS && visible(text))
                    .then_some(())
                    .ok_or_else(|| "Must name a kubeconfig context".into())
            }
            SettingType::SecretReference => Err(format!(
                "\"{}\" is a secret. Its value is kept by the host's secret store, never saved as a setting",
                self.title
            )),
        }
    }

    /// The value in effect: the saved one, else the default.
    pub fn effective<'a>(&'a self, saved: Option<&'a Value>) -> Option<&'a Value> {
        if blank(saved) {
            self.default.as_ref()
        } else {
            saved
        }
    }
}

impl Manifest {
    /// The declared setting `id`.
    pub fn setting(&self, id: &str) -> Option<&Setting> {
        self.settings.iter().find(|setting| setting.id == id)
    }

    /// Every reason `values` cannot be saved as this app's settings, each at
    /// `settings.<id>`: a setting it does not declare, a value its declaration
    /// refuses, a required one left out, or any value at all for a secret.
    ///
    /// A secret-reference is never in `values`, not even as its reference:
    /// only the host's secret store writes that (#543). Its requiredness is
    /// the store's to report, so it does not stop other settings saving.
    pub fn check_setting_values(
        &self,
        values: &Map<String, Value>,
    ) -> Result<(), ValidationErrors> {
        let mut problems = ValidationErrors::default();
        for (id, value) in values {
            let path = format!("settings.{id}");
            match self.setting(id) {
                None => problems.push(
                    Code::UnknownField,
                    path,
                    "This app declares no such setting",
                ),
                Some(setting)
                    if setting.setting_type != SettingType::SecretReference
                        && blank(Some(value)) => {}
                Some(setting) => {
                    if let Err(why) = setting.check_value(value) {
                        problems.push(Code::InvalidValue, path, why);
                    }
                }
            }
        }
        for setting in &self.settings {
            if setting.required
                && setting.setting_type != SettingType::SecretReference
                && blank(values.get(&setting.id))
            {
                problems.push(
                    Code::InvalidValue,
                    format!("settings.{}", setting.id),
                    "Required",
                );
            }
        }
        problems.into_result()
    }

    /// The stored settings this manifest still accepts: the rest are dropped.
    ///
    /// Run when an update or a rollback replaces the manifest a value was
    /// saved against. A value is kept only while it is still declared and
    /// still passes its declaration; a secret keeps only its own reference.
    /// So a string setting that becomes a secret loses its plaintext rather
    /// than keeping it under a secret's name, and a secret that becomes a
    /// string does not turn its reference into a string value.
    pub fn retain_settings(&self, mut stored: Map<String, Value>) -> Map<String, Value> {
        stored.retain(|id, value| match self.setting(id) {
            None => false,
            Some(setting) if setting.setting_type == SettingType::SecretReference => {
                *value == secret_reference(&self.id, id)
            }
            Some(setting) => setting.check_value(value).is_ok(),
        });
        stored
    }

    /// Why `stored` cannot be written to the inventory: a `secret-reference`
    /// setting holding anything but its own reference. Never repeats the value.
    pub fn stored_secret_problems(&self, stored: &Map<String, Value>) -> Vec<String> {
        self.settings
            .iter()
            .filter(|setting| setting.setting_type == SettingType::SecretReference)
            .filter(|setting| {
                stored
                    .get(&setting.id)
                    .is_some_and(|value| *value != secret_reference(&self.id, &setting.id))
            })
            .map(|setting| {
                format!(
                    "{}: the secret setting \"{}\" may hold only its secret reference",
                    self.id, setting.id
                )
            })
            .collect()
    }
}

/// Every rule a manifest's settings break, and every `${settings.…}` written
/// where no setting can go.
pub(super) fn setting_problems(manifest: &Manifest, problems: &mut ValidationErrors) {
    const LABEL: &str =
        "Must be 1–120 characters with no control characters and no bidirectional or invisible format characters";
    let declared = &manifest.settings;
    if declared.len() > MAX_SETTINGS {
        problems.push(
            Code::InvalidValue,
            "settings",
            format!("Declare at most {MAX_SETTINGS} settings"),
        );
    }
    unique(
        problems,
        declared
            .iter()
            .enumerate()
            .map(|(index, setting)| (format!("settings[{index}].id"), setting.id.as_str())),
    );
    for (index, setting) in declared.iter().enumerate() {
        let at = format!("settings[{index}]");
        let kind = setting.setting_type;
        if !identifier(&setting.id) {
            problems.push(
                Code::InvalidValue,
                format!("{at}.id"),
                "Must be 1–64 letters, digits and -",
            );
        }
        if !label(&setting.title) {
            problems.push(Code::InvalidValue, format!("{at}.title"), LABEL);
        }
        if let Some(description) = &setting.description {
            if description.trim().is_empty()
                || description.chars().count() > MAX_SETTING_DESCRIPTION
                || !visible(description)
            {
                problems.push(
                    Code::InvalidValue,
                    format!("{at}.description"),
                    format!("Must be 1–{MAX_SETTING_DESCRIPTION} characters with no control or invisible format characters"),
                );
            }
        }
        let chooses = matches!(kind, SettingType::Select | SettingType::MultiSelect);
        if !chooses && !setting.options.is_empty() {
            problems.push(
                Code::InvalidField,
                format!("{at}.options"),
                "Only a select or multi-select setting lists options",
            );
        }
        if chooses {
            if setting.options.is_empty() || setting.options.len() > MAX_SETTING_OPTIONS {
                problems.push(
                    Code::InvalidValue,
                    format!("{at}.options"),
                    format!("List 1–{MAX_SETTING_OPTIONS} options"),
                );
            }
            for (position, option) in setting.options.iter().enumerate() {
                let option_at = format!("{at}.options[{position}]");
                if !label(&option.value) {
                    problems.push(Code::InvalidValue, format!("{option_at}.value"), LABEL);
                }
                if !label(&option.label) {
                    problems.push(Code::InvalidValue, format!("{option_at}.label"), LABEL);
                }
            }
            unique(
                problems,
                setting
                    .options
                    .iter()
                    .enumerate()
                    .map(|(position, option)| {
                        (
                            format!("{at}.options[{position}].value"),
                            option.value.as_str(),
                        )
                    }),
            );
        }
        if kind != SettingType::Number {
            for (field, present) in [
                ("minimum", setting.minimum.is_some()),
                ("maximum", setting.maximum.is_some()),
                ("integer", setting.integer),
            ] {
                if present {
                    problems.push(
                        Code::InvalidField,
                        format!("{at}.{field}"),
                        format!("Only a number setting takes `{field}`"),
                    );
                }
            }
        } else if let (Some(minimum), Some(maximum)) = (
            setting.minimum.as_ref().and_then(Number::as_f64),
            setting.maximum.as_ref().and_then(Number::as_f64),
        ) {
            if minimum > maximum {
                problems.push(
                    Code::InvalidValue,
                    format!("{at}.minimum"),
                    "minimum must not be above maximum",
                );
            }
        }
        match (kind, setting.max_length) {
            (SettingType::String, Some(most)) if most == 0 || most > MAX_SETTING_MAX_LENGTH => {
                problems.push(
                    Code::InvalidValue,
                    format!("{at}.maxLength"),
                    format!("maxLength must be 1–{MAX_SETTING_MAX_LENGTH}"),
                )
            }
            (SettingType::String, _) | (_, None) => {}
            (_, Some(_)) => problems.push(
                Code::InvalidField,
                format!("{at}.maxLength"),
                "Only a string setting takes `maxLength`",
            ),
        }
        if let Some(default) = &setting.default {
            let path = format!("{at}.default");
            if kind == SettingType::SecretReference {
                problems.push(
                    Code::InvalidField,
                    path,
                    "A secret has no default: a manifest is public, and a default would publish it",
                );
            } else if setting.required {
                problems.push(
                    Code::InvalidField,
                    path,
                    "A setting with a default is never missing; drop `required` or the default",
                );
            } else if let Err(why) = setting.check_value(default) {
                problems.push(Code::InvalidValue, path, why);
            }
        }
    }
    interpolation_problems(manifest, problems);
}

const NOT_HERE: &str =
    "Settings are interpolated only into a binding argument the host marks as settable";

/// `${settings.…}` anywhere but the whole value of a top-level binding or
/// action argument is refused here; whether that argument is settable, and
/// by a setting of this type, is the capability's to say
/// ([`crate::PluginHost::interpolate`]).
fn interpolation_problems(manifest: &Manifest, problems: &mut ValidationErrors) {
    let Ok(raw) = serde_json::to_value(manifest) else {
        return;
    };
    for (list, entries) in raw.as_object().into_iter().flatten() {
        let Value::Array(entries) = entries else {
            refuse_anywhere(entries, list, problems);
            continue;
        };
        for (index, entry) in entries.iter().enumerate() {
            let at = format!("{list}[{index}]");
            let binding = list == "capabilities" || list == "actions";
            let Value::Object(fields) = entry else {
                refuse_anywhere(entry, &at, problems);
                continue;
            };
            for (field, value) in fields {
                let path = format!("{at}.{field}");
                match value {
                    Value::Object(arguments) if binding && field == "arguments" => {
                        for (key, argument) in arguments {
                            argument_problems(
                                manifest,
                                &format!("{path}.{key}"),
                                key,
                                argument,
                                problems,
                            );
                        }
                    }
                    _ => refuse_anywhere(value, &path, problems),
                }
            }
        }
    }
}

/// One top-level binding argument: a whole-value reference to a declared,
/// non-secret setting that always has a value, or no reference at all.
fn argument_problems(
    manifest: &Manifest,
    path: &str,
    key: &str,
    argument: &Value,
    problems: &mut ValidationErrors,
) {
    if settings::mentions_setting(key) {
        problems.push(Code::InvalidBinding, path, NOT_HERE);
        return;
    }
    let id = match settings::reference(argument) {
        None => return refuse_anywhere(argument, path, problems),
        Some(Err(why)) => return problems.push(Code::InvalidBinding, path, why),
        Some(Ok(id)) => id,
    };
    match manifest.setting(id) {
        None => problems.push(
            Code::InvalidBinding,
            path,
            format!("No setting \"{id}\" is declared"),
        ),
        Some(setting) if setting.setting_type == SettingType::SecretReference => problems.push(
            Code::InvalidBinding,
            path,
            "A secret-reference setting is never interpolated; the host's secret store supplies a secret where a capability declares it",
        ),
        Some(setting) if !setting.required && setting.default.is_none() => problems.push(
            Code::InvalidBinding,
            path,
            format!("Setting \"{id}\" fills this argument, so it must be required or have a default"),
        ),
        Some(_) => {}
    }
}

/// A problem for every key and string at or under `value` that mentions a setting.
fn refuse_anywhere(value: &Value, path: &str, problems: &mut ValidationErrors) {
    match value {
        Value::String(text) if settings::mentions_setting(text) => {
            problems.push(Code::InvalidBinding, path, NOT_HERE)
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                refuse_anywhere(item, &format!("{path}[{index}]"), problems);
            }
        }
        Value::Object(fields) => {
            for (key, item) in fields {
                let at = format!("{path}.{key}");
                if settings::mentions_setting(key) {
                    problems.push(Code::InvalidBinding, &at, NOT_HERE);
                }
                refuse_anywhere(item, &at, problems);
            }
        }
        _ => {}
    }
}
