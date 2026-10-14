use super::tests::{configure, fake_core, install, manifest};
use super::*;

fn writer() -> String {
    let mut value: Value = serde_json::from_str(&manifest()).unwrap();
    value["permissions"] = json!(["k8s.listCustomResource", "k8s.annotate"]);
    value["actions"] = json!([{"name":"refresh","title":"Refresh","target":"k8s.annotate","resource":"applications","arguments":{"key":"argocd.argoproj.io/refresh","value":"normal"}}]);
    value.to_string()
}
fn install_writer(path: &Path) -> Result<Inventory, String> {
    configure(
        path,
        json!({"action":"install","manifest":writer(),"grants":["k8s.listCustomResource","k8s.annotate"]}),
    )
}
fn policy(path: &Path, allow: bool) -> Result<Inventory, String> {
    configure(
        path,
        json!({"action":"unsignedApps","allowUnsignedApps":allow}),
    )
}
#[test]
fn unsigned_policy_defaults_off_and_legacy_mode_cannot_authorize_writes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("apps.json");
    install(&path, fake_core());
    let original: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for legacy in [None, Some(false), Some(true)] {
        let mut stored = original.clone();
        stored.as_object_mut().unwrap().remove("allowUnsignedApps");
        if let Some(value) = legacy {
            stored["developerMode"] = json!(value);
        }
        fs::write(&path, stored.to_string()).unwrap();
        let state = serde_json::to_value(read(&path).unwrap()).unwrap();
        assert_eq!(state["allowUnsignedApps"], false);
        assert_eq!(state["plugins"][0]["enabled"], legacy != Some(false));
        assert!(install_writer(&path).is_err());
    }
}
#[tokio::test]
async fn unsigned_writes_require_policy_and_toggle_revokes_existing_brokers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("apps.json");
    assert!(
        install_writer(&path).is_err(),
        "permission grants alone must not authorize unsigned writes"
    );
    policy(&path, true).unwrap();
    let state = install_writer(&path).unwrap();
    let revision = state.plugins[0].revision;
    let core = fake_core();
    let mut registry = Registry::new();
    register(
        &mut registry,
        path.clone(),
        core.clone(),
        srelens_kube::client_cache::ClientCache::new_many(vec![]),
    );
    let selected = json!({"id":"org.example.argocd","revision":revision,"capability":"applications","context":"cluster/a","namespace":"team","name":"app"});
    let reader = json!({"id":"org.example.argocd","revision":revision,"capability":"applications","context":"cluster/a"});
    registry
        .invoke("extensions.read", reader.clone())
        .await
        .unwrap();
    let off = serde_json::to_value(policy(&path, false).unwrap()).unwrap();
    assert_eq!(off["plugins"].as_array().unwrap().len(), 1);
    assert_eq!(off["plugins"][0]["enabled"], false);
    assert!(off["plugins"][0]["policyBlocked"]
        .as_str()
        .unwrap()
        .contains("Allow unsigned apps"));
    for (id, payload) in [
        ("extensions.read", reader),
        ("extensions.resource", selected.clone()),
        (
            "extensions.action",
            json!({"resource":selected,"action":"refresh","uid":"u","resourceVersion":"1"}),
        ),
    ] {
        let error = registry.invoke(id, payload).await.unwrap_err().to_string();
        assert!(error.contains("Allow unsigned apps"), "{id}: {error}");
    }
    assert!(configure(
        &path,
        json!({"action":"enable","id":"org.example.argocd","enabled":true})
    )
    .is_err());
    let on = policy(&path, true).unwrap();
    assert!(
        !on.plugins[0].enabled,
        "enabling policy must not reactivate apps"
    );
    configure(
        &path,
        json!({"action":"enable","id":"org.example.argocd","enabled":true}),
    )
    .unwrap();
}
#[tokio::test]
async fn validation_reports_policy_and_executable_manifests_stay_unsupported() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("apps.json");
    let mut registry = Registry::new();
    register(
        &mut registry,
        path.clone(),
        fake_core(),
        srelens_kube::client_cache::ClientCache::new_many(vec![]),
    );
    let payload = json!({"manifest":writer(),"grants":["k8s.listCustomResource","k8s.annotate"]});
    let report = registry
        .invoke("extensions.validate", payload.clone())
        .await
        .unwrap();
    assert!(report["errors"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["message"]
            .as_str()
            .unwrap()
            .contains("Allow unsigned apps")));
    policy(&path, true).unwrap();
    assert_eq!(
        registry
            .invoke("extensions.validate", payload)
            .await
            .unwrap()["errors"],
        json!([])
    );
    let mut executable: Value = serde_json::from_str(&manifest()).unwrap();
    executable["kind"] = json!("executable");
    assert!(configure(&path,json!({"action":"install","manifest":executable.to_string(),"grants":["k8s.listCustomResource"]})).is_err());
}

#[test]
fn verified_writers_are_allowed_and_a_forged_proof_does_not_bypass_policy() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("apps.json");
    // Published bytes: https://github.com/srelens/extension-argocd/releases/tag/v0.3.0
    // Kept separately from the retired API fixture so those rejection tests remain intact.
    let source = include_str!("../../tests/fixtures/argocd-0.3.0-manifest.json");
    let signature = include_bytes!("../../tests/fixtures/argocd-0.3.0-manifest.sig");
    let state = configure(&path,json!({"action":"install","manifest":source,"signature":signature.to_vec(),"grants":["k8s.listCustomResource","k8s.annotate","k8s.mergePatch"]})).unwrap();
    assert!(state.plugins[0].enabled);
    assert!(!state.plugins[0].manifest.actions.is_empty());
    assert!(policy(&path, false).unwrap().plugins[0].enabled);
    let mut stored: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    stored["plugins"][0]["signatureProof"]["signature"][0] = json!(0);
    // A forged cached quarantine/policy verdict is never authoritative either.
    stored["plugins"][0]["quarantined"] = Value::Null;
    stored["plugins"][0]["policyBlocked"] = Value::Null;
    fs::write(&path, stored.to_string()).unwrap();
    let state = read(&path).unwrap();
    assert!(!state.plugins[0].enabled);
    assert!(state.plugins[0].quarantined.is_some());
    assert!(state.plugins[0].policy_blocked.is_some());
}

#[test]
fn source_labels_do_not_exempt_writers_and_read_only_apps_keep_normal_grants() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("apps.json");
    install(&path, fake_core());
    let original: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for source in ["local", "catalog"] {
        let mut stored = original.clone();
        stored["plugins"][0]["source"] = json!(source);
        fs::write(&path, stored.to_string()).unwrap();
        assert!(read(&path).unwrap().plugins[0].enabled);
        // Stored grants cannot turn a declared action into a read-only app.
        stored["plugins"][0]["manifest"] = serde_json::from_str(&writer()).unwrap();
        fs::write(&path, stored.to_string()).unwrap();
        assert!(!read(&path).unwrap().plugins[0].enabled);
    }
    assert!(configure(
        &path,
        json!({"action":"install","manifest":manifest(),"grants":[]})
    )
    .is_err());
    for invalid in [json!(null), json!("true"), json!(1)] {
        let mut stored = original.clone();
        stored["allowUnsignedApps"] = invalid;
        fs::write(&path, stored.to_string()).unwrap();
        assert!(read(&path).is_err());
    }
    assert!(configure(
        &path,
        json!({"action":"unsignedApps","allow_unsigned_apps":true})
    )
    .is_err());
    assert!(configure(&path, json!({"action":"unsignedApps"})).is_err());
}

#[test]
fn updates_and_rollback_cannot_add_unsigned_writes_while_policy_is_off() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("apps.json");
    install(&path, fake_core());
    let before = fs::read(&path).unwrap();
    assert!(install_writer(&path).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
    policy(&path, true).unwrap();
    let revision = install_writer(&path).unwrap().plugins[0].revision;
    install(&path, fake_core());
    policy(&path, false).unwrap();
    assert!(configure(&path,json!({"action":"rollback","id":"org.example.argocd","revision":revision,"grants":["k8s.listCustomResource","k8s.annotate"]})).is_err());
    assert!(read(&path).unwrap().plugins[0].enabled);
    policy(&path, true).unwrap();
    configure(&path,json!({"action":"rollback","id":"org.example.argocd","revision":revision,"grants":["k8s.listCustomResource","k8s.annotate"]})).unwrap();
    configure(
        &path,
        json!({"action":"settings","id":"org.example.argocd","settings":{"team":"platform"}}),
    )
    .unwrap();
    policy(&path, false).unwrap();
    let saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(saved["allowUnsignedApps"], false);
    assert_eq!(saved["plugins"][0]["enabled"], false);
    assert_eq!(saved["plugins"][0]["settings"]["team"], "platform");
    assert!(saved["plugins"][0].get("policyBlocked").is_none());
    assert!(!policy(&path, true).unwrap().plugins[0].enabled);
}
