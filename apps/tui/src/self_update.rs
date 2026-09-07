//! `srelens-tui update` — move this binary to the latest stable release.
//!
//! The desktop app updates itself from Settings; the TUI is a loose binary on
//! someone's `PATH`, so it has to do the same job by hand. The shape here is
//! the one `crates/kube/src/toolbox_install.rs` already uses for kubectl and
//! helm: every decision is a pure function over strings, and the one step that
//! touches the network takes an injected `fetch`, so the whole path is tested
//! without a request.
//!
//! Two rules the code is built around, both of them about not leaving someone
//! worse off than before they ran it:
//!
//! - Nothing is written until the download's SHA-256 matches the checksum the
//!   release publishes. A corrupt or substituted archive never reaches the
//!   path the user runs.
//! - The final step is always a rename, never a copy into place, so an
//!   interrupted update cannot produce a half-written binary.

use std::fmt;
use std::path::{Path, PathBuf};

/// Where the release metadata and assets live. `releases/latest` is the
/// GitHub endpoint that skips pre-releases, which is what makes this the
/// stable channel without any filtering of our own.
pub const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/srelens/srelens/releases/latest";
const DOWNLOAD_BASE: &str = "https://github.com/srelens/srelens/releases/download";

/// The name of the binary inside every archive, and on disk.
const BIN: &str = if cfg!(windows) {
    "srelens-tui.exe"
} else {
    "srelens-tui"
};

#[derive(Debug, PartialEq, Eq)]
pub enum UpdateError {
    /// No release archive is built for this OS/architecture.
    UnsupportedPlatform { os: String, arch: String },
    /// The release API answered with something unparseable.
    BadRelease(String),
    /// The checksum file did not list the archive we downloaded.
    ChecksumMissing { asset: String },
    /// The archive's hash is not the published one.
    ChecksumMismatch { expected: String, actual: String },
    /// The archive did not contain the binary.
    BinaryMissing { asset: String },
    /// A download failed; retrying may work.
    Download(String),
    /// The archive could not be opened.
    Archive(String),
    /// Something on disk refused.
    Io(String),
    /// The binary belongs to a package manager, which should do the updating.
    PackageManaged {
        manager: &'static str,
        path: PathBuf,
    },
    /// The binary's directory cannot be written to by this user.
    NotWritable { path: PathBuf },
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedPlatform { os, arch } => write!(
                f,
                "no srelens-tui release is built for {os}/{arch} — build from source with `cargo build --release -p srelens-tui`"
            ),
            Self::BadRelease(why) => write!(f, "could not read the latest release: {why}"),
            Self::ChecksumMissing { asset } => write!(
                f,
                "the release's checksum file does not list {asset}, so the download cannot be verified"
            ),
            Self::ChecksumMismatch { expected, actual } => write!(
                f,
                "the download does not match its published checksum (expected {expected}, got {actual}) — nothing was changed"
            ),
            Self::BinaryMissing { asset } => {
                write!(f, "{asset} does not contain {BIN}")
            }
            Self::Download(why) => write!(f, "download failed: {why}"),
            Self::Archive(why) => write!(f, "could not read the downloaded archive: {why}"),
            Self::Io(why) => write!(f, "{why}"),
            Self::PackageManaged { manager, path } => write!(
                f,
                "{} installed this copy ({}), so let it do the upgrade — srelens will not write over a file a package manager owns",
                manager,
                path.display()
            ),
            Self::NotWritable { path } => write!(
                f,
                "cannot write to {} — re-run with the rights to change it, or install srelens-tui somewhere you own",
                path.display()
            ),
        }
    }
}

impl std::error::Error for UpdateError {}

fn io(e: std::io::Error) -> UpdateError {
    UpdateError::Io(e.to_string())
}

/// The release-asset triple for an OS/arch, and whether the binary is linked
/// against musl.
///
/// `musl` is the running binary's own libc, not a preference: a musl build
/// exists because the host cannot run the glibc one, so updating it to a glibc
/// archive would hand the user a binary that no longer starts. The caller
/// passes `cfg!(target_env = "musl")`, which is decided when THIS binary was
/// compiled.
pub fn triple_for(os: &str, arch: &str, musl: bool) -> Result<&'static str, UpdateError> {
    let unsupported = || UpdateError::UnsupportedPlatform {
        os: os.to_string(),
        arch: arch.to_string(),
    };
    match (os, arch, musl) {
        ("linux", "x86_64", false) => Ok("x86_64-unknown-linux-gnu"),
        ("linux", "x86_64", true) => Ok("x86_64-unknown-linux-musl"),
        ("linux", "aarch64", false) => Ok("aarch64-unknown-linux-gnu"),
        ("linux", "aarch64", true) => Ok("aarch64-unknown-linux-musl"),
        ("macos", "x86_64", _) => Ok("x86_64-apple-darwin"),
        ("macos", "aarch64", _) => Ok("aarch64-apple-darwin"),
        ("windows", "x86_64", _) => Ok("x86_64-pc-windows-msvc"),
        _ => Err(unsupported()),
    }
}

/// The triple for the binary that is running.
pub fn current_triple() -> Result<&'static str, UpdateError> {
    triple_for(
        std::env::consts::OS,
        std::env::consts::ARCH,
        cfg!(target_env = "musl"),
    )
}

/// The asset filename for a version and triple. Windows ships a `.zip`;
/// everything else a `.tar.gz`.
pub fn asset_name(version: &str, triple: &str) -> String {
    let ext = if triple.contains("windows") {
        "zip"
    } else {
        "tar.gz"
    };
    format!("srelens-tui-{version}-{triple}.{ext}")
}

/// The checksum file published beside the archives.
pub fn sums_name(version: &str) -> String {
    format!("srelens-tui-{version}-SHA256SUMS.txt")
}

/// A release asset's download URL.
pub fn asset_url(version: &str, file: &str) -> String {
    format!("{DOWNLOAD_BASE}/srelens-v{version}/{file}")
}

/// The version of the latest release, from the API's JSON.
///
/// Tags are `srelens-v<version>`; the prefix is stripped so the rest of the
/// module deals in versions only.
pub fn parse_latest_version(body: &[u8]) -> Result<String, UpdateError> {
    let value: serde_json::Value = serde_json::from_slice(body)
        .map_err(|e| UpdateError::BadRelease(format!("the API did not return JSON: {e}")))?;
    let tag = value
        .get("tag_name")
        .and_then(|t| t.as_str())
        .ok_or_else(|| UpdateError::BadRelease("no tag_name in the response".into()))?;
    let version = tag
        .strip_prefix("srelens-v")
        .ok_or_else(|| UpdateError::BadRelease(format!("unexpected tag {tag}")))?;
    if version.is_empty() {
        return Err(UpdateError::BadRelease(format!(
            "empty version in tag {tag}"
        )));
    }
    Ok(version.to_string())
}

/// Look one archive up in a `sha256sum`-format file.
///
/// Both spellings appear in the wild and the release has shipped each: GNU
/// writes `<hash>  <name>` and binary mode writes `<hash> *<name>`. Matching
/// the trailing name rather than splitting on a fixed column handles both.
pub fn checksum_for(sums: &str, asset: &str) -> Result<String, UpdateError> {
    for line in sums.lines() {
        let mut parts = line.split_whitespace();
        let (Some(hash), Some(name)) = (parts.next(), parts.next()) else {
            continue;
        };
        let name = name.strip_prefix('*').unwrap_or(name);
        if name == asset {
            if hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Ok(hash.to_ascii_lowercase());
            }
            return Err(UpdateError::ChecksumMissing {
                asset: asset.to_string(),
            });
        }
    }
    Err(UpdateError::ChecksumMissing {
        asset: asset.to_string(),
    })
}

/// Verify `bytes` against an expected hex SHA-256.
pub fn verify_sha256(bytes: &[u8], expected_hex: &str) -> Result<(), UpdateError> {
    use sha2::{Digest, Sha256};
    let actual = format!("{:x}", Sha256::digest(bytes));
    if actual.eq_ignore_ascii_case(expected_hex) {
        Ok(())
    } else {
        Err(UpdateError::ChecksumMismatch {
            expected: expected_hex.to_ascii_lowercase(),
            actual,
        })
    }
}

/// Pull the binary out of a downloaded archive, choosing the reader by the
/// asset's extension rather than by the host, so a test can exercise both.
pub fn extract_binary(archive: &[u8], asset: &str) -> Result<Vec<u8>, UpdateError> {
    if asset.ends_with(".zip") {
        extract_from_zip(archive, asset)
    } else {
        extract_from_targz(archive, asset)
    }
}

fn extract_from_targz(archive: &[u8], asset: &str) -> Result<Vec<u8>, UpdateError> {
    use std::io::Read;
    let decoder = flate2::read::GzDecoder::new(archive);
    let mut tar = tar::Archive::new(decoder);
    for entry in tar
        .entries()
        .map_err(|e| UpdateError::Archive(e.to_string()))?
    {
        let mut entry = entry.map_err(|e| UpdateError::Archive(e.to_string()))?;
        let path = entry
            .path()
            .map_err(|e| UpdateError::Archive(e.to_string()))?
            .into_owned();
        // The archive stores files at its root, so entries arrive as `./name`
        // or `name` depending on how they were added.
        if path
            .file_name()
            .map(|n| n == "srelens-tui")
            .unwrap_or(false)
        {
            let mut bytes = Vec::new();
            entry
                .read_to_end(&mut bytes)
                .map_err(|e| UpdateError::Archive(e.to_string()))?;
            return Ok(bytes);
        }
    }
    Err(UpdateError::BinaryMissing {
        asset: asset.to_string(),
    })
}

fn extract_from_zip(archive: &[u8], asset: &str) -> Result<Vec<u8>, UpdateError> {
    use std::io::Read;
    let reader = std::io::Cursor::new(archive);
    let mut zip = zip::ZipArchive::new(reader).map_err(|e| UpdateError::Archive(e.to_string()))?;
    for i in 0..zip.len() {
        let mut file = zip
            .by_index(i)
            .map_err(|e| UpdateError::Archive(e.to_string()))?;
        let name = file
            .name()
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_string();
        if name == "srelens-tui.exe" {
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)
                .map_err(|e| UpdateError::Archive(e.to_string()))?;
            return Ok(bytes);
        }
    }
    Err(UpdateError::BinaryMissing {
        asset: asset.to_string(),
    })
}

/// Whether `latest` is a higher version than `current`.
///
/// Both are parsed as semver so `0.10.0` sorts above `0.9.0`, which a string
/// comparison gets backwards. Anything unparseable is treated as "no update",
/// because refusing to act on a version we do not understand is safer than
/// overwriting a binary on a guess.
pub fn is_newer(current: &str, latest: &str) -> bool {
    match (
        semver::Version::parse(current),
        semver::Version::parse(latest),
    ) {
        (Ok(current), Ok(latest)) => latest > current,
        _ => false,
    }
}

/// The package manager that owns `path`, if one does.
///
/// srelens will not write over a file a package manager tracks: the manager's
/// database would still name the old version, and the next upgrade would put
/// it back. The desktop app already declines for AUR on the same grounds.
pub fn package_manager_for(path: &Path) -> Option<&'static str> {
    let text = path.to_string_lossy().replace('\\', "/");
    // Ordered longest-prefix-first where they nest, so a Cellar path is not
    // reported as the more general /usr/local.
    let owners: &[(&str, &str)] = &[
        ("/opt/homebrew/", "Homebrew"),
        ("/home/linuxbrew/.linuxbrew/", "Homebrew"),
        ("/usr/local/Cellar/", "Homebrew"),
        ("/usr/bin/", "your distribution's package manager"),
        ("/snap/", "snap"),
        ("/var/lib/flatpak/", "Flatpak"),
        ("/nix/store/", "Nix"),
        ("scoop/apps/", "Scoop"),
        ("scoop/shims/", "Scoop"),
        ("WinGet/Packages/", "winget"),
        ("Chocolatey/", "Chocolatey"),
    ];
    owners
        .iter()
        .find(|(prefix, _)| text.contains(prefix))
        .map(|(_, manager)| *manager)
}

/// What an update would do, resolved before anything is downloaded.
#[derive(Debug, PartialEq, Eq)]
pub struct Plan {
    pub current: String,
    pub latest: String,
    pub asset: String,
    pub archive_url: String,
    pub sums_url: String,
    pub target: PathBuf,
}

/// What checking for an update found.
///
/// `UpToDate` and `AheadOfStable` are deliberately not the same answer. The
/// endpoint consulted describes the STABLE channel, so a dev build that sorts
/// above the newest stable release has not been told it is the latest of
/// anything — only that no newer stable applies to it. Collapsing the two would
/// have `update` report a pre-release as "the latest release" while newer
/// pre-releases exist, which is the difference AGENTS.md is about: say what you
/// know, not what you guess.
#[derive(Debug, PartialEq, Eq)]
pub enum Check {
    /// Running exactly the latest stable release.
    UpToDate { latest: String },
    /// Running something that sorts above it — a dev or locally built binary.
    AheadOfStable { latest: String },
    /// A newer stable release is available.
    Available(Box<Plan>),
}

/// Resolve the latest stable release and decide whether it is worth
/// downloading. Not finding an update is a normal outcome, not a failure.
pub fn plan(
    current: &str,
    target: PathBuf,
    fetch: &impl Fn(&str) -> Result<Vec<u8>, UpdateError>,
) -> Result<Check, UpdateError> {
    let triple = current_triple()?;
    let body = fetch(LATEST_RELEASE_URL)?;
    let latest = parse_latest_version(&body)?;
    if !is_newer(current, &latest) {
        // An unparseable current version lands here too, and is reported as
        // up to date rather than ahead: claiming to be ahead of a release we
        // could not compare against would be the same overclaim in reverse.
        let ahead = matches!(
            (semver::Version::parse(current), semver::Version::parse(&latest)),
            (Ok(current), Ok(latest)) if current > latest
        );
        return Ok(if ahead {
            Check::AheadOfStable { latest }
        } else {
            Check::UpToDate { latest }
        });
    }
    let asset = asset_name(&latest, triple);
    Ok(Check::Available(Box::new(Plan {
        current: current.to_string(),
        archive_url: asset_url(&latest, &asset),
        sums_url: asset_url(&latest, &sums_name(&latest)),
        asset,
        latest,
        target,
    })))
}

/// Download, verify, and install the binary a [`Plan`] names.
///
/// Verification happens before anything is written, so a mismatched download
/// leaves the installed binary untouched.
pub fn apply(
    plan: &Plan,
    fetch: &impl Fn(&str) -> Result<Vec<u8>, UpdateError>,
) -> Result<(), UpdateError> {
    if let Some(manager) = package_manager_for(&plan.target) {
        return Err(UpdateError::PackageManaged {
            manager,
            path: plan.target.clone(),
        });
    }
    let dir = plan.target.parent().unwrap_or_else(|| Path::new("."));
    if !writable_dir(dir) {
        return Err(UpdateError::NotWritable {
            path: dir.to_path_buf(),
        });
    }

    let sums = fetch(&plan.sums_url)?;
    let sums = String::from_utf8(sums)
        .map_err(|_| UpdateError::BadRelease("the checksum file is not text".into()))?;
    let expected = checksum_for(&sums, &plan.asset)?;

    let archive = fetch(&plan.archive_url)?;
    verify_sha256(&archive, &expected)?;

    let binary = extract_binary(&archive, &plan.asset)?;
    replace_running_binary(&plan.target, &binary)
}

/// Can this process create files in `dir`? Asked by trying, because the
/// permission bits do not account for ownership, ACLs or a read-only mount,
/// and a wrong guess here turns into a confusing failure halfway through.
fn writable_dir(dir: &Path) -> bool {
    let probe = dir.join(format!(".srelens-tui-write-test-{}", std::process::id()));
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// Put `bytes` at `target`, replacing the binary that is currently running.
///
/// The new file is written beside the target and renamed in, so the last step
/// is atomic and an interrupted update cannot leave a truncated binary on the
/// user's `PATH`.
///
/// Windows cannot unlink a running image, which is what makes this more than a
/// rename: it CAN rename one, so the running binary is moved aside first and
/// the new one takes its place. The displaced file cannot be deleted until the
/// process exits, so it is left for the next run to clear.
pub fn replace_running_binary(target: &Path, bytes: &[u8]) -> Result<(), UpdateError> {
    let dir = target.parent().unwrap_or_else(|| Path::new("."));
    let staged = dir.join(format!(".{}.new-{}", BIN, std::process::id()));
    std::fs::write(&staged, bytes).map_err(io)?;
    set_executable(&staged)?;

    if cfg!(windows) {
        let displaced = dir.join(format!("{BIN}.old"));
        let _ = std::fs::remove_file(&displaced);
        if let Err(e) = std::fs::rename(target, &displaced) {
            let _ = std::fs::remove_file(&staged);
            return Err(io(e));
        }
        if let Err(e) = std::fs::rename(&staged, target) {
            // Put the original back rather than leaving the user with nothing.
            let _ = std::fs::rename(&displaced, target);
            let _ = std::fs::remove_file(&staged);
            return Err(io(e));
        }
        // Fails while this process holds the image open; the next run clears it.
        let _ = std::fs::remove_file(&displaced);
    } else if let Err(e) = std::fs::rename(&staged, target) {
        let _ = std::fs::remove_file(&staged);
        return Err(io(e));
    }
    Ok(())
}

/// Remove the file a previous Windows update left behind. A no-op everywhere
/// else, and best-effort: a leftover is untidy, never harmful.
pub fn clear_displaced_binary(target: &Path) {
    if cfg!(windows) {
        if let Some(dir) = target.parent() {
            let _ = std::fs::remove_file(dir.join(format!("{BIN}.old")));
        }
    }
}

#[cfg(unix)]
fn set_executable(path: &Path) -> Result<(), UpdateError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).map_err(io)
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> Result<(), UpdateError> {
    Ok(())
}
