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
        assert_eq!(wrapper, wrapper_script(&next));
        assert!(wrapper.contains("srelens-tui is now srectl"));
    }

    std::fs::write(&next, b"newer-release").unwrap();
    apply_rebrand(&legacy).unwrap();
    assert_eq!(
        std::fs::read(&next).unwrap(),
        b"newer-release",
        "an existing srectl is a later release and must not be overwritten"
    );
}
