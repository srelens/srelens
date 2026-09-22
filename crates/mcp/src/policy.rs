//! Who decides whether a mutating tool may run. The decision is injected so
//! the same gate serves a GUI (prompt a human), a headless CLI (explicit
//! flags), and tests (a stub) without branching inside the request handler.

use serde_json::Value;
use srelens_capability::Impact;

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
    pub fn effect(self) -> &'static str {
        match self {
            ConsentKind::Destructive => "mutates the cluster",
            ConsentKind::SensitiveRead => "returns sensitive material",
        }
    }
}

/// One gated call, with everything a policy needs to decide and everything a
/// human needs to be asked.
///
/// A struct rather than four arguments because the host metadata is what grows:
/// `impact` and `confirm_text` arrived with #548, the host-owned confirmation
/// (#552) reads them, and a policy that ignores them still compiles.
#[derive(Debug, Clone, PartialEq)]
pub struct ConsentRequest {
    pub tool: String,
    /// The arguments as the caller sent them, `_confirm` included — a policy
    /// may read that hint; the tool never does.
    pub args: Value,
    pub kind: ConsentKind,
    /// How much this call disturbs. Not the same question as `kind`: a Secret
    /// read and a node drain are different kinds AND different levels, and two
    /// destructive calls can differ in level between themselves.
    pub impact: Impact,
    /// The host's own sentence for this call, rendered from the capability's
    /// confirmation template against `args`.
    ///
    /// `None` when the capability carries no template, or when the template
    /// names something this call has no value for. A surface that gets `None`
    /// shows the tool summary; it does not show half a sentence.
    pub confirm_text: Option<String>,
}

impl ConsentRequest {
    /// What to put in front of a person or an agent: the host's sentence if it
    /// has one, else a plain statement of the effect.
    pub fn prompt(&self) -> String {
        self.confirm_text
            .clone()
            .unwrap_or_else(|| format!("`{}` {}", self.tool, self.kind.effect()))
    }
}

#[async_trait::async_trait]
pub trait ConfirmPolicy: Send + Sync {
    async fn confirm(&self, request: &ConsentRequest) -> Decision;
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
    async fn confirm(&self, request: &ConsentRequest) -> Decision {
        Decision::Denied(format!(
            "`{}` {} and no consent mechanism is configured for this srelens process",
            request.tool,
            request.kind.effect()
        ))
    }
}

#[async_trait::async_trait]
impl ConfirmPolicy for FlagGated {
    async fn confirm(&self, request: &ConsentRequest) -> Decision {
        let (tool, kind) = (&request.tool, request.kind);
        if !self.allows(kind) {
            return Decision::Denied(format!(
                "`{tool}` {}; this srelens process was not started with {}",
                kind.effect(),
                kind.flag()
            ));
        }
        let confirmed = request.args.get("_confirm").and_then(Value::as_bool).unwrap_or(false);
        if !confirmed {
            // The host's own words, where it has them: an agent asked to state
            // intent should be told what it is stating intent about, and the
            // level is the part `mutates the cluster` cannot carry.
            let detail = request
                .confirm_text
                .as_deref()
                .map(|text| format!(" {text}"))
                .unwrap_or_default();
            return Decision::Denied(format!(
                "`{tool}` {} ({} impact).{detail} Re-send with \"_confirm\": true to state intent.",
                kind.effect(),
                request.impact.as_str(),
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

    /// A request as `McpServer::consent_request` would build one, with the
    /// host metadata left at its least informative so a test that does not
    /// care about it reads as though it isn't there.
    fn request(tool: &str, args: Value, kind: ConsentKind) -> ConsentRequest {
        ConsentRequest {
            tool: tool.into(),
            args,
            kind,
            impact: match kind {
                ConsentKind::Destructive => Impact::High,
                ConsentKind::SensitiveRead => Impact::Medium,
            },
            confirm_text: None,
        }
    }

    #[tokio::test]
    async fn always_deny_refuses_everything() {
        let d = AlwaysDeny
            .confirm(&request("k8s_deletePod", json!({}), ConsentKind::Destructive))
            .await;
        assert!(matches!(d, Decision::Denied(_)));
    }

    #[tokio::test]
    async fn always_deny_refuses_a_sensitive_read_too() {
        let d = AlwaysDeny
            .confirm(&request("k8s.getSecret", json!({}), ConsentKind::SensitiveRead))
            .await;
        assert!(matches!(d, Decision::Denied(_)));
    }

    #[tokio::test]
    async fn flag_gated_requires_both_flag_and_confirm() {
        let with_flag = permissive();
        let without_flag = FlagGated::new(false, false);
        let k = ConsentKind::Destructive;
        let confirmed = request("t", json!({ "_confirm": true }), k);
        let bare = request("t", json!({}), k);

        // The full 2x2. Only flag AND _confirm approves.
        assert_eq!(with_flag.confirm(&confirmed).await, Decision::Approved);
        assert!(matches!(with_flag.confirm(&bare).await, Decision::Denied(_)));
        assert!(matches!(without_flag.confirm(&confirmed).await, Decision::Denied(_)));
        assert!(matches!(without_flag.confirm(&bare).await, Decision::Denied(_)));
    }

    #[tokio::test]
    async fn flag_gated_denial_explains_which_half_is_missing() {
        let d = FlagGated::new(false, false)
            .confirm(&request("t", json!({ "_confirm": true }), ConsentKind::Destructive))
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
            .confirm(&request("k8s.getSecret", json!({ "_confirm": true }), ConsentKind::SensitiveRead))
            .await;
        assert_eq!(d, Decision::Approved);
    }

    /// ...and must NOT let a mutating tool through.
    #[tokio::test]
    async fn allowing_sensitive_reads_does_not_authorize_a_destructive_tool() {
        let d = FlagGated::new(false, true)
            .confirm(&request("k8s.deletePod", json!({ "_confirm": true }), ConsentKind::Destructive))
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
            .confirm(&request("k8s.getSecret", json!({ "_confirm": true }), ConsentKind::SensitiveRead))
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
            .confirm(&request("k8s.getSecret", json!({}), ConsentKind::SensitiveRead))
            .await;
        assert!(matches!(d, Decision::Denied(_)));
    }

    /// An agent told only "mutates the cluster" cannot tell a node drain from
    /// a status refresh. The denial it is asked to re-send now carries the
    /// host's level and the host's own sentence.
    #[tokio::test]
    async fn the_denial_carries_the_level_and_the_hosts_words() {
        let mut req = request("extensions.action", json!({}), ConsentKind::Destructive);
        req.confirm_text = Some("Apply the desired resources of Application team/api?".into());
        match permissive().confirm(&req).await {
            Decision::Denied(r) => {
                assert!(r.contains("high impact"), "got: {r}");
                assert!(r.contains("Apply the desired resources"), "got: {r}");
            }
            other => panic!("expected denial, got {other:?}"),
        }
    }

    /// And a capability with no template is still a complete sentence — the
    /// host does not show half of one.
    #[tokio::test]
    async fn a_denial_without_host_words_still_reads() {
        match permissive()
            .confirm(&request("toolbox.install", json!({}), ConsentKind::Destructive))
            .await
        {
            Decision::Denied(r) => {
                assert!(r.contains("mutates the cluster (high impact)."), "got: {r}");
                assert!(!r.contains(".."), "no gap where the sentence would be: {r}");
            }
            other => panic!("expected denial, got {other:?}"),
        }
    }

    /// The fallback a confirming surface uses: the host's sentence when there
    /// is one, a plain statement of the effect when there is not.
    #[test]
    fn prompt_prefers_the_hosts_sentence() {
        let mut req = request("k8s.drainNode", json!({}), ConsentKind::Destructive);
        assert_eq!(req.prompt(), "`k8s.drainNode` mutates the cluster");
        req.confirm_text = Some("Drain node-1?".into());
        assert_eq!(req.prompt(), "Drain node-1?");
    }
}
