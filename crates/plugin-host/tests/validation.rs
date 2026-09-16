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
            "detailTabs":[],"detailLinks":[]}
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
    value["contributions"]["detailLinks"] = json!([{
        "id":"inspect", "title":"Inspect", "capability":"applications", "forKinds":["Application"]
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
            (
                "EXTENSION_INVALID_KIND",
                "contributions.detailLinks[0].forKinds[0]"
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
    let cases: [(fn(&mut Value), &str, &str); 5] = [
        (
            |value| value["contributions"]["pages"][0]["badge"] = json!("new"),
            "EXTENSION_UNKNOWN_FIELD",
            "contributions.pages[0].badge",
        ),
        // The pre-release name, renamed to `detailLinks` and reserved for declared mutations.
        (
            |value| {
                let contributions = value["contributions"].as_object_mut().unwrap();
                contributions.remove("detailLinks");
                contributions.insert("rowActions".into(), json!([]));
            },
            "EXTENSION_UNKNOWN_FIELD",
            "contributions.rowActions",
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
fn names_titles_and_groups_refuse_bidirectional_and_invisible_characters() {
    let cases: [(fn(&mut Value), &str); 5] = [
        // A right-to-left override displays this name as "Argo CD".
        (|value| value["name"] = json!("\u{202E}DC ogrA"), "name"),
        (
            |value| value["contributions"]["pages"][0]["title"] = json!("Applications\u{200B}"),
            "contributions.pages[0].title",
        ),
        (
            |value| value["capabilities"][0]["title"] = json!("\u{2066}List applications\u{2069}"),
            "capabilities[0].title",
        ),
        (
            |value| value["contributions"]["pages"][0]["group"] = json!("Git\u{FEFF}Ops"),
            "contributions.pages[0].group",
        ),
        (
            |value| {
                value["contributions"]["detailLinks"] = json!([{
                    "id":"inspect", "title":"Inspect\u{2060}", "capability":"applications",
                    "forKinds":["argoproj.io/Application"]
                }]);
            },
            "contributions.detailLinks[0].title",
        ),
    ];
    for (change, want_path) in cases {
        let mut value = manifest();
        change(&mut value);
        assert_eq!(
            problems(&errors(&value)),
            expected(&[("EXTENSION_INVALID_VALUE", want_path)]),
            "{value}"
        );
    }
    // Every format character (Unicode general category Cf) is refused, not only the
    // bidirectional and zero-width ones: the ends of each range, and the tags outside the BMP.
    let characters = "\u{00AD}\u{0600}\u{0605}\u{061C}\u{06DD}\u{070F}\u{0890}\u{0891}\u{08E2}\
        \u{180E}\u{200B}\u{200F}\u{202A}\u{202E}\u{2060}\u{2064}\u{2066}\u{206F}\u{FEFF}\
        \u{FFF9}\u{FFFB}\u{110BD}\u{110CD}\u{13430}\u{1343F}\u{1BCA0}\u{1BCA3}\u{1D173}\
        \u{1D17A}\u{E0001}\u{E0020}\u{E007F}";
    assert_eq!(characters.chars().count(), 32);
    for character in characters.chars() {
        let mut value = manifest();
        value["name"] = json!(format!("Git{character}Ops"));
        assert_eq!(
            problems(&errors(&value)),
            expected(&[("EXTENSION_INVALID_VALUE", "name")]),
            "U+{:04X}",
            u32::from(character)
        );
    }
}

/// A problem message quotes the rejected value, and the problem list is rendered as the
/// host's own text. A value carrying a bidirectional override or a control character must
/// not reach the screen as-is, or it reorders or reshapes the row that reports it.
#[test]
fn problem_messages_never_echo_control_or_format_characters() {
    let mut value = manifest();
    value["contributions"]["detailLinks"] = json!([{
        "id":"inspect", "title":"Inspect", "capability":"applications",
        "forKinds":["\u{202E}apps/Deployment", "core\u{0008}Pod", "/Pod", "/Pod"]
    }]);
    let found = errors(&value);
    let messages: Vec<&str> = found.iter().map(|e| e.message.as_str()).collect();
    assert!(
        messages.iter().any(|m| m.contains("\\u{202e}")),
        "the override is shown as an escape: {messages:?}"
    );
    assert!(
        messages.iter().any(|m| m.contains("\\u{8}")),
        "the control character is shown as an escape: {messages:?}"
    );
    for message in &messages {
        assert!(
            !message
                .chars()
                .any(|c| c.is_control() || srelens_plugin_host::is_format_character(c)),
            "raw character in {message:?}"
        );
    }
}

#[test]
fn names_and_titles_in_any_script_still_validate() {
    let mut value = manifest();
    value["name"] = json!("Übersicht — café");
    value["capabilities"][0]["title"] = json!("アプリケーション一覧");
    value["contributions"]["pages"][0]["title"] = json!("Приложения");
    value["contributions"]["pages"][0]["group"] = json!("عمليات GitOps");
    value["contributions"]["detailTabs"] = json!([{
        "id":"details", "title":"अनुप्रयोग विवरण", "capability":"applications",
        "forKinds":["argoproj.io/Application"]
    }]);
    value["contributions"]["detailLinks"] = json!([{
        "id":"inspect", "title":"애플리케이션 검사 🔍", "capability":"applications",
        "forKinds":["argoproj.io/Application"]
    }]);
    Manifest::parse(&value.to_string()).unwrap_or_else(|errors| panic!("{errors}"));
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
fn an_unsupported_api_range_is_reported_with_the_other_problems() {
    let mut value = manifest();
    value["srelensApiVersion"] = json!("^99");
    value["id"] = json!("Not a domain");
    assert_eq!(
        problems(&errors(&value)),
        expected(&[
            ("EXTENSION_API_INCOMPATIBLE", "srelensApiVersion"),
            ("EXTENSION_INVALID_ID", "id"),
        ])
    );
    // One this host cannot decode is told the version it needs, not the field it lacks.
    value["contributions"]["dashboardCards"] = json!([]);
    assert_eq!(
        problems(&errors(&value)),
        expected(&[("EXTENSION_API_INCOMPATIBLE", "srelensApiVersion")])
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
