//! An app's operations as tools (#574): `PluginHost::register_tools` registers each
//! reader, declared action and sidecar operation as `plugin/<id>/<name>`, with the
//! host's schema and annotations, checks every call, and hands it to the broker's route.
//!
//! Against the real host registry, because what is checked is that a manifest cannot
//! make a tool look weaker than the host capability behind it.
use serde_json::{json, Map, Value};
use srelens_capability::{Annotations, CapabilityError, Impact, Registry};
use srelens_plugin_host::{
    Manifest, PluginHost, ToolRoute, SIDECAR_OPERATION, SIDECAR_OPERATION_CONFIRM,
};
use std::sync::{Arc, Mutex};

/// Argo CD with a reader, a logs binding and a declared action, plus a sidecar.
fn manifest() -> Value {
    json!({
        "id":"org.example.gitops", "name":"GitOps", "version":"0.1.0", "srelensApiVersion":"^0.6",
        "kind":"executable",
        "permissions":["k8s.listCustomResource","k8s.annotate","k8s.streamLogs"],
        "capabilities":[
            {"name":"applications","title":"List applications","target":"k8s.listCustomResource",
             "arguments":{"group":"argoproj.io","version":"v1alpha1","plural":"applications",
                 "kind":"Application","namespaced":true},
             "inputs":["context","namespace"]},
            {"name":"logs","title":"Controller logs","target":"k8s.streamLogs","inputs":[],
             "arguments":{"resource":"applications","selector":".spec.selector"}}],
        "actions":[{"name":"refresh","title":"Refresh","target":"k8s.annotate","resource":"applications",
            "arguments":{"key":"argocd.argoproj.io/refresh","value":"normal"}}],
        "sidecar":{"binaries":{"linux-amd64":"bin/linux-amd64/gitops-helper"},
            "operations":[{"name":"diff","title":"Diff an application","inputs":[
                {"name":"application","type":"string","required":true,"maxLength":253}]}]},
        "contributions":{"pages":[{"id":"applications","title":"Applications","capability":"applications"}],
            "detailTabs":[],"detailLinks":[]}
    })
}

type Calls = Arc<Mutex<Vec<(String, Map<String, Value>)>>>;

/// A route that records what reached it and answers with the operation's name.
fn recording() -> (ToolRoute, Calls) {
    let calls: Calls = Arc::default();
    let seen = calls.clone();
    let route: ToolRoute = Arc::new(move |name, input| {
        seen.lock().unwrap().push((name.clone(), input));
        Box::pin(async move { Ok(json!({ "ran": name })) })
    });
    (route, calls)
}

fn host() -> PluginHost {
    PluginHost::new(Arc::new(srelens_registry::build_registry()))
}

fn register(
    value: &Value,
    route: ToolRoute,
) -> Result<(Registry, srelens_plugin_host::Registration), String> {
    let manifest = Manifest::parse(&value.to_string()).map_err(|e| e.to_string())?;
    let mut reg = Registry::new();
    let registration =
        host().register_tools(&mut reg, &manifest, &manifest.permission_names(), route)?;
    Ok((reg, registration))
}

#[tokio::test]
async fn every_reader_action_and_operation_is_a_tool_and_a_pod_binding_is_not() {
    let (route, calls) = recording();
    let (reg, registration) = register(&manifest(), route).unwrap();
    assert_eq!(
        reg.ids(),
        [
            "plugin/org.example.gitops/applications",
            "plugin/org.example.gitops/diff",
            "plugin/org.example.gitops/refresh",
        ]
    );
    assert_eq!(registration.ids().len(), 3);
    let reader = reg.get("plugin/org.example.gitops/applications").unwrap();
    assert_eq!(reader.summary, "GitOps: List applications");
    // A reader is read in one cluster, and in one namespace when it takes one.
    assert_eq!(reader.input_schema["required"], json!(["context"]));
    let properties: Vec<&String> = reader.input_schema["properties"]
        .as_object()
        .unwrap()
        .keys()
        .collect();
    assert_eq!(properties, ["context", "namespace"]);
    assert_eq!(reader.input_schema["additionalProperties"], false);

    let answer = reg
        .invoke(
            "plugin/org.example.gitops/applications",
            json!({"context":"prod","namespace":"argocd"}),
        )
        .await
        .unwrap();
    assert_eq!(answer, json!({"ran":"applications"}));
    let action = reg.get("plugin/org.example.gitops/refresh").unwrap();
    assert_eq!(
        action.input_schema["required"],
        json!(["context", "name", "uid", "resourceVersion"])
    );
    reg.invoke(
        "plugin/org.example.gitops/refresh",
        json!({"context":"prod","namespace":"argocd","name":"web","uid":"u-1","resourceVersion":"7"}),
    )
    .await
    .unwrap();
    reg.invoke(
        "plugin/org.example.gitops/diff",
        json!({"application":"web"}),
    )
    .await
    .unwrap();
    let calls = calls.lock().unwrap();
    let names: Vec<&str> = calls.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["applications", "refresh", "diff"]);
    assert_eq!(calls[0].1["namespace"], "argocd");
    assert_eq!(calls[1].1["resourceVersion"], "7");
}

#[test]
fn a_tool_runs_under_its_host_capabilitys_annotations() {
    let core = srelens_registry::build_registry();
    let (route, _) = recording();
    let (reg, _) = register(&manifest(), route).unwrap();
    let annotations = |name: &str| {
        reg.get(&format!("plugin/org.example.gitops/{name}"))
            .unwrap()
            .annotations
    };
    // The reader: read-only and ungated, as its target is.
    let reader = annotations("applications");
    assert!(reader.read_only && !reader.requires_confirm && !reader.sensitive);
    assert_eq!(reader.impact, Impact::Low);
    // The action: its primitive's row, gated, with the primitive's own words.
    let primitive = core.get("k8s.annotate").unwrap().annotations;
    let action = annotations("refresh");
    assert_eq!(
        action,
        Annotations::for_binding(primitive, Annotations::WEAKEST)
    );
    assert!(!action.read_only && action.requires_confirm);
    assert!(action.impact >= Impact::Medium);
    assert_eq!(action.confirm, primitive.confirm);
    // The sidecar operation: its sidecar may ask the broker to run the app's refresh,
    // so it is gated as that write is, in the host's own words for an operation.
    let operation = annotations("diff");
    assert!(!operation.read_only && operation.requires_confirm && operation.sensitive);
    assert!(operation.impact >= primitive.impact);
    assert_eq!(operation.confirm, Some(SIDECAR_OPERATION_CONFIRM));
}

#[test]
fn a_sidecar_operation_is_a_read_when_its_app_declares_no_action() {
    let (route, _) = recording();
    let mut value = manifest();
    value.as_object_mut().unwrap().remove("actions");
    value["permissions"] = json!(["k8s.listCustomResource", "k8s.streamLogs"]);
    let (reg, _) = register(&value, route).unwrap();
    let operation = reg
        .get("plugin/org.example.gitops/diff")
        .unwrap()
        .annotations;
    assert_eq!(operation, SIDECAR_OPERATION);
    // Nothing it can reach changes anything outside its sandbox, so it is not gated.
    assert!(operation.read_only && !operation.requires_confirm);
}

#[tokio::test]
async fn every_call_is_checked_before_it_reaches_the_route() {
    let (route, calls) = recording();
    let (reg, _) = register(&manifest(), route).unwrap();
    let refused = |name: &'static str, input: Value| {
        let reg = &reg;
        async move {
            match reg
                .invoke(&format!("plugin/org.example.gitops/{name}"), input)
                .await
            {
                Err(CapabilityError::InvalidInput(why)) => why,
                other => panic!("{name}: {other:?}"),
            }
        }
    };
    for (name, input, says) in [
        ("applications", json!({}), "requires `context`"),
        ("applications", json!("prod"), "takes an object"),
        (
            "applications",
            json!({"context":"prod","group":"other.io"}),
            "takes no input `group`",
        ),
        (
            "applications",
            json!({"context":7}),
            "`context` must be a string",
        ),
        (
            "applications",
            json!({"context":"c".repeat(1025)}),
            "`context` is at most 1024 bytes",
        ),
        (
            "refresh",
            json!({"context":"prod","name":"web","uid":"u"}),
            "requires `resourceVersion`",
        ),
        (
            "refresh",
            json!({"context":"prod","name":"web","uid":"u","resourceVersion":"1","patch":{}}),
            "takes no input `patch`",
        ),
        ("diff", json!({}), "requires `application`"),
        (
            "diff",
            json!({"application":"a".repeat(254)}),
            "`application` is at most 253 bytes",
        ),
    ] {
        let why = refused(name, input.clone()).await;
        assert!(why.contains(says), "{name} {input}: {why}");
    }
    assert!(
        calls.lock().unwrap().is_empty(),
        "a refused call reached the route"
    );
}

#[tokio::test]
async fn a_revoked_registration_leaves_every_snapshot_unable_to_run_its_tools() {
    let (route, calls) = recording();
    let (reg, registration) = register(&manifest(), route).unwrap();
    // A snapshot shared with a caller that is still holding it.
    let held = Arc::new(reg.clone());
    registration.revoke();
    for registry in [&reg, &*held] {
        let refused = registry
            .invoke(
                "plugin/org.example.gitops/diff",
                json!({"application":"web"}),
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(refused.contains("withdrawn"), "{refused}");
    }
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
fn nothing_is_registered_unless_every_tool_is() {
    let (route, _) = recording();
    // Not granted.
    let manifest_value = manifest();
    let manifest = Manifest::parse(&manifest_value.to_string()).unwrap();
    let mut reg = Registry::new();
    let refused = host()
        .register_tools(
            &mut reg,
            &manifest,
            &["k8s.listCustomResource".into()],
            route.clone(),
        )
        .err()
        .unwrap();
    assert!(refused.contains("not been granted"), "{refused}");
    assert!(reg.ids().is_empty());
    // An action whose primitive refuses what it binds: a token it does not know.
    let mut bad = manifest_value.clone();
    bad["actions"][0]["arguments"]["value"] = json!("$requestedBy");
    let manifest = Manifest::parse(&bad.to_string()).unwrap();
    let refused = host()
        .register_tools(
            &mut reg,
            &manifest,
            &manifest.permission_names(),
            route.clone(),
        )
        .err()
        .unwrap();
    assert!(refused.contains("$now"), "{refused}");
    assert!(reg.ids().is_empty(), "{:?}", reg.ids());
    // Registered twice.
    let manifest = Manifest::parse(&manifest_value.to_string()).unwrap();
    host()
        .register_tools(
            &mut reg,
            &manifest,
            &manifest.permission_names(),
            route.clone(),
        )
        .unwrap();
    let refused = host()
        .register_tools(&mut reg, &manifest, &manifest.permission_names(), route)
        .err()
        .unwrap();
    assert!(refused.contains("already registered"), "{refused}");
}

#[test]
fn a_reader_that_lists_versions_is_a_tool_whichever_version_a_cluster_serves() {
    let (route, _) = recording();
    let mut value = manifest();
    let arguments = value["capabilities"][0]["arguments"]
        .as_object_mut()
        .unwrap();
    arguments.remove("version");
    value["capabilities"][0]["versions"] = json!(["v1", "v1alpha1"]);
    let (reg, _) = register(&value, route).unwrap();
    assert!(reg.get("plugin/org.example.gitops/applications").is_some());
    assert!(reg.get("plugin/org.example.gitops/refresh").is_some());
}

#[tokio::test]
async fn declared_image_cursor_is_exposed_and_accepts_bounded_pages_only() {
    let source = json!({
        "kind":"declarative", "id":"org.example.images", "name":"Images", "version":"0.1.0", "srelensApiVersion":"^0.8",
        "permissions":["k8s.listWorkloadImages"],
        "capabilities":[{"name":"images","title":"Images","target":"k8s.listWorkloadImages",
            "arguments":{"kind":"Deployment"},"inputs":["context","namespace","cursor"]}],
        "contributions":{"pages":[{"id":"images","title":"Images","capability":"images"}],"detailTabs":[],"detailLinks":[]}
    });
    let (route, calls) = recording();
    let (reg, _registration) = register(&source, route).unwrap();
    let tool = reg.get("plugin/org.example.images/images").unwrap();
    assert_eq!(tool.input_schema["properties"]["cursor"]["maxLength"], 8192);
    for cursor in ["".to_owned(), "a".repeat(8192)] {
        reg.invoke(
            "plugin/org.example.images/images",
            json!({"context":"demo","cursor":cursor}),
        )
        .await
        .unwrap();
        assert_eq!(calls.lock().unwrap().last().unwrap().1["cursor"], cursor);
    }
    let before = calls.lock().unwrap().len();
    for input in [
        json!({"cursor":"a".repeat(8193)}),
        json!({"cursor":false}),
        json!({"cursor":"é"}),
        json!({"cursor":"a b"}),
        json!({"namespace":"a".repeat(1025)}),
    ] {
        let mut input = input.as_object().unwrap().clone();
        input.insert("context".into(), json!("demo"));
        assert!(reg
            .invoke("plugin/org.example.images/images", Value::Object(input))
            .await
            .is_err());
    }
    assert_eq!(calls.lock().unwrap().len(), before);
    let mut unpaged = source;
    unpaged["capabilities"][0]["inputs"] = json!(["context", "namespace"]);
    let (route, _) = recording();
    let (reg, _registration) = register(&unpaged, route).unwrap();
    assert!(reg
        .invoke(
            "plugin/org.example.images/images",
            json!({"context":"demo","cursor":""})
        )
        .await
        .is_err());
}
