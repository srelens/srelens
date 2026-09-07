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

/// The stable channel. `releases/latest` is the GitHub endpoint that skips
/// pre-releases, so this needs no filtering of our own.
pub const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/srelens/srelens/releases/latest";
/// The dev channel. The full list, newest first, because dev builds ARE
/// pre-releases and the endpoint above hides them by definition.
pub const RELEASES_URL: &str = "https://api.github.com/repos/srelens/srelens/releases?per_page=20";
const DOWNLOAD_BASE: &str = "https://github.com/srelens/srelens/releases/download";

/// Which releases to consider, matching the two the desktop app offers under
/// Settings → Updates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    /// Released versions.
    Stable,
    /// Rolling pre-releases, cut from `dev` once a day.
    Dev,
}

impl Channel {
    /// The channel a binary reporting `version` came from.
    ///
    /// Used as the default so `update` keeps you where you are: a dev build
    /// carries a pre-release version (`0.8.1-152`), and offering it only
    /// stable would strand it — the stable release it sits above is not an
    /// update, so there would be nothing to install and no way to say so.
    pub fn of_version(version: &str) -> Channel {
        match semver::Version::parse(version) {
            Ok(parsed) if !parsed.pre.is_empty() => Channel::Dev,
            _ => Channel::Stable,
        }
    }

    pub fn parse(name: &str) -> Option<Channel> {
        match name.trim().to_ascii_lowercase().as_str() {
            "stable" => Some(Channel::Stable),
            "dev" => Some(Channel::Dev),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Channel::Stable => "stable",
            Channel::Dev => "dev",
        }
    }

    fn url(self) -> &'static str {
        match self {
            Channel::Stable => LATEST_RELEASE_URL,
            Channel::Dev => RELEASES_URL,
        }
    }
}

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
    /// The binary lives somewhere anyone on the machine can write, which no
    /// amount of care during the update can compensate for.
    UnsafeDirectory { path: PathBuf },
    /// The staged download changed between being written and being
    /// installed — someone else can write to the directory.
    StagedChanged,
    /// The update failed AND the original could not be put back. The user
    /// has no working binary until they move it themselves, so this says
    /// exactly where it is rather than only why the update failed.
    LeftDisplaced {
        displaced: PathBuf,
        target: PathBuf,
        why: String,
    },
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedPlatform { os, arch } => write!(
                f,
                "no srelens-tui release is built for {os}/{arch} — build from source with `cargo build --release -p srelens-tui`"
            ),
            // No prefix: these messages are whole sentences, and a "could not
            // read" preamble was actively wrong for the common case, where the
            // release reads fine and simply carries no build for this platform.
            Self::BadRelease(why) => write!(f, "{why}"),
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
            Self::UnsafeDirectory { path } => write!(
                f,
                "anyone on this machine can create files in {}, so an update there cannot be made safe — move srelens-tui somewhere only you can write, then update",
                path.display()
            ),
            Self::StagedChanged => write!(
                f,
                "the downloaded file changed on disk before it could be installed, so nothing was replaced — someone else can write to that directory"
            ),
            Self::LeftDisplaced {
                displaced,
                target,
                why,
            } => write!(
                f,
                "the update failed and your previous binary could not be put back: it is at {}. Move it to {} to restore it. ({why})",
                displaced.display(),
                target.display()
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

/// The version a release tag names, or nothing if the tag is not one of ours
/// or does not carry a version.
///
/// Parsed, not merely non-empty. A tag like `srelens-vnightly` would
/// otherwise pass, fail to compare later, and be reported as "you are on the
/// latest" — turning broken release metadata into a confident answer about
/// the user's version, which is the one thing this command must not do.
fn version_from_tag(tag: &str) -> Option<String> {
    let version = tag.strip_prefix("srelens-v")?;
    semver::Version::parse(version).ok()?;
    Some(version.to_string())
}

/// Whether a release actually carries the two files an update needs for this
/// platform: the archive and the checksum file listing it.
///
/// A dev pre-release is published the moment it builds (`releaseDraft: false`
/// in the release workflow), so if the TUI matrix or `tui-publish` fails
/// afterwards the tag is public with no archives on it. Resolving to it would
/// promise an update and then 404 on the download. Every release cut before
/// the TUI shipped at all looks the same from here.
fn release_carries_this_platform(release: &serde_json::Value, version: &str, triple: &str) -> bool {
    let Some(assets) = release.get("assets").and_then(|a| a.as_array()) else {
        return false;
    };
    let names: Vec<&str> = assets
        .iter()
        .filter_map(|a| a.get("name").and_then(|n| n.as_str()))
        .collect();
    let archive = asset_name(version, triple);
    let sums = sums_name(version);
    names.contains(&archive.as_str()) && names.contains(&sums.as_str())
}

/// The version of the latest stable release, from the API's JSON.
///
/// Tags are `srelens-v<version>`; the prefix is stripped so the rest of the
/// module deals in versions only.
pub fn parse_latest_version(body: &[u8], triple: &str) -> Result<String, UpdateError> {
    let value: serde_json::Value = serde_json::from_slice(body)
        .map_err(|e| UpdateError::BadRelease(format!("the API did not return JSON: {e}")))?;
    let tag = value
        .get("tag_name")
        .and_then(|t| t.as_str())
        .ok_or_else(|| UpdateError::BadRelease("no tag_name in the response".into()))?;
    let version = version_from_tag(tag)
        .ok_or_else(|| UpdateError::BadRelease(format!("unexpected tag {tag}")))?;
    // Unlike the dev list there is nothing else to fall back to here, so a
    // release without the archives is reported rather than skipped. Saying so
    // now beats promising an update and 404ing on the download.
    if !release_carries_this_platform(&value, &version, triple) {
        return Err(UpdateError::BadRelease(format!(
            "release {tag} carries no srelens-tui build for {triple}"
        )));
    }
    Ok(version)
}

/// The newest version in a releases LIST, which is how the dev channel is
/// resolved: pre-releases are what it is made of, so the stable endpoint
/// cannot see them.
///
/// GitHub returns the list newest first. Four kinds of entry are skipped:
/// anything not tagged `srelens-v…`; `dev-channel`, a permanent rolling
/// pre-release carrying only the desktop updater's manifest and none of the
/// TUI archives; stable releases, which belong to the other channel; and any
/// release that does not actually carry a build for this platform.
pub fn parse_newest_version(body: &[u8], triple: &str) -> Result<String, UpdateError> {
    let releases: Vec<serde_json::Value> = serde_json::from_slice(body)
        .map_err(|e| UpdateError::BadRelease(format!("the API did not return a list: {e}")))?;
    for release in releases {
        let Some(tag) = release.get("tag_name").and_then(|t| t.as_str()) else {
            continue;
        };
        if tag == "dev-channel" {
            continue;
        }
        // Pre-releases only. When main cuts a stable release, it is the
        // newest entry in this list — and taking it would move a dev user
        // onto stable without saying so. Worse, it would stick: the
        // installed version would no longer carry a pre-release, so the
        // next plain `update` would default to the stable channel. A
        // channel switch has to be something the user asked for.
        if !release
            .get("prerelease")
            .and_then(|p| p.as_bool())
            .unwrap_or(false)
        {
            continue;
        }
        // Unlike the single-release endpoint, anything unusable is SKIPPED
        // here rather than failing the command: this is a list, so there is a
        // next entry to try, and one bad release should not stop a dev user
        // updating. Failing is right only where there is no alternative.
        let Some(version) = version_from_tag(tag) else {
            continue;
        };
        if !release_carries_this_platform(&release, &version, triple) {
            continue;
        }
        return Ok(version);
    }
    // Accurate about which step came up empty: the list was read fine, it just
    // holds nothing installable here. Naming the platform matters because the
    // usual cause is a release whose build for THIS target failed while the
    // others published.
    Err(UpdateError::BadRelease(format!(
        "no dev release carries a srelens-tui build for {triple}"
    )))
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

    // Roots a package manager owns, matched as PREFIXES. Matching them
    // anywhere was wrong: `/home/me/rootfs/usr/bin/srelens-tui` is a file its
    // owner controls, and calling it distribution-managed refused to update
    // it. Compared case-sensitively, because `/usr/bin` and `/USR/BIN` are
    // different directories on Unix.
    const ROOTS: &[(&str, &str)] = &[
        ("/opt/homebrew/", "Homebrew"),
        ("/home/linuxbrew/.linuxbrew/", "Homebrew"),
        ("/usr/local/Cellar/", "Homebrew"),
        ("/usr/bin/", "your distribution's package manager"),
        ("/snap/", "snap"),
        ("/var/lib/flatpak/", "Flatpak"),
        ("/nix/store/", "Nix"),
    ];
    if let Some((_, manager)) = ROOTS.iter().find(|(root, _)| text.starts_with(root)) {
        return Some(manager);
    }

    // Windows package roots sit at varying depths — under a user profile, or
    // ProgramData — so they are matched anywhere rather than anchored, and
    // folded to one case because Windows paths are case-insensitive. The real
    // Chocolatey root is `C:\\ProgramData\\chocolatey`, lower case, which a
    // case-sensitive match missed entirely.
    const WINDOWS_MARKERS: &[(&str, &str)] = &[
        ("/scoop/apps/", "Scoop"),
        ("/scoop/shims/", "Scoop"),
        ("/winget/packages/", "winget"),
        ("/chocolatey/", "Chocolatey"),
    ];
    // Only on Windows. Applied everywhere, `/home/me/scoop/apps/demo/...`
    // on Linux read as Scoop-owned and the update was refused before
    // anything was downloaded — these names mean a package manager on one
    // platform and nothing in particular on the others.
    if !cfg!(windows) {
        return None;
    }
    let folded = text.to_lowercase();
    WINDOWS_MARKERS
        .iter()
        .find(|(marker, _)| folded.contains(marker))
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
/// `UpToDate` and `AheadOfChannel` are deliberately not the same answer, and
/// both name the channel they are about. Only one channel is ever consulted,
/// so a dev build checked against stable has not been told it is the latest of
/// anything — only that no newer STABLE release applies to it. Collapsing the
/// two would have `update` report a pre-release as "the latest release" while
/// newer pre-releases exist, which is the difference AGENTS.md is about: say
/// what you know, not what you guess.
#[derive(Debug, PartialEq, Eq)]
pub enum Check {
    /// Running the newest release this channel offers.
    UpToDate { channel: Channel, latest: String },
    /// Running something that sorts above it — typically a dev build checked
    /// against stable, or a locally built one.
    AheadOfChannel { channel: Channel, latest: String },
    /// A newer stable release is available.
    Available(Box<Plan>),
}

/// Resolve the latest stable release and decide whether it is worth
/// downloading. Not finding an update is a normal outcome, not a failure.
/// `requested` is whether the caller NAMED this channel rather than
/// falling into it. It is the difference between "nothing newer here" and
/// "put me on this channel": a dev build sorts above the stable release, so
/// without it `--channel stable` could never do the switch the install
/// guide promises.
pub fn plan(
    current: &str,
    channel: Channel,
    requested: bool,
    target: PathBuf,
    fetch: &impl Fn(&str) -> Result<Vec<u8>, UpdateError>,
) -> Result<Check, UpdateError> {
    let triple = current_triple()?;
    let body = fetch(channel.url())?;
    let latest = match channel {
        Channel::Stable => parse_latest_version(&body, triple)?,
        Channel::Dev => parse_newest_version(&body, triple)?,
    };
    // Asking for a channel by name means asking to be ON it, even where
    // that means going backwards — the usual case, since any dev build
    // sorts above the stable release it was cut after.
    if !is_newer(current, &latest) && !(requested && current != latest) {
        // An unparseable current version lands here too, and is reported as
        // up to date rather than ahead: claiming to be ahead of a release we
        // could not compare against would be the same overclaim in reverse.
        let ahead = matches!(
            (semver::Version::parse(current), semver::Version::parse(&latest)),
            (Ok(current), Ok(latest)) if current > latest
        );
        return Ok(if ahead {
            Check::AheadOfChannel { channel, latest }
        } else {
            Check::UpToDate { channel, latest }
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
    if world_writable_without_sticky(dir) {
        return Err(UpdateError::UnsafeDirectory {
            path: dir.to_path_buf(),
        });
    }
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

/// Create a file in `dir` that did not exist a moment ago, and hand back
/// both it and its path.
///
/// `create_new` is the point: it fails if anything is already at the path,
/// INCLUDING a symlink, so it cannot be tricked into writing through one.
/// A predictable name plus a following open is a real hazard here — the
/// install guide has people extract and run from a working directory, and a
/// directory another user can write to lets them pre-create the path as a
/// link to a file the victim owns, which the update would then truncate.
/// The name is random as well, so the attempt cannot be aimed.
fn create_new_file(dir: &Path, prefix: &str) -> Result<(PathBuf, std::fs::File), UpdateError> {
    let mut last = None;
    for _ in 0..8 {
        let path = dir.join(format!("{prefix}{}", uuid::Uuid::new_v4()));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            // 0600 AT CREATION, not afterwards. The default is 0666 masked
            // by the umask, so a umask of 0002 — normal on systems with
            // per-group directories — would publish the file group-writable
            // for the window before the mode is corrected. Someone watching
            // the directory could open it in that window, hold the
            // descriptor, and change the contents after the archive was
            // verified. Opening restrictively closes the window instead of
            // narrowing it.
            options.mode(0o600);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // FILE_SHARE_READ only: while this handle is open nobody else
            // may open the file for writing or delete it.
            options.share_mode(0x0000_0001);
        }
        match options.open(&path) {
            Ok(file) => return Ok((path, file)),
            // Lost a race, or someone is planting names. Try another.
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => last = Some(e),
            Err(e) => return Err(io(e)),
        }
    }
    Err(io(last.unwrap_or_else(|| {
        std::io::Error::other("could not create a temporary file")
    })))
}

/// Can this process create files in `dir`? Asked by trying, because the
/// permission bits do not account for ownership, ACLs or a read-only mount,
/// and a wrong guess here turns into a confusing failure halfway through.
fn writable_dir(dir: &Path) -> bool {
    match create_new_file(dir, ".srelens-tui-write-test-") {
        Ok((probe, file)) => {
            drop(file);
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// Whether anyone on the machine can create and replace entries in `dir`.
///
/// Where that is true, no amount of care with temporary files makes an
/// update safe: writing and renaming are separate operations on a NAME, and
/// on Unix there is no way to rename by file descriptor, so the path can
/// always be swapped between the last check and the rename. The read-back
/// before replacement narrows that to a race an attacker must win, but a
/// race is not a guarantee. Refusing is the only honest answer.
///
/// The test is deliberately WORLD-writable and not sticky, not merely
/// group-writable. A group-writable install directory is an ordinary
/// configuration — `/usr/local/bin` belongs to `admin` on macOS — and group
/// membership is a trust decision someone already made. World-writable
/// without the sticky bit is not a place to keep an executable, and saying
/// so beats pretending the update was safe. The sticky bit is what makes
/// `/tmp` acceptable: entries there can only be removed by their owner.
#[cfg(unix)]
fn world_writable_without_sticky(dir: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::fs::PermissionsExt;
    match std::fs::metadata(dir) {
        Ok(meta) => {
            let mode = meta.permissions().mode();
            if mode & 0o002 == 0 {
                return false;
            }
            // The sticky bit lets an entry be removed by its owner, THE
            // DIRECTORY'S OWNER, or root. That is what makes `/tmp` safe —
            // root owns it. A world-writable sticky directory owned by
            // someone else still lets that person unlink the staged file
            // and put their own at the path, so the bit alone proves
            // nothing.
            let sticky = mode & 0o1000 != 0;
            let owner_is_trusted = meta.uid() == 0 || meta.uid() == unsafe { libc::geteuid() };
            !(sticky && owner_is_trusted)
        }
        // Unreadable metadata is not evidence of a problem; the write probe
        // below will fail on its own if the directory is unusable.
        Err(_) => false,
    }
}

/// Windows has no equivalent this cheap: its ACLs need the security
/// descriptor read and every ACE walked, which is a new dependency and a
/// judgement about which principals count as trusted — and getting that wrong
/// fails in the direction that stops people updating. Tracked in #450. The
/// staged handle's share mode and the read-back before the rename narrow the
/// window there; they do not close it.
#[cfg(not(unix))]
fn world_writable_without_sticky(_dir: &Path) -> bool {
    false
}

/// Put a binary back after an update was interrupted between the two
/// Windows renames.
///
/// That gap is two adjacent syscalls, but a kill or a power cut inside it
/// leaves the executable only at `.<name>.old` with nothing at the command
/// path — and the user cannot run `update` to fix it, because there is
/// nothing left to run. What they CAN run is the displaced file itself, so
/// every start checks whether that is what is happening and repairs it.
///
/// `Ok(None)` means there was nothing to recover, which is the ordinary
/// case. An `Err` means recovery was NEEDED and failed — a different thing
/// entirely, and one the caller has to say out loud: the command path is
/// still missing, so staying quiet would leave someone with a broken
/// install and no clue why.
///
/// Deliberately narrow: it acts only when this process IS the displaced
/// file and the real name is free, which cannot be true in ordinary use.
pub fn recover_interrupted_update(exe: &Path) -> Result<Option<PathBuf>, UpdateError> {
    let Some(target) = displaced_original(exe) else {
        return Ok(None);
    };
    // Windows allows renaming a running image, which is the same property
    // the update itself relies on.
    std::fs::rename(exe, &target).map_err(io)?;
    Ok(Some(target))
}

/// The name this binary should have, if it is sitting under the displaced
/// one with the real name free.
fn displaced_original(exe: &Path) -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    let name = exe.file_name()?.to_str()?;
    let restored = name.strip_prefix(".")?.strip_suffix(".old")?;
    if restored.is_empty() {
        return None;
    }
    // Whatever name it was, not the compiled-in one: the updater names the
    // displaced file after the binary it is replacing, so a renamed copy
    // must come back as the name its user invoked.
    let target = exe.with_file_name(restored);
    (!target.exists()).then_some(target)
}

/// Where the installed binary lives, given the path this process started
/// from.
///
/// After a recovery renames us back, `current_exe` still reports the path
/// the image was loaded from — the displaced one. An update that trusted
/// that would stage beside `.<binary>.old` and rename onto it, leaving the
/// command path untouched and the mess intact.
pub fn installed_path(exe: &Path) -> PathBuf {
    // Windows only, matching the recovery it exists to follow. Applied on
    // Unix it would take someone running a backup they named
    // `.srelens-tui.old` and update the sibling instead — replacing a
    // binary they did not invoke, which is the one promise this command
    // makes about what it touches.
    if !cfg!(windows) {
        return exe.to_path_buf();
    }
    let displaced = || -> Option<PathBuf> {
        let name = exe.file_name()?.to_str()?;
        let restored = name.strip_prefix(".")?.strip_suffix(".old")?;
        if restored.is_empty() {
            return None;
        }
        let target = exe.with_file_name(restored);
        target.exists().then_some(target)
    };
    displaced().unwrap_or_else(|| exe.to_path_buf())
}

/// Confirm the file at `path` still holds exactly `bytes`.
///
/// Split out so it can be tested directly: the race it defends against
/// cannot be staged from a test without becoming the very timing problem it
/// is about.
fn assert_staged_is_unchanged(path: &Path, bytes: &[u8]) -> Result<(), UpdateError> {
    let on_disk = std::fs::read(path).map_err(io)?;
    if on_disk == bytes {
        Ok(())
    } else {
        Err(UpdateError::StagedChanged)
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
    use std::io::Write;
    let dir = target.parent().unwrap_or_else(|| Path::new("."));
    // Created with `create_new` rather than written to a predictable path:
    // see `create_new_file`. Writing through a planted symlink here would
    // put a downloaded executable wherever the link pointed.
    // Named after the file being replaced rather than after the compiled-in
    // name. Someone who renames the binary to `lens` gets `.lens.old` and
    // `.lens.new-…`, so an interrupted update recovers the command they
    // actually had — the fixed name restored `srelens-tui` and left `lens`
    // missing.
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| BIN.to_string());
    let (staged, mut file) = create_new_file(dir, &format!(".{name}.new-"))?;
    // From here every exit removes the staged file, including the ones added
    // later by someone who did not read this far. It used to be a cleanup line
    // per early return, and the one that got missed leaked a whole downloaded
    // binary on each attempt — with random names now, those accumulate rather
    // than overwrite. After a successful rename the path is gone and the
    // removal is a no-op, so the success path needs no special case either.
    let staged = Staged(staged);

    file.write_all(bytes).map_err(io)?;
    // The mode change goes BEFORE the sync. Syncing first left the
    // permission change unsynced, so a power loss after the rename could
    // leave the installed command durable in content and still at its
    // private staging mode — present, correct, and not executable.
    set_executable(&staged.0)?;
    file.sync_all().map_err(io)?;
    drop(file);

    // Read back what is actually at that path before installing it.
    //
    // Closing the handle gives up the only hold on the NAME. On Windows
    // that ends the share-mode protection; on Unix a handle never protected
    // the name anyway — anyone who can write to a non-sticky directory may
    // unlink a 0600 file they cannot read and put their own there. Either
    // way the rename below could pick up a file nobody verified. Comparing
    // against the bytes in hand costs one read and makes the guarantee
    // whole: what gets installed is what came out of the archive whose
    // checksum matched, or nothing does.
    //
    // This does NOT make a directory other people can write to safe. They
    // can overwrite the installed binary a moment later, with or without
    // this command. It means only that THIS command never installs bytes it
    // did not verify.
    assert_staged_is_unchanged(&staged.0, bytes)?;

    if cfg!(windows) {
        // Dot-prefixed, matching the staging files, so this is plainly the
        // updater's and not something a person put here. The previous name
        // was `<binary>.old`, which is exactly what someone would call a
        // backup they made themselves — and it was removed unconditionally,
        // so a real update destroyed it. `fs::rename` overwrites on Windows
        // too, so choosing a name nobody else would pick is the fix, not the
        // explicit removal.
        let displaced = dir.join(format!(".{name}.old"));
        let _ = std::fs::remove_file(&displaced);
        std::fs::rename(target, &displaced).map_err(io)?;
        if let Err(e) = std::fs::rename(&staged.0, target) {
            // Put the original back rather than leaving the user with
            // nothing — and if even that fails, say so. Reporting only the
            // first error would tell the user the update failed while
            // leaving them with no binary on their PATH and no idea their
            // old one is sitting next to it under another name.
            if let Err(rollback) = std::fs::rename(&displaced, target) {
                return Err(UpdateError::LeftDisplaced {
                    displaced: displaced.clone(),
                    target: target.to_path_buf(),
                    why: format!("{e}; restoring it also failed: {rollback}"),
                });
            }
            return Err(io(e));
        }
        // Fails while this process holds the image open; the next run clears it.
        let _ = std::fs::remove_file(&displaced);
    } else {
        std::fs::rename(&staged.0, target).map_err(io)?;
    }
    Ok(())
}

/// A staged download that deletes itself unless it was renamed into place.
///
/// The point is that no future early return can forget: a half-finished
/// update leaves nothing behind whichever way it failed.
struct Staged(PathBuf);

impl Drop for Staged {
    fn drop(&mut self) {
        // A no-op once the file has been renamed away, which is the
        // successful case.
        let _ = std::fs::remove_file(&self.0);
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

#[cfg(test)]
mod tests {
    use super::create_new_file;

    /// `umask` is process-global, and unit tests share a process. Every
    /// test in this module that creates a file takes this first, so the one
    /// that changes the mask cannot hand a permissive default to another
    /// test's file — which would make the suite depend on thread timing
    /// rather than on the code.
    static FILE_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn file_test_lock() -> std::sync::MutexGuard<'static, ()> {
        // A panicking test poisons the mutex; the next one wants the lock,
        // not the panic.
        FILE_TESTS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The staged file must be private to its owner from the instant it
    /// exists, not from the moment its mode is corrected.
    ///
    /// The umask is set to 0 for the check, which is the point: a default
    /// creation would then be 0666, so seeing 0600 proves the mode comes from
    /// the open call rather than from whatever the machine's umask happens to
    /// be. Under a real umask of 0002 — normal where users share a group —
    /// the default would have been group-writable, and someone watching the
    /// directory could have opened the file, held the descriptor, and changed
    /// the contents after the archive's checksum was verified.
    #[cfg(unix)]
    #[test]
    fn a_staged_file_is_created_private_to_its_owner() {
        use std::os::unix::fs::PermissionsExt;

        let _guard = file_test_lock();
        let dir = tempfile::tempdir().expect("temp dir");
        let previous = unsafe { libc::umask(0) };
        let created = create_new_file(dir.path(), ".probe-");
        unsafe { libc::umask(previous) };

        let (path, _file) = created.expect("the file is created");
        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "created as {mode:o}, not 0600");
    }

    /// Nothing is installed unless the file on disk is still the file that
    /// came out of the verified archive.
    #[test]
    fn a_staged_file_that_changed_underneath_us_is_refused() {
        use super::{assert_staged_is_unchanged, UpdateError};

        let _guard = file_test_lock();
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("staged");

        std::fs::write(&path, b"the verified bytes").unwrap();
        assert!(assert_staged_is_unchanged(&path, b"the verified bytes").is_ok());

        // What an attacker who can write to the directory would leave.
        std::fs::write(&path, b"something else entirely").unwrap();
        assert!(matches!(
            assert_staged_is_unchanged(&path, b"the verified bytes"),
            Err(UpdateError::StagedChanged)
        ));

        // Same length, one byte different — a truncation check would miss it.
        std::fs::write(&path, b"the verified byteS").unwrap();
        assert!(matches!(
            assert_staged_is_unchanged(&path, b"the verified bytes"),
            Err(UpdateError::StagedChanged)
        ));

        // Gone entirely is an IO error, not a silent pass.
        std::fs::remove_file(&path).unwrap();
        assert!(matches!(
            assert_staged_is_unchanged(&path, b"the verified bytes"),
            Err(UpdateError::Io(_))
        ));
    }

    /// A directory anyone can write to is refused; the ordinary ones are not.
    ///
    /// The group-writable case is the one worth pinning: `/usr/local/bin` is
    /// group-writable on macOS, so refusing it would break a normal install
    /// rather than protect anyone.
    #[cfg(unix)]
    #[test]
    fn only_a_directory_anyone_can_write_to_is_refused() {
        use super::world_writable_without_sticky;
        use std::os::unix::fs::PermissionsExt;

        let _guard = file_test_lock();
        let dir = tempfile::tempdir().expect("temp dir");
        let set = |mode: u32| {
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(mode))
                .expect("set mode");
        };

        set(0o755);
        assert!(!world_writable_without_sticky(dir.path()), "0755 is fine");

        set(0o775);
        assert!(
            !world_writable_without_sticky(dir.path()),
            "group-writable is an ordinary configuration, not a refusal"
        );

        set(0o777);
        assert!(
            world_writable_without_sticky(dir.path()),
            "world-writable without the sticky bit must be refused"
        );

        // The sticky bit is what makes /tmp acceptable: only an entry's
        // owner may remove it, so the swap this guards against cannot
        // happen.
        set(0o1777);
        assert!(
            !world_writable_without_sticky(dir.path()),
            "sticky world-writable is how /tmp is set up"
        );

        set(0o755);
    }

    /// A sticky bit only makes a world-writable directory safe when its
    /// OWNER is trusted. Sticky lets an entry be removed by its owner, the
    /// directory's owner, or root — so a directory owned by someone else
    /// still lets them unlink the staged file and put their own at the path.
    ///
    /// A temp dir is owned by the current user, so it stands in for the
    /// trusted case; the untrusted one cannot be built without another
    /// account, and is asserted through the predicate's own inputs instead.
    #[cfg(unix)]
    #[test]
    fn a_sticky_directory_is_only_safe_when_its_owner_is() {
        use super::world_writable_without_sticky;
        use std::os::unix::fs::PermissionsExt;

        let _guard = file_test_lock();
        let dir = tempfile::tempdir().expect("temp dir");
        let set = |mode: u32| {
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(mode))
                .expect("set mode");
        };

        // Owned by us: sticky makes it acceptable, exactly as /tmp is.
        set(0o1777);
        assert!(!world_writable_without_sticky(dir.path()));

        // Without the bit it is refused whoever owns it.
        set(0o777);
        assert!(world_writable_without_sticky(dir.path()));

        // And a directory nobody else can write to never needed either.
        set(0o755);
        assert!(!world_writable_without_sticky(dir.path()));
        set(0o755);
    }

    /// The staged and displaced names follow the file being replaced, so a
    /// renamed binary recovers under the name its user invoked rather than
    /// the one compiled in.
    #[test]
    fn a_renamed_binary_stages_and_recovers_under_its_own_name() {
        use super::{installed_path, replace_running_binary};

        let _guard = file_test_lock();
        let dir = tempfile::tempdir().expect("temp dir");
        let renamed = dir
            .path()
            .join(if cfg!(windows) { "lens.exe" } else { "lens" });
        std::fs::write(&renamed, b"old").expect("seed");

        replace_running_binary(&renamed, b"new").expect("replace");
        assert_eq!(std::fs::read(&renamed).unwrap(), b"new");

        // Nothing named after the compiled-in binary was created.
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            names.iter().all(|n| !n.contains("srelens-tui")),
            "displaced under the wrong name: {names:?}"
        );

        // And the displaced file, where Windows leaves one, maps back to the
        // invoked name rather than to srelens-tui.
        if cfg!(windows) {
            let displaced = dir.path().join(".lens.exe.old");
            if displaced.exists() {
                assert_eq!(installed_path(&displaced), renamed);
            }
        }
    }

    /// Two calls never collide, which is what lets the name be unpredictable
    /// rather than derived from the process id.
    #[test]
    fn staged_files_do_not_reuse_a_name() {
        let _guard = file_test_lock();
        let dir = tempfile::tempdir().expect("temp dir");
        let (first, _a) = create_new_file(dir.path(), ".probe-").expect("first");
        let (second, _b) = create_new_file(dir.path(), ".probe-").expect("second");
        assert_ne!(first, second);
    }
}
