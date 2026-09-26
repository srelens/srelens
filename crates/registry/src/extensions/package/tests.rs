//! The package format's own tests: what `read` accepts and refuses, what `unpack` leaves
//! on disk, and the fixtures the rest of the suite installs.
use super::*;
use crate::extensions::signing::tests::test_publisher_sign;
use serde_json::{json, Value};
use std::io::Write as _;

/// The source directory of a fixture package: `example` (unsigned, `org.example.packaged`)
/// or `signed` (signed by the test publisher, `test.signed.packaged`).
pub(in crate::extensions) fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/packages")
        .join(name)
}

/// Who may sign what in these tests: the delegations the test root ships, the test
/// publisher's `test.signed` among them (#559).
pub(in crate::extensions) fn shipped() -> Delegations {
    crate::extensions::TrustRoot::pinned().shipped()
}

/// A fixture package, packed.
pub(in crate::extensions) fn packed(name: &str) -> Vec<u8> {
    pack(&fixture(name)).unwrap()
}

/// Every file of a fixture package, `digests.json` and its signature included.
fn files_of(name: &str) -> BTreeMap<String, Vec<u8>> {
    files_under(&fixture(name))
        .unwrap()
        .into_iter()
        .map(|(path, source)| (path, fs::read(source).unwrap()))
        .collect()
}

pub(in crate::extensions) fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

/// A tar archive written header by header, so a test can write what `tar::Builder`
/// refuses to: a `..` path, an absolute path, a link.
pub(in crate::extensions) struct Raw(tar::Builder<Vec<u8>>);

impl Raw {
    pub(in crate::extensions) fn new() -> Self {
        Self(tar::Builder::new(Vec::new()))
    }

    fn entry_with(mut self, kind: tar::EntryType, path: &[u8], data: &[u8], link: &[u8]) -> Self {
        let mut header = tar::Header::new_ustar();
        let ustar = header.as_ustar_mut().unwrap();
        ustar.name[..path.len()].copy_from_slice(path);
        ustar.linkname[..link.len()].copy_from_slice(link);
        header.set_size(data.len() as u64);
        header.set_entry_type(kind);
        header.set_mode(0o644);
        header.set_cksum();
        self.0.append(&header, data).unwrap();
        self
    }

    pub(in crate::extensions) fn entry(
        self,
        kind: tar::EntryType,
        path: &[u8],
        data: &[u8],
    ) -> Self {
        self.entry_with(kind, path, data, b"")
    }

    pub(in crate::extensions) fn file(self, path: &str, data: &[u8]) -> Self {
        self.entry(tar::EntryType::Regular, path.as_bytes(), data)
    }

    fn files(self, files: &BTreeMap<String, Vec<u8>>) -> Self {
        files
            .iter()
            .fold(self, |raw, (path, data)| raw.file(path, data))
    }

    pub(in crate::extensions) fn tar(self) -> Vec<u8> {
        self.0.into_inner().unwrap()
    }

    pub(in crate::extensions) fn gz(self) -> Vec<u8> {
        gzip(&self.tar())
    }
}

/// The digest list for `files` (which must not hold one), for `id` at `version`.
fn digests_for(files: &BTreeMap<String, Vec<u8>>, id: &str, version: &str) -> Vec<u8> {
    let files: Vec<Value> = files
        .iter()
        .filter(|(path, _)| path.as_str() != DIGESTS && path.as_str() != SIGNATURE)
        .map(|(path, data)| json!({"path": path, "size": data.len(), "sha256": sha256_hex(data)}))
        .collect();
    serde_json::to_vec_pretty(&json!({
        "format": FORMAT, "formatVersion": FORMAT_VERSION, "id": id, "version": version, "files": files,
    }))
    .unwrap()
}

fn refused(archive: &[u8]) -> String {
    match read(archive, &mut Discard, &shipped()) {
        Ok(package) => panic!("accepted a package for {}", package.list.id),
        Err(reason) => reason,
    }
}

fn assert_refused(archive: &[u8], expected: &str) {
    let reason = refused(archive);
    assert!(
        reason.contains(expected),
        "expected {expected:?}, got {reason:?}"
    );
}

/// The fixtures' digest lists describe their files, and the signed one carries the test
/// publisher's signature over its list. `UPDATE_CATALOG=1 cargo test -p srelens-registry`
/// makes them again after a fixture file changes.
#[test]
fn fixture_digest_lists_are_current() {
    for name in ["example", "signed"] {
        let dir = fixture(name);
        let want = digest_list(&dir).unwrap();
        if std::env::var("UPDATE_CATALOG").is_ok() {
            fs::write(dir.join(DIGESTS), &want).unwrap();
            if name == "signed" {
                fs::write(dir.join(SIGNATURE), test_publisher_sign(&want)).unwrap();
            }
            continue;
        }
        assert_eq!(
            fs::read(dir.join(DIGESTS)).unwrap(),
            want,
            "{name}/digests.json is stale — run UPDATE_CATALOG=1 cargo test -p srelens-registry"
        );
    }
    let signed = fixture("signed");
    let digests = fs::read_to_string(signed.join(DIGESTS)).unwrap();
    let manifest = fs::read_to_string(signed.join(MANIFEST)).unwrap();
    verify_signed(
        &digests,
        &fs::read(signed.join(SIGNATURE)).unwrap(),
        &manifest,
        &shipped(),
    )
    .unwrap();
}

#[test]
fn a_valid_package_is_read_whole() {
    let archive = packed("example");
    let package = read(&archive, &mut Discard, &shipped()).unwrap();
    assert_eq!(package.list.id, "org.example.packaged");
    assert_eq!(package.list.version, "1.0.0");
    assert_eq!(package.signature, None);
    assert_eq!(package.sha256, sha256_hex(&archive));
    assert_eq!(package.digest, sha256_hex(package.digests.as_bytes()));
    assert_eq!(
        package.manifest,
        fs::read_to_string(fixture("example").join(MANIFEST)).unwrap()
    );
    let review = serde_json::to_value(package.review()).unwrap();
    let paths: Vec<&str> = review["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["path"].as_str().unwrap())
        .collect();
    assert_eq!(
        paths,
        [
            "LICENSE",
            "README.md",
            "extension.json",
            "icons/icon.svg",
            "schemas/settings.schema.json"
        ]
    );
    let icon = review["icon"].as_str().unwrap();
    assert!(icon.starts_with("data:image/svg+xml;base64,"), "{icon}");
    assert!(!package.carries_binaries());
    check_installable(&package).unwrap();
    // The same directory always packs to the same archive.
    assert_eq!(packed("example"), archive);
}

#[test]
fn a_signed_package_carries_its_publishers_signature_over_the_digest_list() {
    let package = read(&packed("signed"), &mut Discard, &shipped()).unwrap();
    assert_eq!(package.list.id, "test.signed.packaged");
    let signature = package.signature.as_deref().unwrap();
    verify_signed(&package.digests, signature, &package.manifest, &shipped()).unwrap();
    // The signature is over the list, not the manifest.
    assert!(signing::verify_for(
        &package.list.id,
        package.manifest.as_bytes(),
        signature,
        None,
        &shipped()
    )
    .is_err());
    let icon = package.review().icon.unwrap();
    assert!(icon.starts_with("data:image/png;base64,"), "{icon}");
}

#[test]
fn a_tampered_file_is_refused() {
    let mut files = files_of("example");
    files.insert("README.md".into(), b"# Something else\n".to_vec());
    assert_refused(
        &Raw::new().files(&files).gz(),
        "README.md does not match its digest",
    );
    // In a signed package too: the signature holds, the file does not.
    let mut files = files_of("signed");
    files.get_mut("icons/icon.png").unwrap().push(0);
    assert_refused(
        &Raw::new().files(&files).gz(),
        "icons/icon.png does not match its digest",
    );
}

#[test]
fn a_missing_file_is_refused() {
    let mut files = files_of("example");
    files.remove("LICENSE");
    assert_refused(
        &Raw::new().files(&files).gz(),
        "LICENSE is named in the digest list but missing",
    );
    let mut files = files_of("example");
    files.remove(DIGESTS);
    assert_refused(&Raw::new().files(&files).gz(), "has no digests.json");
}

#[test]
fn an_extra_file_is_refused() {
    let mut files = files_of("example");
    files.insert("schemas/extra.json".into(), b"{}".to_vec());
    assert_refused(
        &Raw::new().files(&files).gz(),
        "holds schemas/extra.json, which its digest list does not name",
    );
}

#[test]
fn a_digest_list_changed_after_signing_is_refused() {
    // The file and the list agree, but the list is not the one the publisher signed.
    let mut files = files_of("signed");
    files.get_mut("icons/icon.png").unwrap().push(0);
    files.insert(
        DIGESTS.into(),
        digests_for(&files, "test.signed.packaged", "1.0.0"),
    );
    assert_refused(&Raw::new().files(&files).gz(), "signature is invalid");
    // A signature under a key that does not speak for the app.
    let mut files = files_of("example");
    let digests = files[DIGESTS].clone();
    files.insert(SIGNATURE.into(), test_publisher_sign(&digests));
    assert_refused(
        &Raw::new().files(&files).gz(),
        "No publisher is trusted to sign",
    );
    // Not an Ed25519 signature at all.
    let mut files = files_of("signed");
    files.insert(SIGNATURE.into(), vec![0; 12]);
    assert_refused(&Raw::new().files(&files).gz(), "64-byte Ed25519 signature");
}

/// A package signer is held to its publisher's namespaces as a single-file release's is
/// (#559): the test publisher, delegated `test.signed`, cannot sign a package that names an
/// app of srelens's, and the signature naming its key does not get it past that.
#[test]
fn a_package_signed_outside_its_publishers_namespace_is_refused() {
    let mut files = files_of("signed");
    let manifest = String::from_utf8(files[MANIFEST].clone())
        .unwrap()
        .replace("test.signed.packaged", "org.srelens.packaged");
    files.insert(MANIFEST.into(), manifest.into_bytes());
    let digests = digests_for(&files, "org.srelens.packaged", "1.0.0");
    files.insert(SIGNATURE.into(), test_publisher_sign(&digests));
    files.insert(DIGESTS.into(), digests.clone());
    assert_refused(&Raw::new().files(&files).gz(), "signature is invalid");
    let manifest = String::from_utf8(files[MANIFEST].clone()).unwrap();
    let refused = verify_signed(
        std::str::from_utf8(&digests).unwrap(),
        &test_publisher_sign(&digests),
        &manifest,
        &shipped(),
    )
    .unwrap_err();
    assert!(refused.contains("signature is invalid"), "{refused}");
    // In its own namespace, the same key is its publisher's.
    let package = read(&packed("signed"), &mut Discard, &shipped()).unwrap();
    assert_eq!(package.signer.unwrap().name, "Test Publisher");
}

#[test]
fn links_devices_and_extended_headers_are_refused() {
    let files = files_of("example");
    for (kind, what) in [
        (tar::EntryType::Symlink, "a symbolic link"),
        (tar::EntryType::Link, "a hard link"),
        (tar::EntryType::Char, "a device or a pipe"),
        (tar::EntryType::Block, "a device or a pipe"),
        (tar::EntryType::Fifo, "a device or a pipe"),
        (tar::EntryType::XHeader, "a PAX extended header"),
        (tar::EntryType::XGlobalHeader, "a PAX extended header"),
        (tar::EntryType::GNULongName, "a GNU long name"),
        (tar::EntryType::GNUSparse, "a sparse file"),
        (tar::EntryType::Continuous, "an unsupported kind of entry"),
    ] {
        let archive = Raw::new()
            .files(&files)
            .entry_with(kind, b"icons/logo.svg", b"", b"../../../etc/passwd")
            .gz();
        assert_refused(&archive, what);
    }
    // A link placed before the files it would redirect is refused before they are read.
    let archive = Raw::new()
        .entry_with(tar::EntryType::Symlink, b"schemas", b"", b"/etc")
        .files(&files)
        .gz();
    assert_refused(&archive, "a symbolic link");
}

#[test]
fn paths_outside_the_package_or_its_layout_are_refused() {
    let files = files_of("example");
    for (path, why) in [
        (&b"../evil"[..], "leaves the package"),
        (b"icons/../../evil.svg", "leaves the package"),
        (b"/etc/passwd", "is absolute"),
        (b"./extension.json", "starts with '.'"),
        (b"icons//icon.svg", "has an empty segment"),
        (b"icons/.hidden.svg", "starts with '.'"),
        (b"icons\\..\\evil.svg", "may use only letters"),
        (b"C:/evil", "may use only letters"),
        (b"icons/logo .svg", "may use only letters"),
        (b"icons/lo\xe2\x80\xaego.svg", "may use only letters"),
        (b"icons/logo.svg.", "ends with '.'"),
        (b"icons/nul.svg", "names a Windows device"),
        (b"schemas/COM1.json", "names a Windows device"),
        (b"other.txt", "is not part of the package layout"),
        (b"readme.md", "is not part of the package layout"),
        (b"icons/logo.gif", "is not an .svg or .png image"),
        (b"schemas/settings.yaml", "is not a .json schema"),
        (
            b"bin/freebsd-amd64/tool",
            "not under bin/ for a supported platform",
        ),
        (b"bin/tool", "not under bin/ for a supported platform"),
        (b"bin/linux-amd64", "is not part of the package layout"),
        (b"docs/guide.md", "is not part of the package layout"),
        (b"a/b/c/d/e/f/g.json", "is nested more than 6 deep"),
        (b"schemas/a/b/c/d/e/f.json", "is nested more than 6 deep"),
        (b"icons/\xff.svg", "is not UTF-8"),
    ] {
        let archive = Raw::new()
            .files(&files)
            .entry(tar::EntryType::Regular, path, b"x")
            .gz();
        assert_refused(&archive, why);
    }
    for (path, why) in [
        (
            &b"extension.json/"[..],
            "a directory where the layout has none",
        ),
        (b"docs/", "a directory where the layout has none"),
        (
            b"bin/plan9-amd64/",
            "not under bin/ for a supported platform",
        ),
        (b"../", "leaves the package"),
    ] {
        let archive = Raw::new()
            .entry(tar::EntryType::Directory, path, b"")
            .files(&files)
            .gz();
        assert_refused(&archive, why);
    }
    // Directories the layout has are fine, before their files or not.
    let archive = Raw::new()
        .entry(tar::EntryType::Directory, b"icons/", b"")
        .entry(tar::EntryType::Directory, b"bin/", b"")
        .entry(tar::EntryType::Directory, b"bin/linux-arm64/", b"")
        .files(&files)
        .gz();
    read(&archive, &mut Discard, &shipped()).unwrap();
}

#[test]
fn paths_one_file_system_would_merge_are_refused() {
    let mut files = files_of("example");
    files.insert("icons/Icon.svg".into(), files["icons/icon.svg"].clone());
    files.insert(
        DIGESTS.into(),
        digests_for(&files, "org.example.packaged", "1.0.0"),
    );
    assert_refused(&Raw::new().files(&files).gz(), "differ only in case");
    // The same path twice: a later entry would replace the earlier on unpacking.
    let files = files_of("example");
    let archive = Raw::new()
        .files(&files)
        .file("icons/icon.svg", b"<svg>other</svg>")
        .gz();
    assert_refused(&archive, "icons/icon.svg\" twice");
    // A file where another path needs a directory.
    let mut files = files_of("example");
    files.insert("schemas/a.json".into(), b"{}".to_vec());
    files.insert("schemas/a.json/b.json".into(), b"{}".to_vec());
    let archive = Raw::new().files(&files).gz();
    assert_refused(&archive, "as a file and as a directory");
}

#[test]
fn oversized_packages_are_refused() {
    // Compressed, before anything is read.
    assert_refused(&vec![0; MAX_PACKAGE_BYTES + 1], "exceeds 16 MiB");
    // One file over its place's limit, refused at its header.
    let files = files_of("example");
    let archive = Raw::new()
        .files(&files)
        .file("icons/big.svg", &vec![b' '; MAX_ICON_BYTES as usize + 1])
        .gz();
    assert_refused(&archive, "icons/big.svg is larger than the package allows");
    // Files that come to more than the whole package may unpack to.
    let chunk = vec![0u8; 40 * 1024 * 1024];
    let archive = Raw::new()
        .files(&files)
        .file("bin/linux-amd64/one", &chunk)
        .file("bin/linux-amd64/two", &chunk)
        .gz();
    assert!(archive.len() < MAX_PACKAGE_BYTES);
    assert_refused(&archive, "unpacks to more than 64 MiB");
    // Zeros after the end of the archive, which compress to almost nothing: a bomb is
    // stopped at the limit rather than read to its end.
    let mut tar = Raw::new().files(&files).tar();
    tar.resize(tar.len() + MAX_STREAM_BYTES as usize, 0);
    let archive = gzip(&tar);
    assert!(archive.len() < MAX_PACKAGE_BYTES);
    assert_refused(&archive, "unpacks to more than 64 MiB");
    // More entries than the limit.
    let archive = (0..=MAX_ENTRIES)
        .fold(Raw::new(), |raw, n| {
            raw.entry(
                tar::EntryType::Directory,
                format!("schemas/d{n}/").as_bytes(),
                b"",
            )
        })
        .gz();
    assert_refused(&archive, "more than 256 entries");
}

#[test]
fn nothing_may_follow_the_archive() {
    let files = files_of("example");
    let mut tar = Raw::new().files(&files).tar();
    // Another entry after the end-of-archive blocks, which a reader told to skip zero
    // blocks would unpack.
    tar.extend(Raw::new().file("schemas/hidden.json", b"{}").tar());
    assert_refused(&gzip(&tar), "data after the end of its archive");
    // Bytes after the compressed stream, or a second one.
    let mut archive = Raw::new().files(&files).gz();
    archive.extend(b"trailing");
    assert_refused(&archive, "data after its compressed archive");
    let mut archive = Raw::new().files(&files).gz();
    archive.extend(gzip(&Raw::new().file("schemas/x.json", b"{}").tar()));
    assert_refused(&archive, "data after its compressed archive");
    // Not gzip, or a truncated one.
    assert_refused(
        &Raw::new().files(&files).tar(),
        "not a valid gzip-compressed tar",
    );
    let archive = Raw::new().files(&files).gz();
    assert!(refused(&archive[..archive.len() / 2]).starts_with("The package"));
}

#[test]
fn a_digest_list_is_read_only_in_its_exact_form() {
    let files = files_of("example");
    let list: Value = serde_json::from_slice(&files[DIGESTS]).unwrap();
    let with = |edit: &dyn Fn(&mut Value)| {
        let mut list = list.clone();
        edit(&mut list);
        serde_json::to_vec(&list).unwrap()
    };
    for (why, raw) in [
        ("not a digest list", with(&|l| l["signedBy"] = json!("me"))),
        (
            "not a digest list",
            with(&|l| l["files"][0]["mode"] = json!(420)),
        ),
        (
            "not a srelens-extension-package",
            with(&|l| l["format"] = json!("zip")),
        ),
        ("format version 2", with(&|l| l["formatVersion"] = json!(2))),
        ("not an app ID", with(&|l| l["id"] = json!("../evil"))),
        (
            "names a Windows device",
            with(&|l| l["id"] = json!("con.example")),
        ),
        ("invalid version", with(&|l| l["version"] = json!("one"))),
        (
            "once, in byte order",
            with(&|l| l["files"].as_array_mut().unwrap().reverse()),
        ),
        (
            "once, in byte order",
            with(&|l| {
                let first = l["files"][0].clone();
                l["files"].as_array_mut().unwrap().insert(0, first);
            }),
        ),
        (
            "64 lowercase hex",
            with(&|l| l["files"][0]["sha256"] = json!("ABC")),
        ),
        (
            "cannot cover",
            with(&|l| {
                l["files"].as_array_mut().unwrap().insert(
                    3,
                    json!({"path": "digests.json", "size": 1, "sha256": "0".repeat(64)}),
                );
            }),
        ),
        (
            "does not list extension.json",
            with(&|l| {
                l["files"]
                    .as_array_mut()
                    .unwrap()
                    .retain(|f| f["path"] != "extension.json");
            }),
        ),
        (
            "leaves the package",
            with(&|l| l["files"][0]["path"] = json!("../LICENSE")),
        ),
    ] {
        let reason = parse_digests(&raw).unwrap_err();
        assert!(reason.contains(why), "expected {why:?}, got {reason:?}");
    }
    // A repeated key is refused rather than read last-wins.
    let text = String::from_utf8(files[DIGESTS].clone()).unwrap();
    let repeated = text.replacen("\"id\":", "\"id\": \"org.other.app\", \"id\":", 1);
    assert!(parse_digests(repeated.as_bytes())
        .unwrap_err()
        .contains("duplicate field"));
    // A manifest is not a digest list, whoever signed it.
    assert!(parse_digests(&files[MANIFEST]).is_err());
    assert!(parse_digests(&vec![b' '; MAX_DIGESTS_BYTES + 1])
        .unwrap_err()
        .contains("exceeds 64 KiB"));
}

#[test]
fn the_manifest_must_be_the_app_and_version_the_list_names() {
    let mut files = files_of("example");
    let manifest = String::from_utf8(files[MANIFEST].clone())
        .unwrap()
        .replace("\"1.0.0\"", "\"1.0.1\"");
    files.insert(MANIFEST.into(), manifest.into_bytes());
    files.insert(
        DIGESTS.into(),
        digests_for(&files, "org.example.packaged", "1.0.0"),
    );
    assert_refused(
        &Raw::new().files(&files).gz(),
        "does not name the app and version",
    );
    files.insert(
        DIGESTS.into(),
        digests_for(&files, "org.example.other", "1.0.1"),
    );
    assert_refused(
        &Raw::new().files(&files).gz(),
        "does not name the app and version",
    );
    // A logo that is not the image its name says.
    let mut files = files_of("example");
    files.insert(
        "icons/icon.svg".into(),
        b"<html><script></script></html>".to_vec(),
    );
    files.insert(
        DIGESTS.into(),
        digests_for(&files, "org.example.packaged", "1.0.0"),
    );
    assert_refused(
        &Raw::new().files(&files).gz(),
        "not the image its name says",
    );
}

#[test]
fn binaries_are_carried_but_not_installable_here() {
    let mut files = files_of("example");
    files.insert("bin/linux-amd64/tool".into(), b"\x7fELF".to_vec());
    files.insert("bin/windows-amd64/tool.exe".into(), b"MZ".to_vec());
    files.insert(
        DIGESTS.into(),
        digests_for(&files, "org.example.packaged", "1.0.0"),
    );
    let package = read(&Raw::new().files(&files).gz(), &mut Discard, &shipped()).unwrap();
    assert!(package.carries_binaries());
    assert!(check_installable(&package)
        .unwrap_err()
        .contains("declarative apps only"));
}

fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn unpacking_puts_the_verified_files_in_the_apps_private_directory() {
    let root = tempfile::tempdir().unwrap();
    let archive = packed("example");
    let package = read(&archive, &mut Discard, &shipped()).unwrap();
    unpack(root.path(), &archive, &package, &shipped()).unwrap();
    let app = root.path().join("org.example.packaged");
    assert_eq!(entries(&app), [package.digest.clone()]);
    let version = app.join(&package.digest);
    for (path, data) in files_of("example") {
        assert_eq!(fs::read(version.join(&path)).unwrap(), data, "{path}");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode(root.path().join("org.example.packaged").as_path()),
            0o700
        );
        assert_eq!(mode(&version), 0o700);
        assert_eq!(mode(&version.join("icons")), 0o700);
        assert_eq!(mode(&version.join("icons/icon.svg")), 0o600);
        assert_eq!(mode(&version.join(MANIFEST)), 0o600);
    }
    // The logo, read back and checked against the list.
    let icon = installed_icon(root.path(), "org.example.packaged", &package.digest)
        .unwrap()
        .unwrap();
    assert_eq!(Some(icon.clone()), package.review().icon);

    // Unpacked again, the copy is replaced and nothing else is left behind.
    fs::write(version.join("README.md"), b"changed on disk").unwrap();
    unpack(root.path(), &archive, &package, &shipped()).unwrap();
    assert_eq!(entries(&app), [package.digest.clone()]);
    assert_eq!(
        fs::read(version.join("README.md")).unwrap(),
        files_of("example")["README.md"]
    );

    // A logo changed on disk is refused, not shown.
    fs::write(version.join("icons/icon.svg"), b"<svg>changed</svg>").unwrap();
    assert!(
        installed_icon(root.path(), "org.example.packaged", &package.digest)
            .unwrap_err()
            .contains("no longer matches")
    );
    fs::write(version.join(DIGESTS), b"{}").unwrap();
    assert!(
        installed_icon(root.path(), "org.example.packaged", &package.digest)
            .unwrap_err()
            .contains("digests.json no longer matches")
    );
    assert!(installed_icon(root.path(), "org.example.packaged", "not-a-digest").is_err());
}

#[test]
fn pruning_keeps_exactly_the_versions_the_inventory_names() {
    let root = tempfile::tempdir().unwrap();
    for name in ["example", "signed"] {
        let archive = packed(name);
        let package = read(&archive, &mut Discard, &shipped()).unwrap();
        unpack(root.path(), &archive, &package, &shipped()).unwrap();
    }
    let example = read(&packed("example"), &mut Discard, &shipped())
        .unwrap()
        .digest;
    let app = root.path().join("org.example.packaged");
    // An older version, and what an install that stopped part-way leaves.
    fs::create_dir(app.join("0".repeat(64))).unwrap();
    fs::create_dir(app.join(".staging-left")).unwrap();
    fs::write(root.path().join("stray"), b"").unwrap();

    let keep = BTreeMap::from([(
        "org.example.packaged".to_owned(),
        BTreeSet::from([example.clone()]),
    )]);
    prune(root.path(), &keep);
    assert_eq!(entries(root.path()), ["org.example.packaged"]);
    assert_eq!(entries(&app), [example.clone()]);

    // An app whose ID differs only in case shares the directory, not the versions.
    let keep = BTreeMap::from([
        ("org.example.packaged".to_owned(), BTreeSet::new()),
        (
            "org.Example.Packaged".to_owned(),
            BTreeSet::from([example.clone()]),
        ),
    ]);
    prune(root.path(), &keep);
    assert_eq!(entries(&app), [example]);

    prune(root.path(), &BTreeMap::new());
    assert_eq!(entries(root.path()), Vec::<String>::new());
    // A root that was never made is nothing to prune.
    prune(&root.path().join("missing"), &BTreeMap::new());
}

/// An install that stopped after moving a kept version's copy aside, before its
/// replacement was in place, leaves the version the inventory names with no directory.
/// Pruning puts that copy back instead of removing the only one.
#[test]
fn an_interrupted_reinstall_gets_its_replaced_copy_back() {
    let root = tempfile::tempdir().unwrap();
    let archive = packed("example");
    let package = read(&archive, &mut Discard, &shipped()).unwrap();
    unpack(root.path(), &archive, &package, &shipped()).unwrap();
    let app = root.path().join("org.example.packaged");
    let version = app.join(&package.digest);
    let aside = app.join(format!("{REPLACED_PREFIX}{}", package.digest));
    let keep = BTreeMap::from([(
        "org.example.packaged".to_owned(),
        BTreeSet::from([package.digest.clone()]),
    )]);

    fs::rename(&version, &aside).unwrap();
    prune(root.path(), &keep);
    assert_eq!(entries(&app), [package.digest.clone()]);
    assert_eq!(
        fs::read(version.join(MANIFEST)).unwrap(),
        files_of("example")[MANIFEST]
    );
    assert!(
        installed_icon(root.path(), "org.example.packaged", &package.digest)
            .unwrap()
            .is_some()
    );

    // With the new copy in place, the one aside is only a leftover.
    fs::create_dir(&aside).unwrap();
    prune(root.path(), &keep);
    assert_eq!(entries(&app), [package.digest.clone()]);
    // So is a copy of a version nothing keeps, or one that is not a directory.
    fs::create_dir(app.join(format!("{REPLACED_PREFIX}{}", "0".repeat(64)))).unwrap();
    fs::rename(&version, &aside).unwrap();
    fs::remove_dir_all(&aside).unwrap();
    fs::write(&aside, b"not a directory").unwrap();
    prune(root.path(), &keep);
    assert_eq!(entries(&app), Vec::<String>::new());
}

#[test]
fn packing_needs_a_digest_list_that_still_describes_the_files() {
    let dir = tempfile::tempdir().unwrap();
    for (path, data) in files_of("example") {
        let target = dir.path().join(&path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, data).unwrap();
    }
    pack(dir.path()).unwrap();
    fs::write(
        dir.path().join("README.md"),
        b"edited after the list was made",
    )
    .unwrap();
    assert!(pack(dir.path())
        .unwrap_err()
        .contains("does not describe the files"));
    fs::write(dir.path().join(".DS_Store"), b"").unwrap();
    assert!(digest_list(dir.path())
        .unwrap_err()
        .contains("starts with '.'"));
}
