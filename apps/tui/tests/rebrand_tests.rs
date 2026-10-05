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
    }
}
