//! Bridges the capability registry to the Tauri WebView as a command.
//!
//! This is the Tauri half of "one definition, two surfaces": the same
//! `Registry` that backs the MCP server is invoked here from the frontend.

use serde_json::Value;
use srelens_capability::audit::{AuditSink, Source};
use srelens_capability::Registry;
use std::sync::Arc;
use tauri::State;

/// Tauri-managed state holding the capability registry.
pub struct AppRegistry(pub Registry);

/// The audit sink both surfaces write to — the same `audit.jsonl` the MCP
/// server appends to, not a second file in a second format.
///
/// Managed separately from `AppRegistry` because the path is only known once
/// the app's config directory resolves, inside `setup` (`lib.rs`), where the
/// MCP token store and prompt directory are resolved too. A host that cannot
/// resolve one manages `NoopAudit` instead, so a missing config directory
/// costs the trail, never the operation.
pub struct AppAudit(pub Arc<dyn AuditSink>);

/// Invoke a backend capability by id. The WebView calls this via
/// `invoke('invoke_capability', { id, input })`.
///
/// **Every mutating or sensitive call made here is audited** (#555). It used
/// to call `Registry::invoke` straight through, so an Argo CD sync or a Flux
/// reconcile clicked in the app left no record while the identical call from
/// an agent over MCP was written to `audit.jsonl` — the one screen an operator
/// opens after an incident could not show what the operator themselves had
/// done. `invoke_audited` is the registry's own entry point and does the
/// recording for both surfaces, so the two cannot drift apart again.
///
/// Read-only calls are not recorded from here, and that asymmetry with MCP is
/// deliberate — see `srelens_capability::audit::is_audited_from_ui`.
///
/// `"auto"` as the decision, because the UI has no host-owned consent policy
/// yet: a write clicked in the app is confirmed by the frontend before it ever
/// reaches this command. #552 moves that gate into the host, and this is where
/// its verdict will be passed in.
#[tauri::command]
pub async fn invoke_capability(
    id: String,
    input: Value,
    registry: State<'_, AppRegistry>,
    audit: State<'_, AppAudit>,
) -> Result<Value, String> {
    registry
        .0
        .invoke_audited(&id, input, audit.0.as_ref(), Source::Ui, "auto")
        .await
        .map_err(|e| {
            // Surface capability failures (connection timeouts, denied RBAC, bad
            // input) to the application log for post-hoc diagnosis.
            let message = e.to_string();
            log::warn!("capability '{id}' failed: {message}");
            message
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use srelens_capability::audit::{AuditRecord, NoopAudit};
    use srelens_capability::{Annotations, Capability};
    use std::sync::Mutex;
    use tauri::Manager;

    #[derive(Default)]
    struct Spy(Mutex<Vec<AuditRecord>>);

    impl AuditSink for Spy {
        fn record(&self, rec: AuditRecord) {
            self.0.lock().unwrap().push(rec);
        }
    }

    impl Spy {
        fn seen(&self) -> Vec<AuditRecord> {
            self.0.lock().unwrap().clone()
        }
    }

    fn mutating(id: &str) -> Capability {
        let mut cap = Capability::read_only(id, "mutates something", |input| async move {
            if input.get("fail").is_some() {
                return Err(srelens_capability::CapabilityError::Handler(
                    "the cluster said no".into(),
                ));
            }
            if input.get("reject").is_some() {
                return Err(srelens_capability::CapabilityError::InvalidInput(
                    "a resourceVersion is required".into(),
                ));
            }
            Ok(json!({ "ok": true }))
        });
        cap.annotations = Annotations::MUTATING;
        cap
    }

    /// Both bridge paths through a MockRuntime app: a registered capability's
    /// value passes through untouched, and a failure (unknown id here) comes
    /// back as the flat string the WebView renders.
    #[tokio::test]
    async fn passes_values_through_and_flattens_errors() {
        let mut registry = Registry::new();
        registry.register(Capability::read_only(
            "test.echo",
            "echo",
            |input| async move { Ok(json!({ "echoed": input })) },
        ));
        let app = tauri::test::mock_app();
        app.manage(AppRegistry(registry));
        app.manage(AppAudit(Arc::new(NoopAudit)));

        let value = invoke_capability(
            "test.echo".into(),
            json!({"k": 1}),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
        assert_eq!(value, json!({ "echoed": { "k": 1 } }));

        let e = invoke_capability("test.missing".into(), json!({}), app.state(), app.state())
            .await
            .unwrap_err();
        assert!(e.contains("test.missing"), "unexpected error: {e}");
    }

    /// #555, the whole point: a write clicked in the app is in the trail, and
    /// it says it came from the app rather than from an agent.
    #[tokio::test]
    async fn a_mutating_call_from_the_ui_is_recorded_as_ui() {
        let mut registry = Registry::new();
        registry.register(mutating("extensions.action"));
        let spy = Arc::new(Spy::default());
        let app = tauri::test::mock_app();
        app.manage(AppRegistry(registry));
        app.manage(AppAudit(spy.clone()));

        invoke_capability(
            "extensions.action".into(),
            json!({
                "action": "sync",
                "resource": {
                    "id": "org.example.argocd", "revision": 3,
                    "context": "prod", "namespace": "team", "name": "web"
                }
            }),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();

        let seen = spy.seen();
        assert_eq!(seen.len(), 1, "expected one audit record, got {seen:?}");
        assert_eq!(seen[0].source, Source::Ui);
        assert_eq!(seen[0].source.as_str(), "ui");
        assert_eq!(seen[0].tool, "extensions.action");
        assert_eq!(seen[0].outcome, srelens_capability::audit::OUTCOME_OK);
        let app_ref = seen[0].app.as_ref().expect("the app must be named");
        assert_eq!(app_ref.id, "org.example.argocd");
        assert_eq!(app_ref.revision, 3);
        assert_eq!(seen[0].cluster.as_deref(), Some("prod"));
        assert_eq!(seen[0].resource.as_deref(), Some("team/web"));
    }

    /// A read the app makes dozens of times a minute is not an event. The
    /// trail is capped and its readers are looking for writes; burying those
    /// under list calls is the failure mode this avoids.
    #[tokio::test]
    async fn a_read_only_call_from_the_ui_is_not_recorded() {
        let mut registry = Registry::new();
        registry.register(Capability::read_only("k8s.listPods", "lists", |_| async {
            Ok(json!([]))
        }));
        let spy = Arc::new(Spy::default());
        let app = tauri::test::mock_app();
        app.manage(AppRegistry(registry));
        app.manage(AppAudit(spy.clone()));

        invoke_capability(
            "k8s.listPods".into(),
            json!({ "context": "prod" }),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();

        assert!(spy.seen().is_empty(), "a plain read is not an audit event");
    }

    /// A read that RETURNS secret material is, though — and its arguments go
    /// in redacted, exactly as they do for the same call over MCP.
    #[tokio::test]
    async fn a_sensitive_read_from_the_ui_is_recorded_with_its_values_redacted() {
        let mut registry = Registry::new();
        let mut cap = Capability::read_only("k8s.getSecret", "reads a secret", |_| async {
            Ok(json!({}))
        });
        cap.annotations = Annotations::SENSITIVE_READ;
        registry.register(cap);
        let spy = Arc::new(Spy::default());
        let app = tauri::test::mock_app();
        app.manage(AppRegistry(registry));
        app.manage(AppAudit(spy.clone()));

        invoke_capability(
            "k8s.getSecret".into(),
            json!({ "context": "prod", "namespace": "team", "name": "db-creds" }),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();

        let seen = spy.seen();
        assert_eq!(seen.len(), 1, "a sensitive read belongs in the trail");
        assert_eq!(seen[0].args["name"], json!("<redacted>"));
        assert!(
            !seen[0].args.to_string().contains("db-creds"),
            "a sensitive call's values must not reach the log: {:?}",
            seen[0].args
        );
    }

    /// The two ways a UI write does not happen, told apart. "srelens would not
    /// do this" and "the cluster would not" are different answers to "did my
    /// sync go through?", and a trail that renders both as `error` cannot give
    /// either.
    #[tokio::test]
    async fn a_rejected_call_records_its_reason_and_a_failed_one_records_the_failure() {
        let mut registry = Registry::new();
        registry.register(mutating("extensions.action"));
        let spy = Arc::new(Spy::default());
        let app = tauri::test::mock_app();
        app.manage(AppRegistry(registry));
        app.manage(AppAudit(spy.clone()));

        let rejected = invoke_capability(
            "extensions.action".into(),
            json!({ "reject": true }),
            app.state(),
            app.state(),
        )
        .await;
        assert!(rejected.is_err());

        let failed = invoke_capability(
            "extensions.action".into(),
            json!({ "fail": true }),
            app.state(),
            app.state(),
        )
        .await;
        assert!(failed.is_err());

        let seen = spy.seen();
        assert_eq!(seen.len(), 2, "both attempts are events: {seen:?}");
        assert_eq!(seen[0].outcome, srelens_capability::audit::OUTCOME_REJECTED);
        assert!(
            seen[0]
                .error
                .as_deref()
                .unwrap_or_default()
                .contains("resourceVersion"),
            "a rejection carries its reason: {:?}",
            seen[0].error
        );
        assert_eq!(seen[1].outcome, srelens_capability::audit::OUTCOME_FAILED);
        assert!(
            seen[1]
                .error
                .as_deref()
                .unwrap_or_default()
                .contains("the cluster said no"),
            "a failure carries what failed: {:?}",
            seen[1].error
        );
    }
}
