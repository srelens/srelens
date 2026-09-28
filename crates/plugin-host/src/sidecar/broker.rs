//! The capability broker a sidecar calls back into (#573).
//!
//! A sidecar gets Kubernetes and external data the way a declarative app does,
//! and only that way: through the app facade the host UI calls,
//! `extensions.read`, `extensions.resource` and `extensions.action`
//! (`crates/registry/src/extensions`). Those capabilities check, on every
//! call, that the app is installed, enabled and at this revision, that its
//! grants cover the binding, that it is enabled for the cluster, and they reach
//! the cluster with the user's own credentials, so its RBAC answers. There is
//! no second path: this module only builds their input and passes it through
//! [`Registry::invoke_audited`].
//!
//! What it adds is what a call from a process, rather than a click, needs:
//!
//! - **Identity from the host.** The app's ID and revision come from the
//!   supervisor that started the process ([`AppIdentity`]), never from the
//!   sidecar, which cannot name either: a call naming `id` or `revision` is
//!   refused.
//! - **An explicit context.** Every call carries `context: {clusterId,
//!   namespace}`. There is no current cluster to fall back on; a call without
//!   one is refused before anything runs.
//! - **Confirmation.** A call the host gates (`requires_confirm` or
//!   `destructive`, the rule MCP's gate reads) is put to a person through
//!   [`Consent`] first, with the host's own sentence. Declined, it never runs
//!   and is recorded as denied.
//! - **An audit record** for every write, with `source` `app` and the app's
//!   ID and revision (#555). An approved call runs to its end on a task of its
//!   own, so a sidecar that cancels or exits cannot leave a write unrecorded.
//!
//! No credential crosses. The facade returns what the app declared, never a
//! kubeconfig, a token or a Secret's value; `network.http`'s secrets are
//! injected by the host and never answered (#568); and a capability marked
//! `sensitive` is refused outright. The methods and their parameters are in
//! `docs/extensions/sidecar-protocol.md`, "Calls from the sidecar".

use serde_json::{json, Map, Value};
use srelens_capability::audit::{self, AuditSink, Source};
use srelens_capability::{CapabilityError, Impact, Registry};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use super::connection::Broker;
use super::protocol::{code, method, RpcError};
pub use srelens_sidecar_protocol::CallContext;
use srelens_sidecar_protocol::{
    MAX_CLUSTER_ID_BYTES, MAX_IDENTIFIER_LEN, MAX_NAMESPACE_LEN, MAX_OBJECT_NAME_LEN, MAX_TOKEN_LEN,
};

/// Who the sidecar is: the installed app its supervisor started it for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppIdentity {
    pub id: String,
    /// The installed revision. An update or a disable makes every call refuse
    /// until the host starts the sidecar of the new revision.
    pub revision: u64,
    /// The app's name, for the confirmation.
    pub name: String,
    /// Who signed it, or `None` for an unsigned app, for the confirmation's
    /// "Requested by app `<name>` (`<publisher>` / unsigned)" (#552).
    pub publisher: Option<String>,
}

/// One call a person must confirm before it runs.
#[derive(Debug, Clone, PartialEq)]
pub struct ConsentRequest {
    /// The app asking. Unlike an MCP caller's, this is known: the supervisor
    /// started the process that asked.
    pub app: AppIdentity,
    /// The host capability that will run.
    pub tool: String,
    /// Exactly what it will be given.
    pub args: Value,
    /// The host's own level for it.
    pub impact: Impact,
    /// The host's sentence for this call, rendered from the capability's
    /// confirmation template; `None` when it has none.
    pub confirm_text: Option<String>,
    pub cluster_id: String,
    pub namespace: Option<String>,
}

impl ConsentRequest {
    /// What to put in front of a person: the host's sentence, else a plain
    /// statement of what runs.
    pub fn prompt(&self) -> String {
        self.confirm_text.clone().unwrap_or_else(|| {
            format!(
                "Allow {} to run `{}` in cluster {}?",
                self.app.name, self.tool, self.cluster_id
            )
        })
    }
}

/// Who decides whether a sidecar's gated call may run. The desktop answers
/// with the single host confirmation (#552); `Err` carries why not, which the
/// sidecar is told.
pub trait Consent: Send + Sync + 'static {
    fn confirm<'a>(
        &'a self,
        request: &'a ConsentRequest,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;
}

/// The default: nothing can ask a person, so nothing gated runs.
pub struct NoConsent;

impl Consent for NoConsent {
    fn confirm<'a>(
        &'a self,
        request: &'a ConsentRequest,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            Err(format!(
                "`{}` needs a person's confirmation, and this srelens process has no way to ask for it",
                request.tool
            ))
        })
    }
}

/// The broker for one app's sidecar.
pub struct CapabilityBroker {
    registry: Arc<Registry>,
    app: AppIdentity,
    audit: Arc<dyn AuditSink>,
    consent: Arc<dyn Consent>,
}

impl CapabilityBroker {
    /// `registry` is the host's, with the `extensions.*` facade in it. Every
    /// write is recorded to `audit`, and every gated call put to `consent`.
    pub fn new(
        registry: Arc<Registry>,
        app: AppIdentity,
        audit: Arc<dyn AuditSink>,
        consent: Arc<dyn Consent>,
    ) -> CapabilityBroker {
        CapabilityBroker {
            registry,
            app,
            audit,
            consent,
        }
    }

    async fn serve(&self, name: &str, params: Value) -> Result<Value, RpcError> {
        let (tool, context, args) = self.input(name, params)?;
        let Some(annotations) = self.registry.get(tool).map(|c| c.annotations) else {
            return Err(RpcError::new(
                code::METHOD_NOT_FOUND,
                format!("srelens serves no `{tool}` here, so `{name}` cannot be answered"),
            ));
        };
        // The facade carries no such capability today: keeping a Secret out
        // rests on its readers. This keeps one out should it ever carry one.
        if annotations.sensitive {
            return Err(RpcError::new(
                code::METHOD_NOT_FOUND,
                format!("`{tool}` returns sensitive material, which srelens never gives a sidecar"),
            ));
        }
        if !(annotations.requires_confirm || annotations.destructive) {
            // Awaited here, so a call the sidecar cancels stops, and the
            // limit on calls in flight bounds what it can start.
            return self
                .registry
                .invoke_audited(tool, args, self.audit.as_ref(), Source::Sidecar, "auto")
                .await
                .map_err(|error| self.refusal(error));
        }
        if tool == "extensions.action" {
            self.declared_action(&args).await?;
        }
        let request = ConsentRequest {
            app: self.app.clone(),
            tool: tool.to_owned(),
            args: args.clone(),
            impact: annotations.impact,
            confirm_text: annotations.confirm_text(&args),
            cluster_id: context.cluster_id.clone(),
            namespace: context.namespace.clone(),
        };
        if let Err(why) = self.consent.confirm(&request).await {
            self.record_denied(tool, &args, annotations.sensitive, &why);
            return Err(RpcError::new(code::CONSENT_DENIED, why));
        }
        // Approved: on a task of its own, so once it starts it finishes and is
        // recorded, whether or not the sidecar is still waiting for the answer.
        // Each one took a person's yes, which is what bounds how many run.
        let registry = self.registry.clone();
        let sink = self.audit.clone();
        let tool = tool.to_owned();
        tokio::spawn(async move {
            registry
                .invoke_audited(&tool, args, sink.as_ref(), Source::Sidecar, "approved")
                .await
        })
        .await
        .map_err(|e| {
            RpcError::new(
                code::INTERNAL_ERROR,
                format!("srelens's call for the extension failed: {e}"),
            )
        })?
        .map_err(|error| self.refusal(error))
    }

    /// The facade's refusal as the sidecar is told it.
    fn refusal(&self, error: CapabilityError) -> RpcError {
        let declared = format!("plugin/{}/", self.app.id);
        match error {
            // The facade looks a binding up by the app's own capability ID.
            CapabilityError::NotFound(id) if id.starts_with(&declared) => RpcError::new(
                code::INVALID_PARAMS,
                format!(
                    "{} declares no capability `{}`",
                    self.app.name,
                    &id[declared.len()..]
                ),
            ),
            CapabilityError::NotFound(id) => RpcError::new(
                code::METHOD_NOT_FOUND,
                format!("srelens serves no `{id}` here"),
            ),
            CapabilityError::InvalidInput(why) => RpcError::new(code::INVALID_PARAMS, why),
            CapabilityError::Handler(why) => RpcError::new(code::CAPABILITY_FAILED, why),
        }
    }

    /// Before anyone is asked: the object is read through the facade, which
    /// makes every check the action will, and reports the actions the app
    /// declares for it. A person is asked only about one of those, on an
    /// object that exists.
    async fn declared_action(&self, args: &Value) -> Result<(), RpcError> {
        let detail = self
            .registry
            .invoke("extensions.resource", args["resource"].clone())
            .await
            .map_err(|error| self.refusal(error))?;
        let declared = detail["actions"]
            .as_array()
            .is_some_and(|actions| actions.iter().any(|a| *a == args["action"]));
        if declared {
            return Ok(());
        }
        Err(invalid(format!(
            "{} declares no action `{}` for `{}`",
            self.app.name,
            args["action"].as_str().unwrap_or_default(),
            args["resource"]["capability"].as_str().unwrap_or_default()
        )))
    }

    /// A refused confirmation is not an invocation, so it is recorded here, as
    /// MCP records its own (`crates/mcp/src/stdio.rs`).
    fn record_denied(&self, tool: &str, args: &Value, sensitive: bool, why: &str) {
        let redacted = audit::redact(args, sensitive);
        let (app, cluster, resource) = audit::describe_call_target(tool, args, &redacted);
        self.audit.record(audit::AuditRecord {
            source: Source::Sidecar,
            tool: tool.to_owned(),
            app,
            cluster,
            resource,
            decision: "denied",
            outcome: audit::OUTCOME_REJECTED,
            error: Some(audit::redact_call_error(tool, why, args, &redacted)),
            args: redacted,
        });
    }

    /// The facade capability `name` calls, the context, and its input, with
    /// the app's identity and the context put in by the host.
    fn input(
        &self,
        name: &str,
        params: Value,
    ) -> Result<(&'static str, CallContext, Value), RpcError> {
        let HostCall {
            tool,
            context,
            fields: given,
        } = host_call(name, params)?;
        let namespace = context.namespace.clone().unwrap_or_default();
        let selection = || {
            json!({
                "id": self.app.id,
                "revision": self.app.revision,
                "capability": given["capability"],
                "context": context.cluster_id,
                "namespace": namespace,
            })
        };
        let args = match tool {
            "extensions.read" => selection(),
            "extensions.resource" => {
                let mut selection = selection();
                selection["name"] = given["name"].clone();
                selection
            }
            _ => {
                let mut resource = selection();
                resource["name"] = given["name"].clone();
                json!({
                    "resource": resource,
                    "action": given["action"],
                    "uid": given["uid"],
                    "resourceVersion": given["resourceVersion"],
                })
            }
        };
        Ok((tool, context, args))
    }
}

/// A `host/*` call held to the protocol's shapes: the facade capability that
/// answers it, the cluster it names, and its fields. Knows nothing of the
/// app; the broker adds who is asking.
#[derive(Debug)]
pub(crate) struct HostCall {
    pub tool: &'static str,
    pub context: CallContext,
    pub fields: Map<String, Value>,
}

/// Read one call a sidecar made to the host, or say why it is refused, with
/// the same codes and sentences as always. The committed protocol schema
/// takes exactly what this takes (see the conformance test).
pub(crate) fn host_call(name: &str, params: Value) -> Result<HostCall, RpcError> {
    let (tool, fields): (&'static str, &[&str]) = match name {
        method::HOST_READ => ("extensions.read", &["capability"]),
        method::HOST_RESOURCE => ("extensions.resource", &["capability", "name"]),
        method::HOST_ACTION => (
            "extensions.action",
            &["capability", "name", "action", "uid", "resourceVersion"],
        ),
        _ => {
            return Err(RpcError::new(
                code::METHOD_NOT_FOUND,
                format!(
                    "srelens takes no `{name}` call from a sidecar; it takes {}, {} and {}",
                    method::HOST_READ,
                    method::HOST_RESOURCE,
                    method::HOST_ACTION
                ),
            ))
        }
    };
    let Value::Object(mut params) = params else {
        return Err(invalid(format!("`{name}` takes an object")));
    };
    let context = call_context(params.remove("context"))?;
    let mut given = Map::new();
    for (key, value) in params {
        if !fields.contains(&key.as_str()) {
            return Err(invalid(format!(
                "`{name}` takes {}, and not `{key}`",
                std::iter::once("context")
                    .chain(fields.iter().copied())
                    .map(|f| format!("`{f}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        let Value::String(_) = value else {
            return Err(invalid(format!("`{name}`: `{key}` must be a string")));
        };
        given.insert(key, value);
    }
    if let Some(missing) = fields.iter().find(|f| !given.contains_key(**f)) {
        return Err(invalid(format!("`{name}` needs `{missing}`")));
    }
    for (field, value) in &given {
        let value = value.as_str().unwrap_or_default();
        let (fits, shape) = match field.as_str() {
            "capability" | "action" => (is_identifier(value), IDENTIFIER),
            "name" => (is_object_name(value), OBJECT_NAME),
            _ => (is_token(value), TOKEN),
        };
        if !fits {
            return Err(invalid(format!("`{name}`: `{field}` must be {shape}")));
        }
    }
    Ok(HostCall {
        tool,
        context,
        fields: given,
    })
}

impl Broker for CapabilityBroker {
    fn call<'a>(
        &'a self,
        method: &'a str,
        params: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, RpcError>> + Send + 'a>> {
        Box::pin(self.serve(method, params))
    }
}

fn invalid(message: String) -> RpcError {
    RpcError::new(code::INVALID_PARAMS, message)
}

const CONTEXT_SHAPE: &str =
    "`context` must be {\"clusterId\": \"<cluster>\", \"namespace\": \"<namespace>\" or null}";

/// The explicit context of one call, or why it is not one.
fn call_context(context: Option<Value>) -> Result<CallContext, RpcError> {
    let Some(context) = context.filter(|c| !c.is_null()) else {
        return Err(invalid(format!(
            "Every call names its cluster: {CONTEXT_SHAPE}. srelens has no current cluster to assume"
        )));
    };
    if !context.is_object() {
        return Err(invalid(format!("{CONTEXT_SHAPE}, not {}", kind(&context))));
    }
    let parsed: CallContext =
        serde_json::from_value(context).map_err(|e| invalid(format!("{CONTEXT_SHAPE} ({e})")))?;
    if parsed.cluster_id.trim().is_empty() || parsed.cluster_id.len() > MAX_CLUSTER_ID_BYTES {
        return Err(invalid(format!(
            "`context.clusterId` must name a cluster, in at most {MAX_CLUSTER_ID_BYTES} bytes"
        )));
    }
    if let Some(namespace) = &parsed.namespace {
        if !is_namespace(namespace) {
            return Err(invalid(format!(
                "`context.namespace` must be a Kubernetes namespace name, or null for none; not {:?}",
                namespace.chars().take(64).collect::<String>()
            )));
        }
    }
    Ok(parsed)
}

/// What a JSON value is, for a refusal: "a string", "an array", …
fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "true or false",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

const IDENTIFIER: &str = "1 to 64 ASCII letters, digits and hyphens, as the manifest names it";
const OBJECT_NAME: &str =
    "a Kubernetes object name: 1 to 253 ASCII letters, digits, dots and hyphens";
const TOKEN: &str = "1 to 128 printable ASCII characters, as the object carries it";

/// A binding or action name, as the manifest holds one to (`identifier` in
/// `manifest.rs`).
fn is_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_IDENTIFIER_LEN
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// An object name, as `ResourceIn::validate` (`crates/kube/src/gitops.rs`)
/// holds one to.
fn is_object_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_OBJECT_NAME_LEN
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
}

/// A `uid` or `resourceVersion`: short, and nothing that is not a visible
/// character.
fn is_token(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_TOKEN_LEN && value.bytes().all(|b| b.is_ascii_graphic())
}

/// A Kubernetes namespace name: an RFC 1123 label. The rule `extensions.read`
/// holds a namespace to.
fn is_namespace(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAMESPACE_LEN
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !name.starts_with('-')
        && !name.ends_with('-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use srelens_capability::audit::{AuditRecord, AuditSink};
    use srelens_capability::{Annotations, Capability, CapabilityError, Impact};
    use std::sync::Mutex;

    const APP: &str = "org.example.scanner";

    fn app() -> AppIdentity {
        AppIdentity {
            id: APP.into(),
            revision: 7,
            name: "Scanner".into(),
            publisher: None,
        }
    }

    type Seen = Arc<Mutex<Vec<(String, Value)>>>;

    /// The host facade as stand-ins that record what they were called with.
    /// `extensions.read` answers by the binding it is asked for.
    fn registry(seen: &Seen) -> Registry {
        let mut registry = Registry::new();
        let s = seen.clone();
        registry.register(Capability::read_only(
            "extensions.read",
            "read",
            move |input| {
                let s = s.clone();
                async move {
                    s.lock()
                        .unwrap()
                        .push(("extensions.read".into(), input.clone()));
                    match input["capability"].as_str() {
                        Some("not-enabled") => Err(CapabilityError::Handler(
                            "App is not enabled for this cluster".into(),
                        )),
                        Some("bad-input") => Err(CapabilityError::InvalidInput(
                            "Namespace must be a Kubernetes namespace name".into(),
                        )),
                        // As the facade answers a binding the app does not declare.
                        Some("undeclared") => Err(CapabilityError::NotFound(format!(
                            "plugin/{APP}/undeclared"
                        ))),
                        _ => Ok(json!({"rows": [{"name": "web"}]})),
                    }
                }
            },
        ));
        let s = seen.clone();
        registry.register(Capability::read_only(
            "extensions.resource",
            "inspect",
            move |input| {
                let s = s.clone();
                async move {
                    s.lock()
                        .unwrap()
                        .push(("extensions.resource".into(), input));
                    Ok(json!({"object": {}, "actions": ["sync"]}))
                }
            },
        ));
        let s = seen.clone();
        let mut action = Capability::read_only("extensions.action", "act", move |input| {
            let s = s.clone();
            async move {
                s.lock().unwrap().push(("extensions.action".into(), input));
                Ok(json!({"ok": true}))
            }
        });
        // As `crates/registry/src/extensions/resource.rs` registers it.
        action.annotations = Annotations::MUTATING
            .with_impact(Impact::High)
            .with_confirm(
                "Run the declared action[ ({action})][ on {resource}][ in cluster {cluster}]?",
            );
        registry.register(action);
        registry
    }

    #[derive(Default)]
    struct Spy(Mutex<Vec<AuditRecord>>);

    impl AuditSink for Spy {
        fn record(&self, rec: AuditRecord) {
            self.0.lock().unwrap().push(rec);
        }
    }

    impl Spy {
        fn records(&self) -> Vec<AuditRecord> {
            self.0.lock().unwrap().clone()
        }
    }

    /// Says yes, and keeps what it was asked.
    #[derive(Default)]
    struct Approve(Mutex<Vec<ConsentRequest>>);

    impl Consent for Approve {
        fn confirm<'a>(
            &'a self,
            request: &'a ConsentRequest,
        ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
            self.0.lock().unwrap().push(request.clone());
            Box::pin(async { Ok(()) })
        }
    }

    struct Decline;

    impl Consent for Decline {
        fn confirm<'a>(
            &'a self,
            _request: &'a ConsentRequest,
        ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
            Box::pin(async { Err("The person declined".to_owned()) })
        }
    }

    struct Harness {
        broker: CapabilityBroker,
        seen: Seen,
        audit: Arc<Spy>,
    }

    fn harness(consent: Arc<dyn Consent>) -> Harness {
        let seen = Seen::default();
        let audit = Arc::new(Spy::default());
        let broker =
            CapabilityBroker::new(Arc::new(registry(&seen)), app(), audit.clone(), consent);
        Harness {
            broker,
            seen,
            audit,
        }
    }

    fn on(cluster: &str, namespace: Value) -> Value {
        json!({"clusterId": cluster, "namespace": namespace})
    }

    impl Harness {
        async fn call(&self, method: &str, params: Value) -> Result<Value, RpcError> {
            self.broker.call(method, params).await
        }

        fn seen(&self) -> Vec<(String, Value)> {
            self.seen.lock().unwrap().clone()
        }
    }

    #[tokio::test]
    async fn a_read_goes_to_extensions_read_as_this_app_on_the_cluster_it_names() {
        let h = harness(Arc::new(NoConsent));
        let params = json!({"context": on("prod", json!("team")), "capability": "applications"});
        let answer = h.call("host/read", params).await;
        assert_eq!(answer, Ok(json!({"rows": [{"name": "web"}]})));
        assert_eq!(
            h.seen(),
            [(
                "extensions.read".to_owned(),
                json!({"id": APP, "revision": 7, "capability": "applications",
                       "context": "prod", "namespace": "team"})
            )]
        );
    }

    #[tokio::test]
    async fn a_null_namespace_is_every_namespace_or_none() {
        let h = harness(Arc::new(NoConsent));
        let params = json!({"context": on("prod", Value::Null), "capability": "applications"});
        h.call("host/read", params).await.unwrap();
        assert_eq!(h.seen()[0].1["namespace"], "");
    }

    #[tokio::test]
    async fn a_resource_is_inspected_as_this_app_on_the_cluster_it_names() {
        let h = harness(Arc::new(NoConsent));
        let params = json!({"context": on("prod", json!("team")), "capability": "applications", "name": "web"});
        h.call("host/resource", params).await.unwrap();
        assert_eq!(
            h.seen(),
            [(
                "extensions.resource".to_owned(),
                json!({"id": APP, "revision": 7, "capability": "applications",
                       "context": "prod", "namespace": "team", "name": "web"})
            )]
        );
    }

    #[tokio::test]
    async fn a_call_without_an_explicit_context_is_refused_before_anything_runs() {
        let h = harness(Arc::new(Approve::default()));
        for context in [
            None,
            Some(Value::Null),
            Some(json!("prod")),
            Some(json!({"namespace": "team"})),
            Some(json!({"clusterId": "", "namespace": "team"})),
            Some(json!({"clusterId": "   ", "namespace": "team"})),
            Some(json!({"clusterId": 7, "namespace": "team"})),
            Some(json!({"clusterId": "prod"})),
            Some(json!({"clusterId": "prod", "namespace": ""})),
            // The spelling Rust would write, not the one the protocol takes.
            Some(json!({"cluster_id": "prod", "namespace": "team"})),
            Some(json!({"clusterId": "prod", "namespace": 3})),
            Some(json!({"clusterId": "prod", "namespace": "Not_A_Namespace"})),
            Some(json!({"clusterId": "prod", "namespace": null, "current": true})),
            Some(json!({"clusterId": "x".repeat(4097), "namespace": null})),
        ] {
            for method in ["host/read", "host/resource", "host/action"] {
                let mut params = json!({"capability": "applications", "name": "web",
                    "action": "sync", "uid": "u", "resourceVersion": "1"});
                if method == "host/read" {
                    params = json!({"capability": "applications"});
                }
                if let Some(context) = &context {
                    params["context"] = context.clone();
                }
                let error = h.call(method, params).await.unwrap_err();
                assert_eq!(
                    error.code,
                    code::INVALID_PARAMS,
                    "{method} {context:?}: {error:?}"
                );
                assert!(
                    error.message.contains("context"),
                    "{method} {context:?}: {error}"
                );
            }
        }
        assert!(h.seen().is_empty(), "something ran: {:?}", h.seen());
        assert!(h.audit.records().is_empty());
    }

    /// A context that is not an object is refused in the protocol's words,
    /// naming what it was: never a Rust type name, and never read as a list.
    #[tokio::test]
    async fn a_context_that_is_not_an_object_is_refused_in_the_protocols_words() {
        let h = harness(Arc::new(NoConsent));
        for (context, what) in [
            (json!("prod"), "a string"),
            (json!(7), "a number"),
            (json!(true), "true or false"),
            (json!(["prod", "team"]), "an array"),
            (json!([]), "an array"),
        ] {
            let error = h
                .call(
                    "host/read",
                    json!({"context": context, "capability": "applications"}),
                )
                .await
                .unwrap_err();
            assert_eq!(error.code, code::INVALID_PARAMS, "{context}");
            assert_eq!(
                error.message,
                format!("{CONTEXT_SHAPE}, not {what}"),
                "{context}"
            );
        }
        assert!(h.seen().is_empty(), "something ran: {:?}", h.seen());
    }

    #[tokio::test]
    async fn a_missing_context_says_there_is_no_current_cluster() {
        let h = harness(Arc::new(NoConsent));
        let error = h
            .call("host/read", json!({"capability": "applications"}))
            .await
            .unwrap_err();
        assert!(error.message.contains("no current cluster"), "{error}");
        let error = h
            .call(
                "host/read",
                json!({"capability": "applications", "context": {"clusterId": "prod"}}),
            )
            .await
            .unwrap_err();
        assert!(error.message.contains("null"), "{error}");
    }

    #[tokio::test]
    async fn a_sidecar_cannot_name_another_app_a_revision_or_a_second_cluster() {
        let h = harness(Arc::new(Approve::default()));
        let context = on("prod", json!("team"));
        for extra in [
            json!({"id": "org.example.other"}),
            json!({"revision": 1}),
            json!({"cluster": "staging"}),
            json!({"namespace": "kube-system"}),
            json!({"card": "failing"}),
        ] {
            let mut params = json!({"context": context, "capability": "applications"});
            for (key, value) in extra.as_object().unwrap() {
                params[key] = value.clone();
            }
            let error = h.call("host/read", params).await.unwrap_err();
            assert_eq!(error.code, code::INVALID_PARAMS, "{extra}: {error:?}");
        }
        assert!(h.seen().is_empty(), "something ran: {:?}", h.seen());
    }

    #[tokio::test]
    async fn a_method_that_is_not_a_broker_call_is_refused_by_name() {
        let h = harness(Arc::new(Approve::default()));
        for method in [
            "k8s.getSecret",
            "k8s.listContexts",
            "extensions.configure",
            "extensions.read",
            "network.http",
            "host/exec",
            "",
        ] {
            let params = json!({"context": on("prod", Value::Null), "capability": "applications"});
            let error = h.call(method, params).await.unwrap_err();
            assert_eq!(error.code, code::METHOD_NOT_FOUND, "{method}: {error:?}");
            assert!(error.message.contains("host/read"), "{method}: {error}");
        }
        assert!(h.seen().is_empty(), "something ran: {:?}", h.seen());
    }

    fn action_params() -> Value {
        json!({"context": on("prod", json!("team")), "capability": "applications",
               "name": "web", "action": "sync", "uid": "u-1", "resourceVersion": "42"})
    }

    #[tokio::test]
    async fn an_action_is_put_to_a_person_first_in_the_hosts_own_words_then_run_and_recorded() {
        let consent = Arc::new(Approve::default());
        let h = harness(consent.clone());
        assert_eq!(
            h.call("host/action", action_params()).await,
            Ok(json!({"ok": true}))
        );

        let asked = consent.0.lock().unwrap().clone();
        assert_eq!(asked.len(), 1);
        let request = &asked[0];
        assert_eq!(request.app, app());
        assert_eq!(request.tool, "extensions.action");
        assert_eq!(request.impact, Impact::High);
        assert_eq!(request.cluster_id, "prod");
        assert_eq!(request.namespace.as_deref(), Some("team"));
        let sentence = request.prompt();
        assert!(
            sentence.contains("(sync)") && sentence.contains("in cluster prod"),
            "{sentence}"
        );

        let selection = json!({"id": APP, "revision": 7, "capability": "applications",
                "context": "prod", "namespace": "team", "name": "web"});
        let expected = json!({"resource": selection,
            "action": "sync", "uid": "u-1", "resourceVersion": "42"});
        // The object is read first, through the same facade, for the actions
        // it has: a person is asked only about one the app declares.
        assert_eq!(
            h.seen(),
            [
                ("extensions.resource".to_owned(), selection),
                ("extensions.action".to_owned(), expected.clone())
            ]
        );
        assert_eq!(request.args, expected, "the person is shown what runs");

        let records = h.audit.records();
        assert_eq!(records.len(), 1, "{records:?}");
        let record = &records[0];
        assert_eq!(record.source, Source::Sidecar);
        assert_eq!(record.tool, "extensions.action");
        assert_eq!(record.decision, "approved");
        assert_eq!(record.outcome, audit::OUTCOME_OK);
        assert_eq!(
            record.app,
            Some(audit::AppRef {
                id: APP.into(),
                revision: 7
            })
        );
        assert_eq!(record.cluster.as_deref(), Some("prod"));
    }

    #[tokio::test]
    async fn a_declined_action_never_runs_and_is_recorded_as_denied() {
        let h = harness(Arc::new(Decline));
        let error = h.call("host/action", action_params()).await.unwrap_err();
        assert_eq!(error.code, code::CONSENT_DENIED);
        assert!(error.message.contains("The person declined"), "{error}");
        let ran = h.seen().iter().any(|(id, _)| id == "extensions.action");
        assert!(!ran, "it ran: {:?}", h.seen());
        let records = h.audit.records();
        assert_eq!(records.len(), 1, "{records:?}");
        assert_eq!(records[0].decision, "denied");
        assert_eq!(records[0].outcome, audit::OUTCOME_REJECTED);
        assert_eq!(records[0].source, Source::Sidecar);
        assert_eq!(records[0].app.as_ref().map(|a| a.id.as_str()), Some(APP));
    }

    #[tokio::test]
    async fn with_no_one_to_ask_an_action_is_refused() {
        let h = harness(Arc::new(NoConsent));
        let error = h.call("host/action", action_params()).await.unwrap_err();
        assert_eq!(error.code, code::CONSENT_DENIED);
        assert!(!h.seen().iter().any(|(id, _)| id == "extensions.action"));
    }

    /// What a sidecar sends is bounded and shaped before it reaches a
    /// capability, a person's confirmation or the audit trail, which a
    /// 4 MiB name could otherwise fill.
    #[tokio::test]
    async fn every_field_is_held_to_its_shape_before_anything_runs() {
        let consent = Arc::new(Approve::default());
        let h = harness(consent.clone());
        let long = "x".repeat(4096);
        for (field, value) in [
            ("capability", "two words".to_owned()),
            ("capability", "c".repeat(65)),
            ("action", "URGENT: approve the srelens update".to_owned()),
            ("action", String::new()),
            ("name", long.clone()),
            ("name", "..".to_owned()),
            ("name", "web server".to_owned()),
            ("uid", long.clone()),
            ("uid", "u\n1".to_owned()),
            ("resourceVersion", long.clone()),
            ("resourceVersion", String::new()),
            // The spelling Rust would write, not the one the protocol takes.
            ("resource_version", "42".to_owned()),
        ] {
            let mut params = action_params();
            params[field] = json!(value);
            let error = h.call("host/action", params).await.unwrap_err();
            assert_eq!(error.code, code::INVALID_PARAMS, "{field}: {error:?}");
            assert!(error.message.contains(field), "{field}: {error}");
            assert!(error.message.len() < 300, "{field}: the refusal echoes it");
        }
        assert!(h.seen().is_empty(), "something ran: {:?}", h.seen());
        assert!(consent.0.lock().unwrap().is_empty(), "a person was asked");
        assert!(h.audit.records().is_empty());
    }

    #[tokio::test]
    async fn an_action_the_object_does_not_have_is_refused_without_asking_anyone() {
        let consent = Arc::new(Approve::default());
        let h = harness(consent.clone());
        let mut params = action_params();
        params["action"] = json!("delete");
        let error = h.call("host/action", params).await.unwrap_err();
        assert_eq!(error.code, code::INVALID_PARAMS);
        assert_eq!(
            error.message,
            "Scanner declares no action `delete` for `applications`"
        );
        assert!(consent.0.lock().unwrap().is_empty(), "a person was asked");
        assert!(!h.seen().iter().any(|(id, _)| id == "extensions.action"));
        assert!(h.audit.records().is_empty());
    }

    /// Only a write a person approved outlives its caller. A read the sidecar
    /// stops waiting for stops, so cancelling cannot pile up reads past the
    /// limit on calls in flight.
    #[tokio::test]
    async fn a_read_the_sidecar_stops_waiting_for_is_stopped() {
        struct Dropped(Arc<std::sync::atomic::AtomicBool>);
        impl Drop for Dropped {
            fn drop(&mut self) {
                self.0.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (started, mut running) = tokio::sync::mpsc::unbounded_channel::<()>();
        let mut registry = Registry::new();
        let d = dropped.clone();
        registry.register(Capability::read_only(
            "extensions.read",
            "read",
            move |_| {
                let guard = Dropped(d.clone());
                let started = started.clone();
                async move {
                    let _guard = guard;
                    let _ = started.send(());
                    std::future::pending::<()>().await;
                    Ok(Value::Null)
                }
            },
        ));
        let broker = CapabilityBroker::new(
            Arc::new(registry),
            app(),
            Arc::new(Spy::default()),
            Arc::new(NoConsent),
        );
        let params = json!({"context": on("prod", Value::Null), "capability": "applications"});
        {
            let call = broker.call("host/read", params);
            tokio::select! {
                _ = call => panic!("the read finished"),
                _ = running.recv() => {}
            }
        }
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert!(
            dropped.load(std::sync::atomic::Ordering::SeqCst),
            "the read went on without its caller"
        );
    }

    #[tokio::test]
    async fn a_read_is_not_put_to_a_person_and_not_recorded() {
        let consent = Arc::new(Approve::default());
        let h = harness(consent.clone());
        let params = json!({"context": on("prod", Value::Null), "capability": "applications"});
        h.call("host/read", params).await.unwrap();
        assert!(consent.0.lock().unwrap().is_empty());
        assert!(h.audit.records().is_empty());
    }

    #[tokio::test]
    async fn the_hosts_refusal_keeps_its_words_and_bad_input_is_invalid_params() {
        let h = harness(Arc::new(NoConsent));
        let read = |capability: &str| json!({"context": on("prod", Value::Null), "capability": capability});
        let error = h.call("host/read", read("not-enabled")).await.unwrap_err();
        assert_eq!(error.code, code::CAPABILITY_FAILED);
        assert_eq!(error.message, "App is not enabled for this cluster");
        let error = h.call("host/read", read("bad-input")).await.unwrap_err();
        assert_eq!(error.code, code::INVALID_PARAMS);
        assert_eq!(
            error.message,
            "Namespace must be a Kubernetes namespace name"
        );
    }

    #[tokio::test]
    async fn a_binding_the_app_does_not_declare_is_refused_in_the_apps_terms() {
        let h = harness(Arc::new(NoConsent));
        let params = json!({"context": on("prod", Value::Null), "capability": "undeclared"});
        let error = h.call("host/read", params).await.unwrap_err();
        assert_eq!(error.code, code::INVALID_PARAMS);
        assert_eq!(error.message, "Scanner declares no capability `undeclared`");
    }

    #[tokio::test]
    async fn a_host_without_the_app_facade_refuses_by_name() {
        let broker = CapabilityBroker::new(
            Arc::new(Registry::new()),
            app(),
            Arc::new(Spy::default()),
            Arc::new(NoConsent),
        );
        let params = json!({"context": on("prod", Value::Null), "capability": "applications"});
        let error = broker.call("host/read", params).await.unwrap_err();
        assert_eq!(error.code, code::METHOD_NOT_FOUND);
        assert!(error.message.contains("extensions.read"), "{error}");
    }

    #[tokio::test]
    async fn a_sensitive_read_is_never_answered_to_a_sidecar() {
        let seen = Seen::default();
        let mut registry = registry(&seen);
        let s = seen.clone();
        let mut read = Capability::read_only("extensions.read", "read", move |input| {
            let s = s.clone();
            async move {
                s.lock().unwrap().push(("extensions.read".into(), input));
                Ok(json!({"data": {"token": "c2VjcmV0"}}))
            }
        });
        read.annotations = Annotations::SENSITIVE_READ;
        registry.register(read);
        let consent = Arc::new(Approve::default());
        let broker = CapabilityBroker::new(
            Arc::new(registry),
            app(),
            Arc::new(Spy::default()),
            consent.clone(),
        );
        let params = json!({"context": on("prod", Value::Null), "capability": "applications"});
        let error = broker.call("host/read", params).await.unwrap_err();
        assert_eq!(error.code, code::METHOD_NOT_FOUND);
        assert!(error.message.contains("sensitive"), "{error}");
        assert!(seen.lock().unwrap().is_empty(), "it ran");
        assert!(
            consent.0.lock().unwrap().is_empty(),
            "no one should be asked"
        );
    }

    /// Once a person has approved it, an action is not cut off halfway, and
    /// its record is written, even when the sidecar stops waiting for it.
    #[tokio::test]
    async fn an_approved_action_runs_to_the_end_and_is_recorded_when_the_sidecar_stops_waiting() {
        let (started, mut running) = tokio::sync::mpsc::unbounded_channel::<()>();
        let release = Arc::new(tokio::sync::Notify::new());
        let mut registry = Registry::new();
        let r = release.clone();
        let mut action = Capability::read_only("extensions.action", "act", move |_| {
            let started = started.clone();
            let release = r.clone();
            async move {
                let _ = started.send(());
                release.notified().await;
                Ok(json!({"ok": true}))
            }
        });
        action.annotations = Annotations::MUTATING;
        registry.register(action);
        registry.register(Capability::read_only(
            "extensions.resource",
            "inspect",
            |_| async { Ok(json!({"actions": ["sync"]})) },
        ));
        let audit = Arc::new(Spy::default());
        let broker = CapabilityBroker::new(
            Arc::new(registry),
            app(),
            audit.clone(),
            Arc::new(Approve::default()),
        );
        let call = broker.call("host/action", action_params());
        tokio::select! {
            _ = call => panic!("the action finished before it was released"),
            _ = running.recv() => {}
        }
        // The call's future is gone: the sidecar cancelled, or it exited.
        release.notify_one();
        let recorded = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if let Some(record) = audit.records().pop() {
                    return record;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the action was recorded");
        assert_eq!(recorded.outcome, audit::OUTCOME_OK);
        assert_eq!(recorded.decision, "approved");
    }

    /// Calls to `method`: a valid one, then that one with each field and the
    /// context changed to a value at, inside or past its bounds, removed, or
    /// joined by one it does not take. ASCII only: the schema counts
    /// characters where the broker counts bytes, which differ only past ASCII.
    fn conformance_cases(method: &str) -> Vec<Value> {
        let fields: &[&str] = match method {
            "host/read" => &["capability"],
            "host/resource" => &["capability", "name"],
            _ => &["capability", "name", "action", "uid", "resourceVersion"],
        };
        let mut base = json!({"context": {"clusterId": "prod", "namespace": "team"}});
        for field in fields {
            base[*field] = json!(match *field {
                "capability" => "applications",
                "name" => "web",
                "action" => "sync",
                "uid" => "u-1",
                _ => "42",
            });
        }
        let mut cases = vec![base.clone()];
        for field in fields {
            let values = match *field {
                "capability" | "action" => vec![
                    json!("a"),
                    json!("a".repeat(64)),
                    json!("a".repeat(65)),
                    json!("Two-Words"),
                    json!("two words"),
                    json!("under_score"),
                    json!("dot.ted"),
                    json!(""),
                    json!(7),
                ],
                "name" => vec![
                    json!("a"),
                    json!("a".repeat(253)),
                    json!("a".repeat(254)),
                    json!("web.v1-2"),
                    json!("."),
                    json!(".."),
                    json!("..."),
                    json!("web server"),
                    json!("web/1"),
                    json!(""),
                    Value::Null,
                ],
                _ => vec![
                    json!("!"),
                    json!("~".repeat(128)),
                    json!("x".repeat(129)),
                    json!("u 1"),
                    json!("u\n1"),
                    json!("u\u{7f}"),
                    json!(""),
                    json!(1),
                ],
            };
            for value in values {
                let mut case = base.clone();
                case[*field] = value;
                cases.push(case);
            }
            let mut missing = base.clone();
            missing.as_object_mut().unwrap().remove(*field);
            cases.push(missing);
        }
        for context in [
            json!({"clusterId": "prod", "namespace": null}),
            json!({"clusterId": "  prod", "namespace": "team"}),
            json!({"clusterId": "x".repeat(4096), "namespace": "a"}),
            json!({"clusterId": "x".repeat(4097), "namespace": "a"}),
            json!({"clusterId": "", "namespace": "team"}),
            json!({"clusterId": "   ", "namespace": "team"}),
            json!({"clusterId": 7, "namespace": "team"}),
            json!({"clusterId": "prod"}),
            json!({"namespace": "team"}),
            json!({"clusterId": "prod", "namespace": ""}),
            json!({"clusterId": "prod", "namespace": "a".repeat(63)}),
            json!({"clusterId": "prod", "namespace": "a".repeat(64)}),
            json!({"clusterId": "prod", "namespace": "team-1"}),
            json!({"clusterId": "prod", "namespace": "-team"}),
            json!({"clusterId": "prod", "namespace": "team-"}),
            json!({"clusterId": "prod", "namespace": "Team"}),
            json!({"clusterId": "prod", "namespace": "te.am"}),
            json!({"clusterId": "prod", "namespace": 3}),
            json!({"cluster_id": "prod", "namespace": "team"}),
            json!({"clusterId": "prod", "namespace": null, "current": true}),
            json!("prod"),
            json!(["prod", "team"]),
            json!(["prod", null]),
            Value::Null,
        ] {
            let mut case = base.clone();
            case["context"] = context;
            cases.push(case);
        }
        let mut no_context = base.clone();
        no_context.as_object_mut().unwrap().remove("context");
        cases.push(no_context);
        for extra in ["id", "revision", "cluster", "namespace", "resource_version"] {
            let mut case = base.clone();
            case[extra] = json!("x");
            cases.push(case);
        }
        cases.extend([json!([]), json!("host/read"), Value::Null, json!({})]);
        cases
    }

    /// The committed schema and the broker agree on every `host/*` call: what
    /// the broker takes, the schema allows, and what it refuses, the schema
    /// refuses. Otherwise the schema's patterns could drift from
    /// `is_identifier` and the rest, and an SDK that checks its calls against
    /// the schema would send ones srelens refuses.
    #[test]
    fn the_protocol_schema_takes_exactly_the_calls_the_broker_takes() {
        let root = srelens_sidecar_protocol::schema();
        let (mut accepted, mut refused) = (0, 0);
        for (method, definition) in [
            (method::HOST_READ, "HostReadParams"),
            (method::HOST_RESOURCE, "HostResourceParams"),
            (method::HOST_ACTION, "HostActionParams"),
        ] {
            let validator = jsonschema::draft7::new(&json!({
                "definitions": root["definitions"],
                "allOf": [{"$ref": format!("#/definitions/{definition}")}],
            }))
            .expect("the schema compiles");
            for params in conformance_cases(method) {
                let broker = host_call(method, params.clone());
                let schema = validator.is_valid(&params);
                assert_eq!(
                    broker.is_ok(),
                    schema,
                    "{method} {params}: the broker says {broker:?}, the schema says {schema}"
                );
                if schema {
                    accepted += 1;
                } else {
                    refused += 1;
                }
            }
        }
        // Guards the guard: both verdicts are exercised.
        assert!(accepted >= 30, "{accepted} accepted");
        assert!(refused >= 100, "{refused} refused");
    }
}
