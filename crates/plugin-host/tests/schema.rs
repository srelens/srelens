//! The committed manifest JSON Schema is what editors and CI validate manifests against.
use serde_json::{json, Value};
use srelens_plugin_host::{Manifest, SUPPORTED_API_VERSIONS};

/// The published URL manifests name in `$schema`.
const SCHEMA_URL: &str =
    "https://raw.githubusercontent.com/srelens/srelens/main/schemas/extension-manifest.v0.1.json";

/// One file per API line, named after the newest supported version: `v0.1` for 0.1.0.
fn schema_path() -> String {
    let newest = semver::Version::parse(SUPPORTED_API_VERSIONS.last().unwrap()).unwrap();
    format!(
        "{}/../../schemas/extension-manifest.v{}.{}.json",
        env!("CARGO_MANIFEST_DIR"),
        newest.major,
        newest.minor
    )
}

/// The committed schema MUST equal the Rust contract. Regenerate it with
/// `UPDATE_CATALOG=1 cargo test -p srelens-plugin-host`, the same knob the registry uses
/// for its committed catalogs.
#[test]
fn committed_manifest_schema_matches_the_contract() {
    let path = schema_path();
    let generated = Manifest::schema();
    if std::env::var("UPDATE_CATALOG").is_ok() {
        std::fs::create_dir_all(std::path::Path::new(&path).parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&generated).unwrap() + "\n",
        )
        .unwrap();
        return;
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!("{path} is missing: run UPDATE_CATALOG=1 cargo test -p srelens-plugin-host")
    });
    let committed: Value = serde_json::from_str(&committed).unwrap();
    // Compare parsed values, never text: map key order depends on which serde_json
    // features the rest of the workspace enables (see AGENTS.md).
    assert!(
        committed == generated,
        "{path} is stale: run UPDATE_CATALOG=1 cargo test -p srelens-plugin-host"
    );
}

#[test]
fn manifests_may_name_their_schema_for_editors() {
    assert!(Manifest::schema()["properties"].get("$schema").is_some());
    for source in [
        include_str!("../../../examples/extensions/argocd.json"),
        include_str!("../../../examples/extensions/flux.json"),
    ] {
        let value: Value = serde_json::from_str(source).unwrap();
        assert_eq!(value["$schema"], json!(SCHEMA_URL));
        Manifest::parse(source).unwrap();
        // `$schema` is editor metadata: removing it leaves the same valid contract.
        let mut without = value.clone();
        without.as_object_mut().unwrap().remove("$schema");
        Manifest::parse(&without.to_string()).unwrap();
    }
}
