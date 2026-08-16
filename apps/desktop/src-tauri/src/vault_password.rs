//! Master-password lifecycle for the secrets vault (issue #208 follow-up,
//! mqlens's model): the vault key derives from a user-chosen password
//! (argon2id, `vault.json` meta + verifier — see `vault.rs`), set up
//! MANDATORILY at first launch via the frontend `VaultGate`. Unlocking is by
//! password, or by the biometric skip (`vault_biometric.rs`) when enrolled.
//! An opt-in recovery copy of the password lives in one OS keychain entry,
//! read only by the explicit "Forgot password?" flow.

use std::path::Path;
use std::sync::Arc;

use crate::vault::{self, RecoveryStore, Vault};
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
    let biometric_available = app.biometry().status().map(|s| s.is_available).unwrap_or(false);
    Ok(status_core(&vault, &dir, biometric_available))
}

/// Everything but the Tauri plumbing (#28 seam): the AppHandle contributes
/// only the vault dir and the biometric-sensor probe, so the mode decision
/// itself is unit-tested here.
fn status_core(vault: &Vault, dir: &Path, biometric_available: bool) -> VaultStatus {
    // EXISTENCE of vault.json decides the mode, not its readability: a
    // truncated/corrupt meta must present as locked (fail closed), never as
    // setup-required — setup would rekey the still-encrypted vault away.
    let has_meta = vault::meta_path(dir).exists();
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
    VaultStatus {
        mode,
        key_source,
        biometric_available,
        biometric_enrolled: vault::biometric_marker_path(dir).exists(),
    }
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
    let dir = vault_biometric::vault_dir(&app)?;
    // The returned transition lock is HELD across the retirement and purge
    // below: released early, a second process could establish and re-key a
    // password (even refreshing a biometric item with its newer key) while
    // this process was still blocked here — then resume and purge that
    // valid enrollment out from under it.
    let _transition =
        setup_password_core(&vault, &dir, &vault::KeyringRecovery, &password, keep_recovery)?;
    // The machine key's KEYCHAIN home is retired here, not in the core: it
    // is the real OS keychain's `master-key` entry, and the #28 seam exists
    // precisely so the unit suite can never touch it — a test run on a
    // developer machine with a genuine machine-key-era vault must not
    // delete the credential that vault still decrypts with.
    vault::delete_master_key_from_keychain();
    // Any machine-key-era biometric enrollment held the OLD key — purge it;
    // the user re-enables the skip afterwards (it then stores the new key).
    // Plugin-bound, so it stays on the command side of the #28 seam.
    vault_biometric::purge(&app);
    Ok(())
}

/// The whole setup transaction minus the Tauri-plugin step (#28 seam):
/// recovery operations arrive as `&dyn RecoveryStore` so every path —
/// including the rollbacks — is unit-tested against in-memory doubles.
/// Success hands back the still-held transition lock so the command's
/// keychain retirement and biometric purge stay serialized with the setup.
fn setup_password_core(
    vault: &Vault,
    dir: &Path,
    recovery: &dyn RecoveryStore,
    password: &str,
    keep_recovery: bool,
) -> Result<std::fs::File, String> {
    if password.len() < MIN_PASSWORD_LEN {
        return Err(format!("the master password must be at least {MIN_PASSWORD_LEN} characters"));
    }
    // The whole setup — existence check, recovery mutation, stage, re-key,
    // promote — runs under the inter-process transition lock, so two
    // concurrent setups serialize and the loser sees "already set".
    let transition = vault::transition_lock(dir).map_err(|e| e.to_string())?;
    // EXISTENCE check, matching `vault_status`: a corrupt-but-present
    // vault.json is a fail-closed locked state, never a setup invitation.
    if vault::meta_path(dir).exists() {
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
    let (meta, key) = vault::build_meta(password)?;
    // Order matters: the RECOVERABLE step (keychain recovery copy) lands
    // first, before the transition — if it fails, nothing has changed and
    // setup can simply be retried.
    if keep_recovery {
        recovery.store(password)?;
        // The marker IS the opt-in as far as every later flow is concerned —
        // its write is part of the transaction, before anything irreversible:
        // a failure here rolls the copy back and aborts with nothing changed,
        // instead of stranding a stored copy no flow will ever consult.
        if let Err(e) = std::fs::write(vault::recovery_marker_path(dir), b"") {
            recovery.delete();
            return Err(format!("could not record the recovery choice: {e}"));
        }
    } else {
        // Opting out purges BOTH keychain accounts — a stale main or staged
        // copy from a prior install must not survive an explicit opt-out.
        recovery.delete();
        let _ = std::fs::remove_file(vault::recovery_marker_path(dir));
    }
    // A stale staged copy (prior install, interrupted change) never belongs
    // to a fresh setup either way.
    recovery.delete_staged();
    // Two-phase transition (crash-recoverable): stage the new meta as
    // `.next`, re-key, then promote — the transition lock is already held
    // from the top of this command. A crash before the re-key leaves the
    // machine-key vault intact (the stale stage is dropped at next open); a
    // crash after it is healed by the open-time promote — `vault.json`
    // never claims a key the vault doesn't have.
    // Staging failure must roll back the recovery artifacts too — they were
    // persisted just above, and a failed setup must leave NOTHING behind.
    if let Err(e) = vault::write_meta_next(dir, &meta) {
        if keep_recovery {
            recovery.delete();
            let _ = std::fs::remove_file(vault::recovery_marker_path(dir));
        }
        return Err(e);
    }
    if let Err(e) = vault.rekey_from_current(key, "password") {
        let _ = std::fs::remove_file(vault::meta_next_path(dir));
        if keep_recovery {
            recovery.delete();
            let _ = std::fs::remove_file(vault::recovery_marker_path(dir));
        }
        return Err(e.to_string());
    }
    vault::promote_meta_next(dir)?;
    // Retire the machine key's FILE home — dir-scoped, so safe under test.
    // Its keychain home is the command's to retire (see the caller): the
    // real OS entry must stay beyond this unit-testable core's reach.
    let _ = std::fs::remove_file(dir.join("master.key"));
    Ok(transition)
}

#[tauri::command]
pub async fn vault_unlock_password(
    password: String,
    app: tauri::AppHandle,
    vault: tauri::State<'_, Arc<Vault>>,
) -> Result<(), String> {
    let dir = vault_biometric::vault_dir(&app)?;
    // Also recovers an interrupted password transition (staged `.next` meta).
    vault::unlock_with_master_password(&vault, &dir, &password)
}

/// The explicit "Forgot password?" flow: read the opt-in keychain recovery
/// copy (the OS guards this read), unlock the vault with it, and hand it
/// back for one-time display so the user can note it or change it.
#[tauri::command]
pub async fn vault_recover_password(
    app: tauri::AppHandle,
    vault: tauri::State<'_, Arc<Vault>>,
) -> Result<String, String> {
    let dir = vault_biometric::vault_dir(&app)?;
    recover_password_core(&vault, &dir, &vault::KeyringRecovery)
}

/// The recover flow behind the #28 seam — see `vault_recover_password`.
fn recover_password_core(
    vault: &Vault,
    dir: &Path,
    recovery: &dyn RecoveryStore,
) -> Result<String, String> {
    // The filesystem opt-in marker is authoritative: an opted-out vault
    // never consults EITHER keychain account — a stale credential from a
    // prior install must not be revealed (or promoted) against the user's
    // explicit choice.
    if !vault::recovery_marker_path(dir).exists() {
        return Err("no recovery copy was stored for this vault".into());
    }
    // The main copy pairs with the current password; a STAGED copy (left by
    // a password change that crashed before its final promote) pairs with
    // the staged/promoted meta. Try main first, then the stage — whichever
    // unlocks is the truth, and a working staged copy finishes its promote.
    let main = recovery.read();
    if let Ok(password) = &main {
        if vault::unlock_with_master_password(vault, dir, password).is_ok() {
            return Ok(password.clone());
        }
    }
    if let Some(staged) = recovery.read_staged() {
        if vault::unlock_with_master_password(vault, dir, &staged).is_ok() {
            recovery.promote_staged();
            return Ok(staged);
        }
    }
    match main {
        // "No recovery copy was stored" beats the mismatch message when
        // nothing was ever stored.
        Err(e) => Err(e),
        Ok(_) => Err("the recovery copy no longer matches this vault's password".into()),
    }
}

/// Change the password: verify the current one, re-encrypt under the new
/// derivation, refresh the biometric item (if enrolled) and the recovery
/// copy — but ONLY if one exists: the recovery choice was made at setup, and
/// a change must never silently reverse an explicit opt-out.
#[tauri::command]
/// Returns an optional WARNING on success: a biometric-store refresh failure
/// is reconciled (the enrollment is purged) but must be surfaced, not silent.
pub async fn vault_change_password(
    current: String,
    new: String,
    app: tauri::AppHandle,
    vault: tauri::State<'_, Arc<Vault>>,
) -> Result<Option<String>, String> {
    let dir = vault_biometric::vault_dir(&app)?;
    // The returned transition lock is HELD across the biometric refresh:
    // released early, a second process's change could re-key the vault while
    // this one was still storing its (now old) key in the biometric item —
    // marker enrolled, key mismatched, and the next launch's biometric
    // unlock fails and purges the enrollment.
    let (new_key, _transition) =
        change_password_core(&vault, &dir, &vault::KeyringRecovery, &current, &new)?;
    // A refresh failure has already reconciled (purged) the enrollment —
    // pass its note along as a warning on an otherwise-successful change.
    // Plugin-bound, so it stays on the command side of the #28 seam.
    Ok(vault_biometric::refresh_stored_key(&app, &new_key).err())
}

/// The change transaction minus the biometric refresh (#28 seam): returns
/// the NEW key on success — plus the still-held transition lock, so the
/// command's refresh stays serialized with the re-key exactly as it was
/// before the extraction.
fn change_password_core(
    vault: &Vault,
    dir: &Path,
    recovery: &dyn RecoveryStore,
    current: &str,
    new: &str,
) -> Result<([u8; 32], std::fs::File), String> {
    if new.len() < MIN_PASSWORD_LEN {
        return Err(format!("the new password must be at least {MIN_PASSWORD_LEN} characters"));
    }
    // The WHOLE change — meta read, current-password verification, recovery
    // mutation, stage, re-key, promote — runs under one inter-process
    // transition lock. Two concurrent changes therefore serialize: the
    // loser re-reads the winner's meta here and its (now old) current
    // password is rejected cleanly, before it can touch the recovery copy.
    let transition = vault::transition_lock(dir).map_err(|e| e.to_string())?;
    let old_meta = vault::read_meta(dir).ok_or("no master password is set")?;
    let current_key = vault::unlock_key_for(&old_meta, current)?;
    // The vault may still be locked (changing straight from the gate's
    // "forgot my password → recovered → change it" path): unlock inline with
    // the just-verified key — NOT via unlock_with_master_password, which
    // takes the transition lock we already hold.
    if vault.current_key().is_none() {
        vault.unlock_with(current_key, "password")?;
    }
    // The recovery copy's fate must be KNOWN before anything changes: an
    // unreachable keychain fails the change up front, never leaving a stale
    // copy that a later "Forgot password?" would trust. The filesystem
    // opt-in marker is authoritative: opted-out vaults never consult the
    // keychain at all — a keychain-less host has nothing to refresh and must
    // still be able to change the password.
    let recovery_enabled = if vault::recovery_marker_path(dir).exists() {
        recovery
            .state()
            .map_err(|e| {
                format!("the OS keychain is unreachable, so the recovery copy can't be refreshed — try again later ({e})")
            })?
            .is_some()
    } else {
        false
    };
    if recovery_enabled {
        // STAGE the new copy — the main entry (which pairs with the current
        // password) is untouched until after the meta promote, so no crash
        // window leaves "Forgot password?" holding a value that unlocks
        // nothing: main pairs with the old meta, the stage with the staged
        // meta, and the recover flow tries both.
        recovery.store_staged(new)?;
    }
    let (new_meta, new_key) = match vault::build_meta(new) {
        Ok(built) => built,
        Err(e) => {
            recovery.delete_staged();
            return Err(e);
        }
    };
    // Same two-phase transition as setup: stage, re-key, promote — the
    // transition lock is already held from the top of this command; a crash
    // in between is healed at the next unlock (unlock_with_master_password).
    let _ = std::fs::remove_file(vault::meta_next_path(dir));
    if let Err(e) = vault::write_meta_next(dir, &new_meta) {
        recovery.delete_staged();
        return Err(e);
    }
    if let Err(e) = vault.rekey_from_current(new_key, "password") {
        let _ = std::fs::remove_file(vault::meta_next_path(dir));
        recovery.delete_staged();
        return Err(e.to_string());
    }
    vault::promote_meta_next(dir)?;
    if recovery_enabled {
        // Best-effort: a crash before this promote is covered by the recover
        // flow's staged-copy fallback, which finishes the promote itself.
        recovery.promote_staged();
    }
    Ok((new_key, transition))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::test_support::{MemKeychain, MemRecovery};

    fn test_pw(tag: &str) -> String {
        format!("{tag}-{}-passphrase", tag.len())
    }

    fn temp_dir(label: &str) -> std::path::PathBuf {
        // A counter, not `{:?}` of Instant: that debug form contains `:`,
        // which is invalid in a Windows path component.
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let d = std::env::temp_dir().join(format!(
            "srelens-vpw-{label}-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A fresh machine-key vault (what a first launch resolves to) over an
    /// in-memory keychain — the state `setup_password_core` migrates from.
    fn fresh_vault(dir: &std::path::Path) -> Vault {
        Vault::with_backend(dir, Box::new(MemKeychain::empty()))
    }

    #[test]
    fn status_walks_setup_locked_unlocked() {
        let dir = temp_dir("status");
        let vault = fresh_vault(&dir);
        assert_eq!(status_core(&vault, &dir, false).mode, "setup-required");

        let pw = test_pw("status");
        setup_password_core(&vault, &dir, &MemRecovery::default(), &pw, false).unwrap();
        let s = status_core(&vault, &dir, true);
        assert_eq!(s.mode, "unlocked");
        assert!(s.biometric_available);
        assert!(!s.biometric_enrolled);

        // A fresh open of the same dir starts password-locked.
        let reopened = fresh_vault(&dir);
        let s = status_core(&reopened, &dir, false);
        assert_eq!(s.mode, "locked");
        assert_eq!(s.key_source, "password-locked");
    }

    #[test]
    fn setup_rejects_short_passwords_and_double_setup() {
        let dir = temp_dir("setup-guards");
        let vault = fresh_vault(&dir);
        let recovery = MemRecovery::default();
        let e = setup_password_core(&vault, &dir, &recovery, "short", false).unwrap_err();
        assert!(e.contains("at least 8"), "got: {e}");

        let pw = test_pw("guards");
        setup_password_core(&vault, &dir, &recovery, &pw, false).unwrap();
        let e = setup_password_core(&vault, &dir, &recovery, &pw, false).unwrap_err();
        assert!(e.contains("already set"), "got: {e}");
    }

    #[test]
    fn setup_refuses_a_locked_vault() {
        // A vault opened AFTER someone else's setup starts locked — and a
        // locked vault must never be re-keyed (its secrets would be
        // destroyed). Meta is removed to make the state "no password set,
        // but no usable key either" (e.g. stray corruption cleanup).
        let dir = temp_dir("setup-locked");
        let first = fresh_vault(&dir);
        setup_password_core(&first, &dir, &MemRecovery::default(), &test_pw("locked"), false)
            .unwrap();
        let locked = fresh_vault(&dir);
        std::fs::remove_file(vault::meta_path(&dir)).unwrap();
        let e = setup_password_core(&locked, &dir, &MemRecovery::default(), &test_pw("x"), false)
            .unwrap_err();
        assert!(e.contains("locked"), "got: {e}");
    }

    #[test]
    fn setup_with_recovery_stores_the_copy_and_marker() {
        let dir = temp_dir("setup-recovery");
        let vault = fresh_vault(&dir);
        let recovery = MemRecovery::default();
        // A stale staged copy from a prior install must not survive setup.
        recovery.store_staged(&test_pw("stale-stage")).unwrap();
        let pw = test_pw("recover-on");
        setup_password_core(&vault, &dir, &recovery, &pw, true).unwrap();
        assert_eq!(recovery.main.lock().unwrap().as_deref(), Some(pw.as_str()));
        assert!(recovery.staged.lock().unwrap().is_none(), "stale stage purged");
        assert!(vault::recovery_marker_path(&dir).exists());
        // The machine-key file is retired.
        assert!(!dir.join("master.key").exists());
    }

    #[test]
    fn setup_opt_out_purges_a_stale_copy() {
        let dir = temp_dir("setup-optout");
        let vault = fresh_vault(&dir);
        let recovery = MemRecovery::default();
        recovery.store(&test_pw("stale-main")).unwrap();
        recovery.store_staged(&test_pw("stale-two")).unwrap();
        setup_password_core(&vault, &dir, &recovery, &test_pw("optout"), false).unwrap();
        assert!(recovery.main.lock().unwrap().is_none());
        assert!(recovery.staged.lock().unwrap().is_none());
        assert!(!vault::recovery_marker_path(&dir).exists());
    }

    #[test]
    fn setup_rolls_back_recovery_when_the_keychain_fails() {
        let dir = temp_dir("setup-broken");
        let vault = fresh_vault(&dir);
        let recovery = MemRecovery { broken: true, ..Default::default() };
        let e =
            setup_password_core(&vault, &dir, &recovery, &test_pw("broken"), true).unwrap_err();
        assert!(e.contains("unreachable"), "got: {e}");
        // Nothing changed: still setup-required, no marker.
        assert!(!vault::meta_path(&dir).exists());
        assert!(!vault::recovery_marker_path(&dir).exists());
    }

    #[test]
    fn unlock_and_secret_round_trip_across_reopen() {
        let dir = temp_dir("unlock");
        let vault = fresh_vault(&dir);
        let pw = test_pw("unlock");
        setup_password_core(&vault, &dir, &MemRecovery::default(), &pw, false).unwrap();
        vault.update(|s| s.mcp_token = Some(test_pw("token"))).unwrap();

        let reopened = fresh_vault(&dir);
        let e = vault::unlock_with_master_password(&reopened, &dir, &test_pw("wrong"))
            .unwrap_err();
        assert!(!e.is_empty());
        vault::unlock_with_master_password(&reopened, &dir, &pw).unwrap();
        assert_eq!(reopened.load().mcp_token.as_deref(), Some(test_pw("token").as_str()));
        assert_eq!(reopened.key_source(), "password");
    }

    #[test]
    fn recover_requires_the_marker_and_returns_the_matching_copy() {
        let dir = temp_dir("recover");
        let vault = fresh_vault(&dir);
        let recovery = MemRecovery::default();
        let pw = test_pw("recover");
        setup_password_core(&vault, &dir, &recovery, &pw, true).unwrap();

        let reopened = fresh_vault(&dir);
        assert_eq!(recover_password_core(&reopened, &dir, &recovery).unwrap(), pw);
        assert_eq!(reopened.key_source(), "password");

        // Without the marker, the copy is never consulted — opt-out is
        // authoritative even with a stale credential still stored.
        std::fs::remove_file(vault::recovery_marker_path(&dir)).unwrap();
        let e = recover_password_core(&reopened, &dir, &recovery).unwrap_err();
        assert!(e.contains("no recovery copy"), "got: {e}");
    }

    #[test]
    fn recover_falls_back_to_a_staged_copy_and_promotes_it() {
        // The crash window a change leaves: meta already promoted to the NEW
        // password, main recovery copy still the OLD one, stage holding the
        // new. Recover must try the stage and finish the promote.
        let dir = temp_dir("recover-staged");
        let vault = fresh_vault(&dir);
        let recovery = MemRecovery::default();
        let old_pw = test_pw("old");
        setup_password_core(&vault, &dir, &recovery, &old_pw, true).unwrap();
        let new_pw = test_pw("new");
        change_password_core(&vault, &dir, &recovery, &old_pw, &new_pw).unwrap();
        // Simulate the crash: regress main to the old copy, stage the new.
        *recovery.main.lock().unwrap() = Some(old_pw.clone());
        *recovery.staged.lock().unwrap() = Some(new_pw.clone());

        let reopened = fresh_vault(&dir);
        assert_eq!(recover_password_core(&reopened, &dir, &recovery).unwrap(), new_pw);
        assert_eq!(recovery.main.lock().unwrap().as_deref(), Some(new_pw.as_str()));
        assert!(recovery.staged.lock().unwrap().is_none(), "stage promoted");
    }

    #[test]
    fn recover_reports_a_mismatched_copy() {
        let dir = temp_dir("recover-mismatch");
        let vault = fresh_vault(&dir);
        let recovery = MemRecovery::default();
        setup_password_core(&vault, &dir, &recovery, &test_pw("real"), true).unwrap();
        *recovery.main.lock().unwrap() = Some(test_pw("mismatched"));

        let reopened = fresh_vault(&dir);
        let e = recover_password_core(&reopened, &dir, &recovery).unwrap_err();
        assert!(e.contains("no longer matches"), "got: {e}");
    }

    #[test]
    fn change_verifies_the_current_password_and_rotates_the_key() {
        let dir = temp_dir("change");
        let vault = fresh_vault(&dir);
        let recovery = MemRecovery::default();
        let old_pw = test_pw("change-old");
        setup_password_core(&vault, &dir, &recovery, &old_pw, false).unwrap();
        vault.update(|s| s.mcp_token = Some(test_pw("keep"))).unwrap();

        let e = change_password_core(&vault, &dir, &recovery, &test_pw("wrong-cur"), &test_pw("n"))
            .unwrap_err();
        assert!(!e.is_empty());
        let short = &test_pw("s")[..1];
        let e = change_password_core(&vault, &dir, &recovery, &old_pw, short).unwrap_err();
        assert!(e.contains("at least 8"), "got: {e}");

        let new_pw = test_pw("change-new");
        change_password_core(&vault, &dir, &recovery, &old_pw, &new_pw).unwrap();
        // The old password no longer unlocks; the new one does, and the
        // secrets survived the re-encryption.
        let reopened = fresh_vault(&dir);
        assert!(vault::unlock_with_master_password(&reopened, &dir, &old_pw).is_err());
        vault::unlock_with_master_password(&reopened, &dir, &new_pw).unwrap();
        assert_eq!(reopened.load().mcp_token.as_deref(), Some(test_pw("keep").as_str()));
    }

    #[test]
    fn change_refreshes_the_recovery_copy_only_when_opted_in() {
        let dir = temp_dir("change-recovery");
        let vault = fresh_vault(&dir);
        let recovery = MemRecovery::default();
        let old_pw = test_pw("cr-old");
        setup_password_core(&vault, &dir, &recovery, &old_pw, true).unwrap();
        let new_pw = test_pw("cr-new");
        change_password_core(&vault, &dir, &recovery, &old_pw, &new_pw).unwrap();
        assert_eq!(recovery.main.lock().unwrap().as_deref(), Some(new_pw.as_str()));
        assert!(recovery.staged.lock().unwrap().is_none(), "stage promoted after the meta");
    }

    #[test]
    fn change_fails_up_front_when_the_recovery_keychain_is_unreachable() {
        // Opted-in vault, but the keychain can't be asked: failing the whole
        // change beats silently stranding a stale copy that a later
        // "Forgot password?" would trust.
        let dir = temp_dir("change-broken");
        let vault = fresh_vault(&dir);
        let working = MemRecovery::default();
        let old_pw = test_pw("cb-old");
        setup_password_core(&vault, &dir, &working, &old_pw, true).unwrap();

        let broken = MemRecovery { broken: true, ..Default::default() };
        let e = change_password_core(&vault, &dir, &broken, &old_pw, &test_pw("cb-new"))
            .unwrap_err();
        assert!(e.contains("unreachable"), "got: {e}");
        // The change never started: the old password still unlocks.
        let reopened = fresh_vault(&dir);
        vault::unlock_with_master_password(&reopened, &dir, &old_pw).unwrap();
    }

    #[test]
    fn change_from_the_locked_gate_unlocks_inline() {
        // The "forgot → recovered → change it now" path: the vault is still
        // locked when the change begins; verification of the current
        // password must unlock it inline rather than failing.
        let dir = temp_dir("change-locked");
        let first = fresh_vault(&dir);
        let recovery = MemRecovery::default();
        let old_pw = test_pw("cl-old");
        setup_password_core(&first, &dir, &recovery, &old_pw, false).unwrap();

        let locked = fresh_vault(&dir);
        assert!(locked.current_key().is_none());
        let new_pw = test_pw("cl-new");
        change_password_core(&locked, &dir, &recovery, &old_pw, &new_pw).unwrap();
        assert!(locked.current_key().is_some(), "unlocked inline by the change");
    }
}
