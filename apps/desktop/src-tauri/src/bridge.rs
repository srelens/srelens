//! Bridges the capability registry to the Tauri WebView as a command.
//!
//! This is the Tauri half of "one definition, two surfaces": the same
//! `Registry` that backs the MCP server is invoked here from the frontend.

use serde_json::Value;
use srelens_capability::audit::{AuditSink, Source};
use srelens_capability::Registry;
use std::sync::Arc;
use tauri::{AppHandle, Runtime, State, Window};

use crate::node_shells;
use crate::window_streams::WindowStreams;

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

/// Package bytes use raw IPC: a 512 MiB package's base64 cannot fit a V8 string.
/// Encoding stays in Rust; verification, grants and auditing stay in the registry.
fn package_invocation(
    body: &tauri::ipc::InvokeBody,
    metadata: Option<&str>,
) -> Result<(String, Value), String> {
    use base64::Engine as _;
    let tauri::ipc::InvokeBody::Raw(bytes) = body else {
        return Err("A package upload must contain raw bytes".into());
    };
    if bytes.len() > srelens_registry::extension_package::MAX_PACKAGE_BYTES {
        return Err("The package exceeds 512 MiB".into());
    }
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Metadata {
        id: String,
        input: serde_json::Map<String, Value>,
    }
    let mut metadata: Metadata =
        serde_json::from_str(metadata.ok_or("The package upload has no metadata")?)
            .map_err(|e| format!("Invalid package upload metadata: {e}"))?;
    if metadata.id != "extensions.packageManifest"
        && !(metadata.id == "extensions.configure"
            && metadata.input.get("action").and_then(Value::as_str) == Some("installPackage"))
    {
        return Err("Raw package uploads only review or install a package".into());
    }
    metadata.input.insert(
        "package".into(),
        Value::String(base64::engine::general_purpose::STANDARD.encode(bytes)),
    );
    Ok((metadata.id, Value::Object(metadata.input)))
}

#[tauri::command]
pub async fn invoke_package_capability<R: Runtime>(
    request: tauri::ipc::Request<'_>,
    window: Window<R>,
    app: AppHandle<R>,
    registry: State<'_, AppRegistry>,
    audit: State<'_, AppAudit>,
    owned: State<'_, WindowStreams>,
) -> Result<Value, String> {
    let (id, input) = package_invocation(
        request.body(),
        request
            .headers()
            .get("x-srelens-package-input")
            .and_then(|h| h.to_str().ok()),
    )?;
    invoke_capability(id, input, window, app, registry, audit, owned).await
}

/// Invoke a backend capability by id. The WebView calls this via
/// `invoke('invoke_capability', { id, input })`.
///
/// A node debug pod this creates (`k8s.createNodeDebugPod`) becomes the
/// calling window's, and the host deletes it (#734, `crate::node_shells`).
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
pub async fn invoke_capability<R: Runtime>(
    id: String,
    input: Value,
    window: Window<R>,
    app: AppHandle<R>,
    registry: State<'_, AppRegistry>,
    audit: State<'_, AppAudit>,
    owned: State<'_, WindowStreams>,
) -> Result<Value, String> {
    let epoch = owned.epoch(window.label());
    // A node debug pod is the host's to delete (#734), so its creation is
    // where the host learns of it: the call's own input names the context.
    let adopts = (id == node_shells::NODE_DEBUG_CAPABILITY).then(|| input.clone());
    let out = registry
        .0
        .invoke_audited(&id, input, audit.0.as_ref(), Source::Ui, "auto")
        .await
        .map_err(|e| {
            // Surface capability failures (connection timeouts, denied RBAC, bad
            // input) to the application log for post-hoc diagnosis.
            let message = e.to_string();
            log::warn!("capability '{id}' failed: {message}");
            message
        })?;
    if let Some(input) = adopts {
        node_shells::adopt(&app, &owned, window.label(), epoch, &input, &out).await?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use srelens_capability::audit::{AuditRecord, NoopAudit};
    use srelens_capability::{Annotations, Capability};
    use std::sync::Mutex;
    use tauri::Manager;

    #[test]
    fn raw_package_bytes_reach_the_existing_review_and_install_capabilities() {
        for (id, input) in [
            ("extensions.packageManifest", json!({})),
            (
                "extensions.configure",
                json!({"action": "installPackage", "grants": ["k8s.listCustomResource"], "reviewedRevision": 7}),
            ),
        ] {
            let metadata = json!({"id": id, "input": input}).to_string();
            let (got_id, got) = package_invocation(
                &tauri::ipc::InvokeBody::Raw(vec![0x1f, 0x8b, 0x08, 0, 0xff]),
                Some(&metadata),
            )
            .unwrap();
            let mut expected = input;
            expected["package"] = json!("H4sIAP8=");
            assert_eq!(got_id, id);
            assert_eq!(got, expected);
        }
    }

    #[test]
    fn raw_package_transport_refuses_other_capabilities_actions_and_json_bodies() {
        let raw = tauri::ipc::InvokeBody::Raw(vec![]);
        for metadata in [
            json!({"id": "k8s.listPods", "input": {}}),
            json!({"id": "extensions.configure", "input": {"action": "unsignedApps", "allowUnsignedApps": true}}),
            json!({"id": "extensions.configure", "input": {"action": "install"}}),
            json!({"id": "extensions.packageManifest", "input": null}),
        ] {
            assert!(package_invocation(&raw, Some(&metadata.to_string())).is_err());
        }
        assert!(package_invocation(&raw, None).is_err());
        assert!(package_invocation(&raw, Some("not JSON")).is_err());
        assert!(package_invocation(
            &tauri::ipc::InvokeBody::Json(json!({})),
            Some(r#"{"id":"extensions.packageManifest","input":{}}"#)
        )
        .is_err());
    }

    #[test]
    fn raw_package_transport_checks_the_limit_before_encoding() {
        let at_limit = tauri::ipc::InvokeBody::Raw(vec![0; 512 * 1024 * 1024]);
        let (_, input) = package_invocation(
            &at_limit,
            Some(r#"{"id":"extensions.packageManifest","input":{}}"#),
        )
        .unwrap();
        assert_eq!(input["package"].as_str().unwrap().len(), 715_827_884);
        drop(input);
        drop(at_limit);
        let body = tauri::ipc::InvokeBody::Raw(vec![0; 512 * 1024 * 1024 + 1]);
        let reason = package_invocation(
            &body,
            Some(r#"{"id":"extensions.packageManifest","input":{}}"#),
        )
        .unwrap_err();
        assert!(reason.contains("512 MiB"), "{reason}");
    }

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

    #[test]
    fn raw_ipc_dispatches_the_package_command_and_audits_the_install() {
        let mut registry = Registry::new();
        registry.register(mutating("extensions.configure"));
        let audit = Arc::new(Spy::default());
        let app = tauri::test::mock_builder()
            .invoke_handler(tauri::generate_handler![invoke_package_capability])
            .manage(AppRegistry(registry))
            .manage(AppAudit(audit.clone()))
            .manage(WindowStreams::default())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        let metadata = r#"{"id":"extensions.configure","input":{"action":"installPackage","grants":["k8s.listCustomResource"],"reviewedRevision":7}}"#;
        // HeaderMap's type comes from the request, keeping the test on Tauri's wire.
        let mut request = tauri::webview::InvokeRequest {
            cmd: "invoke_package_capability".into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: if cfg!(windows) {
                "http://tauri.localhost"
            } else {
                "tauri://localhost"
            }
            .parse()
            .unwrap(),
            body: tauri::ipc::InvokeBody::Raw(vec![0x1f, 0x8b, 0x08, 0, 0xff]),
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.to_string(),
        };
        request
            .headers
            .insert("x-srelens-package-input", metadata.parse().unwrap());
        let response = tauri::test::get_ipc_response(&webview, request).unwrap();
        assert_eq!(
            response.deserialize::<Value>().unwrap(),
            json!({"ok": true})
        );
        let records = audit.seen();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].tool, "extensions.configure");
        assert_eq!(records[0].source, Source::Ui);
        assert!(!serde_json::to_string(&records[0].args)
            .unwrap()
            .contains("H4sIAP8="));
    }

    /// `invoke_capability` as the `main` window calls it.
    async fn call(
        app: &tauri::App<tauri::test::MockRuntime>,
        id: &str,
        input: Value,
    ) -> Result<Value, String> {
        if app.try_state::<WindowStreams>().is_none() {
            app.manage(WindowStreams::default());
        }
        let window = match app.get_webview_window("main") {
            Some(webview) => webview.as_ref().window(),
            None => crate::window_streams::tests::mock_window(app, "main"),
        };
        invoke_capability(
            id.into(),
            input,
            window,
            app.handle().clone(),
            app.state(),
            app.state(),
            app.state(),
        )
        .await
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

        let value = call(&app, "test.echo", json!({"k": 1})).await.unwrap();
        assert_eq!(value, json!({ "echoed": { "k": 1 } }));

        let e = call(&app, "test.missing", json!({})).await.unwrap_err();
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

        call(
            &app,
            "extensions.action",
            json!({
                "action": "sync",
                "resource": {
                    "id": "org.example.argocd", "revision": 3,
                    "context": "prod", "namespace": "team", "name": "web"
                }
            }),
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

        call(&app, "k8s.listPods", json!({ "context": "prod" }))
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

        call(
            &app,
            "k8s.getSecret",
            json!({ "context": "prod", "namespace": "team", "name": "db-creds" }),
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

        let rejected = call(&app, "extensions.action", json!({ "reject": true })).await;
        assert!(rejected.is_err());

        let failed = call(&app, "extensions.action", json!({ "fail": true })).await;
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
