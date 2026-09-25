//! Typed settings (#542) through the inventory: what a save accepts, what the
//! inventory file may hold, what survives an update, and what a request
//! sends. A secret's value reaches none of the inventory, the audit log or an
//! MCP client.
use super::tests::{configure, fake_core, manifest};
use super::*;
use serde_json::json;
use std::sync::Mutex;

const ID: &str = "org.example.argocd";
const SECRET: &str = "hunter2-very-secret";

/// The example app with one setting of each kind the tests need.
fn with_settings(settings: Value) -> String {
    let mut value: Value = serde_json::from_str(&manifest()).unwrap();
    // A secret setting needs the secret store's permission (#543).
    if settings
        .as_array()
        .is_some_and(|all| all.iter().any(|s| s["type"] == "secret-reference"))
    {
        value["permissions"] = json!(["k8s.listCustomResource", "extension.secretStore"]);
    }
    value["settings"] = settings;
    value.to_string()
}

fn declared() -> Value {
    json!([
        {"id":"team","type":"string","title":"Team"},
        {"id":"expiryWindowDays","type":"number","title":"Warn before expiry (days)","default":14,"minimum":1,"integer":true},
        {"id":"prometheusUrl","type":"url","title":"Prometheus URL","required":true},
        {"id":"token","type":"secret-reference","title":"API token"}
    ])
}

fn install(path: &Path, source: String) -> Inventory {
    // Granted exactly what the manifest requests.
    let grants = Manifest::parse(&source).unwrap().permissions;
    configure(
        path,
        json!({"action":"install","manifest":source,"grants":grants}),
    )
    .unwrap()
}

fn save(path: &Path, settings: Value) -> Result<Inventory, String> {
    configure(
        path,
        json!({"action":"settings","id":ID,"settings":settings}),
    )
}

fn on_disk(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
}

#[test]
fn saved_settings_are_held_to_the_manifest_declarations() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("apps.json");
    install(&path, with_settings(declared()));

    let saved = save(
        &path,
        json!({"team":"platform","prometheusUrl":"https://prom:9090"}),
    )
    .unwrap();
    assert_eq!(
        Value::Object(saved.plugins[0].settings.clone()),
        json!({"team":"platform","prometheusUrl":"https://prom:9090"})
    );

    // Everything the form would have stopped, and what it could not know.
    for (settings, path_at_fault) in [
        (json!({"team":"platform"}), "settings.prometheusUrl"),
        (
            json!({"prometheusUrl":"ftp://prom"}),
            "settings.prometheusUrl",
        ),
        (
            json!({"prometheusUrl":"https://prom","expiryWindowDays":0}),
            "settings.expiryWindowDays",
        ),
        (
            json!({"prometheusUrl":"https://prom","undeclared":"x"}),
            "settings.undeclared",
        ),
    ] {
        let refused = save(&path, settings.clone()).err().unwrap();
        assert!(refused.contains(path_at_fault), "{settings} → {refused}");
    }
    // A refused save changes nothing.
    assert_eq!(read(&path).unwrap().plugins[0].settings["team"], "platform");
}

#[test]
fn a_cleared_field_is_not_stored() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("apps.json");
    install(&path, with_settings(declared()));
    let saved = save(&path, json!({"team":"","prometheusUrl":"https://prom"})).unwrap();
    assert!(saved.plugins[0].settings.get("team").is_none());
}

#[test]
fn a_secret_setting_value_never_reaches_the_inventory_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("apps.json");
    install(&path, with_settings(declared()));
    for value in [json!(SECRET), json!({"secretRef": SECRET})] {
        let refused = save(&path, json!({"prometheusUrl":"https://prom","token":value}))
            .err()
            .unwrap();
        assert!(refused.contains("settings.token"), "{refused}");
        assert!(
            !refused.contains(SECRET),
            "the refusal repeated the secret: {refused}"
        );
    }
    assert!(!on_disk(&path).contains(SECRET));
}

#[test]
fn the_inventory_writer_refuses_a_secret_value_whatever_path_put_it_there() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("apps.json");
    install(&path, with_settings(declared()));
    let before = on_disk(&path);

    let mut state = read(&path).unwrap();
    state.plugins[0]
        .settings
        .insert("token".into(), json!(SECRET));
    let refused = write(&path, &state).err().unwrap();
    assert!(!refused.contains(SECRET), "{refused}");
    assert_eq!(on_disk(&path), before, "nothing was written");

    // Its own reference is the one thing a secret setting may hold.
    let mut state = read(&path).unwrap();
    state.plugins[0].settings.insert(
        "token".into(),
        srelens_plugin_host::secret_reference(ID, "token"),
    );
    write(&path, &state).unwrap();
}

#[tokio::test]
async fn a_hand_edited_secret_value_is_dropped_on_load_and_then_from_disk() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("apps.json");
    install(&path, with_settings(declared()));
    let mut raw: Value = serde_json::from_str(&on_disk(&path)).unwrap();
    raw["plugins"][0]["settings"] = json!({"team":"platform","token":SECRET});
    fs::write(&path, raw.to_string()).unwrap();

    let reg = {
        let mut reg = Registry::new();
        register(
            &mut reg,
            path.clone(),
            fake_core(),
            srelens_kube::client_cache::ClientCache::new_many(vec![]),
        );
        reg
    };
    let listed = reg.invoke("extensions.list", json!({})).await.unwrap();
    assert!(
        !listed.to_string().contains(SECRET),
        "extensions.list returned the secret"
    );
    assert_eq!(listed["plugins"][0]["settings"], json!({"team":"platform"}));

    configure(&path, json!({"action":"enable","id":ID,"enabled":false})).unwrap();
    assert!(
        !on_disk(&path).contains(SECRET),
        "the next save kept it on disk"
    );
}

#[test]
fn an_update_keeps_only_the_settings_the_new_version_accepts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("apps.json");
    let v1 = json!([
        {"id":"team","type":"string","title":"Team"},
        {"id":"token","type":"string","title":"Token"},
        {"id":"retired","type":"string","title":"Retired"}
    ]);
    install(&path, with_settings(v1.clone()));
    save(
        &path,
        json!({"team":"platform","token":SECRET,"retired":"x"}),
    )
    .unwrap();
    let revision = read(&path).unwrap().plugins[0].revision;

    // Version 2 makes the token a secret and drops `retired`.
    let mut v2: Value = serde_json::from_str(&with_settings(json!([
        {"id":"team","type":"string","title":"Team"},
        {"id":"token","type":"secret-reference","title":"Token"}
    ])))
    .unwrap();
    v2["version"] = json!("2.0.0");
    let updated = install(&path, v2.to_string());
    assert_eq!(
        Value::Object(updated.plugins[0].settings.clone()),
        json!({"team":"platform"})
    );
    assert!(
        !on_disk(&path).contains(SECRET),
        "the plaintext outlived its type change"
    );

    // Rolling back reconciles against the restored version the same way.
    let restored = configure(
        &path,
        json!({"action":"rollback","id":ID,"revision":revision,"grants":["k8s.listCustomResource"]}),
    )
    .unwrap();
    assert_eq!(
        Value::Object(restored.plugins[0].settings.clone()),
        json!({"team":"platform"})
    );
}

#[tokio::test]
async fn mcp_never_carries_a_secret_setting_into_a_response_or_the_audit_log() {
    #[derive(Default)]
    struct Spy(Mutex<Vec<srelens_capability::audit::AuditRecord>>);
    impl srelens_capability::audit::AuditSink for Spy {
        fn record(&self, rec: srelens_capability::audit::AuditRecord) {
            self.0.lock().unwrap().push(rec);
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("apps.json");
    install(&path, with_settings(declared()));
    let mut reg = Registry::new();
    register(
        &mut reg,
        path.clone(),
        fake_core(),
        srelens_kube::client_cache::ClientCache::new_many(vec![]),
    );
    let spy = Arc::new(Spy::default());
    let server = srelens_mcp::McpServer::new(Arc::new(reg))
        .with_policy(Arc::new(srelens_mcp::policy::FlagGated::new(true, false)))
        .with_audit(spy.clone());
    let call = |name: &str, arguments: Value| json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":arguments}});
    let refused = srelens_mcp::stdio::handle_request(
        &server,
        &call(
            "extensions.configure",
            json!({"action":"settings","id":ID,"_confirm":true,
                   "settings":{"prometheusUrl":"https://prom","token":SECRET}}),
        ),
        srelens_mcp::Transport::Stdio,
    )
    .await
    .unwrap();
    assert_eq!(refused["result"]["isError"], true, "{refused}");
    assert!(
        !refused.to_string().contains(SECRET),
        "the MCP response carried it: {refused}"
    );

    let listed = srelens_mcp::stdio::handle_request(
        &server,
        &call("extensions.list", json!({})),
        srelens_mcp::Transport::Stdio,
    )
    .await
    .unwrap();
    assert!(!listed.to_string().contains(SECRET));

    let records = spy.0.lock().unwrap();
    assert!(!records.is_empty());
    for record in records.iter() {
        let line = format!("{:?} {:?}", record.args, record.error);
        assert!(
            !line.contains(SECRET),
            "the audit record carried it: {line}"
        );
    }
    assert!(!on_disk(&path).contains(SECRET));
}

// ------------------------------------------------ interpolation, end to end

/// The Argo CD example with its Refresh action's annotation value taken from
/// a select setting, and an annotate primitive that answers with what it was
/// sent.
fn refresh_app(path: &Path, refresh_value: &str, settings: Value) -> (Arc<Registry>, u64) {
    try_refresh_app(path, refresh_value, settings).unwrap()
}

fn try_refresh_app(
    path: &Path,
    refresh_value: &str,
    settings: Value,
) -> Result<(Arc<Registry>, u64), String> {
    let mut core = crate::build_registry_with_paths(
        srelens_kube::client_cache::ClientCache::new_many(vec![]),
        vec![],
    );
    for id in ["k8s.getCustomResource", "k8s.annotate"] {
        let mut cap = core.get(id).unwrap().clone();
        cap.handler = Arc::new(|args| Box::pin(async move { Ok(args) }));
        core.register(cap);
    }
    super::tests::serve_crds(&mut core, &["applications.argoproj.io/v1alpha1"]);
    let core = Arc::new(core);
    let mut value: Value =
        serde_json::from_str(include_str!("../../../../examples/extensions/argocd.json")).unwrap();
    value["id"] = json!(ID);
    value["settings"] = settings;
    let refresh = value["actions"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|action| action["name"] == "refresh")
        .unwrap();
    refresh["arguments"]["value"] = json!(refresh_value);
    mutate(
        path,
        core.clone(),
        Configure::UnsignedApps {
            allow_unsigned_apps: true,
        },
    )?;
    let grants: Vec<String> = serde_json::from_value(value["permissions"].clone()).unwrap();
    let state = mutate(
        path,
        core.clone(),
        Configure::Install {
            signature: None,
            manifest: value.to_string(),
            grants,
            reviewed_revision: None,
        },
    )?;
    Ok((core, state.plugins[0].revision))
}

fn refresh_mode() -> Value {
    json!([{"id":"refreshMode","type":"select","title":"Refresh mode","default":"normal",
        "options":[{"value":"normal","label":"Normal"},{"value":"hard","label":"Hard"}]}])
}

async fn refresh(
    path: &Path,
    core: Arc<Registry>,
    revision: u64,
) -> Result<Value, CapabilityError> {
    let mut reg = Registry::new();
    resource::register(
        &mut reg,
        path.to_path_buf(),
        core,
        srelens_kube::client_cache::ClientCache::new_many(vec![]),
    );
    let binding = read(path).unwrap().plugins[0].manifest.capabilities[0]
        .name
        .clone();
    reg.invoke(
        "extensions.action",
        json!({"resource":{"id":ID,"revision":revision,"capability":binding,"context":"cluster/a","namespace":"team","name":"app"},
               "action":"refresh","uid":"u","resourceVersion":"2"}),
    )
    .await
}

#[tokio::test]
async fn an_action_sends_the_setting_saved_when_it_runs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("apps.json");
    let (core, revision) = refresh_app(&path, "${settings.refreshMode}", refresh_mode());
    let sent = refresh(&path, core.clone(), revision).await.unwrap();
    assert_eq!(sent["key"], "argocd.argoproj.io/refresh");
    assert_eq!(
        sent["value"], "normal",
        "the default while nothing is saved"
    );

    mutate(
        &path,
        core.clone(),
        Configure::Settings {
            id: ID.into(),
            settings: json!({"refreshMode":"hard"}).as_object().unwrap().clone(),
        },
    )
    .unwrap();
    let revision = read(&path).unwrap().plugins[0].revision;
    let sent = refresh(&path, core, revision).await.unwrap();
    assert_eq!(sent["value"], "hard");
}

#[test]
fn the_annotation_primitive_decides_what_setting_may_fill_its_value() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("apps.json");
    // A number is not a type `value` takes: refused at install.
    let refused = try_refresh_app(
        &path,
        "${settings.days}",
        json!([{"id":"days","type":"number","title":"Days","default":1}]),
    )
    .err()
    .unwrap();
    assert!(refused.contains("actions[0].arguments.value"), "{refused}");
    // Nor is `key`, which is not settable at all.
    let mut value: Value =
        serde_json::from_str(include_str!("../../../../examples/extensions/argocd.json")).unwrap();
    value["settings"] = refresh_mode();
    value["actions"][0]["arguments"]["key"] = json!("${settings.refreshMode}");
    let manifest = Manifest::parse(&value.to_string()).unwrap();
    let problems = validate_app(&manifest, &manifest.permissions.clone(), fake_core())
        .err()
        .unwrap();
    assert!(
        problems
            .0
            .iter()
            .any(|p| p.path == "actions[0].arguments.key"),
        "{problems}"
    );
}

#[test]
fn a_saved_value_the_primitive_would_refuse_is_refused_when_saved() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("apps.json");
    let (core, _) = refresh_app(
        &path,
        "${settings.note}",
        json!([{"id":"note","type":"string","title":"Note","default":"normal"}]),
    );
    // `k8s.annotate` refuses a value that looks like a token it does not know.
    let refused = mutate(
        &path,
        core,
        Configure::Settings {
            id: ID.into(),
            settings: json!({"note":"$bogus"}).as_object().unwrap().clone(),
        },
    )
    .err()
    .unwrap();
    assert!(refused.contains("actions[0].arguments"), "{refused}");
    // `k8s.annotate` quotes the value it refuses; the saved value is not repeated.
    assert!(!refused.contains("bogus"), "{refused}");
    assert!(refused.contains("is not a value this host substitutes"), "{refused}");
    assert!(read(&path).unwrap().plugins[0].settings.is_empty());
}

#[test]
fn the_access_review_names_the_setting_an_action_writes() {
    let manifest = |settings: Value| {
        let mut value: Value =
            serde_json::from_str(include_str!("../../../../examples/extensions/argocd.json"))
                .unwrap();
        value["settings"] = settings;
        value["actions"][0]["arguments"]["value"] = json!("${settings.refreshMode}");
        Manifest::parse(&value.to_string()).unwrap()
    };
    let before = manifest(refresh_mode());
    let mut widened = refresh_mode();
    widened[0]["options"]
        .as_array_mut()
        .unwrap()
        .push(json!({"value":"sync","label":"Sync"}));
    let after = manifest(widened);
    let grants = before.permissions.clone();
    let diff = permission_diff(Some((&before, &grants, 1)), &after, &grants);
    assert!(
        diff.added
            .iter()
            .any(|item| item.contains("refreshMode") && item.contains("sync")),
        "a setting that can now write another value is new access: {diff:?}"
    );
}
