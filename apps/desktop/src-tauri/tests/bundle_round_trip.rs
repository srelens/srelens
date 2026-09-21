//! The setup bundle end to end (issue #654), without a cluster and without a
//! window: machine A's real settings document and kubeconfig files go into a
//! sealed bundle, and machine B — a second, empty config directory with its
//! own settings document — comes out holding the same setup.
//!
//! The module's own unit tests cover the pieces. What is only reachable here
//! is the join between them and the REAL settings capability: exporting reads
//! `settings.get` and importing writes `settings.set`, and those go through
//! the registry the Tauri bridge and the MCP server both use, with its
//! cross-process lock and its 256-byte key validation. A bundle that round
//! trips in `bundle.rs` and is then rejected by the settings capability would
//! pass every unit test and fail on a reader's machine.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use srelens_desktop_lib::bundle::{self, ExportSources, Group};
use srelens_registry::build_registry_with_paths_and_settings;

const KUBECONFIG_A: &str = "\
apiVersion: v1
kind: Config
clusters:
- name: prod
  cluster: { server: 'https://10.0.0.1:6443' }
contexts:
- name: prod-eu
  context: { cluster: prod, user: ann }
users:
- name: ann
  user: {}
current-context: prod-eu
";

const KUBECONFIG_B: &str = "\
apiVersion: v1
kind: Config
clusters:
- name: staging
  cluster: { server: 'https://10.0.0.2:6443' }
contexts:
- name: staging-eu
  context: { cluster: staging, user: ann }
users:
- name: ann
  user: {}
current-context: staging-eu
";

fn temp_dir(label: &str) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "srelens-bundle-e2e-{label}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A registry built exactly as the desktop builds it, pointed at one machine's
/// settings document.
fn registry_for(settings_path: &Path) -> srelens_capability::Registry {
    build_registry_with_paths_and_settings(
        srelens_kube::client_cache::ClientCache::new_many(Vec::new()),
        Vec::new(),
        Some(settings_path.to_path_buf()),
    )
}

async fn settings_values(
    registry: &srelens_capability::Registry,
) -> BTreeMap<String, Value> {
    let out = registry.invoke("settings.get", json!({})).await.unwrap();
    out.get("values")
        .and_then(Value::as_object)
        .map(|map| map.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default()
}

#[tokio::test]
async fn a_setup_exported_on_one_machine_is_the_same_setup_on_another() {
    // --- machine A: a configured srelens -----------------------------------
    let machine_a = temp_dir("machine-a");
    let settings_a = machine_a.join("settings.json");
    let registry_a = registry_for(&settings_a);

    registry_a
        .invoke(
            "settings.set",
            json!({ "values": {
                "srelens.defaultNamespace": "kube-system",
                "srelens.contextProfiles": { "prod-eu": { "displayName": "Production EU", "color": "var(--mark-red)" } },
                "srelens.contextOrder": ["prod-eu", "staging-eu"],
                // The one key that must NOT survive: these paths exist only
                // on machine A.
                "srelens.kubeconfigFiles": ["/Users/ann/work/prod.yaml"],
            }}),
        )
        .await
        .unwrap();

    // Two kubeconfigs, reached by absolute path the way the app reads them.
    let kube_a = machine_a.join("prod.yaml");
    let kube_b = machine_a.join("staging.yaml");
    fs::write(&kube_a, KUBECONFIG_A).unwrap();
    fs::write(&kube_b, KUBECONFIG_B).unwrap();

    // An assistant skill the reader wrote.
    let skills_a = machine_a.join("assistant").join("skills");
    fs::create_dir_all(&skills_a).unwrap();
    fs::write(skills_a.join("triage.md"), "# Triage\nStart with events.").unwrap();

    let bundle = bundle::collect(ExportSources {
        base: &machine_a,
        kubeconfig_paths: &[kube_a, kube_b],
        settings: settings_values(&registry_a).await,
        secrets: None,
        app_version: "0.15.0".into(),
        created: "2026-09-21T10:00:00Z".into(),
    });

    let sealed = bundle::seal("correct horse battery staple", &bundle).unwrap();
    let carried = machine_a.join("srelens-setup.srelens");
    fs::write(&carried, &sealed).unwrap();

    // --- machine B: a fresh srelens ----------------------------------------
    let machine_b = temp_dir("machine-b");
    let settings_b = machine_b.join("settings.json");
    let registry_b = registry_for(&settings_b);
    assert!(settings_values(&registry_b).await.is_empty(), "machine B starts empty");

    let raw = bundle::read_bundle_file(&carried).unwrap();
    let opened = bundle::open("correct horse battery staple", &raw).unwrap();

    let groups = [Group::Settings, Group::Kubeconfigs, Group::Skills];
    let report = bundle::apply_files(&machine_b, &opened, &groups, &[]).unwrap();
    registry_b
        .invoke(
            "settings.set",
            json!({ "values": bundle::importable_settings(&opened) }),
        )
        .await
        .unwrap();

    // Both clusters arrived, in the directory the app enumerates on every
    // context listing — so they need no entry in any settings key to be seen.
    assert_eq!(report.kubeconfigs_added.len(), 2, "{report:?}");
    let managed = machine_b.join("kubeconfigs");
    let landed: Vec<String> = srelens_kube::connect::kubeconfig_files_in(&managed)
        .iter()
        .map(|p| fs::read_to_string(p).unwrap())
        .collect();
    assert!(landed.contains(&KUBECONFIG_A.to_string()));
    assert!(landed.contains(&KUBECONFIG_B.to_string()));

    // The reader's names and colours came with them.
    let on_b = settings_values(&registry_b).await;
    assert_eq!(on_b.get("srelens.defaultNamespace").unwrap(), "kube-system");
    assert_eq!(
        on_b.get("srelens.contextProfiles").unwrap(),
        &json!({ "prod-eu": { "displayName": "Production EU", "color": "var(--mark-red)" } })
    );
    assert_eq!(on_b.get("srelens.contextOrder").unwrap(), &json!(["prod-eu", "staging-eu"]));

    // Machine A's absolute paths did not. Carrying them would point machine B
    // at directories that do not exist there.
    assert!(
        !on_b.contains_key("srelens.kubeconfigFiles"),
        "machine A's kubeconfig paths reached machine B"
    );

    assert_eq!(report.skills_added, vec!["triage.md".to_string()]);
    assert_eq!(
        fs::read_to_string(machine_b.join("assistant").join("skills").join("triage.md")).unwrap(),
        "# Triage\nStart with events."
    );

    // --- importing the same file again is a no-op ---------------------------
    let existing = srelens_kube::connect::kubeconfig_files_in(&managed);
    let again = bundle::apply_files(&machine_b, &opened, &groups, &existing).unwrap();
    assert!(again.kubeconfigs_added.is_empty(), "{again:?}");
    assert_eq!(again.kubeconfigs_already_present.len(), 2);
    assert!(again.skills_added.is_empty());
    assert_eq!(
        srelens_kube::connect::kubeconfig_files_in(&managed).len(),
        2,
        "a second import duplicated the clusters"
    );

    let _ = fs::remove_dir_all(&machine_a);
    let _ = fs::remove_dir_all(&machine_b);
}

#[tokio::test]
async fn the_wrong_passphrase_leaves_the_second_machine_untouched() {
    let machine_a = temp_dir("wrong-a");
    let settings_a = machine_a.join("settings.json");
    let registry_a = registry_for(&settings_a);
    registry_a
        .invoke("settings.set", json!({ "values": { "srelens.defaultNamespace": "prod" }}))
        .await
        .unwrap();
    let kube = machine_a.join("prod.yaml");
    fs::write(&kube, KUBECONFIG_A).unwrap();

    let sealed = bundle::seal(
        "the right passphrase",
        &bundle::collect(ExportSources {
            base: &machine_a,
            kubeconfig_paths: &[kube],
            settings: settings_values(&registry_a).await,
            secrets: None,
            app_version: "0.15.0".into(),
            created: "2026-09-21T10:00:00Z".into(),
        }),
    )
    .unwrap();

    let machine_b = temp_dir("wrong-b");
    let error = bundle::open("the wrong passphrase", &sealed).unwrap_err();
    assert!(error.contains("passphrase"), "unexpected error: {error}");
    // Nothing was written before the refusal — the failure is at the door.
    assert!(!machine_b.join("kubeconfigs").exists());
    assert!(fs::read_dir(&machine_b).unwrap().next().is_none());

    let _ = fs::remove_dir_all(&machine_a);
    let _ = fs::remove_dir_all(&machine_b);
}
