//! The capability catalog: a stable, serializable projection of the registry
//! (id + the annotation flags the frontend needs). Emitted to a committed JSON
//! so a Vitest test can cross-check the palette — and Settings can name the
//! confirm-gated capabilities — without linking Rust. Kept in sync by a test
//! in `lib.rs`.
use serde::Serialize;
use srelens_capability::Registry;

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
        })
        .collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use srelens_capability::{Annotations, Capability};

    /// Every annotation preset, projected. The `SENSITIVE_READ` row is the one
    /// that matters: `readOnly: true, destructive: false, requiresConfirm:
    /// true` is a combination the other two flags cannot express, so a
    /// consumer reading only them would report `k8s.getSecret` as ungated.
    #[test]
    fn carries_the_gate_flag_no_other_field_implies() {
        let mut reg = Registry::new();
        let mut sensitive = Capability::read_only("t.secret", "reads a secret", |_| async {
            Ok(json!({}))
        });
        sensitive.annotations = Annotations::SENSITIVE_READ;
        reg.register(sensitive);
        let mut mutating = Capability::read_only("t.install", "installs a tool", |_| async {
            Ok(json!({}))
        });
        mutating.annotations = Annotations::MUTATING;
        reg.register(mutating);
        reg.register(Capability::read_only("t.list", "lists", |_| async { Ok(json!({})) }));

        let by_id = |id: &str| -> (bool, bool, bool) {
            let e = catalog_of(&reg).into_iter().find(|e| e.id == id).expect("registered");
            (e.read_only, e.destructive, e.requires_confirm)
        };
        assert_eq!(by_id("t.secret"), (true, false, true), "a sensitive read is gated");
        assert_eq!(by_id("t.install"), (false, false, true), "a non-destructive change is gated");
        assert_eq!(by_id("t.list"), (true, false, false), "an ordinary read is not");
    }
}
