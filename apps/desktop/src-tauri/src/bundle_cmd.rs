//! Tauri commands over `bundle.rs` — the dialogs, the app config dir, the
//! settings capability and the vault. Everything with a decision in it lives
//! in `bundle.rs` (the `#28` seam); what is here is plumbing plus the two
//! rules that need the host to enforce them:
//!
//! - **Settings go through the settings capability, not a file write.** The
//!   capability holds the cross-process lock that keeps a GUI save and a
//!   headless MCP save from dropping each other's keys; writing
//!   `settings.json` directly from here would bypass it.
//! - **Secrets go through `Vault::update`.** An imported API key is written
//!   into this machine's vault, under this machine's master password. The
//!   bundle's passphrase never becomes a vault password, and no secret from a
//!   bundle is ever written to disk in the clear.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use tauri::{AppHandle, Manager, Runtime, State};
use tauri_plugin_dialog::DialogExt;

use crate::bridge::AppRegistry;
use crate::bundle::{self, Bundle, BundleSummary, ExportSources, Group, ImportReport};
use crate::vault::{Secrets, Vault};

/// Matches the vault's own master-password minimum: a bundle is as sensitive
/// as the vault it can carry, so it is not held to a weaker bar.
const MIN_PASSPHRASE_LEN: usize = 8;

const KUBECONFIG_FILES_KEY: &str = "srelens.kubeconfigFiles";

fn config_dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    app.path().app_config_dir().map_err(|e| e.to_string())
}

/// The settings document's `values` map, read through the capability.
async fn read_settings(registry: &AppRegistry) -> Result<BTreeMap<String, Value>, String> {
    let out = registry
        .0
        .invoke("settings.get", json!({}))
        .await
        .map_err(|e| format!("could not read settings: {e}"))?;
    Ok(out
        .get("values")
        .and_then(Value::as_object)
        .map(|map| map.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default())
}

/// Every kubeconfig this machine reads: the host's discovery (home `.kube`,
/// `KUBECONFIG`, the managed directory) plus whatever the reader added by
/// hand. Without the second half, a file picked through "Add kubeconfig…"
/// would be visible in the app and absent from its own backup.
fn kubeconfig_paths(settings: &BTreeMap<String, Value>) -> Vec<PathBuf> {
    let mut paths = crate::capabilities::all_kubeconfig_paths();
    let extra = settings
        .get(KUBECONFIG_FILES_KEY)
        .and_then(Value::as_array)
        .map(|entries| {
            entries.iter().filter_map(Value::as_str).map(PathBuf::from).collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for path in extra {
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    paths
}

/// The secrets an export should carry — or the refusal that stops it.
///
/// A locked vault is refused rather than exported empty. `Vault::load` answers
/// `Secrets::default()` whenever the derived key is absent (`read_secrets` in
/// `vault.rs`), and every locked state reaches that: so without this gate a
/// reader who ticked "include the API keys" on a locked vault would have got a
/// bundle with `secrets: None`, an export that reported success, and a preview
/// on the new machine listing no API keys — with nothing anywhere saying the
/// vault had been shut at the time. That is a failed read rendered as a fact
/// about the reader's setup, which is the one thing this codebase does not do.
fn secrets_for_export(vault: &Vault, include_secrets: bool) -> Result<Option<Secrets>, String> {
    if !include_secrets {
        return Ok(None);
    }
    if !vault.is_unlocked() {
        return Err(format!(
            "srelens's secrets vault is {} — unlock it and export again, or export without the API keys",
            vault.key_source()
        ));
    }
    // An app's secrets are never exported (#543): they are kept for that app
    // on this machine, and the inventory that references them is not carried
    // either.
    let mut secrets = vault.load();
    secrets.extension_secrets.clear();
    Ok(Some(secrets))
}

fn check_passphrase(passphrase: &str) -> Result<(), String> {
    if passphrase.chars().count() < MIN_PASSPHRASE_LEN {
        return Err(format!("the passphrase must be at least {MIN_PASSPHRASE_LEN} characters"));
    }
    Ok(())
}

/// Write the bundle's bytes wherever the reader chooses. `None` means they
/// cancelled the save dialog — not a failure, and not an empty file either.
#[tauri::command]
pub async fn bundle_export<R: Runtime>(
    app: AppHandle<R>,
    registry: State<'_, AppRegistry>,
    vault: State<'_, std::sync::Arc<Vault>>,
    passphrase: String,
    include_secrets: bool,
    filename: String,
) -> Result<Option<String>, String> {
    check_passphrase(&passphrase)?;
    let base = config_dir(&app)?;
    let settings = read_settings(&registry).await?;
    let paths = kubeconfig_paths(&settings);
    let secrets = secrets_for_export(&vault, include_secrets)?;

    let bundle = bundle::collect(ExportSources {
        base: &base,
        kubeconfig_paths: &paths,
        settings,
        secrets,
        app_version: app.package_info().version.to_string(),
        created: now_rfc3339(),
    });
    let sealed = bundle::seal(&passphrase, &bundle)?;

    let picked = app
        .dialog()
        .file()
        .set_file_name(&filename)
        .add_filter("srelens setup bundle", &["srelens"])
        .blocking_save_file();
    let Some(file) = picked else { return Ok(None) };
    let path = file.as_path().ok_or("invalid save path")?.to_path_buf();
    std::fs::write(&path, &sealed).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(Some(path.to_string_lossy().into_owned()))
}

/// Choose a bundle to import. Separate from the import itself so the reader
/// picks the file first and is asked for its passphrase second — being asked
/// for a passphrase before choosing what it unlocks reads backwards.
#[tauri::command]
pub async fn bundle_pick_file<R: Runtime>(app: AppHandle<R>) -> Result<Option<String>, String> {
    let picked = app
        .dialog()
        .file()
        .add_filter("srelens setup bundle", &["srelens"])
        .blocking_pick_file();
    let Some(file) = picked else { return Ok(None) };
    Ok(Some(file.as_path().ok_or("invalid path")?.to_string_lossy().into_owned()))
}

/// Open a bundle and describe it, writing nothing. The manifest lives inside
/// the ciphertext, so this needs the passphrase — which is also where a wrong
/// one is reported, before any import is offered.
#[tauri::command]
pub async fn bundle_preview(path: String, passphrase: String) -> Result<BundleSummary, String> {
    let raw = bundle::read_bundle_file(Path::new(&path))?;
    Ok(bundle::open(&passphrase, &raw)?.summary())
}

/// Apply the selected groups. Files first, then settings, then secrets: each
/// step's result is reported, and a later failure never un-writes an earlier
/// one, so the report is the truth about what is now on this machine.
#[tauri::command]
pub async fn bundle_import<R: Runtime>(
    app: AppHandle<R>,
    registry: State<'_, AppRegistry>,
    vault: State<'_, std::sync::Arc<Vault>>,
    path: String,
    passphrase: String,
    groups: Vec<Group>,
) -> Result<ImportReport, String> {
    let base = config_dir(&app)?;
    let raw = bundle::read_bundle_file(Path::new(&path))?;
    let opened = bundle::open(&passphrase, &raw)?;

    // `?`, not `unwrap_or_default()`. An empty map here is not a harmless
    // fallback: `kubeconfig_paths` would then return only the discovered paths
    // and drop everything under `srelens.kubeconfigFiles`, those files would be
    // missing from `existing`, and `apply_files` — which dedupes by comparing
    // against exactly this list — would write a second copy of each one. The
    // reader would get duplicated clusters and a report announcing them as
    // added, with nothing saying the settings read had failed.
    let existing = kubeconfig_paths(&read_settings(&registry).await?);
    let mut report = bundle::apply_files(&base, &opened, &groups, &existing)?;

    if groups.contains(&Group::Settings) {
        report.settings_written = import_settings(&registry, &opened).await?;
    }
    if groups.contains(&Group::Secrets) {
        report.secrets_written = import_secrets(&vault, &opened)?;
    }
    Ok(report)
}

/// Merge the bundle's settings in through the capability. Returns the keys
/// written, in the order the preview lists them.
async fn import_settings(
    registry: &AppRegistry,
    opened: &Bundle,
) -> Result<Vec<String>, String> {
    let values = bundle::importable_settings(opened);
    if values.is_empty() {
        return Ok(Vec::new());
    }
    let keys: Vec<String> = values.keys().cloned().collect();
    registry
        .0
        .invoke("settings.set", json!({ "values": values }))
        .await
        .map_err(|e| format!("could not write settings: {e}"))?;
    Ok(keys)
}

/// Write the bundle's secrets into THIS machine's vault. An existing local
/// key is left alone: the reader importing a bundle on a machine they have
/// already set up should not lose the key they are currently using to a
/// staler copy, and the report says which ones were skipped by omission.
fn import_secrets(vault: &Vault, opened: &Bundle) -> Result<Vec<String>, String> {
    let Some(incoming) = opened.secrets.clone() else { return Ok(Vec::new()) };
    let current = vault.load();
    let mut written = Vec::new();

    let mut next = Secrets::default();
    for (slug, key) in &incoming.llm_keys {
        if key.is_empty() || current.llm_keys.get(slug).is_some_and(|k| !k.is_empty()) {
            continue;
        }
        next.llm_keys.insert(slug.clone(), key.clone());
        written.push(slug.clone());
    }
    let token = incoming
        .mcp_token
        .filter(|t| !t.is_empty())
        .filter(|_| current.mcp_token.as_deref().unwrap_or_default().is_empty());
    if token.is_some() {
        written.push("MCP token".into());
    }

    if written.is_empty() {
        return Ok(written);
    }
    vault
        .update(|secrets| {
            secrets.llm_keys.extend(next.llm_keys);
            if let Some(token) = token {
                secrets.mcp_token = Some(token);
            }
        })
        .map_err(|e| format!("could not save the imported secrets: {e}"))?;
    Ok(written)
}

/// An RFC 3339 timestamp in UTC. Written out rather than pulled from a date
/// crate: this is the only place in the desktop crate that needs one, and it
/// is descriptive text in a preview, never parsed back.
fn now_rfc3339() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default();
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Howard Hinnant's `civil_from_days`, the standard days-since-epoch to
/// calendar-date conversion.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;



    #[test]
    fn a_short_passphrase_is_refused_before_anything_is_written() {
        assert!(check_passphrase("short").is_err());
        assert!(check_passphrase("longenough").is_ok());
        // Counted in characters, so a passphrase of eight non-ASCII ones is
        // not rejected for being seven bytes short of a byte-length check.
        assert!(check_passphrase("パスワード合言葉").is_ok());
    }

    #[test]
    fn extra_kubeconfig_paths_from_settings_join_the_discovered_ones() {
        let settings = BTreeMap::from([(
            KUBECONFIG_FILES_KEY.to_string(),
            json!(["/tmp/extra-a.yaml", "/tmp/extra-b.yaml"]),
        )]);
        let paths = kubeconfig_paths(&settings);
        assert!(paths.contains(&PathBuf::from("/tmp/extra-a.yaml")));
        assert!(paths.contains(&PathBuf::from("/tmp/extra-b.yaml")));
    }

    #[test]
    fn a_malformed_kubeconfig_files_setting_does_not_stop_an_export() {
        let settings =
            BTreeMap::from([(KUBECONFIG_FILES_KEY.to_string(), json!("not-an-array"))]);
        // Discovery still answers; the bad key contributes nothing.
        let _ = kubeconfig_paths(&settings);
    }

    fn vault_with(dir: &Path, secrets: Secrets) -> Vault {
        let vault = crate::vault::test_vault(dir);
        vault
            .update(|s| {
                *s = secrets;
            })
            .unwrap();
        vault
    }

    fn temp_dir(label: &str) -> PathBuf {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "srelens-bundle-cmd-{label}-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn bundle_with(secrets: Option<Secrets>) -> Bundle {
        Bundle { secrets, ..Bundle::default() }
    }

    #[test]
    fn an_imported_key_lands_in_this_machines_vault() {
        let dir = temp_dir("secrets-new");
        let vault = vault_with(&dir, Secrets::default());
        let incoming = Secrets {
            mcp_token: Some("token-from-the-old-machine".into()),
            llm_keys: BTreeMap::from([("anthropic".into(), "sk-ant-from-bundle".into())]),
            ..Default::default()
        };

        let written = import_secrets(&vault, &bundle_with(Some(incoming))).unwrap();

        assert_eq!(written, vec!["anthropic".to_string(), "MCP token".to_string()]);
        let stored = vault.load();
        assert_eq!(stored.llm_keys.get("anthropic").unwrap(), "sk-ant-from-bundle");
        assert_eq!(stored.mcp_token.as_deref(), Some("token-from-the-old-machine"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_key_this_machine_already_has_is_not_replaced_by_the_bundles_copy() {
        let dir = temp_dir("secrets-keep");
        let vault = vault_with(
            &dir,
            Secrets {
                mcp_token: Some("mine".into()),
                llm_keys: BTreeMap::from([("anthropic".into(), "sk-ant-mine".into())]),
                ..Default::default()
            },
        );
        let incoming = Secrets {
            mcp_token: Some("theirs".into()),
            llm_keys: BTreeMap::from([
                ("anthropic".into(), "sk-ant-theirs".into()),
                ("openai".into(), "sk-openai-theirs".into()),
            ]),
            ..Default::default()
        };

        let written = import_secrets(&vault, &bundle_with(Some(incoming))).unwrap();

        assert_eq!(written, vec!["openai".to_string()], "only the key this machine lacked");
        let stored = vault.load();
        assert_eq!(stored.llm_keys.get("anthropic").unwrap(), "sk-ant-mine");
        assert_eq!(stored.llm_keys.get("openai").unwrap(), "sk-openai-theirs");
        assert_eq!(stored.mcp_token.as_deref(), Some("mine"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_locked_vault_refuses_the_export_rather_than_writing_one_with_no_secrets() {
        // `Vault::load` answers `Secrets::default()` while locked, so without
        // the gate in `secrets_for_export` this export would SUCCEED, carry no
        // API keys, and say nothing — and the preview on the new machine would
        // show no secrets group at all, as though the reader had never had one.
        let dir = temp_dir("secrets-locked");
        let vault = vault_with(
            &dir,
            Secrets {
                mcp_token: None,
                llm_keys: BTreeMap::from([("anthropic".into(), "sk-ant-mine".into())]),
                ..Default::default()
            },
        );
        // Put the vault into password mode and shut it, the same three steps
        // setup + lock take: re-key onto a password-derived key, write the
        // meta whose existence IS password mode, then discard the key.
        let (meta, key) = crate::vault::build_meta("a master password").unwrap();
        vault.rekey_from_current(key, "password").unwrap();
        crate::vault::write_meta(&dir, &meta).unwrap();
        vault.discard_key().unwrap();

        assert!(!vault.is_unlocked(), "fixture precondition: the vault is locked");
        assert_eq!(
            vault.load(),
            Secrets::default(),
            "a locked vault reads as empty — which is exactly what makes the silent export possible"
        );

        let error = secrets_for_export(&vault, true).unwrap_err();
        assert!(error.contains("vault is"), "unexpected error: {error}");
        assert!(error.contains("unlock it"), "unexpected error: {error}");

        // An export that did not ask for secrets is untouched by this: it has
        // nothing to be wrong about.
        assert_eq!(secrets_for_export(&vault, false).unwrap(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unlocked_vault_exports_the_secrets_it_actually_holds() {
        let dir = temp_dir("secrets-unlocked");
        let held = Secrets {
            mcp_token: Some("tok".into()),
            llm_keys: BTreeMap::from([("anthropic".into(), "sk-ant-mine".into())]),
            ..Default::default()
        };
        let vault = vault_with(&dir, held.clone());
        assert!(vault.is_unlocked());

        assert_eq!(secrets_for_export(&vault, true).unwrap(), Some(held));
        assert_eq!(secrets_for_export(&vault, false).unwrap(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #543: an app's secret is kept for that app on this machine, and the
    /// bundle is how settings leave it. The export carries the API keys and
    /// the MCP token a person chose to include, never an app's secret.
    #[test]
    fn an_export_never_carries_an_apps_secret() {
        let dir = temp_dir("secrets-apps");
        let vault = vault_with(
            &dir,
            Secrets {
                mcp_token: Some("tok".into()),
                extension_secrets: BTreeMap::from([(
                    "org.example.metrics/token".into(),
                    "glc_app-secret-value".into(),
                )]),
                ..Default::default()
            },
        );
        let exported = secrets_for_export(&vault, true).unwrap().unwrap();
        assert!(exported.extension_secrets.is_empty(), "{exported:?}");
        assert_eq!(exported.mcp_token.as_deref(), Some("tok"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The other direction (review of #543): a bundle is a file anyone can
    /// hand you, and an app secret it carried would sit unreferenced on this
    /// machine. Import writes the API keys and the MCP token, nothing else.
    #[test]
    fn an_import_never_writes_an_apps_secret() {
        let dir = temp_dir("secrets-import-apps");
        let vault = vault_with(&dir, Secrets::default());
        let incoming = Secrets {
            llm_keys: BTreeMap::from([("anthropic".into(), "sk-ant-from-bundle".into())]),
            extension_secrets: BTreeMap::from([(
                "org.example.metrics/token".into(),
                "planted-by-a-bundle".into(),
            )]),
            ..Default::default()
        };
        import_secrets(&vault, &bundle_with(Some(incoming))).unwrap();
        let stored = vault.load();
        assert!(stored.extension_secrets.is_empty(), "{stored:?}");
        assert_eq!(stored.llm_keys.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_bundle_without_secrets_writes_nothing_to_the_vault() {
        let dir = temp_dir("secrets-none");
        let vault = vault_with(&dir, Secrets::default());
        assert!(import_secrets(&vault, &bundle_with(None)).unwrap().is_empty());
        assert_eq!(vault.load(), Secrets::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_export_timestamp_is_a_real_rfc_3339_utc_instant() {
        let now = now_rfc3339();
        assert_eq!(now.len(), 20, "unexpected timestamp: {now}");
        assert!(now.ends_with('Z') && now.contains('T'), "unexpected timestamp: {now}");
        // A fixed instant, so the conversion itself is checked rather than
        // just the shape: 2026-09-21T10:00:00Z.
        assert_eq!(civil_from_days(20_717), (2026, 9, 21));
        // Leap day, the case an off-by-one in the era arithmetic breaks.
        assert_eq!(civil_from_days(18_321), (2020, 2, 29));
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }
}
