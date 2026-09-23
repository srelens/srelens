//! Manifest problems are reported together, each with a stable code and the JSON path
//! of the value at fault, so an author can fix several in one pass.
use serde_json::{json, Value};
use srelens_plugin_host::{Manifest, ValidationCode, ValidationError};

fn manifest() -> Value {
    json!({
        "id":"org.example.gitops", "name":"GitOps", "version":"0.1.0", "srelensApiVersion":"^0.3",
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
fn table_columns_and_joins_require_declared_readers_and_bounded_sources() {
    let mut value = manifest();
    value["contributions"]["joins"] = json!([{
        "id":"vulns", "capability":"applications",
        "match":{"label":"trivy-operator.resource.name","kindLabel":"trivy-operator.resource.kind"}
    }]);
    value["contributions"]["tableColumns"] = json!([{
        "id":"critical", "title":"Critical CVEs", "forKinds":["apps/Deployment"],
        "source":{"join":"vulns","jsonPath":".report.summary.criticalCount"},
        "format":"number", "sortable":true, "filterable":true
    }]);
    let parsed = Manifest::parse(&value.to_string()).expect("a declared join and column are valid");
    assert_eq!(parsed.contributions.joins[0].id, "vulns");
    assert_eq!(parsed.contributions.table_columns[0].id, "critical");

    value["contributions"]["joins"][0]["capability"] = json!("missing");
    value["contributions"]["tableColumns"][0]["source"]["join"] = json!("absent");
    assert_eq!(
        problems(&errors(&value)),
        expected(&[
            (
                "EXTENSION_UNRESOLVED_CAPABILITY",
                "contributions.joins[0].capability"
            ),
            (
                "EXTENSION_INVALID_BINDING",
                "contributions.tableColumns[0].source.join"
            ),
        ])
    );
}

/// The manifest with one card of each type over its custom-resource reader.
fn with_cards() -> Value {
    let mut value = manifest();
    value["contributions"]["dashboardCards"] = json!([
        {"id":"expiring", "title":"Certificates expiring soon", "size":"s", "type":"count",
         "source":"applications", "predicate":{"jsonPath":".status.notAfter","within":"14d"},
         "target":{"page":"applications"}},
        {"id":"by-status", "title":"By status", "size":"m", "type":"countByStatus", "source":"applications"},
        {"id":"critical", "title":"Critical CVEs", "size":"m", "type":"metric", "source":"applications",
         "metric":{"jsonPath":".report.summary.criticalCount","aggregate":"sum"}},
        {"id":"soonest", "title":"Soonest to expire", "size":"l", "type":"list", "source":"applications",
         "predicate":{"jsonPath":".status.notAfter","before":"30d"},
         "list":{"jsonPath":".status.notAfter","order":"asc","limit":5}}
    ]);
    value
}

#[test]
fn dashboard_cards_of_every_type_and_size_parse() {
    use srelens_plugin_host::{CardAggregate, CardSize, CardType};
    let parsed = Manifest::parse(&with_cards().to_string()).expect("every card type is valid");
    let cards = &parsed.contributions.dashboard_cards;
    assert_eq!(
        cards.iter().map(|c| c.card_type).collect::<Vec<_>>(),
        [CardType::Count, CardType::CountByStatus, CardType::Metric, CardType::List]
    );
    assert_eq!(
        cards.iter().map(|c| c.size).collect::<Vec<_>>(),
        [CardSize::S, CardSize::M, CardSize::M, CardSize::L]
    );
    assert_eq!(cards[2].metric.as_ref().unwrap().aggregate, CardAggregate::Sum);
    assert_eq!(cards[0].target.as_ref().unwrap().page, "applications");
    // Round-trips to the same JSON: a stored manifest is re-verified from its own bytes.
    let again: Value = serde_json::to_value(&parsed).unwrap();
    assert_eq!(again["contributions"]["dashboardCards"], with_cards()["contributions"]["dashboardCards"]);
}

#[test]
fn a_card_with_an_unknown_type_or_size_is_a_schema_error_at_its_field() {
    for (field, value) in [("type", "gauge"), ("size", "xl")] {
        let mut manifest = with_cards();
        manifest["contributions"]["dashboardCards"][0][field] = json!(value);
        let errors = errors(&manifest);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(errors[0].path, format!("contributions.dashboardCards[0].{field}"));
        assert!(errors[0].message.contains(value), "{}", errors[0].message);
    }
}

#[test]
fn card_rules_are_reported_at_the_field_that_has_to_change() {
    let mut value = with_cards();
    let cards = &mut value["contributions"]["dashboardCards"];
    cards[0]["source"] = json!("missing");
    cards[0]["predicate"] = json!({"jsonPath":".status.notAfter","within":"soon"});
    cards[0]["target"] = json!({"page":"nowhere"});
    cards[1]["id"] = json!("expiring");
    cards[1]["title"] = json!("By\u{202e}status");
    cards[1]["metric"] = json!({"jsonPath":".x","aggregate":"sum"});
    cards[2].as_object_mut().unwrap().remove("metric");
    cards[3]["list"]["limit"] = json!(11);
    cards[3]["list"]["jsonPath"] = json!(".status[*].notAfter");
    assert_eq!(
        problems(&errors(&value)),
        expected(&[
            ("EXTENSION_UNRESOLVED_CAPABILITY", "contributions.dashboardCards[0].source"),
            ("EXTENSION_INVALID_BINDING", "contributions.dashboardCards[0].predicate"),
            ("EXTENSION_UNRESOLVED_PAGE", "contributions.dashboardCards[0].target.page"),
            ("EXTENSION_DUPLICATE_IDENTIFIER", "contributions.dashboardCards[1].id"),
            ("EXTENSION_INVALID_VALUE", "contributions.dashboardCards[1].title"),
            ("EXTENSION_INVALID_BINDING", "contributions.dashboardCards[1].metric"),
            ("EXTENSION_INVALID_BINDING", "contributions.dashboardCards[2].metric"),
            ("EXTENSION_INVALID_VALUE", "contributions.dashboardCards[3].list.limit"),
            ("EXTENSION_INVALID_VALUE", "contributions.dashboardCards[3].list.jsonPath"),
        ])
    );
}

#[test]
fn a_bad_card_path_fails_install_rather_than_counting_nothing() {
    for (card, field, path) in [
        (0, "predicate", "status.notAfter"),
        (2, "metric", ".report..critical"),
    ] {
        let mut value = with_cards();
        value["contributions"]["dashboardCards"][card][field]["jsonPath"] = json!(path);
        let found = errors(&value);
        let at = if field == "predicate" {
            format!("contributions.dashboardCards[{card}].predicate")
        } else {
            format!("contributions.dashboardCards[{card}].metric.jsonPath")
        };
        assert!(found.iter().any(|e| e.path == at), "{path}: {found:?}");
    }
}

#[test]
fn a_card_source_is_a_custom_resource_reader_and_its_target_a_page_over_it() {
    let mut value = with_cards();
    value["permissions"] = json!(["k8s.listCustomResource", "k8s.listEvents"]);
    value["capabilities"].as_array_mut().unwrap().push(json!({
        "name":"events","title":"Events","target":"k8s.listEvents","arguments":{},"inputs":["context","namespace"]}));
    value["capabilities"].as_array_mut().unwrap().push(json!({
        "name":"projects","title":"Projects","target":"k8s.listCustomResource",
        "arguments":{"group":"argoproj.io"},"inputs":["context","namespace"]}));
    value["contributions"]["pages"].as_array_mut().unwrap().push(json!(
        {"id":"projects","title":"Projects","capability":"projects"}));
    let cards = &mut value["contributions"]["dashboardCards"];
    cards[1]["source"] = json!("events");
    // The target page lists another source, so the card's predicate would filter the wrong rows.
    cards[0]["target"] = json!({"page":"projects"});
    assert_eq!(
        problems(&errors(&value)),
        expected(&[
            ("EXTENSION_UNRESOLVED_CAPABILITY", "contributions.dashboardCards[1].source"),
            ("EXTENSION_INVALID_BINDING", "contributions.dashboardCards[0].target.page"),
        ])
    );
}

#[test]
fn a_manifest_declares_at_most_sixteen_cards() {
    let mut value = manifest();
    let cards: Vec<Value> = (0..17)
        .map(|i| json!({"id":format!("card-{i}"),"title":"Card","size":"s","type":"count","source":"applications"}))
        .collect();
    value["contributions"]["dashboardCards"] = json!(cards);
    assert_eq!(
        problems(&errors(&value)),
        expected(&[("EXTENSION_INVALID_VALUE", "contributions.dashboardCards")])
    );
    value["contributions"]["dashboardCards"].as_array_mut().unwrap().pop();
    assert!(Manifest::parse(&value.to_string()).is_ok());
}

#[test]
fn table_column_rejects_malformed_json_paths_instead_of_silently_blank_cells() {
    let mut value = manifest();
    value["contributions"]["tableColumns"] = json!([{
        "id":"score", "title":"Score", "forKinds":["apps/Deployment"],
        "source":{"jsonPath":".status..score"}, "format":"number"
    }]);
    assert_eq!(
        problems(&errors(&value)),
        expected(&[(
            "EXTENSION_INVALID_VALUE",
            "contributions.tableColumns[0].source.jsonPath"
        )])
    );
    value["contributions"]["tableColumns"][0]["source"]["jsonPath"] =
        json!(".status['unterminated'");
    assert_eq!(
        problems(&errors(&value)),
        expected(&[(
            "EXTENSION_INVALID_VALUE",
            "contributions.tableColumns[0].source.jsonPath"
        )])
    );
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
    // An input name repeated as a bound argument is quoted by a different rule.
    value["capabilities"][0]["inputs"] = json!(["\u{202E}context"]);
    value["capabilities"][0]["arguments"]["\u{202E}context"] = json!("x");
    let found = errors(&value);
    let messages: Vec<&str> = found.iter().map(|e| e.message.as_str()).collect();
    assert!(
        messages
            .iter()
            .any(|m| m.contains("is a bound argument") && m.contains("\\u{202e}")),
        "the input collision quotes an escape: {messages:?}"
    );
    assert!(
        messages.iter().any(|m| m.contains("\\u{202e}")),
        "the override is shown as an escape: {messages:?}"
    );
    assert!(
        messages.iter().any(|m| m.contains("\\u{8}")),
        "the control character is shown as an escape: {messages:?}"
    );
    for problem in &found {
        for text in [problem.message.as_str(), problem.path.as_str()] {
            assert!(
                !text
                    .chars()
                    .any(|c| c.is_control() || srelens_plugin_host::is_format_character(c)),
                "raw character in {text:?}"
            );
        }
    }
    // A manifest that is not JSON is reported through the same funnel.
    let broken = srelens_plugin_host::Manifest::parse("{\"name\": \"\u{202E}").unwrap_err();
    assert!(
        !broken
            .to_string()
            .chars()
            .any(srelens_plugin_host::is_format_character),
        "{broken}"
    );
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
    value["contributions"]["notYetAContribution"] = json!([]);
    assert_eq!(
        problems(&errors(&value)),
        expected(&[("EXTENSION_API_INCOMPATIBLE", "srelensApiVersion")])
    );
}

#[test]
fn too_many_printer_columns_are_refused_at_the_field_path() {
    use srelens_plugin_host::MAX_PRINTER_COLUMNS;
    let mut value = manifest();
    let columns: Vec<_> = (0..=MAX_PRINTER_COLUMNS)
        .map(|i| json!({"name": format!("Col{i}"), "jsonPath": ".status.x"}))
        .collect();
    value["capabilities"][0]["arguments"]["printerColumns"] = json!(columns);
    let errors = errors(&value);
    assert_eq!(
        problems(&errors),
        expected(&[(
            "EXTENSION_INVALID_VALUE",
            "capabilities[0].arguments.printerColumns"
        )])
    );
    assert!(errors[0]
        .message
        .contains(&format!("at most {MAX_PRINTER_COLUMNS}")));
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
