//! Declared actions (#549): an app names a host primitive and the kind of a
//! reader binding it already holds, and the host builds the request.
//!
//! These run against the real host registry rather than a fixture, because
//! what is being checked is that an app cannot widen what the real primitives
//! accept.
use serde_json::{json, Value};
use srelens_capability::{CapabilityError, Impact, Registry};
use srelens_plugin_host::{Manifest, PluginHost, ValidationCode, ValidationError};
use std::sync::Arc;

fn manifest() -> Value {
    json!({
        "id":"org.example.gitops", "name":"GitOps", "version":"0.1.0", "srelensApiVersion":"^0.3",
        "kind":"declarative",
        "permissions":["k8s.listCustomResource","k8s.annotate"],
        "capabilities":[{
            "name":"applications", "title":"List applications", "target":"k8s.listCustomResource",
            "arguments":{"group":"argoproj.io","version":"v1alpha1","plural":"applications",
                "kind":"Application","namespaced":true},
            "inputs":["context","namespace"]
        }],
        "actions":[{
            "name":"refresh", "title":"Refresh", "target":"k8s.annotate", "resource":"applications",
            "arguments":{"key":"argocd.argoproj.io/refresh","value":"normal"}
        }],
        "contributions":{"pages":[{"id":"applications","title":"Applications","capability":"applications"}],
            "detailTabs":[],"detailLinks":[]}
    })
}

fn parse(value: &Value) -> Result<Manifest, Vec<ValidationError>> {
    Manifest::parse(&value.to_string()).map_err(|errors| errors.0)
}

fn problems(errors: &[ValidationError]) -> Vec<(ValidationCode, String)> {
    let mut problems: Vec<_> = errors
        .iter()
        .map(|error| (error.code, error.path.clone()))
        .collect();
    problems.sort();
    problems
}

/// Installs `value` against the real host registry, returning the registry it
/// registered into or every problem found.
fn install(value: &Value) -> Result<Registry, String> {
    let core = Arc::new(srelens_registry::build_registry());
    let host = PluginHost::new(core);
    let manifest = parse(value).map_err(|errors| {
        errors
            .iter()
            .map(ValidationError::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    })?;
    let grants = manifest.permission_names();
    let mut reg = Registry::new();
    host.register(&mut reg, manifest, &grants)?;
    Ok(reg)
}

/// Why installing `value` was refused. `Registry` carries handlers and so has
/// no `Debug`, which is why this is not `expect_err`.
fn refused(value: &Value, what: &str) -> String {
    install(value)
        .err()
        .unwrap_or_else(|| panic!("expected a refusal: {what}"))
}

#[tokio::test]
async fn an_action_is_registered_with_the_identity_of_the_reader_binding_it_names() {
    let reg = install(&manifest()).expect("installs");
    let cap = reg
        .get("plugin/org.example.gitops/refresh")
        .expect("the action is registered");
    assert_eq!(cap.summary, "GitOps: Refresh");

    // The host fixes the kind and the template; the surface supplies only the
    // object that was reviewed.
    let mut inputs: Vec<&str> = cap.input_schema["properties"]
        .as_object()
        .expect("an object input")
        .keys()
        .map(String::as_str)
        .collect();
    inputs.sort_unstable();
    assert_eq!(
        inputs,
        ["context", "name", "namespace", "resourceVersion", "uid"]
    );

    // Called with those five, the request reaches the cluster client — which
    // is only possible because the host bound the kind. A missing `group` or
    // `key` would have been refused as invalid input instead.
    let reviewed = json!({"context":"not-a-context","namespace":"argo","name":"api",
        "uid":"u","resourceVersion":"2"});
    match (cap.handler)(reviewed.clone()).await {
        Err(CapabilityError::Handler(_)) => {}
        other => panic!("expected the request to reach the client: {other:?}"),
    }

    // And the app cannot reach past what it declared once installed.
    let mut overridden = reviewed;
    overridden["key"] = json!("kubectl.kubernetes.io/restartedAt");
    match (cap.handler)(overridden).await {
        Err(CapabilityError::InvalidInput(why)) => {
            assert!(why.contains("cannot be overridden"), "{why}")
        }
        other => panic!("expected the override to be refused: {other:?}"),
    }
}

#[test]
fn an_action_carries_the_hosts_gate_level_and_words_for_the_primitive_it_binds() {
    let reg = install(&manifest()).expect("installs");
    let cap = reg
        .get("plugin/org.example.gitops/refresh")
        .expect("action");
    let core = srelens_registry::build_registry();
    let primitive = core.get("k8s.annotate").expect("registered").annotations;
    assert!(!cap.annotations.read_only && cap.annotations.requires_confirm);
    assert_eq!(cap.annotations.impact, primitive.impact);
    assert_eq!(cap.annotations.impact, Impact::Medium);
    assert_eq!(cap.annotations.confirm, primitive.confirm);
}

#[test]
fn an_action_can_only_target_a_kind_the_manifest_holds_a_reader_binding_for() {
    let mut value = manifest();
    value["actions"][0]["resource"] = json!("nodes");
    assert_eq!(
        problems(&parse(&value).unwrap_err()),
        vec![(
            ValidationCode::UnresolvedCapability,
            "actions[0].resource".to_owned()
        )]
    );

    // A reader that does not fix the kind cannot scope an action either: the
    // caller would choose it.
    let mut value = manifest();
    value["capabilities"][0]["arguments"]
        .as_object_mut()
        .unwrap()
        .remove("kind");
    value["capabilities"][0]["inputs"] = json!(["context", "namespace", "kind"]);
    let unfixed = refused(&value, "an unfixed kind");
    assert!(unfixed.contains("kind"), "{unfixed}");
}

#[test]
fn an_action_cannot_bind_the_identity_the_host_fills_in() {
    for identity in ["group", "version", "plural", "kind", "namespaced", "uid"] {
        let mut value = manifest();
        value["actions"][0]["arguments"][identity] = json!("anything");
        assert_eq!(
            problems(&parse(&value).unwrap_err()),
            vec![(
                ValidationCode::InvalidBinding,
                format!("actions[0].arguments.{identity}")
            )],
            "{identity}"
        );
    }
}

#[test]
fn an_action_template_the_primitive_refuses_is_refused_at_install() {
    let mut value = manifest();
    value["actions"][0]["arguments"]["value"] = json!("$requestedBy");
    let unknown_token = refused(&value, "an unknown token");
    assert!(unknown_token.contains("$now"), "{unknown_token}");

    // The deny-list is the host's, and an app cannot get past it by declaring
    // an action instead of calling the primitive.
    let mut value = manifest();
    value["permissions"] = json!(["k8s.listCustomResource", "k8s.mergePatch"]);
    value["actions"][0] = json!({
        "name":"adopt", "title":"Adopt", "target":"k8s.mergePatch", "resource":"applications",
        "arguments":{"patch":{"metadata":{"ownerReferences":[]}}}
    });
    let denied = refused(&value, "a deny-listed path");
    assert!(denied.contains("cannot be written"), "{denied}");
}

#[test]
fn an_actions_target_is_a_permission_the_app_declares_and_the_user_grants() {
    let mut value = manifest();
    value["permissions"] = json!(["k8s.listCustomResource"]);
    assert_eq!(
        problems(&parse(&value).unwrap_err()),
        vec![(ValidationCode::PermissionMismatch, "permissions".to_owned())]
    );

    let core = Arc::new(srelens_registry::build_registry());
    let host = PluginHost::new(core);
    let manifest = parse(&manifest()).expect("parses");
    let mut reg = Registry::new();
    assert!(
        host.register(&mut reg, manifest, &["k8s.listCustomResource".into()])
            .is_err(),
        "an ungranted action target must not register"
    );
    assert!(reg.ids().is_empty(), "and nothing is left behind");
}

#[test]
fn an_action_shares_the_name_space_of_the_capabilities_beside_it() {
    let mut value = manifest();
    value["actions"][0]["name"] = json!("applications");
    assert_eq!(
        problems(&parse(&value).unwrap_err()),
        vec![(
            ValidationCode::DuplicateIdentifier,
            "actions[0].name".to_owned()
        )]
    );

    let mut value = manifest();
    value["actions"][0]["title"] = json!("Refresh\u{202E}");
    assert_eq!(
        problems(&parse(&value).unwrap_err()),
        vec![(ValidationCode::InvalidValue, "actions[0].title".to_owned())]
    );
}

/// A manifest that declares no actions is the ordinary case and must stay
/// exactly as valid as it was before actions existed.
#[test]
fn a_manifest_with_no_actions_is_unchanged() {
    let mut value = manifest();
    value.as_object_mut().unwrap().remove("actions");
    value["permissions"] = json!(["k8s.listCustomResource"]);
    let manifest = parse(&value).expect("parses");
    assert!(manifest.actions.is_empty());
    // And it serializes without an empty `actions`, so a stored manifest that
    // was signed before this field existed still round-trips to its own bytes.
    let stored = serde_json::to_value(&manifest).expect("serializes");
    assert!(stored.get("actions").is_none(), "{stored}");
}

#[tokio::test]
async fn builtin_bindings_keep_host_identity_safety_and_revocation() {
    for (reader, target, kind, arguments, impact) in [
        (
            "k8s.listDeployments",
            "k8s.requestRolloutRestart",
            "Deployment",
            json!({}),
            Impact::High,
        ),
        (
            "k8s.listStatefulSets",
            "k8s.requestRolloutRestart",
            "StatefulSet",
            json!({}),
            Impact::High,
        ),
        (
            "k8s.listDaemonSets",
            "k8s.requestRolloutRestart",
            "DaemonSet",
            json!({}),
            Impact::High,
        ),
        (
            "k8s.listNodes",
            "k8s.requestCordonNode",
            "Node",
            json!({"unschedulable":true}),
            Impact::Medium,
        ),
    ] {
        let mut value = manifest();
        value["permissions"] = json!([reader, target]);
        value["capabilities"][0]["target"] = json!(reader);
        value["capabilities"][0]["arguments"] = json!({});
        value["capabilities"][0]["inputs"] = if kind == "Node" {
            json!(["context"])
        } else {
            json!(["context", "namespace"])
        };
        value["actions"][0]["target"] = json!(target);
        value["actions"][0]["arguments"] = arguments;
        value["contributions"]["pages"] = json!([]);
        let parsed = parse(&value).unwrap();
        let mut core = srelens_registry::build_registry();
        let mut primitive = core.get(target).unwrap().clone();
        primitive.handler = Arc::new(|input| Box::pin(async move { Ok(input) }));
        core.register(primitive);
        let host = PluginHost::new(Arc::new(core));
        let mut registry = Registry::new();
        let registration = host
            .register(&mut registry, parsed.clone(), &parsed.permission_names())
            .unwrap();
        let id = "plugin/org.example.gitops/refresh";
        let cap = registry.get(id).unwrap();
        assert_eq!(cap.annotations.impact, impact);
        assert!(cap.annotations.requires_confirm);
        let input = json!({"context":"srelens-context:/config#prod","namespace":if kind=="Node" {""} else {"team"},"name":"api","uid":"u","resourceVersion":"2"});
        let result = registry.invoke(id, input.clone()).await.unwrap();
        assert_eq!(result["kind"], kind);
        assert_eq!(result["group"], if kind == "Node" { "" } else { "apps" });
        assert_eq!(result["context"], input["context"]);
        assert_eq!(result["resourceVersion"], "2");
        for field in ["group", "kind", "plural", "unschedulable", "preconditions"] {
            let mut forged = input.clone();
            forged[field] = json!("forged");
            assert!(registry.invoke(id, forged).await.is_err());
        }
        let mcp = srelens_mcp::McpServer::new(Arc::new(registry.clone()));
        let request = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":id,"arguments":input.clone()}});
        let response =
            srelens_mcp::stdio::handle_request(&mcp, &request, srelens_mcp::Transport::Stdio)
                .await
                .unwrap();
        assert_eq!(response["result"]["isError"], true);
        let snapshot = registry.clone();
        registration.unregister(&mut registry);
        assert!(snapshot.invoke(id, input).await.is_err());

        value["actions"][0]["target"] = json!("k8s.setFields");
        value["actions"][0]["arguments"] = json!({"fields":{"/spec/replicas":0}});
        value["permissions"] = json!([reader, "k8s.setFields"]);
        assert!(
            install(&value).is_err(),
            "a built-in reader must not unlock general writes"
        );
    }
}
