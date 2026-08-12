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

/// What Settings needs to render the Touch ID control.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultBiometricStatus {
    /// A usable biometric sensor exists on this machine.
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

/// Sensor availability + gate state. Never hard-fails: a plugin/platform
/// error reads as unavailable so Settings simply hides the control.
#[tauri::command]
pub async fn vault_biometric_status(
    app: tauri::AppHandle,
    vault: tauri::State<'_, Arc<Vault>>,
) -> Result<VaultBiometricStatus, String> {
    let available = app.biometry().status().map(|s| s.is_available).unwrap_or(false);
    let enabled = vault_dir(&app).map(|d| vault::biometric_marker_path(&d).exists()).unwrap_or(false);
    Ok(VaultBiometricStatus { available, enabled, unlocked: vault.current_key().is_some() })
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
    if app.biometry().remove_data(data_options()).is_err() {
        let still_present = app.biometry().has_data(data_options()).unwrap_or(true);
        if still_present {
            return Err("the biometric store is unavailable — try disabling again later".into());
        }
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
    })
}
