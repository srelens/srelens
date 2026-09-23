use serde_json::{json, Value};
use srelens_capability::{Annotations, Capability, Registry};
use srelens_plugin_host::{Manifest, PluginHost};
use std::sync::Arc;

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

/// The same rule over the metadata #548 adds. `plugin_cannot_downgrade_core_consent_annotations`
/// above compares whole rows, so it would pass even if `impact` and `confirm`
/// were dropped from both sides; this names them.
///
/// The level is the field an extension would most like to soften — it is the
/// one a confirming surface renders as "how alarmed should you be" — and the
/// wording is the one it would most like to own.
#[test]
fn a_binding_carries_the_hosts_level_and_the_hosts_words() {
    use srelens_capability::Impact;
    let mut reg = core();
    let mut cap = reg.get("k8s.listCustomResource").unwrap().clone();
    cap.annotations = Annotations::DESTRUCTIVE.with_confirm("Drain[ {resource}]?");
    reg.register(cap);
    let host = PluginHost::new(Arc::new(reg.clone()));
    let _installed = host
        .register(
            &mut reg,
            Manifest::parse(&manifest().to_string()).unwrap(),
            &["k8s.listCustomResource".into()],
        )
        .unwrap();
    let bound = reg.get("plugin/org.example.gitops/applications").unwrap();
    assert_eq!(bound.annotations.impact, Impact::High);
    assert_eq!(bound.annotations.confirm, Some("Drain[ {resource}]?"));
}

/// And the gate closes over a host row that forgot it: a capability that
/// mutates but was registered ungated comes out of the broker gated AND at a
/// level that matches, rather than gated at `low` — which would read to a
/// confirming surface as "stop the user for something that changes nothing".
#[test]
fn a_binding_of_an_ungated_host_row_is_gated_and_levelled() {
    use srelens_capability::Impact;
    let mut reg = core();
    let mut cap = reg.get("k8s.listCustomResource").unwrap().clone();
    cap.annotations = Annotations {
        read_only: false,
        ..Annotations::READ_ONLY
    };
    reg.register(cap);
    let host = PluginHost::new(Arc::new(reg.clone()));
    let _installed = host
        .register(
            &mut reg,
            Manifest::parse(&manifest().to_string()).unwrap(),
            &["k8s.listCustomResource".into()],
        )
        .unwrap();
    let bound = reg.get("plugin/org.example.gitops/applications").unwrap();
    assert!(bound.annotations.requires_confirm);
    assert_eq!(bound.annotations.impact, Impact::Medium);
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
        let readers: std::collections::BTreeSet<_> = manifest.capabilities.iter().map(|binding| format!("plugin/{}/{}", manifest.id, binding.name)).collect();
        let count = manifest.capabilities.len() + manifest.actions.len();
        let grants = manifest.permissions.clone();
        let mut reg = Registry::new();
        let _installed = host.register(&mut reg, manifest, &grants).unwrap();
        assert_eq!(reg.ids().len(), count);
        for cap in reg.entries() {
            assert_eq!(cap.annotations.read_only, readers.contains(&cap.id));
            if !cap.annotations.read_only { assert!(cap.annotations.requires_confirm); }
            assert!(cap.input_schema["properties"].get("group").is_none());
            assert!(cap.input_schema["properties"].get("context").is_some());
        }
    }
}

#[test]
fn gitops_examples_declare_curated_inspector_panels() {
    let flux = Manifest::parse(include_str!("../../../examples/extensions/flux.json")).unwrap();
    let argo = Manifest::parse(include_str!("../../../examples/extensions/argocd.json")).unwrap();
    for kind in [
        "kustomize.toolkit.fluxcd.io/Kustomization",
        "helm.toolkit.fluxcd.io/HelmRelease",
    ] {
        assert!(
            flux.contributions
                .detail_panels
                .iter()
                .any(|panel| panel.for_kinds.iter().any(|candidate| candidate == kind)),
            "missing Flux panel for {kind}"
        );
    }
    assert!(argo.contributions.detail_panels.iter().any(|panel| panel
        .for_kinds
        .contains(&"argoproj.io/Application".to_owned())));
}

#[test]
fn the_gitops_examples_resolve_status_with_rules_and_badge_workloads() {
    use srelens_capability::status::{first_match, resolve_status, NormalizedStatus as S};
    let flux = Manifest::parse(include_str!("../../../examples/extensions/flux.json")).unwrap();
    let argo = Manifest::parse(include_str!("../../../examples/extensions/argocd.json")).unwrap();
    for manifest in [&flux, &argo] {
        // Migrated: no page counts by printer-column index any more, and
        // every custom-resource reader's kind has a resolver.
        assert!(manifest
            .contributions
            .pages
            .iter()
            .all(|page| page.status_columns.is_none()));
        for binding in &manifest.capabilities {
            if let Some(kind) = Manifest::reader_kind(binding) {
                assert!(
                    manifest.status_rules_for(&kind).is_some(),
                    "{} has no resolver for {kind}",
                    manifest.id
                );
            }
        }
    }
    let flux_rules = flux
        .status_rules_for("helm.toolkit.fluxcd.io/HelmRelease")
        .unwrap();
    let ready = |status: &str, message: &str| {
        json!({"spec":{},"status":{"conditions":[
            {"type":"Reconciling","status":"False","message":"idle"},
            {"type":"Ready","status":status,"message":message}]}})
    };
    let resolved = |rules, object: Value| {
        let got = resolve_status(rules, &object);
        (got.status, got.label, got.reason)
    };
    assert_eq!(
        resolved(
            flux_rules,
            ready("True", "Release reconciliation succeeded")
        ),
        (S::Healthy, "Ready".into(), None)
    );
    assert_eq!(
        resolved(flux_rules, ready("False", "install retries exhausted")),
        (
            S::Error,
            "Not ready".into(),
            Some("install retries exhausted".into())
        )
    );
    let mut suspended = ready("True", "ok");
    suspended["spec"]["suspend"] = json!(true);
    assert_eq!(resolved(flux_rules, suspended).0, S::Suspended);
    assert_eq!(resolved(flux_rules, json!({})).0, S::Unknown);

    let argo_rules = argo.status_rules_for("argoproj.io/Application").unwrap();
    let app = |health: &str, sync: &str| {
        json!({"status":{"health":{"status":health,"message":"waiting for rollout"},
            "sync":{"status":sync,"revision":"abc123"}}})
    };
    assert_eq!(resolved(argo_rules, app("Healthy", "Synced")).0, S::Healthy);
    assert_eq!(
        resolved(argo_rules, app("Healthy", "OutOfSync")),
        (S::Warning, "Out of sync".into(), Some("abc123".into()))
    );
    assert_eq!(resolved(argo_rules, app("Degraded", "Synced")).0, S::Error);
    assert_eq!(
        resolved(argo_rules, app("Progressing", "Synced")).0,
        S::Progressing
    );
    assert_eq!(
        resolved(argo_rules, app("Suspended", "Synced")).0,
        S::Suspended
    );

    // GitOps ownership on built-in workloads: the exit criterion of #517.
    // Shaped as the host's metadata read hands a Deployment `team/guestbook`
    // to a direct badge: its identity plus the metadata the rule may read.
    let owned_named = |manifest: &Manifest, name: &str, metadata: Value| {
        let badge = &manifest.contributions.badges[0];
        assert!(badge.for_kinds.iter().any(|kind| kind == "apps/Deployment"));
        assert!(badge.join.is_none());
        let mut metadata = metadata;
        metadata["name"] = json!(name);
        metadata["namespace"] = json!("team");
        let object = json!({"apiVersion": "apps/v1", "kind": "Deployment", "metadata": metadata});
        first_match(&badge.rules, &object).map(|b| (b.label, b.reason))
    };
    let owned = |manifest: &Manifest, metadata: Value| owned_named(manifest, "guestbook", metadata);
    // A tracking id copied onto another workload names `team/guestbook`, not
    // `team/clone`: Argo CD does not own `clone`, so it gets no badge.
    assert_eq!(
        owned_named(
            &argo,
            "clone",
            json!({"annotations":{"argocd.argoproj.io/tracking-id":"guestbook:apps/Deployment:team/guestbook"}})
        ),
        None
    );
    assert_eq!(
        owned(
            &flux,
            json!({"labels":{"kustomize.toolkit.fluxcd.io/name":"apps"}})
        ),
        Some(("Flux".into(), Some("apps".into())))
    );
    assert_eq!(
        owned(
            &flux,
            json!({"labels":{"helm.toolkit.fluxcd.io/name":"podinfo"}})
        ),
        Some(("Flux".into(), Some("podinfo".into())))
    );
    assert_eq!(
        owned(
            &argo,
            json!({"annotations":{"argocd.argoproj.io/tracking-id":"guestbook:apps/Deployment:team/guestbook"}})
        ),
        Some((
            "Argo CD".into(),
            Some("guestbook:apps/Deployment:team/guestbook".into())
        ))
    );
    // `app.kubernetes.io/instance` alone is Helm's label too; it is not
    // ownership by Argo CD.
    assert_eq!(
        owned(
            &argo,
            json!({"labels":{"app.kubernetes.io/instance":"guestbook"}})
        ),
        None
    );
    assert_eq!(owned(&flux, json!({})), None);
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
    value["contributions"]["detailLinks"] = json!([{"id":"inspect","title":"Inspect","capability":"applications","forKinds":["argoproj.io/Application"]}]);
    assert!(Manifest::parse(&value.to_string()).is_ok());
    value["contributions"]["detailLinks"][0]["forKinds"] = json!(["Application"]);
    assert!(Manifest::parse(&value.to_string()).is_err());
    value["contributions"]["detailLinks"] = json!([]);
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
fn host_accepts_api_03_and_rejects_retired_api_lines() {
    let mut value = manifest();
    value["srelensApiVersion"] = json!("^0.3");
    Manifest::parse(&value.to_string()).unwrap();
    for retired in ["^0.1", "^0.2"] {
        value["srelensApiVersion"] = json!(retired);
        let error = Manifest::parse(&value.to_string()).unwrap_err().to_string();
        assert!(error.contains(&format!("requires API {retired}")), "{error}");
        assert!(error.contains("host supports 0.3.0"), "{error}");
    }
}

#[test]
fn api_ranges_negotiate_against_every_supported_version() {
    use srelens_plugin_host::{negotiate_api_version_in, SUPPORTED_API_VERSIONS};
    let req = |range: &str| semver::VersionReq::parse(range).unwrap();
    let both = ["0.1.0", "0.2.0"];
    let negotiated = |range: &str, supported: &[&str]| {
        negotiate_api_version_in(&req(range), supported).map(|v| v.to_string())
    };
    // A ^0.1 manifest keeps installing on a host that also supports 0.2.
    assert_eq!(negotiated("^0.1", &both).as_deref(), Some("0.1.0"));
    assert_eq!(negotiated("^0.2", &both).as_deref(), Some("0.2.0"));
    // A range spanning several supported versions is served under the highest.
    assert_eq!(negotiated(">=0.1, <1", &both).as_deref(), Some("0.2.0"));
    assert_eq!(negotiated("^0.3", &both), None);
    assert_eq!(negotiated("^0.1", &["0.2.0"]), None);
    // The host's own set parses and is ordered oldest to newest.
    let parsed: Vec<semver::Version> = SUPPORTED_API_VERSIONS
        .iter()
        .map(|v| semver::Version::parse(v).unwrap())
        .collect();
    assert!(!parsed.is_empty() && parsed.windows(2).all(|w| w[0] < w[1]));
    assert!(Manifest::parse(&manifest().to_string()).is_ok());
}

#[test]
fn a_manifest_for_a_newer_api_is_told_the_version_it_needs_not_an_unknown_field() {
    let mut newer = manifest();
    newer["srelensApiVersion"] = json!("^0.9");
    newer["contributions"]["notYetAContribution"] = json!([]);
    let error = Manifest::parse(&newer.to_string()).unwrap_err().to_string();
    assert!(error.contains("requires API ^0.9"), "{error}");
    assert!(
        error.contains(&srelens_plugin_host::SUPPORTED_API_VERSIONS.join(", ")),
        "{error}"
    );
    // A supported range still gets the strict schema.
    newer["srelensApiVersion"] = json!("^0.3");
    assert!(Manifest::parse(&newer.to_string())
        .unwrap_err()
        .to_string()
        .contains("unknown field"));
}

#[test]
fn fields_must_exist_in_every_api_version_the_range_admits() {
    use srelens_plugin_host::{
        check_api_fields_in, matching_api_versions_in, ApiField, API_FIELDS, SUPPORTED_API_VERSIONS,
    };
    let version = |v: &str| semver::Version::parse(v).unwrap();
    let range = |r: &str| semver::VersionReq::parse(r).unwrap();
    let both = ["0.1.0", "0.2.0"];
    let fields = [
        ApiField {
            path: "contributions.dashboardCards",
            introduced: "0.2.0",
            removed: None,
        },
        ApiField {
            path: "contributions.pages[].badges",
            introduced: "0.2.0",
            removed: None,
        },
        // A 0.1 field a later line removed; a rename is this plus an addition.
        ApiField {
            path: "contributions.detailLinks",
            introduced: "0.1.0",
            removed: Some("0.2.0"),
        },
    ];
    let with_detail_link = || {
        let mut value = manifest();
        value["contributions"]["detailLinks"] = json!([{"id":"inspect","title":"Inspect","capability":"applications","forKinds":["argoproj.io/Application"]}]);
        value
    };
    let admits = |r: &str| matching_api_versions_in(&range(r), &both);
    assert_eq!(admits("^0.1"), [version("0.1.0")]);
    assert_eq!(admits("^0.2"), [version("0.2.0")]);
    assert_eq!(admits(">=0.1, <0.3"), [version("0.1.0"), version("0.2.0")]);
    assert!(admits("^0.3").is_empty());

    assert!(check_api_fields_in(&manifest(), &admits("^0.1"), &fields).is_ok());
    // An empty value contributes nothing, so it does not count as using the field.
    assert!(check_api_fields_in(&manifest(), &admits("^0.2"), &fields).is_ok());
    let mut empty_cards = manifest();
    empty_cards["contributions"]["dashboardCards"] = json!([]);
    assert!(check_api_fields_in(&empty_cards, &admits("^0.1"), &fields).is_ok());

    // A 0.2 field under ^0.1 fails even on a host that knows the field.
    let mut top_level = manifest();
    top_level["contributions"]["dashboardCards"] = json!([{"id": "card"}]);
    let error = check_api_fields_in(&top_level, &admits("^0.1"), &fields).unwrap_err();
    assert!(
        error.contains("`contributions.dashboardCards` requires API 0.2.0"),
        "{error}"
    );
    assert!(error.contains("admits API 0.1.0"), "{error}");
    assert!(check_api_fields_in(&top_level, &admits("^0.2"), &fields).is_ok());
    // A range spanning 0.1 and 0.2 still claims 0.1 hosts, so the 0.2 field is rejected.
    let spanning = check_api_fields_in(&top_level, &admits(">=0.1, <0.3"), &fields).unwrap_err();
    assert!(spanning.contains("admits API 0.1.0"), "{spanning}");

    let mut per_page = manifest();
    per_page["contributions"]["pages"][0]["badges"] = json!([{"id": "badge"}]);
    assert!(check_api_fields_in(&per_page, &admits("^0.1"), &fields).is_err());
    assert!(check_api_fields_in(&per_page, &admits("^0.2"), &fields).is_ok());

    // A field removed in 0.2 is accepted under ^0.1 and rejected by any range admitting 0.2.
    let removed = check_api_fields_in(&with_detail_link(), &admits("^0.2"), &fields).unwrap_err();
    assert!(
        removed.contains("`contributions.detailLinks` was removed in API 0.2.0"),
        "{removed}"
    );
    assert!(check_api_fields_in(&with_detail_link(), &admits(">=0.1, <0.3"), &fields).is_err());
    assert!(check_api_fields_in(&with_detail_link(), &admits("^0.1"), &fields).is_ok());

    // Every gated field names a supported version and is removed only after it arrived.
    for field in API_FIELDS {
        assert!(
            SUPPORTED_API_VERSIONS.contains(&field.introduced),
            "{}",
            field.path
        );
        if let Some(removed) = field.removed {
            assert!(
                version(removed) > version(field.introduced),
                "{}",
                field.path
            );
        }
    }
}

#[test]
fn stored_manifests_are_rechecked_against_the_hosts_api_fields() {
    use srelens_plugin_host::ApiField;
    // An app installed while 0.2 was newest, with a range that also admits 0.3, which
    // later removes a field the app uses.
    let mut value = manifest();
    value["srelensApiVersion"] = json!(">=0.2, <0.4");
    value["contributions"]["detailLinks"] = json!([{"id":"inspect","title":"Inspect","capability":"applications","forKinds":["argoproj.io/Application"]}]);
    // The inventory deserializes stored manifests directly rather than through parse.
    let stored: Manifest = serde_json::from_value(value).unwrap();
    let fields = [ApiField {
        path: "contributions.detailLinks",
        introduced: "0.1.0",
        removed: Some("0.3.0"),
    }];
    assert!(stored
        .check_api_fields(&["0.1.0", "0.2.0"], &fields)
        .is_ok());
    let error = stored
        .check_api_fields(&["0.1.0", "0.2.0", "0.3.0"], &fields)
        .unwrap_err();
    assert!(error.contains("was removed in API 0.3.0"), "{error}");
    assert!(error.contains("admits API 0.3.0"), "{error}");
    // Unused collection fields serialize as empty and do not count as using a field.
    let mut unused = manifest();
    unused["srelensApiVersion"] = json!(">=0.2, <0.4");
    let unused: Manifest = serde_json::from_value(unused).unwrap();
    assert!(unused
        .check_api_fields(&["0.1.0", "0.2.0", "0.3.0"], &fields)
        .is_ok());
}

#[test]
fn core_kinds_use_an_explicit_empty_api_group() {
    let mut value = manifest();
    value["contributions"]["detailTabs"] =
        json!([{"id":"detail","title":"Details","capability":"applications","forKinds":["/Pod"]}]);
    assert!(Manifest::parse(&value.to_string()).is_ok());
}

#[test]
fn dashboard_references_are_validated() {
    let mut value = manifest();
    value["contributions"]["pages"][0]["group"] = json!("Workloads");
    value["contributions"]["pages"][0]["statusColumns"] = json!({"ready":0});
    value["contributions"]["pages"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id":"overview", "title":"Overview", "capability":"applications",
            "dashboard":{"pages":["applications"]}
        }));
    assert!(Manifest::parse(&value.to_string()).is_ok());
    value["contributions"]["pages"][1]["dashboard"]["pages"] = json!(["missing"]);
    assert!(Manifest::parse(&value.to_string()).is_err());
    value["contributions"]["pages"][1]["dashboard"]["pages"] = json!(["overview"]);
    assert!(Manifest::parse(&value.to_string()).is_err());
}
