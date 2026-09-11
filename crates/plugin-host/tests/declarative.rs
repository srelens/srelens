use serde_json::{json, Value};
use srelens_capability::{Annotations, Capability, Registry};
use srelens_plugin_host::{Manifest, PluginHost};
use std::sync::Arc;

fn manifest() -> Value {
    json!({
        "id":"org.example.gitops", "name":"GitOps", "version":"0.1.0", "srelensApiVersion":"^0.1",
        "kind":"declarative", "permissions":["k8s.listCustomResource"],
        "capabilities":[{"name":"applications","title":"List applications", "target":"k8s.listCustomResource",
            "arguments":{"group":"argoproj.io"},"inputs":["context","namespace"]}],
        "contributions":{"pages":[{"id":"applications","title":"Applications","capability":"applications"}],
            "detailTabs":[],"rowActions":[]}
    })
}
fn core() -> Registry {
    let mut registry = Registry::new();
    let mut cap = Capability::read_only(
        "k8s.listCustomResource",
        "fixture",
        |v| async move { Ok(v) },
    );
    cap.input_schema = json!({"type":"object","properties":{"context":{"type":"string"},"namespace":{"type":"string"},"group":{"type":"string"}},"required":["context","group"]});
    registry.register(cap);
    registry
}

#[tokio::test]
async fn plugin_is_callable_through_mcp_with_bound_resource_scope() {
    let mut reg = core();
    let host = PluginHost::new(Arc::new(reg.clone()));
    let installed = host
        .register(
            &mut reg,
            Manifest::parse(&manifest().to_string()).unwrap(),
            &["k8s.listCustomResource".into()],
        )
        .unwrap();
    let mcp = srelens_mcp::McpServer::new(Arc::new(reg));
    let id = "plugin/org.example.gitops/applications";
    assert!(mcp.list_tools().iter().any(|tool| tool.name == id));
    assert_eq!(
        mcp.call_tool(id, json!({"context":"staging","namespace":"argo"}))
            .await
            .unwrap(),
        json!({"context":"staging","namespace":"argo","group":"argoproj.io"})
    );
    assert!(mcp
        .call_tool(id, json!({"context":"prod","group":""}))
        .await
        .is_err());
    assert_eq!(installed.manifest().contributions.pages.len(), 1);
}

#[test]
fn validation_rejects_unsupported_code_and_ambiguous_contributions() {
    for change in [
        json!({"backend":{"entry":"evil.js"}}),
        json!({"kind":"lens-compat"}),
        json!({"srelensApiVersion":"^99"}),
        json!({"id":"../escape"}),
    ] {
        let mut value = manifest();
        value
            .as_object_mut()
            .unwrap()
            .extend(change.as_object().unwrap().clone());
        assert!(Manifest::parse(&value.to_string()).is_err(), "{value}");
    }
    let mut value = manifest();
    value["contributions"]["pages"][0]["capability"] = json!("missing");
    assert!(Manifest::parse(&value.to_string()).is_err());
}

#[test]
fn permissions_and_registration_are_fail_closed_and_atomic() {
    let mut reg = core();
    let host = PluginHost::new(Arc::new(reg.clone()));
    let parsed = || Manifest::parse(&manifest().to_string()).unwrap();
    assert!(host.register(&mut reg, parsed(), &[]).is_err());
    assert_eq!(reg.ids().len(), 1);
    let _installed = host
        .register(&mut reg, parsed(), &["k8s.listCustomResource".into()])
        .unwrap();
    assert!(host
        .register(&mut reg, parsed(), &["k8s.listCustomResource".into()])
        .is_err());
    assert_eq!(reg.ids().len(), 2);
}

#[tokio::test]
async fn revocation_also_blocks_existing_mcp_registry_snapshots() {
    let mut reg = core();
    let host = PluginHost::new(Arc::new(reg.clone()));
    let installed = host
        .register(
            &mut reg,
            Manifest::parse(&manifest().to_string()).unwrap(),
            &["k8s.listCustomResource".into()],
        )
        .unwrap();
    let mcp = srelens_mcp::McpServer::new(Arc::new(reg.clone()));
    installed.unregister(&mut reg);
    assert_eq!(reg.ids(), vec!["k8s.listCustomResource"]);
    assert!(mcp
        .call_tool(
            "plugin/org.example.gitops/applications",
            json!({"context":"staging"})
        )
        .await
        .is_err());
}

#[test]
fn plugin_cannot_downgrade_core_consent_annotations() {
    for annotation in [
        Annotations::MUTATING,
        Annotations::DESTRUCTIVE,
        Annotations::SENSITIVE_READ,
    ] {
        let mut reg = core();
        let mut cap = reg.get("k8s.listCustomResource").unwrap().clone();
        cap.annotations = annotation;
        reg.register(cap);
        let host = PluginHost::new(Arc::new(reg.clone()));
        let _installed = host
            .register(
                &mut reg,
                Manifest::parse(&manifest().to_string()).unwrap(),
                &["k8s.listCustomResource".into()],
            )
            .unwrap();
        let id = "plugin/org.example.gitops/applications";
        assert_eq!(reg.get(id).unwrap().annotations, annotation);
        let mcp = srelens_mcp::McpServer::new(Arc::new(reg));
        assert!(mcp.requires_confirm(id));
        assert_eq!(mcp.is_sensitive(id), annotation.sensitive);
    }
}

#[tokio::test]
async fn mcp_transport_requires_real_consent_before_dispatch() {
    use srelens_mcp::{policy::FlagGated, stdio::handle_request, Transport};
    let mut core = core();
    let mut cap = core.get("k8s.listCustomResource").unwrap().clone();
    cap.annotations = Annotations::MUTATING;
    core.register(cap);
    let host = PluginHost::new(Arc::new(core));
    let mut reg = Registry::new();
    let _installed = host
        .register(
            &mut reg,
            Manifest::parse(&manifest().to_string()).unwrap(),
            &["k8s.listCustomResource".into()],
        )
        .unwrap();
    let server = srelens_mcp::McpServer::new(Arc::new(reg))
        .with_policy(Arc::new(FlagGated::new(true, false)));
    let mut request = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"plugin/org.example.gitops/applications","arguments":{"context":"staging"}}});
    let denied = handle_request(&server, &request, Transport::Stdio)
        .await
        .unwrap();
    assert_eq!(denied["result"]["isError"], json!(true), "{denied}");
    request["params"]["arguments"]["_confirm"] = json!(true);
    let allowed = handle_request(&server, &request, Transport::Stdio)
        .await
        .unwrap();
    assert!(
        allowed.get("error").is_none() && allowed["result"]["isError"] != json!(true),
        "{allowed}"
    );
    assert!(!allowed.to_string().contains("_confirm"));
}

#[test]
fn gitops_examples_bind_to_the_real_host_contract() {
    let core = Arc::new(srelens_registry::build_registry());
    let host = PluginHost::new(core);
    for source in [
        include_str!("../../../examples/extensions/argocd.json"),
        include_str!("../../../examples/extensions/flux.json"),
    ] {
        let manifest = Manifest::parse(source).unwrap();
        let count = manifest.capabilities.len();
        let mut reg = Registry::new();
        let _installed = host
            .register(&mut reg, manifest, &["k8s.listCustomResource".into()])
            .unwrap();
        assert_eq!(reg.ids().len(), count);
        for cap in reg.entries() {
            assert!(cap.annotations.read_only);
            assert!(cap.input_schema["properties"].get("group").is_none());
            assert!(cap.input_schema["properties"].get("context").is_some());
        }
    }
}

#[test]
fn a_later_invalid_binding_does_not_partially_register() {
    let mut value = manifest();
    let mut second = value["capabilities"][0].clone();
    second["name"] = json!("invalid");
    second["inputs"] = json!(["context", "unknown"]);
    value["capabilities"].as_array_mut().unwrap().push(second);
    let mut reg = core();
    let host = PluginHost::new(Arc::new(reg.clone()));
    assert!(host
        .register(
            &mut reg,
            Manifest::parse(&value.to_string()).unwrap(),
            &["k8s.listCustomResource".into()]
        )
        .is_err());
    assert_eq!(reg.ids().len(), 1);
}

#[test]
fn a_second_manifest_cannot_share_an_extension_identity() {
    let mut reg = core();
    let host = PluginHost::new(Arc::new(reg.clone()));
    let grants = ["k8s.listCustomResource".into()];
    let _installed = host
        .register(
            &mut reg,
            Manifest::parse(&manifest().to_string()).unwrap(),
            &grants,
        )
        .unwrap();
    let mut other = manifest();
    other["capabilities"][0]["name"] = json!("other");
    other["contributions"]["pages"][0]["capability"] = json!("other");
    assert!(host
        .register(
            &mut reg,
            Manifest::parse(&other.to_string()).unwrap(),
            &grants
        )
        .is_err());
}

#[tokio::test]
async fn invocation_rejects_missing_and_non_object_inputs() {
    let mut reg = core();
    let host = PluginHost::new(Arc::new(reg.clone()));
    let _installed = host
        .register(
            &mut reg,
            Manifest::parse(&manifest().to_string()).unwrap(),
            &["k8s.listCustomResource".into()],
        )
        .unwrap();
    for input in [json!(null), json!([]), json!({})] {
        assert!(reg
            .invoke("plugin/org.example.gitops/applications", input)
            .await
            .is_err());
    }
}

#[test]
fn manifest_wire_contract_and_contribution_identity_are_strict() {
    for (field, value) in [
        ("version", json!("invalid")),
        ("name", json!("")),
        ("permissions", json!([])),
        ("capabilities", json!([])),
        ("srelensApiVersion", json!("???")),
    ] {
        let mut invalid = manifest();
        invalid[field] = value;
        assert!(Manifest::parse(&invalid.to_string()).is_err());
    }
    let mut value = manifest();
    value["contributions"]["detailTabs"] = json!([{"id":"detail","title":"Details","capability":"applications","forKinds":["argoproj.io/Application"]}]);
    value["contributions"]["rowActions"] = json!([{"id":"inspect","title":"Inspect","capability":"applications","forKinds":["argoproj.io/Application"]}]);
    assert!(Manifest::parse(&value.to_string()).is_ok());
    value["contributions"]["rowActions"][0]["forKinds"] = json!(["Application"]);
    assert!(Manifest::parse(&value.to_string()).is_err());
    value["contributions"]["rowActions"] = json!([]);
    value["contributions"]["detailTabs"][0]["id"] = json!("applications");
    assert!(Manifest::parse(&value.to_string()).is_err());
    let mut value = manifest();
    let version = value
        .as_object_mut()
        .unwrap()
        .remove("srelensApiVersion")
        .unwrap();
    value["api_version"] = version;
    assert!(Manifest::parse(&value.to_string()).is_err());
    assert!(Manifest::schema()["properties"]
        .get("srelensApiVersion")
        .is_some());
    assert!(Manifest::parse(&" ".repeat(256 * 1024 + 1)).is_err());
}

#[test]
fn core_kinds_use_an_explicit_empty_api_group() {
    let mut value = manifest();
    value["contributions"]["detailTabs"] =
        json!([{"id":"detail","title":"Details","capability":"applications","forKinds":["/Pod"]}]);
    assert!(Manifest::parse(&value.to_string()).is_ok());
}
