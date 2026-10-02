//! What a sidecar author's build pulls in. The protocol crate's `schema`
//! feature is off by default, so an author who never generates the committed
//! JSON Schema does not build schemars and the crates it brings. The SDK's own
//! tests turn the feature on, but only as a dev-dependency, and
//! `cargo tree -e no-dev` leaves those out: it is the tree an author builds.

use std::process::Command;

/// The name of every crate `cargo tree` lists under `package`, development
/// dependencies left out. `--locked --offline`: the lockfile and the registry
/// cache this build already used, with no network.
fn crates_in_the_tree_of(package: &str) -> Vec<String> {
    let output = Command::new(env!("CARGO"))
        .args(["tree", "-p", package])
        .args(["-e", "no-dev", "--prefix", "none", "--locked", "--offline"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("cargo runs");
    assert!(
        output.status.success(),
        "cargo tree -p {package} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("cargo tree writes UTF-8")
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .map(str::to_owned)
        .collect()
}

#[test]
fn a_sidecar_built_with_default_features_does_not_build_schemars() {
    // The SDK, its protocol crate, and the example: a real sidecar's own
    // dependencies, which is the build the saving is for.
    for package in [
        "srelens-sidecar",
        "srelens-sidecar-protocol",
        "srelens-sidecar-hello-world",
    ] {
        let tree = crates_in_the_tree_of(package);
        assert!(
            tree.iter().any(|name| name == package),
            "cargo tree -p {package} did not list {package}: {tree:?}"
        );
        assert!(
            !tree.iter().any(|name| name == "schemars"),
            "{package} builds schemars without the protocol crate's `schema` feature; \
             a sidecar author who never generates the schema should not pay for it"
        );
    }
}
