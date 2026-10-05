//! The last `srelens-tui` build installs `srectl` beside itself.

use std::path::Path;

use srelens_tui::rebrand::{
    apply_rebrand, legacy_file_name, next_command_path, next_file_name, should_rebrand,
    wrapper_script,
};

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
fn applying_the_rebrand_installs_srectl_and_keeps_a_newer_copy() {
    let dir = tempfile::tempdir().unwrap();
    let legacy = dir.path().join(legacy_file_name());
    std::fs::write(&legacy, b"bridge-bytes").unwrap();

    let next = apply_rebrand(&legacy).unwrap();
    assert_eq!(next, next_command_path(&legacy));
    assert_eq!(std::fs::read(&next).unwrap(), b"bridge-bytes");

    #[cfg(unix)]
    {
        let wrapper = std::fs::read_to_string(&legacy).unwrap();
        assert_eq!(wrapper, wrapper_script(&next).unwrap());
        assert!(wrapper.contains("[ -t 2 ] && echo 'srelens-tui is now srectl' >&2"));
        assert!(wrapper.contains("exec '"));
    }

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

    apply_rebrand(&legacy).unwrap();
    assert_eq!(
        std::fs::read(&next).unwrap(),
        b"newer-release",
        "an existing srectl is a later release and must not be overwritten"
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
    let err = apply_rebrand(&legacy).unwrap_err();
    assert!(err.contains("not a regular file"), "{err}");

    std::fs::remove_dir(&next).unwrap();

    // On Unix: existing file is not executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(&next, b"not-executable").unwrap();
        std::fs::set_permissions(&next, std::fs::Permissions::from_mode(0o644)).unwrap();
        let err = apply_rebrand(&legacy).unwrap_err();
        assert!(err.contains("not executable"), "{err}");

        // If other-execute is set (0o641) but owner execute is missing, it is still refused:
        let my_uid = unsafe { libc::getuid() };
        if my_uid != 0 {
            std::fs::set_permissions(&next, std::fs::Permissions::from_mode(0o641)).unwrap();
            let err = apply_rebrand(&legacy).unwrap_err();
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
            let err = apply_rebrand(&legacy).unwrap_err();
            assert!(err.contains("owned by a different user"), "{err}");
        }
    }

    // 2. Integration check exercising UID rejection on a system binary with different UID:
    for candidate in &["/usr/bin/true", "/bin/sh", "/usr/bin/whoami"] {
        let candidate_path = Path::new(candidate);
        if let Ok(meta) = std::fs::metadata(candidate_path) {
            if meta.uid() != my_uid {
                let err =
                    srelens_tui::rebrand::is_trusted_existing_command(&legacy, candidate_path)
                        .unwrap_err();
                assert!(err.contains("owned by a different user"), "{err}");
                break;
            }
        }
    }
}
