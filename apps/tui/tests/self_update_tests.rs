//! Tests for `srelens-tui update`.
//!
//! No stable release carries the TUI archives yet — #444 only just landed — so
//! the download-verify-replace path cannot be proven against a real release.
//! It is proven here instead: the archives are built in the test, hashed with
//! the same function the code uses, and served through the injected `fetch`.
//! Nothing reaches the network.

use std::path::{Path, PathBuf};

use srelens_tui::self_update::{
    apply, apply_with_keys, asset_name, asset_url, checksum_for, extract_binary, is_newer,
    package_manager_for, parse_latest_version, parse_newest_version, plan, replace_running_binary,
    sums_name, sums_signature_name, triple_for, verify_sha256, Channel, Check, Plan, UpdateError,
    LATEST_RELEASE_URL, RELEASES_URL,
};
use srelens_tui::update_signature::SignatureProblem;

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

/// The triple under test, so fixtures name assets this platform would want.
fn here() -> &'static str {
    triple_for(
        std::env::consts::OS,
        std::env::consts::ARCH,
        cfg!(target_env = "musl"),
    )
    .expect("this platform has a release target")
}

/// The asset names a release must carry for `version` to be installable
/// here, as the GitHub API would list them: the archive, the checksum file,
/// and the checksum file's signature.
fn assets_for(version: &str) -> String {
    format!(
        r#"[{{"name":"{}"}},{{"name":"{}"}},{{"name":"{}"}}]"#,
        asset_name(version, here()),
        sums_name(version),
        sums_signature_name(version)
    )
}

/// The same release before signing reached it: archive and checksums only.
fn unsigned_assets_for(version: &str) -> String {
    format!(
        r#"[{{"name":"{}"}},{{"name":"{}"}}]"#,
        asset_name(version, here()),
        sums_name(version)
    )
}

/// A releases-list response. Each entry is (tag, is_prerelease, carries the
/// archives) — the three things the dev channel filters on.
fn releases_json(entries: &[(&str, bool, bool)]) -> Vec<u8> {
    let items: Vec<String> = entries
        .iter()
        .map(|(tag, prerelease, complete)| {
            let version = tag.strip_prefix("srelens-v").unwrap_or("0.0.0");
            let assets = if *complete {
                assets_for(version)
            } else {
                "[]".to_string()
            };
            format!(r#"{{"tag_name":"{tag}","prerelease":{prerelease},"assets":{assets}}}"#)
        })
        .collect();
    format!("[{}]", items.join(",")).into_bytes()
}

/// A single-release response carrying the assets an update needs.
fn release_json(tag: &str) -> Vec<u8> {
    let version = tag.strip_prefix("srelens-v").unwrap_or("0.0.0");
    format!(
        r#"{{"tag_name":"{tag}","prerelease":false,"assets":{}}}"#,
        assets_for(version)
    )
    .into_bytes()
}

/// The binary name inside an archive for the platform under test.
fn bin_name() -> &'static str {
    if cfg!(windows) {
        "srectl.exe"
    } else {
        "srectl"
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

/// `parse_latest_version` for this platform.
fn parse_latest_version_here(body: &[u8]) -> Result<String, UpdateError> {
    parse_latest_version(body, here())
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
        "srectl-1.2.3-x86_64-unknown-linux-gnu.tar.gz"
    );
    // Windows is a zip, never a bare exe — size-baseline.mjs treats any .exe
    // as a desktop installer.
    assert_eq!(
        asset_name("1.2.3", "x86_64-pc-windows-msvc"),
        "srectl-1.2.3-x86_64-pc-windows-msvc.zip"
    );
    assert_eq!(sums_name("1.2.3"), "srectl-1.2.3-SHA256SUMS.txt");
    assert_eq!(
        asset_url("1.2.3", "srectl-1.2.3-SHA256SUMS.txt"),
        "https://github.com/srelens/srelens/releases/download/srelens-v1.2.3/srectl-1.2.3-SHA256SUMS.txt"
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
    let body = releases_json(&[
        ("srelens-v0.8.1-152", true, true),
        ("srelens-v0.8.1-150", true, true),
        ("srelens-v0.8.0", false, false),
    ]);
    assert_eq!(parse_newest_version(&body, here()).unwrap(), "0.8.1-152");
}

/// The rolling `dev-channel` release is a permanent pre-release carrying only
/// the desktop updater's manifest — no TUI archives — so resolving to it would
/// build URLs for assets that are not there.
#[test]
fn the_dev_channel_skips_the_rolling_manifest_release() {
    let body = releases_json(&[
        ("dev-channel", true, false),
        ("some-other-tag", true, false),
        ("srelens-v0.8.1-152", true, true),
    ]);
    assert_eq!(parse_newest_version(&body, here()).unwrap(), "0.8.1-152");
}

/// When main cuts a stable release it becomes the newest entry in this
/// list. Taking it would move a dev user onto stable without saying so —
/// and it would stick, because the installed version would no longer carry
/// a pre-release, so the next plain `update` would default to stable.
#[test]
fn the_dev_channel_does_not_offer_a_stable_release() {
    let body = releases_json(&[
        ("srelens-v0.9.0", false, false),
        ("srelens-v0.8.1-152", true, true),
    ]);
    assert_eq!(parse_newest_version(&body, here()).unwrap(), "0.8.1-152");

    // And a list of nothing but stable releases has no dev build to offer.
    let stable_only = releases_json(&[("srelens-v0.9.0", false, true)]);
    assert!(matches!(
        parse_newest_version(&stable_only, here()),
        Err(UpdateError::BadRelease(_))
    ));
}

#[test]
fn a_list_with_no_srelens_release_is_an_error_not_a_guess() {
    assert!(matches!(
        parse_newest_version(br#"[{"tag_name":"dev-channel"}]"#, here()),
        Err(UpdateError::BadRelease(_))
    ));
    assert!(matches!(
        parse_newest_version(b"[]", here()),
        Err(UpdateError::BadRelease(_))
    ));
    assert!(matches!(
        parse_newest_version(b"not a list", here()),
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
        plan(
            "0.7.0",
            Channel::Stable,
            false,
            PathBuf::from("/tmp/x"),
            &stable
        )
        .unwrap(),
        Check::Available(_)
    ));

    let dev = |url: &str| -> Result<Vec<u8>, UpdateError> {
        assert_eq!(url, RELEASES_URL);
        Ok(releases_json(&[("srelens-v0.8.1-152", true, true)]))
    };
    match plan(
        "0.8.1-150",
        Channel::Dev,
        false,
        PathBuf::from("/tmp/x"),
        &dev,
    )
    .unwrap()
    {
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

/// Naming a channel means asking to be ON it, even when that goes backwards.
///
/// The usual case rather than a corner: any dev build sorts above the
/// stable release it was cut after, so without this `--channel stable`
/// could never perform the switch the install guide promises.
#[test]
fn naming_the_stable_channel_from_a_dev_build_installs_stable() {
    let fetch = |_: &str| -> Result<Vec<u8>, UpdateError> { Ok(release_json("srelens-v0.8.0")) };

    // Asked for: the downgrade is the point.
    match plan(
        "0.8.1-152",
        Channel::Stable,
        true,
        PathBuf::from("/tmp/x"),
        &fetch,
    )
    .unwrap()
    {
        Check::Available(plan) => assert_eq!(plan.latest, "0.8.0"),
        other => panic!("expected the switch to be planned, got {other:?}"),
    }

    // Not asked for: falling into stable by default is no reason to move
    // someone backwards.
    assert_eq!(
        plan(
            "0.8.1-152",
            Channel::Stable,
            false,
            PathBuf::from("/tmp/x"),
            &fetch
        )
        .unwrap(),
        Check::AheadOfChannel {
            channel: Channel::Stable,
            latest: "0.8.0".into()
        }
    );

    // Already on it, asked for or not: nothing to do.
    let fetch = |_: &str| -> Result<Vec<u8>, UpdateError> { Ok(release_json("srelens-v1.0.0")) };
    assert_eq!(
        plan(
            "1.0.0",
            Channel::Stable,
            true,
            PathBuf::from("/tmp/x"),
            &fetch
        )
        .unwrap(),
        Check::UpToDate {
            channel: Channel::Stable,
            latest: "1.0.0".into()
        }
    );
}

/// A dev build checked against dev is up to date; the same build checked
/// against stable is ahead of it. Two different facts, two different answers.
#[test]
fn a_dev_build_is_up_to_date_on_dev_and_ahead_on_stable() {
    let dev = |_: &str| -> Result<Vec<u8>, UpdateError> {
        Ok(releases_json(&[("srelens-v0.8.1-152", true, true)]))
    };
    assert_eq!(
        plan(
            "0.8.1-152",
            Channel::Dev,
            false,
            PathBuf::from("/tmp/x"),
            &dev
        )
        .unwrap(),
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
            false,
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
        parse_latest_version_here(&release_json("srelens-v0.9.1")).unwrap(),
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
            matches!(
                parse_latest_version(body, here()),
                Err(UpdateError::BadRelease(_))
            ),
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
                parse_latest_version_here(&release_json(tag)),
                Err(UpdateError::BadRelease(_))
            ),
            "{tag} should be refused"
        );
    }
    // A pre-release version is still a version.
    assert_eq!(
        parse_latest_version_here(&release_json("srelens-v0.8.1-152")).unwrap(),
        "0.8.1-152"
    );
}

/// The list endpoint is the other way round: there is a next entry to try, so
/// one unreadable tag should not stop a dev user updating.
#[test]
fn the_dev_channel_skips_a_tag_it_cannot_read_and_takes_the_next() {
    let body = releases_json(&[
        ("srelens-vnightly", true, false),
        ("srelens-v0.8.1-152", true, true),
    ]);
    assert_eq!(parse_newest_version(&body, here()).unwrap(), "0.8.1-152");
}

// ---------------------------------------------------------------------------
// Temporary files
// ---------------------------------------------------------------------------

/// The staged file is created with `create_new`, so a path planted in
/// advance cannot be written through. On Unix that is the difference between
/// truncating a symlink's target and refusing; the same guard is what stops
/// a stale leftover being reused on any platform.
/// A staged download must not survive a failure. Unix only because the
/// failure has to be forced, and a directory where the binary belongs makes
/// the final rename fail there; on Windows the same setup renames the
/// directory aside instead and succeeds.
///
/// This matters more than it reads: the staged names are random, so a leak
/// accumulates a full copy of the binary per attempt rather than reusing one
/// path.
#[cfg(unix)]
#[test]
fn a_failed_replacement_leaves_nothing_staged_behind() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join(bin_name());
    std::fs::create_dir(&target).expect("a directory where the binary should be");

    let result = replace_running_binary(&target, b"new");
    assert!(result.is_err(), "renaming onto a directory must fail");

    let strays: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name != bin_name())
        .collect();
    assert!(strays.is_empty(), "left behind: {strays:?}");
}

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

/// A dev pre-release goes public the moment it builds, so if the TUI matrix
/// or `tui-publish` fails afterwards the tag is out there with no archives on
/// it. Offering it would promise an update and then 404 on the download.
/// This is not hypothetical: every release cut before the TUI shipped looks
/// exactly like that, and the dev check offered one.
#[test]
fn a_release_without_the_archives_is_passed_over_for_the_last_complete_one() {
    let body = format!(
        r#"[
        {{"tag_name":"srelens-v0.8.1-152","prerelease":true,"assets":[]}},
        {{"tag_name":"srelens-v0.8.1-150","prerelease":true,"assets":{}}}
    ]"#,
        assets_for("0.8.1-150")
    );
    assert_eq!(
        parse_newest_version(body.as_bytes(), here()).unwrap(),
        "0.8.1-150"
    );
}

/// Half a release is no better than none: the archive without the checksum
/// file cannot be verified, so it is not installable either.
#[test]
fn a_release_missing_only_the_checksum_file_is_also_passed_over() {
    let body = format!(
        r#"[{{"tag_name":"srelens-v0.8.1-152","prerelease":true,"assets":[{{"name":"{}"}}]}}]"#,
        asset_name("0.8.1-152", here())
    );
    assert!(matches!(
        parse_newest_version(body.as_bytes(), here()),
        Err(UpdateError::BadRelease(_))
    ));
}

/// Dev pre-releases are public before `sign-artifacts` runs, and signing
/// them is best-effort, so an unsigned one is expected rather than suspicious.
/// It is passed over, as an incomplete one is, for the newest that IS signed:
/// the dev channel installs signed builds only.
#[test]
fn the_dev_channel_passes_over_a_release_whose_checksums_are_not_signed() {
    let body = format!(
        r#"[
        {{"tag_name":"srelens-v0.8.1-152","prerelease":true,"assets":{}}},
        {{"tag_name":"srelens-v0.8.1-150","prerelease":true,"assets":{}}}
    ]"#,
        unsigned_assets_for("0.8.1-152"),
        assets_for("0.8.1-150")
    );
    assert_eq!(
        parse_newest_version(body.as_bytes(), here()).unwrap(),
        "0.8.1-150"
    );
}

/// A stable release is published only after signing succeeds, so one without
/// a signature is not a release to wait out. It is refused, by name.
#[test]
fn a_stable_release_whose_checksums_are_not_signed_is_refused_by_name() {
    let body = format!(
        r#"{{"tag_name":"srelens-v0.9.0","prerelease":false,"assets":{}}}"#,
        unsigned_assets_for("0.9.0")
    );
    match parse_latest_version(body.as_bytes(), here()) {
        Err(UpdateError::BadRelease(why)) => {
            assert!(why.contains("srelens-v0.9.0"), "{why}");
            assert!(why.contains("signature"), "{why}");
        }
        other => panic!("expected the unsigned release to be refused, got {other:?}"),
    }
}

/// On stable there is nothing to fall back to, so the same situation is
/// reported instead of skipped — a named reason now beats a 404 later.
#[test]
fn a_stable_release_without_a_build_for_this_platform_says_so() {
    let body = br#"{"tag_name":"srelens-v0.9.0","prerelease":false,"assets":[]}"#;
    match parse_latest_version(body, here()) {
        Err(UpdateError::BadRelease(why)) => {
            assert!(why.contains("srelens-v0.9.0"), "{why}");
            assert!(why.contains(here()), "{why}");
        }
        other => panic!("expected BadRelease, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Checksums
// ---------------------------------------------------------------------------

/// The release writes sums on Linux (two spaces); a developer regenerating
/// them on a system defaulting to binary mode writes `*name`. Both must read.
#[test]
fn a_checksum_is_found_in_either_sha256sum_format() {
    let hash = "a".repeat(64);
    let asset = "srectl-1.2.3-x86_64-unknown-linux-gnu.tar.gz";
    let gnu = format!("{hash}  {asset}\n{}  other.tar.gz\n", "b".repeat(64));
    let binary_mode = format!("{hash} *{asset}\n");

    assert_eq!(checksum_for(&gnu, asset).unwrap(), hash);
    assert_eq!(checksum_for(&binary_mode, asset).unwrap(), hash);
}

#[test]
fn a_checksum_file_that_does_not_list_the_asset_is_refused() {
    let sums = format!("{}  some-other-file.tar.gz\n", "a".repeat(64));
    assert!(matches!(
        checksum_for(&sums, "srectl-1.2.3-x86_64-apple-darwin.tar.gz"),
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
    let archive = targz("srectl", b"ELF-ish");
    let got = extract_binary(&archive, "srectl-1.2.3-x86_64-unknown-linux-gnu.tar.gz");
    assert_eq!(got.unwrap(), b"ELF-ish");
}

#[test]
fn the_binary_is_pulled_out_of_a_zip() {
    let archive = zip_with("srectl.exe", b"MZ-ish");
    let got = extract_binary(&archive, "srectl-1.2.3-x86_64-pc-windows-msvc.zip");
    assert_eq!(got.unwrap(), b"MZ-ish");
}

#[test]
fn an_archive_without_the_binary_is_an_error_naming_the_asset() {
    let archive = targz("README", b"nope");
    let asset = "srectl-1.2.3-x86_64-unknown-linux-gnu.tar.gz";
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
    ] {
        assert_eq!(
            package_manager_for(Path::new(path)),
            Some(manager),
            "{path}"
        );
    }
}

/// The Windows markers are Windows-only. Applied everywhere,
/// `/home/me/scoop/apps/…` on Linux read as Scoop-owned and the update was
/// refused before anything was downloaded — these names mean a package
/// manager on one platform and nothing in particular on the others.
#[cfg(windows)]
#[test]
fn a_windows_package_manager_is_recognised_at_any_depth_and_any_case() {
    for (path, manager) in [
        (
            r"C:\ProgramData\chocolatey\bin\srelens-tui.exe",
            "Chocolatey",
        ),
        (
            r"C:\PROGRAMDATA\CHOCOLATEY\bin\srelens-tui.exe",
            "Chocolatey",
        ),
        (
            r"C:\Users\me\Scoop\Apps\srelens-tui\current\srelens-tui.exe",
            "Scoop",
        ),
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

/// The same names on Unix mean nothing in particular, and refusing there
/// would block an update to a file its owner controls.
#[cfg(unix)]
#[test]
fn windows_package_markers_do_not_apply_on_unix() {
    for path in [
        "/home/me/scoop/apps/demo/srelens-tui",
        "/home/me/scoop/shims/srelens-tui",
        "/opt/chocolatey/srelens-tui",
    ] {
        assert_eq!(package_manager_for(Path::new(path)), None, "{path}");
    }
}

/// The layouts `brew install srelens/tap/srelens-tui` actually produces.
///
/// Homebrew installs into `<prefix>/Cellar/<formula>/<version>/bin` and links
/// that into `<prefix>/bin`, so what `apply` checks is the resolved Cellar
/// path — `/usr/local/bin` alone is where the install guide tells people to
/// put a copy by hand, and must stay updatable.
#[test]
fn a_homebrew_install_is_recognised_on_both_prefixes() {
    for path in [
        "/opt/homebrew/Cellar/srelens-tui/1.2.3/bin/srelens-tui",
        "/usr/local/Cellar/srelens-tui/1.2.3/bin/srelens-tui",
        "/home/linuxbrew/.linuxbrew/Cellar/srelens-tui/1.2.3/bin/srelens-tui",
        "/opt/homebrew/bin/srelens-tui",
        "/home/linuxbrew/.linuxbrew/bin/srelens-tui",
    ] {
        assert_eq!(
            package_manager_for(Path::new(path)),
            Some("Homebrew"),
            "{path}"
        );
    }

    // The hand-install location, which shares a prefix with Intel Homebrew
    // and must not be mistaken for it.
    assert_eq!(
        package_manager_for(Path::new("/usr/local/bin/srelens-tui")),
        None
    );
}

/// A package-manager root only counts at the START of the path.
///
/// An unpacked root filesystem, a container image being edited, a chroot
/// staging directory — all contain `/usr/bin/` partway through, and all
/// belong to whoever unpacked them. Matching the marker anywhere refused to
/// update a file its owner controls.
#[test]
fn a_package_root_buried_inside_another_path_is_not_its_owner() {
    for path in [
        "/home/me/rootfs/usr/bin/srelens-tui",
        "/home/me/containers/alpine/usr/bin/srelens-tui",
        "/tmp/extract/snap/srelens-tui",
        "/home/me/backup/nix/store/srelens-tui",
    ] {
        assert_eq!(package_manager_for(Path::new(path)), None, "{path}");
    }

    // The same markers at the front still count.
    assert_eq!(
        package_manager_for(Path::new("/usr/bin/srelens-tui")),
        Some("your distribution's package manager")
    );
}

/// Unix paths are case-sensitive, so a differently-cased lookalike is a
/// different directory and not the package manager's.
#[test]
fn a_unix_root_is_matched_case_sensitively() {
    assert_eq!(package_manager_for(Path::new("/USR/BIN/srelens-tui")), None);
    assert_eq!(package_manager_for(Path::new("/Snap/srelens-tui")), None);
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
            false,
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
            false,
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
            false,
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
            false,
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
        false,
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
        plan.sums_url.ends_with("srectl-2.0.0-SHA256SUMS.txt"),
        "{}",
        plan.sums_url
    );
}

/// The bridge release is still published as `srelens-tui`. Checking for an
/// update from that same version finds no `srectl` archive and is not a
/// failure: there is nothing newer to install.
#[test]
fn a_release_that_still_publishes_srelens_tui_is_current() {
    let fetch = |_: &str| -> Result<Vec<u8>, UpdateError> {
        Ok(br#"{"tag_name":"srelens-v1.2.0","prerelease":false,"assets":[{"name":"srelens-tui-1.2.0-SHA256SUMS.txt"}]}"#.to_vec())
    };
    assert_eq!(
        plan(
            "1.2.0",
            Channel::Stable,
            false,
            PathBuf::from("/tmp/srelens-tui"),
            &fetch
        )
        .unwrap(),
        Check::UpToDate {
            channel: Channel::Stable,
            latest: "1.2.0".into()
        }
    );
}

/// A newer tag that also lacks a `srectl` archive is a release we could not
/// take, not a claim that this build is the latest.
#[test]
fn a_newer_release_without_srectl_is_still_an_error() {
    let fetch = |_: &str| -> Result<Vec<u8>, UpdateError> {
        Ok(br#"{"tag_name":"srelens-v2.0.0","prerelease":false,"assets":[{"name":"srelens-tui-2.0.0-SHA256SUMS.txt"}]}"#.to_vec())
    };
    let err = plan(
        "1.2.0",
        Channel::Stable,
        false,
        PathBuf::from("/tmp/srelens-tui"),
        &fetch,
    )
    .unwrap_err();
    let UpdateError::BadRelease(message) = err else {
        panic!("expected the missing build to be reported, got {err:?}");
    };
    assert!(message.contains("carries no srectl build"), "{message}");
}

#[test]
fn a_newer_dev_release_without_srectl_is_still_an_error() {
    let fetch = |_: &str| -> Result<Vec<u8>, UpdateError> {
        let body = format!(
            r#"[
            {{"tag_name":"srelens-v2.0.0-dev.1","prerelease":true,"assets":[{{"name":"srelens-tui-2.0.0-dev.1-SHA256SUMS.txt"}}]}}
        ]"#
        );
        Ok(body.into_bytes())
    };
    let err = plan(
        "1.2.0-dev.1",
        Channel::Dev,
        false,
        PathBuf::from("/tmp/srelens-tui"),
        &fetch,
    )
    .unwrap_err();
    let UpdateError::BadRelease(message) = err else {
        panic!("expected the missing build to be reported, got {err:?}");
    };
    assert!(
        message.contains("release srelens-v2.0.0-dev.1 carries no srectl build"),
        "{message}"
    );
}

#[test]
fn a_newer_dev_release_without_signature_is_still_an_error() {
    let fetch = |_: &str| -> Result<Vec<u8>, UpdateError> {
        let body = format!(
            r#"[
            {{"tag_name":"srelens-v2.0.0-dev.1","prerelease":true,"assets":{}}}
        ]"#,
            unsigned_assets_for("2.0.0-dev.1")
        );
        Ok(body.into_bytes())
    };
    let err = plan(
        "1.2.0-dev.1",
        Channel::Dev,
        false,
        PathBuf::from("/tmp/srelens-tui"),
        &fetch,
    )
    .unwrap_err();
    let UpdateError::BadRelease(message) = err else {
        panic!("expected the missing signature to be reported, got {err:?}");
    };
    assert!(
        message.contains("release srelens-v2.0.0-dev.1 is not signed"),
        "{message}"
    );
}

#[test]
fn a_dev_release_that_still_publishes_srelens_tui_at_same_version_is_current() {
    let fetch = |_: &str| -> Result<Vec<u8>, UpdateError> {
        let body = format!(
            r#"[
            {{"tag_name":"srelens-v1.2.0-dev.1","prerelease":true,"assets":[{{"name":"srelens-tui-1.2.0-dev.1-SHA256SUMS.txt"}}]}}
        ]"#
        );
        Ok(body.into_bytes())
    };
    assert_eq!(
        plan(
            "1.2.0-dev.1",
            Channel::Dev,
            false,
            PathBuf::from("/tmp/srelens-tui"),
            &fetch
        )
        .unwrap(),
        Check::UpToDate {
            channel: Channel::Dev,
            latest: "1.2.0-dev.1".into()
        }
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
            false,
            PathBuf::from("/tmp/srelens-tui"),
            &fetch
        ),
        Err(UpdateError::Download(_))
    ));
}

// ---------------------------------------------------------------------------
// Applying — the part no release can exercise yet
// ---------------------------------------------------------------------------

/// A signing key made for one test, standing in for the release key, which
/// no test can sign with.
struct TestKey(pgp::composed::SignedSecretKey);

impl TestKey {
    fn new() -> Self {
        let mut params = pgp::composed::SecretKeyParamsBuilder::default();
        params
            .key_type(pgp::composed::KeyType::Ed25519Legacy)
            .can_sign(true)
            .can_certify(true)
            .primary_user_id("srelens update test key <test@test.invalid>".into());
        let key = params
            .build()
            .expect("key parameters")
            .generate(rand::thread_rng())
            .expect("a test key");
        Self(key)
    }

    /// The public half, armored the way `KEYS` holds it.
    fn public(&self) -> String {
        self.0
            .to_public_key()
            .to_armored_string(Default::default())
            .expect("armor the public key")
    }

    fn fingerprint(&self) -> String {
        use pgp::types::KeyDetails;
        format!("{:X}", self.0.fingerprint())
    }

    /// A detached binary signature over `data`, armored, as `sign-artifacts`
    /// writes it with `gpg --armor --detach-sign`.
    fn sign(&self, data: &[u8]) -> Vec<u8> {
        pgp::composed::DetachedSignature::sign_binary_data(
            rand::thread_rng(),
            &self.0.primary_key,
            &pgp::types::Password::empty(),
            pgp::crypto::hash::HashAlgorithm::Sha256,
            data,
        )
        .expect("sign")
        .to_armored_bytes(Default::default())
        .expect("armor the signature")
    }
}

/// A release an update can be applied from, served without the network: the
/// archive, its checksum file, and that file's signature by `key`.
struct Release {
    plan: Plan,
    archive: Vec<u8>,
    sums: Vec<u8>,
    signature: Vec<u8>,
    key: TestKey,
    /// Every URL the update asked for, in order.
    asked: std::cell::RefCell<Vec<String>>,
}

impl Release {
    /// The fetch an update makes, answered from this release.
    fn fetch(&self, url: &str) -> Result<Vec<u8>, UpdateError> {
        self.asked.borrow_mut().push(url.to_string());
        if url == self.plan.sums_url {
            Ok(self.sums.clone())
        } else if url == self.plan.sums_signature_url {
            Ok(self.signature.clone())
        } else if url == self.plan.archive_url {
            Ok(self.archive.clone())
        } else {
            Err(UpdateError::Download(format!("404 Not Found for {url}")))
        }
    }

    fn asked_for(&self, url: &str) -> bool {
        self.asked.borrow().iter().any(|asked| asked == url)
    }

    /// The keys a build trusts when the key that signed this release is the
    /// release key.
    fn trusted_keys(&self) -> String {
        self.key.public()
    }

    /// Replace the checksum file, signed by the same key, as a release that
    /// really published it would be.
    fn with_signed_sums(mut self, sums: Vec<u8>) -> Self {
        self.signature = self.key.sign(&sums);
        self.sums = sums;
        self
    }
}

/// Build a release whose plan targets a real file in `dir`, carrying an
/// archive of `body`, a checksum file that matches it, and that file's
/// signature.
fn staged(dir: &Path, body: &'static [u8]) -> Release {
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
    let sums = format!("{}  {}\n", sha256_hex(&archive), asset).into_bytes();
    let key = TestKey::new();
    let plan = Plan {
        current: "1.0.0".into(),
        latest: "2.0.0".into(),
        archive_url: asset_url("2.0.0", &asset),
        sums_url: asset_url("2.0.0", &sums_name("2.0.0")),
        sums_signature_url: asset_url("2.0.0", &sums_signature_name("2.0.0")),
        asset,
        target,
    };
    Release {
        plan,
        archive,
        signature: key.sign(&sums),
        sums,
        key,
        asked: Default::default(),
    }
}

#[test]
fn a_verified_download_replaces_the_installed_binary() {
    let dir = tempfile::tempdir().unwrap();
    let release = staged(dir.path(), b"the new binary");

    let signer = apply_with_keys(
        &release.plan,
        &|url: &str| release.fetch(url),
        &release.trusted_keys(),
    )
    .expect("the update applies");

    assert_eq!(
        signer,
        release.key.fingerprint(),
        "it says which key vouched for the release"
    );
    let plan = &release.plan;
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
    let release = staged(dir.path(), b"unused");
    // A signed checksum file naming a hash the archive does not have: what a
    // truncated or corrupted download looks like.
    let sums = format!("{}  {}\n", "a".repeat(64), release.plan.asset).into_bytes();
    let release = release.with_signed_sums(sums);

    match apply_with_keys(
        &release.plan,
        &|url: &str| release.fetch(url),
        &release.trusted_keys(),
    ) {
        Err(UpdateError::ChecksumMismatch { .. }) => {}
        other => panic!("expected a checksum mismatch, got {other:?}"),
    }
    assert_eq!(
        std::fs::read(&release.plan.target).unwrap(),
        b"the old binary",
        "the installed binary must be untouched"
    );
}

/// The case the checksum alone could not catch (#448): someone able to
/// replace release assets replaces the archive AND the checksum file, so the
/// two agree. Only the signature over the checksum file tells them apart.
#[test]
fn a_checksum_file_changed_after_it_was_signed_is_refused_before_the_archive_is_fetched() {
    let dir = tempfile::tempdir().unwrap();
    let mut release = staged(dir.path(), b"the genuine binary");
    let substitute = archive_for(&release.plan.asset, b"someone else's binary");
    release.sums = format!("{}  {}\n", sha256_hex(&substitute), release.plan.asset).into_bytes();
    release.archive = substitute;

    match apply_with_keys(
        &release.plan,
        &|url: &str| release.fetch(url),
        &release.trusted_keys(),
    ) {
        Err(UpdateError::Unverified { why, .. }) => assert_eq!(why, SignatureProblem::Mismatch),
        other => panic!("expected the checksums to fail their signature, got {other:?}"),
    }
    assert!(
        !release.asked_for(&release.plan.archive_url),
        "nothing past the checksums is downloaded once they fail"
    );
    assert_eq!(
        std::fs::read(&release.plan.target).unwrap(),
        b"the old binary"
    );
}

/// A signature is only as good as the key behind it: one this build does not
/// trust is no better than none.
#[test]
fn checksums_signed_by_a_key_this_build_does_not_trust_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let release = staged(dir.path(), b"unused");
    let someone_else = TestKey::new();

    match apply_with_keys(
        &release.plan,
        &|url: &str| release.fetch(url),
        &someone_else.public(),
    ) {
        Err(UpdateError::Unverified { why, .. }) => {
            assert_eq!(why, SignatureProblem::UnknownSigner)
        }
        other => panic!("expected an untrusted signer, got {other:?}"),
    }
    assert!(!release.asked_for(&release.plan.archive_url));
    assert_eq!(
        std::fs::read(&release.plan.target).unwrap(),
        b"the old binary"
    );
}

/// `apply` is what the command runs, so it must trust the keys compiled in
/// from `KEYS` and nothing else, not whatever key a test or a release offers.
#[test]
fn a_plain_apply_trusts_only_the_keys_compiled_in() {
    let dir = tempfile::tempdir().unwrap();
    let release = staged(dir.path(), b"unused");

    match apply(&release.plan, &|url: &str| release.fetch(url)) {
        Err(UpdateError::Unverified { why, .. }) => {
            assert_eq!(why, SignatureProblem::UnknownSigner)
        }
        other => panic!("expected the test key to be untrusted, got {other:?}"),
    }
    assert_eq!(
        std::fs::read(&release.plan.target).unwrap(),
        b"the old binary"
    );
}

#[test]
fn a_binary_a_package_manager_owns_is_refused_before_anything_is_downloaded() {
    let dir = tempfile::tempdir().unwrap();
    let mut release = staged(dir.path(), b"unused");
    release.plan.target = PathBuf::from("/opt/homebrew/bin").join(bin_name());
    let fetch = |_: &str| -> Result<Vec<u8>, UpdateError> {
        panic!("nothing should be downloaded for a package-managed binary")
    };

    match apply(&release.plan, &fetch) {
        Err(UpdateError::PackageManaged { manager, .. }) => assert_eq!(manager, "Homebrew"),
        other => panic!("expected PackageManaged, got {other:?}"),
    }
}

/// Add one entry to `dir`'s ACL with `icacls`, so the check reads a real
/// Windows ACL rather than a model of one. Principals are given by SID
/// (`*S-1-1-0` is Everyone), which no display language can rename.
#[cfg(windows)]
fn grant(dir: &Path, entry: &str) {
    let out = std::process::Command::new("icacls")
        .arg(dir)
        .args(["/grant", entry])
        .output()
        .expect("icacls runs");
    assert!(
        out.status.success(),
        "icacls /grant {entry}: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// The Windows half of the unsafe-directory refusal (#450). A directory whose
/// ACL lets everyone with an account put a file at a name, or take the
/// directory over, is refused before anything is downloaded, as a
/// world-writable one is on Unix. Adding a file is enough on its own: the
/// update renames the running binary aside before renaming the new one in,
/// and between the two the name is free for anyone who can create it.
#[cfg(windows)]
#[test]
fn a_windows_directory_anyone_can_write_to_is_refused_before_anything_is_downloaded() {
    for entry in [
        // Everyone: Modify.
        "*S-1-1-0:(M)",
        // Authenticated Users: create files, and nothing else.
        "*S-1-5-11:(WD)",
        // BUILTIN\Users: rewrite the ACL, and so grant itself the rest.
        "*S-1-5-32-545:(WDAC)",
        // `C:\`'s own: Modify for Authenticated Users on whatever is created
        // inside. The staged binary inherits it, so anyone could rewrite it
        // between its read-back and the rename, and the installed one after.
        "*S-1-5-11:(OI)(CI)(IO)(M)",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let release = staged(dir.path(), b"unused");
        grant(dir.path(), entry);
        let fetch = |_: &str| -> Result<Vec<u8>, UpdateError> {
            panic!("{entry}: nothing should be downloaded into a directory anyone can write to")
        };

        match apply(&release.plan, &fetch) {
            Err(UpdateError::UnsafeDirectory { path }) => assert_eq!(path, dir.path(), "{entry}"),
            other => panic!("{entry}: expected UnsafeDirectory, got {other:?}"),
        }
    }
}

/// The ordinary places, and the entries Windows puts on them, are not
/// refused: a refusal there would stop people updating without protecting
/// anyone.
#[cfg(windows)]
#[test]
fn the_ordinary_windows_install_directories_are_not_refused() {
    for entry in [
        // As fresh from %TEMP%: the user, SYSTEM and Administrators.
        None,
        // `C:\`'s own entry: Authenticated Users may create FOLDERS there.
        // A folder planted at the binary's name breaks the update but
        // cannot be run.
        Some("*S-1-5-11:(AD)"),
        // Modify for Authenticated Users on the FOLDERS created inside later,
        // (CI)(IO): it reaches neither this directory nor the files the
        // update creates in it.
        Some("*S-1-5-11:(CI)(IO)(M)"),
        // A named group someone chose to trust, the counterpart of a
        // group-writable directory on Unix: Backup Operators.
        Some("*S-1-5-32-551:(M)"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let release = staged(dir.path(), b"unused");
        if let Some(entry) = entry {
            grant(dir.path(), entry);
        }
        // Reaching the download is the proof the directory was accepted.
        let fetch = |_: &str| -> Result<Vec<u8>, UpdateError> {
            Err(UpdateError::Download(
                "stopped at the first download".into(),
            ))
        };

        match apply(&release.plan, &fetch) {
            Err(UpdateError::Download(_)) => {}
            other => panic!("{entry:?}: expected to reach the download, got {other:?}"),
        }
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

/// What `update` guarantees has to be stated where someone running it sees
/// it, not only in the install guide (#448): `update --help`.
#[test]
fn update_help_says_what_is_verified_before_installing() {
    use clap::CommandFactory;
    let mut cli = srelens_tui::Cli::command();
    let help = cli
        .find_subcommand_mut("update")
        .expect("an update subcommand")
        .render_long_help()
        .to_string();
    assert!(help.contains("signed"), "{help}");
    assert!(help.contains("KEYS"), "{help}");
}
