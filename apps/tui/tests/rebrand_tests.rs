//! The last `srelens-tui` build installs `srectl` beside itself.

use std::path::Path;

use srectl::rebrand::{
    apply_rebrand, legacy_file_name, next_command_path, next_file_name, replaces_existing,
    should_rebrand, wrapper_script,
};

const THIS_VERSION: &str = env!("CARGO_PKG_VERSION");

#[test]
fn only_an_srectl_reporting_an_older_version_is_replaced() {
    // `srectl --version` prints `srectl <version>`; the `version` subcommand
    // prints `srectl v<version>`.
    assert!(replaces_existing(Some("srectl 0.15.0-dev.3\n"), "0.15.0"));
    assert!(replaces_existing(Some("srectl v0.14.2\n"), "0.15.0"));
    assert!(!replaces_existing(Some("srectl 0.15.0\n"), "0.15.0"));
    assert!(!replaces_existing(Some("srectl 0.16.0-dev.1\n"), "0.15.0"));
    // A version that cannot be read keeps the existing command.
    assert!(!replaces_existing(None, "0.15.0"));
    assert!(!replaces_existing(Some(""), "0.15.0"));
    assert!(!replaces_existing(Some("command not found\n"), "0.15.0"));
}

#[test]
fn an_older_srectl_beside_the_legacy_command_is_replaced_and_a_current_one_kept() {
    // The real srectl, which reports THIS_VERSION, stands in for one a
    // previous install left beside the legacy command.
    let built = Path::new(env!("CARGO_BIN_EXE_srectl"));
    let built_len = std::fs::metadata(built).unwrap().len();
    let dir = tempfile::tempdir().unwrap();
    let legacy = dir.path().join(legacy_file_name());
    let next = next_command_path(&legacy);
    std::fs::copy(built, &next).unwrap();

    for this_version in [THIS_VERSION, "0.0.1"] {
        std::fs::write(&legacy, b"bridge-bytes").unwrap();
        apply_rebrand(&legacy, this_version).unwrap();
        assert_eq!(
            std::fs::metadata(&next).unwrap().len(),
            built_len,
            "an srectl as new as {this_version} or newer must be kept"
        );
    }

    std::fs::write(&legacy, b"bridge-bytes").unwrap();
    apply_rebrand(&legacy, "99.0.0").unwrap();
    // `assert!`, not `assert_eq!`: a failure would print the whole binary.
    assert!(
        std::fs::read(&next).unwrap() == b"bridge-bytes",
        "an srectl older than this build must be replaced by it"
    );
}

#[test]
fn a_manual_install_rebrands_and_a_managed_or_cargo_one_does_not() {
    assert!(should_rebrand(
        &Path::new("/home/me/.local/bin").join(legacy_file_name())
    ));
    assert!(!should_rebrand(
        &Path::new("/home/me/.local/bin").join(next_file_name())
    ));
    assert!(!should_rebrand(
        &Path::new("/opt/homebrew/bin").join(legacy_file_name())
    ));
    assert!(!should_rebrand(
        &Path::new("/work/srelens/target/debug").join(legacy_file_name())
    ));
    assert!(!should_rebrand(
        &Path::new("/work/srelens/target/release").join(legacy_file_name())
    ));
    assert!(!should_rebrand(
        &Path::new("/work/srelens/target/x86_64-unknown-linux-gnu/debug").join(legacy_file_name())
    ));
    assert!(!should_rebrand(
        &Path::new("/work/srelens/target/x86_64-unknown-linux-gnu/release")
            .join(legacy_file_name())
    ));
    assert!(!should_rebrand(
        &Path::new("/work/srelens/target/llvm-cov-target/debug").join(legacy_file_name())
    ));
}

#[test]
fn applying_the_rebrand_installs_srectl_and_keeps_one_it_cannot_read() {
    let dir = tempfile::tempdir().unwrap();
    let legacy = dir.path().join(legacy_file_name());
    std::fs::write(&legacy, b"bridge-bytes").unwrap();

    let next = apply_rebrand(&legacy, THIS_VERSION).unwrap();
    assert_eq!(next, next_command_path(&legacy));
    assert_eq!(std::fs::read(&next).unwrap(), b"bridge-bytes");

    #[cfg(unix)]
    {
        let wrapper = std::fs::read_to_string(&legacy).unwrap();
        assert_eq!(wrapper, wrapper_script(&next).unwrap());
        assert!(wrapper.contains("[ -t 2 ] && echo 'srelens-tui is now srectl' >&2"));
        assert!(wrapper.contains("exec '"));
    }

    // An srectl whose version cannot be read: it may be a later release.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(&next, b"newer-release").unwrap();
        std::fs::set_permissions(&next, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    #[cfg(not(unix))]
    {
        std::fs::write(&next, b"newer-release").unwrap();
    }

    apply_rebrand(&legacy, THIS_VERSION).unwrap();
    assert_eq!(
        std::fs::read(&next).unwrap(),
        b"newer-release",
        "an existing srectl that may be a later release must not be overwritten"
    );
}

#[test]
fn an_untrusted_or_invalid_existing_srectl_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let legacy = dir.path().join(legacy_file_name());
    std::fs::write(&legacy, b"bridge-bytes").unwrap();
    let next = next_command_path(&legacy);

    // Existing path is a directory
    std::fs::create_dir(&next).unwrap();
    let err = apply_rebrand(&legacy, THIS_VERSION).unwrap_err();
    assert!(err.contains("not a regular file"), "{err}");

    std::fs::remove_dir(&next).unwrap();

    // On Unix: existing file is not executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(&next, b"not-executable").unwrap();
        std::fs::set_permissions(&next, std::fs::Permissions::from_mode(0o644)).unwrap();
        let err = apply_rebrand(&legacy, THIS_VERSION).unwrap_err();
        assert!(err.contains("not executable"), "{err}");

        // If other-execute is set (0o641) but owner execute is missing, it is still refused:
        let my_uid = unsafe { libc::getuid() };
        if my_uid != 0 {
            std::fs::set_permissions(&next, std::fs::Permissions::from_mode(0o641)).unwrap();
            let err = apply_rebrand(&legacy, THIS_VERSION).unwrap_err();
            assert!(err.contains("not executable"), "{err}");
        }
    }
}

#[cfg(unix)]
#[test]
fn an_existing_srectl_with_different_uid_is_refused() {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let legacy = dir.path().join(legacy_file_name());
    std::fs::write(&legacy, b"bridge-bytes").unwrap();
    let next = next_command_path(&legacy);
    std::fs::write(&next, b"other-user-srectl").unwrap();
    std::fs::set_permissions(&next, std::fs::Permissions::from_mode(0o755)).unwrap();

    let my_uid = unsafe { libc::getuid() };
    let different_uid = if my_uid == 0 { 1000 } else { 0 };

    // 1. If running as root / CAP_CHOWN (e.g. in containerized CI), test apply_rebrand directly:
    if let Ok(c_path) = CString::new(next.as_os_str().as_bytes()) {
        if unsafe { libc::chown(c_path.as_ptr(), different_uid, libc::gid_t::MAX) } == 0 {
            let err = apply_rebrand(&legacy, THIS_VERSION).unwrap_err();
            assert!(err.contains("owned by a different user"), "{err}");
        }
    }

    // 2. Integration check exercising UID rejection on a system binary with different UID:
    for candidate in &["/usr/bin/true", "/bin/sh", "/usr/bin/whoami"] {
        let candidate_path = Path::new(candidate);
        if let Ok(meta) = std::fs::metadata(candidate_path) {
            if meta.uid() != my_uid {
                let err = srectl::rebrand::is_trusted_existing_command(&legacy, candidate_path)
                    .unwrap_err();
                assert!(err.contains("owned by a different user"), "{err}");
                break;
            }
        }
    }
}
