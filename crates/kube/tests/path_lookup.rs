//! `helm_binary` against a real file on a real `PATH`.
//!
//! A test binary of its own, with one test in it, because it repoints `PATH`
//! for the whole process: tests in one binary run on parallel threads, and a
//! sibling resolving anything mid-run would see the scratch directory.

#[test]
fn helm_binary_finds_helm_under_the_name_the_platform_gives_executables() {
    let dir = tempfile::tempdir().unwrap();
    // `helm.exe` on Windows, `helm` everywhere else.
    let helm = dir
        .path()
        .join(format!("helm{}", std::env::consts::EXE_SUFFIX));
    std::fs::write(&helm, b"").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&helm, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::env::set_var("PATH", dir.path());

    assert_eq!(srelens_kube::helm_cli::helm_binary(), Ok(helm));
}
