//! What a status rule (#541) resolves an object to: first hit wins, six
//! normalized statuses, a word always, an optional reason read from the object,
//! and nothing the host would refuse at install treated as a match.

use serde_json::{json, Value};
use srelens_capability::status::{
    first_match, resolve_status, resolve_statuses, rule_problems, NormalizedStatus,
    ResolvedStatus, StatusRule, MAX_RULES,
};

fn rules(value: Value) -> Vec<StatusRule> {
    serde_json::from_value(value).expect("rules deserialize")
}

/// The Flux rule set the reference manifest ships, in its order.
fn flux() -> Vec<StatusRule> {
    rules(json!([
        {"when": [{"jsonPath": ".spec.suspend", "equals": true}],
         "status": "suspended", "label": "Suspended"},
        {"when": [{"jsonPath": ".status.conditions[?(@.type==\"Reconciling\")].status", "equals": "True"}],
         "status": "progressing", "label": "Reconciling",
         "reason": ".status.conditions[?(@.type==\"Reconciling\")].message"},
        {"when": [{"jsonPath": ".status.conditions[?(@.type==\"Ready\")].status", "equals": "True"}],
         "status": "healthy", "label": "Ready"},
        {"when": [{"jsonPath": ".status.conditions[?(@.type==\"Ready\")].status", "equals": "False"}],
         "status": "error", "label": "Not ready",
         "reason": ".status.conditions[?(@.type==\"Ready\")].message"},
        {"when": [], "status": "unknown", "label": "Unknown"}
    ]))
}

fn condition(kind: &str, status: &str, message: &str) -> Value {
    json!({"type": kind, "status": status, "message": message})
}

fn resource(suspend: bool, conditions: Vec<Value>) -> Value {
    json!({"spec": {"suspend": suspend}, "status": {"conditions": conditions}})
}

fn resolved(status: NormalizedStatus, label: &str, reason: Option<&str>) -> ResolvedStatus {
    ResolvedStatus {
        status,
        label: label.into(),
        reason: reason.map(str::to_owned),
    }
}

#[test]
fn the_first_rule_that_holds_wins_even_when_a_later_one_also_holds() {
    rule_problems(&flux())
        .is_empty()
        .then_some(())
        .expect("the reference rules are well formed");
    // Suspended AND Ready: the suspend rule is first, so it is the answer.
    let suspended_but_ready = resource(true, vec![condition("Ready", "True", "Applied")]);
    assert_eq!(
        resolve_status(&flux(), &suspended_but_ready),
        resolved(NormalizedStatus::Suspended, "Suspended", None)
    );
    // Reorder the same rules and the same object answers differently.
    let mut reordered = flux();
    reordered.swap(0, 2);
    assert_eq!(
        resolve_status(&reordered, &suspended_but_ready).status,
        NormalizedStatus::Healthy
    );
}

#[test]
fn every_normalized_status_is_reachable_and_serializes_lowercase() {
    let all = rules(json!([
        {"when": [{"jsonPath": ".s", "equals": "h"}], "status": "healthy", "label": "H"},
        {"when": [{"jsonPath": ".s", "equals": "w"}], "status": "warning", "label": "W"},
        {"when": [{"jsonPath": ".s", "equals": "e"}], "status": "error", "label": "E"},
        {"when": [{"jsonPath": ".s", "equals": "p"}], "status": "progressing", "label": "P"},
        {"when": [{"jsonPath": ".s", "equals": "s"}], "status": "suspended", "label": "S"},
        {"when": [{"jsonPath": ".s", "equals": "u"}], "status": "unknown", "label": "U"}
    ]));
    for (value, status, wire) in [
        ("h", NormalizedStatus::Healthy, "healthy"),
        ("w", NormalizedStatus::Warning, "warning"),
        ("e", NormalizedStatus::Error, "error"),
        ("p", NormalizedStatus::Progressing, "progressing"),
        ("s", NormalizedStatus::Suspended, "suspended"),
        ("u", NormalizedStatus::Unknown, "unknown"),
    ] {
        let got = resolve_status(&all, &json!({"s": value}));
        assert_eq!(got.status, status);
        assert_eq!(serde_json::to_value(got.status).unwrap(), json!(wire));
    }
    assert!(serde_json::from_value::<NormalizedStatus>(json!("degraded")).is_err());
}

#[test]
fn every_condition_of_a_rule_must_hold() {
    let both = rules(json!([
        {"when": [{"jsonPath": ".a", "equals": 1}, {"jsonPath": ".b", "absent": true}],
         "status": "warning", "label": "Both"}
    ]));
    assert!(first_match(&both, &json!({"a": 1})).is_some());
    assert!(first_match(&both, &json!({"a": 1, "b": 2})).is_none());
    assert!(first_match(&both, &json!({"b": null})).is_none());
}

#[test]
fn the_reason_is_read_from_the_object_when_the_rule_names_one() {
    let failed = resource(
        false,
        vec![condition(
            "Ready",
            "False",
            "kustomize build failed: accumulating resources",
        )],
    );
    assert_eq!(
        resolve_status(&flux(), &failed),
        resolved(
            NormalizedStatus::Error,
            "Not ready",
            Some("kustomize build failed: accumulating resources")
        )
    );
    // Numbers and booleans are written as text; a missing value, an empty
    // string, an object or a list is no reason rather than a made-up one.
    let reason = |value: Value| {
        let rule = rules(json!([{"when": [], "status": "warning", "label": "W", "reason": ".r"}]));
        resolve_status(&rule, &json!({ "r": value })).reason
    };
    assert_eq!(reason(json!(3)).as_deref(), Some("3"));
    assert_eq!(reason(json!(false)).as_deref(), Some("false"));
    assert_eq!(reason(json!("  ")), None);
    assert_eq!(reason(json!({"a": 1})), None);
    assert_eq!(reason(json!([1])), None);
    assert_eq!(reason(Value::Null), None);
    // Bounded: a reason is one line of a cell, not the controller's log.
    let long = reason(json!("x".repeat(5_000))).unwrap();
    assert!(long.chars().count() <= 201, "{}", long.len());
    assert!(long.ends_with('…'));
}

#[test]
fn nothing_matching_is_unknown_for_a_status_and_nothing_for_a_badge() {
    let only_ready = rules(json!([
        {"when": [{"jsonPath": ".status.conditions[?(@.type=='Ready')].status", "equals": "True"}],
         "status": "healthy", "label": "Ready"}
    ]));
    let pending = resource(false, vec![]);
    // A resolver always says something: the host's own word when no rule does.
    assert_eq!(
        resolve_status(&only_ready, &pending),
        resolved(NormalizedStatus::Unknown, "Unknown", None)
    );
    // A badge is an assertion; no rule holding means no badge.
    assert_eq!(first_match(&only_ready, &pending), None);
}

#[test]
fn a_rule_set_the_host_would_refuse_matches_nothing() {
    // Fails closed: a typo in a path, a bad operator or a blank word does not
    // become a status. Each is also refused at install; this is the backstop
    // for a caller that did not check.
    for broken in [
        json!([{"when": [{"jsonPath": ".spec.*", "present": true}], "status": "healthy", "label": "Ready"}]),
        json!([{"when": [{"jsonPath": ".spec.suspend"}], "status": "healthy", "label": "Ready"}]),
        json!([{"when": [], "status": "healthy", "label": "  "}]),
        json!([{"when": [], "status": "healthy", "label": "Ready", "reason": "status"}]),
    ] {
        let declared = rules(broken.clone());
        assert!(!rule_problems(&declared).is_empty(), "{broken} is refused");
        assert_eq!(first_match(&declared, &json!({"spec": {"suspend": true}})), None);
        assert_eq!(
            resolve_status(&declared, &json!({})).status,
            NormalizedStatus::Unknown
        );
    }
}

#[test]
fn a_badge_always_carries_a_word() {
    let problems = |value: Value| rule_problems(&rules(value));
    for blank in ["", "   ", "\u{200b}", "\u{202e}Ready"] {
        let found = problems(json!([{"when": [], "status": "healthy", "label": blank}]));
        assert!(
            found.iter().any(|(path, _)| path == "rules[0].label"),
            "{blank:?}: {found:?}"
        );
    }
    let long = problems(json!([{"when": [], "status": "healthy", "label": "x".repeat(41)}]));
    assert!(long.iter().any(|(path, _)| path == "rules[0].label"));
    assert!(problems(json!([{"when": [], "status": "healthy", "label": "Flux"}])).is_empty());
}

#[test]
fn rule_lists_and_their_conditions_are_bounded_and_located() {
    let one = json!({"when": [], "status": "unknown", "label": "U"});
    let found = rule_problems(&rules(json!(vec![one.clone(); MAX_RULES + 1])));
    assert!(found.iter().any(|(path, _)| path == "rules"), "{found:?}");
    assert!(rule_problems(&rules(json!(vec![one; MAX_RULES]))).is_empty());
    assert!(rule_problems(&[])
        .iter()
        .any(|(path, _)| path == "rules"));
    let nine = vec![json!({"jsonPath": ".a", "present": true}); 9];
    let found = rule_problems(&rules(
        json!([{"when": nine, "status": "unknown", "label": "U"}]),
    ));
    assert!(found.iter().any(|(path, _)| path == "rules[0].when"), "{found:?}");
    let found = rule_problems(&rules(json!([
        {"when": [], "status": "unknown", "label": "U"},
        {"when": [{"jsonPath": ".a", "present": true}, {"jsonPath": "a", "present": true}],
         "status": "unknown", "label": "U", "reason": ".ok"}
    ])));
    assert_eq!(
        found.iter().map(|(path, _)| path.as_str()).collect::<Vec<_>>(),
        vec!["rules[1].when[1]"]
    );
}

#[test]
fn labels_and_reasons_leave_the_host_escaped() {
    let rule = rules(json!([{"when": [], "status": "warning", "label": "Drift", "reason": ".r"}]));
    let got = resolve_status(&rule, &json!({"r": "Resume\u{202e} first\u{200b}"}));
    let reason = got.reason.unwrap();
    assert!(!reason.contains('\u{202e}') && !reason.contains('\u{200b}'), "{reason}");
    assert!(reason.contains("\\u{202e}"), "{reason}");
}

#[test]
fn a_list_resolves_per_object_in_order() {
    // The seam #540's countByStatus reads: one answer per object, in order.
    let objects = [
        resource(true, vec![]),
        resource(false, vec![condition("Ready", "True", "ok")]),
        resource(false, vec![condition("Ready", "False", "broken")]),
        json!({}),
    ];
    let statuses: Vec<_> = resolve_statuses(&flux(), &objects)
        .into_iter()
        .map(|resolved| resolved.status)
        .collect();
    assert_eq!(
        statuses,
        vec![
            NormalizedStatus::Suspended,
            NormalizedStatus::Healthy,
            NormalizedStatus::Error,
            NormalizedStatus::Unknown
        ]
    );
}
