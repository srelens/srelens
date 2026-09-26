//! An administrator's policy (#578), as the broker applies it: at install and on every
//! call, to apps installed before it changed as much as to new ones.
use super::tests::{fake_core, manifest};
use super::*;
use serde_json::json;
use srelens_kube::client_cache::ClientCache;

fn policy(value: Value) -> AppPolicy {
    AppPolicy::parse(&value.to_string()).unwrap()
}

/// An inventory file held to `policy`, as a web user's row is.
fn governed(dir: &Path, policy: &SharedPolicy) -> Apps {
    Apps::from(dir.join("extensions.json")).governed_by(policy.clone())
}

fn change(apps: &Apps, input: Value) -> Result<Inventory, String> {
    let input = serde_json::from_value::<Configure>(input).map_err(|e| e.to_string())?;
    configure(
        apps,
        fake_core(),
        &srelens_plugin_host::NoSecretStore,
        input,
    )
}

/// Why `result` was refused; a change that went through fails the test.
fn refused(result: Result<Inventory, String>) -> String {
    match result {
        Ok(_) => panic!("the change was allowed"),
        Err(reason) => reason,
    }
}

fn install_local(apps: &Apps) -> Result<Inventory, String> {
    change(
        apps,
        json!({"action":"install","manifest":manifest(),"grants":["k8s.listCustomResource"]}),
    )
}

/// The example app, with one write action.
fn writer() -> String {
    let mut value: Value = serde_json::from_str(&manifest()).unwrap();
    value["permissions"] = json!(["k8s.listCustomResource", "k8s.annotate"]);
    value["actions"] = json!([{"name":"refresh","title":"Refresh","target":"k8s.annotate","resource":"applications","arguments":{"key":"argocd.argoproj.io/refresh","value":"normal"}}]);
    value.to_string()
}

fn broker(apps: &Apps) -> Registry {
    let mut registry = Registry::new();
    register(
        &mut registry,
        apps.clone(),
        fake_core(),
        ClientCache::new_many(vec![]),
    );
    registry
}

/// Every call an app's views make, each naming `revision` of the example app.
fn calls(revision: u64) -> Vec<(&'static str, Value)> {
    let id = "org.example.argocd";
    let selected = json!({"id":id,"revision":revision,"capability":"applications","context":"cluster/a","namespace":"team","name":"app"});
    let scope = json!({"id":id,"revision":revision,"context":"cluster/a","namespace":"team","kind":"apps/Deployment",
        "resource":{"apiVersion":"apps/v1","kind":"Deployment","metadata":{"name":"web","namespace":"team"}}});
    vec![
        (
            "extensions.read",
            json!({"id":id,"revision":revision,"capability":"applications","context":"cluster/a"}),
        ),
        ("extensions.resource", selected.clone()),
        (
            "extensions.action",
            json!({"resource":selected,"action":"refresh","uid":"u","resourceVersion":"1"}),
        ),
        (
            "extensions.resolveColumns",
            json!({"id":id,"revision":revision,"context":"cluster/a","kind":"apps/Deployment","uids":[]}),
        ),
        (
            "extensions.resolveCards",
            json!({"id":id,"revision":revision,"context":"cluster/a"}),
        ),
        ("extensions.resolvePanels", scope.clone()),
        ("extensions.resolveLinks", scope),
    ]
}

async fn refused_everywhere(registry: &Registry, revision: u64, reason: &str) {
    for (id, payload) in calls(revision) {
        let error = registry.invoke(id, payload).await.unwrap_err().to_string();
        assert!(error.contains(reason), "{id}: {error}");
    }
}

#[test]
fn a_policy_is_checked_whole_and_what_it_leaves_out_is_allowed() {
    assert_eq!(policy(json!({})), AppPolicy::default());
    let listed = serde_json::to_value(AppPolicy::default()).unwrap();
    // Readable in full over the API: every field, with what it defaults to.
    assert_eq!(
        listed,
        json!({"allowedApps":null,"blockedApps":[],"allowedPublishers":null,"allowUnsignedApps":true,
            "allowedCapabilities":null,"allowWriteActions":true,"networkCeiling":[],
            "allowExecutableApps":false,"requiredApps":[]})
    );
    let written = json!({"allowedApps":["org.example.argocd"],"blockedApps":["org.example.other"],
        "allowedPublishers":["srelens"],"allowUnsignedApps":false,
        "allowedCapabilities":["k8s.annotate","k8s.listCustomResource","network.http"],
        "allowWriteActions":false,"networkCeiling":["api.example.com","*.corp.example.com:8443","[::1]"],
        "allowExecutableApps":false,"requiredApps":["org.example.argocd"]});
    assert_eq!(
        serde_json::to_value(policy(written.clone())).unwrap(),
        written
    );

    for (bad, says) in [
        (json!({"allowedapps":[]}), "unknown field"),
        (json!({"blockedApps":"org.example.app"}), "invalid type"),
        (
            json!({"blockedApps":["org.srelens.*"]}),
            "blockedApps: \"org.srelens.*\" is not an app ID",
        ),
        (
            json!({"allowedApps":["argocd"]}),
            "allowedApps: \"argocd\" is not an app ID",
        ),
        (
            json!({"allowedPublishers":["acme"]}),
            "\"acme\" is not a publisher this host trusts (srelens)",
        ),
        (
            json!({"allowedCapabilities":["k8s.listPods"]}),
            "\"k8s.listPods\" is not a capability an app can be granted",
        ),
        (
            json!({"allowedCapabilities":["k8s.deleteContext"]}),
            "is not a capability an app can be granted",
        ),
        (
            json!({"networkCeiling":["https://api.example.com"]}),
            "networkCeiling: \"https://api.example.com\" is not a host",
        ),
        (
            json!({"networkCeiling":["${settings.url}"]}),
            "is not a host",
        ),
        (json!({"allowExecutableApps":true}), "#521"),
        (
            json!({"blockedApps":["org.example.a"],"requiredApps":["org.example.a"]}),
            "requiredApps: org.example.a is also in blockedApps",
        ),
        (
            json!({"allowedApps":["org.example.a"],"requiredApps":["org.example.b"]}),
            "requiredApps: org.example.b is not in allowedApps",
        ),
    ] {
        let error = AppPolicy::parse(&bad.to_string()).unwrap_err();
        assert!(error.contains(says), "{bad}: {error}");
    }
    // Every problem at once, so an operator fixes the file in one pass.
    let error = AppPolicy::parse(
        &json!({"allowedApps":["x"],"allowedPublishers":["acme"],"allowExecutableApps":true})
            .to_string(),
    )
    .unwrap_err();
    assert_eq!(error.lines().count(), 3, "{error}");
    assert!(AppPolicy::parse(&" ".repeat(MAX_POLICY_BYTES + 1))
        .unwrap_err()
        .contains("1 MiB"));
    // A policy cannot be built past its checks by deserializing one some other way.
    assert!(serde_json::from_value::<AppPolicy>(json!({"allowExecutableApps":true})).is_err());
}

#[tokio::test]
async fn a_blocked_app_can_be_neither_installed_nor_called() {
    let dir = tempfile::tempdir().unwrap();
    let rules = SharedPolicy::new(policy(json!({"blockedApps":["org.example.argocd"]})));
    let apps = governed(dir.path(), &rules);
    let error = refused(install_local(&apps));
    assert!(
        error.contains("The administrator's policy blocks org.example.argocd"),
        "{error}"
    );
    let registry = broker(&apps);
    let report = registry
        .invoke(
            "extensions.validate",
            json!({"manifest":manifest(),"grants":["k8s.listCustomResource"]}),
        )
        .await
        .unwrap();
    assert_eq!(
        report["errors"][0]["code"], "EXTENSION_POLICY_REFUSED",
        "{report}"
    );
    assert_eq!(report["errors"][0]["path"], "id", "{report}");
    assert!(read(&apps.inventory).unwrap().plugins.is_empty());

    // Installed before the block, it is refused from its next call on: nothing is
    // rebuilt, and the saved inventory is not touched.
    rules.replace(AppPolicy::default());
    let revision = install_local(&apps).unwrap().plugins[0].revision;
    registry
        .invoke("extensions.read", calls(revision)[0].1.clone())
        .await
        .unwrap();
    let saved = fs::read(dir.path().join("extensions.json")).unwrap();
    rules.replace(policy(json!({"blockedApps":["org.example.argocd"]})));
    let listed = registry.invoke("extensions.list", json!({})).await.unwrap();
    assert_eq!(listed["plugins"][0]["enabled"], false);
    assert_eq!(
        listed["plugins"][0]["policyBlocked"],
        "The administrator's policy blocks org.example.argocd"
    );
    assert_eq!(
        listed["policy"]["blockedApps"],
        json!(["org.example.argocd"])
    );
    refused_everywhere(&registry, revision, "policy blocks org.example.argocd").await;
    let error = refused(change(
        &apps,
        json!({"action":"enable","id":"org.example.argocd","enabled":true}),
    ));
    assert!(error.contains("policy blocks"), "{error}");
    assert_eq!(fs::read(dir.path().join("extensions.json")).unwrap(), saved);

    // A change to it that is not a use is still the user's: it is saved as they left
    // it, and never saved disabled on the policy's account.
    let changed = change(
        &apps,
        json!({"action":"settings","id":"org.example.argocd","settings":{"team":"platform"}}),
    )
    .unwrap();
    assert!(changed.plugins[0].policy_blocked.is_some());
    let stored: Value =
        serde_json::from_slice(&fs::read(dir.path().join("extensions.json")).unwrap()).unwrap();
    assert_eq!(stored["plugins"][0]["enabled"], true);
    assert!(stored.get("policy").is_none());
    assert!(stored["plugins"][0].get("policyBlocked").is_none());

    // Lifting the block brings it back as its user left it.
    rules.replace(AppPolicy::default());
    registry
        .invoke("extensions.read", calls(revision)[0].1.clone())
        .await
        .unwrap();
}

#[tokio::test]
async fn a_capability_outside_the_policy_is_refused_at_call_time_for_an_app_installed_before_it() {
    let dir = tempfile::tempdir().unwrap();
    let rules = SharedPolicy::new(AppPolicy::default());
    let apps = governed(dir.path(), &rules);
    let revision = install_local(&apps).unwrap().plugins[0].revision;
    let registry = broker(&apps);
    registry
        .invoke("extensions.read", calls(revision)[0].1.clone())
        .await
        .unwrap();

    rules.replace(policy(json!({"allowedCapabilities":["k8s.listEvents"]})));
    refused_everywhere(
        &registry,
        revision,
        "The administrator's policy does not allow k8s.listCustomResource",
    )
    .await;
    // And a fresh registry over the same inventory, as after a restart, agrees.
    refused_everywhere(
        &broker(&apps),
        revision,
        "does not allow k8s.listCustomResource",
    )
    .await;
    let error = refused(install_local(&apps));
    assert!(
        error.contains("does not allow k8s.listCustomResource"),
        "{error}"
    );
    let report = registry
        .invoke(
            "extensions.validate",
            json!({"manifest":manifest(),"grants":["k8s.listCustomResource"]}),
        )
        .await
        .unwrap();
    assert_eq!(report["errors"][0]["path"], "permissions", "{report}");
}

#[tokio::test]
async fn an_app_that_writes_is_refused_whole_once_the_policy_turns_writes_off() {
    let dir = tempfile::tempdir().unwrap();
    let rules = SharedPolicy::new(AppPolicy::default());
    let apps = governed(dir.path(), &rules);
    change(
        &apps,
        json!({"action":"unsignedApps","allowUnsignedApps":true}),
    )
    .unwrap();
    let grants = json!(["k8s.listCustomResource", "k8s.annotate"]);
    let revision = change(
        &apps,
        json!({"action":"install","manifest":writer(),"grants":grants}),
    )
    .unwrap()
    .plugins[0]
        .revision;
    let registry = broker(&apps);
    registry
        .invoke("extensions.read", calls(revision)[0].1.clone())
        .await
        .unwrap();

    rules.replace(policy(json!({"allowWriteActions":false})));
    refused_everywhere(
        &registry,
        revision,
        "The administrator's policy does not allow apps that write to clusters",
    )
    .await;
    // Naming the action primitive in the capability list refuses it the same way.
    rules.replace(policy(
        json!({"allowedCapabilities":["k8s.listCustomResource"]}),
    ));
    refused_everywhere(&registry, revision, "does not allow k8s.annotate").await;
    // The read-only version of the same app is allowed.
    rules.replace(policy(json!({"allowWriteActions":false})));
    change(
        &apps,
        json!({"action":"install","manifest":manifest(),"grants":["k8s.listCustomResource"],"reviewedRevision":revision}),
    )
    .unwrap();
}

#[tokio::test]
async fn publishers_and_unsigned_apps_are_the_policys_to_allow() {
    let dir = tempfile::tempdir().unwrap();
    let rules = SharedPolicy::new(policy(json!({"allowUnsignedApps":false})));
    let apps = governed(dir.path(), &rules);
    let error = refused(install_local(&apps));
    assert!(error.contains("allows only signed apps"), "{error}");
    // Published bytes, signed by srelens.
    let signed = json!({"action":"install",
        "manifest":include_str!("../../tests/fixtures/argocd-0.3.0-manifest.json"),
        "signature":include_bytes!("../../tests/fixtures/argocd-0.3.0-manifest.sig").to_vec(),
        "grants":["k8s.listCustomResource","k8s.annotate","k8s.mergePatch"]});
    rules.replace(policy(
        json!({"allowUnsignedApps":false,"allowedPublishers":[]}),
    ));
    let error = refused(change(&apps, signed.clone()));
    assert!(
        error.contains("does not allow apps signed by srelens"),
        "{error}"
    );
    rules.replace(policy(
        json!({"allowUnsignedApps":false,"allowedPublishers":["srelens"]}),
    ));
    let state = change(&apps, signed).unwrap();
    assert!(state.plugins[0].enabled && state.plugins[0].policy_blocked.is_none());

    // A signature that no longer verifies is no signature, to the policy as to
    // everything else: the app is refused as unsigned, not as srelens's.
    let path = dir.path().join("extensions.json");
    let mut stored: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    stored["plugins"][0]["signatureProof"]["signature"][0] = json!(0);
    fs::write(&path, stored.to_string()).unwrap();
    let state = read(&apps.inventory).unwrap();
    assert!(state.plugins[0].quarantined.is_some());
    assert_eq!(
        state.plugins[0].policy_blocked.as_deref(),
        Some("The administrator's policy allows only signed apps")
    );
}

#[test]
fn only_the_allowed_apps_install_and_a_required_one_stays() {
    let dir = tempfile::tempdir().unwrap();
    let rules = SharedPolicy::new(policy(json!({"allowedApps":["org.example.other"]})));
    let apps = governed(dir.path(), &rules);
    let error = refused(install_local(&apps));
    assert!(
        error.contains("does not allow org.example.argocd"),
        "{error}"
    );

    rules.replace(policy(json!({"requiredApps":["org.example.argocd"]})));
    let state = install_local(&apps).unwrap();
    let required = state.policy.as_ref().unwrap();
    assert!(required.requires("org.example.argocd"));
    for (action, says) in [
        (
            json!({"action":"remove","id":"org.example.argocd"}),
            "can't be removed",
        ),
        (
            json!({"action":"enable","id":"org.example.argocd","enabled":false}),
            "can't be disabled",
        ),
    ] {
        let error = refused(change(&apps, action));
        assert!(
            error.contains("The administrator's policy requires org.example.argocd")
                && error.contains(says),
            "{error}"
        );
    }
    // Only an app that is there is kept: one not installed says so.
    let error = refused(change(
        &apps,
        json!({"action":"remove","id":"org.example.missing"}),
    ));
    assert!(error.contains("not installed"), "{error}");
    // Once the policy no longer requires it, it goes like any other app.
    rules.replace(AppPolicy::default());
    change(&apps, json!({"action":"remove","id":"org.example.argocd"})).unwrap();
}

#[test]
fn a_rollback_to_a_version_the_policy_refuses_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let rules = SharedPolicy::new(AppPolicy::default());
    let apps = governed(dir.path(), &rules);
    change(
        &apps,
        json!({"action":"unsignedApps","allowUnsignedApps":true}),
    )
    .unwrap();
    let grants = json!(["k8s.listCustomResource", "k8s.annotate"]);
    let revision = change(
        &apps,
        json!({"action":"install","manifest":writer(),"grants":grants}),
    )
    .unwrap()
    .plugins[0]
        .revision;
    change(
        &apps,
        json!({"action":"install","manifest":manifest(),"grants":["k8s.listCustomResource"],"reviewedRevision":revision}),
    )
    .unwrap();
    rules.replace(policy(json!({"allowWriteActions":false})));
    let error = refused(change(
        &apps,
        json!({"action":"rollback","id":"org.example.argocd","revision":revision,"grants":grants}),
    ));
    assert!(error.contains("does not allow apps that write"), "{error}");
}

#[tokio::test]
async fn under_a_policy_an_app_cannot_open_plain_http_to_the_host() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("extensions.json");
    let mut metrics: Value = serde_json::from_str(&manifest()).unwrap();
    metrics["permissions"] =
        json!(["k8s.listCustomResource", {"capability":"network.http","hosts":["127.0.0.1:9090"]}]);
    metrics["capabilities"].as_array_mut().unwrap().push(json!({"name":"up","title":"Up",
        "target":"network.http","arguments":{"url":"http://127.0.0.1:9090/api/v1/query"},"inputs":[]}));
    let grants = json!(["k8s.listCustomResource", "network.http"]);
    let install = json!({"action":"install","manifest":metrics.to_string(),"grants":grants});
    // Allowed on the desktop, where this computer is the person's...
    let desktop = Apps::from(path.clone());
    change(&desktop, install).unwrap();
    change(
        &desktop,
        json!({"action":"loopbackHttp","id":"org.example.argocd","allowLoopbackHttp":true}),
    )
    .unwrap();
    // ...and never under a policy, where it is the host every user shares, whatever
    // the saved inventory says.
    let rules = SharedPolicy::new(policy(json!({"networkCeiling":["127.0.0.1:9090"]})));
    let apps = governed(dir.path(), &rules);
    let state = read(&apps.inventory).unwrap();
    assert!(!state.plugins[0].allow_loopback_http);
    assert!(state.plugins[0].policy_blocked.is_none());
    let error = refused(change(
        &apps,
        json!({"action":"loopbackHttp","id":"org.example.argocd","allowLoopbackHttp":true}),
    ));
    assert!(error.contains("over HTTPS only"), "{error}");
    let revision = state.plugins[0].revision;
    let error = broker(&apps)
        .invoke("extensions.read", json!({"id":"org.example.argocd","revision":revision,"capability":"up","context":"cluster/a"}))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("over HTTPS only"), "{error}");
}

#[tokio::test]
async fn network_http_under_a_policy_reaches_only_hosts_its_ceiling_allows() {
    let dir = tempfile::tempdir().unwrap();
    // A port nothing listens on, so a request the policy lets through fails to connect
    // instead of reaching anything.
    let closed = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let allowed = format!("127.0.0.1:{}", closed.port());
    let mut metrics: Value = serde_json::from_str(&manifest()).unwrap();
    metrics["permissions"] = json!(["k8s.listCustomResource",
        {"capability":"network.http","hosts":[allowed, "metrics.example.invalid"]}]);
    for (name, url) in [
        ("inside", format!("https://{allowed}/api")),
        ("outside", "https://metrics.example.invalid/api".to_owned()),
    ] {
        metrics["capabilities"].as_array_mut().unwrap().push(
            json!({"name":name,"title":name,"target":"network.http","arguments":{"url":url},"inputs":[]}),
        );
    }
    let grants = json!(["k8s.listCustomResource", "network.http"]);
    let install = json!({"action":"install","manifest":metrics.to_string(),"grants":grants});

    // With no ceiling, network.http reaches no host: nothing to install.
    let rules = SharedPolicy::new(AppPolicy::default());
    let apps = governed(dir.path(), &rules);
    let error = refused(change(&apps, install.clone()));
    assert!(error.contains("lets network.http reach no host"), "{error}");

    rules.replace(policy(json!({"networkCeiling":[allowed]})));
    let revision = change(&apps, install).unwrap().plugins[0].revision;
    let registry = broker(&apps);
    let read = |name: &str| json!({"id":"org.example.argocd","revision":revision,"capability":name,"context":"cluster/a"});
    let error = registry
        .invoke("extensions.read", read("outside"))
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("outside the hosts the administrator's policy lets network.http reach"),
        "{error}"
    );
    // A host inside the ceiling gets past the policy, to a connection that fails.
    let error = registry
        .invoke("extensions.read", read("inside"))
        .await
        .unwrap_err()
        .to_string();
    assert!(!error.contains("administrator's policy"), "{error}");

    // Emptying the ceiling afterwards refuses the app whole, from its next call.
    rules.replace(AppPolicy::default());
    let error = registry
        .invoke("extensions.read", read("inside"))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("lets network.http reach no host"), "{error}");
}

#[test]
fn a_saved_inventory_carries_no_policy_of_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("extensions.json");
    let desktop = Apps::from(path.clone());
    install_local(&desktop).unwrap();
    // The desktop has no policy, and reports none.
    assert!(read(&desktop.inventory).unwrap().policy.is_none());
    assert!(serde_json::to_value(read(&path).unwrap())
        .unwrap()
        .get("policy")
        .is_none());
    // A policy written into the file is not one: the inventory is refused, as any
    // field the host does not save is.
    let mut stored: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    stored["policy"] = serde_json::to_value(AppPolicy::default()).unwrap();
    fs::write(&path, stored.to_string()).unwrap();
    assert!(read(&path).is_err());
}

/// Whatever path a future write takes, an inventory a policy was applied to is never
/// saved: its verdicts are the policy's, not the user's.
#[test]
fn an_inventory_held_to_a_policy_is_never_saved() {
    let dir = tempfile::tempdir().unwrap();
    let rules = SharedPolicy::new(AppPolicy::default());
    let apps = governed(dir.path(), &rules);
    install_local(&apps).unwrap();
    let before = fs::read(dir.path().join("extensions.json")).unwrap();
    let governed_state = read(&apps.inventory).unwrap();
    let error = write(&apps.inventory, &governed_state).unwrap_err();
    assert!(error.contains("held to a policy"), "{error}");
    assert_eq!(
        fs::read(dir.path().join("extensions.json")).unwrap(),
        before
    );
    // The inventory as saved still saves.
    write(&apps.inventory, &read_saved(&apps.inventory).unwrap()).unwrap();
}

/// A required app is disabled by nothing the user does: turning off their own switch
/// for unsigned apps that write would disable a required one, so it is refused.
#[test]
fn the_unsigned_switch_cannot_disable_a_required_app() {
    let dir = tempfile::tempdir().unwrap();
    let rules = SharedPolicy::new(policy(json!({"requiredApps":["org.example.argocd"]})));
    let apps = governed(dir.path(), &rules);
    change(
        &apps,
        json!({"action":"unsignedApps","allowUnsignedApps":true}),
    )
    .unwrap();
    let grants = json!(["k8s.listCustomResource", "k8s.annotate"]);
    change(
        &apps,
        json!({"action":"install","manifest":writer(),"grants":grants}),
    )
    .unwrap();
    let error = refused(change(
        &apps,
        json!({"action":"unsignedApps","allowUnsignedApps":false}),
    ));
    assert!(
        error.contains(
            "The administrator's policy requires org.example.argocd, so it can't be disabled"
        ),
        "{error}"
    );
    assert!(read(&apps.inventory).unwrap().plugins[0].enabled);
    // Without the requirement, the switch is the user's again.
    rules.replace(AppPolicy::default());
    let state = change(
        &apps,
        json!({"action":"unsignedApps","allowUnsignedApps":false}),
    )
    .unwrap();
    assert!(!state.plugins[0].enabled);
}
