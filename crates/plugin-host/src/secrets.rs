//! Secret settings (#543): where an app's `secret-reference` values live, and
//! the one path by which the host may put one into a request.
//!
//! The inventory holds only a reference (`secret_reference`). The value is
//! kept by a [`SecretStore`] the host supplies: on the desktop, the encrypted
//! vault whose master key is in the OS keychain. Nothing here is Tauri-aware.
//!
//! A secret is write-only from every caller: a person can set it, clear it
//! and see whether it is set. It is read back only by
//! [`crate::PluginHost::inject_secret`], for an argument a host capability
//! declares as a secret slot (`Capability::secret_slots`). No capability
//! declares one today; brokered HTTP (#568) is the first that will.
use crate::{secret_reference, Manifest, PluginHost};
use serde_json::{Map, Value};
use srelens_capability::settings::SettingType;
use srelens_capability::Capability;
use std::collections::BTreeSet;

/// The permission an app requests to keep secrets in the host's store. A
/// manifest lists it exactly when it declares a `secret-reference` setting,
/// and never binds it.
pub const SECRET_STORE_PERMISSION: &str = "extension.secretStore";

/// Where the store keeps `setting` of app `app`: the text inside the
/// inventory's `{"secretRef": …}`. Host-minted from the two ids, so one app's
/// setting can never name another app's secret.
pub fn secret_key(app: &str, setting: &str) -> String {
    format!("{app}/{setting}")
}

/// A secret's value in memory. It has no `Display`, no `Serialize` and a
/// `Debug` that prints nothing of it, so it cannot reach a response, a log
/// line, an error or a panic message by accident; [`SecretValue::expose`] is
/// the only way to the text.
#[derive(Clone)]
pub struct SecretValue(String);

impl SecretValue {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    /// The text, for the one place that must have it: the store writing it,
    /// or a request the host builds.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretValue(<redacted>)")
    }
}

/// The host's secret store. Every error is the store's own reason, and never
/// holds a value.
pub trait SecretStore: Send + Sync {
    /// Whether a secret can be stored now; `Err` says why not (no keychain,
    /// locked, no store on this host). Storing fails closed on it: a secret is
    /// never kept anywhere else instead.
    fn status(&self) -> Result<(), String>;
    /// What `status` would say, for a report, without doing anything to find
    /// out that only storing a secret should do (opening a vault nothing has
    /// opened yet, and so possibly prompting for the keychain). Defaults to
    /// `status`.
    fn peek_status(&self) -> Result<(), String> {
        self.status()
    }
    /// Keep `value` under `key`, replacing what was there.
    fn put(&self, key: &str, value: &SecretValue) -> Result<(), String>;
    /// Whether a value is kept under `key`.
    fn contains(&self, key: &str) -> Result<bool, String>;
    /// Delete every app secret whose key is not in `keep`. The inventory is
    /// the source of truth and the store follows it, so removing an app, a
    /// reset, or an update that drops a setting all delete through this.
    fn retain(&self, keep: &BTreeSet<String>) -> Result<(), String>;
    /// The value under `key`. Called only by [`PluginHost::inject_secret`].
    fn reveal(&self, key: &str) -> Result<Option<SecretValue>, String>;
}

/// A host with no secret store: the web host, and any registry built without
/// one. Stores nothing, so there is nothing to delete or reveal.
pub struct NoSecretStore;

impl SecretStore for NoSecretStore {
    fn status(&self) -> Result<(), String> {
        Err("This host has no secret store".into())
    }
    fn put(&self, _key: &str, _value: &SecretValue) -> Result<(), String> {
        Err("This host has no secret store".into())
    }
    fn contains(&self, _key: &str) -> Result<bool, String> {
        Ok(false)
    }
    fn retain(&self, _keep: &BTreeSet<String>) -> Result<(), String> {
        Ok(())
    }
    fn reveal(&self, _key: &str) -> Result<Option<SecretValue>, String> {
        Ok(None)
    }
}

impl Manifest {
    /// Whether this app declares a `secret-reference` setting.
    pub fn declares_secrets(&self) -> bool {
        self.settings
            .iter()
            .any(|setting| setting.setting_type == SettingType::SecretReference)
    }

    /// The store keys this app's `settings` hold a reference to: its own
    /// references for its declared secrets, and nothing else.
    pub fn secret_keys(&self, settings: &Map<String, Value>) -> BTreeSet<String> {
        self.settings
            .iter()
            .filter(|setting| setting.setting_type == SettingType::SecretReference)
            .filter(|setting| {
                settings.get(&setting.id) == Some(&secret_reference(&self.id, &setting.id))
            })
            .map(|setting| secret_key(&self.id, &setting.id))
            .collect()
    }
}

impl PluginHost {
    /// The stored secret for `setting`, to go into `argument` of `target`, or
    /// why not.
    ///
    /// **The one read path.** Everything must hold: `target` declares
    /// `argument` a secret slot, the app declares `setting` as a
    /// `secret-reference`, the app was granted [`SECRET_STORE_PERMISSION`],
    /// the saved setting is the host's own reference for it, and the store has
    /// a value. No reason names the value.
    ///
    /// It does not know whether the app is enabled, quarantined, or scoped to
    /// the cluster of the call: a caller (#568) reaches it only after the
    /// checks every app request makes (`resolver_app` in the registry), and
    /// must keep it that way.
    pub fn inject_secret(
        target: &Capability,
        argument: &str,
        manifest: &Manifest,
        grants: &[String],
        settings: &Map<String, Value>,
        setting: &str,
        store: &dyn SecretStore,
    ) -> Result<SecretValue, String> {
        if !target.takes_secret(argument) {
            return Err(format!("{} takes no secret in `{argument}`", target.id));
        }
        let declared = manifest
            .setting(setting)
            .filter(|declared| declared.setting_type == SettingType::SecretReference)
            .ok_or_else(|| format!("No secret setting \"{setting}\" is declared"))?;
        if !grants.iter().any(|grant| grant == SECRET_STORE_PERMISSION) {
            return Err(format!(
                "{} was not granted {SECRET_STORE_PERMISSION}",
                manifest.id
            ));
        }
        let not_set = || format!("Secret \"{}\" ({setting}) is not set", declared.title);
        if settings.get(setting) != Some(&secret_reference(&manifest.id, setting)) {
            return Err(not_set());
        }
        store
            .reveal(&secret_key(&manifest.id, setting))?
            .ok_or_else(not_set)
    }
}
