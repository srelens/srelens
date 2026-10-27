use crossterm::event::KeyCode;
use srectl::app::{ActiveView, App};
use srectl::commands::{CrdMeta, ResourceKind};
use srectl::views::resource_table::ResourceTableState;

use crate::common::{self, ch, key, press, type_str};

fn table(app: &App) -> &ResourceTableState {
    match &app.active_view {
        ActiveView::Table(t) => t,
        _ => panic!("expected a table view"),
    }
}

fn mock_prometheus_crds() -> Vec<CrdMeta> {
    vec![
        CrdMeta {
            crd_name: "prometheusagents.monitoring.coreos.com".into(),
            group: "monitoring.coreos.com".into(),
            version: "v1alpha1".into(),
            kind: "PrometheusAgent".into(),
            plural: "prometheusagents".into(),
            singular: "prometheusagent".into(),
            namespaced: true,
            short_names: vec![],
            printer_columns: vec![],
            created_at: None,
        },
        CrdMeta {
            crd_name: "prometheuses.monitoring.coreos.com".into(),
            group: "monitoring.coreos.com".into(),
            version: "v1".into(),
            kind: "Prometheus".into(),
            plural: "prometheuses".into(),
            singular: "prometheus".into(),
            namespaced: true,
            short_names: vec![],
            printer_columns: vec![],
            created_at: None,
        },
        CrdMeta {
            crd_name: "prometheusrules.monitoring.coreos.com".into(),
            group: "monitoring.coreos.com".into(),
            version: "v1".into(),
            kind: "PrometheusRule".into(),
            plural: "prometheusrules".into(),
            singular: "prometheusrule".into(),
            namespaced: true,
            short_names: vec![],
            printer_columns: vec![],
            created_at: None,
        },
    ]
}

/// D1: Input Disorder & Interleaving (The "Messy Human")
/// Human types prefix ":prome", navigates down with arrow keys to the 3rd suggestion
/// ("prometheusrules"), then presses Space. The command buffer must expand to
/// "prometheusrules ", and typing the namespace followed by Enter navigates to that CRD in that namespace.
#[tokio::test]
async fn journey_command_down_arrow_selection_and_space_completion() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    app.crds = mock_prometheus_crds();
    app.namespaces = vec!["monitoring".into(), "kube-system".into(), "default".into()];

    // 1. Human types prefix
    press(&mut app, ch(':')).await;
    type_str(&mut app, "prome").await;

    // 2. Human arrows down past the first two items
    press(&mut app, key(KeyCode::Down)).await;
    press(&mut app, key(KeyCode::Down)).await;

    // 3. Human presses Space to commit selection and begin namespace
    press(&mut app, ch(' ')).await;
    assert_eq!(app.command_buffer, "prometheusrules ");

    // 4. Human types namespace and presses Enter
    type_str(&mut app, "moni").await;
    press(&mut app, key(KeyCode::Enter)).await;

    assert_eq!(app.active_namespace, "monitoring");
    let rules_crd = &app.crds[2];
    assert_eq!(
        table(&app).kind,
        ResourceKind::CustomResource(rules_crd.clone())
    );
}

/// D2: Ambiguity & Collisions (The "Fuzzy Human")
/// A cluster has a CRD whose plural is "deployments" (e.g. deployments.acme.io).
/// When typing ":deployments.acme.io", pressing Space must complete with the exact CRD name,
/// not collapse to the built-in ":deployments" command.
#[tokio::test]
async fn journey_crd_plural_collides_with_static_command_preserves_crd() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    let acme_crd = CrdMeta {
        crd_name: "deployments.acme.io".into(),
        group: "acme.io".into(),
        version: "v1".into(),
        kind: "Deployment".into(),
        plural: "deployments".into(),
        singular: "deployment".into(),
        namespaced: true,
        short_names: vec![],
        printer_columns: vec![],
        created_at: None,
    };
    app.crds = vec![acme_crd.clone()];
    app.namespaces = vec!["staging".into()];

    press(&mut app, ch(':')).await;
    type_str(&mut app, "deployments.acme.io").await;
    press(&mut app, ch(' ')).await;

    assert_eq!(app.command_buffer, "deployments.acme.io ");

    type_str(&mut app, "stag").await;
    press(&mut app, key(KeyCode::Enter)).await;
    assert_eq!(app.active_namespace, "staging");
    assert_eq!(table(&app).kind, ResourceKind::CustomResource(acme_crd));
}

/// D3: Temporal & Asynchrony (The "Impatient Human")
/// Human enters a command with namespace before namespace discovery finishes or
/// when namespace discovery failed. The app must execute without panic or corruption.
#[tokio::test]
async fn journey_command_typing_before_namespace_discovery_completes() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    app.namespaces.clear(); // Empty namespaces (still loading)
    app.namespace_discovery_error = Some("transient timeout reaching API server".into());

    press(&mut app, ch(':')).await;
    type_str(&mut app, "deployments custom-ns").await;
    press(&mut app, key(KeyCode::Enter)).await;

    assert_eq!(app.active_namespace, "custom-ns");
    assert_eq!(table(&app).kind, ResourceKind::Deployments);
}

/// D6: Feedback & Truthfulness ("Say what you know, not what you guess")
/// Executing an unknown command must display a clear warning toast and not alter
/// the current view or corrupt the active namespace.
#[tokio::test]
async fn journey_unknown_command_shows_truthful_toast_without_state_corruption() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    let initial_kind = table(&app).kind.clone();
    let initial_ns = app.active_namespace.clone();

    press(&mut app, ch(':')).await;
    type_str(&mut app, "definitelynotarealcommand").await;
    press(&mut app, key(KeyCode::Enter)).await;

    assert_eq!(table(&app).kind, initial_kind);
    assert_eq!(app.active_namespace, initial_ns);
    assert!(app
        .toast
        .as_ref()
        .is_some_and(|(msg, _, _)| msg.contains("Unknown command")));
}

/// D7: Visual & Perceptual Reality (What the Human Actually Sees)
/// Verifies that when typing a namespace-taking command with no namespaces loaded,
/// the rendered screen buffer displays the truthful contract placeholder
/// " <namespace>" (lowercase, not uppercase).
#[tokio::test]
async fn journey_command_bar_renders_lowercase_namespace_hint() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    app.namespaces.clear();

    press(&mut app, ch(':')).await;
    type_str(&mut app, "deployments ").await;

    let screen = common::render_app(&mut app, 120, 30);
    assert!(
        screen.contains("deployments <namespace>"),
        "Screen should display suggestion with ' <namespace>' suffix, got:\n{}",
        screen
    );
}
