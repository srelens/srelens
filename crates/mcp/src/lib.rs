//! Bridges the capability registry to the Model Context Protocol.

pub mod audit;
pub mod auth;
pub mod completeness;
pub mod http;
pub mod policy;
pub mod prompts;
pub mod resources;
pub mod stdio;
pub mod subscriptions;

use std::sync::Arc;

use srelens_capability::{CapabilityError, Registry};
use serde_json::Value;

/// The largest JSON-RPC request either transport accepts, in bytes: one stdio line
/// (without its `\n` or `\r\n` ending) or one HTTP request body. 4 MiB leaves room for the largest
/// capability input — a 256 KiB manifest, even with every character escaped — and
/// bounds what a client can make the server hold before any field is checked.
pub const MAX_REQUEST_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct ToolDescriptor {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    /// MCP `annotations.readOnlyHint` — the tool doesn't mutate its
    /// environment. Clients (e.g. Cursor) use it to auto-approve read-only
    /// calls in non-interactive mode instead of rejecting them for lack of a
    /// human approver.
    pub read_only: bool,
    /// MCP `annotations.destructiveHint` — the tool may perform destructive
    /// updates (only meaningful when `read_only` is false).
    pub destructive: bool,
}

/// Which transport a request arrived on. Recorded in the audit log and used to
/// tell an operator how an agent reached them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Stdio,
    Http,
}

impl Transport {
    pub fn as_str(self) -> &'static str {
        match self {
            Transport::Stdio => "stdio",
            Transport::Http => "http",
        }
    }
}

/// An MCP request is one of the two sources the audit trail knows about; the
/// other is the desktop UI, which never reaches this crate.
impl From<Transport> for crate::audit::Source {
    fn from(t: Transport) -> Self {
        match t {
            Transport::Stdio => crate::audit::Source::McpStdio,
            Transport::Http => crate::audit::Source::McpHttp,
        }
    }
}

pub struct McpServer {
    registry: Arc<Registry>,
    confirm_policy: Arc<dyn crate::policy::ConfirmPolicy>,
    audit: Arc<dyn crate::audit::AuditSink>,
    prompts: crate::prompts::PromptLibrary,
    kind_resolver: std::sync::Arc<dyn crate::resources::KindResolver>,
    watcher: std::sync::Arc<dyn crate::resources::ObjectWatcher>,
}

impl McpServer {
    pub fn new(registry: Arc<Registry>) -> Self {
        Self {
            registry,
            // Fail closed: a host that wires nothing permits nothing.
            confirm_policy: Arc::new(crate::policy::AlwaysDeny),
            audit: Arc::new(crate::audit::NoopAudit),
            // Built-ins only until a host supplies a user prompt directory.
            prompts: crate::prompts::PromptLibrary::new(None),
            // Fail closed: no resolver means no object resources, only the two
            // fixed ones.
            kind_resolver: std::sync::Arc::new(crate::resources::NoKinds),
            // Fail closed: refuse subscriptions rather than accept ones that
            // will never fire.
            watcher: std::sync::Arc::new(crate::resources::NoWatcher),
        }
    }

    pub fn with_policy(mut self, policy: Arc<dyn crate::policy::ConfirmPolicy>) -> Self {
        self.confirm_policy = policy;
        self
    }

    pub fn confirm_policy(&self) -> &Arc<dyn crate::policy::ConfirmPolicy> {
        &self.confirm_policy
    }

    pub fn with_audit(mut self, audit: Arc<dyn crate::audit::AuditSink>) -> Self {
        self.audit = audit;
        self
    }

    pub fn audit(&self) -> &Arc<dyn crate::audit::AuditSink> {
        &self.audit
    }

    pub fn with_prompts(mut self, prompts: crate::prompts::PromptLibrary) -> Self {
        self.prompts = prompts;
        self
    }

    pub fn prompts(&self) -> &crate::prompts::PromptLibrary {
        &self.prompts
    }

    pub fn with_kind_resolver(
        mut self,
        kind_resolver: std::sync::Arc<dyn crate::resources::KindResolver>,
    ) -> Self {
        self.kind_resolver = kind_resolver;
        self
    }

    pub fn kind_resolver(&self) -> &std::sync::Arc<dyn crate::resources::KindResolver> {
        &self.kind_resolver
    }

    pub fn with_watcher(
        mut self,
        watcher: std::sync::Arc<dyn crate::resources::ObjectWatcher>,
    ) -> Self {
        self.watcher = watcher;
        self
    }

    pub fn watcher(&self) -> &std::sync::Arc<dyn crate::resources::ObjectWatcher> {
        &self.watcher
    }

    /// Whether a tool reads sensitive material, so the audit log can redact
    /// its arguments wholesale.
    pub fn is_sensitive(&self, name: &str) -> bool {
        self.registry
            .get(name)
            .map(|c| c.annotations.sensitive)
            .unwrap_or(false)
    }

    pub fn list_tools(&self) -> Vec<ToolDescriptor> {
        self.registry
            .ids()
            .into_iter()
            .filter_map(|id| self.registry.get(id))
            .map(|cap| ToolDescriptor {
                name: cap.id.clone(),
                description: cap.summary.clone(),
                input_schema: cap.input_schema.clone(),
                read_only: cap.annotations.read_only,
                destructive: cap.annotations.destructive,
            })
            .collect()
    }

    pub async fn call_tool(&self, name: &str, args: Value) -> Result<Value, CapabilityError> {
        self.registry.invoke(name, args).await
    }

    /// Call a tool and let the registry write the audit record for it.
    ///
    /// `handle_request` used to build the record itself, which is why the
    /// desktop bridge had none: two call sites, one of them forgotten. The
    /// registry is where both surfaces meet, so it does the recording and this
    /// only supplies what MCP knows and it does not — which transport the call
    /// came in on, and what the consent policy decided.
    pub async fn call_tool_audited(
        &self,
        name: &str,
        args: Value,
        transport: Transport,
        decision: &'static str,
    ) -> Result<Value, CapabilityError> {
        self.registry
            .invoke_audited(name, args, self.audit.as_ref(), transport.into(), decision)
            .await
    }

    /// Whether a tool should be consent-gated over remote transports: it
    /// mutates the cluster (destructive), or otherwise requires explicit
    /// confirmation (e.g. `k8s.getSecret`, which is a `SENSITIVE_READ` and so
    /// sets `requires_confirm` itself even though it's read-only).
    pub fn requires_confirm(&self, name: &str) -> bool {
        self.consent_kind(name).is_some()
    }

    /// *Why* a tool needs consent, or `None` if it doesn't — the single source
    /// of truth for the gate in `handle_request`.
    ///
    /// The split reads off `read_only`, not `sensitive`: a gated capability
    /// that changes nothing is gated because of what it *returns*
    /// (`SENSITIVE_READ`), while anything else gated mutates something. Using
    /// `sensitive` here would misfile `k8s.diffManifest`, which is `sensitive`
    /// (its output can echo Secret data, so the audit log redacts it) but
    /// isn't gated at all.
    pub fn consent_kind(&self, name: &str) -> Option<crate::policy::ConsentKind> {
        let cap = self.registry.get(name)?;
        if !(cap.annotations.requires_confirm || cap.annotations.destructive) {
            return None;
        }
        Some(if cap.annotations.read_only {
            crate::policy::ConsentKind::SensitiveRead
        } else {
            crate::policy::ConsentKind::Destructive
        })
    }

    /// The whole gated call, for a policy to decide on: the kind, the host's
    /// impact level and the host's confirmation sentence rendered against
    /// these arguments. `None` when the tool is not gated at all.
    ///
    /// Rendered here rather than in each policy so every surface — the GUI
    /// prompt, the headless denial, the host confirmation (#552) — shows one
    /// sentence, written once, in the host.
    pub fn consent_request(&self, name: &str, args: &Value) -> Option<crate::policy::ConsentRequest> {
        let kind = self.consent_kind(name)?;
        let annotations = self.registry.get(name)?.annotations;
        Some(crate::policy::ConsentRequest {
            tool: name.to_string(),
            args: args.clone(),
            kind,
            impact: annotations.impact,
            confirm_text: annotations.confirm_text(args),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use srelens_capability::Capability;
    use serde_json::json;

    fn registry_with_ping() -> Arc<Registry> {
        let mut reg = Registry::new();
        reg.register(Capability::read_only("ping", "health check", |v| async move {
            Ok(json!({ "echo": v }))
        }));
        Arc::new(reg)
    }

    #[test]
    fn kind_resolver_builder_and_getter_expose_injected_resolver() {
        struct PodKinds;

        impl crate::resources::KindResolver for PodKinds {
            fn scope(&self, kind: &str) -> Option<crate::resources::KindScope> {
                (kind == "Pod").then_some(crate::resources::KindScope::Namespaced)
            }
        }

        let server =
            McpServer::new(registry_with_ping()).with_kind_resolver(Arc::new(PodKinds));

        assert_eq!(
            server.kind_resolver().scope("Pod"),
            Some(crate::resources::KindScope::Namespaced)
        );
    }

    #[test]
    fn list_tools_mirrors_registry() {
        let server = McpServer::new(registry_with_ping());
        let tools = server.list_tools();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "ping");
        assert_eq!(tools[0].description, "health check");
    }

    #[tokio::test]
    async fn call_tool_invokes_capability() {
        let server = McpServer::new(registry_with_ping());
        let out = server.call_tool("ping", json!("hi")).await.unwrap();
        assert_eq!(out, json!({ "echo": "hi" }));
    }

    /// The vulnerability this closes: a `SENSITIVE_READ` capability like
    /// `k8s.getSecret` mutates nothing, so plain `destructive` gating alone
    /// would let it run with no prompt at all. `SENSITIVE_READ` sets
    /// `requires_confirm: true` itself to close that gap.
    #[tokio::test]
    async fn sensitive_read_capability_requires_confirm() {
        let mut reg = Registry::new();
        let mut cap = Capability::read_only("k8s.getSecret", "reads a secret", |_| async {
            Ok(json!({}))
        });
        cap.annotations = srelens_capability::Annotations::SENSITIVE_READ;
        reg.register(cap);
        let server = McpServer::new(Arc::new(reg));

        assert!(
            server.requires_confirm("k8s.getSecret"),
            "a sensitive-read capability must be consent-gated"
        );
    }

    /// A gated capability that changes nothing is gated for what it RETURNS,
    /// so it must classify as a sensitive read — otherwise headless operators
    /// are back to needing `--mcp-allow-destructive` to read a Secret.
    #[tokio::test]
    async fn a_gated_read_only_capability_is_a_sensitive_read() {
        let mut reg = Registry::new();
        let mut cap = Capability::read_only("k8s.getSecret", "reads a secret", |_| async {
            Ok(json!({}))
        });
        cap.annotations = srelens_capability::Annotations::SENSITIVE_READ;
        reg.register(cap);
        let server = McpServer::new(Arc::new(reg));

        assert_eq!(
            server.consent_kind("k8s.getSecret"),
            Some(crate::policy::ConsentKind::SensitiveRead)
        );
    }

    /// A gated capability that mutates classifies as destructive, whether or
    /// not it is flagged `destructive` — `MUTATING` (e.g. `k8s.applyManifest`,
    /// `k8s.helmRepoUpdate`) changes state and belongs behind the write flag.
    #[tokio::test]
    async fn a_gated_mutating_capability_is_destructive() {
        let mut reg = Registry::new();
        let mut destructive = Capability::read_only("k8s.deletePod", "deletes", |_| async {
            Ok(json!({}))
        });
        destructive.annotations = srelens_capability::Annotations::DESTRUCTIVE;
        reg.register(destructive);
        let mut mutating = Capability::read_only("k8s.applyManifest", "applies", |_| async {
            Ok(json!({}))
        });
        mutating.annotations = srelens_capability::Annotations::MUTATING;
        reg.register(mutating);
        let server = McpServer::new(Arc::new(reg));

        assert_eq!(
            server.consent_kind("k8s.deletePod"),
            Some(crate::policy::ConsentKind::Destructive)
        );
        assert_eq!(
            server.consent_kind("k8s.applyManifest"),
            Some(crate::policy::ConsentKind::Destructive),
            "a non-destructive mutation still belongs behind the write flag"
        );
    }

    /// The case that pins WHICH annotation drives the split. `SENSITIVE_READ`
    /// happens to set both `read_only` and `sensitive`, so the two readings
    /// agree there and neither of the tests above can tell them apart. They
    /// diverge on a capability that mutates AND handles sensitive material —
    /// a Secret write, say. Classifying that as a sensitive read would mean
    /// `--mcp-allow-sensitive-reads` alone authorizes *modifying* Secrets:
    /// read permission escalating to write. It mutates, so it is destructive.
    #[tokio::test]
    async fn a_gated_capability_that_mutates_sensitive_data_is_destructive() {
        let mut reg = Registry::new();
        let mut cap = Capability::read_only("k8s.updateConfigData", "writes a Secret", |_| async {
            Ok(json!({}))
        });
        cap.annotations =
            srelens_capability::Annotations { sensitive: true, ..srelens_capability::Annotations::MUTATING };
        reg.register(cap);
        let server = McpServer::new(Arc::new(reg));

        assert_eq!(
            server.consent_kind("k8s.updateConfigData"),
            Some(crate::policy::ConsentKind::Destructive),
            "a sensitive WRITE must not be unlocked by the sensitive-read flag"
        );
    }

    #[tokio::test]
    async fn an_ungated_capability_has_no_consent_kind() {
        let server = McpServer::new(registry_with_ping());
        assert_eq!(server.consent_kind("ping"), None);
        assert_eq!(server.consent_kind("no-such-tool"), None);
    }

    #[tokio::test]
    async fn plain_read_only_capability_does_not_require_confirm() {
        let server = McpServer::new(registry_with_ping());
        assert!(!server.requires_confirm("ping"));
    }

    /// Regression test for the collateral gating this branch fixes:
    /// `k8s.diffManifest` is read-only and sets `sensitive: true` (its diff
    /// output can echo Secret data, so it's redacted in the audit log) but is
    /// NOT `requires_confirm` and NOT `destructive` — it makes no cluster
    /// change (a server dry-run apply). `sensitive` alone must not gate it,
    /// or a purely read-only tool gets denied over headless MCP with a
    /// mutation warning it doesn't deserve.
    #[tokio::test]
    async fn sensitive_but_not_confirm_gated_capability_is_not_gated() {
        let mut reg = Registry::new();
        let mut cap = Capability::read_only("k8s.diffManifest", "diff a manifest", |_| async {
            Ok(json!({}))
        });
        cap.annotations = srelens_capability::Annotations {
            sensitive: true,
            ..srelens_capability::Annotations::READ_ONLY
        };
        reg.register(cap);
        let server = McpServer::new(Arc::new(reg));

        assert!(
            !server.requires_confirm("k8s.diffManifest"),
            "a read-only, non-confirm-requiring capability must not be gated just because it is sensitive"
        );
    }
}
