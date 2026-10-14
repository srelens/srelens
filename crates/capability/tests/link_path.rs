//! A resource link's `path` (#728): the predicate grammar (#550), plus `[*]`
//! to read every element of a list, since a link names a set of targets.

use serde_json::{json, Value};
use srelens_capability::{check_link_path, check_path, resolve_each, MAX_LINK_VALUES};

/// An HTTPRoute with two rules, one backend each, and one with two.
fn route() -> Value {
    json!({
        "apiVersion": "gateway.networking.k8s.io/v1",
        "kind": "HTTPRoute",
        "metadata": {"name": "web", "namespace": "team"},
        "spec": {"rules": [
            {"backendRefs": [{"name": "api", "port": 80}]},
            {"backendRefs": [{"name": "web", "port": 80}, {"name": "old", "namespace": "legacy", "port": 80}]},
            {"filters": []}
        ]}
    })
}

#[test]
fn each_reads_every_element_of_a_list() {
    let path = ".spec.rules[*].backendRefs[*]";
    assert_eq!(check_link_path(path), Ok(()));
    let route = route();
    let names: Vec<&str> = resolve_each(&route, path)
        .unwrap()
        .into_iter()
        .map(|backend| backend["name"].as_str().unwrap())
        .collect();
    // In document order; the rule with no backendRefs contributes nothing.
    assert_eq!(names, ["api", "web", "old"]);
}

#[test]
fn a_path_without_each_reads_the_one_value_a_predicate_would() {
    let kustomization = json!({"spec": {"sourceRef": {"kind": "GitRepository", "name": "repo"}}});
    assert_eq!(
        resolve_each(&kustomization, ".spec.sourceRef").unwrap(),
        vec![&kustomization["spec"]["sourceRef"]]
    );
    // Index, quoted key and the first-match filter are the predicate grammar's.
    assert_eq!(
        resolve_each(&route(), ".spec.rules[1]['backendRefs'][0].name").unwrap(),
        vec![&json!("web")]
    );
    assert_eq!(
        resolve_each(
            &json!({"refs": [{"kind": "A", "name": "a"}, {"kind": "B", "name": "b"}]}),
            ".refs[?(@.kind==\"B\")].name"
        )
        .unwrap(),
        vec![&json!("b")]
    );
}

#[test]
fn an_unset_field_is_no_value_not_an_error() {
    for object in [
        json!({}),
        json!({"spec": null}),
        json!({"spec": {"rules": null}}),
    ] {
        assert_eq!(
            resolve_each(&object, ".spec.rules[*].backendRefs[*]"),
            Ok(vec![])
        );
    }
    // `[*]` over something that is not a list reaches nothing, as `.a.b`
    // over a string does.
    assert_eq!(
        resolve_each(&json!({"spec": {"rules": "x"}}), ".spec.rules[*]"),
        Ok(vec![])
    );
    // A null element is an unset one.
    assert_eq!(
        resolve_each(&json!({"refs": [null, "a"]}), ".refs[*]"),
        Ok(vec![&json!("a")])
    );
}

#[test]
fn each_is_a_link_path_form_and_never_a_predicates() {
    // A predicate asks one question about one value; `[*]` addresses a set.
    assert!(check_path(".spec.rules[*].backendRefs").is_err());
    assert!(check_link_path(".spec.rules[*].backendRefs").is_ok());
    // Everything else the predicate grammar refuses, a link path refuses.
    for bad in [
        "spec.sourceRef",
        ".spec..sourceRef",
        ".spec.rules[]",
        ".spec.rules[-1]",
        ".spec.rules[?(@.kind!=\"A\")]",
        "..name",
        ".spec.rules[*",
        ".spec.rules[* ]",
    ] {
        assert!(check_link_path(bad).is_err(), "{bad}");
    }
}

#[test]
fn the_values_one_path_reaches_are_bounded() {
    let many: Vec<Value> = (0..MAX_LINK_VALUES + 1)
        .map(|i| json!({"name": format!("s-{i}")}))
        .collect();
    let object = json!({"refs": many});
    assert_eq!(
        resolve_each(&object, ".refs[*]").map(|v| v.len()).ok(),
        None
    );
    let error = resolve_each(&object, ".refs[*]").unwrap_err();
    assert!(error.contains(&MAX_LINK_VALUES.to_string()), "{error}");
    let within = json!({"refs": &object["refs"].as_array().unwrap()[..MAX_LINK_VALUES]});
    assert_eq!(
        resolve_each(&within, ".refs[*]").unwrap().len(),
        MAX_LINK_VALUES
    );
    // Nested lists are bounded by what they visit, not only by what they
    // yield: wide lists of empty lists yield nothing but cost a step each.
    let wide: Vec<Value> = (0..MAX_LINK_VALUES + 1)
        .map(|_| json!({"refs": []}))
        .collect();
    assert!(resolve_each(&json!({"rules": wide}), ".rules[*].refs[*]").is_err());
}
