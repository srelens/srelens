//! The public catalog, signed by the catalog role of the pinned root (#559). A catalog that
//! is unsigned, signed by the wrong keys, expired, or older than one this host has already
//! verified is refused, and the last verified one is kept.
use super::http_policy;
use super::package;
use super::signing::{self, ReleaseSignature, MAX_RELEASE_SIGNATURE};
use super::trust::{Delegations, Envelope, Signer, TrustRoot, CATALOG_TYPE, MAX_PUBLISHERS};
use super::*;
use sha2::{Digest, Sha256};
use srelens_plugin_host::{
    is_format_character, negotiate_api_version_in, MAX_MANIFEST_BYTES, SUPPORTED_API_VERSIONS,
};
use std::{
    collections::BTreeSet,
    io::Read as _,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};
/// The signed catalog. `catalog.json` beside it is the unsigned catalog that hosts released
/// before #559 read, which cannot read this one.
const CATALOG_URL: &str =
    "https://raw.githubusercontent.com/srelens/extensions/main/catalog.signed.json";
/// The largest catalog document, as its signed bytes decode.
pub(super) const MAX_CATALOG: usize = 1024 * 1024;
/// The largest signed catalog file: that document in base64, its signatures, and room to
/// spare.
pub(super) const MAX_SIGNED_CATALOG: usize = 2 * 1024 * 1024;
/// The only catalog schema this host reads. Version 1 was the unsigned catalog.
const SCHEMA_VERSION: u32 = 2;
const TTL: u64 = 24 * 60 * 60;
// Catalog metadata is additive. Released hosts ignore fields they don't know, so the
// catalog can gain categories or revocations (#561) without every installed host
// rejecting it; a breaking change bumps `schemaVersion` instead.
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub(super) struct Catalog {
    #[serde(rename = "_type")]
    kind: String,
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    /// Higher for every catalog published. A host refuses one lower than it has verified.
    version: u64,
    /// When hosts stop trusting this catalog, as an RFC 3339 time.
    expires: String,
    /// Publisher delegations, each signed by the catalog role (`trust.rs`).
    publishers: Vec<Envelope>,
    extensions: Vec<Entry>,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
struct Entry {
    id: String,
    name: String,
    description: String,
    /// A link for readers. It names no signer: a delegated namespace does (`trust.rs`).
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
    /// The same release as a `.srelens-extension` package (#562), which carries the app's
    /// logo and files as well. A host that predates packages ignores it and installs
    /// `manifestUrl`, which a release with a package goes on publishing; so does a host
    /// that keeps no app files (the web host).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    package: Option<ReleasePackage>,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
struct ReleasePackage {
    /// A GitHub release asset whose name ends `.srelens-extension`.
    url: String,
    /// SHA-256 of the package file, lowercase hex.
    sha256: String,
}
/// A catalog whose signatures, publishers and fields have all been checked.
struct Verified {
    catalog: Catalog,
    delegations: Delegations,
    /// When it expires, in seconds since the Unix epoch.
    expires_at: i64,
    /// The SHA-256 of its signed bytes: two catalogs of one version must be one catalog.
    digest: String,
}
impl std::fmt::Debug for Verified {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "catalog version {}", self.catalog.version)
    }
}
impl Verified {
    fn expired(&self) -> bool {
        i64::try_from(now()).unwrap_or(i64::MAX) >= self.expires_at
    }
}
/// The cache on disk: the signed catalog exactly as it was verified, and when. It is
/// verified again on every read, so it is trusted no more than a download would be.
#[derive(Serialize, Deserialize)]
struct Cached {
    #[serde(rename = "signedCatalog")]
    signed_catalog: Envelope,
    #[serde(rename = "fetchedAt")]
    fetched_at: u64,
}
/// A catalog as this host holds it: the last one it verified, and whether a refresh failed.
#[derive(Debug)]
struct Loaded {
    verified: Verified,
    fetched_at: u64,
    stale: bool,
    error: Option<String>,
}
/// What `extensions.catalog` answers. The host fields are computed for every answer.
#[derive(Serialize, JsonSchema)]
struct Snapshot {
    catalog: CatalogView,
    #[serde(rename = "fetchedAt")]
    fetched_at: u64,
    stale: bool,
    error: Option<String>,
    /// The newest supported extension API version. Deprecated in favour of
    /// `host_api_versions`, and kept for API 0.1 clients until a new API line removes it.
    #[serde(rename = "hostApiVersion")]
    host_api_version: String,
    /// Every extension API version this host supports, oldest first.
    #[serde(rename = "hostApiVersions")]
    host_api_versions: Vec<String>,
    incompatible: Vec<String>,
}
/// The catalog as readers see it: its entries, and who is delegated what.
#[derive(Serialize, JsonSchema)]
struct CatalogView {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    version: u64,
    expires: String,
    publishers: Vec<PublisherView>,
    extensions: Vec<Entry>,
}
#[derive(Serialize, JsonSchema)]
struct PublisherView {
    id: String,
    name: String,
    namespaces: Vec<String>,
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
/// What a reviewer is shown before an install: the exact manifest, its publisher
/// signature, and for a package (#562) what else it holds.
#[derive(Debug, Serialize, JsonSchema)]
pub(super) struct Review {
    manifest: String,
    /// Over the manifest; for a package, over its digest list.
    signature: Option<Vec<u8>>,
    /// The key the release signature names, passed back to `extensions.configure`.
    #[serde(rename = "keyId", skip_serializing_if = "Option::is_none")]
    key_id: Option<String>,
    /// Who signed it: the publisher delegated its namespace.
    #[serde(rename = "signedBy", skip_serializing_if = "Option::is_none")]
    signed_by: Option<Signer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    package: Option<package::PackageReview>,
}
impl Review {
    /// The review of a package [`package::read`] verified.
    pub(super) fn of_package(verified: &package::Package) -> Self {
        Self {
            manifest: verified.manifest.clone(),
            signature: verified.signature.clone(),
            key_id: None,
            signed_by: verified.signer.clone(),
            package: Some(verified.review()),
        }
    }
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn https_url(raw: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(raw).map_err(|_| "Invalid catalog URL")?;
    // The shared policy's strictest reading: no port, and never plain HTTP.
    if http_policy::check_url(&url, http_policy::UrlRules::default()).is_err() {
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
/// An RFC 3339 time as seconds since the Unix epoch.
fn parse_expires(text: &str) -> Result<i64, String> {
    chrono::DateTime::parse_from_rfc3339(text)
        .map(|time| time.timestamp())
        .map_err(|_| format!("The catalog's expiry {text:?} is not an RFC 3339 time"))
}
/// A catalog document's own rules, before its publishers are verified: the bytes a
/// signature was checked over, or the fuzzer's.
pub(super) fn parse_catalog(raw: &[u8]) -> Result<Catalog, String> {
    if raw.len() > MAX_CATALOG {
        return Err("Catalog exceeds 1 MiB".into());
    }
    let catalog: Catalog =
        serde_json::from_slice(raw).map_err(|e| format!("Invalid extension catalog: {e}"))?;
    if catalog.kind != "catalog" {
        return Err("Not a catalog document".into());
    }
    if catalog.schema_version != SCHEMA_VERSION {
        return Err("Unsupported extension catalog version".into());
    }
    if catalog.version == 0 {
        return Err("A catalog's version starts at 1".into());
    }
    parse_expires(&catalog.expires)?;
    if catalog.publishers.len() > MAX_PUBLISHERS {
        return Err(format!(
            "A catalog delegates at most {MAX_PUBLISHERS} publishers"
        ));
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
        if let Some(release) = &entry.release.package {
            let url = https_url(&release.url)?;
            if url.host_str() != Some("github.com")
                || !allowed_download(&url)
                || !url.path().ends_with(".srelens-extension")
            {
                return Err(
                    "Catalog packages must be GitHub release assets named *.srelens-extension"
                        .into(),
                );
            }
            if !hex(&release.sha256, 64) {
                return Err("Invalid catalog package checksum".into());
            }
        }
        semver::Version::parse(&entry.release.version).map_err(|_| "Invalid release version")?;
        semver::VersionReq::parse(&entry.release.srelens_api_version)
            .map_err(|_| "Invalid extension API range")?;
    }
    Ok(catalog)
}
/// A signed catalog, verified: signed by the catalog role, its document valid, and each of
/// its publishers delegated by that role, with no two sharing a namespace. Expiry is the
/// caller's to judge: a download must be current, and the cache may be served as stale.
fn verify_catalog(envelope: &Envelope, trust: &TrustRoot) -> Result<Verified, String> {
    let payload = trust
        .open_catalog_signed(envelope, CATALOG_TYPE, MAX_CATALOG)
        .map_err(|e| format!("The catalog's signature did not verify: {e}"))?;
    let catalog = parse_catalog(&payload)?;
    let publishers = catalog
        .publishers
        .iter()
        .map(|delegation| trust.publisher(delegation))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Verified {
        delegations: Delegations::new(publishers)?,
        expires_at: parse_expires(&catalog.expires)?,
        digest: format!("{:x}", Sha256::digest(&payload)),
        catalog,
    })
}
/// A downloaded catalog, as this host will accept it: signed, current, and no older than
/// `cached`, the one it last verified. The same version must be the same catalog, so a
/// signer's mistake cannot give two hosts two catalogs under one number.
fn accept(
    raw: &[u8],
    trust: &TrustRoot,
    cached: Option<&Verified>,
) -> Result<(Envelope, Verified), String> {
    if raw.len() > MAX_SIGNED_CATALOG {
        return Err("Signed catalog exceeds 2 MiB".into());
    }
    let envelope: Envelope = serde_json::from_slice(raw)
        .map_err(|e| format!("Refusing an unsigned or malformed catalog: {e}"))?;
    let verified = verify_catalog(&envelope, trust)?;
    if verified.expired() {
        return Err(format!(
            "Refusing catalog version {}: it expired on {}",
            verified.catalog.version, verified.catalog.expires
        ));
    }
    if let Some(cached) = cached {
        if verified.catalog.version < cached.catalog.version {
            return Err(format!(
                "Refusing catalog version {}: this host has already verified version {}, and a catalog never goes back",
                verified.catalog.version, cached.catalog.version
            ));
        }
        if verified.catalog.version == cached.catalog.version && verified.digest != cached.digest {
            return Err(format!(
                "Refusing catalog version {}: it differs from the version {} this host already verified",
                verified.catalog.version, cached.catalog.version
            ));
        }
    }
    Ok((envelope, verified))
}
/// [`accept`] with nothing verified before it, for the fuzz targets: the envelope it
/// accepted, or why not.
#[cfg(any(test, feature = "fuzzing"))]
pub(super) fn fuzz_accept(raw: &[u8], trust: &TrustRoot) -> Result<Envelope, String> {
    accept(raw, trust, None).map(|(envelope, _)| envelope)
}
fn download(url: &str, limit: usize) -> Result<Vec<u8>, String> {
    let url = https_url(url)?;
    if url.as_str() != CATALOG_URL && !allowed_download(&url) {
        return Err("Unsupported extension download URL".into());
    }
    // Catalog browsing may be the first network operation, before any kube
    // client exists.
    http_policy::install_crypto_provider();
    let client = reqwest::blocking::Client::builder()
        .timeout(http_policy::TIMEOUT)
        .connect_timeout(http_policy::CONNECT_TIMEOUT)
        .user_agent("srelens-extension-catalog")
        .redirect(http_policy::redirects(|url| {
            allowed_download(url)
                .then_some(())
                .ok_or("Unsupported extension download redirect")
        }))
        .build()
        .map_err(|e| e.to_string())?;
    let response = client
        .get(url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("Download extension catalog/manifest: {e}"))?;
    read_download(response, limit)
}
/// A download's body: at most `limit` bytes, or which of the two things went wrong.
fn read_download(body: impl std::io::Read, limit: usize) -> Result<Vec<u8>, String> {
    http_policy::read_limited_blocking(body, limit).map_err(|error| match error {
        http_policy::BlockingBodyError::TooLarge => "Extension download exceeds size limit".into(),
        http_policy::BlockingBodyError::Read(error) => {
            format!("Download extension catalog/manifest: {error}")
        }
    })
}
/// The cached catalog, verified again, and when it was fetched; `None` when there is no
/// cache or it cannot be trusted. A cache written before #559 holds an unsigned catalog,
/// so it is never trusted and the signed one is fetched.
fn read_cache(path: &Path, trust: &TrustRoot) -> Option<(Verified, u64)> {
    let f = fs::File::open(path).ok()?;
    let mut raw = Vec::new();
    f.take((MAX_SIGNED_CATALOG + 65536) as u64 + 1)
        .read_to_end(&mut raw)
        .ok()?;
    let cached: Cached = serde_json::from_slice(&raw).ok()?;
    let verified = verify_catalog(&cached.signed_catalog, trust).ok()?;
    Some((verified, cached.fetched_at))
}
/// Fetched less than a day ago, and not in the future.
fn is_fresh(fetched_at: u64) -> bool {
    fetched_at <= now() && now() - fetched_at < TTL
}
impl Loaded {
    fn snapshot(&self) -> Snapshot {
        let catalog = &self.verified.catalog;
        Snapshot {
            incompatible: catalog
                .extensions
                .iter()
                .filter(|e| !compatible(&e.release.srelens_api_version))
                .map(|e| e.id.clone())
                .collect(),
            catalog: CatalogView {
                schema_version: catalog.schema_version,
                version: catalog.version,
                expires: catalog.expires.clone(),
                publishers: self
                    .verified
                    .delegations
                    .publishers()
                    .iter()
                    .map(|publisher| PublisherView {
                        id: publisher.id.clone(),
                        name: publisher.name.clone(),
                        namespaces: publisher.namespaces.clone(),
                    })
                    .collect(),
                extensions: catalog.extensions.clone(),
            },
            fetched_at: self.fetched_at,
            stale: self.stale,
            error: self.error.clone(),
            host_api_version: newest_api_version(),
            host_api_versions: host_api_versions(),
        }
    }
}
fn load_with(
    path: &Path,
    trust: &TrustRoot,
    refresh: bool,
    fetch: impl FnOnce() -> Result<Vec<u8>, String>,
) -> Result<Loaded, String> {
    // Serialize refreshes across windows/processes and atomically replace validated caches.
    let _lock = super::super::settings::write_lock(path)?;
    let cached = read_cache(path, trust);
    if let Some((verified, fetched_at)) = &cached {
        // An expired catalog is never current, however recently it was fetched.
        if !refresh && is_fresh(*fetched_at) && !verified.expired() {
            let (verified, fetched_at) = cached.unwrap();
            return Ok(Loaded {
                verified,
                fetched_at,
                stale: false,
                error: None,
            });
        }
    }
    let result = fetch().and_then(|raw| accept(&raw, trust, cached.as_ref().map(|(v, _)| v)));
    match result {
        Ok((envelope, verified)) => save_cache(path, envelope, verified),
        // A refused or failed download keeps the last verified catalog.
        Err(error) => match cached {
            Some((verified, fetched_at)) => {
                let error = if verified.expired() {
                    format!(
                        "{error}. The cached catalog expired on {}",
                        verified.catalog.expires
                    )
                } else {
                    error
                };
                Ok(Loaded {
                    verified,
                    fetched_at,
                    stale: true,
                    error: Some(error),
                })
            }
            None => Err(error),
        },
    }
}
/// Replace the cache with a catalog verified now. The caller holds the cache's lock.
fn save_cache(path: &Path, envelope: Envelope, verified: Verified) -> Result<Loaded, String> {
    let fetched_at = now();
    let raw = serde_json::to_vec(&Cached {
        signed_catalog: envelope,
        fetched_at,
    })
    .map_err(|e| e.to_string())?;
    crate::durable::replace(path, &raw).map_err(|e| format!("Save extension catalog: {e}"))?;
    Ok(Loaded {
        verified,
        fetched_at,
        stale: false,
        error: None,
    })
}
fn load(path: &Path, trust: &TrustRoot, refresh: bool) -> Result<Loaded, String> {
    load_with(path, trust, refresh, || {
        download(CATALOG_URL, MAX_SIGNED_CATALOG)
    })
}
/// The catalog cache a registry's apps read, and the root it is verified against.
#[derive(Clone)]
pub(super) enum CatalogCache {
    /// This host's own cache file (the desktop). `extensions.catalog` fetches the catalog
    /// into it when it is a day old, has expired, or the reader asks.
    Owned { path: PathBuf, trust: TrustRoot },
    /// The cache every user of one server shares. Capabilities only read it.
    Shared(SharedCatalog),
}
impl CatalogCache {
    fn path(&self) -> &Path {
        match self {
            Self::Owned { path, .. } => path,
            Self::Shared(shared) => &shared.path,
        }
    }
    pub(super) fn trust(&self) -> &TrustRoot {
        match self {
            Self::Owned { trust, .. } => trust,
            Self::Shared(shared) => &shared.trust,
        }
    }
    /// The catalog as this cache has it; an owned cache is fetched first when it is a day
    /// old, has expired, or `refresh` asks. The shared cache never is: it is what the
    /// server last fetched.
    fn load(&self, refresh: bool) -> Result<Loaded, String> {
        match self {
            Self::Owned { path, trust } => load(path, trust, refresh),
            Self::Shared(shared) => shared.read(),
        }
    }
    /// The last verified catalog, fresh or stale. Never fetches.
    fn cached(&self) -> Option<Loaded> {
        load_with(self.path(), self.trust(), false, || {
            Err("the cached catalog is read without fetching".into())
        })
        .ok()
    }
    /// Whether the cached catalog, fresh or stale, lists this exact release. Never fetches.
    pub(super) fn lists_release(&self, id: &str, sha256: &str) -> bool {
        self.cached_entry(|entry| entry.id == id && entry.release.sha256 == sha256)
    }
    /// Whether the cached catalog, fresh or stale, lists this exact package (#562). Never
    /// fetches.
    pub(super) fn lists_package(&self, id: &str, sha256: &str) -> bool {
        self.cached_entry(|entry| {
            entry.id == id
                && entry
                    .release
                    .package
                    .as_ref()
                    .is_some_and(|package| package.sha256 == sha256)
        })
    }
    /// Whether the cached catalog, fresh or stale, has an entry `matches`. Never fetches.
    fn cached_entry(&self, matches: impl Fn(&Entry) -> bool) -> bool {
        self.cached()
            .is_some_and(|loaded| loaded.verified.catalog.extensions.iter().any(matches))
    }
    /// The package of the release `sha256` names, downloaded and verified as a review
    /// verifies it, and exactly the package `package_sha256` names: what was reviewed.
    pub(super) fn download_package(
        &self,
        id: &str,
        sha256: &str,
        package_sha256: &str,
    ) -> Result<Vec<u8>, String> {
        let loaded = self.load(false)?;
        current(&loaded)?;
        let entry = release(&loaded.verified.catalog, id, sha256)?;
        if entry.release.package.as_ref().map(|p| p.sha256.as_str()) != Some(package_sha256) {
            return Err(CHANGED.into());
        }
        let delegations =
            Delegations::merged(&loaded.verified.delegations, &self.trust().shipped());
        fetch_package(entry, &delegations).map(|(_, archive)| archive)
    }
    /// Who may sign what, as an install checks it. Never fetches. Refused when the root
    /// itself is unavailable, with why: then nothing verifies, and which IDs are reserved
    /// is unknown, so nothing can be installed safely.
    pub(super) fn authority(&self) -> Result<Authority, String> {
        if let Some(reason) = self.trust().unavailable() {
            return Err(reason);
        }
        let shipped = self.trust().shipped();
        let Some(loaded) = self.cached() else {
            return Ok(Authority {
                signers: shipped.clone(),
                reserved: shipped,
                expired: None,
            });
        };
        let known = Delegations::merged(&loaded.verified.delegations, &shipped);
        // An expired catalog still says which namespaces are taken, but vouches for no key:
        // a host kept from newer catalogs must not go on trusting a key they withdrew.
        Ok(if loaded.verified.expired() {
            Authority {
                signers: shipped,
                reserved: known,
                expired: Some(format!(
                    "the catalog expired on {} and could not be refreshed, so only the publishers this build ships are trusted until it is",
                    loaded.verified.catalog.expires
                )),
            }
        } else {
            Authority {
                signers: known.clone(),
                reserved: known,
                expired: None,
            }
        })
    }
}
/// Why an install or review is refused when the catalog no longer lists what was asked for.
const CHANGED: &str = "Catalog release changed; refresh and review it again";
/// The release `sha256` names for `id`, if this host can install it.
fn release<'a>(catalog: &'a Catalog, id: &str, sha256: &str) -> Result<&'a Entry, String> {
    let entry = catalog
        .extensions
        .iter()
        .find(|e| e.id == id && e.release.sha256 == sha256)
        .ok_or(CHANGED)?;
    if !compatible(&entry.release.srelens_api_version) {
        return Err("Extension requires a different host API version".to_string());
    }
    Ok(entry)
}
/// Freshness is the point of an expiry: a host that cannot get a current catalog does not
/// install from an old one (TUF's freeze protection).
fn current(loaded: &Loaded) -> Result<(), String> {
    if loaded.verified.expired() {
        return Err(format!(
            "The catalog expired on {} and could not be refreshed; refresh it before installing from it",
            loaded.verified.catalog.expires
        ));
    }
    Ok(())
}
/// Who may sign what, as an install checks it (#559).
#[derive(Clone, Debug)]
pub(super) struct Authority {
    /// The delegations a signature is verified under: the cached catalog's while it is
    /// current, and this build's shipped ones for any namespace it does not delegate.
    pub(super) signers: Delegations,
    /// The namespaces an unsigned app may not take: every delegation the host knows of, an
    /// expired catalog's included.
    pub(super) reserved: Delegations,
    /// When the cached catalog has expired, why its delegations vouch for no key.
    pub(super) expired: Option<String>,
}
/// The catalog cache every user of one web server shares (#515).
///
/// The catalog is not anyone's: it is the fixed public catalog, the same for every user.
/// What is per user is the inventory, and every install from this cache downloads and
/// verifies its release again for the user installing it. So one cache serves them all,
/// and nothing a user sends can write it: their `extensions.catalog` and
/// `extensions.catalogManifest` read it and never fetch into it, and only the server, on
/// its own schedule through [`SharedCatalog::refresh_if_stale`], replaces it — with what it
/// downloaded from the fixed catalog URL and verified, exactly as a desktop refresh does.
#[derive(Clone)]
pub struct SharedCatalog {
    path: PathBuf,
    trust: TrustRoot,
    /// Why the server's last refresh failed, if it did, for readers of a stale cache.
    last_error: Arc<Mutex<Option<String>>>,
}
impl SharedCatalog {
    /// The shared cache kept at `path`, verified against the root this build pins. Nothing
    /// is read or fetched until it is used.
    pub fn new(path: PathBuf) -> Self {
        Self::with_trust(path, TrustRoot::pinned())
    }
    /// [`SharedCatalog::new`], verified against `trust` instead: a test's root.
    pub fn with_trust(path: PathBuf, trust: TrustRoot) -> Self {
        Self {
            path,
            trust,
            last_error: Arc::default(),
        }
    }
    /// Fetch the catalog into the cache when it is missing, a day old or expired, as a
    /// desktop does when its catalog is opened. For the server's own schedule: no
    /// capability calls it. A failed or refused fetch keeps the cache as it was, and says
    /// why.
    pub fn refresh_if_stale(&self) -> Result<(), String> {
        self.refresh_if_stale_with(|| download(CATALOG_URL, MAX_SIGNED_CATALOG))
    }
    /// [`SharedCatalog::refresh_if_stale`], with the fetch supplied. What it returns is
    /// verified as a downloaded catalog is.
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
        let current = || {
            read_cache(&self.path, &self.trust)
                .filter(|(verified, fetched_at)| is_fresh(*fetched_at) && !verified.expired())
                .is_some()
        };
        {
            let _lock = super::super::settings::write_lock(&self.path)?;
            if current() {
                return Ok(());
            }
        }
        let raw = fetch()?;
        let _lock = super::super::settings::write_lock(&self.path)?;
        // Another refresh may have saved a copy while this one downloaded; it is at least
        // as new, so it stays.
        if current() {
            return Ok(());
        }
        // Checked against the cache as it is now, under the lock it is saved under.
        let cached = read_cache(&self.path, &self.trust);
        let (envelope, verified) = accept(&raw, &self.trust, cached.as_ref().map(|(v, _)| v))?;
        save_cache(&self.path, envelope, verified).map(|_| ())
    }
    /// What a user reads: the cache as the server last fetched it. Never fetches and never
    /// writes it; a cache past its day, or past its expiry, is returned as stale, with why
    /// it was not refreshed.
    fn read(&self) -> Result<Loaded, String> {
        let why = match self.last_error.lock().unwrap().clone() {
            Some(error) => format!("the server could not refresh the shared catalog: {error}"),
            None => "the server has not refreshed the shared catalog yet".to_owned(),
        };
        load_with(&self.path, &self.trust, false, || Err(why))
    }
}
/// A cache file holding a catalog of `extensions` that delegates `publishers`, signed by
/// the test catalog key, as fetched at `fetched_at`: for tests elsewhere that need one.
#[cfg(test)]
pub(super) fn test_cache(publishers: &[Envelope], extensions: Value, fetched_at: u64) -> Vec<u8> {
    test_cache_expiring(publishers, extensions, fetched_at, "2100-01-01T00:00:00Z")
}
/// [`test_cache`], expiring at `expires`.
#[cfg(test)]
pub(super) fn test_cache_expiring(
    publishers: &[Envelope],
    extensions: Value,
    fetched_at: u64,
    expires: &str,
) -> Vec<u8> {
    use super::trust::testing;
    let document = json!({
        "_type": "catalog",
        "schemaVersion": SCHEMA_VERSION,
        "version": 1,
        "expires": expires,
        "publishers": publishers,
        "extensions": extensions,
    });
    serde_json::to_vec(&Cached {
        signed_catalog: testing::sign_json(
            CATALOG_TYPE,
            &document,
            &[&testing::key(testing::CATALOG_SEED)],
        ),
        fetched_at,
    })
    .unwrap()
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
/// Where the signature of a release in a delegated namespace is published: beside its
/// manifest. A release no publisher is delegated is unsigned, and none is fetched.
fn signature_url(entry: &Entry, delegations: &Delegations) -> Option<String> {
    signing::reserved(&entry.id, delegations).then(|| format!("{}.sig", entry.release.manifest_url))
}
fn verify_release(
    entry: &Entry,
    raw: &[u8],
    signature: Option<ReleaseSignature>,
    delegations: &Delegations,
) -> Result<Review, String> {
    let manifest = verify_manifest(entry, raw)?;
    let signed_by = match (delegations.owner(&entry.id), &signature) {
        (Some(_), Some(signature)) => Some(signing::verify(
            raw,
            &signature.bytes,
            signature.key_id.as_deref(),
            delegations,
        )?),
        (Some(publisher), None) => {
            return Err(format!(
                "{} is signed by {}, and this release's publisher signature is missing",
                entry.id, publisher.name
            ))
        }
        (None, Some(_)) => return Err("Unrecognized app publisher signature".into()),
        (None, None) => None,
    };
    Ok(Review {
        manifest,
        key_id: signature.as_ref().and_then(|s| s.key_id.clone()),
        signature: signature.map(|s| s.bytes),
        signed_by,
        package: None,
    })
}
/// A signed app's package is published beside its manifest, in the same release (#562):
/// the key that signed the release is the publisher's, and so is the release it is in.
fn check_package_url(
    entry: &Entry,
    release: &ReleasePackage,
    delegations: &Delegations,
) -> Result<(), String> {
    if !signing::reserved(&entry.id, delegations) {
        return Ok(());
    }
    let directory = entry
        .release
        .manifest_url
        .rsplit_once('/')
        .map(|(directory, _)| format!("{directory}/"))
        .unwrap_or_default();
    let name = release.url.strip_prefix(&directory).unwrap_or_default();
    if name.is_empty() || name.contains('/') || !name.ends_with(".srelens-extension") {
        return Err(
            "A signed app's package must be published beside its manifest, in the same release"
                .into(),
        );
    }
    Ok(())
}
/// The package `archive` is the release: the checksum the catalog gives, and a manifest
/// that is the release's own (the exact bytes `manifestUrl` serves, which the catalog's
/// `sha256` names). A package in a delegated namespace is signed by that publisher; any
/// other's signature is refused, as a single-file release's is.
fn verify_package(
    entry: &Entry,
    archive: &[u8],
    delegations: &Delegations,
) -> Result<package::Package, String> {
    let release = entry
        .release
        .package
        .as_ref()
        .ok_or("This catalog release has no package")?;
    check_package_url(entry, release, delegations)?;
    if package::sha256_hex(archive) != release.sha256 {
        return Err("Extension package checksum does not match the catalog".into());
    }
    let verified = package::read(archive, &mut package::Discard, delegations)?;
    package::check_installable(&verified)?;
    verify_manifest(entry, verified.manifest.as_bytes())?;
    match (delegations.owner(&entry.id), &verified.signature) {
        (Some(publisher), None) => Err(format!(
            "{} is signed by {}, and this release's package is not signed",
            entry.id, publisher.name
        )),
        (None, Some(_)) => Err("Unrecognized app publisher signature".into()),
        _ => Ok(verified),
    }
}
/// A catalog release's package, downloaded and verified.
fn fetch_package(
    entry: &Entry,
    delegations: &Delegations,
) -> Result<(package::Package, Vec<u8>), String> {
    let release = entry
        .release
        .package
        .as_ref()
        .ok_or("This catalog release has no package")?;
    check_package_url(entry, release, delegations)?;
    let archive = download(&release.url, package::MAX_PACKAGE_BYTES)?;
    verify_package(entry, &archive, delegations).map(|verified| (verified, archive))
}
/// The catalog's capabilities. `packages` says whether this host installs packages
/// (#562): when it does, a release that has one is reviewed as its package.
pub(super) fn register(
    reg: &mut Registry,
    cache: CatalogCache,
    core: Arc<Registry>,
    packages: bool,
) {
    let c = cache.clone();
    reg.register(Capability::typed::<ListIn, Snapshot, _, _>(
        "extensions.catalog",
        "Browse the native extension catalog with a durable cache; never connects clusters",
        Annotations::READ_ONLY,
        move |input| {
            let cache = c.clone();
            async move {
                tokio::task::spawn_blocking(move || cache.load(input.refresh).map(|l| l.snapshot()))
                    .await
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?
                    .map_err(CapabilityError::Handler)
            }
        },
    ));
    reg.register(Capability::typed::<ManifestIn, Review, _, _>("extensions.catalogManifest", "Download and checksum-verify a catalog manifest for permission review; does not install it", Annotations::READ_ONLY, move |input| {
        let cache = cache.clone(); let core = core.clone();
        async move { tokio::task::spawn_blocking(move || {
            let loaded = cache.load(false)?;
            current(&loaded)?;
            let entry = release(&loaded.verified.catalog, &input.id, &input.sha256)?;
            let delegations = Delegations::merged(&loaded.verified.delegations, &cache.trust().shipped());
            let review = if packages && entry.release.package.is_some() {
                Review::of_package(&fetch_package(entry, &delegations)?.0)
            } else {
                let signature = signature_url(entry, &delegations)
                    .map(|url| download(&url, MAX_RELEASE_SIGNATURE))
                    .transpose()?
                    .map(|raw| signing::parse_release_signature(&raw))
                    .transpose()?;
                verify_release(entry, &download(&entry.release.manifest_url, MAX_MANIFEST_BYTES)?, signature, &delegations)?
            };
            let manifest = &review.manifest;
            let parsed = Manifest::parse(&manifest)?;
            super::validate_app(&parsed, &parsed.permission_names(), core)?;
            Ok(review)
        }).await.map_err(|e| CapabilityError::Handler(e.to_string()))?.map_err(CapabilityError::Handler) }
    }));
}
#[cfg(test)]
mod tests {
    use super::super::trust::testing;
    use super::*;
    use ring::signature::Ed25519KeyPair;

    fn trust() -> TrustRoot {
        testing::root()
    }
    /// The signed fixture: the unsigned fixture's two entries, as version 1, expiring in
    /// 2100, delegating srelens and Example Labs (`tests/fixtures/trust/generate.sh`).
    fn fixture() -> Vec<u8> {
        include_bytes!("../../tests/fixtures/extension-catalog.signed.json").to_vec()
    }
    /// The fixture's document at another version and expiry, with the entries of `source`.
    fn document(source: &[u8], version: u64, expires: &str) -> Value {
        let source: Value = serde_json::from_slice(source).unwrap();
        json!({
            "_type": "catalog",
            "schemaVersion": 2,
            "version": version,
            "expires": expires,
            "publishers": [testing::srelens_publisher(), testing::example_publisher()],
            "extensions": source["extensions"],
        })
    }
    fn unsigned() -> &'static [u8] {
        include_bytes!("../../tests/fixtures/extension-catalog.json")
    }
    fn signed_by(document: &Value, keys: &[&Ed25519KeyPair]) -> Vec<u8> {
        serde_json::to_vec(&testing::sign_json(CATALOG_TYPE, document, keys)).unwrap()
    }
    fn signed(document: &Value) -> Vec<u8> {
        signed_by(document, &[&testing::key(testing::CATALOG_SEED)])
    }
    fn at(version: u64) -> Vec<u8> {
        signed(&document(unsigned(), version, "2100-01-01T00:00:00Z"))
    }
    /// The fixture's document, as parse_catalog reads it.
    fn payload() -> Vec<u8> {
        serde_json::to_vec(&document(unsigned(), 1, "2100-01-01T00:00:00Z")).unwrap()
    }
    fn first_entry() -> Entry {
        parse_catalog(&payload()).unwrap().extensions.remove(0)
    }
    fn delegations() -> Delegations {
        let trust = trust();
        let example = trust.publisher(&testing::example_publisher()).unwrap();
        Delegations::merged(&Delegations::new(vec![example]).unwrap(), &trust.shipped())
    }
    fn owned(path: &Path) -> CatalogCache {
        CatalogCache::Owned {
            path: path.to_path_buf(),
            trust: trust(),
        }
    }
    fn raw_signature(bytes: &[u8]) -> Option<ReleaseSignature> {
        Some(signing::parse_release_signature(bytes).unwrap())
    }
    /// A cache file whose fetch time is `fetched_at`, as a host writes it.
    fn age(path: &Path, fetched_at: u64) {
        let mut cached: Cached = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        cached.fetched_at = fetched_at;
        fs::write(path, serde_json::to_vec(&cached).unwrap()).unwrap();
    }
    #[test]
    fn the_signed_fixture_verifies_under_the_test_root() {
        let loaded = load_with(
            &tempfile::tempdir().unwrap().path().join("c.json"),
            &trust(),
            true,
            || Ok(fixture()),
        )
        .unwrap();
        assert_eq!(loaded.verified.catalog.version, 1);
        assert_eq!(loaded.verified.catalog.extensions.len(), 2);
        let publishers: Vec<_> = loaded
            .verified
            .delegations
            .publishers()
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(publishers, ["srelens", "Example Labs"]);
    }
    #[test]
    fn a_cached_release_is_known_by_id_and_exact_checksum_without_fetching() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("apps.catalog.json");
        let cache = owned(&path);
        let entry = first_entry();
        assert!(
            !cache.lists_release(&entry.id, &entry.release.sha256),
            "no cache yet"
        );
        load_with(&path, &trust(), true, || Ok(fixture())).unwrap();
        assert!(cache.lists_release(&entry.id, &entry.release.sha256));
        assert!(!cache.lists_release(&entry.id, &"0".repeat(64)));
        assert!(!cache.lists_release("org.other.app", &entry.release.sha256));
    }
    #[test]
    fn validates_catalog_and_reports_api_compatibility() {
        let catalog = parse_catalog(&payload()).unwrap();
        assert_eq!(catalog.extensions.len(), 2);
        assert!(!compatible(&catalog.extensions[0].release.srelens_api_version));
        assert!(compatible("^0.3"));
        assert!(compatible("^0.4"));
        assert!(!compatible("^0.2"));
        assert!(!compatible("^99"));
        let mut value: Value = serde_json::from_slice(&payload()).unwrap();
        value["extensions"][1] = value["extensions"][0].clone();
        assert!(parse_catalog(&serde_json::to_vec(&value).unwrap()).is_err());
        for (field, bad) in [
            ("schemaVersion", json!(3)),
            ("schemaVersion", json!(1)),
            ("version", json!(0)),
            ("expires", json!("next tuesday")),
            ("_type", json!("root")),
        ] {
            let mut value: Value = serde_json::from_slice(&payload()).unwrap();
            value[field] = bad;
            assert!(
                parse_catalog(&serde_json::to_vec(&value).unwrap()).is_err(),
                "{field}"
            );
        }
        // The catalog of hosts before #559 is a different document, not a signed one.
        assert!(parse_catalog(unsigned()).is_err());
    }
    #[test]
    fn verifies_exact_bytes_identity_and_api_before_review() {
        let mut entry = first_entry();
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
    fn releases_in_a_delegated_namespace_require_their_publishers_signature() {
        let delegations = delegations();
        let mut entry = first_entry();
        let raw = include_bytes!("../../tests/fixtures/argocd-manifest.json");
        let sig = include_bytes!("../../tests/fixtures/argocd-manifest.sig");
        entry.release.sha256 = format!("{:x}", Sha256::digest(raw));
        assert!(signing::verify_for(&entry.id, raw, sig, None, &delegations).is_ok());
        assert_eq!(
            signature_url(&entry, &delegations),
            Some(format!("{}.sig", entry.release.manifest_url))
        );
        assert!(
            verify_release(&entry, raw, raw_signature(sig), &delegations)
                .unwrap_err()
                .contains("requires API ^0.1")
        );
        // A current manifest still requires a signature. Reusing the old signature after
        // upgrading its API range is tampering, never a newly signed release.
        let current = std::str::from_utf8(raw).unwrap().replace("^0.1", "^0.3");
        let raw = current.as_bytes();
        entry.release.srelens_api_version = "^0.3".into();
        entry.release.sha256 = format!("{:x}", Sha256::digest(raw));
        assert!(
            verify_release(&entry, raw, raw_signature(sig), &delegations)
                .unwrap_err()
                .contains("signature")
        );
        assert!(verify_release(&entry, raw, None, &delegations)
            .unwrap_err()
            .contains("missing"));
        let mut changed = raw.to_vec();
        changed.push(b' ');
        entry.release.sha256 = format!("{:x}", Sha256::digest(&changed));
        assert!(
            verify_release(&entry, &changed, raw_signature(sig), &delegations)
                .unwrap_err()
                .contains("signature")
        );
        // Lookalike prefixes are ordinary, unsigned third-party entries.
        entry.id = "org.srelensx.argocd".into();
        assert_eq!(signature_url(&entry, &delegations), None);
    }
    /// #559 removed the repository heuristic: a repository URL neither makes an entry
    /// official nor excuses one from its publisher's signature.
    #[test]
    fn the_repository_does_not_decide_who_signs_a_release() {
        let delegations = delegations();
        let mut entry = first_entry();
        for repository in [
            "https://github.com/srelens/extension-argocd",
            "https://github.com/SRELENS/Extension-ArgoCD",
            "https://github.com/attacker/extension-argocd",
        ] {
            entry.repository = repository.into();
            entry.id = "org.other.argocd".into();
            assert_eq!(signature_url(&entry, &delegations), None, "{repository}");
            entry.id = "org.srelens.argocd".into();
            assert!(
                signature_url(&entry, &delegations).is_some(),
                "{repository}"
            );
        }
    }
    #[test]
    fn a_third_party_publisher_signs_its_own_catalog_releases() {
        let delegations = delegations();
        let key = testing::key(testing::EXAMPLE_SEED);
        let source = super::super::tests::manifest().replacen(
            "org.example.argocd",
            "com.example-labs.gitops",
            1,
        );
        let raw = source.as_bytes();
        let mut entry = first_entry();
        entry.id = "com.example-labs.gitops".into();
        entry.release.version = Manifest::parse(&source).unwrap().version;
        entry.release.srelens_api_version = "^0.4".into();
        entry.release.sha256 = format!("{:x}", Sha256::digest(raw));
        assert!(signature_url(&entry, &delegations).is_some());
        let keyed = ReleaseSignature {
            key_id: Some(testing::id(&key)),
            bytes: key.sign(raw).as_ref().to_vec(),
        };
        let review = verify_release(&entry, raw, Some(keyed.clone()), &delegations).unwrap();
        assert_eq!(review.signed_by.unwrap().name, "Example Labs");
        assert_eq!(review.key_id, keyed.key_id);
        // The srelens key cannot stand in for it, and neither can a missing signature.
        let srelens_sig = ReleaseSignature {
            key_id: Some("ab".repeat(32)),
            bytes: keyed.bytes.clone(),
        };
        assert!(verify_release(&entry, raw, Some(srelens_sig), &delegations)
            .unwrap_err()
            .contains("signed only by Example Labs"));
        assert!(verify_release(&entry, raw, None, &delegations)
            .unwrap_err()
            .contains("missing"));
    }
    #[test]
    fn verify_manifest_rejects_oversized_and_invalid_encoding() {
        let entry = first_entry();
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
        let mut entry = first_entry();
        entry.id = "org.thirdparty.app".into();
        entry.release.srelens_api_version = "^0.3".into();
        entry.repository = "https://github.com/thirdparty/app".into();
        entry.release.sha256 = format!("{:x}", Sha256::digest(&raw));
        assert!(
            verify_release(&entry, &raw, raw_signature(&[1; 64]), &delegations())
                .unwrap_err()
                .contains("Unrecognized")
        );
    }
    #[tokio::test]
    async fn catalog_capabilities_registered_and_invoked() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("catalog.json");
        // Seed the cache in the shape `load` writes. A cache that fails its verification
        // is fetched again, so the capability would test the live catalog instead (#615).
        let seeded = load_with(&path, &trust(), false, || Ok(fixture())).unwrap();

        let mut reg = Registry::new();
        let core = Arc::new(Registry::new());
        register(&mut reg, owned(&path), core, true);

        let cap_list = reg.get("extensions.catalog").unwrap();
        let list_res = (cap_list.handler)(serde_json::json!({"refresh": false}))
            .await
            .unwrap();
        // Served from the seeded cache, not a fresh download.
        assert_eq!(list_res["fetchedAt"], json!(seeded.fetched_at));
        assert_eq!(list_res["catalog"]["version"], json!(1));
        assert_eq!(
            list_res["catalog"]["publishers"][1]["name"],
            json!("Example Labs")
        );

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
        let shared = SharedCatalog::with_trust(path.clone(), trust());
        let mut reg = Registry::new();
        register(
            &mut reg,
            CatalogCache::Shared(shared.clone()),
            Arc::new(Registry::new()),
            false,
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
        let entry = first_entry();
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
        age(&path, 0);
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
        // The server refuses a rolled-back catalog as a desktop does, and keeps its copy.
        let mut older = document(unsigned(), 1, "2100-01-01T00:00:00Z");
        older["extensions"].as_array_mut().unwrap().truncate(1);
        let refused = shared
            .refresh_if_stale_with(|| Ok(signed(&older)))
            .unwrap_err();
        assert!(refused.contains("differs from the version 1"), "{refused}");
        assert_eq!(fs::read(&path).unwrap(), aged);
    }
    /// A shared cache a day old, so the next server refresh downloads.
    fn aged_shared_catalog(dir: &Path) -> SharedCatalog {
        let shared = SharedCatalog::with_trust(dir.join("extensions.catalog.json"), trust());
        shared.refresh_if_stale_with(|| Ok(fixture())).unwrap();
        age(&shared.path, 0);
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
                    let _ = tx.send(reader.read().map(|loaded| loaded.stale));
                });
                let read = rx
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .expect("a read waited for the server's download");
                assert!(read.unwrap(), "the cached copy, as it was");
                Ok(at(2))
            })
            .unwrap();
        let (verified, fetched_at) = read_cache(&shared.path, &trust()).unwrap();
        assert!(is_fresh(fetched_at));
        assert_eq!(verified.catalog.version, 2);
    }
    /// A copy another refresh saved while this one downloaded is at least as new, so
    /// this one does not replace it.
    #[test]
    fn a_refresh_does_not_replace_a_copy_saved_while_it_downloaded() {
        let dir = tempfile::tempdir().unwrap();
        let shared = aged_shared_catalog(dir.path());
        let mut one = document(unsigned(), 3, "2100-01-01T00:00:00Z");
        one["extensions"].as_array_mut().unwrap().truncate(1);
        shared
            .refresh_if_stale_with(|| {
                let _lock = super::super::super::settings::write_lock(&shared.path).unwrap();
                let (envelope, verified) = accept(&signed(&one), &shared.trust, None).unwrap();
                save_cache(&shared.path, envelope, verified).unwrap();
                Ok(at(2))
            })
            .unwrap();
        let (kept, _) = read_cache(&shared.path, &trust()).unwrap();
        assert_eq!(kept.catalog.extensions.len(), 1);
        assert_eq!(kept.catalog.version, 3);
    }
    /// A catalog entry for the `signed` fixture package: an app of the test publisher,
    /// which the test root delegates `test.signed`, with its package beside its manifest.
    fn packaged_entry(archive: &[u8]) -> Entry {
        let manifest =
            std::fs::read(super::super::package::tests::fixture("signed").join(package::MANIFEST))
                .unwrap();
        let mut value: Value = serde_json::from_slice(&payload()).unwrap();
        let entry = &mut value["extensions"][0];
        let repository = "https://github.com/srelens-test-publisher/extension-packaged";
        entry["id"] = json!("test.signed.packaged");
        entry["repository"] = json!(repository);
        entry["release"]["version"] = json!("1.0.0");
        entry["release"]["srelensApiVersion"] = json!("^0.4");
        entry["release"]["manifestUrl"] = json!(format!(
            "{repository}/releases/download/v1.0.0/manifest.json"
        ));
        entry["release"]["sha256"] = json!(package::sha256_hex(&manifest));
        entry["release"]["package"] = json!({
            "url": format!("{repository}/releases/download/v1.0.0/packaged.srelens-extension"),
            "sha256": package::sha256_hex(archive),
        });
        parse_catalog(&serde_json::to_vec(&value).unwrap())
            .unwrap()
            .extensions
            .remove(0)
    }
    #[test]
    fn a_catalog_package_is_verified_as_the_release_it_is_listed_for() {
        let archive = super::super::package::tests::packed("signed");
        let entry = packaged_entry(&archive);
        let verified = verify_package(&entry, &archive, &delegations()).unwrap();
        assert!(verified.signature.is_some());
        let review = serde_json::to_value(Review::of_package(&verified)).unwrap();
        assert_eq!(review["manifest"], json!(verified.manifest));
        assert!(review["package"]["icon"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png"));

        // Other bytes than the catalog lists.
        let mut changed = archive.clone();
        changed.push(0);
        assert!(verify_package(&entry, &changed, &delegations())
            .unwrap_err()
            .contains("checksum"));
        // A package whose manifest is not the release's own, the one manifestUrl serves.
        let mut other = entry.clone();
        other.release.sha256 = "0".repeat(64);
        assert!(verify_package(&other, &archive, &delegations())
            .unwrap_err()
            .contains("checksum"));
        // A signed app's package is published beside its manifest, nowhere else.
        let mut other = entry.clone();
        other.release.package.as_mut().unwrap().url = "https://github.com/srelens-test-publisher/extension-packaged/releases/download/v0.9.0/packaged.srelens-extension".into();
        assert!(verify_package(&other, &archive, &delegations())
            .unwrap_err()
            .contains("beside its manifest"));
        let mut other = entry.clone();
        other.release.package.as_mut().unwrap().url = "https://github.com/attacker/extension-packaged/releases/download/v1.0.0/packaged.srelens-extension".into();
        assert!(verify_package(&other, &archive, &delegations())
            .unwrap_err()
            .contains("beside its manifest"));
        // And it is signed: the same files without the signature are refused.
        let unsigned = {
            use std::io::Read as _;
            let mut tar = Vec::new();
            flate2::read::GzDecoder::new(&archive[..])
                .read_to_end(&mut tar)
                .unwrap();
            let mut entries = tar::Archive::new(&tar[..]);
            let raw = entries.entries().unwrap().fold(
                super::super::package::tests::Raw::new(),
                |raw, entry| {
                    let mut entry = entry.unwrap();
                    let path = entry.path().unwrap().to_string_lossy().into_owned();
                    let mut data = Vec::new();
                    entry.read_to_end(&mut data).unwrap();
                    if path == package::SIGNATURE {
                        raw
                    } else {
                        raw.file(&path, &data)
                    }
                },
            );
            raw.gz()
        };
        let mut other = entry.clone();
        other.release.package.as_mut().unwrap().sha256 = package::sha256_hex(&unsigned);
        assert!(verify_package(&other, &unsigned, &delegations())
            .unwrap_err()
            .contains("not signed"));
        // A third party's package carries no signature this host could check.
        let example = super::super::package::tests::packed("example");
        let mut value: Value = serde_json::from_slice(&payload()).unwrap();
        let third = &mut value["extensions"][0];
        let manifest =
            std::fs::read(super::super::package::tests::fixture("example").join(package::MANIFEST))
                .unwrap();
        third["id"] = json!("org.example.packaged");
        third["repository"] = json!("https://github.com/example/packaged");
        third["release"]["version"] = json!("1.0.0");
        third["release"]["srelensApiVersion"] = json!("^0.4");
        third["release"]["manifestUrl"] =
            json!("https://github.com/example/packaged/releases/download/v1.0.0/manifest.json");
        third["release"]["sha256"] = json!(package::sha256_hex(&manifest));
        third["release"]["package"] = json!({
            "url": "https://github.com/example/packaged/releases/download/v1.0.0/app.srelens-extension",
            "sha256": package::sha256_hex(&example),
        });
        let third = parse_catalog(&serde_json::to_vec(&value).unwrap())
            .unwrap()
            .extensions
            .remove(0);
        assert!(verify_package(&third, &example, &delegations())
            .unwrap()
            .signature
            .is_none());
    }
    #[test]
    fn a_catalog_package_must_be_a_github_release_asset_with_a_checksum() {
        let base: Value = serde_json::from_slice(&payload()).unwrap();
        for (package, why) in [
            (
                json!({"url": "https://example.com/app.srelens-extension", "sha256": "0".repeat(64)}),
                "GitHub release assets",
            ),
            (
                json!({"url": "https://github.com/a/b/releases/download/v1/app.tar.gz", "sha256": "0".repeat(64)}),
                "GitHub release assets",
            ),
            (
                json!({"url": "http://github.com/a/b/releases/download/v1/app.srelens-extension", "sha256": "0".repeat(64)}),
                "HTTPS",
            ),
            (
                json!({"url": "https://github.com/a/b/releases/download/v1/app.srelens-extension", "sha256": "abc"}),
                "package checksum",
            ),
            (
                json!({"url": "https://github.com/a/b/releases/download/v1/app.srelens-extension"}),
                "Invalid extension catalog",
            ),
        ] {
            let mut value = base.clone();
            value["extensions"][0]["release"]["package"] = package.clone();
            let refused = parse_catalog(&serde_json::to_vec(&value).unwrap()).map(|_| ());
            assert!(
                refused.as_ref().is_err_and(|reason| reason.contains(why)),
                "{package}: {refused:?}"
            );
        }
        let mut value = base;
        value["extensions"][0]["release"]["package"] = json!({
            "url": "https://github.com/a/b/releases/download/v1/app.srelens-extension",
            "sha256": "0".repeat(64),
        });
        let catalog = parse_catalog(&serde_json::to_vec(&value).unwrap()).unwrap();
        // It survives the cache, which a host that predates packages reads without it.
        let cached = serde_json::to_value(&catalog).unwrap();
        assert!(cached["extensions"][0]["release"]["package"].is_object());
        assert!(
            serde_json::to_value(&parse_catalog(&payload()).unwrap()).unwrap()["extensions"][0]
                ["release"]
                .get("package")
                .is_none()
        );
    }
    #[test]
    fn a_package_install_is_refused_when_the_catalog_no_longer_lists_what_was_reviewed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("catalog.json");
        let archive = super::super::package::tests::packed("signed");
        let entry = packaged_entry(&archive);
        let mut value: Value = serde_json::from_slice(&payload()).unwrap();
        value["extensions"][0] = serde_json::to_value(&entry).unwrap();
        let catalog = signed(&value);
        load_with(&path, &trust(), true, || Ok(catalog.clone())).unwrap();
        let cache = owned(&path);
        assert!(cache.lists_package(&entry.id, &package::sha256_hex(&archive)));
        assert!(!cache.lists_package(&entry.id, &"0".repeat(64)));
        // Neither a release nor a package the catalog does not list is downloaded.
        for (sha256, package_sha256) in [
            ("0".repeat(64), package::sha256_hex(&archive)),
            (entry.release.sha256.clone(), "0".repeat(64)),
        ] {
            let refused = cache
                .download_package(&entry.id, &sha256, &package_sha256)
                .unwrap_err();
            assert_eq!(refused, CHANGED);
        }
    }
    #[test]
    fn additive_catalog_fields_do_not_break_released_hosts() {
        let mut value: Value = serde_json::from_slice(&payload()).unwrap();
        value["revoked"] = json!([{"id": "org.srelens.argocd", "versions": ["0.0.1"]}]);
        value["extensions"][0]["publisher"] = json!({"name": "srelens"});
        value["extensions"][0]["categories"] = json!(["gitops"]);
        value["extensions"][0]["release"]["signatureUrl"] = json!("https://example.invalid");
        value["extensions"][0]["testedHost"]["platform"] = json!("linux");
        let catalog = parse_catalog(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(catalog.extensions.len(), 2);
        // And through the signature: a signed catalog with fields this host does not know.
        let loaded = load_with(
            &tempfile::tempdir().unwrap().path().join("c.json"),
            &trust(),
            true,
            || Ok(signed(&value)),
        )
        .unwrap();
        assert_eq!(loaded.verified.catalog.extensions.len(), 2);
        value["extensions"][0]["release"]["sha256"] = json!("bad");
        assert!(parse_catalog(&serde_json::to_vec(&value).unwrap()).is_err());
    }
    #[test]
    fn a_download_that_fails_part_way_is_not_called_too_large() {
        let failed =
            read_download(super::super::http_policy::tests::FailingBody, 1024).unwrap_err();
        assert!(
            failed.starts_with("Download extension catalog/manifest: "),
            "{failed}"
        );
        assert!(failed.contains("timed out"), "{failed}");
        assert!(!failed.contains("size limit"), "{failed}");
        assert_eq!(
            read_download(&b"abcde"[..], 4).unwrap_err(),
            "Extension download exceeds size limit"
        );
        assert_eq!(read_download(&b"abcd"[..], 4).unwrap(), b"abcd");
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
        let first = load_with(&path, &trust(), false, || Ok(fixture())).unwrap();
        assert!(!first.stale);
        let cached = load_with(&path, &trust(), false, || {
            panic!("fresh cache should avoid network")
        })
        .unwrap();
        assert_eq!(first.fetched_at, cached.fetched_at);
        let stale = load_with(&path, &trust(), true, || Err("offline".into())).unwrap();
        assert!(stale.stale);
        assert!(stale.error.unwrap().contains("offline"));
        assert!(
            load_with(&dir.path().join("missing"), &trust(), false, || Err(
                "offline".into()
            ))
            .is_err()
        );
    }
    #[test]
    fn a_day_old_or_corrupt_cache_is_refetched_and_failed_refresh_does_not_overwrite_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        load_with(&path, &trust(), false, || Ok(fixture())).unwrap();
        age(&path, now() - TTL - 1);
        let before = fs::read(&path).unwrap();
        assert!(
            load_with(&path, &trust(), false, || Err("offline".into()))
                .unwrap()
                .stale
        );
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(
            !load_with(&path, &trust(), false, || Ok(fixture()))
                .unwrap()
                .stale
        );
        fs::write(&path, "invalid json").unwrap();
        assert!(load_with(&path, &trust(), false, || Err("offline".into())).is_err());
        assert_eq!(
            load_with(&path, &trust(), false, || Ok(fixture()))
                .unwrap()
                .verified
                .catalog
                .extensions
                .len(),
            2
        );
    }
    /// Acceptance (#559): a catalog older than the one this host verified is refused, and
    /// the verified one kept.
    #[test]
    fn a_rolled_back_catalog_is_refused_and_the_verified_one_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        load_with(&path, &trust(), true, || Ok(at(5))).unwrap();
        let kept = fs::read(&path).unwrap();
        // Validly signed, current, and older: what a network attacker replays.
        let refused = load_with(&path, &trust(), true, || Ok(at(4))).unwrap();
        assert!(refused.stale);
        let error = refused.error.unwrap();
        assert!(error.contains("already verified version 5"), "{error}");
        assert_eq!(refused.verified.catalog.version, 5);
        assert_eq!(fs::read(&path).unwrap(), kept);
        // The same version is the same catalog, or it is refused.
        let mut changed = document(unsigned(), 5, "2100-01-01T00:00:00Z");
        changed["extensions"].as_array_mut().unwrap().truncate(1);
        let refused = load_with(&path, &trust(), true, || Ok(signed(&changed))).unwrap();
        assert!(refused.error.unwrap().contains("differs"));
        // The same catalog again is accepted, and a newer one replaces it.
        assert!(
            !load_with(&path, &trust(), true, || Ok(at(5)))
                .unwrap()
                .stale
        );
        assert_eq!(
            load_with(&path, &trust(), true, || Ok(at(6)))
                .unwrap()
                .verified
                .catalog
                .version,
            6
        );
    }
    /// Acceptance (#559): an expired catalog is refused, and an expired cache is not
    /// installed from.
    #[tokio::test]
    async fn an_expired_catalog_is_refused_and_not_installed_from() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let expired = signed(&document(unsigned(), 9, "2020-01-01T00:00:00Z"));
        let refused = load_with(&path, &trust(), true, || Ok(expired.clone())).unwrap_err();
        assert!(
            refused.contains("expired on 2020-01-01T00:00:00Z"),
            "{refused}"
        );
        assert!(!path.exists());
        load_with(&path, &trust(), true, || Ok(at(1))).unwrap();
        let refused = load_with(&path, &trust(), true, || Ok(expired.clone())).unwrap();
        assert!(refused.stale && refused.error.unwrap().contains("expired"));
        assert_eq!(refused.verified.catalog.version, 1);

        // A cached catalog that has since expired: refetched however recently it was
        // fetched, served as stale when that fails, and refused for installs.
        let (envelope, verified) = {
            let short = signed(&document(unsigned(), 2, "2000-01-01T00:00:00Z"));
            let envelope: Envelope = serde_json::from_slice(&short).unwrap();
            let verified = verify_catalog(&envelope, &trust()).unwrap();
            (envelope, verified)
        };
        save_cache(&path, envelope, verified).unwrap();
        let stale = load_with(&path, &trust(), false, || Err("offline".into())).unwrap();
        assert!(stale.stale);
        let error = stale.error.unwrap();
        assert!(
            error.contains("offline") && error.contains("expired on 2000-01-01"),
            "{error}"
        );
        let mut reg = Registry::new();
        register(
            &mut reg,
            CatalogCache::Shared(SharedCatalog::with_trust(path.clone(), trust())),
            Arc::new(Registry::new()),
            false,
        );
        let entry = first_entry();
        let refused = reg
            .invoke(
                "extensions.catalogManifest",
                json!({"id": entry.id, "sha256": entry.release.sha256}),
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(refused.contains("expired on 2000-01-01"), "{refused}");
    }
    /// Acceptance (#559): a tampered, unsigned or wrongly signed catalog is refused, and the
    /// last verified cache kept.
    #[test]
    fn a_tampered_catalog_keeps_the_last_verified_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        load_with(&path, &trust(), true, || Ok(fixture())).unwrap();
        let kept = fs::read(&path).unwrap();
        // One entry's checksum swapped under the catalog key's signature.
        let mut tampered: Envelope = serde_json::from_slice(&fixture()).unwrap();
        let mut doc: Value = serde_json::from_slice(
            &super::super::trust::decode_base64(&tampered.payload, MAX_CATALOG).unwrap(),
        )
        .unwrap();
        doc["extensions"][0]["release"]["sha256"] = json!("0".repeat(64));
        tampered.payload = super::super::trust::encode_base64(&serde_json::to_vec(&doc).unwrap());
        let stranger = testing::key(0x66);
        for (what, raw) in [
            ("tampered", serde_json::to_vec(&tampered).unwrap()),
            ("unsigned", unsigned().to_vec()),
            ("signed by another key", signed_by(&doc, &[&stranger])),
            (
                "signed by a publisher",
                signed_by(&doc, &[&testing::key(testing::EXAMPLE_SEED)]),
            ),
            ("truncated", fixture()[..100].to_vec()),
        ] {
            let loaded = load_with(&path, &trust(), true, || Ok(raw)).unwrap();
            assert!(loaded.stale, "{what}");
            assert!(loaded.error.is_some(), "{what}");
            assert_eq!(
                loaded.verified.catalog.extensions[0].release.sha256,
                first_entry().release.sha256,
                "{what}"
            );
            assert_eq!(fs::read(&path).unwrap(), kept, "{what}");
        }
        // With nothing verified before, there is nothing to keep, and it says why.
        let refused = load_with(&dir.path().join("fresh.json"), &trust(), true, || {
            Ok(unsigned().to_vec())
        })
        .unwrap_err();
        assert!(refused.contains("unsigned or malformed"), "{refused}");
    }
    /// A publisher delegation the catalog role did not sign, or two publishers claiming one
    /// namespace, fail the whole catalog: a host cannot tell which delegation was meant.
    #[test]
    fn a_catalog_with_an_unsigned_or_overlapping_delegation_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mut forged = document(unsigned(), 1, "2100-01-01T00:00:00Z");
        let squatter = testing::key(0x55);
        let payload =
            super::super::trust::decode_base64(&testing::example_publisher().payload, 4096)
                .unwrap();
        forged["publishers"][1] = json!(testing::sign(
            super::super::trust::PUBLISHER_TYPE,
            &payload,
            &[&squatter]
        ));
        let refused = load_with(&dir.path().join("a.json"), &trust(), true, || {
            Ok(signed(&forged))
        })
        .unwrap_err();
        assert!(refused.contains("catalog role"), "{refused}");
        let mut overlapping = document(unsigned(), 1, "2100-01-01T00:00:00Z");
        overlapping["publishers"][1] = json!(testing::publisher(
            "squatter",
            "Squatter",
            &[testing::public(&squatter)],
            &["org.srelens.flux"]
        ));
        let refused = load_with(&dir.path().join("b.json"), &trust(), true, || {
            Ok(signed(&overlapping))
        })
        .unwrap_err();
        assert!(refused.contains("overlaps"), "{refused}");
    }
    #[test]
    fn a_host_with_no_pinned_root_refuses_every_catalog_and_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let unavailable = TrustRoot::from_signed_root(br#"{"placeholder": "x"}"#);
        assert!(unavailable.is_err());
        let path = dir.path().join("c.json");
        let cache = CatalogCache::Owned {
            path: path.clone(),
            trust: testing::placeholder_root(),
        };
        let refused = load_with(&path, cache.trust(), true, || Ok(fixture())).unwrap_err();
        assert!(refused.contains("key ceremony"), "{refused}");
        assert!(cache.authority().unwrap_err().contains("key ceremony"));
    }
    #[test]
    fn rejects_invalid_catalog_fields_and_unrecognized_wire_payloads() {
        let base: Value = serde_json::from_slice(&payload()).unwrap();
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
        assert!(accept(&vec![b' '; MAX_SIGNED_CATALOG + 1], &trust(), None)
            .unwrap_err()
            .contains("2 MiB"));
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
        let base: Value = serde_json::from_slice(&payload()).unwrap();
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
    fn snapshot_lists_every_supported_api_version_and_distrusts_caches_from_before_559() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let state = load_with(&path, &trust(), false, || Ok(fixture()))
            .unwrap()
            .snapshot();
        let value = serde_json::to_value(&state).unwrap();
        let supported = json!(srelens_plugin_host::SUPPORTED_API_VERSIONS);
        assert_eq!(value["hostApiVersions"], supported);
        // The singular field stays for API 0.1 clients, as the newest supported version.
        let newest = json!(srelens_plugin_host::SUPPORTED_API_VERSIONS.last().unwrap());
        assert_eq!(value["hostApiVersion"], newest);
        // A cache a host wrote before #559 holds an unsigned catalog. However fresh, it is
        // not trusted: the signed catalog is fetched, and without one nothing is shown.
        let legacy = json!({
            "catalog": serde_json::from_slice::<Value>(unsigned()).unwrap(),
            "fetchedAt": now(), "stale": false, "error": null, "incompatible": [],
        });
        fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        assert!(load_with(&path, &trust(), false, || Err("offline".into())).is_err());
        let fetched = std::cell::Cell::new(false);
        load_with(&path, &trust(), false, || {
            fetched.set(true);
            Ok(fixture())
        })
        .unwrap();
        assert!(fetched.get(), "a cache from before #559 must be refetched");
    }
    /// The public catalog as it stood when API 0.4 was cut (#709), and the signed bytes of
    /// each release it listed: Argo CD 0.3.0 and Flux 0.4.0, both `^0.3` and using none
    /// of 0.4's fields. `public_catalog_release_smoke` follows the live catalog as it
    /// moves on; these stay, because installed copies of them go on reverifying.
    fn published_0_3_line() -> (Catalog, [(&'static str, &'static [u8], &'static [u8]); 2]) {
        let catalog = parse_catalog(
            &serde_json::to_vec(&document(
                include_bytes!("../../tests/fixtures/extension-catalog-api-0.3.json"),
                1,
                "2100-01-01T00:00:00Z",
            ))
            .unwrap(),
        )
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
    /// Migration (#559): the Flux and Argo CD releases published before it carry a bare
    /// signature that names no key. They verify under the srelens delegation, install as
    /// signed by srelens, and go on verifying on every load.
    #[test]
    fn the_published_0_3_line_releases_still_verify_install_and_reverify() {
        let (catalog, releases) = published_0_3_line();
        assert_eq!(catalog.extensions.len(), releases.len());
        let delegations = delegations();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("apps.json");
        for (id, raw, signature) in releases {
            let entry = catalog.extensions.iter().find(|e| e.id == id).unwrap();
            assert_eq!(entry.release.srelens_api_version, "^0.3", "{id}");
            // Offered, checksum and publisher signature verified, and valid on this host.
            assert!(compatible(&entry.release.srelens_api_version), "{id}");
            let review =
                verify_release(entry, raw, raw_signature(signature), &delegations).unwrap();
            assert_eq!(review.signed_by.as_ref().unwrap().name, "srelens", "{id}");
            let parsed = Manifest::parse(&review.manifest).unwrap();
            super::super::validate_app(
                &parsed,
                &parsed.permission_names(),
                super::super::tests::fake_core(),
            )
            .unwrap();
            let state = super::super::tests::configure(
                &path,
                json!({"action":"install","manifest":review.manifest,
                    "signature":signature.to_vec(),"grants":parsed.permission_names()}),
            )
            .unwrap_or_else(|e| panic!("{id}: {e}"));
            let app = state
                .plugins
                .iter()
                .find(|app| app.manifest.id == id)
                .unwrap();
            assert!(app.enabled && app.quarantined.is_none(), "{id}");
            let proof = app.signature_proof.as_ref().unwrap();
            // The delegation this build ships vouches for it, so none is stored, and a host
            // from before #559 can still read the inventory.
            assert!(proof.delegation.is_none(), "{id}");
            assert_eq!(app.signed_by.as_ref().unwrap().name, "srelens", "{id}");
        }
        // Loading the inventory rechecks every app against this host's API fields.
        let state = read(&path).unwrap();
        assert_eq!(state.plugins.len(), 2);
        assert!(state
            .plugins
            .iter()
            .all(|app| app.enabled && app.quarantined.is_none() && app.signed_by.is_some()));
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
        // The root a release pins, not the test root this crate's other tests pin.
        let trust = testing::production_root();
        let loaded = load(&dir.path().join("cache.json"), &trust, true).unwrap();
        assert!(!loaded.stale, "{:?}", loaded.error);
        let core = Arc::new(crate::build_registry_with_paths(
            srelens_kube::client_cache::ClientCache::new_many(vec![]),
            vec![],
        ));
        let delegations = Delegations::merged(&loaded.verified.delegations, &trust.shipped());
        assert!(!loaded.verified.catalog.extensions.is_empty());
        for entry in &loaded.verified.catalog.extensions {
            let raw = download(&entry.release.manifest_url, MAX_MANIFEST_BYTES).unwrap();
            let signature = signature_url(entry, &delegations).map(|url| {
                signing::parse_release_signature(&download(&url, MAX_RELEASE_SIGNATURE).unwrap())
                    .unwrap()
            });
            let source = verify_release(entry, &raw, signature, &delegations)
                .unwrap()
                .manifest;
            let parsed = Manifest::parse(&source).unwrap();
            super::super::validate_app(&parsed, &parsed.permission_names(), core.clone()).unwrap();
            println!("Verified {} {}", entry.id, entry.release.version);
        }
    }
}
