//! Installing executable apps (#574): only from a package that carries exactly the
//! binaries its sidecar names, only with a verified publisher or the unsigned-apps
//! setting, and never run from a binary that changed on disk since.
use super::tests::{configure, fake_core};
use super::*;
use base64::Engine as _;

/// The executable app the sidecar and tool tests install.
pub(super) const SCANNER: &str = "org.example.scanner";

/// A scanner that does its work in its sidecar, shipping a binary for every platform a
/// package may carry so the tests run wherever the host does.
pub(super) fn scanner_manifest() -> Value {
    let binaries: serde_json::Map<String, Value> = srelens_plugin_host::SIDECAR_PLATFORMS
        .iter()
        .map(|platform| {
            (
                platform.to_string(),
                json!(format!("bin/{platform}/scanner")),
            )
        })
        .collect();
    json!({
        "id": SCANNER, "name": "Scanner", "version": "1.0.0", "srelensApiVersion": "^0.6",
        "kind": "executable", "permissions": [], "capabilities": [],
        "sidecar": {"binaries": binaries, "operations": [
            {"name": "scan", "title": "Scan an image", "inputs": [
                {"name": "image", "type": "string", "required": true, "maxLength": 512}]},
            {"name": "status", "title": "Scanner status"}]},
        "contributions": {"pages": [], "detailTabs": [], "detailLinks": []}
    })
}

/// A package of `manifest` carrying `binaries`, by package path.
pub(super) fn package_of(manifest: &Value, binaries: &[(&str, &[u8])]) -> Vec<u8> {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join(package::MANIFEST),
        serde_json::to_vec_pretty(manifest).unwrap(),
    )
    .unwrap();
    for (path, bytes) in binaries {
        let target = dir.path().join(path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, bytes).unwrap();
    }
    fs::write(
        dir.path().join(package::DIGESTS),
        package::digest_list(dir.path()).unwrap(),
    )
    .unwrap();
    package::pack(dir.path()).unwrap()
}

/// The scanner's package, with the same stand-in binary for every platform.
pub(super) fn scanner_package() -> Vec<u8> {
    let paths: Vec<String> = srelens_plugin_host::SIDECAR_PLATFORMS
        .iter()
        .map(|platform| format!("bin/{platform}/scanner"))
        .collect();
    let binaries: Vec<(&str, &[u8])> = paths
        .iter()
        .map(|path| (path.as_str(), &b"#!/bin/false\n"[..]))
        .collect();
    package_of(&scanner_manifest(), &binaries)
}

/// `installPackage`, reviewed against the app's current revision.
pub(super) fn install_package(path: &Path, archive: &[u8]) -> Result<Inventory, String> {
    let reviewed = read(path)?
        .plugins
        .iter()
        .find(|app| app.manifest.id == SCANNER)
        .map(|app| app.revision);
    configure(
        path,
        json!({"action": "installPackage",
            "package": base64::engine::general_purpose::STANDARD.encode(archive), "grants": [],
            "reviewedRevision": reviewed}),
    )
}

/// The scanner, installed with the unsigned-apps setting on: its revision.
pub(super) fn install_scanner(path: &Path) -> u64 {
    configure(
        path,
        json!({"action": "unsignedApps", "allowUnsignedApps": true}),
    )
    .unwrap();
    let state = install_package(path, &scanner_package()).unwrap();
    state
        .plugins
        .iter()
        .find(|app| app.manifest.id == SCANNER)
        .unwrap()
        .revision
}

#[test]
fn an_executable_app_installs_only_from_a_package_that_carries_its_binaries() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("extensions.json");
    configure(
        &path,
        json!({"action": "unsignedApps", "allowUnsignedApps": true}),
    )
    .unwrap();
    // A pasted manifest has nowhere to carry a binary.
    let pasted = configure(
        &path,
        json!({"action": "install", "manifest": scanner_manifest().to_string(), "grants": []}),
    )
    .err()
    .unwrap();
    assert!(pasted.contains("installed from a package"), "{pasted}");
    // A package missing a binary its sidecar names.
    let partial = package_of(
        &scanner_manifest(),
        &[("bin/linux-amd64/scanner", b"#!/bin/false\n")],
    );
    let missing = install_package(&path, &partial).err().unwrap();
    assert!(
        missing.contains("The package carries no bin/darwin-arm64/scanner"),
        "{missing}"
    );
    assert!(read(&path).unwrap().plugins.is_empty());

    let state = install_package(&path, &scanner_package()).unwrap();
    let app = state
        .plugins
        .iter()
        .find(|app| app.manifest.id == SCANNER)
        .unwrap();
    let digest = app.package.clone().unwrap();
    let packages = path.with_extension("packages");
    let binary =
        package::installed_binary(&packages, SCANNER, &digest, "bin/linux-amd64/scanner").unwrap();
    // Owner-only, and runnable by its owner: nothing else in a package is.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&binary).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let manifest = binary
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(package::MANIFEST);
        assert_eq!(
            fs::metadata(manifest).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    // A binary changed since the install is not run.
    fs::write(&binary, b"#!/bin/sh\necho changed\n").unwrap();
    let changed = package::installed_binary(&packages, SCANNER, &digest, "bin/linux-amd64/scanner")
        .unwrap_err();
    assert!(
        changed.contains("no longer matches its digest"),
        "{changed}"
    );
    let unnamed = package::installed_binary(&packages, SCANNER, &digest, "bin/linux-amd64/other")
        .unwrap_err();
    assert!(
        unnamed.contains("carries no bin/linux-amd64/other"),
        "{unnamed}"
    );
}

#[tokio::test]
async fn an_unsigned_executable_app_needs_the_unsigned_apps_setting_even_without_writes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("extensions.json");
    let refused = install_package(&path, &scanner_package()).err().unwrap();
    assert_eq!(refused, UNSIGNED_POLICY_REASON);
    // The review says so first, at the kind.
    let mut reg = Registry::new();
    register(
        &mut reg,
        path.clone(),
        fake_core(),
        srelens_kube::client_cache::ClientCache::new_many(vec![]),
    );
    let verified = package::read(&scanner_package(), &mut package::Discard).unwrap();
    let report = reg
        .invoke(
            "extensions.validate",
            json!({"manifest": verified.manifest, "grants": [], "digests": verified.digests}),
        )
        .await
        .unwrap();
    assert_eq!(report["errors"][0]["path"], "kind", "{report}");
    assert_eq!(report["errors"][0]["message"], UNSIGNED_POLICY_REASON);
    // Turning the setting off again disables it, as it does a writing app.
    configure(
        &path,
        json!({"action": "unsignedApps", "allowUnsignedApps": true}),
    )
    .unwrap();
    install_package(&path, &scanner_package()).unwrap();
    let state = configure(
        &path,
        json!({"action": "unsignedApps", "allowUnsignedApps": false}),
    )
    .unwrap();
    let app = state
        .plugins
        .iter()
        .find(|app| app.manifest.id == SCANNER)
        .unwrap();
    assert!(!app.enabled);
    assert_eq!(app.policy_blocked.as_deref(), Some(UNSIGNED_POLICY_REASON));
}

#[test]
fn a_stored_executable_app_without_its_package_is_quarantined() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("extensions.json");
    install_scanner(&path);
    let mut stored: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let app = stored["plugins"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|app| app["manifest"]["id"] == SCANNER)
        .unwrap();
    app.as_object_mut().unwrap().remove("package");
    fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();
    let state = read(&path).unwrap();
    let app = state
        .plugins
        .iter()
        .find(|app| app.manifest.id == SCANNER)
        .unwrap();
    assert!(!app.enabled);
    assert!(
        app.quarantined
            .as_deref()
            .is_some_and(|why| why.contains("no package to run its sidecar from")),
        "{:?}",
        app.quarantined
    );
}

#[test]
fn a_host_that_keeps_no_app_files_installs_no_executable_app() {
    // The web host (#515): no package directory, so no binary to run.
    let dir = tempfile::tempdir().unwrap();
    let apps = Apps::with_shared_catalog(
        Arc::new(dir.path().join("inventory.json")),
        SharedCatalog::new(dir.path().join("catalog.json")),
    );
    let refused = configure_apps(
        &apps,
        json!({"action": "installPackage",
            "package": base64::engine::general_purpose::STANDARD.encode(scanner_package()),
            "grants": []}),
    )
    .err()
    .unwrap();
    assert_eq!(refused, NO_PACKAGES);
}

fn configure_apps(apps: &Apps, input: Value) -> Result<Inventory, String> {
    let input = serde_json::from_value::<Configure>(input).map_err(|e| e.to_string())?;
    super::configure(
        apps,
        fake_core(),
        &srelens_plugin_host::NoSecretStore,
        input,
    )
}
