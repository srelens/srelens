//! Tests for `srelens-tui update`.
//!
//! No stable release carries the TUI archives yet — #444 only just landed — so
//! the download-verify-replace path cannot be proven against a real release.
//! It is proven here instead: the archives are built in the test, hashed with
//! the same function the code uses, and served through the injected `fetch`.
//! Nothing reaches the network.

use std::path::{Path, PathBuf};

use srelens_tui::self_update::{
    apply, asset_name, asset_url, checksum_for, extract_binary, is_newer, package_manager_for,
    parse_latest_version, parse_newest_version, plan, replace_running_binary, sums_name,
    triple_for, verify_sha256, Channel, Check, Plan, UpdateError, LATEST_RELEASE_URL, RELEASES_URL,
};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// A gzipped tar holding one file at the archive root, the way the release
/// workflow builds it (`tar -czf … -C dir .`, so members arrive as `./name`).
fn targz(name: &str, contents: &[u8]) -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_size(contents.len() as u64);
    header.set_mode(0o755);
    header.set_cksum();
    tar.append_data(&mut header, format!("./{name}"), contents)
        .expect("append");
    let tar = tar.into_inner().expect("finish tar");

    use std::io::Write;
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gz.write_all(&tar).expect("gzip");
    gz.finish().expect("finish gzip")
}

/// A zip holding one file at the root, as the Windows leg builds it.
fn zip_with(name: &str, contents: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    zip.start_file::<_, ()>(name, zip::write::SimpleFileOptions::default())
        .expect("start");
    zip.write_all(contents).expect("write");
    zip.finish().expect("finish").into_inner()
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

fn release_json(tag: &str) -> Vec<u8> {
    format!(r#"{{"tag_name":"{tag}","prerelease":false}}"#).into_bytes()
}

/// The binary name inside an archive for the platform under test.
fn bin_name() -> &'static str {
    if cfg!(windows) {
        "srelens-tui.exe"
    } else {
        "srelens-tui"
    }
}

/// An archive of the right kind for the platform under test, carrying `body`.
fn archive_for(asset: &str, body: &[u8]) -> Vec<u8> {
    if asset.ends_with(".zip") {
        zip_with(bin_name(), body)
    } else {
        targz(bin_name(), body)
    }
}

// ---------------------------------------------------------------------------
// Platform → asset
// ---------------------------------------------------------------------------

#[test]
fn every_target_the_release_builds_has_a_triple_and_nothing_else_does() {
    // The seven legs in .github/workflows/release.yml, exactly.
    assert_eq!(
        triple_for("linux", "x86_64", false).unwrap(),
        "x86_64-unknown-linux-gnu"
    );
    assert_eq!(
        triple_for("linux", "x86_64", true).unwrap(),
        "x86_64-unknown-linux-musl"
    );
    assert_eq!(
        triple_for("linux", "aarch64", false).unwrap(),
        "aarch64-unknown-linux-gnu"
    );
    assert_eq!(
        triple_for("linux", "aarch64", true).unwrap(),
        "aarch64-unknown-linux-musl"
    );
    assert_eq!(
        triple_for("macos", "x86_64", false).unwrap(),
        "x86_64-apple-darwin"
    );
    assert_eq!(
        triple_for("macos", "aarch64", false).unwrap(),
        "aarch64-apple-darwin"
    );
    assert_eq!(
        triple_for("windows", "x86_64", false).unwrap(),
        "x86_64-pc-windows-msvc"
    );

    // No 32-bit and no arm64 Windows leg is built, so there is nothing to
    // point those at; saying so beats downloading the wrong architecture.
    for (os, arch) in [
        ("windows", "aarch64"),
        ("linux", "i686"),
        ("freebsd", "x86_64"),
    ] {
        assert!(
            matches!(
                triple_for(os, arch, false),
                Err(UpdateError::UnsupportedPlatform { .. })
            ),
            "{os}/{arch} should be unsupported"
        );
    }
}

/// A musl binary exists because the host cannot run the glibc one. Updating it
/// across flavours would hand the user something that no longer starts.
#[test]
fn a_musl_binary_updates_to_a_musl_archive() {
    assert!(triple_for("linux", "x86_64", true)
        .unwrap()
        .ends_with("musl"));
    assert!(triple_for("linux", "aarch64", true)
        .unwrap()
        .ends_with("musl"));
    assert!(triple_for("linux", "x86_64", false)
        .unwrap()
        .ends_with("gnu"));
}

#[test]
fn asset_names_match_what_the_release_workflow_publishes() {
    assert_eq!(
        asset_name("1.2.3", "x86_64-unknown-linux-gnu"),
        "srelens-tui-1.2.3-x86_64-unknown-linux-gnu.tar.gz"
    );
    // Windows is a zip, never a bare exe — size-baseline.mjs treats any .exe
    // as a desktop installer.
    assert_eq!(
        asset_name("1.2.3", "x86_64-pc-windows-msvc"),
        "srelens-tui-1.2.3-x86_64-pc-windows-msvc.zip"
    );
    assert_eq!(sums_name("1.2.3"), "srelens-tui-1.2.3-SHA256SUMS.txt");
    assert_eq!(
        asset_url("1.2.3", "srelens-tui-1.2.3-SHA256SUMS.txt"),
        "https://github.com/srelens/srelens/releases/download/srelens-v1.2.3/srelens-tui-1.2.3-SHA256SUMS.txt"
    );
}

// ---------------------------------------------------------------------------
// Channels
// ---------------------------------------------------------------------------

/// The default has to keep a dev user on dev. Offering a pre-release only the
/// stable channel would strand it: the stable release it already sits above is
/// not an update, so there would be nothing to install.
#[test]
fn the_default_channel_is_the_one_the_binary_came_from() {
    // Exactly the shape .github/workflows/release.yml gives a dev build.
    assert_eq!(Channel::of_version("0.8.1-152"), Channel::Dev);
    assert_eq!(Channel::of_version("1.0.0-1"), Channel::Dev);

    assert_eq!(Channel::of_version("0.8.0"), Channel::Stable);
    assert_eq!(Channel::of_version("1.2.3"), Channel::Stable);
    // Unparseable falls to stable rather than assuming someone is on dev.
    assert_eq!(Channel::of_version("nightly"), Channel::Stable);
}

#[test]
fn a_channel_can_be_named_on_the_command_line() {
    assert_eq!(Channel::parse("stable"), Some(Channel::Stable));
    assert_eq!(Channel::parse("dev"), Some(Channel::Dev));
    assert_eq!(Channel::parse("  DEV  "), Some(Channel::Dev));
    assert_eq!(Channel::parse("nightly"), None);
    assert_eq!(Channel::parse(""), None);
    assert_eq!(Channel::Stable.as_str(), "stable");
    assert_eq!(Channel::Dev.as_str(), "dev");
}

/// The dev channel reads the releases LIST, because pre-releases are what it
/// is made of and the stable endpoint hides them by definition.
#[test]
fn the_dev_channel_takes_the_newest_release_of_any_kind() {
    let body = br#"[
        {"tag_name":"srelens-v0.8.1-152","prerelease":true},
        {"tag_name":"srelens-v0.8.1-150","prerelease":true},
        {"tag_name":"srelens-v0.8.0","prerelease":false}
    ]"#;
    assert_eq!(parse_newest_version(body).unwrap(), "0.8.1-152");
}

/// The rolling `dev-channel` release is a permanent pre-release carrying only
/// the desktop updater's manifest — no TUI archives — so resolving to it would
/// build URLs for assets that are not there.
#[test]
fn the_dev_channel_skips_the_rolling_manifest_release() {
    let body = br#"[
        {"tag_name":"dev-channel","prerelease":true},
        {"tag_name":"some-other-tag","prerelease":true},
        {"tag_name":"srelens-v0.8.1-152","prerelease":true}
    ]"#;
    assert_eq!(parse_newest_version(body).unwrap(), "0.8.1-152");
}

#[test]
fn a_list_with_no_srelens_release_is_an_error_not_a_guess() {
    assert!(matches!(
        parse_newest_version(br#"[{"tag_name":"dev-channel"}]"#),
        Err(UpdateError::BadRelease(_))
    ));
    assert!(matches!(
        parse_newest_version(b"[]"),
        Err(UpdateError::BadRelease(_))
    ));
    assert!(matches!(
        parse_newest_version(b"not a list"),
        Err(UpdateError::BadRelease(_))
    ));
}

#[test]
fn each_channel_asks_its_own_endpoint() {
    let stable = |url: &str| -> Result<Vec<u8>, UpdateError> {
        assert_eq!(url, LATEST_RELEASE_URL);
        Ok(release_json("srelens-v0.8.0"))
    };
    assert!(matches!(
        plan("0.7.0", Channel::Stable, PathBuf::from("/tmp/x"), &stable).unwrap(),
        Check::Available(_)
    ));

    let dev = |url: &str| -> Result<Vec<u8>, UpdateError> {
        assert_eq!(url, RELEASES_URL);
        Ok(br#"[{"tag_name":"srelens-v0.8.1-152"}]"#.to_vec())
    };
    match plan("0.8.1-150", Channel::Dev, PathBuf::from("/tmp/x"), &dev).unwrap() {
        Check::Available(plan) => {
            assert_eq!(plan.latest, "0.8.1-152");
            // Pre-release versions order by their numeric identifier, so 152
            // is an update over 150 — a string comparison would agree here by
            // luck and disagree at 99 against 100.
            assert!(
                plan.archive_url.contains("/srelens-v0.8.1-152/"),
                "{}",
                plan.archive_url
            );
        }
        other => panic!("expected an update on the dev channel, got {other:?}"),
    }
}

/// A dev build checked against dev is up to date; the same build checked
/// against stable is ahead of it. Two different facts, two different answers.
#[test]
fn a_dev_build_is_up_to_date_on_dev_and_ahead_on_stable() {
    let dev = |_: &str| -> Result<Vec<u8>, UpdateError> {
        Ok(br#"[{"tag_name":"srelens-v0.8.1-152"}]"#.to_vec())
    };
    assert_eq!(
        plan("0.8.1-152", Channel::Dev, PathBuf::from("/tmp/x"), &dev).unwrap(),
        Check::UpToDate {
            channel: Channel::Dev,
            latest: "0.8.1-152".into()
        }
    );

    let stable = |_: &str| -> Result<Vec<u8>, UpdateError> { Ok(release_json("srelens-v0.8.0")) };
    assert_eq!(
        plan(
            "0.8.1-152",
            Channel::Stable,
            PathBuf::from("/tmp/x"),
            &stable
        )
        .unwrap(),
        Check::AheadOfChannel {
            channel: Channel::Stable,
            latest: "0.8.0".into()
        }
    );
}

// ---------------------------------------------------------------------------
// Release metadata
// ---------------------------------------------------------------------------

#[test]
fn the_release_tag_is_read_without_its_prefix() {
    assert_eq!(
        parse_latest_version(&release_json("srelens-v0.9.1")).unwrap(),
        "0.9.1"
    );
}

#[test]
fn an_unreadable_release_response_is_an_error_not_a_guess() {
    for body in [
        &b"not json"[..],
        br#"{"prerelease":false}"#,
        br#"{"tag_name":"v1.0.0"}"#,
        br#"{"tag_name":"srelens-v"}"#,
    ] {
        assert!(
            matches!(parse_latest_version(body), Err(UpdateError::BadRelease(_))),
            "{:?} should not parse",
            String::from_utf8_lossy(body)
        );
    }
}

/// A tag that is not a version must be reported as bad metadata. Accepting
/// it would fail the later comparison, and that failure reads as "nothing
/// newer" — so a broken release would tell the user they are up to date.
#[test]
fn a_release_tag_that_is_not_a_version_is_rejected() {
    for tag in [
        "srelens-vnightly",
        "srelens-vlatest",
        "srelens-v1.2",
        "srelens-v",
    ] {
        assert!(
            matches!(
                parse_latest_version(&release_json(tag)),
                Err(UpdateError::BadRelease(_))
            ),
            "{tag} should be refused"
        );
    }
    // A pre-release version is still a version.
    assert_eq!(
        parse_latest_version(&release_json("srelens-v0.8.1-152")).unwrap(),
        "0.8.1-152"
    );
}

/// The list endpoint is the other way round: there is a next entry to try, so
/// one unreadable tag should not stop a dev user updating.
#[test]
fn the_dev_channel_skips_a_tag_it_cannot_read_and_takes_the_next() {
    let body = br#"[
        {"tag_name":"srelens-vnightly"},
        {"tag_name":"srelens-v0.8.1-152"}
    ]"#;
    assert_eq!(parse_newest_version(body).unwrap(), "0.8.1-152");
}

// ---------------------------------------------------------------------------
// Temporary files
// ---------------------------------------------------------------------------

/// The staged file is created with `create_new`, so a path planted in
/// advance cannot be written through. On Unix that is the difference between
/// truncating a symlink's target and refusing; the same guard is what stops
/// a stale leftover being reused on any platform.
#[cfg(unix)]
#[test]
fn a_planted_symlink_beside_the_binary_is_not_written_through() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join(bin_name());
    std::fs::write(&target, b"old").unwrap();

    // What an attacker with write access to the directory would leave: a
    // link named the way the updater's staging file used to be named.
    let victim = dir.path().join("private-file");
    std::fs::write(&victim, b"do not truncate me").unwrap();
    let planted = dir
        .path()
        .join(format!(".{}.new-{}", bin_name(), std::process::id()));
    std::os::unix::fs::symlink(&victim, &planted).unwrap();

    replace_running_binary(&target, b"new").expect("the update still succeeds");

    assert_eq!(
        std::fs::read(&victim).unwrap(),
        b"do not truncate me",
        "the linked file must be untouched"
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"new");
}

// ---------------------------------------------------------------------------
// Checksums
// ---------------------------------------------------------------------------

/// The release writes sums on Linux (two spaces); a developer regenerating
/// them on a system defaulting to binary mode writes `*name`. Both must read.
#[test]
fn a_checksum_is_found_in_either_sha256sum_format() {
    let hash = "a".repeat(64);
    let asset = "srelens-tui-1.2.3-x86_64-unknown-linux-gnu.tar.gz";
    let gnu = format!("{hash}  {asset}\n{}  other.tar.gz\n", "b".repeat(64));
    let binary_mode = format!("{hash} *{asset}\n");

    assert_eq!(checksum_for(&gnu, asset).unwrap(), hash);
    assert_eq!(checksum_for(&binary_mode, asset).unwrap(), hash);
}

#[test]
fn a_checksum_file_that_does_not_list_the_asset_is_refused() {
    let sums = format!("{}  some-other-file.tar.gz\n", "a".repeat(64));
    assert!(matches!(
        checksum_for(&sums, "srelens-tui-1.2.3-x86_64-apple-darwin.tar.gz"),
        Err(UpdateError::ChecksumMissing { .. })
    ));
    // A line for the right asset carrying something that is not a hash is the
    // same failure: there is nothing to verify against.
    let malformed = "not-a-hash  wanted.tar.gz\n";
    assert!(matches!(
        checksum_for(malformed, "wanted.tar.gz"),
        Err(UpdateError::ChecksumMissing { .. })
    ));
}

#[test]
fn verification_accepts_the_published_hash_and_rejects_any_other() {
    let bytes = b"payload";
    let hash = sha256_hex(bytes);
    assert!(verify_sha256(bytes, &hash).is_ok());
    assert!(
        verify_sha256(bytes, &hash.to_uppercase()).is_ok(),
        "case-insensitive"
    );
    assert!(matches!(
        verify_sha256(b"different", &hash),
        Err(UpdateError::ChecksumMismatch { .. })
    ));
}

// ---------------------------------------------------------------------------
// Archives
// ---------------------------------------------------------------------------

#[test]
fn the_binary_is_pulled_out_of_a_tarball_stored_at_the_root() {
    let archive = targz("srelens-tui", b"ELF-ish");
    let got = extract_binary(
        &archive,
        "srelens-tui-1.2.3-x86_64-unknown-linux-gnu.tar.gz",
    );
    assert_eq!(got.unwrap(), b"ELF-ish");
}

#[test]
fn the_binary_is_pulled_out_of_a_zip() {
    let archive = zip_with("srelens-tui.exe", b"MZ-ish");
    let got = extract_binary(&archive, "srelens-tui-1.2.3-x86_64-pc-windows-msvc.zip");
    assert_eq!(got.unwrap(), b"MZ-ish");
}

#[test]
fn an_archive_without_the_binary_is_an_error_naming_the_asset() {
    let archive = targz("README", b"nope");
    let asset = "srelens-tui-1.2.3-x86_64-unknown-linux-gnu.tar.gz";
    match extract_binary(&archive, asset) {
        Err(UpdateError::BinaryMissing { asset: named }) => assert_eq!(named, asset),
        other => panic!("expected BinaryMissing, got {other:?}"),
    }
}

#[test]
fn a_corrupt_archive_reports_the_archive_not_a_panic() {
    assert!(matches!(
        extract_binary(b"not a tarball at all", "x-1.0.0-linux.tar.gz"),
        Err(UpdateError::Archive(_)) | Err(UpdateError::BinaryMissing { .. })
    ));
    assert!(matches!(
        extract_binary(b"not a zip at all", "x-1.0.0-windows.zip"),
        Err(UpdateError::Archive(_))
    ));
}

// ---------------------------------------------------------------------------
// Version comparison
// ---------------------------------------------------------------------------

#[test]
fn versions_compare_as_semver_not_as_text() {
    // The case a string comparison gets backwards, and the reason semver is
    // parsed at all.
    assert!(is_newer("0.9.0", "0.10.0"));
    assert!(!is_newer("0.10.0", "0.9.0"));

    assert!(is_newer("1.2.3", "1.2.4"));
    assert!(!is_newer("1.2.3", "1.2.3"), "equal is not newer");
    assert!(!is_newer("1.2.4", "1.2.3"));
    // A pre-release sorts below its release, so a dev build is not offered an
    // "update" to the stable it already contains.
    assert!(is_newer("1.2.3-99", "1.2.3"));
}

#[test]
fn an_unparseable_version_never_triggers_an_update() {
    assert!(!is_newer("not-a-version", "1.0.0"));
    assert!(!is_newer("1.0.0", "not-a-version"));
}

// ---------------------------------------------------------------------------
// Package-manager ownership
// ---------------------------------------------------------------------------

#[test]
fn a_binary_a_package_manager_owns_is_recognised() {
    for (path, manager) in [
        ("/opt/homebrew/bin/srelens-tui", "Homebrew"),
        (
            "/usr/local/Cellar/srelens-tui/1.0.0/bin/srelens-tui",
            "Homebrew",
        ),
        (
            "/usr/bin/srelens-tui",
            "your distribution's package manager",
        ),
        ("/snap/srelens/current/bin/srelens-tui", "snap"),
        ("/nix/store/abc-srelens/bin/srelens-tui", "Nix"),
        (r"C:\Users\me\scoop\shims\srelens-tui.exe", "Scoop"),
        (
            r"C:\Users\me\AppData\Local\Microsoft\WinGet\Packages\x\srelens-tui.exe",
            "winget",
        ),
    ] {
        assert_eq!(
            package_manager_for(Path::new(path)),
            Some(manager),
            "{path}"
        );
    }
}

/// The locations the install guide tells people to use by hand. Reporting one
/// of these as package-managed would refuse to update the ordinary install.
#[test]
fn a_hand_installed_binary_is_not_mistaken_for_a_managed_one() {
    for path in [
        "/usr/local/bin/srelens-tui",
        "/home/me/.local/bin/srelens-tui",
        "/home/me/bin/srelens-tui",
        r"C:\Users\me\bin\srelens-tui.exe",
    ] {
        assert_eq!(package_manager_for(Path::new(path)), None, "{path}");
    }
}

// ---------------------------------------------------------------------------
// Planning
// ---------------------------------------------------------------------------

#[test]
fn being_up_to_date_is_a_quiet_success_not_an_error() {
    let fetch = |url: &str| -> Result<Vec<u8>, UpdateError> {
        assert_eq!(url, LATEST_RELEASE_URL);
        Ok(release_json("srelens-v1.0.0"))
    };
    assert_eq!(
        plan(
            "1.0.0",
            Channel::Stable,
            PathBuf::from("/tmp/srelens-tui"),
            &fetch
        )
        .unwrap(),
        Check::UpToDate {
            channel: Channel::Stable,
            latest: "1.0.0".into()
        }
    );
}

/// Only the STABLE endpoint is consulted, so a dev build sorting above the
/// newest stable has not been told it is the latest of anything. Reporting it
/// as up to date would be a claim about pre-releases nobody checked.
#[test]
fn a_build_ahead_of_stable_is_not_reported_as_up_to_date() {
    let fetch = |_: &str| -> Result<Vec<u8>, UpdateError> { Ok(release_json("srelens-v0.8.0")) };
    // Exactly the shape the dev channel produces: 0.8.1-152 sorts above 0.8.0.
    assert_eq!(
        plan(
            "0.8.1-152",
            Channel::Stable,
            PathBuf::from("/tmp/srelens-tui"),
            &fetch
        )
        .unwrap(),
        Check::AheadOfChannel {
            channel: Channel::Stable,
            latest: "0.8.0".into()
        }
    );

    // A pre-release of the version that IS the latest stable sorts BELOW it,
    // so that one is a real update rather than being ahead.
    let fetch = |_: &str| -> Result<Vec<u8>, UpdateError> { Ok(release_json("srelens-v0.8.0")) };
    assert!(matches!(
        plan(
            "0.8.0-7",
            Channel::Stable,
            PathBuf::from("/tmp/srelens-tui"),
            &fetch
        )
        .unwrap(),
        Check::Available(_)
    ));
}

/// A version that cannot be parsed is reported as up to date, never as ahead:
/// claiming to be ahead of a release we could not compare against is the same
/// overclaim in the other direction.
#[test]
fn an_unparseable_current_version_is_not_claimed_to_be_ahead() {
    let fetch = |_: &str| -> Result<Vec<u8>, UpdateError> { Ok(release_json("srelens-v1.0.0")) };
    assert_eq!(
        plan(
            "nightly",
            Channel::Stable,
            PathBuf::from("/tmp/srelens-tui"),
            &fetch
        )
        .unwrap(),
        Check::UpToDate {
            channel: Channel::Stable,
            latest: "1.0.0".into()
        }
    );
}

#[test]
fn a_newer_release_plans_urls_under_its_own_tag() {
    let fetch = |_: &str| -> Result<Vec<u8>, UpdateError> { Ok(release_json("srelens-v2.0.0")) };
    let plan = match plan(
        "1.0.0",
        Channel::Stable,
        PathBuf::from("/tmp/srelens-tui"),
        &fetch,
    )
    .unwrap()
    {
        Check::Available(plan) => *plan,
        other => panic!("expected an update, got {other:?}"),
    };

    assert_eq!(plan.current, "1.0.0");
    assert_eq!(plan.latest, "2.0.0");
    assert!(
        plan.archive_url.contains("/srelens-v2.0.0/") && plan.archive_url.ends_with(&plan.asset),
        "{}",
        plan.archive_url
    );
    assert!(
        plan.sums_url.ends_with("srelens-tui-2.0.0-SHA256SUMS.txt"),
        "{}",
        plan.sums_url
    );
}

#[test]
fn a_failed_release_lookup_is_reported_rather_than_swallowed() {
    let fetch = |_: &str| -> Result<Vec<u8>, UpdateError> {
        Err(UpdateError::Download("503 Service Unavailable".into()))
    };
    assert!(matches!(
        plan(
            "1.0.0",
            Channel::Stable,
            PathBuf::from("/tmp/srelens-tui"),
            &fetch
        ),
        Err(UpdateError::Download(_))
    ));
}

// ---------------------------------------------------------------------------
// Applying — the part no release can exercise yet
// ---------------------------------------------------------------------------

/// Build a plan whose target is a real file in `dir`, plus a fetch that serves
/// a matching archive and checksum file.
fn staged(
    dir: &Path,
    body: &'static [u8],
) -> (Plan, impl Fn(&str) -> Result<Vec<u8>, UpdateError>) {
    let triple = triple_for(
        std::env::consts::OS,
        std::env::consts::ARCH,
        cfg!(target_env = "musl"),
    )
    .expect("this platform has a release target");
    let asset = asset_name("2.0.0", triple);
    let target = dir.join(bin_name());
    std::fs::write(&target, b"the old binary").expect("seed the installed binary");

    let archive = archive_for(&asset, body);
    let sums = format!("{}  {}\n", sha256_hex(&archive), asset);
    let plan = Plan {
        current: "1.0.0".into(),
        latest: "2.0.0".into(),
        archive_url: asset_url("2.0.0", &asset),
        sums_url: asset_url("2.0.0", &sums_name("2.0.0")),
        asset,
        target,
    };
    let sums_url = plan.sums_url.clone();
    let fetch = move |url: &str| -> Result<Vec<u8>, UpdateError> {
        if url == sums_url {
            Ok(sums.clone().into_bytes())
        } else {
            Ok(archive.clone())
        }
    };
    (plan, fetch)
}

#[test]
fn a_verified_download_replaces_the_installed_binary() {
    let dir = tempfile::tempdir().unwrap();
    let (plan, fetch) = staged(dir.path(), b"the new binary");

    apply(&plan, &fetch).expect("the update applies");

    assert_eq!(
        std::fs::read(&plan.target).unwrap(),
        b"the new binary",
        "the installed binary is the one from the archive"
    );
    // Nothing staged is left lying around next to it.
    let strays: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n != bin_name())
        .collect();
    assert!(strays.is_empty(), "left behind: {strays:?}");
}

/// The property the whole design exists for: a download that does not match
/// its published checksum must not reach the path the user runs.
#[test]
fn a_download_that_fails_verification_leaves_the_old_binary_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let (plan, _) = staged(dir.path(), b"unused");
    let asset = plan.asset.clone();
    let sums_url = plan.sums_url.clone();
    // The checksum file names a hash the archive does not have — what a
    // substituted or truncated download looks like.
    let fetch = move |url: &str| -> Result<Vec<u8>, UpdateError> {
        if url == sums_url {
            Ok(format!("{}  {}\n", "a".repeat(64), asset).into_bytes())
        } else {
            Ok(archive_for(&asset, b"tampered"))
        }
    };

    match apply(&plan, &fetch) {
        Err(UpdateError::ChecksumMismatch { .. }) => {}
        other => panic!("expected a checksum mismatch, got {other:?}"),
    }
    assert_eq!(
        std::fs::read(&plan.target).unwrap(),
        b"the old binary",
        "the installed binary must be untouched"
    );
}

#[test]
fn a_binary_a_package_manager_owns_is_refused_before_anything_is_downloaded() {
    let dir = tempfile::tempdir().unwrap();
    let (mut plan, _) = staged(dir.path(), b"unused");
    plan.target = PathBuf::from("/opt/homebrew/bin").join(bin_name());
    let fetch = |_: &str| -> Result<Vec<u8>, UpdateError> {
        panic!("nothing should be downloaded for a package-managed binary")
    };

    match apply(&plan, &fetch) {
        Err(UpdateError::PackageManaged { manager, .. }) => assert_eq!(manager, "Homebrew"),
        other => panic!("expected PackageManaged, got {other:?}"),
    }
}

#[test]
fn replacing_the_binary_is_atomic_from_the_readers_point_of_view() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join(bin_name());
    std::fs::write(&target, b"old").unwrap();

    replace_running_binary(&target, b"new").expect("replace");
    assert_eq!(std::fs::read(&target).unwrap(), b"new");

    // Twice in a row: on Windows the second call has to cope with the `.old`
    // file the first one could not delete while the image was open.
    replace_running_binary(&target, b"newer").expect("replace again");
    assert_eq!(std::fs::read(&target).unwrap(), b"newer");
}

#[cfg(unix)]
#[test]
fn the_installed_binary_is_executable() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join(bin_name());
    std::fs::write(&target, b"old").unwrap();

    replace_running_binary(&target, b"new").expect("replace");
    let mode = std::fs::metadata(&target).unwrap().permissions().mode();
    assert_eq!(mode & 0o111, 0o111, "mode was {mode:o} — not executable");
}
