//! Resource links to built-in targets and by spec path (#728), API 0.5.
//!
//! A link may point at a built-in kind the host lists, not only at one a declared
//! reader lists, and may name its target through a path on the `from` resource in the
//! predicate grammar (#550) plus `[*]`. Both are validated here, at install.
use serde_json::{json, Value};
use srelens_plugin_host::{builtin_link_kind, Manifest, ValidationCode, ValidationError};

/// A Flux-shaped app on API 0.5 with readers for Kustomizations, GitRepositories and
/// Gateway API HTTPRoutes, and ExternalSecrets.
fn manifest() -> Value {
    let reader = |name: &str, group: &str, kind: &str| {
        json!({"name":name,"title":format!("List {name}"),"target":"k8s.listCustomResource",
            "arguments":{"group":group,"version":"v1","plural":name,"kind":kind,"namespaced":true},
            "inputs":["context","namespace"]})
    };
    json!({
        "id":"org.example.links", "name":"Links", "version":"0.1.0", "srelensApiVersion":"^0.5",
        "kind":"declarative", "permissions":["k8s.listCustomResource"],
        "capabilities":[
            reader("kustomizations","kustomize.toolkit.fluxcd.io","Kustomization"),
            reader("gitrepositories","source.toolkit.fluxcd.io","GitRepository"),
            reader("httproutes","gateway.networking.k8s.io","HTTPRoute"),
            reader("externalsecrets","external-secrets.io","ExternalSecret"),
        ],
        "contributions":{"pages":[{"id":"kustomizations","title":"Kustomizations","capability":"kustomizations"}],
            "detailTabs":[],"detailLinks":[],
            "resourceLinks":[{"id":"source","from":"kustomize.toolkit.fluxcd.io/Kustomization",
                "to":"source.toolkit.fluxcd.io/GitRepository","relation":"references",
                "match":{"path":".spec.sourceRef"}}]}
    })
}

fn with_link(link: Value) -> Value {
    let mut value = manifest();
    value["contributions"]["resourceLinks"] = json!([link]);
    value
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

/// The one problem `value` has, as (code, path).
fn only_problem(value: &Value) -> (String, String) {
    let errors = errors(value);
    assert_eq!(errors.len(), 1, "{errors:?}");
    (code(errors[0].code), errors[0].path.clone())
}

#[test]
fn a_spec_path_names_the_target_and_round_trips() {
    let value = manifest();
    let parsed = Manifest::parse(&value.to_string()).expect("a path link is valid");
    let link = &parsed.contributions.resource_links[0];
    assert_eq!(link.match_by.path.as_deref(), Some(".spec.sourceRef"));
    let serialized = serde_json::to_value(&parsed).unwrap();
    assert_eq!(
        serialized["contributions"]["resourceLinks"],
        value["contributions"]["resourceLinks"]
    );
    // A Kustomization depends on others of its own kind: a path may link a kind to
    // itself, because the path is not the resource's own name.
    Manifest::parse(
        &with_link(
            json!({"id":"depends","from":"kustomize.toolkit.fluxcd.io/Kustomization",
            "to":"kustomize.toolkit.fluxcd.io/Kustomization","relation":"references",
            "match":{"path":".spec.dependsOn[*]"}}),
        )
        .to_string(),
    )
    .expect("a kind may reference its own kind by path");
}

#[test]
fn a_link_may_point_at_a_built_in_kind_the_host_lists() {
    // HTTPRoute → Service, through every backend of every rule.
    Manifest::parse(
        &with_link(
            json!({"id":"backends","from":"gateway.networking.k8s.io/HTTPRoute",
            "to":"/Service","relation":"references",
            "match":{"path":".spec.rules[*].backendRefs[*]"}}),
        )
        .to_string(),
    )
    .expect("a Service is a built-in target");
    // ExternalSecret → the Secret it writes. Only the Secret's identity is ever read.
    Manifest::parse(
        &with_link(
            json!({"id":"target","from":"external-secrets.io/ExternalSecret",
            "to":"/Secret","relation":"references","match":{"path":".spec.target.name"}}),
        )
        .to_string(),
    )
    .expect("a Secret is a built-in target");
    // The metadata selectors reach built-in targets too.
    Manifest::parse(
        &with_link(
            json!({"id":"namespace","from":"kustomize.toolkit.fluxcd.io/Kustomization",
            "to":"/Namespace","relation":"references","match":{"label":"example.io/namespace"}}),
        )
        .to_string(),
    )
    .expect("a Namespace is a built-in target");
    for kind in [
        "/Service",
        "/Secret",
        "apps/Deployment",
        "networking.k8s.io/Ingress",
        "/Node",
    ] {
        assert!(builtin_link_kind(kind).is_some(), "{kind}");
    }
    // A kind name in the wrong group is not the built-in one; an Event is a record
    // about a resource, not a resource another one points at.
    for kind in [
        "acme.io/Service",
        "/Deployment",
        "apps/Service",
        "/Event",
        "/service",
    ] {
        assert!(builtin_link_kind(kind).is_none(), "{kind}");
    }
}

#[test]
fn a_target_neither_a_reader_nor_the_host_lists_is_refused_at_to() {
    for to in ["acme.io/Widget", "/Event", "apps/Service"] {
        let value = with_link(
            json!({"id":"x","from":"gateway.networking.k8s.io/HTTPRoute",
            "to":to,"relation":"references","match":{"path":".spec.parentRefs[*]"}}),
        );
        assert_eq!(
            only_problem(&value),
            (
                "EXTENSION_UNRESOLVED_CAPABILITY".into(),
                "contributions.resourceLinks[0].to".into()
            ),
            "{to}"
        );
        let message = errors(&value)[0].message.clone();
        assert!(message.contains("built-in"), "{message}");
    }
}

#[test]
fn a_path_is_checked_at_install_in_the_link_grammar() {
    for bad in [
        "spec.sourceRef",
        ".spec..sourceRef",
        ".spec.rules[]",
        ".spec.rules[?(@.kind!=\"A\")]",
        "..name",
        "",
    ] {
        let value = with_link(
            json!({"id":"source","from":"kustomize.toolkit.fluxcd.io/Kustomization",
            "to":"source.toolkit.fluxcd.io/GitRepository","relation":"references","match":{"path":bad}}),
        );
        assert_eq!(
            only_problem(&value),
            (
                "EXTENSION_INVALID_BINDING".into(),
                "contributions.resourceLinks[0].match.path".into()
            ),
            "{bad}"
        );
    }
}

#[test]
fn a_path_reads_a_from_kind_a_declared_reader_lists_and_never_a_secret() {
    // The host reads the `from` resource's body through the app's own reader — for the
    // forward link and for the target's reverse view — so a kind no reader lists has no
    // body the app may read.
    let value = with_link(
        json!({"id":"sa","from":"apps/Deployment","to":"/ServiceAccount",
        "relation":"references","match":{"path":".spec.template.spec.serviceAccountName"}}),
    );
    assert_eq!(
        only_problem(&value),
        (
            "EXTENSION_UNRESOLVED_CAPABILITY".into(),
            "contributions.resourceLinks[0].match.path".into()
        )
    );
    // A Secret's body is its values: never read for an app, whatever reader is declared.
    let mut value = with_link(json!({"id":"s","from":"/Secret","to":"/ServiceAccount",
        "relation":"references","match":{"path":".type"}}));
    value["capabilities"].as_array_mut().unwrap().push(json!({"name":"secrets",
        "title":"List secrets","target":"k8s.listCustomResource",
        "arguments":{"group":"","version":"v1","plural":"secrets","kind":"Secret","namespaced":true},
        "inputs":["context","namespace"]}));
    let problems = errors(&value);
    assert!(
        problems
            .iter()
            .any(|p| p.path == "contributions.resourceLinks[0].match.path"
                && p.code == ValidationCode::InvalidBinding
                && p.message.contains("Secret")),
        "{problems:?}"
    );
}

#[test]
fn a_path_is_one_selector_among_the_others() {
    for (matching, at) in [
        (json!({"path":".spec.sourceRef","name":true}), ".match"),
        (
            json!({"path":".spec.sourceRef","label":"example.io/x"}),
            ".match",
        ),
        (
            json!({"path":".spec.sourceRef","namespaceLabel":"example.io/ns"}),
            ".match.namespaceLabel",
        ),
    ] {
        let value = with_link(
            json!({"id":"source","from":"kustomize.toolkit.fluxcd.io/Kustomization",
            "to":"source.toolkit.fluxcd.io/GitRepository","relation":"references","match":matching}),
        );
        assert_eq!(
            only_problem(&value),
            (
                "EXTENSION_INVALID_BINDING".into(),
                format!("contributions.resourceLinks[0]{at}")
            ),
            "{matching}"
        );
    }
    // `parse` qualifies an annotation, not a path.
    let value = with_link(
        json!({"id":"source","from":"kustomize.toolkit.fluxcd.io/Kustomization",
        "to":"source.toolkit.fluxcd.io/GitRepository","relation":"references",
        "match":{"path":".spec.sourceRef","parse":"argocd-tracking-id"}}),
    );
    assert!(errors(&value)
        .iter()
        .any(|p| p.path == "contributions.resourceLinks[0].match.parse"
            && p.message == "parse requires annotation"));
}

#[test]
fn a_link_path_is_read_through_the_from_readers_version_overrides() {
    // A reader of several served versions (#547) may move the field a link reads: the
    // path is one of the reader's paths, so an override rewrites it for that version.
    let mut value = manifest();
    let reader = &mut value["capabilities"][0];
    reader["arguments"]
        .as_object_mut()
        .unwrap()
        .remove("version");
    reader["versions"] = json!(["v1", "v1beta2"]);
    reader["jsonPathOverrides"] = json!({"v1beta2": {".spec.sourceRef": ".spec.source"}});
    let parsed = Manifest::parse(&value.to_string()).expect("the override names a link path");
    let old = parsed.at_version("kustomizations", "v1beta2").unwrap();
    assert_eq!(
        old.contributions.resource_links[0].match_by.path.as_deref(),
        Some(".spec.source")
    );
    let current = parsed.at_version("kustomizations", "v1").unwrap();
    assert_eq!(
        current.contributions.resource_links[0]
            .match_by
            .path
            .as_deref(),
        Some(".spec.sourceRef")
    );
    // An override that breaks the link grammar is refused where it is written.
    let mut broken = value.clone();
    broken["capabilities"][0]["jsonPathOverrides"] =
        json!({"v1beta2": {".spec.sourceRef": "spec.source"}});
    assert!(errors(&broken)
        .iter()
        .any(|p| p.path.starts_with("capabilities[0].jsonPathOverrides")));
}
