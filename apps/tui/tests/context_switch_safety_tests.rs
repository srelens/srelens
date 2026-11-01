//! Switching context must never leave the old cluster on screen, and results
//! that were in flight for the old context must not land on the new one.

mod common;

use srectl::app::ActiveView;
use srectl::commands::ResourceKind;
use srectl::ui::Modal;
use srectl::views::yaml_view::YamlViewState;
use srelens_kube::contexts::ContextDto;

fn ctx(name: &str) -> ContextDto {
    ContextDto {
        name: name.to_string(),
        stable_id: format!("file/{name}"),
        key: format!("file/{name}"),
        pinned_id: None,
        cluster: format!("c-{name}"),
        server: format!("https://{name}.example.invalid"),
        namespace: String::new(),
        is_current: false,
        is_local: false,
        provider: None,
        source_file: "cluster-config.yaml".to_string(),
        auth_kind: "token".to_string(),
    }
}

#[tokio::test]
async fn switching_context_leaves_a_detail_view_of_the_old_cluster() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    app.contexts = vec![ctx("test-cluster"), ctx("staging")];
    app.active_view = ActiveView::Yaml(YamlViewState::new(
        "web".into(),
        "Pod".into(),
        Some("default".into()),
        "kind: Pod\n".into(),
    ));

    app.switch_context("staging".into()).await;

    assert!(
        matches!(&app.active_view, ActiveView::Table(t) if t.kind == ResourceKind::Pods),
        "a manifest of the old cluster is replaced by a fresh pods table"
    );
}

#[tokio::test]
async fn switching_context_drops_navigation_history_and_modals_of_the_old_cluster() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    app.contexts = vec![ctx("test-cluster"), ctx("staging")];
    app.nav_stack.push(ActiveView::Assistant);
    app.filter_buffer = "nginx".into();
    app.modal = Some(Modal::NodeSsh {
        node_name: "n1".into(),
        destination_input: String::new(),
        cursor_pos: 0,
    });

    app.switch_context("staging".into()).await;

    assert!(app.nav_stack.is_empty(), "Esc must not return to the old cluster");
    assert!(app.filter_buffer.is_empty());
    assert!(app.modal.is_none());
}

#[tokio::test]
async fn cluster_info_from_the_previous_context_is_ignored() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    app.contexts = vec![ctx("test-cluster"), ctx("staging")];
    app.switch_context("staging".into()).await;

    app.handle_cluster_info_update("v1.20.0|3|40|test-cluster");
    assert_eq!(app.cluster_version, "Connecting...");
    assert!(!app.is_connected);

    app.handle_cluster_info_update("v1.31.0|5|60|staging");
    assert_eq!(app.cluster_version, "v1.31.0");
    assert!(app.is_connected);
}

#[tokio::test]
async fn cluster_info_without_a_context_still_applies() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    app.handle_cluster_info_update("v1.31.0|5|60");
    assert_eq!(app.cluster_version, "v1.31.0");
}

#[tokio::test]
async fn a_failure_from_the_previous_context_does_not_mark_the_new_one_unreachable() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    app.contexts = vec![ctx("test-cluster"), ctx("staging")];
    app.switch_context("staging".into()).await;
    app.connection_attempt_start = std::time::Instant::now() - std::time::Duration::from_secs(30);

    app.handle_cluster_info_failure_for("test-cluster", "connection refused");
    assert!(!app.cluster_unreachable);

    app.handle_cluster_info_failure_for("staging", "connection refused");
    assert!(app.cluster_unreachable);
}

#[tokio::test]
async fn crds_discovered_for_the_previous_context_are_not_applied() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    app.contexts = vec![ctx("test-cluster"), ctx("staging")];
    app.switch_context("staging".into()).await;
    let before = app.crds.len();

    let payload = r#"{"context":"test-cluster","crds":[{"crd_name":"widgets.g.io","group":"g.io","version":"v1","kind":"Widget","plural":"widgets","singular":"widget","namespaced":true,"short_names":[]}]}"#;
    app.handle_crds_update(payload);
    assert_eq!(app.crds.len(), before);
}

#[tokio::test]
async fn an_unreachable_cluster_offers_the_context_picker_once() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    app.contexts = vec![ctx("test-cluster"), ctx("staging")];
    app.connection_attempt_start = std::time::Instant::now() - std::time::Duration::from_secs(30);

    app.handle_cluster_info_failure("connection refused");
    assert!(matches!(app.modal, Some(Modal::ContextPicker { .. })));

    app.modal = None;
    app.handle_cluster_info_failure("connection refused");
    assert!(app.modal.is_none(), "dismissed once, not reopened on every retry");
}

#[tokio::test]
async fn a_single_context_has_nowhere_to_switch_so_no_picker_opens() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    app.contexts = vec![ctx("test-cluster")];
    app.connection_attempt_start = std::time::Instant::now() - std::time::Duration::from_secs(30);

    app.handle_cluster_info_failure("connection refused");
    assert!(app.modal.is_none());
}
