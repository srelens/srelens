//! What a declared predicate (#550) means, one operator at a time, and what
//! the host refuses to accept as one.

use serde_json::{json, Value};
use srelens_capability::{check_predicates, resolve, unmet, Predicate, MAX_PREDICATES};

/// A predicate from its JSON, as a manifest writes one.
fn predicate(value: Value) -> Predicate {
    serde_json::from_value(value).expect("a predicate deserializes")
}

/// A HelmRelease as the API server serves one.
fn object() -> Value {
    json!({
        "apiVersion": "helm.toolkit.fluxcd.io/v2",
        "kind": "HelmRelease",
        "metadata": {
            "name": "api",
            "namespace": "team",
            "annotations": {"acme.io/pinned": "yes"}
        },
        "spec": {"suspend": true, "chart": {"spec": {"version": "1.2.3"}}},
        "status": {"conditions": [{"type": "Ready", "status": "False"}]}
    })
}

#[test]
fn equals_holds_only_when_the_value_at_the_path_is_that_literal() {
    let suspended = predicate(json!({"jsonPath": ".spec.suspend", "equals": true, "reason": "r"}));
    assert!(suspended.holds(&object()));
    assert!(!suspended.holds(&json!({"spec": {"suspend": false}})));
    // Absent is not equal to anything.
    assert!(!suspended.holds(&json!({"spec": {}})));
    // And a literal of another type is not the same value.
    assert!(!suspended.holds(&json!({"spec": {"suspend": "true"}})));
}

#[test]
fn not_equals_holds_when_the_value_differs_or_is_not_set() {
    let live = predicate(json!({"jsonPath": ".spec.suspend", "notEquals": true, "reason": "r"}));
    assert!(!live.holds(&object()), "suspended is exactly the literal");
    assert!(live.holds(&json!({"spec": {"suspend": false}})));
    assert!(
        live.holds(&json!({"spec": {}})),
        "a field nobody set is not the literal, which is what `.spec.suspend notEquals true` is for"
    );
}

#[test]
fn present_and_absent_read_null_as_unset() {
    let running = predicate(json!({"jsonPath": ".operation", "present": true, "reason": "r"}));
    let idle = predicate(json!({"jsonPath": ".operation", "absent": true, "reason": "r"}));
    let with = json!({"operation": {"sync": {}}});
    assert!(running.holds(&with) && !idle.holds(&with));
    for without in [json!({}), json!({"operation": null})] {
        assert!(
            idle.holds(&without) && !running.holds(&without),
            "{without} reads as unset"
        );
    }
}

#[test]
fn a_path_addresses_quoted_keys_and_list_elements() {
    let annotated = predicate(
        json!({"jsonPath": ".metadata.annotations['acme.io/pinned']", "equals": "yes", "reason": "r"}),
    );
    let ready = predicate(
        json!({"jsonPath": ".status.conditions[0].status", "equals": "True", "reason": "r"}),
    );
    assert!(annotated.holds(&object()));
    assert!(!ready.holds(&object()), "the first condition is False");
    assert_eq!(
        resolve(&object(), ".spec.chart.spec.version"),
        Some(&json!("1.2.3"))
    );
    assert_eq!(resolve(&object(), "$.metadata.name"), Some(&json!("api")));
    assert_eq!(resolve(&object(), ".spec.missing"), None);
    assert_eq!(resolve(&object(), ".status.conditions[9]"), None);
}

#[test]
fn a_path_the_host_does_not_evaluate_is_refused_rather_than_read_as_false() {
    for path in [
        "spec.suspend",                                  // no leading `.`
        ".spec..suspend",                                // empty segment
        ".spec.*",                                       // wildcard
        ".status.conditions[?(@.type=='Ready')].status", // filter
        "..suspend",                                     // recursive descent
        ".metadata.annotations[acme.io/x]",              // unquoted odd key
        ".spec.a.b.c.d.e.f.g.h.i",                       // deeper than the bound
        ".spec[01]",                                     // a second spelling of `[1]`
    ] {
        let refused = predicate(json!({"jsonPath": path, "present": true, "reason": "r"}));
        assert!(
            refused.check().is_err(),
            "`{path}` must be refused at install"
        );
        assert!(
            !refused.holds(&object()),
            "`{path}` must not hold either: a check the host cannot evaluate fails closed"
        );
    }
}

#[test]
fn exactly_one_operator_is_declared() {
    let two = predicate(
        json!({"jsonPath": ".spec.suspend", "equals": true, "present": true, "reason": "r"}),
    );
    let none = predicate(json!({"jsonPath": ".spec.suspend", "reason": "r"}));
    for bad in [&two, &none] {
        let why = bad.check().expect_err("refused");
        assert!(why.contains("exactly one"), "{why}");
    }
    // `present: false` is a second spelling of `absent`, so it is not one.
    let inverted = predicate(json!({"jsonPath": ".operation", "present": false, "reason": "r"}));
    assert!(inverted.check().is_err());
}

#[test]
fn a_comparand_is_a_literal() {
    for comparand in [json!({"suspend": true}), json!([1, 2])] {
        let refused = predicate(json!({"jsonPath": ".spec", "equals": comparand, "reason": "r"}));
        let why = refused.check().expect_err("refused");
        assert!(why.contains("literal"), "{why}");
    }
    for comparand in [json!("text"), json!(3), json!(false)] {
        let accepted =
            predicate(json!({"jsonPath": ".spec.x", "notEquals": comparand, "reason": "r"}));
        accepted.check().expect("a literal is a comparand");
    }
}

#[test]
fn a_reason_is_required_and_bounded() {
    let blank = predicate(json!({"jsonPath": ".spec.suspend", "equals": true, "reason": "  "}));
    assert!(blank.check().is_err());
    let long =
        predicate(json!({"jsonPath": ".spec.suspend", "equals": true, "reason": "x".repeat(500)}));
    assert!(long.check().is_err());
}

#[test]
fn the_reason_is_escaped_before_it_is_shown() {
    let spoofed = predicate(json!({
        "jsonPath": ".spec.suspend",
        "notEquals": true,
        "reason": "Resume first\u{202e}\u{200b} and retry"
    }));
    let shown = spoofed.reason();
    assert!(
        !shown.contains('\u{202e}') && !shown.contains('\u{200b}'),
        "a manifest's reason cannot reorder or hide the host's words: {shown}"
    );
    assert!(
        shown.contains("\\u{202e}") && shown.starts_with("Resume first"),
        "{shown}"
    );
}

#[test]
fn unmet_reports_the_first_failing_predicate_per_object() {
    let declared = vec![
        predicate(
            json!({"jsonPath": ".operation", "absent": true, "reason": "A sync is already running"}),
        ),
        predicate(
            json!({"jsonPath": ".spec.suspend", "notEquals": true, "reason": "Resume this resource before requesting reconciliation"}),
        ),
    ];
    check_predicates(&declared).expect("both are well formed");
    assert_eq!(
        unmet(&declared, &object())
            .map(Predicate::reason)
            .as_deref(),
        Some("Resume this resource before requesting reconciliation")
    );
    assert!(unmet(&declared, &json!({"spec": {"suspend": false}})).is_none());
    // Per object, so a list view can ask the same question of every row.
    let rows = [object(), json!({"spec": {}}), json!({"operation": {}})];
    assert_eq!(
        rows.iter()
            .filter(|row| unmet(&declared, row).is_none())
            .count(),
        1
    );
}

#[test]
fn a_list_of_predicates_is_bounded() {
    let one = predicate(json!({"jsonPath": ".spec.x", "present": true, "reason": "r"}));
    let many = vec![one.clone(); MAX_PREDICATES + 1];
    assert!(check_predicates(&many).is_err());
    assert!(check_predicates(&vec![one; MAX_PREDICATES]).is_ok());
    assert!(check_predicates(&[]).is_ok());
}
