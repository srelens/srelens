//! Executable apps (#574): the `executable` kind, the sidecar it declares, and the
//! host's own checks on every operation call.
use serde_json::{json, Value};
use srelens_plugin_host::{
    host_platform, Manifest, ManifestKind, ValidationCode, MAX_OPERATION_CALL_BYTES,
    SIDECAR_OPERATION, SIDECAR_PLATFORMS,
};

/// A scanner that does all its work in its sidecar: no bindings, one operation.
fn manifest() -> Value {
    json!({
        "id":"org.example.scanner", "name":"Scanner", "version":"0.1.0", "srelensApiVersion":"^0.6",
        "kind":"executable", "permissions":[], "capabilities":[],
        "sidecar":{
            "binaries":{"linux-amd64":"bin/linux-amd64/scanner","windows-amd64":"bin/windows-amd64/scanner.exe"},
            "operations":[{"name":"scan","title":"Scan an image","inputs":[
                {"name":"image","title":"Image reference","type":"string","required":true,"maxLength":512},
                {"name":"severity","type":"string"},
                {"name":"limit","type":"integer"},
                {"name":"ratio","type":"number"},
                {"name":"fixable","type":"boolean"}]}]},
        "contributions":{"pages":[],"detailTabs":[],"detailLinks":[]}
    })
}

#[test]
fn native_operation_views_require_api_0_8_and_streams_never_auto_run() {
    let mut source = manifest();
    source["srelensApiVersion"] = json!("^0.8");
    source["sidecar"]["operations"][0]["view"] = json!({"autoRun":true});
    let parsed = Manifest::parse(&source.to_string()).expect("native operation view");
    assert_eq!(serde_json::to_value(parsed).unwrap()["sidecar"]["operations"][0]["view"]["autoRun"], true);
    source["srelensApiVersion"] = json!("^0.7");
    assert!(Manifest::parse(&source.to_string()).is_err());
    source["srelensApiVersion"] = json!("^0.8");
    source["sidecar"]["operations"][0]["view"]["stream"] = json!(true);
    assert!(Manifest::parse(&source.to_string()).is_err(), "stream scans require an explicit Run");
    source["sidecar"]["operations"][0]["view"] = json!({"stream":true});
    assert!(Manifest::parse(&source.to_string()).is_ok(), "explicit streams are supported");
}

fn problems(value: &Value) -> Vec<(ValidationCode, String, String)> {
    Manifest::parse(&value.to_string())
        .expect_err("refused")
        .0
        .into_iter()
        .map(|error| (error.code, error.path, error.message))
        .collect()
}

fn refused_at(value: &Value, path: &str) -> (ValidationCode, String) {
    let found = problems(value);
    found
        .iter()
        .find(|(_, at, _)| at == path)
        .map(|(code, _, message)| (*code, message.clone()))
        .unwrap_or_else(|| panic!("nothing refused at {path}: {found:?}"))
}

#[test]
fn an_executable_app_may_do_all_its_work_in_its_sidecar() {
    let parsed = Manifest::parse(&manifest().to_string()).unwrap();
    assert_eq!(parsed.kind, ManifestKind::Executable);
    assert!(parsed.capabilities.is_empty());
    assert_eq!(
        parsed.sidecar_binary("linux-amd64"),
        Some("bin/linux-amd64/scanner")
    );
    assert_eq!(parsed.sidecar_binary("darwin-arm64"), None);
    assert_eq!(parsed.operation("scan").unwrap().title, "Scan an image");
    // The stored form is the one parsed: nothing is added or dropped on the way.
    let stored = serde_json::to_value(&parsed).unwrap();
    let mut expected = manifest();
    expected["sidecar"]["operations"][0]["inputs"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .for_each(|input| {
            input
                .as_object_mut()
                .unwrap()
                .retain(|_, v| v != &json!(false));
        });
    assert_eq!(stored, expected);
}

#[test]
fn a_declarative_app_still_needs_a_capability() {
    let mut value = manifest();
    value["kind"] = json!("declarative");
    value.as_object_mut().unwrap().remove("sidecar");
    value["srelensApiVersion"] = json!("^0.5");
    let (code, message) = refused_at(&value, "capabilities");
    assert_eq!(code, ValidationCode::InvalidValue);
    assert!(message.contains("1–32"), "{message}");
}

#[test]
fn the_kind_and_the_sidecar_come_together() {
    let mut declarative = manifest();
    declarative["kind"] = json!("declarative");
    let (code, message) = refused_at(&declarative, "sidecar");
    assert_eq!(code, ValidationCode::InvalidKind);
    assert!(message.contains("set kind to \"executable\""), "{message}");

    let mut bare = manifest();
    bare.as_object_mut().unwrap().remove("sidecar");
    let (code, message) = refused_at(&bare, "kind");
    assert_eq!(code, ValidationCode::InvalidKind);
    assert!(message.contains("sidecar"), "{message}");
}

#[test]
fn a_binary_is_a_file_directly_under_its_platforms_directory() {
    for (platform, path, at) in [
        (
            "linux-amd64",
            "bin/linux-arm64/scanner",
            "sidecar.binaries.linux-amd64",
        ),
        (
            "linux-amd64",
            "bin/linux-amd64/tools/scanner",
            "sidecar.binaries.linux-amd64",
        ),
        (
            "linux-amd64",
            "bin/linux-amd64/.scanner",
            "sidecar.binaries.linux-amd64",
        ),
        ("linux-amd64", "scanner", "sidecar.binaries.linux-amd64"),
        (
            "linux-amd64",
            "bin/linux-amd64/",
            "sidecar.binaries.linux-amd64",
        ),
        (
            "freebsd-amd64",
            "bin/freebsd-amd64/scanner",
            "sidecar.binaries.freebsd-amd64",
        ),
    ] {
        let mut value = manifest();
        value["sidecar"]["binaries"] = json!({ platform: path });
        let (code, _) = refused_at(&value, at);
        assert_eq!(code, ValidationCode::InvalidValue, "{platform}: {path}");
    }
    let mut none = manifest();
    none["sidecar"]["binaries"] = json!({});
    refused_at(&none, "sidecar.binaries");
    // Every platform a package may carry is one a binary may be named for.
    let mut every = manifest();
    every["sidecar"]["binaries"] = SIDECAR_PLATFORMS
        .iter()
        .map(|platform| {
            (
                platform.to_string(),
                json!(format!("bin/{platform}/scanner")),
            )
        })
        .collect::<serde_json::Map<_, _>>()
        .into();
    Manifest::parse(&every.to_string()).unwrap();
}

#[test]
fn this_host_names_its_own_platform_as_bin_does() {
    if let Some(platform) = host_platform() {
        assert!(SIDECAR_PLATFORMS.contains(&platform), "{platform}");
    }
}

#[test]
fn operations_share_one_name_space_with_bindings_and_skip_the_hosts_own_methods() {
    let mut value = manifest();
    value["permissions"] = json!(["k8s.listCustomResource"]);
    value["capabilities"] = json!([{"name":"scan","title":"Scans","target":"k8s.listCustomResource",
        "arguments":{"group":"aquasecurity.github.io","version":"v1alpha1","plural":"vulnerabilityreports",
            "kind":"VulnerabilityReport","namespaced":true},"inputs":["context","namespace"]}]);
    let (code, message) = refused_at(&value, "sidecar.operations[0].name");
    assert_eq!(code, ValidationCode::DuplicateIdentifier);
    assert!(message.contains("already used"), "{message}");

    for reserved in ["initialize", "activate", "health", "shutdown", "deactivate"] {
        let mut value = manifest();
        value["sidecar"]["operations"][0]["name"] = json!(reserved);
        let (code, message) = refused_at(&value, "sidecar.operations[0].name");
        assert_eq!(code, ValidationCode::InvalidValue, "{reserved}");
        assert!(message.contains("srelens's own"), "{message}");
    }
    let mut none = manifest();
    none["sidecar"]["operations"] = json!([]);
    refused_at(&none, "sidecar.operations");
}

#[test]
fn inputs_are_named_once_and_only_a_string_has_a_length() {
    let input = |extra: Value| {
        let mut value = manifest();
        value["sidecar"]["operations"][0]["inputs"] = json!([extra]);
        value
    };
    for (declared, at) in [
        (
            json!({"name":"limit","type":"integer","maxLength":4}),
            "maxLength",
        ),
        (
            json!({"name":"image","type":"string","maxLength":0}),
            "maxLength",
        ),
        (
            json!({"name":"image","type":"string","maxLength":65537}),
            "maxLength",
        ),
        (json!({"name":"image name","type":"string"}), "name"),
        (
            json!({"name":"image","type":"string","title":"\u{202E}egami"}),
            "title",
        ),
    ] {
        let at = format!("sidecar.operations[0].inputs[0].{at}");
        let (code, _) = refused_at(&input(declared.clone()), &at);
        assert_eq!(code, ValidationCode::InvalidValue, "{declared}");
    }
    let mut twice = manifest();
    twice["sidecar"]["operations"][0]["inputs"] =
        json!([{"name":"image","type":"string"},{"name":"image","type":"integer"}]);
    let (code, _) = refused_at(&twice, "sidecar.operations[0].inputs[1].name");
    assert_eq!(code, ValidationCode::DuplicateIdentifier);
    let mut unknown = manifest();
    unknown["sidecar"]["operations"][0]["inputs"] = json!([{"name":"image","type":"object"}]);
    assert!(Manifest::parse(&unknown.to_string()).is_err());
}

#[test]
fn the_host_builds_the_input_schema_from_the_declarations() {
    let parsed = Manifest::parse(&manifest().to_string()).unwrap();
    let schema = parsed.operation("scan").unwrap().input_schema();
    assert_eq!(
        schema,
        json!({
            "type":"object",
            "properties":{
                "image":{"type":"string","maxLength":512,"title":"Image reference"},
                "severity":{"type":"string","maxLength":1024},
                "limit":{"type":"integer"},
                "ratio":{"type":"number"},
                "fixable":{"type":"boolean"}
            },
            "required":["image"],
            "additionalProperties":false
        })
    );
}

#[test]
fn every_call_is_held_to_the_declarations_before_the_sidecar_sees_it() {
    let parsed = Manifest::parse(&manifest().to_string()).unwrap();
    let scan = parsed.operation("scan").unwrap();
    let accepted = scan
        .check_input(&json!({"image":"nginx:1.27","limit":5,"ratio":0.5,"fixable":true}))
        .unwrap();
    assert_eq!(accepted["image"], "nginx:1.27");
    for (input, says) in [
        (json!("nginx"), "takes an object"),
        (json!({}), "requires `image`"),
        (json!({"image":"nginx","tag":"x"}), "takes no input `tag`"),
        (json!({"image":7}), "`image` must be a string"),
        (
            json!({"image":"nginx","limit":1.5}),
            "`limit` must be a integer",
        ),
        (
            json!({"image":"nginx","fixable":"yes"}),
            "`fixable` must be a boolean",
        ),
        (
            json!({"image":"n".repeat(513)}),
            "`image` is at most 512 bytes",
        ),
        (
            json!({"image":"nginx","severity":"s".repeat(1025)}),
            "`severity` is at most 1024 bytes",
        ),
    ] {
        let why = scan.check_input(&input).unwrap_err();
        assert!(why.contains(says), "{input}: {why}");
        // A refusal names the field, never the value it was given.
        assert!(!why.contains("nnnn") && !why.contains("ssss"), "{why}");
    }
    // Every field has its own limit; the whole call is bounded too.
    let huge = json!({"image":"nginx","severity":"s".repeat(MAX_OPERATION_CALL_BYTES)});
    let why = scan.check_input(&huge).unwrap_err();
    assert!(why.contains("exceeds 256 KiB"), "{why}");
}

#[test]
fn an_operation_runs_under_the_hosts_row_not_the_apps() {
    // Changes nothing outside the sandbox, so it is not gated; its arguments are the
    // app's own vocabulary, so the audit log redacts them whole.
    let row = SIDECAR_OPERATION;
    assert_eq!(
        (row.read_only, row.requires_confirm, row.destructive, row.sensitive, row.impact),
        (true, false, false, true, srelens_capability::Impact::Low)
    );
}

/// The manifest reference's example is a manifest this host accepts: a claim in prose
/// is not a caller, so the example is parsed like one.
#[test]
fn the_reference_example_is_a_manifest_this_host_accepts() {
    // LF in the index but CRLF in a `core.autocrlf=true` checkout; the splits below want LF.
    let reference = include_str!("../../../docs/extensions/manifest.md").replace("\r\n", "\n");
    let section = reference
        .split("\n## Executable apps\n")
        .nth(1)
        .expect("manifest.md has an Executable apps section");
    let example = section
        .split("```json\n")
        .nth(1)
        .and_then(|rest| rest.split("\n```").next())
        .expect("the section opens with a JSON example");
    let parsed = Manifest::parse(example).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(parsed.kind, ManifestKind::Executable);
    let schema = parsed.operation("scan").unwrap().input_schema();
    assert_eq!(schema["required"], json!(["image"]));
}
