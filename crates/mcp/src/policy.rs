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

#[async_trait::async_trait]
pub trait ConfirmPolicy: Send + Sync {
    async fn confirm(&self, tool: &str, args: &Value) -> Decision;
}

/// The default. A host that wires no policy must not permit destructive tools.
pub struct AlwaysDeny;

/// Headless policy: the operator opts the process in with
/// `--mcp-allow-destructive` AND the caller states intent with `_confirm: true`.
/// Neither alone is sufficient.
pub struct FlagGated {
    allow_destructive: bool,
}

impl FlagGated {
    pub fn new(allow_destructive: bool) -> Self {
        Self { allow_destructive }
    }
}

#[async_trait::async_trait]
impl ConfirmPolicy for AlwaysDeny {
    async fn confirm(&self, tool: &str, _args: &Value) -> Decision {
        Decision::Denied(format!(
            "`{tool}` mutates the cluster and no consent mechanism is configured for this srelens process"
        ))
    }
}

#[async_trait::async_trait]
impl ConfirmPolicy for FlagGated {
    async fn confirm(&self, tool: &str, args: &Value) -> Decision {
        if !self.allow_destructive {
            return Decision::Denied(format!(
                "`{tool}` mutates the cluster; this srelens process was not started with --mcp-allow-destructive"
            ));
        }
        let confirmed = args.get("_confirm").and_then(Value::as_bool).unwrap_or(false);
        if !confirmed {
            return Decision::Denied(format!(
                "`{tool}` mutates the cluster. Re-send with \"_confirm\": true to state intent."
            ));
        }
        Decision::Approved
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn always_deny_refuses_everything() {
        let d = AlwaysDeny.confirm("k8s_deletePod", &json!({})).await;
        assert!(matches!(d, Decision::Denied(_)));
    }

    #[tokio::test]
    async fn flag_gated_requires_both_flag_and_confirm() {
        let with_flag = FlagGated::new(true);
        let without_flag = FlagGated::new(false);
        let confirmed = json!({ "_confirm": true });
        let bare = json!({});

        // The full 2x2. Only flag AND _confirm approves.
        assert_eq!(with_flag.confirm("t", &confirmed).await, Decision::Approved);
        assert!(matches!(with_flag.confirm("t", &bare).await, Decision::Denied(_)));
        assert!(matches!(without_flag.confirm("t", &confirmed).await, Decision::Denied(_)));
        assert!(matches!(without_flag.confirm("t", &bare).await, Decision::Denied(_)));
    }

    #[tokio::test]
    async fn flag_gated_denial_explains_which_half_is_missing() {
        let d = FlagGated::new(false).confirm("t", &json!({ "_confirm": true })).await;
        match d {
            Decision::Denied(r) => assert!(r.contains("--mcp-allow-destructive"), "got: {r}"),
            other => panic!("expected denial, got {other:?}"),
        }
    }
}
