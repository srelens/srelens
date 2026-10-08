//! What an agent reads of a tool's answer: the one place every successful MCP tool
//! result becomes text, on both transports and for the in-process agent.

use serde_json::Value;

/// The largest answer a read-only tool sends, in bytes of compact JSON.
///
/// Follows Claude Code's file threshold
/// (<https://code.claude.com/docs/en/mcp.md>, "MCP output limits and
/// warnings", as read on 2026-10-08): a text result over 50,000 characters is
/// saved to a file and the model is handed the path instead. srelens' agent
/// runs with `Read` disallowed, so such an answer is lost to it; better it
/// learns that and asks for less. Bytes, not characters, so a non-ASCII
/// answer is refused a little early rather than lost.
pub(crate) const MAX_RESULT_BYTES: usize = 50_000;

const LAST_APPLIED: &str = "kubectl.kubernetes.io/last-applied-configuration";

/// Trim known noise in place, keeping the result valid JSON (the transcript
/// summarizer parses it): `managedFields`, the copy of the manifest kubectl
/// keeps in an annotation, and the image cache a Node reports.
pub(crate) fn trim(v: &mut Value) {
    match v {
        Value::Array(items) => items.iter_mut().for_each(trim),
        Value::Object(map) => {
            // Only ever under `metadata`; elsewhere the key is someone's data.
            if let Some(Value::Object(meta)) = map.get_mut("metadata") {
                meta.remove("managedFields");
            }
            if let Some(Value::String(applied)) = map
                .get_mut("annotations")
                .and_then(|a| a.get_mut(LAST_APPLIED))
            {
                *applied = format!(
                    "<omitted by srelens: {}-byte copy of the manifest as last applied with kubectl>",
                    applied.len()
                );
            }
            if map.get("kind").and_then(Value::as_str) == Some("Node") {
                if let Some(images) = map.get_mut("status").and_then(|s| s.get_mut("images")) {
                    if let Some(n) = images.as_array().map(Vec::len) {
                        *images = Value::String(format!(
                            "<omitted by srelens: {n} cached container images; k8s.getManifest returns them>"
                        ));
                    }
                }
            }
            map.values_mut().for_each(trim);
        }
        _ => {}
    }
}

/// The text an agent is sent for `v`, trimmed, or — for a read over
/// [`MAX_RESULT_BYTES`] — why it is not sent. A mutation's answer is always
/// sent: refusing it would tell the agent a change that happened did not.
pub(crate) fn render(mut v: Value, read_only: bool) -> Result<String, String> {
    trim(&mut v);
    let text = v.to_string();
    if read_only && text.len() > MAX_RESULT_BYTES {
        return Err(format!(
            "The answer was {} bytes, over srelens' {MAX_RESULT_BYTES}-byte limit for one tool result, \
             so it was not sent. The call itself succeeded; this says nothing about the cluster. \
             To narrow it, ask for less — a namespace, a label or field selector, fewer fields or \
             lines where the tool takes them — and call again.",
            text.len()
        ));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const LAST_APPLIED: &str = "kubectl.kubernetes.io/last-applied-configuration";

    #[test]
    fn managed_fields_go_from_every_metadata_at_any_depth() {
        let mut v = json!({
            "items": [{
                "kind": "Deployment",
                "metadata": { "name": "web", "managedFields": [{ "manager": "kubectl" }] },
                "spec": { "template": { "metadata": { "managedFields": [], "labels": { "app": "web" } } } }
            }]
        });
        trim(&mut v);
        assert_eq!(v["items"][0]["metadata"], json!({ "name": "web" }));
        assert_eq!(
            v["items"][0]["spec"]["template"]["metadata"],
            json!({ "labels": { "app": "web" } })
        );
    }

    /// `managedFields` only ever lives in an object's `metadata`. Anywhere
    /// else it is the user's data — a ConfigMap key may be called that — and
    /// dropping it would show the agent a ConfigMap without it.
    #[test]
    fn a_data_key_named_managed_fields_stays() {
        let mut v = json!({ "kind": "ConfigMap", "data": { "managedFields": "mine" } });
        trim(&mut v);
        assert_eq!(v["data"]["managedFields"], "mine");
    }

    #[test]
    fn last_applied_becomes_a_marker_with_its_size_and_other_annotations_stay() {
        let applied = r#"{"apiVersion":"v1","kind":"ConfigMap"}"#;
        let mut v = json!({
            "metadata": { "annotations": { LAST_APPLIED: applied, "team": "sre" } }
        });
        trim(&mut v);
        let annotations = &v["metadata"]["annotations"];
        assert_eq!(annotations["team"], "sre");
        let marker = annotations[LAST_APPLIED]
            .as_str()
            .expect("the key stays, as a string");
        assert!(marker.starts_with("<omitted by srelens: "), "{marker}");
        assert!(
            marker.contains(&format!("{}-byte", applied.len())),
            "{marker}"
        );
    }

    #[test]
    fn a_nodes_images_become_a_count_naming_get_manifest_and_allocatable_stays() {
        let mut v = json!({
            "kind": "Node",
            "status": {
                "allocatable": { "cpu": "4" },
                "images": [{ "names": ["a"] }, { "names": ["b"] }, { "names": ["c"] }]
            }
        });
        trim(&mut v);
        assert_eq!(v["status"]["allocatable"], json!({ "cpu": "4" }));
        let marker = v["status"]["images"]
            .as_str()
            .expect("images become a marker");
        assert!(marker.contains("3 cached container images"), "{marker}");
        assert!(marker.contains("k8s.getManifest"), "{marker}");
    }

    #[test]
    fn a_node_inside_a_list_is_trimmed_too() {
        let mut v = json!({ "items": [{ "kind": "Node", "status": { "images": [{}] } }] });
        trim(&mut v);
        assert!(v["items"][0]["status"]["images"].is_string());
    }

    #[test]
    fn images_on_any_other_kind_stay() {
        let original =
            json!({ "kind": "ImageCache", "status": { "images": [{ "names": ["a"] }] } });
        let mut v = original.clone();
        trim(&mut v);
        assert_eq!(v, original);
    }

    /// A string of `n` bytes once rendered as compact JSON (`"…"`).
    fn answer_of(n: usize) -> Value {
        Value::String("x".repeat(n - 2))
    }

    #[test]
    fn an_answer_exactly_at_the_limit_is_sent() {
        let text = render(answer_of(MAX_RESULT_BYTES), true).expect("sent");
        assert_eq!(text.len(), MAX_RESULT_BYTES);
    }

    #[test]
    fn a_read_over_the_limit_is_refused_with_its_size_the_limit_and_how_to_narrow() {
        let refusal = render(answer_of(MAX_RESULT_BYTES + 1), true).expect_err("refused");
        assert!(
            refusal.contains(&(MAX_RESULT_BYTES + 1).to_string()),
            "{refusal}"
        );
        assert!(refusal.contains(&MAX_RESULT_BYTES.to_string()), "{refusal}");
        assert!(refusal.contains("succeeded"), "{refusal}");
        assert!(refusal.contains("narrow"), "{refusal}");
        // Claude Code keeps only the ends of an error over ~11,000 chars.
        assert!(refusal.len() < 1_000, "{} chars", refusal.len());
        // The transcript summarizer shows an error's first line.
        assert_eq!(refusal.lines().count(), 1, "{refusal}");
    }

    /// Refusing a mutation's output would tell the agent a change that
    /// happened did not, and invite it to make the change again.
    #[test]
    fn a_mutation_over_the_limit_is_still_sent() {
        let text = render(answer_of(MAX_RESULT_BYTES + 1), false).expect("sent");
        assert_eq!(text.len(), MAX_RESULT_BYTES + 1);
    }

    #[test]
    fn render_trims_before_measuring() {
        let v = json!({
            "metadata": { "name": "big", "managedFields": [{ "fieldsV1": "x".repeat(MAX_RESULT_BYTES) }] }
        });
        let text = render(v, true).expect("trimmed under the limit");
        assert_eq!(
            serde_json::from_str::<Value>(&text).unwrap(),
            json!({ "metadata": { "name": "big" } })
        );
    }
}
