//! Toolbox install core (#55, spec §5–6).
//!
//! Downloads a managed tool from its official source, verifies it against the
//! vendor's published SHA-256, and places it in `~/.srelens/bin` — writing to a
//! temp file and renaming only after verification, so a partial or tampered
//! download never lands as the real binary. This slice covers kubectl (a single
//! binary from dl.k8s.io); krew and helm (tarballs) reuse these primitives.
//!
//! The network is injected as a `fetch` closure, so the planning, checksum
//! parsing/verification, and temp-then-rename are all unit-tested without a
//! real HTTP client (which is wired in at the capability layer that runs it).

use std::path::{Path, PathBuf};

/// Typed install failures, so callers can react correctly: a download may be
/// retried, a checksum mismatch must never be.
#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("unsupported platform: {os}/{arch}")]
    UnsupportedPlatform { os: String, arch: String },
    /// Transient — safe to retry with backoff.
    #[error("download failed: {0}")]
    Download(String),
    /// The bytes don't match the vendor's checksum — loud, never auto-retried.
    #[error("checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },
    #[error("malformed checksum file")]
    BadChecksumFile,
    #[error("could not read archive: {0}")]
    BadArchive(String),
    #[error("archive did not contain {member}")]
    MemberNotFound { member: String },
    #[error("filesystem error: {0}")]
    Io(String),
}

/// The vendor tokens for the running platform (`linux`/`amd64`, `darwin`/`arm64`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Platform {
    pub os: &'static str,
    pub arch: &'static str,
}

impl Platform {
    /// Map Rust's `std::env::consts` names to the kubectl/krew/helm tokens.
    pub fn resolve(os: &str, arch: &str) -> Result<Platform, InstallError> {
        let arch = match arch {
            "x86_64" => "amd64",
            "aarch64" => "arm64",
            _ => return Err(unsupported(os, arch)),
        };
        let os = match os {
            "linux" => "linux",
            "macos" => "darwin",
            "windows" => "windows",
            _ => return Err(unsupported(os, arch)),
        };
        Ok(Platform { os, arch })
    }

    /// The platform srelens is running on.
    pub fn current() -> Result<Platform, InstallError> {
        Platform::resolve(std::env::consts::OS, std::env::consts::ARCH)
    }
}

fn unsupported(os: &str, arch: &str) -> InstallError {
    InstallError::UnsupportedPlatform { os: os.to_string(), arch: arch.to_string() }
}

/// A resolved single-binary download: where to get it, where its checksum is,
/// and where it should land.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryInstall {
    pub binary_url: String,
    pub sha256_url: String,
    pub target: PathBuf,
}

/// The URL that returns the latest stable kubectl version string (e.g. `v1.30.2`).
pub const KUBECTL_STABLE_URL: &str = "https://dl.k8s.io/release/stable.txt";

/// Plan a kubectl install for `version` (a `vX.Y.Z` tag) into `install_dir`.
pub fn kubectl_install(version: &str, platform: &Platform, install_dir: &Path) -> BinaryInstall {
    let ext = if platform.os == "windows" { ".exe" } else { "" };
    let base = format!(
        "https://dl.k8s.io/release/{version}/bin/{}/{}/kubectl{ext}",
        platform.os, platform.arch
    );
    BinaryInstall {
        sha256_url: format!("{base}.sha256"),
        binary_url: base,
        target: install_dir.join(format!("kubectl{ext}")),
    }
}

/// Extract the hex digest from a checksum file. dl.k8s.io serves the bare hash;
/// `sha256sum`-style `<hash>  <file>` lines are also accepted (first token).
pub fn parse_sha256(content: &str) -> Result<String, InstallError> {
    let token = content.split_whitespace().next().unwrap_or_default();
    if token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(token.to_ascii_lowercase())
    } else {
        Err(InstallError::BadChecksumFile)
    }
}

/// Verify `bytes` against an expected hex SHA-256 (case-insensitive).
pub fn verify_sha256(bytes: &[u8], expected_hex: &str) -> Result<(), InstallError> {
    use sha2::{Digest, Sha256};
    let actual = hex::encode(Sha256::digest(bytes));
    if actual.eq_ignore_ascii_case(expected_hex) {
        Ok(())
    } else {
        Err(InstallError::ChecksumMismatch {
            expected: expected_hex.to_ascii_lowercase(),
            actual,
        })
    }
}

/// Download, verify, and atomically install a single binary. `fetch` returns the
/// bytes at a URL (or a retryable [`InstallError::Download`]); it's called for
/// the checksum file and then the binary. The binary is written to a temp file
/// beside the target and renamed in only after the checksum matches, so a failed
/// or tampered download never appears as the real tool.
pub fn install_binary(
    plan: &BinaryInstall,
    fetch: &impl Fn(&str) -> Result<Vec<u8>, InstallError>,
) -> Result<PathBuf, InstallError> {
    let checksum_raw = fetch(&plan.sha256_url)?;
    let checksum = std::str::from_utf8(&checksum_raw).map_err(|_| InstallError::BadChecksumFile)?;
    let expected = parse_sha256(checksum)?;

    let bytes = fetch(&plan.binary_url)?;
    verify_sha256(&bytes, &expected)?;
    write_binary(&plan.target, &bytes)
}

/// Atomically place `bytes` as an executable at `target`: create the parent dir,
/// write a `.partial` sibling, set the exec bit, then rename in. The rename is
/// the last step, so a caller never observes a half-written binary.
fn write_binary(target: &Path, bytes: &[u8]) -> Result<PathBuf, InstallError> {
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir).map_err(io)?;
    }
    let tmp = target.with_extension("partial");
    std::fs::write(&tmp, bytes).map_err(io)?;
    set_executable(&tmp)?;
    std::fs::rename(&tmp, target).map_err(io)?;
    Ok(target.to_path_buf())
}

/// A resolved tarball download: the archive, its checksum, the member to extract,
/// and where that member should land.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveInstall {
    pub archive_url: String,
    pub sha256_url: String,
    /// Path of the binary inside the tar (e.g. `linux-amd64/helm`).
    pub member: String,
    pub target: PathBuf,
}

/// GitHub API endpoint for helm's latest release (helm has no `stable.txt`).
pub const HELM_LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/helm/helm/releases/latest";

/// Pull the `tag_name` (e.g. `v3.16.2`) out of a GitHub "latest release" JSON body.
pub fn parse_github_latest_tag(body: &[u8]) -> Result<String, InstallError> {
    let value: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| InstallError::Download(e.to_string()))?;
    value
        .get("tag_name")
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| InstallError::Download("no tag_name in GitHub release response".to_string()))
}

/// Plan a helm install for `version` (a `vX.Y.Z` tag) into `install_dir`.
pub fn helm_install(version: &str, platform: &Platform, install_dir: &Path) -> ArchiveInstall {
    let ext = if platform.os == "windows" { ".exe" } else { "" };
    let archive_url =
        format!("https://get.helm.sh/helm-{version}-{}-{}.tar.gz", platform.os, platform.arch);
    ArchiveInstall {
        sha256_url: format!("{archive_url}.sha256sum"),
        member: format!("{}-{}/helm{ext}", platform.os, platform.arch),
        target: install_dir.join(format!("helm{ext}")),
        archive_url,
    }
}

/// Download a `.tar.gz`, verify it against the vendor checksum (which covers the
/// whole archive), extract the single planned member, and install it. Uses the
/// same temp-then-rename as [`install_binary`], so an unverified or corrupt
/// archive never yields an installed binary.
pub fn install_from_targz(
    plan: &ArchiveInstall,
    fetch: &impl Fn(&str) -> Result<Vec<u8>, InstallError>,
) -> Result<PathBuf, InstallError> {
    let checksum_raw = fetch(&plan.sha256_url)?;
    let checksum = std::str::from_utf8(&checksum_raw).map_err(|_| InstallError::BadChecksumFile)?;
    let expected = parse_sha256(checksum)?;

    let archive = fetch(&plan.archive_url)?;
    verify_sha256(&archive, &expected)?;

    let bytes = extract_member(&archive, &plan.member)?;
    write_binary(&plan.target, &bytes)
}

/// Read one member's bytes out of a gzip-compressed tar.
fn extract_member(targz: &[u8], member: &str) -> Result<Vec<u8>, InstallError> {
    use flate2::read::GzDecoder;
    use std::io::Read;
    use tar::Archive;

    let mut archive = Archive::new(GzDecoder::new(targz));
    let entries = archive.entries().map_err(|e| InstallError::BadArchive(e.to_string()))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| InstallError::BadArchive(e.to_string()))?;
        let path = entry.path().map_err(|e| InstallError::BadArchive(e.to_string()))?;
        if path.to_string_lossy() == member {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf).map_err(|e| InstallError::BadArchive(e.to_string()))?;
            return Ok(buf);
        }
    }
    Err(InstallError::MemberNotFound { member: member.to_string() })
}

fn io(e: std::io::Error) -> InstallError {
    InstallError::Io(e.to_string())
}

#[cfg(unix)]
fn set_executable(path: &Path) -> Result<(), InstallError> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path).map_err(io)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).map_err(io)
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> Result<(), InstallError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn platform_maps_rust_names_to_vendor_tokens() {
        assert_eq!(
            Platform::resolve("linux", "x86_64").unwrap(),
            Platform { os: "linux", arch: "amd64" }
        );
        assert_eq!(
            Platform::resolve("macos", "aarch64").unwrap(),
            Platform { os: "darwin", arch: "arm64" }
        );
        assert!(matches!(
            Platform::resolve("freebsd", "x86_64"),
            Err(InstallError::UnsupportedPlatform { .. })
        ));
        assert!(matches!(
            Platform::resolve("linux", "riscv64"),
            Err(InstallError::UnsupportedPlatform { .. })
        ));
    }

    #[test]
    fn kubectl_plan_builds_the_dl_k8s_urls_and_target() {
        let p = kubectl_install(
            "v1.30.2",
            &Platform { os: "linux", arch: "amd64" },
            Path::new("/home/u/.srelens/bin"),
        );
        assert_eq!(p.binary_url, "https://dl.k8s.io/release/v1.30.2/bin/linux/amd64/kubectl");
        assert_eq!(p.sha256_url, "https://dl.k8s.io/release/v1.30.2/bin/linux/amd64/kubectl.sha256");
        assert_eq!(p.target, Path::new("/home/u/.srelens/bin/kubectl"));
    }

    #[test]
    fn kubectl_plan_adds_exe_on_windows() {
        let p = kubectl_install(
            "v1.30.2",
            &Platform { os: "windows", arch: "amd64" },
            Path::new("C:/bin"),
        );
        assert!(p.binary_url.ends_with("/kubectl.exe"));
        assert_eq!(p.target, Path::new("C:/bin/kubectl.exe"));
    }

    #[test]
    fn parse_sha256_accepts_bare_and_sha256sum_forms() {
        let hex = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert_eq!(parse_sha256(&format!("{hex}\n")).unwrap(), hex);
        assert_eq!(parse_sha256(&format!("{hex}  kubectl\n")).unwrap(), hex);
        assert!(matches!(parse_sha256("nope"), Err(InstallError::BadChecksumFile)));
        assert!(matches!(parse_sha256(""), Err(InstallError::BadChecksumFile)));
    }

    #[test]
    fn verify_sha256_accepts_a_match_and_rejects_a_mismatch() {
        // Known: SHA-256 of the empty input.
        let empty = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert!(verify_sha256(b"", empty).is_ok());
        assert!(verify_sha256(b"", &empty.to_uppercase()).is_ok());
        assert!(matches!(
            verify_sha256(b"tampered", empty),
            Err(InstallError::ChecksumMismatch { .. })
        ));
    }

    /// A fake network: maps URL → bytes.
    fn net(entries: &[(&str, &[u8])]) -> impl Fn(&str) -> Result<Vec<u8>, InstallError> {
        let map: HashMap<String, Vec<u8>> =
            entries.iter().map(|(u, b)| (u.to_string(), b.to_vec())).collect();
        move |url: &str| {
            map.get(url)
                .cloned()
                .ok_or_else(|| InstallError::Download(format!("404 {url}")))
        }
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        hex::encode(Sha256::digest(bytes))
    }

    #[test]
    fn install_binary_writes_the_verified_bytes_and_makes_them_executable() {
        let dir = tempfile::tempdir().unwrap();
        let plan = kubectl_install(
            "v1.30.2",
            &Platform { os: "linux", arch: "amd64" },
            &dir.path().join("bin"),
        );
        let payload = b"#!/bin/sh\necho kubectl\n";
        let fetch = net(&[
            (plan.sha256_url.as_str(), sha256_hex(payload).as_bytes()),
            (plan.binary_url.as_str(), payload),
        ]);

        let path = install_binary(&plan, &fetch).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), payload);
        assert!(!dir.path().join("bin/kubectl.partial").exists(), "temp file left behind");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "not executable");
        }
    }

    #[test]
    fn install_binary_refuses_a_checksum_mismatch_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let plan = kubectl_install(
            "v1.30.2",
            &Platform { os: "linux", arch: "amd64" },
            dir.path(),
        );
        // Checksum is for different bytes than the binary payload.
        let fetch = net(&[
            (plan.sha256_url.as_str(), sha256_hex(b"expected").as_bytes()),
            (plan.binary_url.as_str(), b"tampered"),
        ]);

        assert!(matches!(
            install_binary(&plan, &fetch),
            Err(InstallError::ChecksumMismatch { .. })
        ));
        assert!(!plan.target.exists(), "a mismatched binary must not be installed");
        assert!(!plan.target.with_extension("partial").exists(), "temp file left behind");
    }

    #[test]
    fn install_binary_surfaces_a_download_failure_as_retryable() {
        let dir = tempfile::tempdir().unwrap();
        let plan = kubectl_install("v1.30.2", &Platform { os: "linux", arch: "amd64" }, dir.path());
        let fetch = net(&[]); // nothing resolves
        assert!(matches!(install_binary(&plan, &fetch), Err(InstallError::Download(_))));
    }

    #[test]
    fn github_latest_tag_is_parsed_and_bad_bodies_rejected() {
        assert_eq!(
            parse_github_latest_tag(br#"{"tag_name":"v3.16.2","name":"Helm"}"#).unwrap(),
            "v3.16.2"
        );
        assert!(matches!(
            parse_github_latest_tag(br#"{"name":"no tag here"}"#),
            Err(InstallError::Download(_))
        ));
        assert!(matches!(
            parse_github_latest_tag(br#"{"tag_name":""}"#),
            Err(InstallError::Download(_))
        ));
        assert!(matches!(parse_github_latest_tag(b"not json"), Err(InstallError::Download(_))));
    }

    #[test]
    fn helm_plan_builds_the_get_helm_urls_member_and_target() {
        let p = helm_install(
            "v3.16.2",
            &Platform { os: "darwin", arch: "arm64" },
            Path::new("/home/u/.srelens/bin"),
        );
        assert_eq!(p.archive_url, "https://get.helm.sh/helm-v3.16.2-darwin-arm64.tar.gz");
        assert_eq!(p.sha256_url, "https://get.helm.sh/helm-v3.16.2-darwin-arm64.tar.gz.sha256sum");
        assert_eq!(p.member, "darwin-arm64/helm");
        assert_eq!(p.target, Path::new("/home/u/.srelens/bin/helm"));
    }

    /// Build a real gzip-compressed tar with the given (path, bytes) members.
    fn make_targz(members: &[(&str, &[u8])]) -> Vec<u8> {
        use flate2::{write::GzEncoder, Compression};
        let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::fast()));
        for (name, bytes) in members {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, name, *bytes).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    #[test]
    fn install_from_targz_extracts_and_installs_the_planned_member() {
        let dir = tempfile::tempdir().unwrap();
        let plan =
            helm_install("v3.16.2", &Platform { os: "linux", arch: "amd64" }, &dir.path().join("bin"));
        let payload = b"#!/bin/sh\necho helm\n";
        let archive = make_targz(&[
            ("linux-amd64/LICENSE", b"license"),
            (plan.member.as_str(), payload),
            ("linux-amd64/README.md", b"readme"),
        ]);
        let fetch = net(&[
            (plan.sha256_url.as_str(), sha256_hex(&archive).as_bytes()),
            (plan.archive_url.as_str(), &archive),
        ]);

        let path = install_from_targz(&plan, &fetch).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), payload);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "not executable");
        }
    }

    #[test]
    fn install_from_targz_errors_when_the_member_is_absent() {
        let dir = tempfile::tempdir().unwrap();
        let plan = helm_install("v3.16.2", &Platform { os: "linux", arch: "amd64" }, dir.path());
        let archive = make_targz(&[("linux-amd64/NOT_HELM", b"x")]);
        let fetch = net(&[
            (plan.sha256_url.as_str(), sha256_hex(&archive).as_bytes()),
            (plan.archive_url.as_str(), &archive),
        ]);
        assert!(matches!(
            install_from_targz(&plan, &fetch),
            Err(InstallError::MemberNotFound { .. })
        ));
        assert!(!plan.target.exists());
    }

    #[test]
    fn install_from_targz_refuses_a_tampered_archive() {
        let dir = tempfile::tempdir().unwrap();
        let plan = helm_install("v3.16.2", &Platform { os: "linux", arch: "amd64" }, dir.path());
        let archive = make_targz(&[(plan.member.as_str(), b"payload")]);
        let fetch = net(&[
            (plan.sha256_url.as_str(), sha256_hex(b"different bytes").as_bytes()),
            (plan.archive_url.as_str(), &archive),
        ]);
        assert!(matches!(
            install_from_targz(&plan, &fetch),
            Err(InstallError::ChecksumMismatch { .. })
        ));
        assert!(!plan.target.exists());
    }
}
