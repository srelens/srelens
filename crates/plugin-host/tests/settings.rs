//! Typed settings (#542): what a manifest may declare, what a person may save
//! into one, and where the host lets a setting fill a binding argument.
use serde_json::{json, Map, Value};
use srelens_capability::settings::SettingType;
use srelens_capability::{Capability, Registry};
use srelens_plugin_host::{
    secret_reference, Manifest, PluginHost, ValidationCode, ValidationError,
};
use std::sync::{Arc, Mutex};

fn manifest() -> Value {
    json!({
        "id":"org.example.certs", "name":"Certificates", "version":"0.1.0", "srelensApiVersion":"^0.4",
        "kind":"declarative", "permissions":["test.read"],
        "capabilities":[{"name":"certificates","title":"List certificates", "target":"test.read",
            "arguments":{"group":"cert-manager.io"},"inputs":["context"]}],
        "contributions":{"pages":[{"id":"certificates","title":"Certificates","capability":"certificates"}],
            "detailTabs":[],"detailLinks":[]}
    })
}

/// One setting of every type, each valid.
fn every_type() -> Value {
    json!([
        {"id":"note","type":"string","title":"Note","maxLength":40},
        {"id":"expiryWindowDays","type":"number","title":"Warn before expiry (days)","default":14,"minimum":1,"maximum":365,"integer":true},
        {"id":"verbose","type":"boolean","title":"Verbose","default":false},
        {"id":"mode","type":"select","title":"Refresh","options":[{"value":"normal","label":"Normal"},{"value":"hard","label":"Hard"}],"default":"normal"},
        {"id":"kinds","type":"multi-select","title":"Kinds","options":[{"value":"a","label":"A"},{"value":"b","label":"B"}]},
        {"id":"prometheusUrl","type":"url","title":"Prometheus URL","required":true,"description":"Where metrics are read from."},
        {"id":"namespace","type":"namespace-selector","title":"Default namespace","default":"cert-manager"},
        {"id":"cluster","type":"cluster-selector","title":"Home cluster"},
        {"id":"token","type":"secret-reference","title":"API token"}
    ])
}

fn with_settings(settings: Value) -> Value {
    let mut value = manifest();
    // A secret setting needs the secret store's permission (#543).
    let secret = settings
        .as_array()
        .is_some_and(|all| all.iter().any(|s| s["type"] == "secret-reference"));
    if secret {
        value["permissions"] = json!(["test.read", "extension.secretStore"]);
    }
    value["settings"] = settings;
    value
}

fn parse(value: &Value) -> Manifest {
    Manifest::parse(&value.to_string()).expect("valid manifest")
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

fn problems(errors: &[ValidationError]) -> Vec<(String, String)> {
    let mut problems: Vec<_> = errors
        .iter()
        .map(|error| (code(error.code), error.path.clone()))
        .collect();
    problems.sort();
    problems
}

fn expected(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut pairs: Vec<_> = pairs
        .iter()
        .map(|(code, path)| (code.to_string(), path.to_string()))
        .collect();
    pairs.sort();
    pairs
}

fn values(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

#[test]
fn every_setting_type_is_declared_and_round_trips() {
    let parsed = parse(&with_settings(every_type()));
    assert_eq!(parsed.settings.len(), 9);
    let types: Vec<SettingType> = parsed.settings.iter().map(|s| s.setting_type).collect();
    assert_eq!(
        types,
        [
            SettingType::String,
            SettingType::Number,
            SettingType::Boolean,
            SettingType::Select,
            SettingType::MultiSelect,
            SettingType::Url,
            SettingType::NamespaceSelector,
            SettingType::ClusterSelector,
            SettingType::SecretReference,
        ]
    );
    let stored = serde_json::to_value(&parsed).unwrap();
    assert_eq!(stored["settings"], every_type());
    // A manifest that declares none stores none, so a signed manifest without
    // the field still round-trips to its own bytes.
    let plain = serde_json::to_value(parse(&manifest())).unwrap();
    assert!(plain.get("settings").is_none());
}

#[test]
fn declarations_are_checked_with_a_path_per_problem() {
    let hostile = "Token \u{202E}nekot";
    let settings = json!([
        {"id":"bad id","type":"string","title":"Bad id"},
        {"id":"dup","type":"string","title":"One"},
        {"id":"dup","type":"string","title":"Two"},
        {"id":"hostile","type":"string","title":hostile},
        {"id":"noOptions","type":"select","title":"No options"},
        {"id":"stray","type":"string","title":"Stray options","options":[{"value":"a","label":"A"}]},
        {"id":"twice","type":"select","title":"Twice","options":[{"value":"a","label":"A"},{"value":"a","label":"Again"}]},
        {"id":"wrongDefault","type":"number","title":"Wrong default","default":"14"},
        {"id":"offMenu","type":"select","title":"Off menu","options":[{"value":"a","label":"A"}],"default":"b"},
        {"id":"credentialed","type":"url","title":"Credentialed","default":"https://user:pw@example.com"},
        {"id":"secretDefault","type":"secret-reference","title":"Secret default","default":"hunter2"},
        {"id":"requiredDefault","type":"string","title":"Both","required":true,"default":"x"},
        {"id":"range","type":"number","title":"Range","minimum":10,"maximum":1},
        {"id":"integerText","type":"string","title":"Integer text","integer":true},
        {"id":"tooLong","type":"string","title":"Too long","maxLength":100000},
    ]);
    let found = errors(&with_settings(settings));
    assert_eq!(
        problems(&found),
        expected(&[
            ("EXTENSION_INVALID_VALUE", "settings[0].id"),
            ("EXTENSION_DUPLICATE_IDENTIFIER", "settings[2].id"),
            ("EXTENSION_INVALID_VALUE", "settings[3].title"),
            ("EXTENSION_INVALID_VALUE", "settings[4].options"),
            ("EXTENSION_INVALID_FIELD", "settings[5].options"),
            (
                "EXTENSION_DUPLICATE_IDENTIFIER",
                "settings[6].options[1].value"
            ),
            ("EXTENSION_INVALID_VALUE", "settings[7].default"),
            ("EXTENSION_INVALID_VALUE", "settings[8].default"),
            ("EXTENSION_INVALID_VALUE", "settings[9].default"),
            ("EXTENSION_INVALID_FIELD", "settings[10].default"),
            ("EXTENSION_INVALID_FIELD", "settings[11].default"),
            ("EXTENSION_INVALID_VALUE", "settings[12].minimum"),
            ("EXTENSION_INVALID_FIELD", "settings[13].integer"),
            ("EXTENSION_INVALID_VALUE", "settings[14].maxLength"),
        ])
    );
    // The hostile title is reported, never echoed as drawn text.
    assert!(found.iter().all(|e| !e.message.contains('\u{202E}')));
    // A secret default would publish the secret in the manifest; the problem
    // says why without repeating it.
    let secret = found
        .iter()
        .find(|e| e.path == "settings[10].default")
        .unwrap();
    // The failure output names the path only: printing the message would
    // print the secret in exactly the case this guards against.
    assert!(
        !secret.message.contains("hunter2"),
        "the refusal at settings[10].default repeats the default"
    );
}

#[test]
fn a_manifest_declares_at_most_32_settings() {
    let many: Vec<Value> = (0..33)
        .map(|i| json!({"id":format!("s{i}"),"type":"boolean","title":format!("S{i}")}))
        .collect();
    assert_eq!(
        problems(&errors(&with_settings(json!(many)))),
        expected(&[("EXTENSION_INVALID_VALUE", "settings")])
    );
}

#[test]
fn interpolation_is_refused_outside_top_level_binding_arguments() {
    let mut value =
        with_settings(json!([{"id":"mode","type":"string","title":"Mode","required":true}]));
    value["contributions"]["pages"][0]["title"] = json!("${settings.mode}");
    value["capabilities"][0]["arguments"]["nested"] = json!({"deep": "${settings.mode}"});
    value["capabilities"][0]["arguments"]["partial"] = json!("v-${settings.mode}");
    value["capabilities"][0]["arguments"]["missing"] = json!("${settings.nope}");
    value["capabilities"][0]["arguments"]["${settings.mode}"] = json!("key");
    assert_eq!(
        problems(&errors(&value)),
        expected(&[
            ("EXTENSION_INVALID_BINDING", "contributions.pages[0].title"),
            (
                "EXTENSION_INVALID_BINDING",
                "capabilities[0].arguments.nested.deep"
            ),
            (
                "EXTENSION_INVALID_BINDING",
                "capabilities[0].arguments.partial"
            ),
            (
                "EXTENSION_INVALID_BINDING",
                "capabilities[0].arguments.missing"
            ),
            (
                "EXTENSION_INVALID_BINDING",
                "capabilities[0].arguments.${settings.mode}"
            ),
        ])
    );
}

#[test]
fn an_interpolated_setting_always_has_a_value_and_is_never_a_secret() {
    let mut value = with_settings(json!([
        {"id":"optional","type":"string","title":"Optional"},
        {"id":"token","type":"secret-reference","title":"Token","required":true},
        {"id":"ok","type":"string","title":"Ok","required":true},
    ]));
    value["capabilities"][0]["arguments"]["a"] = json!("${settings.optional}");
    value["capabilities"][0]["arguments"]["b"] = json!("${settings.token}");
    value["capabilities"][0]["arguments"]["c"] = json!("${settings.ok}");
    assert_eq!(
        problems(&errors(&value)),
        expected(&[
            ("EXTENSION_INVALID_BINDING", "capabilities[0].arguments.a"),
            ("EXTENSION_INVALID_BINDING", "capabilities[0].arguments.b"),
        ])
    );
}

#[test]
fn saved_values_are_checked_against_their_declarations() {
    let parsed = parse(&with_settings(every_type()));
    let good = values(json!({
        "note":"hello", "expiryWindowDays":30, "verbose":true, "mode":"hard", "kinds":["a","b"],
        "prometheusUrl":"https://prom.example:9090/", "namespace":"team-a", "cluster":"/kube/config#prod"
    }));
    parsed
        .check_setting_values(&good)
        .expect("valid values save");
    // Only the required one is needed.
    parsed
        .check_setting_values(&values(json!({"prometheusUrl":"http://prom"})))
        .expect("optional settings may be left out");

    let bad = values(json!({
        "note":"x".repeat(41), "expiryWindowDays":1.5, "verbose":"yes", "mode":"medium",
        "kinds":["a","a"], "prometheusUrl":"ftp://prom", "namespace":"Team_A", "cluster":"prod\nctx",
        "unknown":1
    }));
    let found = parsed.check_setting_values(&bad).unwrap_err().0;
    assert_eq!(
        problems(&found),
        expected(&[
            ("EXTENSION_INVALID_VALUE", "settings.note"),
            ("EXTENSION_INVALID_VALUE", "settings.expiryWindowDays"),
            ("EXTENSION_INVALID_VALUE", "settings.verbose"),
            ("EXTENSION_INVALID_VALUE", "settings.mode"),
            ("EXTENSION_INVALID_VALUE", "settings.kinds"),
            ("EXTENSION_INVALID_VALUE", "settings.prometheusUrl"),
            ("EXTENSION_INVALID_VALUE", "settings.namespace"),
            ("EXTENSION_INVALID_VALUE", "settings.cluster"),
            ("EXTENSION_UNKNOWN_FIELD", "settings.unknown"),
        ])
    );
    // Values the form could never send are refused all the same.
    for (id, value) in [
        ("expiryWindowDays", json!(0)),
        ("expiryWindowDays", json!(366)),
        ("note", json!("line\nbreak")),
        ("note", json!("rtl \u{202E}override")),
        ("prometheusUrl", json!("https://user:secret@prom.example")),
        ("prometheusUrl", json!("javascript:alert(1)")),
        ("kinds", json!(["c"])),
        ("namespace", json!("a".repeat(64))),
    ] {
        let mut map = values(json!({"prometheusUrl":"http://prom"}));
        map.insert(id.into(), value.clone());
        let refused = parsed.check_setting_values(&map).unwrap_err();
        assert!(
            refused.0.iter().any(|e| e.path == format!("settings.{id}")),
            "{id} = {value} must be refused"
        );
    }
}

#[test]
fn a_required_setting_must_be_saved_with_a_value() {
    let parsed = parse(&with_settings(every_type()));
    for missing in [json!({}), json!({"prometheusUrl":""})] {
        let found = parsed.check_setting_values(&values(missing)).unwrap_err().0;
        assert_eq!(
            problems(&found),
            expected(&[("EXTENSION_INVALID_VALUE", "settings.prometheusUrl")])
        );
    }
}

#[test]
fn a_secret_value_is_never_accepted_as_a_setting_and_never_echoed() {
    let parsed = parse(&with_settings(every_type()));
    for secret in [
        json!("hunter2"),
        json!({"secretRef":"hunter2"}),
        secret_reference("org.example.certs", "token"),
    ] {
        let map = values(json!({"prometheusUrl":"http://prom","token":secret}));
        let found = parsed.check_setting_values(&map).unwrap_err().0;
        assert_eq!(
            problems(&found),
            expected(&[("EXTENSION_INVALID_VALUE", "settings.token")])
        );
        let text = format!("{found:?}");
        assert!(
            !text.contains("hunter2"),
            "a refusal repeated the secret: {text}"
        );
    }
    // No refusal repeats any value it was given.
    let map = values(json!({"prometheusUrl":"https://leaky:pw@prom","note":"hunter2\n"}));
    let text = format!("{:?}", parsed.check_setting_values(&map).unwrap_err());
    assert!(
        !text.contains("leaky") && !text.contains("hunter2"),
        "{text}"
    );
}

#[test]
fn stored_settings_keep_only_what_the_manifest_still_declares_validly() {
    let parsed = parse(&with_settings(json!([
        {"id":"url","type":"url","title":"URL"},
        {"id":"token","type":"secret-reference","title":"Token"},
        {"id":"wasSecret","type":"string","title":"Was a secret"},
        {"id":"mode","type":"select","title":"Mode","options":[{"value":"a","label":"A"}]}
    ])));
    let stored = values(json!({
        "url":"https://prom",
        // Was a string setting before this update: the plaintext must go, not
        // become the stored value of a secret.
        "token":"hunter2",
        // Was a secret before this update: its reference is not a string value.
        "wasSecret":secret_reference("org.example.certs", "wasSecret"),
        // The option it held is gone.
        "mode":"b",
        "removed":"x"
    }));
    let kept = parsed.retain_settings(stored);
    assert_eq!(Value::Object(kept), json!({"url":"https://prom"}));

    let reference = secret_reference("org.example.certs", "token");
    let kept = parsed.retain_settings(values(json!({"token": reference.clone()})));
    assert_eq!(kept["token"], reference, "a secret's own reference is kept");
    // Another app's reference is not this app's secret.
    let foreign = secret_reference("org.other.app", "token");
    assert!(parsed
        .retain_settings(values(json!({"token": foreign})))
        .is_empty());
}

#[test]
fn a_secret_reference_names_the_app_and_setting_and_holds_no_value() {
    let reference = secret_reference("org.example.certs", "token");
    assert_eq!(reference, json!({"secretRef":"org.example.certs/token"}));
    let parsed = parse(&with_settings(every_type()));
    assert!(parsed
        .stored_secret_problems(&values(json!({"token":reference})))
        .is_empty());
    let problems = parsed.stored_secret_problems(&values(json!({"token":"hunter2"})));
    assert_eq!(problems.len(), 1);
    assert!(!problems[0].contains("hunter2"));
}

// ---------------------------------------------------------------- the broker

/// A host capability whose `value` argument takes a string or select setting
/// and whose `count` takes a number, recording what its handler was called
/// with. Its own rule refuses a value of "forbidden".
fn core(calls: Arc<Mutex<Vec<Value>>>) -> Registry {
    let mut registry = Registry::new();
    let mut read = Capability::read_only("test.read", "fixture", |v| async move { Ok(v) });
    read.input_schema = json!({"type":"object","properties":{"context":{"type":"string"},"group":{"type":"string"},
        "value":{"type":"string"},"count":{"type":"number"},"flag":{"type":"boolean"},"other":{"type":"string"}},
        "required":["context","group"]});
    let read = read
        .with_settable(
            "value",
            &[SettingType::String, SettingType::Select],
            json!("stand-in"),
        )
        .with_settable("count", &[SettingType::Number], json!(1))
        .checking_bound_arguments(|arguments| match arguments.get("value") {
            Some(v) if v == "forbidden" => Err("`value` may not be forbidden".into()),
            // Echoes what it refuses, as `k8s.annotate`'s `resolve_value` does.
            Some(Value::String(v)) if v.starts_with('$') => {
                Err(format!("`{v}` is not a value this host substitutes"))
            }
            _ => Ok(()),
        });
    let recording = read.handler.clone();
    let read = Capability {
        handler: Arc::new(move |input| {
            calls.lock().unwrap().push(input.clone());
            recording(input)
        }),
        ..read
    };
    registry.register(read);
    registry
}

fn settable_manifest() -> Value {
    let mut value = with_settings(json!([
        {"id":"mode","type":"select","title":"Mode","options":[{"value":"normal","label":"Normal"},{"value":"forbidden","label":"Forbidden"}],"default":"normal"},
        {"id":"text","type":"string","title":"Text","required":true},
        {"id":"days","type":"number","title":"Days","default":14},
        {"id":"flag","type":"boolean","title":"Flag","default":true}
    ]));
    value["capabilities"][0]["arguments"]["value"] = json!("${settings.mode}");
    value["capabilities"][0]["arguments"]["count"] = json!("${settings.days}");
    value
}

#[test]
fn a_setting_fills_only_a_settable_argument_of_a_type_it_accepts() {
    let host = PluginHost::new(Arc::new(core(Default::default())));
    let accepted = parse(&settable_manifest());
    assert!(host
        .binding_problems(0, &accepted, &accepted.capabilities[0])
        .is_empty());

    let mut value = settable_manifest();
    // Not settable on this capability.
    value["capabilities"][0]["arguments"]["other"] = json!("${settings.text}");
    // Settable, but not by a boolean.
    value["capabilities"][0]["arguments"]["value"] = json!("${settings.flag}");
    let refused = parse(&value);
    let found = host.binding_problems(0, &refused, &refused.capabilities[0]);
    assert_eq!(
        problems(&found),
        expected(&[
            (
                "EXTENSION_INVALID_BINDING",
                "capabilities[0].arguments.other"
            ),
            (
                "EXTENSION_INVALID_BINDING",
                "capabilities[0].arguments.value"
            ),
        ])
    );
}

#[test]
fn the_target_rule_checks_the_binding_around_a_stand_in_at_install() {
    let host = PluginHost::new(Arc::new(core(Default::default())));
    // Refused by the capability's own rule with a literal …
    let mut literal = settable_manifest();
    literal["capabilities"][0]["arguments"]["value"] = json!("forbidden");
    let literal = parse(&literal);
    let refused = host.binding_problems(0, &literal, &literal.capabilities[0]);
    assert_eq!(
        problems(&refused),
        expected(&[("EXTENSION_INVALID_BINDING", "capabilities[0].arguments")])
    );
    // … and checked around the stand-in when a setting fills it, whatever
    // options the setting lists: the value is checked when it is chosen.
    let settable = parse(&settable_manifest());
    assert!(host
        .binding_problems(0, &settable, &settable.capabilities[0])
        .is_empty());
}

#[test]
fn saved_values_are_checked_by_every_binding_that_interpolates_them() {
    let host = PluginHost::new(Arc::new(core(Default::default())));
    let parsed = parse(&settable_manifest());
    assert!(host
        .settings_problems(&parsed, &values(json!({"text":"t","mode":"normal"})))
        .is_empty());
    // A value its declaration allows and the capability's own rule refuses.
    let found = host.settings_problems(&parsed, &values(json!({"text":"t","mode":"forbidden"})));
    assert_eq!(
        problems(&found),
        expected(&[("EXTENSION_INVALID_BINDING", "capabilities[0].arguments")])
    );
}

#[tokio::test]
async fn a_request_interpolates_the_current_values_through_the_same_checks() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let core = Arc::new(core(calls.clone()));
    let host = PluginHost::new(core);
    let id = "plugin/org.example.certs/certificates";
    let invoke = |settings: Value| {
        let host = &host;
        async move {
            let mut registry = Registry::new();
            let _registration = host
                .register_with_settings(
                    &mut registry,
                    parse(&settable_manifest()),
                    &["test.read".into()],
                    &values(settings),
                )
                .unwrap();
            registry.invoke(id, json!({"context":"prod"})).await
        }
    };

    // Saved values win; an unsaved setting takes its default.
    invoke(json!({"text":"t","mode":"normal"})).await.unwrap();
    let sent = calls.lock().unwrap().pop().unwrap();
    assert_eq!(sent["value"], "normal");
    assert_eq!(sent["count"], 14);
    assert_eq!(sent["group"], "cert-manager.io");

    invoke(json!({"text":"t","days":30})).await.unwrap();
    assert_eq!(calls.lock().unwrap().pop().unwrap()["count"], 30);

    // A stored value that no longer fits its declaration (a hand-edited
    // inventory) is refused on the request, and the refusal does not repeat it.
    let error = invoke(json!({"text":"t","days":"hunter2"}))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("days"), "{error}");
    assert!(!error.contains("hunter2"), "{error}");
    // As is one the capability's own rule refuses.
    assert!(invoke(json!({"text":"t","mode":"forbidden"}))
        .await
        .is_err());
    assert!(
        calls.lock().unwrap().is_empty(),
        "nothing refused reached the handler"
    );
}

/// PR #691 review: the target's own rule quotes the value it refuses, and with
/// a setting in place that value is the person's. Neither the save nor the
/// request may hand it back.
#[tokio::test]
async fn a_target_refusal_never_repeats_the_setting_value_it_refused() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let host = PluginHost::new(Arc::new(core(calls.clone())));
    let mut value = settable_manifest();
    value["capabilities"][0]["arguments"]["value"] = json!("${settings.text}");
    let parsed = parse(&value);
    let saved = values(json!({"text":"$abc123"}));

    let found = host.settings_problems(&parsed, &saved);
    assert_eq!(
        problems(&found),
        expected(&[("EXTENSION_INVALID_BINDING", "capabilities[0].arguments")])
    );
    let text = format!("{found:?}");
    assert!(
        !text.contains("abc123"),
        "the save refusal repeated it: {text}"
    );
    assert!(text.contains("not a value this host substitutes"), "{text}");

    let mut registry = Registry::new();
    let _registration = host
        .register_with_settings(&mut registry, parsed, &["test.read".into()], &saved)
        .unwrap();
    let error = registry
        .invoke(
            "plugin/org.example.certs/certificates",
            json!({"context":"prod"}),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(
        !error.contains("abc123"),
        "the request refusal repeated it: {error}"
    );
    assert!(
        error.contains("not a value this host substitutes"),
        "{error}"
    );
    assert!(calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_request_that_needs_an_unset_required_setting_says_which() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let host = PluginHost::new(Arc::new(core(calls.clone())));
    let mut value = settable_manifest();
    value["capabilities"][0]["arguments"]["value"] = json!("${settings.text}");
    let mut registry = Registry::new();
    let _registration = host
        .register_with_settings(
            &mut registry,
            parse(&value),
            &["test.read".into()],
            &Map::new(),
        )
        .unwrap();
    let error = registry
        .invoke(
            "plugin/org.example.certs/certificates",
            json!({"context":"prod"}),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("Text"),
        "names the setting by its title: {error}"
    );
    assert!(calls.lock().unwrap().is_empty());
}
