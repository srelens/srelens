//! A fixed public catalog; official manifests require a pinned publisher signature.
use super::*;
use sha2::{Digest, Sha256};
use srelens_plugin_host::{
    is_format_character, negotiate_api_version_in, MAX_MANIFEST_BYTES, SUPPORTED_API_VERSIONS,
};
use std::{
    collections::BTreeSet,
    io::Read as _,
    sync::Mutex,
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
    compatible_in(range, SUPPORTED_API_VERSIONS)
}
/// Whether a host implementing the API versions `supported` can install a release that
/// requires `range`: what the catalog asks before it offers one.
fn compatible_in(range: &str, supported: &[&str]) -> bool {
    semver::VersionReq::parse(range)
        .is_ok_and(|range| negotiate_api_version_in(&range, supported).is_some())
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
        // display as another's, and a control character can hide or reshape the rest of
        // the line. The catalog's name, description and license are rendered as a manifest
        // label is, so they are held to the same rule (`label` in srelens-plugin-host).
        if [&entry.name, &entry.description, &entry.license]
            .iter()
            .any(|s| s.chars().any(|c| c.is_control() || is_format_character(c)))
        {
            return Err(format!(
                "Catalog app {} has a control or invisible formatting character in its name, description or license",
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
/// The cached catalog, validated again, with this host's fields recomputed; `None` when
/// there is no cache or it cannot be trusted.
fn read_cache(path: &Path) -> Option<Snapshot> {
    let f = fs::File::open(path).ok()?;
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
}
/// Fetched less than a day ago, and not in the future.
fn is_fresh(state: &Snapshot) -> bool {
    state.fetched_at <= now() && now() - state.fetched_at < TTL
}
fn load_with(
    path: &Path,
    refresh: bool,
    fetch: impl FnOnce() -> Result<Vec<u8>, String>,
) -> Result<Snapshot, String> {
    // Serialize refreshes across windows/processes and atomically replace validated caches.
    let _lock = super::super::settings::write_lock(path)?;
    let cached = read_cache(path);
    if !refresh && cached.as_ref().is_some_and(is_fresh) {
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
    save_cache(path, catalog)
}
/// Replace the cache with `catalog`, fetched now. The caller holds the cache's lock.
fn save_cache(path: &Path, catalog: Catalog) -> Result<Snapshot, String> {
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
    let raw = serde_json::to_vec(&state).map_err(|e| e.to_string())?;
    crate::durable::replace(path, &raw).map_err(|e| format!("Save extension catalog: {e}"))?;
    Ok(state)
}
fn load(path: &Path, refresh: bool) -> Result<Snapshot, String> {
    load_with(path, refresh, || download(CATALOG_URL, MAX_CATALOG))
}
/// The catalog cache a registry's apps read.
#[derive(Clone)]
pub(super) enum CatalogCache {
    /// This host's own cache file (the desktop). `extensions.catalog` fetches the catalog
    /// into it when it is a day old or the reader asks.
    Owned(PathBuf),
    /// The cache every user of one server shares. Capabilities only read it.
    Shared(SharedCatalog),
}
impl CatalogCache {
    /// The catalog as this cache has it; an owned cache is fetched first when it is a day
    /// old or `refresh` asks. The shared cache never is: it is what the server last fetched.
    fn snapshot(&self, refresh: bool) -> Result<Snapshot, String> {
        match self {
            Self::Owned(path) => load(path, refresh),
            Self::Shared(shared) => shared.read(),
        }
    }
    /// Whether the cached catalog, fresh or stale, lists this exact release. Never fetches.
    pub(super) fn lists_release(&self, id: &str, sha256: &str) -> bool {
        let path = match self {
            Self::Owned(path) => path,
            Self::Shared(shared) => &shared.path,
        };
        cached_release(path, id, sha256)
    }
}
/// The catalog cache every user of one web server shares (#515).
///
/// The catalog is not anyone's: it is the fixed public catalog, the same for every user.
/// What is per user is the inventory, and every install from this cache downloads and
/// verifies its release again for the user installing it. So one cache serves them all,
/// and nothing a user sends can write it: their `extensions.catalog` and
/// `extensions.catalogManifest` read it and never fetch into it, and only the server, on
/// its own schedule through [`SharedCatalog::refresh_if_stale`], replaces it — with what it
/// downloaded from the fixed catalog URL and validated, exactly as a desktop refresh does.
#[derive(Clone)]
pub struct SharedCatalog {
    path: PathBuf,
    /// Why the server's last refresh failed, if it did, for readers of a stale cache.
    last_error: Arc<Mutex<Option<String>>>,
}
impl SharedCatalog {
    /// The shared cache kept at `path`. Nothing is read or fetched until it is used.
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            last_error: Arc::default(),
        }
    }
    /// Fetch the catalog into the cache when it is missing or a day old, as a desktop
    /// does when its catalog is opened. For the server's own schedule: no capability calls
    /// it. A failed fetch keeps the cache as it was, and says why.
    pub fn refresh_if_stale(&self) -> Result<(), String> {
        self.refresh_if_stale_with(|| download(CATALOG_URL, MAX_CATALOG))
    }
    /// [`SharedCatalog::refresh_if_stale`], with the fetch supplied. What it returns is
    /// validated as a downloaded catalog is.
    pub fn refresh_if_stale_with(
        &self,
        fetch: impl FnOnce() -> Result<Vec<u8>, String>,
    ) -> Result<(), String> {
        let outcome = self.refresh_outside_the_lock(fetch);
        *self.last_error.lock().unwrap() = outcome.as_ref().err().cloned();
        outcome
    }
    /// The cache's lock is held to look and to save, never across the download: every
    /// user's catalog read and install check waits on it, and a download may take the
    /// whole of its timeout.
    fn refresh_outside_the_lock(
        &self,
        fetch: impl FnOnce() -> Result<Vec<u8>, String>,
    ) -> Result<(), String> {
        let fresh = || read_cache(&self.path).is_some_and(|cached| is_fresh(&cached));
        {
            let _lock = super::super::settings::write_lock(&self.path)?;
            if fresh() {
                return Ok(());
            }
        }
        let catalog = fetch().and_then(|raw| parse_catalog(&raw))?;
        let _lock = super::super::settings::write_lock(&self.path)?;
        // Another refresh may have saved a copy while this one downloaded; it is at least
        // as new, so it stays.
        if fresh() {
            return Ok(());
        }
        save_cache(&self.path, catalog).map(|_| ())
    }
    /// What a user reads: the cache as the server last fetched it. Never fetches and never
    /// writes it; a cache past its day is returned as stale, with why it was not refreshed.
    fn read(&self) -> Result<Snapshot, String> {
        let why = match self.last_error.lock().unwrap().clone() {
            Some(error) => format!("the server could not refresh the shared catalog: {error}"),
            None => "the server has not refreshed the shared catalog yet".to_owned(),
        };
        load_with(&self.path, false, || Err(why))
    }
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
pub(super) fn register(reg: &mut Registry, cache: CatalogCache, core: Arc<Registry>) {
    let c = cache.clone();
    reg.register(Capability::typed::<ListIn, Snapshot, _, _>(
        "extensions.catalog",
        "Browse the native extension catalog with a durable cache; never connects clusters",
        Annotations::READ_ONLY,
        move |input| {
            let cache = c.clone();
            async move {
                tokio::task::spawn_blocking(move || cache.snapshot(input.refresh))
                    .await
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?
                    .map_err(CapabilityError::Handler)
            }
        },
    ));
    reg.register(Capability::typed::<ManifestIn, Review, _, _>("extensions.catalogManifest", "Download and checksum-verify a catalog manifest for permission review; does not install it", Annotations::READ_ONLY, move |input| {
        let cache = cache.clone(); let core = core.clone();
        async move { tokio::task::spawn_blocking(move || {
            let state = cache.snapshot(false)?;
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
        assert!(!compatible(&catalog.extensions[0].release.srelens_api_version));
        assert!(compatible("^0.3"));
        assert!(compatible("^0.4"));
        assert!(!compatible("^0.2"));
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
        let source = super::super::tests::manifest();
        let raw = source.as_bytes();
        entry.id = "org.example.argocd".into();
        entry.release.srelens_api_version = "^0.4".into();
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
        assert!(super::super::signing::verify_for(&entry.id, raw, &sig).is_ok());
        assert!(verify_release(&entry, raw, Some(sig.clone())).unwrap_err().contains("requires API ^0.1"));
        // A current manifest still requires a signature. Reusing the old signature after
        // upgrading its API range is tampering, never a newly signed release.
        let current = std::str::from_utf8(raw).unwrap().replace("^0.1", "^0.3");
        let raw = current.as_bytes();
        entry.release.srelens_api_version = "^0.3".into();
        entry.release.sha256 = format!("{:x}", Sha256::digest(raw));
        assert!(verify_release(&entry, raw, Some(sig.clone()))
            .unwrap_err()
            .contains("signature"));
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
        assert!(super::super::signing::verify_for(&entry.id, raw, &sig).is_ok());
        assert!(verify_release(&entry, raw, Some(sig.clone())).unwrap_err().contains("requires API ^0.1"));
        let current = std::str::from_utf8(raw).unwrap().replace("^0.1", "^0.3");
        entry.release.srelens_api_version = "^0.3".into();
        entry.release.sha256 = format!("{:x}", Sha256::digest(current.as_bytes()));
        assert!(verify_release(&entry, current.as_bytes(), Some(sig))
            .unwrap_err()
            .contains("signature"));
        assert!(verify_release(&entry, current.as_bytes(), None).unwrap_err().contains("missing"));
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
        manifest_val["srelensApiVersion"] = json!("^0.3");
        let raw = serde_json::to_vec(&manifest_val).unwrap();
        let mut entry = parse_catalog(&fixture()).unwrap().extensions.remove(0);
        entry.id = "org.thirdparty.app".into();
        entry.release.srelens_api_version = "^0.3".into();
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
        // Seed the cache in the shape `load` writes — a `Snapshot`, not a bare
        // catalog. A bare catalog fails the cache parse, so the capability
        // fetched the live catalog: red offline, and online it tested mutable
        // live data instead of this fixture (#615).
        let seeded = load_with(&path, false, || Ok(fixture())).unwrap();

        let mut reg = Registry::new();
        let core = Arc::new(Registry::new());
        register(&mut reg, CatalogCache::Owned(path), core);

        let cap_list = reg.get("extensions.catalog").unwrap();
        let list_res = (cap_list.handler)(serde_json::json!({"refresh": false}))
            .await
            .unwrap();
        // Served from the seeded cache, not a fresh download.
        assert_eq!(list_res["fetchedAt"], json!(seeded.fetched_at));

        let cap_manifest = reg.get("extensions.catalogManifest").unwrap();
        let err_res =
            (cap_manifest.handler)(serde_json::json!({"id": "nonexistent", "sha256": "fake"}))
                .await;
        assert!(err_res.is_err());
    }
    /// The web's shared cache (#515): a user's capabilities read it, whatever they ask,
    /// and neither fetch nor write it. Only the server's refresh fills it.
    #[tokio::test]
    async fn no_capability_fetches_into_or_writes_the_shared_catalog() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache").join("extensions.catalog.json");
        let shared = SharedCatalog::new(path.clone());
        let mut reg = Registry::new();
        register(
            &mut reg,
            CatalogCache::Shared(shared.clone()),
            Arc::new(Registry::new()),
        );
        let refresh = json!({"refresh": true});

        // Nothing fetched yet: the read says so rather than going to the network.
        let refused = reg
            .invoke("extensions.catalog", refresh.clone())
            .await
            .unwrap_err()
            .to_string();
        assert!(
            refused.contains("has not refreshed the shared catalog yet"),
            "{refused}"
        );
        assert!(!path.exists());

        shared.refresh_if_stale_with(|| Ok(fixture())).unwrap();
        let saved = fs::read(&path).unwrap();
        // Asked to refresh, a user gets the server's copy, unchanged on disk.
        let read = reg
            .invoke("extensions.catalog", refresh.clone())
            .await
            .unwrap();
        assert_eq!(read["stale"], json!(false));
        assert_eq!(read["catalog"]["extensions"].as_array().unwrap().len(), 2);
        assert_eq!(fs::read(&path).unwrap(), saved);
        // A fresh cache is not fetched again by the server either.
        shared
            .refresh_if_stale_with(|| unreachable!("a fresh cache is not fetched"))
            .unwrap();
        let entry = parse_catalog(&fixture()).unwrap().extensions.remove(0);
        let cache = CatalogCache::Shared(shared.clone());
        assert!(cache.lists_release(&entry.id, &entry.release.sha256));
        assert!(reg
            .invoke(
                "extensions.catalogManifest",
                json!({"id": "org.example.missing", "sha256": "0".repeat(64)})
            )
            .await
            .is_err());
        assert_eq!(fs::read(&path).unwrap(), saved);

        // A day old, it is served as stale, with why the server has not replaced it.
        let mut old: Snapshot = serde_json::from_slice(&saved).unwrap();
        old.fetched_at = 0;
        fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
        let aged = fs::read(&path).unwrap();
        let read = reg
            .invoke("extensions.catalog", refresh.clone())
            .await
            .unwrap();
        assert_eq!(read["stale"], json!(true));
        assert!(read["error"]
            .as_str()
            .unwrap()
            .contains("has not refreshed"));
        let failed = shared.refresh_if_stale_with(|| Err("offline".into()));
        assert_eq!(failed.unwrap_err(), "offline");
        let read = reg.invoke("extensions.catalog", refresh).await.unwrap();
        assert_eq!(
            read["error"],
            json!("the server could not refresh the shared catalog: offline")
        );
        assert_eq!(
            fs::read(&path).unwrap(),
            aged,
            "a failed refresh keeps the cache"
        );
    }
    /// A shared cache a day old, so the next server refresh downloads.
    fn aged_shared_catalog(dir: &Path) -> SharedCatalog {
        let shared = SharedCatalog::new(dir.join("extensions.catalog.json"));
        shared.refresh_if_stale_with(|| Ok(fixture())).unwrap();
        let mut old = read_cache(&shared.path).unwrap();
        old.fetched_at = 0;
        fs::write(&shared.path, serde_json::to_vec(&old).unwrap()).unwrap();
        shared
    }
    /// The server downloads outside the cache's lock: a user's read meanwhile is
    /// answered from the cache at once, not after the download.
    #[test]
    fn readers_are_answered_from_the_cache_while_the_server_downloads() {
        let dir = tempfile::tempdir().unwrap();
        let shared = aged_shared_catalog(dir.path());
        let reader = shared.clone();
        shared
            .refresh_if_stale_with(|| {
                let (tx, rx) = std::sync::mpsc::channel();
                let reader = reader.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(reader.read());
                });
                let read = rx
                    .recv_timeout(Duration::from_secs(5))
                    .expect("a read waited for the server's download");
                assert!(read.unwrap().stale, "the cached copy, as it was");
                Ok(fixture())
            })
            .unwrap();
        assert!(is_fresh(&read_cache(&shared.path).unwrap()));
    }
    /// A copy another refresh saved while this one downloaded is at least as new, so
    /// this one does not replace it.
    #[test]
    fn a_refresh_does_not_replace_a_copy_saved_while_it_downloaded() {
        let dir = tempfile::tempdir().unwrap();
        let shared = aged_shared_catalog(dir.path());
        let mut one: Value = serde_json::from_slice(&fixture()).unwrap();
        one["extensions"].as_array_mut().unwrap().truncate(1);
        let one = serde_json::to_vec(&one).unwrap();
        shared
            .refresh_if_stale_with(|| {
                let _lock = super::super::super::settings::write_lock(&shared.path).unwrap();
                save_cache(&shared.path, parse_catalog(&one).unwrap()).unwrap();
                Ok(fixture())
            })
            .unwrap();
        let kept = read_cache(&shared.path).unwrap();
        assert_eq!(kept.catalog.extensions.len(), 1);
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
            // Control characters are refused too: catalog text is rendered as a name and a
            // description, exactly as a manifest label is.
            ("/extensions/0/name", "Argo CD\u{0008}\u{0008}X"),
            ("/extensions/1/name", "Flux\u{007F}"),
            ("/extensions/0/description", "GitOps\nresources"),
            // The license is rendered beside the version, so it is held to the same rule.
            ("/extensions/0/license", "MIT\u{202E}"),
            ("/extensions/1/license", "Apache\u{0000}2.0"),
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
    /// The public catalog as it stood when API 0.4 was cut (#709), and the signed bytes of
    /// each release it listed: Argo CD 0.3.0 and Flux 0.4.0, both `^0.3` and using none
    /// of 0.4's fields. `public_catalog_release_smoke` follows the live catalog as it
    /// moves on; these stay, because installed copies of them go on reverifying.
    fn published_0_3_line() -> (Catalog, [(&'static str, &'static [u8], &'static [u8]); 2]) {
        let catalog = parse_catalog(include_bytes!(
            "../../tests/fixtures/extension-catalog-api-0.3.json"
        ))
        .unwrap();
        let releases = [
            (
                "org.srelens.argocd",
                &include_bytes!("../../tests/fixtures/argocd-0.3.0-manifest.json")[..],
                &include_bytes!("../../tests/fixtures/argocd-0.3.0-manifest.sig")[..],
            ),
            (
                "org.srelens.flux",
                &include_bytes!("../../tests/fixtures/flux-0.4.0-manifest.json")[..],
                &include_bytes!("../../tests/fixtures/flux-0.4.0-manifest.sig")[..],
            ),
        ];
        (catalog, releases)
    }
    #[test]
    fn the_published_0_3_line_releases_still_verify_install_and_reverify() {
        let (catalog, releases) = published_0_3_line();
        assert_eq!(catalog.extensions.len(), releases.len());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("apps.json");
        for (id, raw, signature) in releases {
            let entry = catalog.extensions.iter().find(|e| e.id == id).unwrap();
            assert_eq!(entry.release.srelens_api_version, "^0.3", "{id}");
            // Offered, checksum and pinned signature verified, and valid on this host.
            assert!(compatible(&entry.release.srelens_api_version), "{id}");
            let review = verify_release(entry, raw, Some(signature.to_vec())).unwrap();
            let parsed = Manifest::parse(&review.manifest).unwrap();
            super::super::validate_app(
                &parsed,
                &parsed.permissions,
                super::super::tests::fake_core(),
            )
            .unwrap();
            let state = super::super::tests::configure(
                &path,
                json!({"action":"install","manifest":review.manifest,
                    "signature":signature.to_vec(),"grants":parsed.permissions}),
            )
            .unwrap_or_else(|e| panic!("{id}: {e}"));
            let app = state
                .plugins
                .iter()
                .find(|app| app.manifest.id == id)
                .unwrap();
            assert!(app.enabled && app.quarantined.is_none(), "{id}");
            assert!(app.signature_proof.is_some(), "{id}");
        }
        // Loading the inventory rechecks every app against this host's API fields.
        let state = read(&path).unwrap();
        assert_eq!(state.plugins.len(), 2);
        assert!(state
            .plugins
            .iter()
            .all(|app| app.enabled && app.quarantined.is_none()));
    }
    #[test]
    fn a_host_on_the_0_3_line_lists_the_0_4_releases_as_incompatible() {
        // A host that implements 0.3 without 0.4's fields — every release up to the one
        // that cut 0.4 — asks this before it offers a release. The next Flux and Argo CD
        // releases require ^0.4, so it says "Incompatible" instead of failing them on
        // an unknown field; this host offers them, and still offers the 0.3 line.
        let only_0_3 = ["0.3.0"];
        for example in [
            include_str!("../../../../examples/extensions/argocd.json"),
            include_str!("../../../../examples/extensions/flux.json"),
        ] {
            let example: Value = serde_json::from_str(example).unwrap();
            let range = example["srelensApiVersion"].as_str().unwrap();
            assert!(
                !compatible_in(range, &only_0_3),
                "{} {range}",
                example["id"]
            );
            assert!(compatible(range), "{} {range}", example["id"]);
        }
        let (catalog, _) = published_0_3_line();
        for entry in &catalog.extensions {
            assert!(compatible_in(&entry.release.srelens_api_version, &only_0_3));
        }
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
