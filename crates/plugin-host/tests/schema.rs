//! The committed manifest JSON Schema is what editors and CI validate manifests against.
use serde_json::{json, Value};
use srelens_plugin_host::{Manifest, API_FIELDS, SUPPORTED_API_VERSIONS};
use std::collections::BTreeSet;

/// The published URL manifests name in `$schema`.
const SCHEMA_URL: &str =
    "https://raw.githubusercontent.com/srelens/srelens/main/schemas/extension-manifest.v0.4.json";

/// One file per API line: `v0.4` for 0.4.0.
fn schema_path_for(version: &str) -> String {
    let version = semver::Version::parse(version).unwrap();
    format!(
        "{}/../../schemas/extension-manifest.v{}.{}.json",
        env!("CARGO_MANIFEST_DIR"),
        version.major,
        version.minor
    )
}

/// The newest supported line's file, which the contract generates.
fn schema_path() -> String {
    schema_path_for(SUPPORTED_API_VERSIONS.last().unwrap())
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

/// Every example manifest, so a new one cannot skip these checks.
fn examples() -> Vec<(String, String)> {
    let dir = format!("{}/../../examples/extensions", env!("CARGO_MANIFEST_DIR"));
    let mut examples: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .map(|path| {
            let source = std::fs::read_to_string(&path).unwrap();
            (path.display().to_string(), source)
        })
        .collect();
    examples.sort();
    assert!(!examples.is_empty(), "no example manifests in {dir}");
    examples
}

#[test]
fn schema_has_no_rust_only_formats() {
    let schema = Manifest::schema().to_string();
    for rust_format in [
        "\"format\":\"uint",
        "\"format\":\"int",
        "\"format\":\"float",
        "\"format\":\"double",
    ] {
        assert!(
            !schema.contains(rust_format),
            "schema contains {rust_format}"
        );
    }
}

#[test]
fn manifests_may_name_their_schema_for_editors() {
    assert!(Manifest::schema()["properties"].get("$schema").is_some());
    for (path, source) in examples() {
        let value: Value = serde_json::from_str(&source).unwrap();
        assert_eq!(value["$schema"], json!(SCHEMA_URL), "{path}");
        Manifest::parse(&source).unwrap_or_else(|error| panic!("{path}: {error}"));
        // `$schema` is editor metadata: removing it leaves the same valid contract.
        let mut without = value.clone();
        without.as_object_mut().unwrap().remove("$schema");
        Manifest::parse(&without.to_string()).unwrap();
    }
}

/// Every field path `schema` declares, through `$ref`, `items` and `anyOf`/`oneOf`/`allOf`,
/// spelled as `API_FIELDS` spells one: `.` between fields and `[]` into an array.
fn field_paths(schema: &Value) -> BTreeSet<String> {
    fn walk(root: &Value, node: &Value, at: &str, depth: usize, paths: &mut BTreeSet<String>) {
        assert!(depth < 64, "{at}: the schema recurses");
        if let Some(reference) = node.get("$ref").and_then(Value::as_str) {
            let name = reference.rsplit('/').next().unwrap();
            let definitions = root.get("definitions").or_else(|| root.get("$defs"));
            let target = &definitions.expect("definitions")[name];
            return walk(root, target, at, depth + 1, paths);
        }
        for combinator in ["anyOf", "oneOf", "allOf"] {
            for branch in node
                .get(combinator)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                walk(root, branch, at, depth + 1, paths);
            }
        }
        if let Some(items) = node.get("items") {
            walk(root, items, &format!("{at}[]"), depth + 1, paths);
        }
        let properties = node.get("properties").and_then(Value::as_object);
        for (name, property) in properties.into_iter().flatten() {
            let path = if at.is_empty() {
                name.clone()
            } else {
                format!("{at}.{name}")
            };
            paths.insert(path.clone());
            walk(root, property, &path, depth + 1, paths);
        }
    }
    let mut paths = BTreeSet::new();
    walk(schema, schema, "", 0, &mut paths);
    paths
}

/// Whether `path` is, or is inside, an `API_FIELDS` field entry for which `accepts` holds.
fn gated(path: &str, accepts: impl Fn(&srelens_plugin_host::ApiField) -> bool) -> bool {
    API_FIELDS.iter().any(|field| {
        field.form.is_none()
            && (path == field.path
                || path.starts_with(&format!("{}.", field.path))
                || path.starts_with(&format!("{}[]", field.path)))
            && accepts(field)
    })
}

/// An older supported line's schema file stays as it was when the next line was cut. A
/// field the contract has and that file lacks arrived later, so `API_FIELDS` must say
/// so, or a host on the older line meets it as an unknown field (#709). A field the file
/// has and the contract lacks was removed, and must say that.
#[test]
fn a_field_missing_from_an_older_lines_schema_is_gated_in_api_fields() {
    let current = field_paths(&Manifest::schema());
    let (newest, older) = SUPPORTED_API_VERSIONS.split_last().unwrap();
    assert!(!older.is_empty() || API_FIELDS.is_empty(), "{newest}");
    for line in older {
        let version = semver::Version::parse(line).unwrap();
        let path = schema_path_for(line);
        let frozen = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("{path} is missing"));
        let frozen = field_paths(&serde_json::from_str(&frozen).unwrap());
        assert!(
            frozen.contains("contributions.pages[].capability"),
            "{path}"
        );
        for added in current.difference(&frozen) {
            assert!(
                gated(added, |field| semver::Version::parse(field.introduced).unwrap() > version),
                "`{added}` is not in API {line}'s schema: list it in API_FIELDS with a later `introduced`"
            );
        }
        for removed in frozen.difference(&current) {
            assert!(
                gated(removed, |field| field.removed.is_some()),
                "`{removed}` is in API {line}'s schema but not the contract: list its removal in API_FIELDS"
            );
        }
    }
}
