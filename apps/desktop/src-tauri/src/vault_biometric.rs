//! The Touch ID gate for the vault master key (issue #208), following the
//! mqlens pattern: the key is stored in the OS biometric store
//! (`tauri-plugin-biometry` — macOS Touch ID keychain / Windows Hello), and
//! reading it back (`get_data`) raises the biometric prompt. While the gate
//! is on, the plain keychain entry is DELETED — the biometric item is the
//! key's only home — and a non-secret marker file tells vault resolution to
//! open `biometric-locked` instead of consulting the keyring (see
//! `vault::biometric_marker_path`).
//!
//! Lifecycle:
//! - enable: requires an unlocked vault; stores the cached key behind
//!   biometrics FIRST, then deletes the plain keychain entry and writes the
//!   marker — ordered so a failure never leaves the key homeless.
//! - unlock: prompts, verifies the returned key actually decrypts the vault
//!   (a stale item is purged and reported rather than accepted), and
//!   installs it for the rest of the run.
//! - disable: requires an unlocked vault; restores the plain keychain entry
//!   FIRST, then removes the biometric item and marker.

use std::sync::Arc;

use tauri_plugin_biometry::{BiometryExt, DataOptions, GetDataOptions, SetDataOptions};

use crate::vault::{self, Vault};

/// Biometric-store coordinates for the master key.
const BIO_DOMAIN: &str = "app.srelens.desktop.vault";
const BIO_NAME: &str = "master-key";
/// A name that never holds data, so deleting it to probe the store is a no-op.
const BIO_PROBE_NAME: &str = "store-probe";

/// What Settings needs to render the Touch ID control.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultBiometricStatus {
    /// A usable biometric sensor exists and the store serves this build.
    pub available: bool,
    /// The gate is on (the marker exists — the key lives behind biometrics).
    pub enabled: bool,
    /// The vault currently holds a usable key (whatever its source).
    pub unlocked: bool,
}

fn data_options() -> DataOptions {
    DataOptions { domain: BIO_DOMAIN.to_string(), name: BIO_NAME.to_string() }
}

pub(crate) fn vault_dir(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    use tauri::Manager;
    Ok(app.path().app_config_dir().map_err(|e| e.to_string())?.join("mcp"))
}

/// Biometric availability + gate state. Never hard-fails: a plugin/platform
/// error reads as unavailable so Settings simply hides the control.
#[tauri::command]
pub async fn vault_biometric_status(
    app: tauri::AppHandle,
    vault: tauri::State<'_, Arc<Vault>>,
) -> Result<VaultBiometricStatus, String> {
    let available = biometric_available(&app);
    let enabled = vault_dir(&app).map(|d| vault::biometric_marker_path(&d).exists()).unwrap_or(false);
    Ok(VaultBiometricStatus { available, enabled, unlocked: vault.current_key().is_some() })
}

/// The one availability rule, for Settings and the vault gate alike: the
/// sensor works AND the biometric store answers this build (#819).
pub(crate) fn biometric_available<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> bool {
    gate_available(app.biometry().status(), store_probe(app))
}

fn gate_available<E: std::fmt::Display>(
    sensor: Result<tauri_plugin_biometry::Status, E>,
    store: Result<(), E>,
) -> bool {
    if !sensor.map(|s| s.is_available).unwrap_or(false) {
        return false;
    }
    // Only with a working sensor: without one, the store's answer hides
    // nothing, and on Linux every call fails.
    match store {
        Ok(()) => true,
        Err(e) => {
            log::warn!("the biometric store refuses this build, so biometric unlock is hidden: {e}");
            false
        }
    }
}

/// After a failed delete of the key item: may it still be there? A store
/// that refuses this build reads as empty to it (#819), so its "not found"
/// proves nothing.
fn may_still_hold_key<E>(probe: Result<(), E>, present: Result<bool, E>) -> bool {
    probe.is_err() || present.unwrap_or(true)
}

/// Whether the biometric store will serve this build at all. A macOS build
/// signed without its App ID entitlement has a working Touch ID sensor, but
/// the data-protection keychain refuses its writes and deletes with
/// errSecMissingEntitlement (-34018) — while a read just reports "not found",
/// so `has_data` can't tell. Deleting a name that never holds data changes
/// nothing, and is `Ok` wherever the store works.
fn store_probe<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> tauri_plugin_biometry::Result<()> {
    app.biometry()
        .remove_data(DataOptions { domain: BIO_DOMAIN.to_string(), name: BIO_PROBE_NAME.to_string() })
}

/// Turn the gate ON: move the cached master key into the biometric store.
#[tauri::command]
pub async fn vault_biometric_enable(
    app: tauri::AppHandle,
    vault: tauri::State<'_, Arc<Vault>>,
) -> Result<(), String> {
    let key = vault
        .current_key()
        .ok_or("the vault is locked — unlock it before enabling biometric unlock")?;
    // Store behind biometrics FIRST; only then remove the plain entry and
    // write the marker. A failure at any step leaves the previous (working)
    // configuration in place.
    app.biometry()
        .set_data(SetDataOptions {
            domain: BIO_DOMAIN.to_string(),
            name: BIO_NAME.to_string(),
            data: vault::to_hex(&key),
        })
        .map_err(|e| format!("could not store the key in the biometric store: {e}"))?;
    let dir = vault_dir(&app)?;
    // A marker-write failure must take the just-stored item back out: the
    // enable reports failure, so no valid key may linger in the biometric
    // store as a usable-but-unacknowledged unlock method.
    if let Err(e) = std::fs::write(vault::biometric_marker_path(&dir), b"") {
        let _ = app.biometry().remove_data(data_options());
        return Err(e.to_string());
    }
    vault::delete_master_key_from_keychain();
    vault.set_key_source("biometric");
    Ok(())
}

/// Turn the gate OFF: restore the plain keychain entry as the key's home.
#[tauri::command]
pub async fn vault_biometric_disable(
    app: tauri::AppHandle,
    vault: tauri::State<'_, Arc<Vault>>,
) -> Result<(), String> {
    let key = vault
        .current_key()
        .ok_or("the vault is locked — pass the biometric unlock before disabling the gate")?;
    let dir = vault_dir(&app)?;
    // Mode is decided by vault.json's EXISTENCE (the fail-closed rule used
    // everywhere): a present-but-unreadable meta is password mode with the
    // password path broken — disabling then would remove the ONLY working
    // unlock, so refuse until the metadata is readable again.
    let password_mode = vault::meta_path(&dir).exists();
    if password_mode && vault::read_meta(&dir).is_none() {
        return Err(
            "the vault's password metadata is unreadable — biometric unlock is currently the only \
             working unlock and can't be disabled"
                .into(),
        );
    }
    // Legacy machine-key mode restores the plain entry FIRST — the key must
    // never be homeless.
    if !password_mode {
        vault::store_master_key_in_keychain(&key)?;
    }
    // Remove the biometric ITEM before committing the marker change: if the
    // store is unavailable the disable must fail with the marker intact —
    // reporting success while a valid key stays in the biometric store would
    // leave a supposedly disabled unlock method alive.
    if app.biometry().remove_data(data_options()).is_err()
        && may_still_hold_key(store_probe(&app), app.biometry().has_data(data_options()))
    {
        return Err("the biometric store is unavailable — try disabling again later".into());
    }
    match std::fs::remove_file(vault::biometric_marker_path(&dir)) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    vault.set_key_source(if password_mode { "password" } else { "keychain" });
    Ok(())
}

/// Re-store a (new) key in the biometric item if the gate is on — used by
/// password change so the enrolled skip keeps working. `Ok` when nothing is
/// enrolled or the refresh landed. On failure the enrollment is PURGED
/// (item + marker) and `Err` carries a user-facing note: silently keeping
/// the stale old-key item would just fail the next launch's auto prompt and
/// purge then, with no explanation of why the feature vanished.
pub(crate) fn refresh_stored_key(app: &tauri::AppHandle, key: &[u8; 32]) -> Result<(), String> {
    let enrolled = vault_dir(app)
        .map(|d| vault::biometric_marker_path(&d).exists())
        .unwrap_or(false);
    if !enrolled {
        return Ok(());
    }
    match app.biometry().set_data(SetDataOptions {
        domain: BIO_DOMAIN.to_string(),
        name: BIO_NAME.to_string(),
        data: vault::to_hex(key),
    }) {
        Ok(()) => Ok(()),
        Err(e) => {
            purge(app);
            Err(format!(
                "biometric unlock was turned off — its store could not be updated ({e}); re-enable it in Settings → Security"
            ))
        }
    }
}

/// Drop the biometric item + marker outright — used by password setup, which
/// mints a NEW key the old item could never match. Best-effort.
pub(crate) fn purge(app: &tauri::AppHandle) {
    if let Ok(dir) = vault_dir(app) {
        let _ = std::fs::remove_file(vault::biometric_marker_path(&dir));
    }
    let _ = app.biometry().remove_data(data_options());
}

/// Pass the gate: prompt (Touch ID sheet), verify the returned key against
/// the vault, and install it for the rest of the run. A stale item — one
/// that no longer decrypts the vault — is purged so the user isn't stuck in
/// a prompt loop that can never succeed.
#[tauri::command]
pub async fn vault_biometric_unlock(
    app: tauri::AppHandle,
    vault: tauri::State<'_, Arc<Vault>>,
) -> Result<(), String> {
    // The marker is the enrollment: without it, a leftover item in the
    // biometric store (e.g. from a failed enable) is not a sanctioned
    // unlock method and must not be consultable via a direct invocation.
    let dir = vault_dir(&app)?;
    if !vault::biometric_marker_path(&dir).exists() {
        return Err("biometric unlock is not enabled for this vault".into());
    }
    let resp = app
        .biometry()
        .get_data(GetDataOptions {
            domain: BIO_DOMAIN.to_string(),
            name: BIO_NAME.to_string(),
            reason: "Unlock srelens's secrets".to_string(),
            cancel_title: Some("Cancel".to_string()),
        })
        .map_err(|e| format!("biometric unlock failed: {e}"))?;
    let key = vault::key_from_hex(&resp.data)
        .ok_or("the stored biometric key is malformed")
        .map_err(|e| {
            // Purge marker AND item: leaving the marker would keep every
            // later launch auto-raising a prompt for an item that's gone.
            purge(&app);
            e.to_string()
        })?;
    vault.unlock_with(key, "biometric").map_err(|e| {
        purge(&app);
        format!("{e} — the stale biometric item was removed; later launches fall back to the password")
    })?;
    crate::vault_password::emit_vault_unlocked(&app)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri_plugin_biometry::{BiometryType, Status};

    const MISSING_ENTITLEMENT: &str = "Error adding item to keychain: -34018";

    fn sensor(is_available: bool) -> Result<Status, &'static str> {
        Ok(Status { is_available, biometry_type: BiometryType::TouchID, error: None, error_code: None })
    }

    #[test]
    fn a_store_that_refuses_the_probe_hides_the_switch() {
        // #819: an app signed without the keychain entitlement has a working
        // Touch ID sensor, but the store refuses the probe with -34018.
        assert!(!gate_available(sensor(true), Err(MISSING_ENTITLEMENT)));
    }

    #[test]
    fn a_store_that_answers_the_probe_offers_the_switch() {
        assert!(gate_available(sensor(true), Ok(())));
    }

    #[test]
    fn a_store_that_refuses_the_build_never_proves_the_key_gone() {
        // #819: the refused build's has_data reads "not found" — disable must
        // still fail rather than report the gate off over a stored key.
        assert!(may_still_hold_key(Err(MISSING_ENTITLEMENT), Ok(false)));
    }

    #[test]
    fn a_store_that_answers_decides_by_what_it_holds() {
        let answers: Result<(), &str> = Ok(());
        assert!(!may_still_hold_key(answers, Ok(false)));
        assert!(may_still_hold_key(answers, Ok(true)));
        assert!(may_still_hold_key(answers, Err(MISSING_ENTITLEMENT)));
    }

    // `cargo test` binaries are signed ad hoc, without the App ID entitlement:
    // the same position as the release builds #819 broke. Their keychain
    // reports every read as "not found" yet refuses deletes — which is why the
    // probe deletes, and what the first version of this fix, a `has_data`
    // probe, got wrong. Desktop CI is Linux-only, so this runs on a Mac.
    #[cfg(target_os = "macos")]
    #[test]
    fn an_unentitled_build_cannot_reach_the_store() {
        let app = tauri::test::mock_builder()
            .plugin(tauri_plugin_biometry::init())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        assert_eq!(app.biometry().has_data(data_options()).ok(), Some(false));
        assert!(store_probe(app.handle()).is_err());
    }
}
