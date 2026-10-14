//! Typed app settings (#542): the types a setting can have, the one spelling
//! that interpolates a setting into a binding argument, and the argument
//! positions a host capability marks as settable.

use serde_json::{json, Value};
use srelens_capability::settings::{self, SettingType};
use srelens_capability::{Annotations, Capability};

#[test]
fn setting_types_use_the_manifest_spelling() {
    let all = [
        (SettingType::String, "string"),
        (SettingType::Number, "number"),
        (SettingType::Boolean, "boolean"),
        (SettingType::Select, "select"),
        (SettingType::MultiSelect, "multi-select"),
        (SettingType::Url, "url"),
        (SettingType::NamespaceSelector, "namespace-selector"),
        (SettingType::ClusterSelector, "cluster-selector"),
        (SettingType::SecretReference, "secret-reference"),
    ];
    for (kind, wire) in all {
        assert_eq!(serde_json::to_value(kind).unwrap(), json!(wire));
        assert_eq!(
            serde_json::from_value::<SettingType>(json!(wire)).unwrap(),
            kind
        );
    }
    // The Rust spelling is not the manifest's.
    assert!(serde_json::from_value::<SettingType>(json!("MultiSelect")).is_err());
    assert!(serde_json::from_value::<SettingType>(json!("secret")).is_err());
}

#[test]
fn a_reference_is_the_whole_value_and_names_one_setting() {
    assert_eq!(
        settings::reference(&json!("${settings.prometheusUrl}")),
        Some(Ok("prometheusUrl"))
    );
    assert_eq!(
        settings::reference(&json!("${settings.a-b}")),
        Some(Ok("a-b"))
    );
    // Not a reference at all: left to the capability's own rules.
    for plain in [
        json!("$now"),
        json!("plain text"),
        json!(3),
        json!(true),
        json!({}),
    ] {
        assert_eq!(settings::reference(&plain), None, "{plain}");
    }
    // Something that tries to be one and is not: refused, never passed on as
    // literal text a person would read as a working template.
    for broken in [
        "prefix-${settings.url}",
        "${settings.url}/api",
        "${settings.}",
        "${settings.a b}",
        "${settings.a.b}",
        "${ settings.url }",
        "${settings.url",
    ] {
        assert!(
            matches!(settings::reference(&json!(broken)), Some(Err(_))),
            "{broken} must be refused"
        );
    }
}

#[test]
fn mentions_find_a_reference_anywhere_in_a_value() {
    assert!(settings::mentions_setting("${settings.x}"));
    assert!(settings::mentions_setting("see ${settings.x} here"));
    assert!(!settings::mentions_setting("$now"));
    assert!(!settings::mentions_setting("settings.x"));
}

#[test]
fn a_capability_marks_its_settable_arguments_and_nothing_else_is() {
    let plain = Capability::read_only("test.read", "read", |_: Value| async { Ok(json!({})) });
    assert!(plain.settable.is_empty(), "nothing is settable by default");

    let marked = Capability::read_only("test.read", "read", |_: Value| async { Ok(json!({})) })
        .with_settable(
            "value",
            &[SettingType::String, SettingType::Select],
            json!("x"),
        );
    let position = marked.settable_argument("value").expect("marked");
    assert_eq!(
        position.accepts,
        vec![SettingType::String, SettingType::Select]
    );
    assert_eq!(position.stand_in, json!("x"));
    assert!(marked.settable_argument("key").is_none());
    assert_eq!(marked.annotations, Annotations::READ_ONLY);
}

#[test]
fn a_secret_reference_is_never_accepted_by_a_settable_position() {
    let result = std::panic::catch_unwind(|| {
        Capability::read_only("test.read", "read", |_: Value| async { Ok(json!({})) })
            .with_settable("header", &[SettingType::SecretReference], json!("x"))
    });
    assert!(
        result.is_err(),
        "a secret's value is injected by the secret store (#543), never interpolated"
    );
}
