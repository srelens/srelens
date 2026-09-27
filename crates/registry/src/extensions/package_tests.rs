//! Installing, updating, verifying and removing apps that come as packages (#562).
use super::package::tests::{fixture, packed, Raw};
use super::tests::{configure, fake_core};
use super::*;
use base64::Engine as _;

const GRANTS: [&str; 1] = ["k8s.listCustomResource"];

fn encode(archive: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(archive)
}

/// `extensions.configure`'s `installPackage`, as the `@srelens/core` wrapper sends it.
fn install_package(path: &Path, archive: &[u8]) -> Result<Inventory, String> {
    let id = package::read(archive, &mut package::Discard)
        .map(|verified| verified.list.id)
        .unwrap_or_default();
    let reviewed = read(path)?
        .plugins
        .iter()
        .find(|app| app.manifest.id == id)
        .map(|app| app.revision);
    configure(
        path,
        json!({"action": "installPackage", "package": encode(archive), "grants": GRANTS,
            "reviewedRevision": reviewed}),
    )
}

/// The directory an installed version of `id` was unpacked into.
fn unpacked(path: &Path, id: &str, digest: &str) -> PathBuf {
    path.with_extension("packages").join(id).join(digest)
}

fn app<'a>(state: &'a Inventory, id: &str) -> &'a Installed {
    state
        .plugins
        .iter()
        .find(|app| app.manifest.id == id)
        .unwrap_or_else(|| panic!("{id} is not installed"))
}

/// The `example` fixture at another version, packed from a copy.
fn example_at(version: &str) -> Vec<u8> {
    let dir = tempfile::tempdir().unwrap();
    for (path, source) in fixture_files("example") {
        let target = dir.path().join(&path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        let mut data = fs::read(source).unwrap();
        if path == package::MANIFEST {
            data = String::from_utf8(data)
                .unwrap()
                .replace("\"1.0.0\"", &format!("\"{version}\""))
                .into_bytes();
        }
        fs::write(target, data).unwrap();
    }
    fs::write(
        dir.path().join(package::DIGESTS),
        package::digest_list(dir.path()).unwrap(),
    )
    .unwrap();
    package::pack(dir.path()).unwrap()
}

fn fixture_files(name: &str) -> Vec<(String, PathBuf)> {
    let root = fixture(name);
    let mut files = Vec::new();
    let mut pending = vec![root.clone()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                pending.push(entry.path());
            } else {
                let path = entry.path();
                let relative = path
                    .strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                files.push((relative, path));
            }
        }
    }
    files
}

#[tokio::test]
async fn a_package_installs_into_its_private_directory_and_lists_its_logo() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("extensions.json");
    let archive = packed("example");
    let verified = package::read(&archive, &mut package::Discard).unwrap();
    let state = install_package(&path, &archive).unwrap();
    let installed = app(&state, "org.example.packaged");
    assert!(installed.enabled);
    assert!(installed.signature_proof.is_none());
    assert_eq!(installed.package.as_deref(), Some(verified.digest.as_str()));
    assert_eq!(installed.source, Source::Local);
    assert_eq!(installed.grants, GRANTS);
    let version = unpacked(&path, "org.example.packaged", &verified.digest);
    assert_eq!(
        fs::read(version.join("icons/icon.svg")).unwrap(),
        fs::read(fixture("example").join("icons/icon.svg")).unwrap()
    );

    // The list shows the logo, read from the package; the inventory file never holds it.
    let mut reg = Registry::new();
    register(
        &mut reg,
        path.clone(),
        fake_core(),
        srelens_kube::client_cache::ClientCache::new_many(vec![]),
    );
    let listed = reg.invoke("extensions.list", json!({})).await.unwrap();
    let icon = listed["plugins"][0]["icon"].as_str().unwrap();
    assert!(icon.starts_with("data:image/svg+xml;base64,"), "{icon}");
    let saved = fs::read_to_string(&path).unwrap();
    assert!(
        !saved.contains("\"icon\"") && !saved.contains("data:image"),
        "{saved}"
    );
    // A logo planted in the file is ignored when it is read.
    let mut planted: Value = serde_json::from_str(&saved).unwrap();
    planted["plugins"][0]["icon"] = json!("data:image/png;base64,AAAA");
    fs::write(&path, planted.to_string()).unwrap();
    assert_eq!(read(&path).unwrap().plugins[0].icon, None);
    // A logo changed on disk is not shown; the app still is.
    fs::write(version.join("icons/icon.svg"), b"<svg>changed</svg>").unwrap();
    let listed = reg.invoke("extensions.list", json!({})).await.unwrap();
    assert_eq!(
        listed["plugins"][0]["manifest"]["id"],
        "org.example.packaged"
    );
    assert!(listed["plugins"][0].get("icon").is_none(), "{listed}");
}

#[test]
fn a_signed_package_keeps_its_proof_and_is_verified_on_every_load() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("extensions.json");
    let archive = packed("signed");
    let verified = package::read(&archive, &mut package::Discard).unwrap();
    install_package(&path, &archive).unwrap();
    let state = read(&path).unwrap();
    let installed = app(&state, "test.signed.packaged");
    let proof = installed.signature_proof.as_ref().unwrap();
    assert_eq!(proof.digests.as_deref(), Some(verified.digests.as_str()));
    assert_eq!(proof.manifest, verified.manifest);
    assert!(installed.quarantined.is_none() && installed.enabled);

    let saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let quarantined = |edit: &dyn Fn(&mut Value)| {
        let mut stored = saved.clone();
        edit(&mut stored["plugins"][0]);
        fs::write(&path, stored.to_string()).unwrap();
        let state = read(&path).unwrap();
        let app = &state.plugins[0];
        assert!(!app.enabled);
        app.quarantined.clone().expect("quarantined")
    };
    // A list other than the one that was signed.
    let reason = quarantined(&|app| {
        let digests = app["signatureProof"]["digests"]
            .as_str()
            .unwrap()
            .replace("1.0.0", "1.0.1");
        app["signatureProof"]["digests"] = json!(digests);
    });
    assert!(
        reason.contains("signature is invalid") || reason.contains("does not name"),
        "{reason}"
    );
    // The signed list, but for another unpacked version.
    let reason = quarantined(&|app| app["package"] = json!("0".repeat(64)));
    assert!(
        reason.contains("does not match its signed package"),
        "{reason}"
    );
    // A signed package's proof with the package forgotten.
    let reason = quarantined(&|app| {
        app.as_object_mut().unwrap().remove("package");
    });
    assert!(
        reason.contains("does not match its signed package"),
        "{reason}"
    );
    // A single-file proof cannot name a package either.
    let reason = quarantined(&|app| {
        app["signatureProof"]
            .as_object_mut()
            .unwrap()
            .remove("digests");
    });
    assert!(!reason.is_empty());
    // Not a SHA-256 at all.
    let reason = quarantined(&|app| app["package"] = json!("../../elsewhere"));
    assert!(reason.contains("SHA-256"), "{reason}");
}

#[test]
fn an_update_keeps_the_replaced_package_for_rollback_and_prunes_what_nothing_keeps() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("extensions.json");
    let first = package::read(&packed("example"), &mut package::Discard).unwrap();
    install_package(&path, &packed("example")).unwrap();
    let second_archive = example_at("1.1.0");
    let second = package::read(&second_archive, &mut package::Discard).unwrap();
    let state = install_package(&path, &second_archive).unwrap();
    let installed = app(&state, "org.example.packaged");
    assert_eq!(installed.manifest.version, "1.1.0");
    assert_eq!(installed.package.as_deref(), Some(second.digest.as_str()));
    assert_eq!(
        installed.history[0].package.as_deref(),
        Some(first.digest.as_str())
    );
    assert!(unpacked(&path, "org.example.packaged", &first.digest).is_dir());
    assert!(unpacked(&path, "org.example.packaged", &second.digest).is_dir());

    // Rolling back restores the version's package, and drops the one rolled away from.
    let state = configure(
        &path,
        json!({"action": "rollback", "id": "org.example.packaged",
            "revision": installed.history[0].revision, "grants": GRANTS}),
    )
    .unwrap();
    let installed = app(&state, "org.example.packaged");
    assert_eq!(installed.manifest.version, "1.0.0");
    assert_eq!(installed.package.as_deref(), Some(first.digest.as_str()));
    assert!(unpacked(&path, "org.example.packaged", &first.digest).is_dir());
    assert!(!unpacked(&path, "org.example.packaged", &second.digest).exists());

    // A single-file manifest replacing it keeps the package for rollback.
    let source = fs::read_to_string(fixture("example").join(package::MANIFEST))
        .unwrap()
        .replace("\"1.0.0\"", "\"2.0.0\"");
    let state = configure(
        &path,
        json!({"action": "install", "manifest": source, "grants": GRANTS}),
    )
    .unwrap();
    let installed = app(&state, "org.example.packaged");
    assert_eq!(installed.package, None);
    assert_eq!(
        installed.history[0].package.as_deref(),
        Some(first.digest.as_str())
    );
    assert!(unpacked(&path, "org.example.packaged", &first.digest).is_dir());

    // Removed, nothing of it is left.
    configure(
        &path,
        json!({"action": "remove", "id": "org.example.packaged"}),
    )
    .unwrap();
    assert_eq!(
        fs::read_dir(path.with_extension("packages"))
            .unwrap()
            .count(),
        0
    );
}

/// The inventory's save comes after the unpack, and can fail. Every version the saved
/// inventory names is still on disk when it does: `unpack` never touches another version's
/// directory, and replaces a copy of the version it installs only with a copy verified
/// against the same digest list. What a failed update unpacked is pruned by the next save
/// that succeeds.
#[cfg(unix)]
#[test]
fn a_failed_inventory_save_leaves_every_version_it_names_in_place() {
    use std::os::unix::fs::PermissionsExt;
    // SAFETY: geteuid has no preconditions.
    if unsafe { libc::geteuid() } == 0 {
        eprintln!("skipped: root writes into any directory regardless of its mode");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let apps = dir.path().join("apps");
    fs::create_dir(&apps).unwrap();
    let path = apps.join("extensions.json");
    let id = "org.example.packaged";
    let first = package::read(&packed("example"), &mut package::Discard)
        .unwrap()
        .digest;
    let update = example_at("1.1.0");
    let second = package::read(&update, &mut package::Discard)
        .unwrap()
        .digest;
    install_package(&path, &packed("example")).unwrap();
    let saved = fs::read(&path).unwrap();

    // The inventory's directory refuses new files, so the save's temporary file cannot be
    // made. The packages directory beside it, and the lock file that exists already, still
    // work, so each install gets as far as its unpack.
    fs::set_permissions(&apps, fs::Permissions::from_mode(0o500)).unwrap();
    let updated = install_package(&path, &update).err();
    let reinstalled = install_package(&path, &packed("example")).err();
    fs::set_permissions(&apps, fs::Permissions::from_mode(0o700)).unwrap();
    for refused in [updated, reinstalled] {
        let refused = refused.expect("the save was refused");
        assert!(refused.contains("save extension inventory"), "{refused}");
    }

    assert_eq!(fs::read(&path).unwrap(), saved);
    assert_eq!(
        read(&path).unwrap().plugins[0].package.as_deref(),
        Some(first.as_str())
    );
    // The version it names is whole: its logo verifies against its digest list.
    assert!(
        package::installed_icon(&path.with_extension("packages"), id, &first)
            .unwrap()
            .is_some()
    );
    // What the failed update unpacked goes with the next change that saves.
    assert!(unpacked(&path, id, &second).is_dir());
    configure(
        &path,
        json!({"action": "enable", "id": id, "enabled": true}),
    )
    .unwrap();
    assert!(!unpacked(&path, id, &second).exists());
    assert!(unpacked(&path, id, &first).is_dir());
}

#[test]
fn a_package_is_refused_whole_and_leaves_nothing_behind() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("extensions.json");
    let packages = path.with_extension("packages");
    let nothing_installed = |reason: String, expected: &str| {
        assert!(
            reason.contains(expected),
            "expected {expected:?}, got {reason:?}"
        );
        assert!(read(&path).unwrap().plugins.is_empty());
        assert!(!packages.exists() || fs::read_dir(&packages).unwrap().count() == 0);
    };
    // A tampered file, and a path that leaves the package.
    let tampered = {
        let mut tar = Vec::new();
        use std::io::Read as _;
        flate2::read::GzDecoder::new(&packed("example")[..])
            .read_to_end(&mut tar)
            .unwrap();
        let at = tar.windows(9).position(|w| w == b"# Package").unwrap();
        tar[at + 2] = b'p';
        super::package::tests::gzip(&tar)
    };
    nothing_installed(
        install_package(&path, &tampered).err().unwrap(),
        "does not match its digest",
    );
    let traversal = Raw::new().file("../../escape.json", b"{}").gz();
    nothing_installed(
        install_package(&path, &traversal).err().unwrap(),
        "leaves the package",
    );
    // Binaries a declarative app does not run.
    let with_binary = {
        let dir = tempfile::tempdir().unwrap();
        for (path, source) in fixture_files("example") {
            let target = dir.path().join(&path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::copy(source, target).unwrap();
        }
        fs::create_dir_all(dir.path().join("bin/linux-amd64")).unwrap();
        fs::write(dir.path().join("bin/linux-amd64/tool"), b"\x7fELF").unwrap();
        fs::write(
            dir.path().join(package::DIGESTS),
            package::digest_list(dir.path()).unwrap(),
        )
        .unwrap();
        package::pack(dir.path()).unwrap()
    };
    nothing_installed(
        install_package(&path, &with_binary).err().unwrap(),
        "its manifest runs none",
    );
    // Grants that are not what the manifest asks for: refused before anything is unpacked.
    let reason = configure(
        &path,
        json!({"action": "installPackage", "package": encode(&packed("example")), "grants": []}),
    )
    .err()
    .unwrap();
    nothing_installed(reason, "permission");
    // An update that was not the one reviewed.
    install_package(&path, &packed("example")).unwrap();
    let reason = configure(
        &path,
        json!({"action": "installPackage", "package": encode(&example_at("1.2.0")), "grants": GRANTS}),
    )
    .err().unwrap();
    assert!(reason.contains("review this update again"), "{reason}");
    let app_directory = packages.join("org.example.packaged");
    assert_eq!(fs::read_dir(app_directory).unwrap().count(), 1);
}

#[test]
fn an_unsigned_package_cannot_take_a_reserved_id() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("extensions.json");
    // The signed fixture without its signature: the ID is the test publisher's.
    let files: Vec<(String, PathBuf)> = fixture_files("signed")
        .into_iter()
        .filter(|(path, _)| path != package::SIGNATURE)
        .collect();
    let unsigned = files
        .iter()
        .fold(Raw::new(), |raw, (path, source)| {
            raw.file(path, &fs::read(source).unwrap())
        })
        .gz();
    let reason = install_package(&path, &unsigned).err().unwrap();
    assert!(reason.contains("reserved"), "{reason}");
}

#[tokio::test]
async fn a_host_that_keeps_no_app_files_refuses_packages_but_can_review_them() {
    let dir = tempfile::tempdir().unwrap();
    let apps = Apps::with_shared_catalog(
        Arc::new(dir.path().join("inventory.json")),
        SharedCatalog::new(dir.path().join("catalog.json")),
    );
    let mut reg = Registry::new();
    register(
        &mut reg,
        apps,
        fake_core(),
        srelens_kube::client_cache::ClientCache::new_many(vec![]),
    );
    let archive = encode(&packed("example"));
    let review = reg
        .invoke("extensions.packageManifest", json!({"package": archive}))
        .await
        .unwrap();
    assert_eq!(review["package"]["files"].as_array().unwrap().len(), 5);
    let refused = reg
        .invoke(
            "extensions.configure",
            json!({"action": "installPackage", "package": archive, "grants": GRANTS}),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(refused.contains("keeps no files for its apps"), "{refused}");
    let refused = reg
        .invoke(
            "extensions.configure",
            json!({"action": "installCatalogPackage", "id": "org.example.packaged",
                "sha256": "0".repeat(64), "packageSha256": "0".repeat(64), "grants": GRANTS}),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(refused.contains("keeps no files for its apps"), "{refused}");
    assert!(!dir.path().join("inventory.json").exists());
}

/// A package's review, then its check, as the Settings screen makes them: the manifest,
/// signature and digest list `extensions.packageManifest` returns go to
/// `extensions.validate` unchanged.
#[tokio::test]
async fn a_package_is_reviewed_and_validated_as_installing_it_would_be() {
    let dir = tempfile::tempdir().unwrap();
    let mut reg = Registry::new();
    register(
        &mut reg,
        dir.path().join("extensions.json"),
        fake_core(),
        srelens_kube::client_cache::ClientCache::new_many(vec![]),
    );
    for name in ["example", "signed"] {
        let review = reg
            .invoke(
                "extensions.packageManifest",
                json!({"package": encode(&packed(name))}),
            )
            .await
            .unwrap();
        assert_eq!(review["signature"].is_array(), name == "signed", "{review}");
        let checked = reg
            .invoke(
                "extensions.validate",
                json!({"manifest": review["manifest"], "grants": GRANTS,
                    "signature": review["signature"], "digests": review["package"]["digests"]}),
            )
            .await
            .unwrap();
        assert_eq!(checked["errors"], json!([]), "{name}: {checked}");
    }
    let review = reg
        .invoke(
            "extensions.packageManifest",
            json!({"package": encode(&packed("signed"))}),
        )
        .await
        .unwrap();
    let problems = |input: Value| {
        let reg = &reg;
        async move {
            let checked = reg.invoke("extensions.validate", input).await.unwrap();
            checked["errors"]
                .as_array()
                .unwrap()
                .iter()
                .map(|error| error["code"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        }
    };
    // The package's signature is over its list, so without the list it does not verify.
    let codes = problems(json!({"manifest": review["manifest"], "grants": GRANTS,
        "signature": review["signature"]}))
    .await;
    assert!(
        codes.contains(&"EXTENSION_INVALID_SIGNATURE".to_owned()),
        "{codes:?}"
    );
    // A list that names another manifest.
    let example = reg
        .invoke(
            "extensions.packageManifest",
            json!({"package": encode(&packed("example"))}),
        )
        .await
        .unwrap();
    let codes = problems(json!({"manifest": review["manifest"], "grants": GRANTS,
        "signature": review["signature"], "digests": example["package"]["digests"]}))
    .await;
    assert!(
        codes.contains(&"EXTENSION_INVALID_SIGNATURE".to_owned()),
        "{codes:?}"
    );
    // Refused in review as at install.
    let refused = reg
        .invoke(
            "extensions.packageManifest",
            json!({"package": encode(&Raw::new().file("/etc/passwd", b"").gz())}),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(refused.contains("is absolute"), "{refused}");
}

/// The payloads the `@srelens/core` wrappers send are the contract (AGENTS.md): camelCase,
/// the package as base64 text, and nothing the host does not know.
#[test]
fn the_wrapper_payloads_are_accepted_and_other_spellings_refused() {
    let package = encode(b"\x1f\x8b");
    for accepted in [
        json!({"action": "installPackage", "package": package, "grants": GRANTS, "reviewedRevision": 3}),
        json!({"action": "installPackage", "package": package, "grants": GRANTS}),
        json!({"action": "installCatalogPackage", "id": "org.example.app", "sha256": "a",
            "packageSha256": "b", "grants": GRANTS, "reviewedRevision": 3}),
    ] {
        serde_json::from_value::<Configure>(accepted.clone())
            .unwrap_or_else(|e| panic!("{accepted}: {e}"));
    }
    for refused in [
        json!({"action": "installPackage", "package": package, "grants": GRANTS, "reviewed_revision": 3}),
        json!({"action": "installPackage", "package": [31, 139], "grants": GRANTS}),
        json!({"action": "installPackage", "package": package, "grants": GRANTS, "signature": [1]}),
        json!({"action": "installCatalogPackage", "id": "org.example.app", "sha256": "a",
            "package_sha256": "b", "grants": GRANTS}),
    ] {
        assert!(
            serde_json::from_value::<Configure>(refused.clone()).is_err(),
            "{refused}"
        );
    }
    assert!(serde_json::from_value::<ValidateIn>(json!({
        "manifest": "{}", "grants": [], "digests": "{}"
    }))
    .is_ok());
    assert!(serde_json::from_value::<PackageIn>(json!({"package": package})).is_ok());
    assert!(serde_json::from_value::<PackageIn>(json!({"file": package})).is_err());
}

#[test]
fn a_package_the_cached_catalog_lists_is_recorded_as_from_the_catalog() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("extensions.json");
    let archive = packed("example");
    let manifest = fs::read(fixture("example").join(package::MANIFEST)).unwrap();
    let mut catalog: Value = serde_json::from_slice(include_bytes!(
        "../../tests/fixtures/extension-catalog.json"
    ))
    .unwrap();
    let entry = &mut catalog["extensions"][0];
    entry["id"] = json!("org.example.packaged");
    entry["repository"] = json!("https://github.com/example/packaged");
    entry["release"]["version"] = json!("1.0.0");
    entry["release"]["srelensApiVersion"] = json!("^0.4");
    entry["release"]["manifestUrl"] =
        json!("https://github.com/example/packaged/releases/download/v1.0.0/manifest.json");
    entry["release"]["sha256"] = json!(package::sha256_hex(&manifest));
    entry["release"]["package"] = json!({
        "url": "https://github.com/example/packaged/releases/download/v1.0.0/packaged.srelens-extension",
        "sha256": package::sha256_hex(&archive),
    });
    fs::write(
        path.with_extension("catalog.json"),
        serde_json::to_vec(&json!({
            "catalog": catalog, "fetchedAt": 0, "stale": false, "error": null, "incompatible": []
        }))
        .unwrap(),
    )
    .unwrap();
    let state = install_package(&path, &archive).unwrap();
    assert_eq!(app(&state, "org.example.packaged").source, Source::Catalog);
    // Other bytes of the same app are local.
    let state = install_package(&path, &example_at("1.0.1")).unwrap();
    assert_eq!(app(&state, "org.example.packaged").source, Source::Local);
}
