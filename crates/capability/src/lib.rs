//! Capability registry — the single source of truth for backend operations.

mod annotations;
pub mod audit;
mod error;
mod predicate;
pub mod settings;
pub mod status;
mod text;

pub use predicate::{
    check_path, check_predicates, resolve, unmet, CardPredicate, Condition, Predicate, ReferenceFormat,
    MAX_PREDICATES,
};

pub use annotations::{
    check_confirm_template, confirm_fields, render_confirm, Annotations, ConfirmFields, Impact,
    CONFIRM_FIELDS, CONFIRM_FIELD_MAX_CHARS,
};
pub use error::CapabilityError;
pub use text::{escape_invisible, is_format_character};

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::Value;

pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;
pub type Handler =
    Arc<dyn Fn(Value) -> BoxFuture<Result<Value, CapabilityError>> + Send + Sync>;

/// A capability's own rules for the arguments a manifest may bind to it, for
/// the rules an input schema cannot state. Returns why the binding is not one
/// this capability accepts.
pub type BoundArguments =
    Arc<dyn Fn(&serde_json::Map<String, Value>) -> Result<(), String> + Send + Sync>;

#[derive(Clone)]
pub struct Capability {
    pub id: String,
    pub summary: String,
    pub annotations: Annotations,
    pub input_schema: Value,
    pub output_schema: Value,
    pub handler: Handler,
    /// What a manifest may bind to this capability, beyond the input schema.
    ///
    /// A schema says a field is a string; it cannot say that the string is one
    /// of two substitution tokens, that a JSON pointer stays under `spec`, or
    /// that a merge patch does not touch `metadata.finalizers`. Those rules
    /// live with the handler that enforces them (`srelens_kube`'s action
    /// primitives), and the broker runs the same closure where a binding is
    /// accepted — at install, and again when a stored app is reverified — so a
    /// manifest cannot be admitted holding a template the handler would refuse.
    ///
    /// `None` for a capability with no such rule, which is all of them but the
    /// action primitives.
    pub bound_arguments: Option<BoundArguments>,
    /// The arguments a manifest may fill from one of the app's settings
    /// (#542), written `${settings.<id>}`, and the setting types each takes.
    /// Empty for almost every capability: interpolation anywhere else is
    /// refused at install. See [`settings`].
    pub settable: Vec<settings::Settable>,
}

impl Capability {
    /// Build a read-only capability from an async closure.
    pub fn read_only<F, Fut>(id: &str, summary: &str, f: F) -> Self
    where
        F: Fn(Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Value, CapabilityError>> + Send + 'static,
    {
        Self {
            id: id.to_string(),
            summary: summary.to_string(),
            annotations: Annotations::READ_ONLY,
            input_schema: Value::Null,
            output_schema: Value::Null,
            handler: Arc::new(move |v| Box::pin(f(v))),
            bound_arguments: None,
            settable: Vec::new(),
        }
    }

    pub fn typed<I, O, F, Fut>(
        id: &str,
        summary: &str,
        annotations: Annotations,
        f: F,
    ) -> Self
    where
        I: serde::de::DeserializeOwned + schemars::JsonSchema,
        O: serde::Serialize + schemars::JsonSchema,
        F: Fn(I) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<O, CapabilityError>> + Send + 'static,
    {
        let input_schema = serde_json::to_value(schemars::schema_for!(I)).unwrap();
        let output_schema = serde_json::to_value(schemars::schema_for!(O)).unwrap();
        let handler: Handler = Arc::new(move |v: Value| {
            let parsed = serde_json::from_value::<I>(v);
            let fut = parsed.map(&f);
            Box::pin(async move {
                let input = fut.map_err(|e| CapabilityError::InvalidInput(e.to_string()))?;
                let out = input.await?;
                serde_json::to_value(out).map_err(|e| CapabilityError::Handler(e.to_string()))
            })
        });
        Self {
            id: id.to_string(),
            summary: summary.to_string(),
            annotations,
            input_schema,
            output_schema,
            handler,
            bound_arguments: None,
            settable: Vec::new(),
        }
    }

    /// The same capability, with its own rules for what a manifest may bind to
    /// it. See [`Capability::bound_arguments`].
    pub fn checking_bound_arguments<F>(mut self, check: F) -> Self
    where
        F: Fn(&serde_json::Map<String, Value>) -> Result<(), String> + Send + Sync + 'static,
    {
        self.bound_arguments = Some(Arc::new(check));
        self
    }

    /// The same capability, letting a manifest fill `argument` from a setting
    /// of one of the `accepts` types. `stand_in` is a value this capability's
    /// `bound_arguments` rule accepts there (see [`settings::Settable`]).
    ///
    /// # Panics
    /// When `accepts` names [`settings::SettingType::SecretReference`]: a
    /// secret is injected by the host's secret store (#543), never written
    /// into an argument where a handler, a log line or a cluster object could
    /// keep it. That is a host programming error, caught by the first test
    /// that builds the capability.
    pub fn with_settable(
        mut self,
        argument: &str,
        accepts: &[settings::SettingType],
        stand_in: Value,
    ) -> Self {
        assert!(
            !accepts.contains(&settings::SettingType::SecretReference),
            "{}: a secret-reference setting cannot be interpolated into `{argument}`",
            self.id
        );
        self.settable.push(settings::Settable {
            argument: argument.to_owned(),
            accepts: accepts.to_vec(),
            stand_in,
        });
        self
    }

    /// The settable position `argument`, if this capability marks one.
    pub fn settable_argument(&self, argument: &str) -> Option<&settings::Settable> {
        self.settable.iter().find(|s| s.argument == argument)
    }
}

#[derive(Default, Clone)]
pub struct Registry {
    caps: BTreeMap<String, Capability>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, cap: Capability) {
        self.caps.insert(cap.id.clone(), cap);
    }

    /// Remove a capability from this registry snapshot.
    pub fn unregister(&mut self, id: &str) -> Option<Capability> {
        self.caps.remove(id)
    }

    pub fn ids(&self) -> Vec<&str> {
        self.caps.keys().map(String::as_str).collect()
    }

    pub fn entries(&self) -> impl Iterator<Item = &Capability> {
        self.caps.values()
    }

    pub fn get(&self, id: &str) -> Option<&Capability> {
        self.caps.get(id)
    }

    pub async fn invoke(&self, id: &str, input: Value) -> Result<Value, CapabilityError> {
        let cap = self
            .caps
            .get(id)
            .ok_or_else(|| CapabilityError::NotFound(id.to_string()))?;
        (cap.handler)(input).await
    }

    /// Invoke a capability and write one audit record for the call.
    ///
    /// **This is the single writer of an invocation record, for both
    /// surfaces.** The MCP server used to build its own record in
    /// `handle_request` while the desktop bridge called [`Registry::invoke`]
    /// straight through, so a Flux reconcile clicked in the app left nothing
    /// behind while the identical call from an agent was logged (#555). The
    /// registry is where the two paths meet, so the record is written here and
    /// a new surface gets the trail by calling this instead of `invoke`.
    ///
    /// `decision` is the caller's consent verdict — `"auto"` when nothing
    /// needed confirming, `"approved"` when a policy said yes. A refusal never
    /// reaches here, because a refused call is not an invocation:
    /// [`crate::audit::AuditSink::record`] is called directly for those.
    ///
    /// What the record says about the result is read off the error, because
    /// "srelens would not do this" and "the cluster would not" are different
    /// answers to "did my sync happen?":
    ///
    /// - no error — [`audit::OUTCOME_OK`];
    /// - [`CapabilityError::NotFound`] or [`CapabilityError::InvalidInput`] —
    ///   [`audit::OUTCOME_REJECTED`], since nothing ran;
    /// - [`CapabilityError::Handler`] — [`audit::OUTCOME_FAILED`], the call ran
    ///   and did not finish.
    ///
    /// Arguments are redacted before anything is written, and the error text is
    /// scrubbed against that same redaction — a capability that refuses an
    /// argument tends to echo it back.
    pub async fn invoke_audited(
        &self,
        id: &str,
        input: Value,
        sink: &dyn audit::AuditSink,
        source: audit::Source,
        decision: &'static str,
    ) -> Result<Value, CapabilityError> {
        let annotations = self.caps.get(id).map(|c| c.annotations);
        // An unknown capability has no safety class to read, so the UI cannot
        // know whether it was a mutation; nothing was invoked either way, and
        // the bridge logs the refusal. MCP records it, as it records every
        // other call it is asked to make.
        let audited = match source {
            audit::Source::Ui => annotations.as_ref().is_some_and(audit::is_audited_from_ui),
            _ => true,
        };
        if !audited {
            return self.invoke(id, input).await;
        }
        let sensitive = annotations.is_some_and(|a| a.sensitive);
        let redacted = audit::redact(&input, sensitive);
        let called = self.invoke(id, input.clone()).await;
        let (app, cluster, resource) = audit::describe_call_target(id, &input, &redacted);
        sink.record(audit::AuditRecord {
            source,
            tool: id.to_string(),
            app,
            cluster,
            resource,
            decision,
            outcome: match called.as_ref().err() {
                None => audit::OUTCOME_OK,
                Some(CapabilityError::NotFound(_) | CapabilityError::InvalidInput(_)) => {
                    audit::OUTCOME_REJECTED
                }
                Some(CapabilityError::Handler(_)) => audit::OUTCOME_FAILED,
            },
            error: called
                .as_ref()
                .err()
                .map(|e| audit::redact_call_error(id, &e.to_string(), &input, &redacted)),
            args: redacted,
        });
        called
    }
}

/// Crate version sentinel used by the scaffold smoke test.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_non_empty() {
        assert!(!version().is_empty());
    }
}

#[cfg(test)]
mod registry_tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn invoke_returns_handler_output() {
        let mut reg = Registry::new();
        reg.register(Capability::read_only("ping", "health check", |input| async move {
            Ok(json!({ "echo": input }))
        }));

        let out = reg.invoke("ping", json!("hi")).await.unwrap();
        assert_eq!(out, json!({ "echo": "hi" }));
    }

    #[tokio::test]
    async fn invoke_unknown_id_is_not_found() {
        let reg = Registry::new();
        let err = reg.invoke("nope", json!(null)).await.unwrap_err();
        assert!(matches!(err, CapabilityError::NotFound(_)));
    }

    #[test]
    fn ids_lists_registered_capabilities() {
        let mut reg = Registry::new();
        reg.register(Capability::read_only("a", "", |_| async { Ok(json!(null)) }));
        reg.register(Capability::read_only("b", "", |_| async { Ok(json!(null)) }));
        let mut ids = reg.ids();
        ids.sort();
        assert_eq!(ids, vec!["a", "b"]);
    }

    #[derive(Default)]
    struct Spy(std::sync::Mutex<Vec<audit::AuditRecord>>);

    impl audit::AuditSink for Spy {
        fn record(&self, rec: audit::AuditRecord) {
            self.0.lock().unwrap().push(rec);
        }
    }

    impl Spy {
        fn seen(&self) -> Vec<audit::AuditRecord> {
            self.0.lock().unwrap().clone()
        }
    }

    fn reg_with_a_read_and_a_write() -> Registry {
        let mut reg = Registry::new();
        reg.register(Capability::read_only("k8s.listPods", "lists", |_| async {
            Ok(json!([]))
        }));
        let mut write = Capability::read_only("k8s.deletePod", "deletes", |_| async {
            Ok(json!({ "deleted": true }))
        });
        write.annotations = Annotations::DESTRUCTIVE;
        reg.register(write);
        reg
    }

    #[tokio::test]
    async fn install_audit_keeps_a_checked_app_id_without_manifest_values() {
        let mut reg = Registry::new();
        let mut install = Capability::read_only("extensions.configure", "install", |args| async move {
            if args["fail"] == true {
                Err(CapabilityError::InvalidInput("invalid credential hunter2".into()))
            } else {
                Ok(json!({}))
            }
        });
        install.annotations = Annotations::MUTATING;
        reg.register(install);
        let spy = Spy::default();
        for fail in [false, true] {
            let _ = reg.invoke_audited(
                "extensions.configure",
                json!({"action":"install","manifest":"{\"id\":\"org.example.app\",\"credential\":\"hunter2\"}","grants":[],"fail":fail}),
                &spy,
                audit::Source::McpStdio,
                "auto",
            ).await;
        }
        let seen = spy.seen();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].outcome, audit::OUTCOME_OK);
        assert_eq!(seen[1].outcome, audit::OUTCOME_REJECTED);
        for record in seen {
            assert_eq!(record.resource.as_deref(), Some("org.example.app"));
            assert_eq!(record.args["manifest"], "<redacted>");
            assert!(!record.error.as_deref().unwrap_or("").contains("hunter2"));
        }
    }

    /// The asymmetry #555 settles: MCP is a third party and every call it
    /// makes is an event, including the reads. The UI is the user, and only
    /// what they changed (or what secret they revealed) is.
    #[tokio::test]
    async fn mcp_records_every_call_and_the_ui_records_only_the_ones_that_matter() {
        let reg = reg_with_a_read_and_a_write();
        let spy = Spy::default();

        for source in [audit::Source::McpStdio, audit::Source::Ui] {
            reg.invoke_audited("k8s.listPods", json!({}), &spy, source, "auto")
                .await
                .unwrap();
        }

        let seen = spy.seen();
        assert_eq!(seen.len(), 1, "expected only the MCP read, got {seen:?}");
        assert_eq!(seen[0].source, audit::Source::McpStdio);
    }

    /// The same capability from either surface lands in the trail in the same
    /// shape, differing only in who called it — which is the property that
    /// makes one pane able to show both.
    #[tokio::test]
    async fn a_write_is_recorded_identically_from_either_surface() {
        let reg = reg_with_a_read_and_a_write();
        let spy = Spy::default();
        let args = json!({ "context": "prod", "namespace": "team", "name": "web-0" });

        reg.invoke_audited("k8s.deletePod", args.clone(), &spy, audit::Source::Ui, "auto")
            .await
            .unwrap();
        reg.invoke_audited(
            "k8s.deletePod",
            args,
            &spy,
            audit::Source::McpHttp,
            "approved",
        )
        .await
        .unwrap();

        let seen = spy.seen();
        assert_eq!(seen.len(), 2, "both surfaces record, got {seen:?}");
        assert_eq!(seen[0].source.as_str(), "ui");
        assert_eq!(seen[1].source.as_str(), "mcp");
        assert_eq!(seen[0].decision, "auto");
        assert_eq!(seen[1].decision, "approved");
        for rec in &seen {
            assert_eq!(rec.tool, "k8s.deletePod");
            assert_eq!(rec.outcome, audit::OUTCOME_OK);
            assert_eq!(rec.cluster.as_deref(), Some("prod"));
            assert_eq!(rec.resource.as_deref(), Some("team/web-0"));
            assert!(rec.error.is_none());
        }
    }

    /// A capability the registry does not have never ran, so from the UI there
    /// is nothing to say about whether it mutated anything; MCP records the
    /// attempt, because an agent reaching for a tool it was not given is
    /// exactly what the trail is read for.
    #[tokio::test]
    async fn an_unknown_capability_is_recorded_for_mcp_and_not_for_the_ui() {
        let reg = reg_with_a_read_and_a_write();
        let spy = Spy::default();

        for source in [audit::Source::Ui, audit::Source::McpStdio] {
            assert!(reg
                .invoke_audited("k8s.nope", json!({}), &spy, source, "auto")
                .await
                .is_err());
        }

        let seen = spy.seen();
        assert_eq!(seen.len(), 1, "expected only the MCP attempt, got {seen:?}");
        assert_eq!(seen[0].outcome, audit::OUTCOME_REJECTED);
    }

    /// `k8s.helmRepoAdd` takes the repository URL as free text and is audited
    /// because it mutates, so a private chart repo added with its credentials
    /// in the URL — the form `helm repo add` documents — used to land on disk
    /// verbatim, from either surface. The record has to keep saying WHICH repo
    /// was added, so the host and the path stay and only the credentials go.
    #[tokio::test]
    async fn a_url_with_credentials_is_not_written_to_the_log() {
        let mut reg = Registry::new();
        let mut add = Capability::read_only("k8s.helmRepoAdd", "adds a repo", |_| async {
            Ok(json!({ "output": "\"internal\" has been added" }))
        });
        add.annotations = Annotations::MUTATING;
        reg.register(add);

        let dir = std::env::temp_dir().join(format!("srelens-pr660-url-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("audit.jsonl");
        let _ = std::fs::remove_file(&path);
        let log = audit::JsonlAuditLog::new(path.clone(), 5 * 1024 * 1024);

        reg.invoke_audited(
            "k8s.helmRepoAdd",
            json!({
                "context": "prod",
                "name": "internal",
                "url": "https://deploy:s3cr3t-pass@charts.example.com/stable?access_key=AKIAHUNTER2",
            }),
            &log,
            audit::Source::Ui,
            "approved",
        )
        .await
        .unwrap();

        let body = std::fs::read_to_string(&path).unwrap();
        for leaked in ["s3cr3t-pass", "deploy:", "AKIAHUNTER2"] {
            assert!(!body.contains(leaked), "{leaked} reached the log: {body}");
        }
        assert!(
            body.contains("charts.example.com/stable"),
            "the repo the record is about has to survive: {body}"
        );
    }

    #[test]
    fn get_returns_capability_for_registered_id() {
        let mut reg = Registry::new();
        reg.register(Capability::read_only("a", "", |_| async { Ok(json!(null)) }));
        assert!(reg.get("a").is_some());
        assert!(reg.get("missing").is_none());
    }
}

#[cfg(test)]
mod typed_tests {
    use super::*;
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};
    use serde_json::json;

    #[derive(Deserialize, JsonSchema)]
    struct AddIn { a: i64, b: i64 }
    #[derive(Serialize, JsonSchema)]
    struct AddOut { sum: i64 }

    #[tokio::test]
    async fn typed_capability_roundtrips_and_has_schema() {
        let cap = Capability::typed::<AddIn, AddOut, _, _>(
            "math.add", "adds two ints", Annotations::READ_ONLY,
            |input| async move { Ok(AddOut { sum: input.a + input.b }) },
        );
        assert!(cap.input_schema.get("properties").is_some());

        let mut reg = Registry::new();
        reg.register(cap);
        let out = reg.invoke("math.add", json!({ "a": 2, "b": 3 })).await.unwrap();
        assert_eq!(out, json!({ "sum": 5 }));
    }

    #[tokio::test]
    async fn typed_capability_rejects_bad_input() {
        let cap = Capability::typed::<AddIn, AddOut, _, _>(
            "math.add", "", Annotations::READ_ONLY,
            |i| async move { Ok(AddOut { sum: i.a + i.b }) },
        );
        let mut reg = Registry::new();
        reg.register(cap);
        let err = reg.invoke("math.add", json!({ "a": "x" })).await.unwrap_err();
        assert!(matches!(err, CapabilityError::InvalidInput(_)));
    }
}
