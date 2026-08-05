//! Who decides whether a mutating tool may run. The decision is injected so
//! the same gate serves a GUI (prompt a human), a headless CLI (explicit
//! flags), and tests (a stub) without branching inside the request handler.

use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Approved,
    /// Reason is surfaced to the agent so it can adapt rather than retry blindly.
    Denied(String),
}

/// Why a tool needs consent. These are different risks with different blast
/// radii, so a headless operator can grant one without the other: letting an
/// agent read a Secret should not also let it drain a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsentKind {
    /// Changes cluster state (or local host state, e.g. installing a tool).
    Destructive,
    /// Changes nothing, but returns sensitive material — `k8s.getSecret` and
    /// anything else annotated `SENSITIVE_READ`.
    SensitiveRead,
}

impl ConsentKind {
    /// The CLI flag that opts a headless process in to this kind of call.
    pub fn flag(self) -> &'static str {
        match self {
            ConsentKind::Destructive => "--mcp-allow-destructive",
            ConsentKind::SensitiveRead => "--mcp-allow-sensitive-reads",
        }
    }

    /// What the tool does, for a denial an agent has to act on.
    fn effect(self) -> &'static str {
        match self {
            ConsentKind::Destructive => "mutates the cluster",
            ConsentKind::SensitiveRead => "returns sensitive material",
        }
    }
}

#[async_trait::async_trait]
pub trait ConfirmPolicy: Send + Sync {
    async fn confirm(&self, tool: &str, args: &Value, kind: ConsentKind) -> Decision;
}

/// The default. A host that wires no policy must not permit gated tools.
pub struct AlwaysDeny;

/// Headless policy: the operator opts the process in with the flag matching the
/// call's [`ConsentKind`] AND the caller states intent with `_confirm: true`.
/// Neither alone is sufficient, and neither flag implies the other.
pub struct FlagGated {
    allow_destructive: bool,
    allow_sensitive_reads: bool,
}

impl FlagGated {
    pub fn new(allow_destructive: bool, allow_sensitive_reads: bool) -> Self {
        Self { allow_destructive, allow_sensitive_reads }
    }

    fn allows(&self, kind: ConsentKind) -> bool {
        match kind {
            ConsentKind::Destructive => self.allow_destructive,
            ConsentKind::SensitiveRead => self.allow_sensitive_reads,
        }
    }
}

#[async_trait::async_trait]
impl ConfirmPolicy for AlwaysDeny {
    async fn confirm(&self, tool: &str, _args: &Value, kind: ConsentKind) -> Decision {
        Decision::Denied(format!(
            "`{tool}` {} and no consent mechanism is configured for this srelens process",
            kind.effect()
        ))
    }
}

#[async_trait::async_trait]
impl ConfirmPolicy for FlagGated {
    async fn confirm(&self, tool: &str, args: &Value, kind: ConsentKind) -> Decision {
        if !self.allows(kind) {
            return Decision::Denied(format!(
                "`{tool}` {}; this srelens process was not started with {}",
                kind.effect(),
                kind.flag()
            ));
        }
        let confirmed = args.get("_confirm").and_then(Value::as_bool).unwrap_or(false);
        if !confirmed {
            return Decision::Denied(format!(
                "`{tool}` {}. Re-send with \"_confirm\": true to state intent.",
                kind.effect()
            ));
        }
        Decision::Approved
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Both flags on — the baseline for tests about `_confirm` rather than
    /// about which flag gates which kind.
    fn permissive() -> FlagGated {
        FlagGated::new(true, true)
    }

    #[tokio::test]
    async fn always_deny_refuses_everything() {
        let d = AlwaysDeny
            .confirm("k8s_deletePod", &json!({}), ConsentKind::Destructive)
            .await;
        assert!(matches!(d, Decision::Denied(_)));
    }

    #[tokio::test]
    async fn always_deny_refuses_a_sensitive_read_too() {
        let d = AlwaysDeny
            .confirm("k8s.getSecret", &json!({}), ConsentKind::SensitiveRead)
            .await;
        assert!(matches!(d, Decision::Denied(_)));
    }

    #[tokio::test]
    async fn flag_gated_requires_both_flag_and_confirm() {
        let with_flag = permissive();
        let without_flag = FlagGated::new(false, false);
        let confirmed = json!({ "_confirm": true });
        let bare = json!({});
        let k = ConsentKind::Destructive;

        // The full 2x2. Only flag AND _confirm approves.
        assert_eq!(with_flag.confirm("t", &confirmed, k).await, Decision::Approved);
        assert!(matches!(with_flag.confirm("t", &bare, k).await, Decision::Denied(_)));
        assert!(matches!(without_flag.confirm("t", &confirmed, k).await, Decision::Denied(_)));
        assert!(matches!(without_flag.confirm("t", &bare, k).await, Decision::Denied(_)));
    }

    #[tokio::test]
    async fn flag_gated_denial_explains_which_half_is_missing() {
        let d = FlagGated::new(false, false)
            .confirm("t", &json!({ "_confirm": true }), ConsentKind::Destructive)
            .await;
        match d {
            Decision::Denied(r) => assert!(r.contains("--mcp-allow-destructive"), "got: {r}"),
            other => panic!("expected denial, got {other:?}"),
        }
    }

    /// The point of the split: authorizing an agent to READ a Secret must not
    /// also authorize it to delete or drain anything. Granting only
    /// `--mcp-allow-sensitive-reads` lets `k8s.getSecret` through...
    #[tokio::test]
    async fn allowing_sensitive_reads_authorizes_a_sensitive_read() {
        let d = FlagGated::new(false, true)
            .confirm("k8s.getSecret", &json!({ "_confirm": true }), ConsentKind::SensitiveRead)
            .await;
        assert_eq!(d, Decision::Approved);
    }

    /// ...and must NOT let a mutating tool through.
    #[tokio::test]
    async fn allowing_sensitive_reads_does_not_authorize_a_destructive_tool() {
        let d = FlagGated::new(false, true)
            .confirm("k8s.deletePod", &json!({ "_confirm": true }), ConsentKind::Destructive)
            .await;
        match d {
            Decision::Denied(r) => assert!(r.contains("--mcp-allow-destructive"), "got: {r}"),
            other => panic!("expected denial, got {other:?}"),
        }
    }

    /// The converse, which is the actual bug being fixed: before the split,
    /// `--mcp-allow-destructive` was the only way to read a Secret headless.
    /// Now the destructive flag alone must not unlock sensitive reads.
    #[tokio::test]
    async fn allowing_destructive_does_not_authorize_a_sensitive_read() {
        let d = FlagGated::new(true, false)
            .confirm("k8s.getSecret", &json!({ "_confirm": true }), ConsentKind::SensitiveRead)
            .await;
        match d {
            Decision::Denied(r) => {
                assert!(r.contains("--mcp-allow-sensitive-reads"), "got: {r}")
            }
            other => panic!("expected denial, got {other:?}"),
        }
    }

    /// A sensitive read still needs stated intent, exactly like a mutation:
    /// the flag opts the process in, `_confirm` opts the individual call in.
    #[tokio::test]
    async fn a_sensitive_read_still_needs_confirm() {
        let d = FlagGated::new(true, true)
            .confirm("k8s.getSecret", &json!({}), ConsentKind::SensitiveRead)
            .await;
        assert!(matches!(d, Decision::Denied(_)));
    }
}
