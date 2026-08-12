//! Master-password lifecycle for the secrets vault (issue #208 follow-up,
//! mqlens's model): the vault key derives from a user-chosen password
//! (argon2id, `vault.json` meta + verifier — see `vault.rs`), set up
//! MANDATORILY at first launch via the frontend `VaultGate`. Unlocking is by
//! password, or by the biometric skip (`vault_biometric.rs`) when enrolled.
//! An opt-in recovery copy of the password lives in one OS keychain entry,
//! read only by the explicit "Forgot password?" flow.

use std::sync::Arc;

use crate::vault::{self, Vault};
use crate::vault_biometric;

const MIN_PASSWORD_LEN: usize = 8;

/// Everything the `VaultGate` needs to decide what to render.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultStatus {
    /// `"setup-required"` (no master password yet — mandatory first-launch
    /// setup), `"locked"`, or `"unlocked"`.
    pub mode: &'static str,
    /// The vault's `key_source` verbatim, for Settings copy.
    pub key_source: &'static str,
    /// A usable biometric sensor exists on this machine.
    pub biometric_available: bool,
    /// The biometric skip is enrolled (marker present).
    pub biometric_enrolled: bool,
}

#[tauri::command]
pub async fn vault_status(
    app: tauri::AppHandle,
    vault: tauri::State<'_, Arc<Vault>>,
) -> Result<VaultStatus, String> {
    use tauri_plugin_biometry::BiometryExt;
    let dir = vault_biometric::vault_dir(&app)?;
    // EXISTENCE of vault.json decides the mode, not its readability: a
    // truncated/corrupt meta must present as locked (fail closed), never as
    // setup-required — setup would rekey the still-encrypted vault away.
    let has_meta = vault::meta_path(&dir).exists();
    let key_source = vault.key_source();
    let mode = if !has_meta {
        // Fresh install or an upgrade from the machine-key era: the gate
        // shows setup. A legacy vault stays readable underneath (its machine
        // key resolved silently), so setup migrates it losslessly.
        "setup-required"
    } else if vault.current_key().is_some() {
        "unlocked"
    } else {
        "locked"
    };
    Ok(VaultStatus {
        mode,
        key_source,
        biometric_available: app.biometry().status().map(|s| s.is_available).unwrap_or(false),
        biometric_enrolled: vault::biometric_marker_path(&dir).exists(),
    })
}

/// First-launch setup: derive the key, re-encrypt whatever the vault already
/// holds (lossless migration from the machine-key era), retire the machine
/// key's homes, and optionally store the recovery copy.
#[tauri::command]
pub async fn vault_setup_password(
    password: String,
    keep_recovery: bool,
    app: tauri::AppHandle,
    vault: tauri::State<'_, Arc<Vault>>,
) -> Result<(), String> {
    if password.len() < MIN_PASSWORD_LEN {
        return Err(format!("the master password must be at least {MIN_PASSWORD_LEN} characters"));
    }
    let dir = vault_biometric::vault_dir(&app)?;
    // EXISTENCE check, matching `vault_status`: a corrupt-but-present
    // vault.json is a fail-closed locked state, never a setup invitation.
    if vault::meta_path(&dir).exists() {
        return Err("a master password is already set".into());
    }
    // Setup requires a USABLE current key (fresh machine key, or the legacy
    // one being migrated). Any locked state — keychain outage, corrupt meta,
    // stray biometric marker — must fail closed: rekeying a vault we cannot
    // read would silently destroy its secrets.
    if vault.current_key().is_none() {
        return Err(
            "the vault is locked and cannot be re-keyed — restart srelens and try again".into(),
        );
    }
    let (meta, key) = vault::build_meta(&password)?;
    // Order matters: the RECOVERABLE step (keychain recovery copy) lands
    // first, before the irreversible transition — if it fails, nothing has
    // changed and setup can simply be retried. A rekey failure afterwards
    // rolls both the meta and the recovery copy back.
    if keep_recovery {
        vault::store_recovery_password(&password)?;
    } else {
        vault::delete_recovery_password();
    }
    vault::write_meta(&dir, &meta)?;
    if let Err(e) = vault.rekey_from_current(key, "password") {
        let _ = std::fs::remove_file(vault::meta_path(&dir));
        if keep_recovery {
            vault::delete_recovery_password();
        }
        return Err(e.to_string());
    }
    // Retire the machine-key homes: the password is the key's origin now.
    vault::delete_master_key_from_keychain();
    let _ = std::fs::remove_file(dir.join("master.key"));
    // Any machine-key-era biometric enrollment held the OLD key — purge it;
    // the user re-enables the skip afterwards (it then stores the new key).
    vault_biometric::purge(&app);
    Ok(())
}

#[tauri::command]
pub async fn vault_unlock_password(
    password: String,
    app: tauri::AppHandle,
    vault: tauri::State<'_, Arc<Vault>>,
) -> Result<(), String> {
    let dir = vault_biometric::vault_dir(&app)?;
    let meta = vault::read_meta(&dir).ok_or("no master password is set")?;
    let key = vault::unlock_key_for(&meta, &password)?;
    vault.unlock_with(key, "password")
}

/// The explicit "Forgot password?" flow: read the opt-in keychain recovery
/// copy (the OS guards this read), unlock the vault with it, and hand it
/// back for one-time display so the user can note it or change it.
#[tauri::command]
pub async fn vault_recover_password(
    app: tauri::AppHandle,
    vault: tauri::State<'_, Arc<Vault>>,
) -> Result<String, String> {
    let password = vault::read_recovery_password()?;
    let dir = vault_biometric::vault_dir(&app)?;
    if let Some(meta) = vault::read_meta(&dir) {
        let key = vault::unlock_key_for(&meta, &password)
            .map_err(|_| "the recovery copy no longer matches this vault's password")?;
        vault.unlock_with(key, "password")?;
    }
    Ok(password)
}

/// Change the password: verify the current one, re-encrypt under the new
/// derivation, refresh the biometric item (if enrolled) and the recovery
/// copy — but ONLY if one exists: the recovery choice was made at setup, and
/// a change must never silently reverse an explicit opt-out.
#[tauri::command]
pub async fn vault_change_password(
    current: String,
    new: String,
    app: tauri::AppHandle,
    vault: tauri::State<'_, Arc<Vault>>,
) -> Result<(), String> {
    if new.len() < MIN_PASSWORD_LEN {
        return Err(format!("the new password must be at least {MIN_PASSWORD_LEN} characters"));
    }
    let dir = vault_biometric::vault_dir(&app)?;
    let old_meta = vault::read_meta(&dir).ok_or("no master password is set")?;
    let current_key = vault::unlock_key_for(&old_meta, &current)?;
    // The vault may still be locked (changing straight from the gate's
    // "forgot my password → recovered → change it" path): unlock with the
    // just-verified current key first.
    if vault.current_key().is_none() {
        vault.unlock_with(current_key, "password")?;
    }
    let (new_meta, new_key) = vault::build_meta(&new)?;
    vault::write_meta(&dir, &new_meta)?;
    if let Err(e) = vault.rekey_from_current(new_key, "password") {
        let _ = vault::write_meta(&dir, &old_meta);
        return Err(e.to_string());
    }
    if vault::has_recovery_password() {
        vault::store_recovery_password(&new)?;
    }
    vault_biometric::refresh_stored_key(&app, &new_key);
    Ok(())
}
