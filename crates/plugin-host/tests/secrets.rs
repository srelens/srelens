//! Secret settings (#543): the permission a manifest must request to keep
//! one, the value type that never prints, and the single host path that may
//! put a stored secret into a request.
use serde_json::{json, Map, Value};
use srelens_capability::Capability;
use srelens_plugin_host::{
    secret_key, secret_reference, Manifest, PluginHost, SecretStore, SecretValue, ValidationCode,
    SECRET_STORE_PERMISSION,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

const TOKEN: &str = "tok-3f9a1c-very-secret";

fn manifest(permissions: &[&str], settings: Value) -> Value {
    json!({
        "id":"org.example.metrics", "name":"Metrics", "version":"0.1.0", "srelensApiVersion":"^0.4",
        "kind":"declarative", "permissions": permissions,
        "capabilities":[{"name":"rows","title":"Rows","target":"test.read",
            "arguments":{"group":"example.io"},"inputs":["context"]}],
        "settings": settings,
        "contributions":{"pages":[{"id":"rows","title":"Rows","capability":"rows"}],"detailTabs":[],"detailLinks":[]}
    })
}

fn token_setting() -> Value {
    json!([{"id":"token","type":"secret-reference","title":"API token"}])
}

fn problems(value: &Value) -> Vec<(ValidationCode, String)> {
    match Manifest::parse(&value.to_string()) {
        Ok(_) => vec![],
        Err(errors) => errors.0.into_iter().map(|e| (e.code, e.path)).collect(),
    }
}

fn install_problems(value: &Value) -> Vec<(ValidationCode, String)> {
    let manifest = Manifest::parse(&value.to_string()).expect("a manifest a host keeps");
    manifest
        .install_problems()
        .into_iter()
        .map(|e| (e.code, e.path))
        .collect()
}

#[test]
fn a_new_install_of_a_secret_setting_needs_the_secret_store_permission() {
    let missing = manifest(&["test.read"], token_setting());
    assert!(
        install_problems(&missing)
            .contains(&(ValidationCode::PermissionMismatch, "permissions".into())),
        "{:?}",
        install_problems(&missing)
    );
    let granted = manifest(&["test.read", SECRET_STORE_PERMISSION], token_setting());
    assert_eq!(problems(&granted), vec![]);
    assert_eq!(
        install_problems(&granted),
        vec![],
        "the permission and the setting go together"
    );
}

/// #691 shipped in `srelens-v0.15.1-185`, before the permission existed. A
/// manifest stored then is still one the host keeps (it is re-checked on
/// every load and request); only a new install is held to the rule.
#[test]
fn a_stored_manifest_from_before_the_permission_is_still_valid() {
    let stored = manifest(&["test.read"], token_setting());
    assert_eq!(problems(&stored), vec![]);
}

#[test]
fn the_secret_store_permission_is_requested_only_with_a_secret_setting() {
    let plain = json!([{"id":"note","type":"string","title":"Note"}]);
    let unused = manifest(&["test.read", SECRET_STORE_PERMISSION], plain);
    assert!(
        problems(&unused).contains(&(ValidationCode::PermissionMismatch, "permissions".into())),
        "an app asks for exactly what it uses: {:?}",
        problems(&unused)
    );
}

#[test]
fn the_secret_store_is_granted_never_bound() {
    let mut value = manifest(&["test.read", SECRET_STORE_PERMISSION], token_setting());
    value["capabilities"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name":"store","title":"Store","target":SECRET_STORE_PERMISSION,"arguments":{},"inputs":[]}));
    assert!(
        problems(&value).contains(&(
            ValidationCode::UnsupportedTarget,
            "capabilities[1].target".into()
        )),
        "{:?}",
        problems(&value)
    );
}

#[test]
fn a_secret_value_never_prints() {
    // Facts only in the messages: every value compared here holds, or would
    // on failure print, the secret.
    let value = SecretValue::new(TOKEN.to_owned());
    assert!(value.expose() == TOKEN, "expose returns the value it holds");
    let printed_it = format!("{value:?}").contains(TOKEN);
    assert!(!printed_it, "Debug printed the secret");
    // Nested inside anything that derives Debug, as a panic message would be.
    let nested_it = format!("{:?}", Some(vec![value])).contains(TOKEN);
    assert!(!nested_it, "Debug of a value holding it printed the secret");
}

#[test]
fn the_reference_and_its_key_name_the_same_place() {
    assert_eq!(
        secret_key("org.example.metrics", "token"),
        "org.example.metrics/token"
    );
    assert_eq!(
        secret_reference("org.example.metrics", "token"),
        json!({"secretRef": "org.example.metrics/token"})
    );
}

/// A store in memory, as the desktop vault behaves when it is unlocked.
#[derive(Default)]
struct MemoryStore(Mutex<BTreeMap<String, String>>);

impl SecretStore for MemoryStore {
    fn status(&self) -> Result<(), String> {
        Ok(())
    }
    fn put(&self, key: &str, value: &SecretValue) -> Result<(), String> {
        self.0
            .lock()
            .unwrap()
            .insert(key.into(), value.expose().into());
        Ok(())
    }
    fn contains(&self, key: &str) -> Result<bool, String> {
        Ok(self.0.lock().unwrap().contains_key(key))
    }
    fn retain(&self, keep: &BTreeSet<String>) -> Result<(), String> {
        self.0.lock().unwrap().retain(|key, _| keep.contains(key));
        Ok(())
    }
    fn reveal(&self, key: &str) -> Result<Option<SecretValue>, String> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .get(key)
            .cloned()
            .map(SecretValue::new))
    }
}

fn stored() -> (Manifest, Map<String, Value>, MemoryStore) {
    let manifest = Manifest::parse(
        &manifest(&["test.read", SECRET_STORE_PERMISSION], token_setting()).to_string(),
    )
    .unwrap();
    let mut settings = Map::new();
    settings.insert("token".into(), secret_reference(&manifest.id, "token"));
    let store = MemoryStore::default();
    store
        .put(
            &secret_key(&manifest.id, "token"),
            &SecretValue::new(TOKEN.into()),
        )
        .unwrap();
    (manifest, settings, store)
}

fn grants() -> Vec<String> {
    vec!["test.read".into(), SECRET_STORE_PERMISSION.into()]
}

fn http() -> Capability {
    Capability::read_only("test.http", "http", |_: Value| async { Ok(json!({})) })
        .with_secret_slot("headers")
}

#[test]
fn a_secret_is_injected_only_where_the_target_declares_a_slot() {
    let (manifest, settings, store) = stored();
    let got = PluginHost::inject_secret(
        &http(),
        "headers",
        &manifest,
        &grants(),
        &settings,
        "token",
        &store,
    )
    .expect("declared slot, declared secret, granted, set");
    assert!(
        got.expose() == TOKEN,
        "the declared slot gets the stored value"
    );

    // No current host capability declares a slot: `k8s.annotate`'s `value`
    // is the kind of position a setting may fill, and a secret may not.
    let annotate = Capability::read_only("k8s.annotate", "annotate", |_: Value| async {
        Ok(json!({}))
    });
    let refused = PluginHost::inject_secret(
        &annotate,
        "value",
        &manifest,
        &grants(),
        &settings,
        "token",
        &store,
    )
    .unwrap_err();
    assert!(!refused.contains(TOKEN), "the refusal carried the secret");
    assert!(refused.contains("value"), "{refused}");
    let wrong_slot = PluginHost::inject_secret(
        &http(),
        "url",
        &manifest,
        &grants(),
        &settings,
        "token",
        &store,
    );
    assert!(wrong_slot.is_err());
}

#[test]
fn injection_needs_the_grant_the_declaration_and_the_hosts_own_reference() {
    let (manifest, settings, store) = stored();
    let target = http();
    // Not granted: an inventory edited by hand, or an older grant list.
    let ungranted = PluginHost::inject_secret(
        &target,
        "headers",
        &manifest,
        &["test.read".to_owned()],
        &settings,
        "token",
        &store,
    );
    assert!(ungranted.unwrap_err().contains(SECRET_STORE_PERMISSION));
    // Not a declared secret.
    assert!(PluginHost::inject_secret(
        &target,
        "headers",
        &manifest,
        &grants(),
        &settings,
        "other",
        &store
    )
    .is_err());
    // A reference to another app's secret is never followed.
    let mut foreign = settings.clone();
    foreign.insert(
        "token".into(),
        secret_reference("org.example.other", "token"),
    );
    store
        .put(
            &secret_key("org.example.other", "token"),
            &SecretValue::new("theirs".into()),
        )
        .unwrap();
    let crossed = PluginHost::inject_secret(
        &target,
        "headers",
        &manifest,
        &grants(),
        &foreign,
        "token",
        &store,
    );
    assert!(crossed.is_err());
    // Not set.
    let empty = PluginHost::inject_secret(
        &target,
        "headers",
        &manifest,
        &grants(),
        &Map::new(),
        "token",
        &store,
    );
    assert!(empty.unwrap_err().contains("not set"));
}

#[test]
fn a_refused_injection_never_names_the_value() {
    let (manifest, settings, store) = stored();
    let annotate = Capability::read_only("k8s.annotate", "annotate", |_: Value| async {
        Ok(json!({}))
    });
    for why in [
        PluginHost::inject_secret(
            &annotate,
            "value",
            &manifest,
            &grants(),
            &settings,
            "token",
            &store,
        )
        .unwrap_err(),
        PluginHost::inject_secret(
            &http(),
            "headers",
            &manifest,
            &[],
            &settings,
            "token",
            &store,
        )
        .unwrap_err(),
    ] {
        assert!(
            !why.contains(TOKEN),
            "a refused injection carried the secret"
        );
    }
}
