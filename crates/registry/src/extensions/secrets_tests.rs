//! Secret settings (#543) through the registry: `extension.secretStore` sets,
//! clears and reports a secret, the value lives only in the host's store,
//! and every exit — the inventory, `extensions.list`, the capability's own
//! answer, errors, MCP and the audit log — carries the reference at most.
use super::tests::{fake_core, manifest};
use super::*;
use serde_json::json;
use srelens_plugin_host::{secret_key, secret_reference, SecretStore, SecretValue, SECRET_STORE_PERMISSION};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

const ID: &str = "org.example.argocd";
const SECRET: &str = "dd-api-7c1e4f-hunter2";

/// The host's store as the unlocked desktop vault behaves, with a switch for
/// the states in which it must refuse (no keychain, locked).
#[derive(Default)]
struct MemoryStore {
    values: Mutex<BTreeMap<String, String>>,
    unavailable: Mutex<Option<String>>,
    /// `retain` fails while set, as a vault locked mid-session would.
    retain_fails: Mutex<bool>,
}

impl MemoryStore {
    fn keys(&self) -> Vec<String> {
        self.values.lock().unwrap().keys().cloned().collect()
    }
    fn holds_value(&self) -> bool {
        self.values.lock().unwrap().values().any(|v| v == SECRET)
    }
}

impl SecretStore for MemoryStore {
    fn status(&self) -> Result<(), String> {
        match &*self.unavailable.lock().unwrap() {
            Some(why) => Err(why.clone()),
            None => Ok(()),
        }
    }
    fn put(&self, key: &str, value: &SecretValue) -> Result<(), String> {
        self.status()?;
        self.values.lock().unwrap().insert(key.into(), value.expose().into());
        Ok(())
    }
    fn contains(&self, key: &str) -> Result<bool, String> {
        Ok(self.values.lock().unwrap().contains_key(key))
    }
    fn retain(&self, keep: &BTreeSet<String>) -> Result<(), String> {
        if *self.retain_fails.lock().unwrap() {
            return Err("the vault is locked".into());
        }
        self.values.lock().unwrap().retain(|key, _| keep.contains(key));
        Ok(())
    }
    fn reveal(&self, key: &str) -> Result<Option<SecretValue>, String> {
        Ok(self.values.lock().unwrap().get(key).cloned().map(SecretValue::new))
    }
}

fn with_secret(settings: Value) -> String {
    let mut value: Value = serde_json::from_str(&manifest()).unwrap();
    value["settings"] = settings;
    value["permissions"] = json!(["k8s.listCustomResource", SECRET_STORE_PERMISSION]);
    value.to_string()
}

fn declared() -> Value {
    json!([
        {"id":"team","type":"string","title":"Team"},
        {"id":"token","type":"secret-reference","title":"API token"},
        {"id":"webhook","type":"secret-reference","title":"Webhook secret"}
    ])
}

fn grants() -> Value {
    json!(["k8s.listCustomResource", SECRET_STORE_PERMISSION])
}

fn registry(path: &Path, store: Arc<MemoryStore>) -> Registry {
    let mut reg = Registry::new();
    register_with_secrets(
        &mut reg,
        path.to_path_buf(),
        fake_core(),
        srelens_kube::client_cache::ClientCache::new_many(vec![]),
        store,
    );
    reg
}

async fn install(reg: &Registry, source: String) -> Value {
    let reviewed = reg.invoke("extensions.list", json!({})).await.unwrap()["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|app| app["manifest"]["id"] == ID)
        .map(|app| app["revision"].clone());
    let mut input = json!({"action":"install","manifest":source,"grants":grants()});
    if let Some(revision) = reviewed {
        input["reviewedRevision"] = revision;
    }
    reg.invoke("extensions.configure", input).await.unwrap()
}

async fn set(reg: &Registry, setting: &str) -> Result<Value, CapabilityError> {
    reg.invoke(
        "extension.secretStore",
        json!({"action":"set","id":ID,"setting":setting,"secret":SECRET}),
    )
    .await
}

async fn listed(reg: &Registry) -> Value {
    reg.invoke("extensions.list", json!({})).await.unwrap()
}

fn on_disk(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
}

#[tokio::test]
async fn set_keeps_the_value_in_the_store_and_only_the_reference_in_the_inventory() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("apps.json");
    let store = Arc::new(MemoryStore::default());
    let reg = registry(&path, store.clone());
    install(&reg, with_secret(declared())).await;

    let answer = set(&reg, "token").await.unwrap();
    assert_eq!(answer, json!({"set": true}), "write-only: whether it is set, nothing else");
    assert_eq!(
        store.reveal(&secret_key(ID, "token")).unwrap().unwrap().expose(),
        SECRET
    );
    assert!(!on_disk(&path).contains(SECRET), "the inventory file holds the value");
    let listed = listed(&reg).await;
    assert!(!listed.to_string().contains(SECRET), "extensions.list returned it");
    assert_eq!(listed["plugins"][0]["settings"]["token"], secret_reference(ID, "token"));
    assert_eq!(listed["secretStore"], json!({"available": true}));
}

#[tokio::test]
async fn clear_deletes_the_value_and_the_reference() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("apps.json");
    let store = Arc::new(MemoryStore::default());
    let reg = registry(&path, store.clone());
    install(&reg, with_secret(declared())).await;
    set(&reg, "token").await.unwrap();
    set(&reg, "webhook").await.unwrap();

    let answer = reg
        .invoke("extension.secretStore", json!({"action":"clear","id":ID,"setting":"token"}))
        .await
        .unwrap();
    assert_eq!(answer, json!({"set": false}));
    assert_eq!(store.keys(), vec![secret_key(ID, "webhook")], "only the one named");
    assert!(listed(&reg).await["plugins"][0]["settings"].get("token").is_none());

    // With no setting named, every secret the app keeps: what a reset does.
    reg.invoke("extension.secretStore", json!({"action":"clear","id":ID}))
        .await
        .unwrap();
    assert!(store.keys().is_empty(), "{:?}", store.keys());
    assert!(listed(&reg).await["plugins"][0]["settings"].get("webhook").is_none());
}

#[tokio::test]
async fn removing_an_app_deletes_its_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("apps.json");
    let store = Arc::new(MemoryStore::default());
    let reg = registry(&path, store.clone());
    install(&reg, with_secret(declared())).await;
    set(&reg, "token").await.unwrap();

    reg.invoke("extensions.configure", json!({"action":"remove","id":ID}))
        .await
        .unwrap();
    assert!(store.keys().is_empty(), "the removed app's secret outlived it");
}

#[tokio::test]
async fn a_removal_the_store_cannot_follow_yet_is_swept_by_the_next_change() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("apps.json");
    let store = Arc::new(MemoryStore::default());
    let reg = registry(&path, store.clone());
    install(&reg, with_secret(declared())).await;
    set(&reg, "token").await.unwrap();

    // Locked when the app is removed: the removal still happens.
    *store.retain_fails.lock().unwrap() = true;
    reg.invoke("extensions.configure", json!({"action":"remove","id":ID}))
        .await
        .unwrap();
    assert!(listed(&reg).await["plugins"].as_array().unwrap().is_empty());
    // Unreferenced, so nothing can reach it; the next change deletes it.
    *store.retain_fails.lock().unwrap() = false;
    reg.invoke(
        "extensions.configure",
        json!({"action":"unsignedApps","allowUnsignedApps":false}),
    )
    .await
    .unwrap();
    assert!(store.keys().is_empty(), "{:?}", store.keys());
}

#[tokio::test]
async fn an_update_that_drops_a_secret_or_changes_its_type_deletes_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("apps.json");
    let store = Arc::new(MemoryStore::default());
    let reg = registry(&path, store.clone());
    install(&reg, with_secret(declared())).await;
    set(&reg, "token").await.unwrap();
    set(&reg, "webhook").await.unwrap();
    let revision = listed(&reg).await["plugins"][0]["revision"].clone();

    // Version 2 turns `token` into a string and drops `webhook`.
    let mut v2: Value = serde_json::from_str(&with_secret(json!([
        {"id":"team","type":"string","title":"Team"},
        {"id":"token","type":"string","title":"API token"},
        {"id":"other","type":"secret-reference","title":"Other"}
    ])))
    .unwrap();
    v2["version"] = json!("2.0.0");
    install(&reg, v2.to_string()).await;
    let after = listed(&reg).await;
    assert!(
        after["plugins"][0]["settings"].get("token").is_none(),
        "a reference must not become a string value: {after}"
    );
    assert!(store.keys().is_empty(), "{:?}", store.keys());

    // Rolling back does not bring the secrets back: they were deleted.
    reg.invoke(
        "extensions.configure",
        json!({"action":"rollback","id":ID,"revision":revision,"grants":grants()}),
    )
    .await
    .unwrap();
    let restored = listed(&reg).await;
    assert!(restored["plugins"][0]["settings"].get("token").is_none());
    assert!(!store.holds_value());
}

#[tokio::test]
async fn a_rollback_to_a_version_without_the_secret_deletes_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("apps.json");
    let store = Arc::new(MemoryStore::default());
    let reg = registry(&path, store.clone());
    // Version 1 has no secret; version 2 adds one and it is set.
    install(&reg, manifest()).await.to_string();
    let revision = listed(&reg).await["plugins"][0]["revision"].clone();
    let mut v2: Value = serde_json::from_str(&with_secret(declared())).unwrap();
    v2["version"] = json!("2.0.0");
    install(&reg, v2.to_string()).await;
    set(&reg, "token").await.unwrap();

    reg.invoke(
        "extensions.configure",
        json!({"action":"rollback","id":ID,"revision":revision,"grants":["k8s.listCustomResource"]}),
    )
    .await
    .unwrap();
    assert!(store.keys().is_empty(), "{:?}", store.keys());
}

#[tokio::test]
async fn setting_a_secret_needs_the_secret_store_grant() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("apps.json");
    let store = Arc::new(MemoryStore::default());
    let reg = registry(&path, store.clone());
    install(&reg, with_secret(declared())).await;
    // An inventory edited by hand, or written by a host with a bug.
    let mut raw: Value = serde_json::from_str(&on_disk(&path)).unwrap();
    raw["plugins"][0]["grants"] = json!(["k8s.listCustomResource"]);
    fs::write(&path, raw.to_string()).unwrap();

    let refused = set(&reg, "token").await.unwrap_err().to_string();
    assert!(refused.contains(SECRET_STORE_PERMISSION), "{refused}");
    assert!(!refused.contains(SECRET), "{refused}");
    assert!(store.keys().is_empty());
    assert!(!on_disk(&path).contains("secretRef"));
}

#[tokio::test]
async fn only_a_declared_secret_of_an_installed_app_can_be_set() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("apps.json");
    let store = Arc::new(MemoryStore::default());
    let reg = registry(&path, store.clone());
    install(&reg, with_secret(declared())).await;
    for (id, setting) in [(ID, "team"), (ID, "undeclared"), ("org.example.none", "token")] {
        let refused = reg
            .invoke(
                "extension.secretStore",
                json!({"action":"set","id":id,"setting":setting,"secret":SECRET}),
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(!refused.contains(SECRET), "{refused}");
    }
    assert!(store.keys().is_empty());
}

#[tokio::test]
async fn a_secret_must_be_one_bounded_piece_of_text() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("apps.json");
    let store = Arc::new(MemoryStore::default());
    let reg = registry(&path, store.clone());
    install(&reg, with_secret(declared())).await;
    let long = "x".repeat(16 * 1024 + 1);
    for secret in [json!(""), json!(long), json!("a\u{0}b"), json!(123456789), json!(["x"])] {
        let refused = reg
            .invoke(
                "extension.secretStore",
                json!({"action":"set","id":ID,"setting":"token","secret":secret}),
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(!refused.contains("123456789"), "the refusal echoed the value: {refused}");
        assert!(refused.len() < 400, "the refusal echoed the value: {} bytes", refused.len());
    }
    assert!(store.keys().is_empty());
}

/// The desktop bridge logs a failed call's message (`bridge.rs`), and serde's
/// own messages quote what they could not read. Whatever shape a caller
/// sends, the refusal must not carry a value from it.
#[tokio::test]
async fn a_malformed_call_is_refused_without_quoting_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("apps.json");
    let store = Arc::new(MemoryStore::default());
    let reg = registry(&path, store.clone());
    install(&reg, with_secret(declared())).await;
    for input in [
        json!(SECRET),
        json!([SECRET]),
        json!({"action": SECRET}),
        json!({"action":"set","id":ID,"setting":"token","secret":SECRET,"extra":SECRET}),
        json!({"action":"set","id":ID,"setting":{"x":SECRET},"secret":SECRET}),
        json!({"action":"clear","id":ID,"setting":"token","secret":SECRET}),
    ] {
        let refused = reg
            .invoke("extension.secretStore", input.clone())
            .await
            .unwrap_err()
            .to_string();
        assert!(!refused.contains(SECRET), "{input} → {refused}");
    }
    assert!(store.keys().is_empty());
}

#[tokio::test]
async fn an_unavailable_store_fails_closed_and_says_why() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("apps.json");
    let store = Arc::new(MemoryStore::default());
    let reg = registry(&path, store.clone());
    install(&reg, with_secret(declared())).await;
    *store.unavailable.lock().unwrap() =
        Some("the secrets vault is locked — unlock it with your master password".into());

    let refused = set(&reg, "token").await.unwrap_err().to_string();
    assert!(refused.contains("locked"), "{refused}");
    assert!(!refused.contains(SECRET), "{refused}");
    assert!(store.keys().is_empty());
    let disk = on_disk(&path);
    assert!(!disk.contains(SECRET) && !disk.contains("secretRef"), "{disk}");
    let listed = listed(&reg).await;
    assert_eq!(listed["secretStore"]["available"], false);
    assert!(listed["secretStore"]["reason"].as_str().unwrap().contains("locked"));
    // The state is reported, never saved to disk.
    let saved: Value = serde_json::from_str(&on_disk(&path)).unwrap();
    assert!(saved.get("secretStore").is_none(), "{saved}");
}

#[tokio::test]
async fn a_reference_whose_value_is_gone_reads_as_not_set() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("apps.json");
    let store = Arc::new(MemoryStore::default());
    let reg = registry(&path, store.clone());
    install(&reg, with_secret(declared())).await;
    set(&reg, "token").await.unwrap();
    // The vault was replaced, reset or restored from elsewhere.
    store.values.lock().unwrap().clear();
    assert!(listed(&reg).await["plugins"][0]["settings"].get("token").is_none());

    // While the store cannot be asked, the reference is what is known.
    set(&reg, "token").await.unwrap();
    store.values.lock().unwrap().clear();
    *store.unavailable.lock().unwrap() = Some("locked".into());
    assert_eq!(
        listed(&reg).await["plugins"][0]["settings"]["token"],
        secret_reference(ID, "token")
    );
}

#[tokio::test]
async fn neither_mcp_nor_the_audit_log_ever_carries_the_value() {
    #[derive(Default)]
    struct Spy(Mutex<Vec<srelens_capability::audit::AuditRecord>>);
    impl srelens_capability::audit::AuditSink for Spy {
        fn record(&self, rec: srelens_capability::audit::AuditRecord) {
            self.0.lock().unwrap().push(rec);
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("apps.json");
    let store = Arc::new(MemoryStore::default());
    let reg = registry(&path, store.clone());
    install(&reg, with_secret(declared())).await;
    let spy = Arc::new(Spy::default());
    let server = srelens_mcp::McpServer::new(Arc::new(reg))
        .with_policy(Arc::new(srelens_mcp::policy::FlagGated::new(true, true)))
        .with_audit(spy.clone());
    let call = |arguments: Value| json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"extension.secretStore","arguments":arguments}});
    let handle = |request: Value| {
        let server = &server;
        async move {
            srelens_mcp::stdio::handle_request(server, &request, srelens_mcp::Transport::Stdio)
                .await
                .unwrap()
        }
    };
    let stored = handle(call(json!({"action":"set","id":ID,"setting":"token","secret":SECRET,"_confirm":true}))).await;
    assert_eq!(stored["result"]["isError"], false, "{stored}");
    // Refused calls quote nothing either: unconfirmed, and with the store locked.
    let unconfirmed = handle(call(json!({"action":"set","id":ID,"setting":"token","secret":SECRET}))).await;
    *store.unavailable.lock().unwrap() = Some("locked".into());
    let locked = handle(call(json!({"action":"set","id":ID,"setting":"token","secret":SECRET,"_confirm":true}))).await;
    for answer in [&stored, &unconfirmed, &locked] {
        assert!(!answer.to_string().contains(SECRET), "MCP answered with it: {answer}");
    }
    let records = spy.0.lock().unwrap();
    assert!(records.len() >= 3, "{}", records.len());
    for record in records.iter() {
        let line = format!("{:?} {:?} {:?}", record.args, record.error, record.resource);
        assert!(!line.contains(SECRET), "the audit record carried it: {line}");
    }
}

#[tokio::test]
async fn a_settings_save_never_touches_a_stored_secret() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("apps.json");
    let store = Arc::new(MemoryStore::default());
    let reg = registry(&path, store.clone());
    install(&reg, with_secret(declared())).await;
    set(&reg, "token").await.unwrap();
    reg.invoke(
        "extensions.configure",
        json!({"action":"settings","id":ID,"settings":{"team":"platform"}}),
    )
    .await
    .unwrap();
    assert_eq!(store.keys(), vec![secret_key(ID, "token")]);
    assert_eq!(
        listed(&reg).await["plugins"][0]["settings"]["token"],
        secret_reference(ID, "token")
    );
}

#[test]
fn a_secret_is_refused_in_a_settable_position_at_install() {
    // `k8s.annotate`'s `value` takes a string or select setting; a secret
    // there would be written into a cluster object.
    let mut value: Value =
        serde_json::from_str(include_str!("../../../../examples/extensions/argocd.json")).unwrap();
    // Required, so the only rule that can refuse it is the one about secrets.
    value["settings"] =
        json!([{"id":"token","type":"secret-reference","title":"Token","required":true}]);
    let mut permissions: Vec<String> = serde_json::from_value(value["permissions"].clone()).unwrap();
    permissions.push(SECRET_STORE_PERMISSION.into());
    value["permissions"] = json!(permissions);
    value["actions"][0]["arguments"]["value"] = json!("${settings.token}");
    let refused = Manifest::parse(&value.to_string()).unwrap_err();
    assert!(
        refused
            .0
            .iter()
            .any(|p| p.path == "actions[0].arguments.value" && p.message.contains("never interpolated")),
        "{refused}"
    );
    // And the host position itself takes no secret, whatever the manifest
    // says: `k8s.annotate` and `k8s.setStatusCondition` accept string and
    // select only, and no host capability declares a secret slot.
    let core = crate::build_registry();
    for (id, argument) in [("k8s.annotate", "value"), ("k8s.setStatusCondition", "message")] {
        let target = core.get(id).unwrap();
        let position = target.settable_argument(argument).unwrap();
        assert!(!position.accepts.contains(&srelens_capability::settings::SettingType::SecretReference));
        assert!(!target.takes_secret(argument));
    }
}

#[test]
fn the_access_review_names_the_secrets_an_app_keeps() {
    let before = Manifest::parse(&manifest()).unwrap();
    let after = Manifest::parse(&with_secret(declared())).unwrap();
    let grants: Vec<String> = serde_json::from_value(grants()).unwrap();
    let diff = permission_diff(
        Some((&before, &before.permissions.clone(), 1)),
        &after,
        &grants,
    );
    assert!(
        diff.added.iter().any(|item| item.contains(SECRET_STORE_PERMISSION)),
        "{diff:?}"
    );
    assert!(
        diff.added
            .iter()
            .any(|item| item.contains("Keep secrets") && item.contains("token") && item.contains("webhook")),
        "an update that keeps another secret is new access: {diff:?}"
    );
}
