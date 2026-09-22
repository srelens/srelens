//! The capability catalog: a stable, serializable projection of the registry
//! (id + the annotation flags the frontend needs). Emitted to a committed JSON
//! so a Vitest test can cross-check the palette — and Settings can name the
//! confirm-gated capabilities — without linking Rust. Kept in sync by a test
//! in `lib.rs`.
use serde::Serialize;
use srelens_capability::{Impact, Registry};

#[derive(Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub struct CatalogEntry {
    pub id: String,
    pub read_only: bool,
    pub destructive: bool,
    /// Whether execution stops at a consent step. This is the flag that
    /// actually gates a call (`McpServer::requires_confirm`), and it is not
    /// derivable from the other two: `SENSITIVE_READ` is `read_only` and not
    /// `destructive` and is still gated, and `MUTATING` is neither `read_only`
    /// nor `destructive` and is gated too. Carried so the frontend can NAME
    /// the gated set rather than transcribe a guess at it — the new design's
    /// `Agent access` pane shipped six invented ids from the mock (`node.drain`,
    /// `resource.delete`, …) under a heading claiming completeness.
    pub requires_confirm: bool,
    /// Reads or reveals secret material. Shown beside the capabilities an app is granted.
    pub sensitive: bool,
    /// `low`, `medium` or `high` — how much a successful call disturbs, which
    /// `requires_confirm` cannot say. Two gated capabilities are not equally
    /// alarming, and a pane that shows one badge for both teaches a reader to
    /// click through all of them.
    pub impact: Impact,
    /// The host's confirmation template, or `null` when the host authored
    /// none and a confirming surface falls back to the summary.
    ///
    /// The template, not a rendered sentence: it is rendered against a call's
    /// arguments, which the catalog does not have. The scheme is documented on
    /// `srelens_capability::render_confirm` and in
    /// `docs/extensions/capabilities.md`. Carried here so the host
    /// confirmation (#552) reads the same words the MCP gate does, rather than
    /// a second copy in UI constants — which is where they live today.
    pub confirm: Option<&'static str>,
}

/// Project a registry into a sorted catalog for the frontend bridge.
pub fn catalog_of(reg: &Registry) -> Vec<CatalogEntry> {
    let mut out: Vec<CatalogEntry> = reg
        .entries()
        .map(|c| CatalogEntry {
            id: c.id.clone(),
            read_only: c.annotations.read_only,
            destructive: c.annotations.destructive,
            requires_confirm: c.annotations.requires_confirm,
            sensitive: c.annotations.sensitive,
            impact: c.annotations.impact,
            confirm: c.annotations.confirm,
        })
        .collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use srelens_capability::{Annotations, Capability};

    /// Every annotation preset, projected. The `SENSITIVE_READ` row is the one
    /// that matters: `readOnly: true, destructive: false, requiresConfirm:
    /// true` is a combination the other two flags cannot express, so a
    /// consumer reading only them would report `k8s.getSecret` as ungated.
    #[test]
    fn carries_the_gate_flag_no_other_field_implies() {
        let mut reg = Registry::new();
        let mut sensitive =
            Capability::read_only("t.secret", "reads a secret", |_| async { Ok(json!({})) });
        sensitive.annotations = Annotations::SENSITIVE_READ;
        reg.register(sensitive);
        let mut mutating =
            Capability::read_only("t.install", "installs a tool", |_| async { Ok(json!({})) });
        mutating.annotations = Annotations::MUTATING;
        reg.register(mutating);
        reg.register(Capability::read_only("t.list", "lists", |_| async {
            Ok(json!({}))
        }));

        let by_id = |id: &str| -> (bool, bool, bool, bool) {
            let e = catalog_of(&reg)
                .into_iter()
                .find(|e| e.id == id)
                .expect("registered");
            (e.read_only, e.destructive, e.requires_confirm, e.sensitive)
        };
        assert_eq!(
            by_id("t.secret"),
            (true, false, true, true),
            "a sensitive read is gated"
        );
        assert_eq!(
            by_id("t.install"),
            (false, false, true, false),
            "a non-destructive change is gated"
        );
        assert_eq!(
            by_id("t.list"),
            (true, false, false, false),
            "an ordinary read is not"
        );
    }

    /// The fields #548 adds. `sensitive` was already projected; `impact` and
    /// the confirmation template were not, and the pane that shows an app's
    /// grants could only say "requires confirmation" about a Secret read and a
    /// node drain alike.
    #[test]
    fn projects_the_level_and_the_hosts_wording() {
        let mut reg = Registry::new();
        let mut drain =
            Capability::read_only("t.drain", "drains a node", |_| async { Ok(json!({})) });
        drain.annotations = Annotations::DESTRUCTIVE;
        reg.register(drain);
        reg.register(Capability::read_only("t.list", "lists", |_| async {
            Ok(json!({}))
        }));

        let by_id = |id: &str| {
            catalog_of(&reg)
                .into_iter()
                .find(|e| e.id == id)
                .expect("registered")
        };
        let drain = by_id("t.drain");
        assert_eq!(drain.impact, Impact::High);
        assert_eq!(drain.confirm, Annotations::DESTRUCTIVE.confirm);
        let list = by_id("t.list");
        assert_eq!(list.impact, Impact::Low);
        assert_eq!(list.confirm, None, "an ordinary read asks nothing");
    }

    /// The JSON the frontend imports: lowercase words, and `null` rather than a
    /// missing key, so `CapabilityFacts` in `packages/core` stays assignable.
    #[test]
    fn serializes_the_level_as_a_lowercase_word() {
        let mut reg = Registry::new();
        let mut cap =
            Capability::read_only("t.secret", "reads a secret", |_| async { Ok(json!({})) });
        cap.annotations = Annotations::SENSITIVE_READ;
        reg.register(cap);
        reg.register(Capability::read_only("t.list", "lists", |_| async {
            Ok(json!({}))
        }));
        let json = serde_json::to_value(catalog_of(&reg)).unwrap();
        assert_eq!(json[1]["impact"], json!("medium"));
        assert_eq!(
            json[1]["confirm"],
            json!(Annotations::SENSITIVE_READ.confirm.unwrap())
        );
        assert_eq!(json[0]["impact"], json!("low"));
        assert_eq!(json[0]["confirm"], Value::Null);
    }
}
