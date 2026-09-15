//! A fixed public catalog; official manifests require a pinned publisher signature.
use super::*;
use sha2::{Digest, Sha256};
use srelens_plugin_host::{
    is_format_character, negotiate_api_version, MAX_MANIFEST_BYTES, SUPPORTED_API_VERSIONS,
};
use std::{
    collections::BTreeSet,
    io::Read as _,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
const CATALOG_URL: &str = "https://raw.githubusercontent.com/srelens/extensions/main/catalog.json";
pub(super) const MAX_CATALOG: usize = 1024 * 1024;
const TTL: u64 = 24 * 60 * 60;
// Catalog metadata is additive. Released hosts ignore fields they don't know, so the
// catalog can gain publishers, categories or revocations without every installed
// host rejecting it; a breaking change bumps `schemaVersion` instead.
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub(super) struct Catalog {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    extensions: Vec<Entry>,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
struct Entry {
    id: String,
    name: String,
    description: String,
    repository: String,
    license: String,
    release: Release,
    #[serde(rename = "testedHost")]
    tested_host: TestedHost,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
struct TestedHost {
    repository: String,
    revision: String,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
struct Release {
    version: String,
    #[serde(rename = "manifestUrl")]
    manifest_url: String,
    sha256: String,
    #[serde(rename = "srelensApiVersion")]
    srelens_api_version: String,
    prerelease: bool,
}
// Also the on-disk cache. Its host fields are recomputed on every load, so a cache written
// by an older host (without `hostApiVersions`) is still read rather than refetched.
#[derive(Serialize, Deserialize, JsonSchema)]
struct Snapshot {
    catalog: Catalog,
    #[serde(rename = "fetchedAt")]
    fetched_at: u64,
    stale: bool,
    error: Option<String>,
    /// Every extension API version this host supports, oldest first.
    /// The newest supported extension API version. Deprecated in favour of
    /// `host_api_versions`, and kept for API 0.1 clients until a new API line removes it.
    #[serde(rename = "hostApiVersion", default)]
    host_api_version: String,
    #[serde(rename = "hostApiVersions", default)]
    host_api_versions: Vec<String>,
    incompatible: Vec<String>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListIn {
    #[serde(default)]
    refresh: bool,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ManifestIn {
    id: String,
    sha256: String,
}
#[derive(Debug, Serialize, JsonSchema)]
struct Review {
    manifest: String,
    signature: Option<Vec<u8>>,
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn https_url(raw: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(raw).map_err(|_| "Invalid catalog URL")?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.port().is_some()
    {
        return Err(
            "Catalog URLs must use HTTPS without credentials, fragments or custom ports".into(),
        );
    }
    Ok(url)
}
fn allowed_download(url: &reqwest::Url) -> bool {
    https_url(url.as_str()).is_ok()
        && match url.host_str() {
            Some("github.com") => {
                url.path().split('/').collect::<Vec<_>>().get(3..5)
                    == Some(&["releases", "download"][..])
            }
            Some("release-assets.githubusercontent.com") => true,
            _ => false,
        }
}
fn compatible(range: &str) -> bool {
    semver::VersionReq::parse(range).is_ok_and(|range| negotiate_api_version(&range).is_some())
}
fn newest_api_version() -> String {
    SUPPORTED_API_VERSIONS
        .last()
        .copied()
        .unwrap_or_default()
        .to_owned()
}
fn host_api_versions() -> Vec<String> {
    SUPPORTED_API_VERSIONS
        .iter()
        .map(|version| (*version).to_owned())
        .collect()
}
fn hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub(super) fn parse_catalog(raw: &[u8]) -> Result<Catalog, String> {
    if raw.len() > MAX_CATALOG {
        return Err("Catalog exceeds 1 MiB".into());
    }
    let catalog: Catalog =
        serde_json::from_slice(raw).map_err(|e| format!("Invalid extension catalog: {e}"))?;
    if catalog.schema_version != 1 {
        return Err("Unsupported extension catalog version".into());
    }
    let mut ids = BTreeSet::new();
    for entry in &catalog.extensions {
        if entry.id.len() > 128
            || !entry.id.contains('.')
            || entry.id.split('.').any(|part| {
                part.is_empty()
                    || !part
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            })
            || !ids.insert(&entry.id)
        {
            return Err("Invalid or duplicate catalog extension ID".into());
        }
        if [&entry.name, &entry.description, &entry.license]
            .iter()
            .any(|s| s.trim().is_empty())
        {
            return Err("Missing catalog metadata".into());
        }
        // A format character, such as a right-to-left override, can make one app's name
        // display as another's.
        if [&entry.name, &entry.description]
            .iter()
            .any(|s| s.chars().any(is_format_character))
        {
            return Err(format!(
                "Catalog app {} has a bidirectional or invisible format character in its name or description",
                entry.id
            ));
        }
        https_url(&entry.repository)?;
        https_url(&entry.tested_host.repository)?;
        let url = https_url(&entry.release.manifest_url)?;
        if url.host_str() != Some("github.com") || !allowed_download(&url) {
            return Err("Catalog manifests must be GitHub release assets".into());
        }
        if !hex(&entry.release.sha256, 64) || !hex(&entry.tested_host.revision, 40) {
            return Err("Invalid catalog checksum or tested revision".into());
        }
        semver::Version::parse(&entry.release.version).map_err(|_| "Invalid release version")?;
        semver::VersionReq::parse(&entry.release.srelens_api_version)
            .map_err(|_| "Invalid extension API range")?;
    }
    Ok(catalog)
}
fn download(url: &str, limit: usize) -> Result<Vec<u8>, String> {
    let url = https_url(url)?;
    if url.as_str() != CATALOG_URL && !allowed_download(&url) {
        return Err("Unsupported extension download URL".into());
    }
    // Catalog browsing may be the first network operation, before any kube
    // client exists. Workspace feature unification can link two providers.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .connect_timeout(Duration::from_secs(10))
        .user_agent("srelens-extension-catalog")
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 || !allowed_download(attempt.url()) {
                attempt.error("Unsupported extension download redirect")
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|e| e.to_string())?;
    let response = client
        .get(url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("Download extension catalog/manifest: {e}"))?;
    let mut raw = Vec::new();
    response
        .take(limit as u64 + 1)
        .read_to_end(&mut raw)
        .map_err(|e| e.to_string())?;
    if raw.len() > limit {
        return Err("Extension download exceeds size limit".into());
    }
    Ok(raw)
}
fn load_with(
    path: &Path,
    refresh: bool,
    fetch: impl FnOnce() -> Result<Vec<u8>, String>,
) -> Result<Snapshot, String> {
    // Serialize refreshes across windows/processes and atomically replace validated caches.
    let _lock = super::super::settings::write_lock(path)?;
    let cached = fs::File::open(path).ok().and_then(|f| {
        let mut raw = Vec::new();
        f.take((MAX_CATALOG + 65536) as u64 + 1)
            .read_to_end(&mut raw)
            .ok()?;
        let mut state: Snapshot = serde_json::from_slice(&raw).ok()?;
        state.catalog = parse_catalog(&serde_json::to_vec(&state.catalog).ok()?).ok()?;
        state.incompatible = state
            .catalog
            .extensions
            .iter()
            .filter(|e| !compatible(&e.release.srelens_api_version))
            .map(|e| e.id.clone())
            .collect();
        state.host_api_version = newest_api_version();
        state.host_api_versions = host_api_versions();
        Some(state)
    });
    if !refresh
        && cached
            .as_ref()
            .is_some_and(|s| s.fetched_at <= now() && now() - s.fetched_at < TTL)
    {
        return Ok(cached.unwrap());
    }
    let result = fetch().and_then(|raw| parse_catalog(&raw));
    let catalog = match result {
        Ok(c) => c,
        Err(error) => {
            return match cached {
                Some(mut s) => {
                    s.stale = true;
                    s.error = Some(error);
                    Ok(s)
                }
                None => Err(error),
            }
        }
    };
    let state = Snapshot {
        incompatible: catalog
            .extensions
            .iter()
            .filter(|e| !compatible(&e.release.srelens_api_version))
            .map(|e| e.id.clone())
            .collect(),
        catalog,
        fetched_at: now(),
        stale: false,
        error: None,
        host_api_version: newest_api_version(),
        host_api_versions: host_api_versions(),
    };
    let mut file =
        tempfile::NamedTempFile::new_in(path.parent().ok_or("Catalog cache has no parent")?)
            .map_err(|e| e.to_string())?;
    file.write_all(&serde_json::to_vec(&state).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(path)
        .map_err(|e| format!("Save extension catalog: {e}"))?;
    Ok(state)
}
fn load(path: &Path, refresh: bool) -> Result<Snapshot, String> {
    load_with(path, refresh, || download(CATALOG_URL, MAX_CATALOG))
}
/// Whether the cached catalog, fresh or stale, lists this exact release. Never fetches.
pub(super) fn cached_release(path: &Path, id: &str, sha256: &str) -> bool {
    load_with(path, false, || {
        Err("the cached catalog is read without fetching".into())
    })
    .is_ok_and(|snapshot| {
        snapshot
            .catalog
            .extensions
            .iter()
            .any(|entry| entry.id == id && entry.release.sha256 == sha256)
    })
}
fn verify_manifest(entry: &Entry, raw: &[u8]) -> Result<String, String> {
    if raw.len() > MAX_MANIFEST_BYTES {
        return Err("Manifest exceeds size limit".into());
    }
    if format!("{:x}", Sha256::digest(raw)) != entry.release.sha256 {
        return Err("Extension manifest checksum does not match the catalog".into());
    }
    let source = std::str::from_utf8(raw).map_err(|_| "Manifest is not UTF-8")?;
    let manifest = Manifest::parse(source)?;
    if manifest.id != entry.id
        || manifest.version != entry.release.version
        || manifest.api_version != entry.release.srelens_api_version
    {
        return Err("Extension manifest identity/version/API does not match the catalog".into());
    }
    Ok(source.into())
}
fn signature_url(entry: &Entry) -> Result<Option<String>, String> {
    if !super::signing::claims_official(&entry.id, &entry.repository) {
        return Ok(None);
    }
    let repository =
        super::signing::repository(&entry.id).ok_or("Unknown official app signing identity")?;
    let expected = format!(
        "{repository}/releases/download/v{}/manifest.json",
        entry.release.version
    );
    // The repository is compared case-insensitively, as GitHub resolves it; the release
    // asset URL is the exact one the signature is published beside.
    if !super::signing::same_repository(&entry.repository, repository)
        || entry.release.manifest_url != expected
    {
        return Err("Official app release does not match its trusted repository".into());
    }
    Ok(Some(format!("{expected}.sig")))
}
fn verify_release(entry: &Entry, raw: &[u8], signature: Option<Vec<u8>>) -> Result<Review, String> {
    let manifest = verify_manifest(entry, raw)?;
    if signature_url(entry)?.is_some() {
        super::signing::verify(
            raw,
            signature
                .as_deref()
                .ok_or("Official app publisher signature is missing")?,
        )?;
    } else if signature.is_some() {
        return Err("Unrecognized app publisher signature".into());
    }
    Ok(Review {
        manifest,
        signature,
    })
}
pub(super) fn register(reg: &mut Registry, path: PathBuf, core: Arc<Registry>) {
    let p = path.clone();
    reg.register(Capability::typed::<ListIn, Snapshot, _, _>(
        "extensions.catalog",
        "Browse the native extension catalog with a durable cache; never connects clusters",
        Annotations::READ_ONLY,
        move |input| {
            let path = p.clone();
            async move {
                tokio::task::spawn_blocking(move || load(&path, input.refresh))
                    .await
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?
                    .map_err(CapabilityError::Handler)
            }
        },
    ));
    reg.register(Capability::typed::<ManifestIn, Review, _, _>("extensions.catalogManifest", "Download and checksum-verify a catalog manifest for permission review; does not install it", Annotations::READ_ONLY, move |input| {
        let path = path.clone(); let core = core.clone();
        async move { tokio::task::spawn_blocking(move || {
            let state = load(&path, false)?;
            let entry = state.catalog.extensions.iter().find(|e| e.id == input.id && e.release.sha256 == input.sha256).ok_or("Catalog release changed; refresh and review it again")?;
            if !compatible(&entry.release.srelens_api_version) { return Err("Extension requires a different host API version".to_string()); }
            let signature = signature_url(entry)?.map(|url| download(&url, 64)).transpose()?;
            let review = verify_release(entry, &download(&entry.release.manifest_url, MAX_MANIFEST_BYTES)?, signature)?;
            let manifest = &review.manifest;
            let parsed = Manifest::parse(&manifest)?;
            super::validate_app(&parsed, &parsed.permissions, core)?;
            Ok(review)
        }).await.map_err(|e| CapabilityError::Handler(e.to_string()))?.map_err(CapabilityError::Handler) }
    }));
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Vec<u8> {
        include_bytes!("../../tests/fixtures/extension-catalog.json").to_vec()
    }
    #[test]
    fn a_cached_release_is_known_by_id_and_exact_checksum_without_fetching() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("apps.catalog.json");
        let entry = parse_catalog(&fixture()).unwrap().extensions.remove(0);
        assert!(
            !cached_release(&path, &entry.id, &entry.release.sha256),
            "no cache yet"
        );
        load_with(&path, true, || Ok(fixture())).unwrap();
        assert!(cached_release(&path, &entry.id, &entry.release.sha256));
        assert!(!cached_release(&path, &entry.id, &"0".repeat(64)));
        assert!(!cached_release(
            &path,
            "org.other.app",
            &entry.release.sha256
        ));
    }
    #[test]
    fn validates_catalog_and_reports_api_compatibility() {
        let catalog = parse_catalog(&fixture()).unwrap();
        assert_eq!(catalog.extensions.len(), 2);
        assert!(compatible(
            &catalog.extensions[0].release.srelens_api_version
        ));
        assert!(!compatible("^99"));
        let mut value: serde_json::Value = serde_json::from_slice(&fixture()).unwrap();
        value["extensions"][1] = value["extensions"][0].clone();
        assert!(parse_catalog(&serde_json::to_vec(&value).unwrap()).is_err());
        value["schemaVersion"] = serde_json::json!(2);
        assert!(parse_catalog(&serde_json::to_vec(&value).unwrap()).is_err());
    }
    #[test]
    fn verifies_exact_bytes_identity_and_api_before_review() {
        let mut entry = parse_catalog(&fixture()).unwrap().extensions.remove(0);
        let raw = include_bytes!("../../tests/fixtures/argocd-manifest.json");
        entry.release.sha256 = format!("{:x}", Sha256::digest(raw));
        assert!(verify_manifest(&entry, raw).is_ok());
        assert!(verify_manifest(&entry, b"{}")
            .unwrap_err()
            .contains("checksum"));
        entry.id = "org.other.extension".into();
        assert!(verify_manifest(&entry, raw)
            .unwrap_err()
            .contains("identity"));
    }
    #[test]
    fn official_releases_require_the_pinned_signature_and_repository() {
        let mut entry = parse_catalog(&fixture()).unwrap().extensions.remove(0);
        let raw = include_bytes!("../../tests/fixtures/argocd-manifest.json");
        let sig = include_bytes!("../../tests/fixtures/argocd-manifest.sig").to_vec();
        entry.release.sha256 = format!("{:x}", Sha256::digest(raw));
        assert!(verify_release(&entry, raw, Some(sig.clone())).is_ok());
        assert!(verify_release(&entry, raw, None)
            .unwrap_err()
            .contains("missing"));
        let mut changed = raw.to_vec();
        changed.push(b' ');
        entry.release.sha256 = format!("{:x}", Sha256::digest(&changed));
        assert!(verify_release(&entry, &changed, Some(sig))
            .unwrap_err()
            .contains("signature"));
        entry.repository = "https://github.com/attacker/extension-argocd".into();
        assert!(signature_url(&entry).is_err());
        entry.id = "org.srelens.unknown".into();
        assert!(signature_url(&entry).is_err());
        // An official repository cannot publish unsigned under an unofficial ID.
        entry.id = "org.other.argocd".into();
        entry.repository = "https://github.com/srelens/extension-argocd".into();
        assert!(signature_url(&entry).is_err());
        // Lookalike prefixes are ordinary, unsigned third-party entries.
        entry.id = "org.srelensx.argocd".into();
        entry.repository = "https://github.com/srelensx/extension-argocd".into();
        assert_eq!(signature_url(&entry).unwrap(), None);
    }
    #[test]
    fn a_trusted_repository_in_another_case_still_requires_the_publisher_signature() {
        let mut entry = parse_catalog(&fixture()).unwrap().extensions.remove(0);
        let raw = include_bytes!("../../tests/fixtures/argocd-manifest.json");
        let sig = include_bytes!("../../tests/fixtures/argocd-manifest.sig").to_vec();
        entry.release.sha256 = format!("{:x}", Sha256::digest(raw));
        // GitHub resolves this to the pinned srelens repository, so the release is official
        // and its signature is required.
        entry.repository = "https://github.com/SRELENS/Extension-ArgoCD".into();
        assert_eq!(
            signature_url(&entry).unwrap(),
            Some(format!("{}.sig", entry.release.manifest_url))
        );
        assert!(verify_release(&entry, raw, None)
            .unwrap_err()
            .contains("missing"));
        assert!(verify_release(&entry, raw, Some(sig)).is_ok());
        // An owner that only looks like the trusted one stays an unsigned third party.
        entry.id = "org.srelensx.argocd".into();
        entry.repository = "https://github.com/SRELENSX/extension-argocd".into();
        assert_eq!(signature_url(&entry).unwrap(), None);
    }
    #[test]
    fn verify_manifest_rejects_oversized_and_invalid_encoding() {
        let entry = parse_catalog(&fixture()).unwrap().extensions.remove(0);
        assert!(verify_manifest(&entry, &vec![b' '; MAX_MANIFEST_BYTES + 1])
            .unwrap_err()
            .contains("size"));
        let bad_utf8 = vec![0xff, 0xff];
        let mut entry2 = entry.clone();
        entry2.release.sha256 = format!("{:x}", Sha256::digest(&bad_utf8));
        assert!(verify_manifest(&entry2, &bad_utf8)
            .unwrap_err()
            .contains("UTF-8"));
    }
    #[test]
    fn verify_release_rejects_unrecognized_signature() {
        let mut manifest_val: Value =
            serde_json::from_slice(include_bytes!("../../tests/fixtures/argocd-manifest.json"))
                .unwrap();
        manifest_val["id"] = json!("org.thirdparty.app");
        let raw = serde_json::to_vec(&manifest_val).unwrap();
        let mut entry = parse_catalog(&fixture()).unwrap().extensions.remove(0);
        entry.id = "org.thirdparty.app".into();
        entry.repository = "https://github.com/thirdparty/app".into();
        entry.release.sha256 = format!("{:x}", Sha256::digest(&raw));
        assert!(verify_release(&entry, &raw, Some(vec![1, 2, 3]))
            .unwrap_err()
            .contains("Unrecognized"));
    }
    #[tokio::test]
    async fn catalog_capabilities_registered_and_invoked() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("catalog.json");
        fs::write(&path, fixture()).unwrap();

        let mut reg = Registry::new();
        let core = Arc::new(Registry::new());
        register(&mut reg, path, core);

        let cap_list = reg.get("extensions.catalog").unwrap();
        let list_res = (cap_list.handler)(serde_json::json!({"refresh": false})).await;
        assert!(list_res.is_ok());

        let cap_manifest = reg.get("extensions.catalogManifest").unwrap();
        let err_res =
            (cap_manifest.handler)(serde_json::json!({"id": "nonexistent", "sha256": "fake"}))
                .await;
        assert!(err_res.is_err());
    }
    #[test]
    fn additive_catalog_fields_do_not_break_released_hosts() {
        let mut value: Value = serde_json::from_slice(&fixture()).unwrap();
        value["revoked"] = json!([{"id": "org.srelens.argocd", "versions": ["0.0.1"]}]);
        value["extensions"][0]["publisher"] = json!({"name": "srelens"});
        value["extensions"][0]["categories"] = json!(["gitops"]);
        value["extensions"][0]["release"]["signatureUrl"] = json!("https://example.invalid");
        value["extensions"][0]["testedHost"]["platform"] = json!("linux");
        let catalog = parse_catalog(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(catalog.extensions.len(), 2);
        value["extensions"][0]["release"]["sha256"] = json!("bad");
        assert!(parse_catalog(&serde_json::to_vec(&value).unwrap()).is_err());
    }
    #[test]
    fn rejects_private_downloads_and_redirects() {
        for raw in [
            "http://github.com/a/b",
            "https://localhost/file",
            "https://github.com.evil/a/b",
            "https://user@github.com/a/b",
            "https://github.com:8443/a/b",
        ] {
            assert!(!allowed_download(&reqwest::Url::parse(raw).unwrap()));
        }
        assert!(allowed_download(
            &reqwest::Url::parse("https://github.com/a/b/releases/download/v1/manifest.json")
                .unwrap()
        ));
        assert!(allowed_download(
            &reqwest::Url::parse("https://release-assets.githubusercontent.com/a").unwrap()
        ));
    }
    #[test]
    fn caches_across_instances_and_keeps_offline_errors_visible() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("catalog.json");
        let first = load_with(&path, false, || Ok(fixture())).unwrap();
        assert!(!first.stale);
        let cached =
            load_with(&path, false, || panic!("fresh cache should avoid network")).unwrap();
        assert_eq!(first.fetched_at, cached.fetched_at);
        let stale = load_with(&path, true, || Err("offline".into())).unwrap();
        assert!(stale.stale);
        assert!(stale.error.unwrap().contains("offline"));
        assert!(load_with(&dir.path().join("missing"), false, || Err("offline".into())).is_err());
    }
    #[test]
    fn expired_or_corrupt_cache_is_refetched_and_failed_refresh_does_not_overwrite_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let mut state = load_with(&path, false, || Ok(fixture())).unwrap();
        state.fetched_at = now() - TTL - 1;
        fs::write(&path, serde_json::to_vec(&state).unwrap()).unwrap();
        let before = fs::read(&path).unwrap();
        assert!(
            load_with(&path, false, || Err("offline".into()))
                .unwrap()
                .stale
        );
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(!load_with(&path, false, || Ok(fixture())).unwrap().stale);
        fs::write(&path, "invalid json").unwrap();
        assert!(load_with(&path, false, || Err("offline".into())).is_err());
        assert_eq!(
            load_with(&path, false, || Ok(fixture()))
                .unwrap()
                .catalog
                .extensions
                .len(),
            2
        );
    }
    #[test]
    fn rejects_invalid_catalog_fields_and_unrecognized_wire_payloads() {
        let base: Value = serde_json::from_slice(&fixture()).unwrap();
        for (pointer, replacement) in [
            ("/extensions/0/release/sha256", json!("bad")),
            (
                "/extensions/0/release/manifestUrl",
                json!("https://localhost/file"),
            ),
            ("/extensions/0/release/version", json!("bad")),
            ("/extensions/0/release/srelensApiVersion", json!("bad")),
            ("/extensions/0/name", json!("")),
            ("/extensions/0/repository", json!("javascript:alert(1)")),
        ] {
            let mut value = base.clone();
            *value.pointer_mut(pointer).unwrap() = replacement;
            assert!(
                parse_catalog(&serde_json::to_vec(&value).unwrap()).is_err(),
                "{pointer}"
            );
        }
        assert!(parse_catalog(&vec![b' '; MAX_CATALOG + 1]).is_err());
        assert!(serde_json::from_value::<ManifestIn>(
            json!({"id":"org.test.app","sha256":"abc","manifestUrl":"https://localhost"})
        )
        .is_err());
        assert!(
            serde_json::from_value::<ListIn>(json!({"refresh":true}))
                .unwrap()
                .refresh
        );
    }
    #[test]
    fn refuses_bidirectional_and_invisible_characters_in_names_and_descriptions() {
        let base: Value = serde_json::from_slice(&fixture()).unwrap();
        for (pointer, text) in [
            // A right-to-left override displays this name as "Argo CD".
            ("/extensions/0/name", "\u{202E}DC ogrA"),
            ("/extensions/0/name", "Argo CD\u{200B}"),
            ("/extensions/0/description", "Soft\u{00AD}hyphen"),
            (
                "/extensions/1/description",
                "GitOps \u{2066}dashboards\u{2069}",
            ),
            ("/extensions/1/description", "\u{FEFF}Flux"),
        ] {
            let mut value = base.clone();
            *value.pointer_mut(pointer).unwrap() = json!(text);
            let parsed = parse_catalog(&serde_json::to_vec(&value).unwrap()).map(|_| ());
            assert!(
                parsed
                    .as_ref()
                    .is_err_and(|reason| reason.contains("invisible")),
                "{pointer} {text:?}: {parsed:?}"
            );
        }
        let mut value = base;
        value["extensions"][0]["name"] = json!("Argo CD — Übersicht");
        value["extensions"][0]["description"] =
            json!("GitOps アプリケーション, приложения и عمليات");
        parse_catalog(&serde_json::to_vec(&value).unwrap()).unwrap();
    }
    #[test]
    fn snapshot_lists_every_supported_api_version_and_reuses_legacy_caches() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let state = load_with(&path, false, || Ok(fixture())).unwrap();
        let value = serde_json::to_value(&state).unwrap();
        let supported = json!(srelens_plugin_host::SUPPORTED_API_VERSIONS);
        assert_eq!(value["hostApiVersions"], supported);
        // The singular field stays for API 0.1 clients, as the newest supported version.
        let newest = json!(srelens_plugin_host::SUPPORTED_API_VERSIONS.last().unwrap());
        assert_eq!(value["hostApiVersion"], newest);
        // A cache written before the list existed is still used rather than refetched.
        let mut legacy = value;
        legacy.as_object_mut().unwrap().remove("hostApiVersions");
        legacy["hostApiVersion"] = json!("0.1.0");
        fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        let cached = load_with(&path, false, || {
            panic!("a legacy cache must not force a refetch")
        })
        .unwrap();
        assert_eq!(
            serde_json::to_value(&cached).unwrap()["hostApiVersions"],
            supported
        );
        assert_eq!(
            serde_json::to_value(&cached).unwrap()["hostApiVersion"],
            newest
        );
    }
    #[test]
    #[ignore = "downloads the public catalog and release manifests; no clusters or installs"]
    fn public_catalog_release_smoke() {
        let dir = tempfile::tempdir().unwrap();
        let state = load(&dir.path().join("cache.json"), true).unwrap();
        let core = Arc::new(crate::build_registry_with_paths(
            srelens_kube::client_cache::ClientCache::new_many(vec![]),
            vec![],
        ));
        assert!(!state.catalog.extensions.is_empty());
        for entry in state.catalog.extensions {
            let raw = download(&entry.release.manifest_url, MAX_MANIFEST_BYTES).unwrap();
            let signature = signature_url(&entry)
                .unwrap()
                .map(|url| download(&url, 64).unwrap());
            let source = verify_release(&entry, &raw, signature).unwrap().manifest;
            let parsed = Manifest::parse(&source).unwrap();
            super::super::validate_app(&parsed, &parsed.permissions, core.clone()).unwrap();
            println!("Verified {} {}", entry.id, entry.release.version);
        }
    }
}
