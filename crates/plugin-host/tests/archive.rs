use srelens_plugin_host::freelens::FluxArchive;
const PACKAGE: &[u8] = include_bytes!("fixtures/freelens-flux-5.3.1.tgz");
#[test]
fn loads_the_upstream_tarball_without_executing_install_scripts() {
    let package = FluxArchive::parse(PACKAGE).unwrap();
    assert_eq!(package.name, "@freelensapp/fluxcd-extension");
    assert_eq!(package.version, "5.3.1");
    assert!(package.renderer.contains("global.LensExtensions"));
    assert!(package.renderer.contains("kubeObjectDetailItems"));
}
#[test]
fn refuses_unverified_or_oversized_packages() {
    let mut changed = PACKAGE.to_vec();
    changed[100] ^= 1;
    assert!(FluxArchive::parse(&changed).is_err());
    assert!(FluxArchive::parse(b"not a tarball").is_err());
    assert!(FluxArchive::parse(&vec![0; 1024 * 1024 + 1]).is_err());
}
