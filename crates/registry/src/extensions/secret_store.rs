//! `extension.secretStore` (#543): set, clear and report an app's secret
//! settings, write-only.
//!
//! The value goes to the host's [`SecretStore`] (on the desktop, the vault
//! whose master key is in the OS keychain) and the inventory keeps only the
//! host-minted reference, written after the value is stored. The inventory is
//! the source of truth and the store follows it: every inventory change ends
//! with [`sweep`], which deletes each stored secret no app references any
//! more. That one rule is how removing an app, a reset, and an update or
//! rollback that drops a secret setting all delete the secret, and it also
//! collects anything a failed step left behind.
//!
//! Nothing here returns, logs or quotes a value: the answer is `{set}`, every
//! refusal is written without the value, and the input's `secret` refuses a
//! value it cannot take without repeating it.
use super::{read, write, Installed, Inventory};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use srelens_capability::settings::SettingType;
use srelens_capability::{Annotations, Capability, CapabilityError, Impact, Registry};
use srelens_plugin_host::{
    secret_key, secret_reference, SecretStore, SecretValue, SECRET_STORE_PERMISSION,
};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The most a secret may hold, in bytes: room for a long token or a PEM key.
pub const MAX_SECRET_BYTES: usize = 16 * 1024;

/// Sensitive, because what goes through it is secret material; mutating and
/// gated, because it changes what the host keeps for an app. Host-authored
/// wording, like every other gated capability (#548).
pub const SECRET_STORE_ANNOTATIONS: Annotations = Annotations {
    read_only: false,
    destructive: false,
    requires_confirm: true,
    sensitive: true,
    impact: Impact::Medium,
    confirm: Some("Change a secret an app keeps in srelens's secrets vault[ ({action})]?"),
};

#[derive(Deserialize, JsonSchema)]
#[serde(tag = "action", deny_unknown_fields)]
enum SecretIn {
    /// Keep `secret` for the app's `setting`, replacing what was kept.
    #[serde(rename = "set")]
    Set {
        id: String,
        setting: String,
        /// The value: 1–16384 bytes of text with no NUL. Never returned.
        #[serde(deserialize_with = "secret_text")]
        #[schemars(with = "String", length(min = 1, max = 16384))]
        secret: SecretValue,
    },
    /// Delete the app's `setting`, or every secret the app keeps when no
    /// setting is named. Needs no grant: deleting is always allowed.
    #[serde(rename = "clear")]
    Clear {
        id: String,
        #[serde(default)]
        setting: Option<String>,
    },
}

/// `secret` as text, refused without echoing it: serde's own message for a
/// wrong type quotes the value it was given.
fn secret_text<'de, D: Deserializer<'de>>(deserializer: D) -> Result<SecretValue, D::Error> {
    const WHY: &str = "`secret` must be 1–16384 bytes of text with no NUL character";
    match Value::deserialize(deserializer) {
        Ok(Value::String(text))
            if !text.is_empty() && text.len() <= MAX_SECRET_BYTES && !text.contains('\0') =>
        {
            Ok(SecretValue::new(text))
        }
        _ => Err(serde::de::Error::custom(WHY)),
    }
}

/// Whether the setting is set after the call. Never the value.
#[derive(Serialize, JsonSchema)]
struct SecretOut {
    set: bool,
}

/// Whether the host can store a secret now, as `extensions.list` reports it.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct SecretStoreState {
    pub available: bool,
    /// Why not, in the host's words, when it cannot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl SecretStoreState {
    pub(super) fn of(store: &dyn SecretStore) -> Self {
        match store.peek_status() {
            Ok(()) => Self {
                available: true,
                reason: None,
            },
            Err(reason) => Self {
                available: false,
                reason: Some(reason),
            },
        }
    }
}

/// The store keys every installed app references.
fn referenced(state: &Inventory) -> BTreeSet<String> {
    state
        .plugins
        .iter()
        .flat_map(|app| app.manifest.secret_keys(&app.settings))
        .collect()
}

/// Delete every stored secret the inventory no longer references. Called
/// after each inventory write, under the inventory's write lock. A store that
/// cannot delete now (locked) leaves the secret unreferenced, where nothing
/// can reach it, and the next change deletes it; the change itself stands.
pub(super) fn sweep(store: &dyn SecretStore, state: &Inventory) {
    let _ = store.retain(&referenced(state));
}

/// For `extensions.list`: the store's state, and each reference whose value
/// the store no longer holds left out, so "Set" is never claimed for a secret
/// that is gone. While the store cannot be asked, the references stay: that
/// they were set is all that is known.
pub(super) fn report(store: &dyn SecretStore, state: &mut Inventory) {
    let status = SecretStoreState::of(store);
    if status.available {
        for app in &mut state.plugins {
            let gone: Vec<String> = app
                .manifest
                .settings
                .iter()
                .filter(|declared| declared.setting_type == SettingType::SecretReference)
                .filter(|declared| {
                    app.settings.get(&declared.id)
                        == Some(&secret_reference(&app.manifest.id, &declared.id))
                })
                .filter(|declared| {
                    matches!(
                        store.contains(&secret_key(&app.manifest.id, &declared.id)),
                        Ok(false)
                    )
                })
                .map(|declared| declared.id.clone())
                .collect();
            for setting in gone {
                app.settings.remove(&setting);
            }
        }
    }
    state.secret_store = Some(status);
}

/// Whether the app `id` installed in the inventory at `inventory` keeps
/// secrets, and with `setting`, whether that is one of its declared secret
/// settings. What a consent prompt may name: anything else in a call is the
/// caller's text, and a caller can put a secret there. An inventory that
/// cannot be read answers `false`, which shows less, never more.
pub fn declares_secret_setting(inventory: &Path, id: &str, setting: Option<&str>) -> bool {
    let Ok(state) = read(inventory) else {
        return false;
    };
    state
        .plugins
        .iter()
        .find(|app| app.manifest.id == id)
        .is_some_and(|app| match setting {
            Some(setting) => is_secret(app, setting),
            None => app.manifest.declares_secrets(),
        })
}

/// A setting that is not one of the app's declared secrets. Names neither the
/// app nor the setting: both are the caller's text, and a caller that swaps
/// fields puts the secret there.
const NO_SUCH_SECRET: &str = "This app declares no secret setting by that name";

fn app<'a>(state: &'a mut Inventory, id: &str) -> Result<&'a mut Installed, String> {
    state
        .plugins
        .iter_mut()
        .find(|app| app.manifest.id == id)
        .ok_or_else(|| "Extension is not installed".into())
}

fn is_secret(app: &Installed, setting: &str) -> bool {
    app.manifest
        .setting(setting)
        .is_some_and(|declared| declared.setting_type == SettingType::SecretReference)
}

fn change(path: &Path, store: &dyn SecretStore, input: SecretIn) -> Result<SecretOut, String> {
    let _lock = crate::settings::write_lock(path)?;
    let mut state = read(path)?;
    let set = match input {
        SecretIn::Set {
            id,
            setting,
            secret,
        } => {
            let app = app(&mut state, &id)?;
            if let Some(reason) = &app.quarantined {
                return Err(format!("This app can't keep secrets: {reason}"));
            }
            if !is_secret(app, &setting) {
                return Err(NO_SUCH_SECRET.into());
            }
            if !app
                .grants
                .iter()
                .any(|grant| grant == SECRET_STORE_PERMISSION)
            {
                return Err(format!(
                    "This app was not granted {SECRET_STORE_PERMISSION}, so it cannot keep secrets"
                ));
            }
            store.status().map_err(|why| {
                format!("The secret was not stored: {why}. Nothing was saved in plain text.")
            })?;
            // The value first, then the reference: a reference never points
            // at nothing, and a value whose reference was not written is
            // unreferenced, so the next sweep deletes it.
            store
                .put(&secret_key(&id, &setting), &secret)
                .map_err(|why| format!("The secret was not stored: {why}"))?;
            app.settings
                .insert(setting.clone(), secret_reference(&id, &setting));
            true
        }
        SecretIn::Clear { id, setting } => {
            let app = app(&mut state, &id)?;
            let settings: Vec<String> = match setting {
                Some(setting) if is_secret(app, &setting) => vec![setting],
                Some(_) => return Err(NO_SUCH_SECRET.into()),
                None => app
                    .manifest
                    .settings
                    .iter()
                    .filter(|declared| declared.setting_type == SettingType::SecretReference)
                    .map(|declared| declared.id.clone())
                    .collect(),
            };
            for setting in settings {
                app.settings.remove(&setting);
            }
            false
        }
    };
    write(path, &state)?;
    sweep(store, &state);
    // An inventory write like any other, so the app streams and the windows
    // listening to them hear of it (#566). They hear only that it changed.
    super::streams::announce(path, &state);
    Ok(SecretOut { set })
}

pub(super) fn register(reg: &mut Registry, path: PathBuf, store: Arc<dyn SecretStore>) {
    let mut capability = Capability::typed::<SecretIn, SecretOut, _, _>(
        SECRET_STORE_PERMISSION,
        "Set or clear a secret an app keeps in srelens's encrypted secrets vault; write-only, never returns a value; requires approval",
        SECRET_STORE_ANNOTATIONS,
        move |input| {
            let path = path.clone();
            let store = store.clone();
            async move {
                tokio::task::spawn_blocking(move || change(&path, store.as_ref(), input))
                    .await
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?
                    .map_err(CapabilityError::Handler)
            }
        },
    );
    // Serde quotes what it cannot read (`invalid type: string "…"`, `unknown
    // variant "…"`), and a caller can put the secret anywhere, including where
    // a name goes. Every value the call carried is scrubbed from every
    // refusal, the way the audit log scrubs errors, before the message
    // reaches the caller, the desktop log (`bridge.rs`) or an MCP client. The
    // one value kept is a recognised `action`, so "set" and "clear" survive
    // in the words of a message.
    let typed = capability.handler.clone();
    capability.handler = Arc::new(move |input: Value| {
        let typed = typed.clone();
        Box::pin(async move {
            let kept = match input.get("action").and_then(Value::as_str) {
                Some(action @ ("set" | "clear")) => serde_json::json!([action]),
                _ => Value::Null,
            };
            let scrub = |why: String| srelens_capability::audit::redact_error(&why, &input, &kept);
            typed(input.clone()).await.map_err(|error| match error {
                CapabilityError::InvalidInput(why) => CapabilityError::InvalidInput(scrub(why)),
                CapabilityError::Handler(why) => CapabilityError::Handler(scrub(why)),
                CapabilityError::NotFound(why) => CapabilityError::NotFound(scrub(why)),
            })
        })
    });
    reg.register(capability);
}
