//! Real lines of each kind, as each side writes them, against the schema:
//! every valid one validates and every invalid one does not. Lines starting
//! with `#` explain the ones below them.

use serde_json::{json, Value};
use srelens_sidecar_protocol::{schema, MethodSpec, MAX_CALL_FIELD_BYTES, METHODS};
use std::path::PathBuf;

fn validator(definition: &str) -> jsonschema::Validator {
    let root = schema();
    jsonschema::draft7::new(&json!({
        "definitions": root["definitions"],
        "allOf": [{"$ref": format!("#/definitions/{definition}")}],
    }))
    .expect("the schema compiles")
}

/// `(line number, message)` for every message in a fixture file.
fn messages(side: &str, file: &str) -> Vec<(usize, Value)> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/messages")
        .join(side)
        .join(file);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty() && !line.starts_with('#'))
        .map(|(n, line)| {
            let message = serde_json::from_str(line)
                .unwrap_or_else(|e| panic!("{side}/{file}:{}: not JSON: {e}", n + 1));
            (n + 1, message)
        })
        .collect()
}

fn check(side: &str, definition: &str) {
    let validator = validator(definition);
    let valid = messages(side, "valid.jsonl");
    let invalid = messages(side, "invalid.jsonl");
    assert!(!valid.is_empty() && !invalid.is_empty());
    for (line, message) in valid {
        let errors: Vec<String> = validator
            .iter_errors(&message)
            .map(|e| e.to_string())
            .collect();
        assert!(
            errors.is_empty(),
            "{side}/valid.jsonl:{line} is refused: {errors:?}"
        );
    }
    for (line, message) in invalid {
        assert!(
            !validator.is_valid(&message),
            "{side}/invalid.jsonl:{line} is accepted: {message}"
        );
    }
}

#[test]
fn what_srelens_writes_is_a_host_message() {
    check("host", "HostMessage");
}

#[test]
fn what_a_sidecar_writes_is_a_sidecar_message() {
    check("sidecar", "SidecarMessage");
}

#[test]
fn the_fixtures_show_every_method_and_both_kinds_of_answer() {
    for side in ["host", "sidecar"] {
        let sends = |spec: &MethodSpec| match side {
            "host" => spec.direction.from_host(),
            _ => spec.direction.from_sidecar(),
        };
        let valid = messages(side, "valid.jsonl");
        let named: Vec<&str> = valid
            .iter()
            .filter_map(|(_, m)| m["method"].as_str())
            .collect();
        for spec in METHODS.iter().filter(|spec| sends(spec)) {
            assert!(
                named.contains(&spec.name),
                "{side}/valid.jsonl has no {}",
                spec.name
            );
        }
        let answers = valid.iter().filter(|(_, m)| m.get("method").is_none());
        let (results, errors): (Vec<_>, Vec<_>) =
            answers.partition(|(_, m)| m.get("result").is_some());
        assert!(
            !results.is_empty() && !errors.is_empty(),
            "{side}/valid.jsonl lacks a result or an error answer"
        );
    }
}

#[test]
fn a_call_id_past_its_bound_is_refused() {
    let sidecar = validator("SidecarMessage");
    let call = |id: String| {
        json!({"jsonrpc": "2.0", "id": id, "method": "host/read",
               "params": {"context": {"clusterId": "kind-dev", "namespace": null},
                          "capability": "applications"}})
    };
    assert!(sidecar.is_valid(&call("x".repeat(MAX_CALL_FIELD_BYTES))));
    assert!(!sidecar.is_valid(&call("x".repeat(MAX_CALL_FIELD_BYTES + 1))));
}
