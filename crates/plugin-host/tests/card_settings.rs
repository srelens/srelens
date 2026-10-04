//! A settings-backed expiry window must be checked by the manifest, not treated as a literal.
use serde_json::{json, Value};
use srelens_plugin_host::{Manifest, ValidationCode};

fn declaration() -> Value {
    let mut value: Value =
        serde_json::from_str(include_str!("../../../examples/extensions/argocd.json")).unwrap();
    value["srelensApiVersion"] = json!("^0.7");
    value["settings"] = json!([{
        "id":"expiryWindow", "type":"select", "title":"Warn before expiry",
        "default":"14d", "options":[
            {"value":"14d","label":"14 days"},{"value":"30d","label":"30 days"}
        ]
    }]);
    value["contributions"]["dashboardCards"] = json!([{
        "id":"expiring", "title":"Expiring soon", "size":"s", "type":"count",
        "source":"applications", "predicate":{
            "jsonPath":".status.notAfter", "within":"${settings.expiryWindow}"
        }, "target":{"page":"applications"}
    }]);
    value
}

fn parse(value: &Value) -> Manifest {
    Manifest::parse(&value.to_string()).unwrap_or_else(|errors| panic!("{errors}"))
}

#[test]
fn a_select_duration_reference_is_valid_on_api_0_7() {
    let parsed = parse(&declaration());
    assert_eq!(
        parsed.contributions.dashboard_cards[0]
            .predicate
            .as_ref()
            .unwrap()
            .within
            .as_deref(),
        Some("${settings.expiryWindow}")
    );
}

#[test]
fn duration_references_refuse_old_or_broad_api_ranges() {
    for range in ["^0.4", "^0.5", "^0.6", ">=0.4, <0.8", ">=0.6, <0.8"] {
        let mut value = declaration();
        value["srelensApiVersion"] = json!(range);
        let errors = Manifest::parse(&value.to_string()).unwrap_err();
        assert!(
            errors
                .0
                .iter()
                .any(|e| e.code == ValidationCode::ApiIncompatible
                    && e.message.contains("requires API 0.7.0")),
            "{range}: {errors}"
        );
    }
    for range in ["^0.7", ">=0.7, <0.8"] {
        let mut value = declaration();
        value["srelensApiVersion"] = json!(range);
        parse(&value);
    }
}

#[test]
fn duration_settings_require_a_declared_select_with_a_default() {
    for change in [
        "undeclared",
        "string",
        "number",
        "secret-reference",
        "no-default",
    ] {
        let mut value = declaration();
        match change {
            "undeclared" => value["settings"] = json!([]),
            "no-default" => {
                value["settings"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("default");
            }
            ty => {
                value["settings"][0]["type"] = json!(ty);
            }
        }
        let errors = Manifest::parse(&value.to_string()).unwrap_err();
        assert!(
            errors
                .0
                .iter()
                .any(|e| e.path == "contributions.dashboardCards[0].predicate"),
            "{change}: {errors}"
        );
    }
}

#[test]
fn every_duration_option_must_be_positive_and_bounded() {
    for invalid in [
        "0d",
        "-1d",
        "3651d",
        "1.5d",
        "14",
        "soon",
        "${settings.other}",
    ] {
        let mut value = declaration();
        value["settings"][0]["options"][1]["value"] = json!(invalid);
        let errors = Manifest::parse(&value.to_string()).unwrap_err();
        assert!(
            errors
                .0
                .iter()
                .any(|e| e.path == "contributions.dashboardCards[0].predicate"),
            "{invalid}: {errors}"
        );
    }
}

#[test]
fn references_stay_whole_and_only_in_the_duration_position() {
    for invalid in [
        "14${settings.expiryWindow}",
        "${ settings.expiryWindow}",
        "${settings.expiryWindow}.",
    ] {
        let mut value = declaration();
        value["contributions"]["dashboardCards"][0]["predicate"]["within"] = json!(invalid);
        assert!(Manifest::parse(&value.to_string()).is_err(), "{invalid}");
    }
    for field in ["title", "source"] {
        let mut value = declaration();
        value["contributions"]["dashboardCards"][0][field] = json!("${settings.expiryWindow}");
        assert!(Manifest::parse(&value.to_string()).is_err(), "{field}");
    }
    let mut value = declaration();
    value["contributions"]["dashboardCards"][0]["predicate"] = json!({
        "jsonPath":".status.notAfter", "before":"${settings.expiryWindow}"});
    assert!(Manifest::parse(&value.to_string()).is_err());
}

#[test]
fn existing_literal_windows_keep_their_api_line() {
    for literal in ["14d", "-1h"] {
        let mut value = declaration();
        value["srelensApiVersion"] = json!("^0.4");
        value["contributions"]["dashboardCards"][0]["predicate"]["within"] = json!(literal);
        parse(&value);
    }
}

#[test]
fn duration_settings_resolve_defaults_and_saved_values_without_moving_the_source() {
    let manifest = parse(&declaration());
    let card = &manifest.contributions.dashboard_cards[0];
    let empty = serde_json::Map::new();
    let default = card.with_settings(&manifest, &empty).unwrap();
    let saved = json!({"expiryWindow":"30d"}).as_object().unwrap().clone();
    let configured = card.with_settings(&manifest, &saved).unwrap();
    let object = json!({"status":{"notAfter":"2026-01-21T00:00:00Z"}});
    assert!(!default
        .predicate
        .as_ref()
        .unwrap()
        .holds_at(&object, 1767225600));
    assert!(configured
        .predicate
        .as_ref()
        .unwrap()
        .holds_at(&object, 1767225600));
    assert_eq!(configured.source, "applications");
    assert_eq!(
        configured.predicate.as_ref().unwrap().json_path,
        ".status.notAfter"
    );
    assert_eq!(configured.target.unwrap().page, "applications");
    assert_eq!(
        card.predicate.as_ref().unwrap().within.as_deref(),
        Some("${settings.expiryWindow}")
    );
}

#[test]
fn corrupt_stored_duration_values_are_errors_not_zero_counts_or_echoed_values() {
    let manifest = parse(&declaration());
    let card = &manifest.contributions.dashboard_cards[0];
    for invalid in [
        json!("private-value"),
        json!(14),
        json!({"secret":"private-value"}),
    ] {
        let saved = json!({"expiryWindow":invalid}).as_object().unwrap().clone();
        let error = card.with_settings(&manifest, &saved).unwrap_err();
        assert!(error.contains("expiryWindow"), "{error}");
        assert!(!error.contains("private-value"), "{error}");
    }
}

#[test]
fn future_duration_boundaries_keep_expired_and_late_certificates_out() {
    let manifest = parse(&declaration());
    let card = manifest.contributions.dashboard_cards[0]
        .with_settings(&manifest, &serde_json::Map::new())
        .unwrap();
    let predicate = card.predicate.as_ref().unwrap();
    for (time, expected) in [
        ("2025-12-31T23:59:59Z", false),
        ("2026-01-01T00:00:01Z", true),
        ("2026-01-15T00:00:00Z", true),
        ("2026-01-15T00:00:01Z", false),
        ("not-a-time", false),
    ] {
        assert_eq!(
            predicate.holds_at(&json!({"status":{"notAfter":time}}), 1767225600),
            expected,
            "{time}"
        );
    }
}
