//! Declared preconditions and availability (#550): what a manifest may say
//! about the object an action writes, and where the host stops saying it.
//!
//! Against the real host registry, like `actions.rs`: what is being checked is
//! that an app cannot widen what the real primitives accept.
use serde_json::{json, Value};
use srelens_capability::Registry;
use srelens_plugin_host::{Manifest, PluginHost, ValidationCode};
use std::sync::Arc;

/// A Flux app whose reconcile action is declared with both predicate lists.
fn manifest() -> Value {
    json!({
        "id":"org.example.flux", "name":"Flux", "version":"0.1.0", "srelensApiVersion":"^0.3",
        "kind":"declarative",
        "permissions":["k8s.listCustomResource","k8s.annotate"],
        "capabilities":[{
            "name":"helmreleases", "title":"List HelmReleases", "target":"k8s.listCustomResource",
            "arguments":{"group":"helm.toolkit.fluxcd.io","version":"v2","plural":"helmreleases",
                "kind":"HelmRelease","namespaced":true},
            "inputs":["context","namespace"]
        }],
        "actions":[{
            "name":"reconcile", "title":"Reconcile", "target":"k8s.annotate", "resource":"helmreleases",
            "arguments":{"key":"reconcile.fluxcd.io/requestedAt","value":"$now"},
            "preconditions":[{"jsonPath":".spec.suspend","notEquals":true,
                "reason":"Resume this resource before requesting reconciliation"}],
            "availableWhen":[{"jsonPath":".spec.suspend","notEquals":true,
                "reason":"Resume this resource before requesting reconciliation"}]
        }],
        "contributions":{"pages":[{"id":"helmreleases","title":"HelmReleases","capability":"helmreleases"}],
            "detailTabs":[],"detailLinks":[]}
    })
}

/// `value` with `mutate` applied to its one declared action.
fn action(mutate: impl FnOnce(&mut Value)) -> Value {
    let mut value = manifest();
    mutate(&mut value["actions"][0]);
    value
}

fn parse(value: &Value) -> Result<Manifest, String> {
    Manifest::parse(&value.to_string()).map_err(|errors| errors.to_string())
}

/// Installs `value` against the real host registry.
fn install(value: &Value) -> Result<Registry, String> {
    let manifest = parse(value)?;
    let grants = manifest.permission_names();
    let mut reg = Registry::new();
    PluginHost::new(Arc::new(srelens_registry::build_registry()))
        .register(&mut reg, manifest, &grants)?;
    Ok(reg)
}

/// Every problem `value` is refused for, as `code path: message`.
fn refusals(value: &Value) -> Vec<String> {
    let errors = Manifest::parse(&value.to_string())
        .err()
        .unwrap_or_else(|| panic!("expected a refusal for {value}"));
    errors
        .0
        .iter()
        .map(|e| format!("{:?} {}: {}", e.code, e.path, e.message))
        .collect()
}

/// A refusal reported at `path`, or a panic naming what was reported instead.
fn refused_at(value: &Value, path: &str) -> String {
    let found = refusals(value);
    found
        .iter()
        .find(|problem| problem.contains(&format!(" {path}: ")))
        .unwrap_or_else(|| panic!("nothing was reported at {path}; got {found:?}"))
        .clone()
}

#[test]
fn an_action_carries_its_preconditions_into_the_binding_the_host_builds() {
    let parsed = parse(&manifest()).expect("installs");
    let binding = parsed
        .action_binding(&parsed.actions[0])
        .expect("the reader scopes the action");
    assert_eq!(
        binding.arguments["preconditions"],
        json!([{"jsonPath":".spec.suspend","notEquals":true,
            "reason":"Resume this resource before requesting reconciliation"}]),
        "the primitive is handed the checks it must make, as it is handed the kind"
    );
    // `availableWhen` decides whether a control is offered; it is not a thing
    // the cluster request needs, so it is not sent to one.
    assert!(!binding.arguments.contains_key("availableWhen"));
    install(&manifest()).expect("registers");
}

#[test]
fn a_path_the_host_cannot_evaluate_is_refused_at_install() {
    // A filter other than the one `key == "string"` form (#541) addresses a
    // set, and is refused.
    let bad = action(|a| {
        a["preconditions"][0]["jsonPath"] = json!(".status.conditions[?(@.type!='Ready')].status");
    });
    let why = refused_at(&bad, "actions[0].preconditions[0]");
    assert!(
        why.contains("InvalidBinding") && why.contains("not a resource path"),
        "{why}"
    );
    // The one filter form names the first matching element and is accepted — from
    // API 0.4, which added it (#709). Under a 0.3 range it is told the version it needs.
    let mut ready = action(|a| {
        a["preconditions"][0]["jsonPath"] = json!(".status.conditions[?(@.type=='Ready')].status");
    });
    let why = refused_at(&ready, "srelensApiVersion");
    assert!(why.contains("requires API 0.4.0"), "{why}");
    ready["srelensApiVersion"] = json!("^0.4");
    parse(&ready).expect("the narrow filter is a path the host evaluates");
}

#[test]
fn an_unknown_operator_is_refused_at_install() {
    // A field the predicate has no operator for is not silently ignored.
    let unknown = action(|a| {
        a["preconditions"][0] = json!({"jsonPath":".spec.suspend","isFalse":true,"reason":"r"})
    });
    assert!(
        refusals(&unknown).iter().any(|p| p.contains("actions[0]")),
        "{:?}",
        refusals(&unknown)
    );
    // And a predicate that declares none of them is not a check at all.
    let none = action(|a| a["preconditions"][0] = json!({"jsonPath":".spec.suspend","reason":"r"}));
    let why = refused_at(&none, "actions[0].preconditions[0]");
    assert!(why.contains("exactly one"), "{why}");
}

#[test]
fn a_comparand_that_is_not_a_literal_is_refused_at_install() {
    let shaped = action(|a| a["preconditions"][0]["notEquals"] = json!({"suspend": true}));
    let why = refused_at(&shaped, "actions[0].preconditions[0]");
    assert!(why.contains("literal"), "{why}");
}

#[test]
fn available_when_is_held_to_the_same_rules_as_a_precondition() {
    let bad = action(|a| a["availableWhen"][0]["jsonPath"] = json!("spec.suspend"));
    let why = refused_at(&bad, "actions[0].availableWhen[0]");
    assert!(why.contains("not a resource path"), "{why}");
}

#[test]
fn a_predicate_carries_a_reason_for_the_operator() {
    let blank = action(|a| a["availableWhen"][0]["reason"] = json!(""));
    let why = refused_at(&blank, "actions[0].availableWhen[0]");
    assert!(why.contains("reason"), "{why}");
}

#[test]
fn an_action_cannot_bind_its_predicates_as_arguments_instead() {
    // One place to declare them, so the host reads one. A second spelling
    // inside `arguments` would be a list nothing validated as a list.
    for key in ["preconditions", "availableWhen"] {
        let smuggled = action(|a| {
            a["arguments"][key] = json!([{"jsonPath":".spec.suspend","absent":true,"reason":"r"}]);
        });
        let why = refused_at(&smuggled, &format!("actions[0].arguments.{key}"));
        assert!(
            why.contains(&format!("{:?}", ValidationCode::InvalidBinding)),
            "{why}"
        );
    }
}

#[test]
fn a_reader_binding_declares_no_predicates() {
    // Only a mutation has preconditions; a reader with them would read as
    // though the host filtered its results, which it does not.
    let mut value = manifest();
    value["capabilities"][0]["preconditions"] =
        json!([{"jsonPath":".spec.suspend","absent":true,"reason":"r"}]);
    assert!(
        Manifest::parse(&value.to_string()).is_err(),
        "a reader binding takes no `preconditions`"
    );
}
