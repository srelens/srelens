//! Manifest problems are reported together, each with a stable code and the JSON path
//! of the value at fault, so an author can fix several in one pass.
use serde_json::{json, Value};
use srelens_plugin_host::{Manifest, ValidationCode, ValidationError};

fn manifest() -> Value {
    json!({
        "id":"org.example.gitops", "name":"GitOps", "version":"0.1.0", "srelensApiVersion":"^0.1",
        "kind":"declarative", "permissions":["k8s.listCustomResource"],
        "capabilities":[{"name":"applications","title":"List applications", "target":"k8s.listCustomResource",
            "arguments":{"group":"argoproj.io"},"inputs":["context","namespace"]}],
        "contributions":{"pages":[{"id":"applications","title":"Applications","capability":"applications"}],
            "detailTabs":[],"rowActions":[]}
    })
}

fn errors(value: &Value) -> Vec<ValidationError> {
    Manifest::parse(&value.to_string()).unwrap_err().0
}

fn code(code: ValidationCode) -> String {
    serde_json::to_value(code)
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned()
}

/// Codes and paths, sorted so the assertion does not depend on check order.
fn problems(errors: &[ValidationError]) -> Vec<(String, String)> {
    let mut problems: Vec<_> = errors
        .iter()
        .map(|error| (code(error.code), error.path.clone()))
        .collect();
    problems.sort();
    problems
}

fn expected(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut pairs: Vec<_> = pairs
        .iter()
        .map(|(code, path)| (code.to_string(), path.to_string()))
        .collect();
    pairs.sort();
    pairs
}

#[test]
fn independent_problems_are_all_reported_with_their_paths() {
    let mut value = manifest();
    value["id"] = json!("Not a domain");
    value["contributions"]["pages"][0]["capability"] = json!("applications.delete");
    value["contributions"]["detailTabs"] = json!([{
        "id":"detail", "title":"Details", "capability":"applications", "forKinds":["Application"]
    }]);
    let errors = errors(&value);
    assert_eq!(
        problems(&errors),
        expected(&[
            ("EXTENSION_INVALID_ID", "id"),
            (
                "EXTENSION_UNRESOLVED_CAPABILITY",
                "contributions.pages[0].capability"
            ),
            (
                "EXTENSION_INVALID_KIND",
                "contributions.detailTabs[0].forKinds[0]"
            ),
        ])
    );
    let unresolved = errors
        .iter()
        .find(|error| error.path == "contributions.pages[0].capability")
        .unwrap();
    assert_eq!(
        unresolved.message,
        "Capability \"applications.delete\" is not declared"
    );
}

#[test]
fn schema_errors_name_the_field_rather_than_a_line_and_column() {
    let cases: [(fn(&mut Value), &str, &str); 4] = [
        (
            |value| value["contributions"]["pages"][0]["badge"] = json!("new"),
            "EXTENSION_UNKNOWN_FIELD",
            "contributions.pages[0].badge",
        ),
        (
            |value| value["capabilities"][0]["inputs"] = json!("context"),
            "EXTENSION_INVALID_FIELD",
            "capabilities[0].inputs",
        ),
        (
            |value| {
                value.as_object_mut().unwrap().remove("permissions");
            },
            "EXTENSION_INVALID_FIELD",
            "permissions",
        ),
        (
            |value| value["kind"] = json!("lens-compat"),
            "EXTENSION_INVALID_KIND",
            "kind",
        ),
    ];
    for (change, want_code, want_path) in cases {
        let mut value = manifest();
        change(&mut value);
        let errors = errors(&value);
        assert_eq!(
            problems(&errors),
            expected(&[(want_code, want_path)]),
            "{value}"
        );
        assert!(
            !errors[0].message.contains(" line "),
            "{}",
            errors[0].message
        );
    }
}

#[test]
fn whole_manifest_problems_have_an_empty_path() {
    let too_large = Manifest::parse(&" ".repeat(256 * 1024 + 1)).unwrap_err().0;
    assert_eq!(
        problems(&too_large),
        expected(&[("EXTENSION_TOO_LARGE", "")])
    );
    let not_json = Manifest::parse("{\"id\":").unwrap_err().0;
    assert_eq!(
        problems(&not_json),
        expected(&[("EXTENSION_INVALID_JSON", "")])
    );
}

#[test]
fn rule_violations_carry_their_codes() {
    let cases: [(fn(&mut Value), &str, &str); 6] = [
        (
            |value| value["srelensApiVersion"] = json!("^99"),
            "EXTENSION_API_INCOMPATIBLE",
            "srelensApiVersion",
        ),
        (
            |value| value["version"] = json!("one"),
            "EXTENSION_INVALID_VERSION",
            "version",
        ),
        (
            |value| value["permissions"] = json!(["k8s.listEvents"]),
            "EXTENSION_PERMISSION_MISMATCH",
            "permissions",
        ),
        (
            |value| {
                value["capabilities"][0]["target"] = json!("plugin/org.other.app/list");
                value["permissions"] = json!(["plugin/org.other.app/list"]);
            },
            "EXTENSION_UNSUPPORTED_TARGET",
            "capabilities[0].target",
        ),
        (
            |value| {
                value["contributions"]["detailTabs"] = json!([{
                    "id":"applications", "title":"Details", "capability":"applications",
                    "forKinds":["argoproj.io/Application"]
                }]);
            },
            "EXTENSION_DUPLICATE_IDENTIFIER",
            "contributions.detailTabs[0].id",
        ),
        (
            |value| {
                value["contributions"]["pages"][0]["statusColumns"] = json!({"ready": 0});
                value["contributions"]["pages"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({
                        "id":"overview", "title":"Overview", "capability":"applications",
                        "dashboard":{"pages":["missing"]}
                    }));
            },
            "EXTENSION_UNRESOLVED_PAGE",
            "contributions.pages[1].dashboard.pages[0]",
        ),
    ];
    for (change, want_code, want_path) in cases {
        let mut value = manifest();
        change(&mut value);
        assert_eq!(
            problems(&errors(&value)),
            expected(&[(want_code, want_path)]),
            "{value}"
        );
    }
}

#[test]
fn errors_use_the_camel_case_wire_shape_and_read_as_one_line_each() {
    let mut value = manifest();
    value["id"] = json!("Not a domain");
    value["version"] = json!("one");
    let errors = Manifest::parse(&value.to_string()).unwrap_err();
    let first = &errors.0[0];
    assert_eq!(
        serde_json::to_value(first).unwrap(),
        json!({"code": code(first.code), "path": first.path, "message": first.message})
    );
    let text = errors.to_string();
    assert_eq!(text.lines().count(), 2, "{text}");
    assert!(
        text.lines()
            .any(|line| line.starts_with("id: ") && line.ends_with("(EXTENSION_INVALID_ID)")),
        "{text}"
    );
}

#[test]
fn every_code_is_documented_in_the_specification() {
    let specification = include_str!("../../../docs/extensions/specification.md");
    for &each in ValidationCode::ALL {
        let name = code(each);
        assert!(
            specification.contains(&format!("`{name}`")),
            "{name} is not documented"
        );
    }
}
