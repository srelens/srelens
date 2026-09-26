//! The `.srelens-extension` package (#562): an app's manifest and its files in one archive,
//! with one digest list naming every file and one publisher signature over that list.
//!
//! A package is a gzip-compressed POSIX tar laid out as:
//!
//! ```text
//! example.srelens-extension
//! ├── extension.json      the manifest, exactly as a single-file release carries it
//! ├── README.md
//! ├── LICENSE
//! ├── icons/              .svg and .png; icons/icon.svg or icons/icon.png is the logo
//! ├── schemas/            .json
//! ├── bin/<platform>/     darwin-arm64, darwin-amd64, linux-amd64, linux-arm64, windows-amd64
//! ├── digests.json        every other file's path, size and SHA-256, with the app ID and version
//! └── digests.json.sig    optional: the publisher's Ed25519 signature over digests.json
//! ```
//!
//! Only `extension.json` and `digests.json` are required. The signature is the one scheme a
//! single-file release uses: Ed25519 over exact bytes, with the key found by
//! [`signing::verify_for`] for the app ID. There it covers `manifest.json`; here it covers the
//! digest list, and the list covers everything else, so a changed, missing or extra file
//! fails verification just as a changed manifest does.
//!
//! [`read`] takes a package whole or not at all. It reads the tar headers as they are
//! written (no GNU long names, PAX headers or sparse files), refuses links, devices and any
//! path outside the layout, and holds every size and count to the limits below. [`unpack`]
//! reads it a second time into a private staging directory, which a durable rename then
//! moves into place; nothing is written until the whole package has verified.
use super::signing;
use crate::durable;
use base64::Engine as _;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use srelens_plugin_host::{is_format_character, MAX_MANIFEST_BYTES};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

/// The manifest, as a single-file release carries it.
pub(super) const MANIFEST: &str = "extension.json";
/// The digest list: every other file's path, size and SHA-256.
pub(super) const DIGESTS: &str = "digests.json";
/// The publisher's signature over the exact bytes of [`DIGESTS`].
pub(super) const SIGNATURE: &str = "digests.json.sig";
/// What a digest list says it is, so neither a manifest nor anything else signed with the
/// same key can pass for one.
const FORMAT: &str = "srelens-extension-package";
const FORMAT_VERSION: u32 = 1;
/// The platforms `bin/` may hold binaries for.
pub(super) const PLATFORMS: [&str; 5] = [
    "darwin-arm64",
    "darwin-amd64",
    "linux-amd64",
    "linux-arm64",
    "windows-amd64",
];
/// The app's logo, in the order it is looked for, with the media type it is shown as.
const LOGOS: [(&str, &str); 2] = [
    ("icons/icon.svg", "image/svg+xml"),
    ("icons/icon.png", "image/png"),
];

/// The largest package file, compressed: what is downloaded, or sent from the app.
pub(super) const MAX_PACKAGE_BYTES: usize = 16 * 1024 * 1024;
/// The most every file together may hold, uncompressed.
pub(super) const MAX_UNPACKED_BYTES: u64 = 64 * 1024 * 1024;
/// The most entries, files and directories, one archive may hold.
pub(super) const MAX_ENTRIES: usize = 256;
/// The largest digest list. It is kept in the inventory for a signed package.
pub(super) const MAX_DIGESTS_BYTES: usize = 64 * 1024;
/// The largest icon: it is sent to the UI as a data URL.
pub(super) const MAX_ICON_BYTES: u64 = 256 * 1024;
/// The largest README, LICENSE or schema.
const MAX_DOCUMENT_BYTES: u64 = 1024 * 1024;
const MAX_PATH_BYTES: usize = 160;
const MAX_SEGMENT_BYTES: usize = 64;
const MAX_DEPTH: usize = 6;
const SIGNATURE_BYTES: u64 = 64;
/// The most the uncompressed tar stream may hold: every file, a header and padding for each
/// entry, and the end-of-archive blocks a tar tool writes (GNU tar pads to 10 KiB).
const MAX_STREAM_BYTES: u64 = MAX_UNPACKED_BYTES + (MAX_ENTRIES as u64 + 1) * 1024 + 20 * 512;
/// The directory an install unpacks into before it is moved into place.
const STAGING_PREFIX: &str = ".staging-";
/// Where an install moves a copy of the version it is replacing, as `<prefix><digest>`,
/// until the new copy is in place. A staging name too, so an unused one is pruned.
const REPLACED_PREFIX: &str = ".staging-replaced-";

/// What a path names in the layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place {
    Manifest,
    Digests,
    Signature,
    Document,
    Icon,
    Schema,
    Binary,
}

impl Place {
    /// The largest file this place may hold.
    fn limit(self) -> u64 {
        match self {
            Self::Manifest => MAX_MANIFEST_BYTES as u64,
            Self::Digests => MAX_DIGESTS_BYTES as u64,
            Self::Signature => SIGNATURE_BYTES,
            Self::Icon => MAX_ICON_BYTES,
            Self::Document | Self::Schema => MAX_DOCUMENT_BYTES,
            Self::Binary => MAX_UNPACKED_BYTES,
        }
    }
}

/// Text from a package, safe to put in a message: control and invisible formatting
/// characters, which could hide or reorder the rest of the line, are replaced.
fn shown(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_control() || is_format_character(c) {
                '\u{FFFD}'
            } else {
                c
            }
        })
        .collect()
}

/// Whether `stem`, the part of a name before its first dot, is a device name on Windows,
/// where `nul.txt` and `CON.json` name devices rather than files.
fn windows_device(stem: &str) -> bool {
    let stem = stem.to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit())
}

/// Why `segment` cannot be one segment of a package path, if it cannot. The rules make a
/// path mean the same file on every platform the host runs on: plain ASCII, no `.` or
/// `..`, nothing Windows strips or reads as a device.
fn segment_problem(segment: &str) -> Option<&'static str> {
    if segment.is_empty() {
        return Some("has an empty segment");
    }
    if segment.len() > MAX_SEGMENT_BYTES {
        return Some("has a segment longer than 64 bytes");
    }
    if !segment
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        return Some("may use only letters, digits, '.', '_' and '-'");
    }
    if segment.starts_with('.') {
        return Some("has a segment that starts with '.'");
    }
    if segment.ends_with('.') {
        return Some("has a segment that ends with '.'");
    }
    if windows_device(segment.split('.').next().unwrap_or(segment)) {
        return Some("names a Windows device");
    }
    None
}

/// The segments of a path that follows the rules, or why it does not.
fn segments(path: &str) -> Result<Vec<&str>, String> {
    let problem = |why: &str| format!("Package path \"{}\" {why}", shown(path));
    if path.is_empty() {
        return Err("Package has an entry with an empty path".into());
    }
    if path.starts_with('/') {
        return Err(problem("is absolute"));
    }
    if path.len() > MAX_PATH_BYTES {
        return Err(problem("is longer than 160 bytes"));
    }
    let segments: Vec<&str> = path.split('/').collect();
    if let Some(why) = segments.iter().find_map(|segment| {
        if *segment == ".." {
            Some("leaves the package")
        } else {
            segment_problem(segment)
        }
    }) {
        return Err(problem(why));
    }
    if segments.len() > MAX_DEPTH {
        return Err(problem("is nested more than 6 deep"));
    }
    Ok(segments)
}

/// Where the file at `path` goes in the layout, or why it cannot be there.
fn check_file(path: &str) -> Result<Place, String> {
    let segments = segments(path)?;
    let problem = |why: &str| Err(format!("Package path \"{path}\" {why}"));
    let extension = |wanted: &[&str]| {
        wanted
            .iter()
            .any(|extension| path.ends_with(&format!(".{extension}")))
    };
    match (segments[0], segments.len()) {
        (MANIFEST, 1) => Ok(Place::Manifest),
        (DIGESTS, 1) => Ok(Place::Digests),
        (SIGNATURE, 1) => Ok(Place::Signature),
        ("README.md" | "LICENSE", 1) => Ok(Place::Document),
        ("icons", n) if n > 1 => {
            if extension(&["svg", "png"]) {
                Ok(Place::Icon)
            } else {
                problem("is not an .svg or .png image")
            }
        }
        ("schemas", n) if n > 1 => {
            if extension(&["json"]) {
                Ok(Place::Schema)
            } else {
                problem("is not a .json schema")
            }
        }
        ("bin", n) if n > 1 && !PLATFORMS.contains(&segments[1]) => {
            problem("is not under bin/ for a supported platform")
        }
        ("bin", n) if n > 2 => Ok(Place::Binary),
        _ => problem("is not part of the package layout"),
    }
}

/// Whether a directory entry at `path` is one the layout can hold files under.
fn check_directory(path: &str) -> Result<(), String> {
    let segments = segments(path)?;
    match segments[0] {
        "icons" | "schemas" => Ok(()),
        "bin" if segments.len() == 1 || PLATFORMS.contains(&segments[1]) => Ok(()),
        "bin" => Err(format!(
            "Package path \"{path}\" is not under bin/ for a supported platform"
        )),
        _ => Err(format!(
            "Package path \"{path}\" is a directory where the layout has none"
        )),
    }
}

/// Every path an archive or a digest list names, folded the way a case-insensitive file
/// system folds it: two paths that differ only in case would be one file on macOS and
/// Windows, and a file cannot also be a directory.
#[derive(Default)]
struct Names {
    all: BTreeSet<String>,
    files: BTreeSet<String>,
}

impl Names {
    fn add(&mut self, path: &str, file: bool) -> Result<(), String> {
        let folded = path.to_ascii_lowercase();
        if !self.all.insert(folded.clone()) {
            return Err(format!(
                "Package holds \"{path}\" twice, or two paths that differ only in case"
            ));
        }
        if file {
            self.files.insert(folded);
        }
        Ok(())
    }

    /// No file sits where another path needs a directory.
    fn check_nesting(&self) -> Result<(), String> {
        for path in &self.all {
            let mut end = 0;
            while let Some(slash) = path[end..].find('/') {
                end += slash;
                if self.files.contains(&path[..end]) {
                    return Err(format!(
                        "Package holds \"{}\" as a file and as a directory",
                        &path[..end]
                    ));
                }
                end += 1;
            }
        }
        Ok(())
    }
}

/// `digests.json`: what a package holds, bound to one app and version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DigestList {
    format: String,
    #[serde(rename = "formatVersion")]
    format_version: u32,
    pub(super) id: String,
    pub(super) version: String,
    /// Each file once, in byte order of its path.
    pub(super) files: Vec<FileDigest>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FileDigest {
    pub(super) path: String,
    pub(super) size: u64,
    /// Lowercase hex.
    pub(super) sha256: String,
}

fn is_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Lowercase hex SHA-256 of `bytes`.
pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// The app ID a digest list names, which also names the app's directory.
fn check_app_id(id: &str) -> Result<(), String> {
    let valid = id.len() <= 128
        && id.contains('.')
        && id.split('.').all(|part| {
            !part.is_empty()
                && part.len() <= 64
                && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        });
    if !valid {
        return Err(format!(
            "digests.json names \"{}\", which is not an app ID",
            shown(id)
        ));
    }
    if windows_device(id.split('.').next().unwrap_or(id)) {
        return Err(format!(
            "App ID {id} cannot be a package: its first segment names a Windows device"
        ));
    }
    Ok(())
}

/// Reads a digest list, refusing anything but the exact form: known fields only, each
/// path once in byte order, every path within the layout, and the manifest listed.
pub(super) fn parse_digests(raw: &[u8]) -> Result<DigestList, String> {
    if raw.len() > MAX_DIGESTS_BYTES {
        return Err("digests.json exceeds 64 KiB".into());
    }
    let list: DigestList = serde_json::from_slice(raw).map_err(|e| {
        format!(
            "digests.json is not a digest list: {}",
            shown(&e.to_string())
        )
    })?;
    if list.format != FORMAT {
        return Err(format!("digests.json is not a {FORMAT} digest list"));
    }
    if list.format_version != FORMAT_VERSION {
        return Err(format!(
            "The package is format version {}; this host reads version {FORMAT_VERSION}",
            list.format_version
        ));
    }
    check_app_id(&list.id)?;
    semver::Version::parse(&list.version).map_err(|_| {
        format!(
            "digests.json has an invalid version \"{}\"",
            shown(&list.version)
        )
    })?;
    if list.files.len() > MAX_ENTRIES {
        return Err(format!("digests.json lists more than {MAX_ENTRIES} files"));
    }
    let mut names = Names::default();
    let mut total = 0u64;
    let mut previous: Option<&str> = None;
    for file in &list.files {
        let place = check_file(&file.path)?;
        if matches!(place, Place::Digests | Place::Signature) {
            return Err(format!(
                "digests.json lists {}, which it cannot cover",
                file.path
            ));
        }
        if previous.is_some_and(|previous| previous >= file.path.as_str()) {
            return Err("digests.json must list each path once, in byte order".into());
        }
        previous = Some(&file.path);
        names.add(&file.path, true)?;
        if !is_hex(&file.sha256, 64) {
            return Err(format!(
                "digests.json gives {} a SHA-256 that is not 64 lowercase hex digits",
                file.path
            ));
        }
        if file.size > place.limit() {
            return Err(format!("{} is larger than the package allows", file.path));
        }
        total = total.saturating_add(file.size);
        if total > MAX_UNPACKED_BYTES {
            return Err("The files in digests.json come to more than 64 MiB".into());
        }
    }
    names.check_nesting()?;
    if !list.files.iter().any(|file| file.path == MANIFEST) {
        return Err("digests.json does not list extension.json".into());
    }
    Ok(list)
}

/// The manifest names the app and version its digest list does.
fn check_identity(list: &DigestList, manifest: &str) -> Result<(), String> {
    let value: serde_json::Value = serde_json::from_str(manifest).map_err(|e| {
        format!(
            "extension.json is not valid JSON: {}",
            shown(&e.to_string())
        )
    })?;
    let named = |field: &str| value.get(field).and_then(serde_json::Value::as_str);
    if named("id") != Some(list.id.as_str()) || named("version") != Some(list.version.as_str()) {
        return Err(format!(
            "extension.json does not name the app and version its digest list names ({} {})",
            list.id, list.version
        ));
    }
    Ok(())
}

/// The list's entry for `extension.json` is `manifest`, and the manifest is the list's app.
pub(super) fn check_manifest_listed(digests: &str, manifest: &str) -> Result<DigestList, String> {
    let list = parse_digests(digests.as_bytes())?;
    let listed = list
        .files
        .iter()
        .find(|file| file.path == MANIFEST)
        .ok_or("digests.json does not list extension.json")?;
    if listed.size != manifest.len() as u64 || listed.sha256 != sha256_hex(manifest.as_bytes()) {
        return Err("extension.json does not match its entry in digests.json".into());
    }
    check_identity(&list, manifest)?;
    Ok(list)
}

/// A package's publisher signature: over the exact bytes of `digests`, by the publisher of
/// the app the list names, and `manifest` is the list's `extension.json`. What an install,
/// a review and every inventory load check a signed package's stored proof with.
pub(super) fn verify_signed(digests: &str, signature: &[u8], manifest: &str) -> Result<(), String> {
    let list = check_manifest_listed(digests, manifest)?;
    signing::verify_for(&list.id, digests.as_bytes(), signature)
}

/// An icon checked to be what its name says, as the UI will show it.
struct Icon {
    media_type: &'static str,
    bytes: Vec<u8>,
}

impl Icon {
    fn new(path: &str, media_type: &'static str, bytes: Vec<u8>) -> Result<Self, String> {
        let valid = match media_type {
            "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
            _ => std::str::from_utf8(&bytes).is_ok_and(|text| text.contains("<svg")),
        };
        if !valid {
            return Err(format!("{path} is not the image its name says it is"));
        }
        Ok(Self { media_type, bytes })
    }

    /// A `data:` URL: the UI shows it as an image, never as markup, so an SVG's scripts
    /// and external references are not run or fetched.
    fn data_url(&self) -> String {
        format!(
            "data:{};base64,{}",
            self.media_type,
            base64::engine::general_purpose::STANDARD.encode(&self.bytes)
        )
    }
}

/// One file a package holds, as a review lists it.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub(super) struct PackageFile {
    path: String,
    size: u64,
}

/// What a reviewer is shown of a package beyond its manifest.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub(super) struct PackageReview {
    /// SHA-256 of the package file, lowercase hex.
    sha256: String,
    /// The exact digest list the signature, if any, covers. `extensions.validate` takes it
    /// with the manifest and signature, so it checks the signature as installing would.
    digests: String,
    /// Every file the package holds, as its digest list names them.
    files: Vec<PackageFile>,
    /// The package's logo as a `data:` URL. Decoration: it says nothing about who
    /// published the app.
    #[serde(skip_serializing_if = "Option::is_none")]
    icon: Option<String>,
}

/// A package read whole and verified by [`read`].
pub(super) struct Package {
    pub(super) list: DigestList,
    /// The exact text of `digests.json`.
    pub(super) digests: String,
    pub(super) signature: Option<Vec<u8>>,
    /// The exact text of `extension.json`.
    pub(super) manifest: String,
    /// SHA-256 of the package file, lowercase hex: what a catalog release names.
    pub(super) sha256: String,
    /// SHA-256 of `digests.json`, lowercase hex: this version's name on disk.
    pub(super) digest: String,
    icon: Option<Icon>,
}

/// Which package this is, without its files.
impl std::fmt::Debug for Package {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_struct("Package")
            .field("id", &self.list.id)
            .field("version", &self.list.version)
            .field("digest", &self.digest)
            .field("signed", &self.signature.is_some())
            .finish()
    }
}

impl Package {
    /// Whether the package carries binaries. This host runs declarative apps only, so an
    /// install refuses them until executable apps have a sandbox (#521).
    pub(super) fn carries_binaries(&self) -> bool {
        self.list
            .files
            .iter()
            .any(|file| file.path.starts_with("bin/"))
    }

    pub(super) fn review(&self) -> PackageReview {
        PackageReview {
            sha256: self.sha256.clone(),
            digests: self.digests.clone(),
            files: self
                .list
                .files
                .iter()
                .map(|file| PackageFile {
                    path: file.path.clone(),
                    size: file.size,
                })
                .collect(),
            icon: self.icon.as_ref().map(Icon::data_url),
        }
    }
}

/// Refuses a package this host cannot install.
pub(super) fn check_installable(package: &Package) -> Result<(), String> {
    if package.carries_binaries() {
        return Err(
            "This package carries binaries under bin/. This host runs declarative apps only; executable apps are not supported yet"
                .into(),
        );
    }
    Ok(())
}

/// Where a package's files go as [`read`] verifies them.
pub(super) trait Sink {
    /// Takes the file at `path` (already checked against the layout), reading `content`
    /// to its end.
    fn file(&mut self, path: &str, content: &mut dyn Read) -> io::Result<()>;
}

/// Keeps nothing: for a package that is only being checked.
pub(super) struct Discard;

impl Sink for Discard {
    fn file(&mut self, _: &str, content: &mut dyn Read) -> io::Result<()> {
        io::copy(content, &mut io::sink()).map(drop)
    }
}

/// Writes each file under `root`, owner-only, and syncs it.
struct Writer<'a> {
    root: &'a Path,
}

impl Sink for Writer<'_> {
    fn file(&mut self, path: &str, content: &mut dyn Read) -> io::Result<()> {
        let target = path
            .split('/')
            .fold(self.root.to_path_buf(), |target, segment| {
                target.join(segment)
            });
        if let Some(parent) = target.parent() {
            let mut builder = fs::DirBuilder::new();
            builder.recursive(true);
            #[cfg(unix)]
            std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
            builder.create(parent)?;
        }
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = options.open(&target)?;
        io::copy(content, &mut file)?;
        file.sync_all()
    }
}

/// The uncompressed stream went past [`MAX_STREAM_BYTES`].
#[derive(Debug)]
struct Oversize;

impl std::fmt::Display for Oversize {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "the package unpacks to more than its limit")
    }
}

impl std::error::Error for Oversize {}

/// A reader that fails with [`Oversize`] rather than give more than `left` bytes.
struct Bounded<R> {
    inner: R,
    left: u64,
}

impl<R: Read> Read for Bounded<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        // One byte more than is allowed is enough to know it is too much.
        let most = buffer
            .len()
            .min(usize::try_from(self.left.saturating_add(1)).unwrap_or(usize::MAX));
        let read = self.inner.read(&mut buffer[..most])?;
        if read as u64 > self.left {
            return Err(io::Error::other(Oversize));
        }
        self.left -= read as u64;
        Ok(read)
    }
}

/// Hashes and counts what is read through it, and keeps the first error, so a failure to
/// read the archive is told apart from a failure to write a file.
struct Hashing<R> {
    inner: R,
    hasher: Sha256,
    count: u64,
    failed: Option<io::Error>,
}

impl<R: Read> Read for Hashing<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self.inner.read(buffer) {
            Ok(read) => {
                self.hasher.update(&buffer[..read]);
                self.count += read as u64;
                Ok(read)
            }
            Err(error) => {
                let copy = io::Error::new(error.kind(), error.to_string());
                if self.failed.is_none() {
                    self.failed = Some(error);
                }
                Err(copy)
            }
        }
    }
}

/// Why the archive could not be read.
fn unreadable(error: io::Error) -> String {
    if error.get_ref().is_some_and(|inner| inner.is::<Oversize>()) {
        return format!(
            "The package unpacks to more than {} MiB",
            MAX_UNPACKED_BYTES / (1024 * 1024)
        );
    }
    format!(
        "The package is not a valid gzip-compressed tar archive: {}",
        shown(&error.to_string())
    )
}

/// Reads and verifies a package, giving each file to `sink` as it goes; see the module
/// documentation for the rules. A package is refused whole: the error says why, and the
/// sink may have been given files of a package that is then refused, which is why
/// [`unpack`] reads a package into its directory only after reading it into [`Discard`].
pub(super) fn read(archive: &[u8], sink: &mut dyn Sink) -> Result<Package, String> {
    if archive.len() > MAX_PACKAGE_BYTES {
        return Err(format!(
            "The package exceeds {} MiB",
            MAX_PACKAGE_BYTES / (1024 * 1024)
        ));
    }
    let sha256 = sha256_hex(archive);
    let mut tar = tar::Archive::new(Bounded {
        inner: flate2::bufread::GzDecoder::new(archive),
        left: MAX_STREAM_BYTES,
    });
    // Each file's size and digest, and the few files kept to be read here.
    let mut found: BTreeMap<String, (u64, String)> = BTreeMap::new();
    let mut kept: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut names = Names::default();
    let mut unpacked = 0u64;
    // Raw: every header is seen as written. A GNU long name or PAX header would otherwise
    // be read whole into memory before its entry, and could rename the next one.
    for (index, entry) in tar.entries().map_err(unreadable)?.raw(true).enumerate() {
        if index == MAX_ENTRIES {
            return Err(format!("The package holds more than {MAX_ENTRIES} entries"));
        }
        let mut entry = entry.map_err(unreadable)?;
        let raw_path = entry.path_bytes().into_owned();
        let path = std::str::from_utf8(&raw_path).map_err(|_| {
            format!(
                "Package path \"{}\" is not UTF-8",
                shown(&String::from_utf8_lossy(&raw_path))
            )
        })?;
        let kind = entry.header().entry_type();
        let refuse = |what: &str| {
            Err(format!(
                "Package entry \"{}\" is {what}; a package holds only files and directories",
                shown(path)
            ))
        };
        let size = entry.size();
        match kind {
            tar::EntryType::Directory => {
                let path = path.strip_suffix('/').unwrap_or(path);
                check_directory(path)?;
                if size != 0 {
                    return Err(format!("Package directory \"{path}\" has contents"));
                }
                names.add(path, false)?;
            }
            tar::EntryType::Regular => {
                let place = check_file(path)?;
                names.add(path, true)?;
                if size > place.limit() {
                    return Err(format!("{path} is larger than the package allows"));
                }
                unpacked = unpacked.saturating_add(size);
                if unpacked > MAX_UNPACKED_BYTES {
                    return Err(format!(
                        "The package unpacks to more than {} MiB",
                        MAX_UNPACKED_BYTES / (1024 * 1024)
                    ));
                }
                let keep = matches!(place, Place::Manifest | Place::Digests | Place::Signature)
                    || LOGOS.iter().any(|(logo, _)| *logo == path);
                let mut content = Hashing {
                    inner: (&mut entry).take(size),
                    hasher: Sha256::new(),
                    count: 0,
                    failed: None,
                };
                let written = if keep {
                    // Within the place's limit, which is small for every file kept.
                    let mut bytes = Vec::with_capacity(size as usize);
                    if let Err(error) = content.read_to_end(&mut bytes) {
                        return Err(unreadable(content.failed.take().unwrap_or(error)));
                    }
                    let written = sink.file(path, &mut &bytes[..]);
                    kept.insert(path.to_owned(), bytes);
                    written
                } else {
                    // Whatever the sink leaves unread is read here, so it is hashed too.
                    sink.file(path, &mut content)
                        .and_then(|()| io::copy(&mut content, &mut io::sink()).map(drop))
                };
                if let Some(error) = content.failed.take() {
                    return Err(unreadable(error));
                }
                written.map_err(|e| format!("Could not unpack {path}: {e}"))?;
                if content.count != size {
                    return Err(format!("The package ends in the middle of {path}"));
                }
                found.insert(
                    path.to_owned(),
                    (size, format!("{:x}", content.hasher.finalize())),
                );
            }
            tar::EntryType::Symlink => return refuse("a symbolic link"),
            tar::EntryType::Link => return refuse("a hard link"),
            tar::EntryType::Char | tar::EntryType::Block | tar::EntryType::Fifo => {
                return refuse("a device or a pipe")
            }
            tar::EntryType::XHeader | tar::EntryType::XGlobalHeader => {
                return refuse("a PAX extended header")
            }
            tar::EntryType::GNULongName | tar::EntryType::GNULongLink => {
                return refuse("a GNU long name")
            }
            tar::EntryType::GNUSparse => return refuse("a sparse file"),
            _ => return refuse("an unsupported kind of entry"),
        }
    }
    names.check_nesting()?;
    // Nothing may follow the end of the archive but the zeros a tar tool pads it with:
    // anything else is entries that another reader might unpack and this one did not.
    let mut rest = tar.into_inner();
    let mut buffer = [0u8; 8192];
    loop {
        let read = rest.read(&mut buffer).map_err(unreadable)?;
        if read == 0 {
            break;
        }
        if buffer[..read].iter().any(|byte| *byte != 0) {
            return Err("The package has data after the end of its archive".into());
        }
    }
    if !rest.inner.get_ref().is_empty() {
        return Err("The package has data after its compressed archive".into());
    }

    let digests = kept
        .remove(DIGESTS)
        .ok_or("The package has no digests.json")?;
    let list = parse_digests(&digests)?;
    let digests = String::from_utf8(digests).map_err(|_| "digests.json is not UTF-8")?;
    let signature = kept.remove(SIGNATURE);
    if let Some(signature) = &signature {
        if signature.len() as u64 != SIGNATURE_BYTES {
            return Err("digests.json.sig is not a 64-byte Ed25519 signature".into());
        }
        signing::verify_for(&list.id, digests.as_bytes(), signature)?;
    }
    // Every file is listed, and every listed file is here, exactly as listed.
    let mut listed: BTreeMap<&str, &FileDigest> = list
        .files
        .iter()
        .map(|file| (file.path.as_str(), file))
        .collect();
    for (path, (size, sha256)) in &found {
        if path == DIGESTS || path == SIGNATURE {
            continue;
        }
        match listed.remove(path.as_str()) {
            None => {
                return Err(format!(
                    "The package holds {path}, which its digest list does not name"
                ))
            }
            Some(file) if file.size != *size || file.sha256 != *sha256 => {
                return Err(format!(
                    "{path} does not match its digest: the package was changed after its digest list was made"
                ))
            }
            Some(_) => {}
        }
    }
    if let Some(path) = listed.keys().next() {
        return Err(format!(
            "{path} is named in the digest list but missing from the package"
        ));
    }
    let manifest = kept
        .remove(MANIFEST)
        .ok_or("The package has no extension.json")?;
    let manifest = String::from_utf8(manifest).map_err(|_| "extension.json is not UTF-8")?;
    check_identity(&list, &manifest)?;
    let icon = LOGOS
        .iter()
        .find_map(|(path, media_type)| {
            kept.remove(*path)
                .map(|bytes| Icon::new(path, media_type, bytes))
        })
        .transpose()?;
    Ok(Package {
        digest: sha256_hex(digests.as_bytes()),
        list,
        digests,
        signature,
        manifest,
        sha256,
        icon,
    })
}

/// The directory an app's unpacked versions are kept in. IDs are ASCII and may differ
/// only in case, which one directory on a case-insensitive file system cannot tell apart;
/// each version is named by its own digest list's hash, and every list names its app, so
/// two such apps share the directory without sharing a version.
fn app_directory(root: &Path, id: &str) -> PathBuf {
    root.join(id.to_ascii_lowercase())
}

/// Unpacks `package`, which [`read`] verified from `archive`, into its private directory:
/// `root/<id>/<digest>`, owner-only. It is read again into a staging directory beside it,
/// with every file synced, and moved into place in one durable rename, so the version's
/// directory is absent or complete. A copy already there is replaced.
///
/// Installing the version is the inventory's save that names it afterwards, an atomic
/// replace of its own; until then nothing uses this directory, and [`prune`] removes it
/// if that save never happens.
pub(super) fn unpack(root: &Path, archive: &[u8], package: &Package) -> Result<(), String> {
    let app = app_directory(root, &package.list.id);
    durable::create_private_dir_all(&app)
        .map_err(|e| format!("Could not create the app's directory: {e}"))?;
    let mut staging = tempfile::Builder::new();
    staging.prefix(STAGING_PREFIX);
    // tempfile's default leaves the directory readable by everyone, less the umask.
    #[cfg(unix)]
    staging.permissions(std::os::unix::fs::PermissionsExt::from_mode(0o700));
    let staging = staging
        .tempdir_in(&app)
        .map_err(|e| format!("Could not create a directory to unpack into: {e}"))?;
    let again = read(
        archive,
        &mut Writer {
            root: staging.path(),
        },
    )?;
    if again.digest != package.digest {
        return Err("The package changed while it was being unpacked".into());
    }
    let target = app.join(&package.digest);
    // The same version may be here already, reinstalled or kept for rollback. It is
    // replaced, not trusted, and moved aside first: the rename cannot replace a directory.
    // An install that stops between the two moves leaves the version absent and its copy
    // aside; `prune` puts that copy back on the next change, because the inventory, which
    // this install never saved, still names the version.
    let aside = match fs::symlink_metadata(&target) {
        Ok(_) => {
            let aside = app.join(format!("{REPLACED_PREFIX}{}", package.digest));
            remove(&aside);
            fs::rename(&target, &aside).map_err(|e| {
                format!("Could not replace the installed copy of this version: {e}")
            })?;
            Some(aside)
        }
        Err(_) => None,
    };
    if let Err(error) = durable::publish_dir(staging.path(), &target) {
        if let Some(aside) = &aside {
            if let Err(back) = fs::rename(aside, &target) {
                return Err(format!(
                    "Could not move the unpacked app into place ({error}), nor put back the copy it was replacing ({back}); that copy goes back on the next change to your apps that saves"
                ));
            }
        }
        return Err(format!(
            "Could not move the unpacked app into place: {error}"
        ));
    }
    // It is `target` now; there is nothing left for the temporary directory to remove.
    let _ = staging.keep();
    if let Some(aside) = aside {
        remove(&aside);
    }
    Ok(())
}

/// Removes a file or a directory tree, never following a symbolic link, and logs a failure.
fn remove(path: &Path) {
    let removed = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return,
        Err(error) => Err(error),
    };
    if let Err(error) = removed {
        log::warn!("could not remove {}: {error}", path.display());
    }
}

/// Removes from `root` every unpacked version no app keeps, and anything an install left
/// part-way. `keep` holds, for each installed app's ID, the digests of its current and
/// kept versions. Failures are logged and left for the next change: a stray directory
/// costs disk, never correctness, because nothing reads a version the inventory does not
/// name. Callers hold the inventory's lock, which every install holds too.
///
/// One leftover is put back rather than removed: the copy of a kept version that an
/// install moved aside to replace it and never replaced (see [`unpack`]).
pub(super) fn prune(root: &Path, keep: &BTreeMap<String, BTreeSet<String>>) {
    let mut wanted: BTreeMap<String, BTreeSet<&str>> = BTreeMap::new();
    for (id, versions) in keep {
        wanted
            .entry(id.to_ascii_lowercase())
            .or_default()
            .extend(versions.iter().map(String::as_str));
    }
    let apps = match fs::read_dir(root) {
        Ok(apps) => apps,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return,
        Err(error) => {
            log::warn!("could not list {}: {error}", root.display());
            return;
        }
    };
    for app in apps.flatten() {
        let path = app.path();
        let versions = app
            .file_name()
            .to_str()
            .and_then(|name| wanted.get(name))
            .filter(|_| fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir()));
        let Some(versions) = versions else {
            remove(&path);
            continue;
        };
        let Ok(entries) = fs::read_dir(&path) else {
            continue;
        };
        let entries: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
        for entry in entries {
            let name = entry
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            if versions.contains(name) {
                continue;
            }
            if let Some(digest) = name
                .strip_prefix(REPLACED_PREFIX)
                .filter(|digest| versions.contains(digest))
            {
                let kept = path.join(digest);
                let is_directory = fs::symlink_metadata(&entry).is_ok_and(|m| m.is_dir());
                if is_directory && fs::symlink_metadata(&kept).is_err() {
                    if let Err(error) = durable::publish_dir(&entry, &kept) {
                        // Kept where it is: it is the only copy, and the next change tries again.
                        log::warn!(
                            "could not put {} back as {}: {error}",
                            entry.display(),
                            kept.display()
                        );
                    }
                    continue;
                }
            }
            remove(&entry);
        }
    }
}

/// At most `limit` bytes of the regular file at `path`, and one more if there are more.
fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let why = |e: io::Error| format!("Could not read {}: {e}", path.display());
    if !fs::symlink_metadata(path).map_err(why)?.is_file() {
        return Err(format!("{} is not a file", path.display()));
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .and_then(|file| file.take(limit.saturating_add(1)).read_to_end(&mut bytes))
        .map_err(why)?;
    Ok(bytes)
}

/// The logo of the version of `id` unpacked under `root` as `digest`, as a `data:` URL,
/// checked against the digest list it was unpacked with: `Ok(None)` when it has none. A
/// file changed on disk since is refused, not shown.
pub(super) fn installed_icon(
    root: &Path,
    id: &str,
    digest: &str,
) -> Result<Option<String>, String> {
    if !is_hex(digest, 64) {
        return Err("The installed package's digest is not a SHA-256".into());
    }
    let directory = app_directory(root, id).join(digest);
    let digests = read_limited(&directory.join(DIGESTS), MAX_DIGESTS_BYTES as u64)?;
    if sha256_hex(&digests) != digest {
        return Err("digests.json no longer matches the installed package".into());
    }
    let list = parse_digests(&digests)?;
    if list.id != id {
        return Err("The installed package is another app's".into());
    }
    for (path, media_type) in LOGOS {
        let Some(file) = list.files.iter().find(|file| file.path == path) else {
            continue;
        };
        let bytes = read_limited(&directory.join(path), file.size)?;
        if bytes.len() as u64 != file.size || sha256_hex(&bytes) != file.sha256 {
            return Err(format!("{path} no longer matches its digest"));
        }
        return Icon::new(path, media_type, bytes).map(|icon| Some(icon.data_url()));
    }
    Ok(None)
}

/// Every file under `dir` a package would hold, by its package path, in byte order.
/// Symbolic links are refused, as a package refuses them.
fn files_under(dir: &Path) -> Result<BTreeMap<String, PathBuf>, String> {
    fn walk(dir: &Path, prefix: &str, files: &mut BTreeMap<String, PathBuf>) -> Result<(), String> {
        let entries =
            fs::read_dir(dir).map_err(|e| format!("Could not list {}: {e}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("Could not list {}: {e}", dir.display()))?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|name| format!("{} is not UTF-8", Path::new(&name).display()))?;
            let path = format!("{prefix}{name}");
            let kind = entry
                .file_type()
                .map_err(|e| format!("Could not read {}: {e}", entry.path().display()))?;
            if kind.is_dir() {
                walk(&entry.path(), &format!("{path}/"), files)?;
            } else if kind.is_file() {
                files.insert(path, entry.path());
            } else {
                return Err(format!("{path} is not a file or a directory"));
            }
        }
        Ok(())
    }
    let mut files = BTreeMap::new();
    walk(dir, "", &mut files)?;
    Ok(files)
}

/// The digest list for the package laid out in `dir`: every file but `digests.json` and
/// its signature, checked against the layout, with the app ID and version from
/// `extension.json`. What a publisher signs, as `digests.json.sig`, before [`pack`].
pub fn digest_list(dir: &Path) -> Result<Vec<u8>, String> {
    let mut names = Names::default();
    let mut files = Vec::new();
    for (path, source) in files_under(dir)? {
        if matches!(check_file(&path)?, Place::Digests | Place::Signature) {
            continue;
        }
        names.add(&path, true)?;
        let bytes = fs::read(&source).map_err(|e| format!("Could not read {path}: {e}"))?;
        files.push(FileDigest {
            path,
            size: bytes.len() as u64,
            sha256: sha256_hex(&bytes),
        });
    }
    names.check_nesting()?;
    let manifest =
        fs::read(dir.join(MANIFEST)).map_err(|e| format!("Could not read {MANIFEST}: {e}"))?;
    let manifest: serde_json::Value = serde_json::from_slice(&manifest)
        .map_err(|e| format!("{MANIFEST} is not valid JSON: {e}"))?;
    let named = |field: &str| {
        manifest
            .get(field)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("{MANIFEST} has no {field}"))
    };
    let list = DigestList {
        format: FORMAT.into(),
        format_version: FORMAT_VERSION,
        id: named("id")?,
        version: named("version")?,
        files,
    };
    let mut raw = serde_json::to_vec_pretty(&list).map_err(|e| e.to_string())?;
    raw.push(b'\n');
    parse_digests(&raw)?;
    Ok(raw)
}

/// The package laid out in `dir`, which holds its `digests.json` and, when it is signed,
/// its `digests.json.sig`. The list must still describe the files exactly, or the package
/// would fail every host's check. Entries are written in byte order of their paths, as
/// plain ustar files with no owner, time or mode beyond read (and execute, under `bin/`),
/// so the same directory always packs to the same archive.
pub fn pack(dir: &Path) -> Result<Vec<u8>, String> {
    let digests =
        fs::read(dir.join(DIGESTS)).map_err(|e| format!("Could not read {DIGESTS}: {e}"))?;
    if parse_digests(&digests)? != parse_digests(&digest_list(dir)?)? {
        return Err(format!(
            "{DIGESTS} does not describe the files in {}; make it again, and sign it again",
            dir.display()
        ));
    }
    let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::best(),
    ));
    for (path, source) in files_under(dir)? {
        let bytes = fs::read(&source).map_err(|e| format!("Could not read {path}: {e}"))?;
        let mut header = tar::Header::new_ustar();
        header.set_size(bytes.len() as u64);
        header.set_mode(if path.starts_with("bin/") {
            0o755
        } else {
            0o644
        });
        header.set_mtime(0);
        header.set_entry_type(tar::EntryType::Regular);
        builder
            .append_data(&mut header, &path, &bytes[..])
            .map_err(|e| format!("Could not pack {path}: {e}"))?;
    }
    let archive = builder
        .into_inner()
        .and_then(|encoder| encoder.finish())
        .map_err(|e| format!("Could not finish the package: {e}"))?;
    if archive.len() > MAX_PACKAGE_BYTES {
        return Err(format!(
            "The package exceeds {} MiB",
            MAX_PACKAGE_BYTES / (1024 * 1024)
        ));
    }
    Ok(archive)
}

#[cfg(test)]
pub(super) mod tests;
