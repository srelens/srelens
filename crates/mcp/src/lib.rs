//! Bridges the capability registry to the Model Context Protocol.

#[cfg(test)]
mod app_tools_tests;
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

/// Every installed app's tool is `plugin/<app id>/<operation>` (#574).
pub const APP_TOOL_PREFIX: &str = "plugin/";

/// The tools installed apps add (#574): readers, declared actions and sidecar
/// operations, which come and go while the server runs.
///
/// Each snapshot is a registry of its own. When the apps change, the source
/// builds a new one and revokes the handlers of the one it replaces, so a
/// caller still holding an older snapshot can see its tools but not run them.
#[async_trait::async_trait]
pub trait ToolSource: Send + Sync {
    /// The snapshot as it stands now, after catching up with any change the
    /// source has not seen yet — including one another process made.
    async fn tools(&self) -> Arc<Registry>;
    /// The snapshot last built, without catching up.
    fn current(&self) -> Arc<Registry>;
    /// Changes every time the snapshot does: what a session that can push
    /// sends `notifications/tools/list_changed` on.
    fn changes(&self) -> tokio::sync::watch::Receiver<u64>;
    /// How often such a session asks [`ToolSource::tools`] unprompted, so a
    /// change another process made reaches its client too.
    fn poll_interval(&self) -> std::time::Duration;
}

/// The notification that tells a client to list the tools again.
pub fn tools_list_changed_notification() -> Value {
    serde_json::json!({ "jsonrpc": "2.0", "method": "notifications/tools/list_changed" })
}

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

/// `registry` without its UI-only capabilities (`Capability::ui_only`, #575).
/// Dropped where the server takes its registry, so that no path through it —
/// listing, calling, auditing, consent — can reach one.
fn without_ui_only(registry: Arc<Registry>) -> Arc<Registry> {
    if !registry.entries().any(|capability| capability.ui_only) {
        return registry;
    }
    let mut offered = (*registry).clone();
    let hidden: Vec<String> = offered
        .entries()
        .filter(|capability| capability.ui_only)
        .map(|capability| capability.id.clone())
        .collect();
    for id in hidden {
        offered.unregister(&id);
    }
    Arc::new(offered)
}

pub struct McpServer {
    registry: Arc<Registry>,
    /// Installed apps' tools, when the host has any (#574).
    app_tools: Option<Arc<dyn ToolSource>>,
    confirm_policy: Arc<dyn crate::policy::ConfirmPolicy>,
    audit: Arc<dyn crate::audit::AuditSink>,
    prompts: crate::prompts::PromptLibrary,
    kind_resolver: std::sync::Arc<dyn crate::resources::KindResolver>,
    watcher: std::sync::Arc<dyn crate::resources::ObjectWatcher>,
}

impl McpServer {
    pub fn new(registry: Arc<Registry>) -> Self {
        Self {
            registry: without_ui_only(registry),
            app_tools: None,
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

    /// Serve installed apps' tools beside the host's own (#574), and tell
    /// clients when they change.
    pub fn with_app_tools(mut self, tools: Arc<dyn ToolSource>) -> Self {
        self.app_tools = Some(tools);
        self
    }

    pub fn app_tools(&self) -> Option<&Arc<dyn ToolSource>> {
        self.app_tools.as_ref()
    }

    /// Whether `name` is an app's tool rather than one of the host's.
    fn is_app_tool(&self, name: &str) -> bool {
        self.app_tools.is_some()
            && name.starts_with(APP_TOOL_PREFIX)
            && self.registry.get(name).is_none()
    }

    /// The registry that holds `name` now, without catching up: the host's
    /// own, or the app tools as last built.
    fn holding(&self, name: &str) -> Arc<Registry> {
        match &self.app_tools {
            Some(tools) if self.is_app_tool(name) => tools.current(),
            _ => self.registry.clone(),
        }
    }

    /// The registry one call of `name` is decided and run against, after the
    /// app tools have caught up. A call holds it from its consent to its end:
    /// if the app changes in between, this snapshot's handler is revoked and
    /// the call refused, rather than run as a newer version of the tool the
    /// person was never asked about.
    pub async fn resolve(&self, name: &str) -> Arc<Registry> {
        match &self.app_tools {
            Some(tools) if self.is_app_tool(name) => tools.tools().await,
            _ => self.registry.clone(),
        }
    }

    /// Let the app tools catch up with any change they have not seen.
    pub async fn refresh_app_tools(&self) {
        if let Some(tools) = &self.app_tools {
            tools.tools().await;
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
        Self::is_sensitive_in(&self.holding(name), name)
    }

    /// [`McpServer::is_sensitive`], in the registry one call resolved to.
    pub fn is_sensitive_in(registry: &Registry, name: &str) -> bool {
        registry
            .get(name)
            .map(|c| c.annotations.sensitive)
            .unwrap_or(false)
    }

    /// The host's tools, then the app tools as last built (#574).
    pub fn list_tools(&self) -> Vec<ToolDescriptor> {
        let describe = |cap: &srelens_capability::Capability| ToolDescriptor {
            name: cap.id.clone(),
            description: cap.summary.clone(),
            input_schema: cap.input_schema.clone(),
            read_only: cap.annotations.read_only,
            destructive: cap.annotations.destructive,
        };
        let mut tools: Vec<ToolDescriptor> = self
            .registry
            .ids()
            .into_iter()
            .filter_map(|id| self.registry.get(id))
            .map(describe)
            .collect();
        if let Some(apps) = &self.app_tools {
            let apps = apps.current();
            tools.extend(
                apps.ids()
                    .into_iter()
                    .filter(|id| self.is_app_tool(id))
                    .filter_map(|id| apps.get(id))
                    .map(describe),
            );
        }
        tools
    }

    pub async fn call_tool(&self, name: &str, args: Value) -> Result<Value, CapabilityError> {
        self.resolve(name).await.invoke(name, args).await
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
        let registry = self.resolve(name).await;
        self.call_tool_audited_in(&registry, name, args, transport, decision)
            .await
    }

    /// [`McpServer::call_tool_audited`], in the registry the call resolved to.
    pub async fn call_tool_audited_in(
        &self,
        registry: &Registry,
        name: &str,
        args: Value,
        transport: Transport,
        decision: &'static str,
    ) -> Result<Value, CapabilityError> {
        registry
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
        Self::consent_kind_in(&self.holding(name), name)
    }

    /// [`McpServer::consent_kind`], in the registry one call resolved to.
    pub fn consent_kind_in(registry: &Registry, name: &str) -> Option<crate::policy::ConsentKind> {
        let cap = registry.get(name)?;
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
        Self::consent_request_in(&self.holding(name), name, args)
    }

    /// [`McpServer::consent_request`], in the registry one call resolved to:
    /// an app's tool is asked about under the annotations of the snapshot the
    /// call will run in.
    pub fn consent_request_in(
        registry: &Registry,
        name: &str,
        args: &Value,
    ) -> Option<crate::policy::ConsentRequest> {
        let kind = Self::consent_kind_in(registry, name)?;
        let annotations = registry.get(name)?.annotations;
        Some(crate::policy::ConsentRequest {
            tool: name.to_string(),
            args: args.clone(),
            kind,
            impact: annotations.impact,
            confirm_text: annotations.confirm_text(args),
            // The transport fills this in; the registry cannot know it.
            caller: None,
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

    /// PR #661 review (CodeRabbit, CWE-451). Three surfaces — the desktop
    /// modal, the assistant card and `AgentConsent` — render
    /// `ConsentRequest::prompt()` as text, so whatever reaches `confirm_text`
    /// is what a person reads before approving. The sanitising lives in
    /// `confirm_fields`; this pins it at the boundary those surfaces actually
    /// read from, and pins the other half of the trade: the caller's whole
    /// untouched value still arrives in `args`, which the dialogs show
    /// beneath the question.
    #[tokio::test]
    async fn a_hostile_name_reaches_the_policy_sanitised_in_the_sentence_and_whole_in_the_args() {
        let long = "z".repeat(400);
        let name = format!("api\u{202E}{long}");
        let mut reg = Registry::new();
        let mut cap = Capability::read_only("k8s.deleteResource", "deletes", |_| async {
            Ok(json!({}))
        });
        cap.annotations = srelens_capability::Annotations::DESTRUCTIVE;
        reg.register(cap);
        let server = McpServer::new(Arc::new(reg));

        let request = server
            .consent_request(
                "k8s.deleteResource",
                &json!({ "context": "prod", "kind": "Pod", "name": name }),
            )
            .expect("a destructive tool is gated");
        let prompt = request.prompt();

        assert!(
            !prompt.contains('\u{202E}'),
            "the override reached the question: {prompt:?}"
        );
        assert!(
            prompt.chars().count() < 200,
            "the question must stay readable, got {} chars",
            prompt.chars().count()
        );
        assert!(
            prompt.ends_with("in cluster prod?"),
            "the question must survive the name: {prompt:?}"
        );
        assert_eq!(
            request.args["name"], name,
            "the caller's whole value still reaches the arguments block"
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

    /// An app's logs and runtime metrics are for srelens's own UI (#575): an
    /// agent's context goes to its LLM provider, and a sidecar's stderr is text
    /// a third party wrote. So a UI-only capability is neither a tool nor
    /// callable as one, on any path the server has.
    #[tokio::test]
    async fn a_ui_only_capability_is_neither_listed_nor_callable() {
        let mut reg = Registry::new();
        reg.register(Capability::read_only("ping", "health check", |v| async move {
            Ok(json!({ "echo": v }))
        }));
        reg.register(
            Capability::read_only("app.logs", "an app's log", |_| async {
                Ok(json!({"lines": ["srelens: the extension is running"]}))
            })
            .only_in_the_ui(),
        );
        let server = McpServer::new(Arc::new(reg));

        let names: Vec<String> = server.list_tools().into_iter().map(|t| t.name).collect();
        assert_eq!(names, ["ping"]);
        assert!(matches!(
            server.call_tool("app.logs", json!({})).await,
            Err(CapabilityError::NotFound(_))
        ));
        assert!(matches!(
            server
                .call_tool_audited("app.logs", json!({}), Transport::Http, "auto")
                .await,
            Err(CapabilityError::NotFound(_))
        ));
        assert!(!server.is_sensitive("app.logs"));
        assert_eq!(server.consent_kind("app.logs"), None);
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
