//! What a declared predicate (#550) means, one operator at a time, and what
//! the host refuses to accept as one.

use serde_json::{json, Value};
use srelens_capability::{
    check_path, check_predicates, resolve, unmet, Condition, Predicate, MAX_PREDICATES,
};

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
        "spec.suspend",                                   // no leading `.`
        ".spec..suspend",                                 // empty segment
        ".spec.*",                                        // wildcard
        ".status.conditions[?(@.type!='Ready')].status",  // a filter other than key == string
        ".status.conditions[?(@.type=='Ready')]x",        // only a segment follows a filter
        ".status.conditions[?(@.type==Ready)].status",    // an unquoted comparand
        ".status.conditions[?(@.a.b=='x')].status",       // a nested key inside the filter
        ".status.conditions[?(@.type=='')].status",       // an empty comparand
        ".status.conditions[?(@.type=='Re'ady')].status", // a quote inside the comparand
        ".status.conditions[?(@.type==\"Ready')].status", // mismatched quotes
        "..suspend",                                      // recursive descent
        ".metadata.annotations[acme.io/x]",               // unquoted odd key
        ".spec.a.b.c.d.e.f.g.h.i",                        // deeper than the bound
        ".spec[01]",                                      // a second spelling of `[1]`
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

/// A Flux resource whose Ready condition is not the first one, which is why an
/// index cannot stand in for the filter.
fn reconciling() -> Value {
    json!({
        "spec": {},
        "status": {"conditions": [
            {"type": "Reconciling", "status": "True", "message": "Applying revision"},
            {"type": "Ready", "status": "Unknown", "message": "Reconciliation in progress"},
            {"type": "Ready", "status": "False", "message": "a duplicate the API server would not keep"}
        ]}
    })
}

#[test]
fn a_filter_selects_the_first_element_whose_key_is_that_string() {
    // The one filter form: `[?(@.key=="literal")]`, in either quote. It names
    // ONE element — the first match, as a Kubernetes printer column reads the
    // same path — so "does this hold" stays a question about one value rather
    // than a quantifier over a set.
    for path in [
        ".status.conditions[?(@.type==\"Ready\")].status",
        ".status.conditions[?(@.type=='Ready')].status",
    ] {
        check_path(path).expect("the narrow filter is a path this host evaluates");
        assert_eq!(
            resolve(&reconciling(), path),
            Some(&json!("Unknown")),
            "{path}"
        );
    }
    assert_eq!(
        resolve(
            &reconciling(),
            ".status.conditions[?(@.type=='Reconciling')].message"
        ),
        Some(&json!("Applying revision"))
    );
    let ready = predicate(json!({
        "jsonPath": ".status.conditions[?(@.type==\"Ready\")].status",
        "equals": "False",
        "reason": "r"
    }));
    assert!(ready.check().is_ok());
    assert!(
        ready.holds(&object()),
        "object()'s only Ready condition is False"
    );
    assert!(
        !ready.holds(&reconciling()),
        "the first Ready condition is the one read, not any of them"
    );
}

#[test]
fn a_filter_that_matches_nothing_is_an_unset_field() {
    let path = ".status.conditions[?(@.type=='Stalled')].status";
    assert_eq!(resolve(&reconciling(), path), None);
    // Not an array, or elements that are not objects: nothing is selected.
    assert_eq!(
        resolve(
            &json!({"status": {"conditions": {"type": "Stalled"}}}),
            path
        ),
        None
    );
    assert_eq!(
        resolve(&json!({"status": {"conditions": ["Stalled"]}}), path),
        None
    );
    // The comparand is a string: a key holding another type is not a match
    // for its spelling.
    assert_eq!(
        resolve(
            &json!({"items": [{"n": 1, "v": "one"}]}),
            ".items[?(@.n=='1')].v"
        ),
        None
    );
    let stalled = |operator: Value| {
        let mut declared = json!({"jsonPath": path, "reason": "r"});
        declared
            .as_object_mut()
            .unwrap()
            .extend(operator.as_object().unwrap().clone());
        predicate(declared)
    };
    assert!(stalled(json!({"absent": true})).holds(&reconciling()));
    assert!(!stalled(json!({"present": true})).holds(&reconciling()));
    assert!(stalled(json!({"notEquals": "True"})).holds(&reconciling()));
}

#[test]
fn a_condition_is_a_predicate_without_a_reason() {
    // Status rules ask "which of these describes the object", not "why was
    // this refused", so their conditions carry no sentence — but they are the
    // same operators over the same paths, evaluated by the same code.
    let suspended: Condition =
        serde_json::from_value(json!({"jsonPath": ".spec.suspend", "equals": true})).unwrap();
    suspended.check().expect("well formed");
    assert!(suspended.holds(&object()));
    assert!(!suspended.holds(&json!({"spec": {"suspend": false}})));
    for broken in [
        json!({"jsonPath": ".spec.suspend"}),
        json!({"jsonPath": ".spec.suspend", "equals": true, "present": true}),
        json!({"jsonPath": ".spec.*", "present": true}),
        json!({"jsonPath": ".spec", "equals": {"suspend": true}}),
        json!({"jsonPath": ".spec.suspend", "present": false}),
    ] {
        let condition: Condition = serde_json::from_value(broken.clone()).unwrap();
        assert!(condition.check().is_err(), "{broken} is refused");
        assert!(!condition.holds(&object()), "{broken} fails closed");
    }
    // A reason is not part of a condition: unknown fields are refused.
    assert!(serde_json::from_value::<Condition>(
        json!({"jsonPath": ".spec.suspend", "equals": true, "reason": "r"})
    )
    .is_err());
}

/// A Deployment as the Inspector holds it, carrying Argo CD's tracking id.
fn tracked(tracking: &str) -> Value {
    json!({"apiVersion": "apps/v1", "kind": "Deployment",
        "metadata": {"name": "api", "namespace": "team",
            "annotations": {"argocd.argoproj.io/tracking-id": tracking}}})
}

#[test]
fn a_tracking_id_names_its_owner_only_when_it_names_the_object_it_is_on() {
    use srelens_capability::ReferenceFormat;
    let format = ReferenceFormat::ArgocdTrackingId;
    let object = tracked("guestbook:apps/Deployment:team/api");
    // The application part, split the way Argo CD writes an application
    // outside its controller namespace: `<namespace>_<name>`.
    assert_eq!(
        format.owner("guestbook:apps/Deployment:team/api", &object),
        Some((None, "guestbook".to_owned()))
    );
    assert_eq!(
        format.owner("apps_guestbook:apps/Deployment:team/api", &object),
        Some((Some("apps".to_owned()), "guestbook".to_owned()))
    );
    // A tracking id copied from another workload names that one, not this.
    assert_eq!(
        format.owner("guestbook:apps/Deployment:team/web", &object),
        None
    );
    assert_eq!(
        format.owner("guestbook:apps/StatefulSet:team/api", &object),
        None
    );
    // Malformed ids, and an empty application, name no owner at all.
    for broken in [
        "guestbook",
        ":apps/Deployment:team/api",
        "_x:apps/Deployment:team/api",
        "x_:apps/Deployment:team/api",
        "guestbook:Deployment:team/api",
    ] {
        assert_eq!(format.owner(broken, &object), None, "{broken}");
    }
}
