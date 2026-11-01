//! Journeys through switching clusters: an unreachable one at startup, and the
//! hop between clusters with a detail view open.
//!
//! D1 input disorder, D3 late results, D4 boundaries, D5 re-entry, D6
//! truthfulness and D7 narrow screens apply. D2 (ambiguity) does not: a
//! context is picked by position or by the name typed, with no CRD-style
//! collisions to disambiguate.
use std::time::{Duration, Instant};

use crossterm::event::KeyCode;
use srectl::app::{ActiveView, App};
use srectl::ui::Modal;
use srectl::views::yaml_view::YamlViewState;

use crate::common::{self, context_named, key, press};

fn two_clusters(app: &mut App) {
    app.contexts = vec![context_named("test-cluster"), context_named("staging")];
}

fn time_out(app: &mut App) {
    app.connection_attempt_start = Instant::now() - Duration::from_secs(30);
}

/// D1: the picker opens by itself after the timeout; a person arrows down,
/// overshoots back up, comes down again and presses Enter twice. One switch
/// happens, to the cluster under the cursor, and the second Enter lands on the
/// table, not on a stale picker.
#[tokio::test]
async fn journey_unreachable_cluster_picker_survives_arrow_chaos_and_double_enter() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    two_clusters(&mut app);
    time_out(&mut app);
    app.handle_cluster_info_failure("connection refused");
    assert!(matches!(app.modal, Some(Modal::ContextPicker { .. })));

    press(&mut app, key(KeyCode::Down)).await;
    press(&mut app, key(KeyCode::Up)).await;
    press(&mut app, key(KeyCode::Down)).await;
    press(&mut app, key(KeyCode::Enter)).await;
    press(&mut app, key(KeyCode::Enter)).await;

    assert_eq!(app.active_context, "staging");
    assert!(app.modal.is_none(), "the picker closed and did not come back");
}

/// D3: a cluster-info answer for the cluster just left arrives after the
/// switch. It must not make the new cluster look connected, and a failure from
/// the old one must not mark the new one unreachable.
#[tokio::test]
async fn journey_late_answers_from_the_cluster_just_left_are_ignored() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    two_clusters(&mut app);
    app.switch_context("staging".into()).await;
    time_out(&mut app);

    app.handle_cluster_info_update("v9.9.9|50|500|test-cluster");
    app.handle_cluster_info_failure_for("test-cluster", "connection refused");

    assert_eq!(app.cluster_version, "Connecting...", "still waiting on staging");
    assert!(!app.cluster_unreachable);
    assert!(app.modal.is_none(), "no unreachable picker for a cluster we are not on");
}

/// D5: prod -> staging -> prod with a manifest open each time. Every hop lands
/// on a pods table with no history, modal or filter carried over.
#[tokio::test]
async fn journey_hopping_clusters_with_a_manifest_open_never_shows_the_old_cluster() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    two_clusters(&mut app);
    for target in ["staging", "test-cluster", "staging"] {
        app.active_view = ActiveView::Yaml(YamlViewState::new(
            "web".into(),
            "Pod".into(),
            Some("default".into()),
            "kind: Pod\n".into(),
        ));
        app.nav_stack.push(ActiveView::Assistant);
        app.filter_buffer = "nginx".into();
        app.switch_context(target.into()).await;
        assert_eq!(app.active_context, target);
        assert!(matches!(app.active_view, ActiveView::Table(_)), "back on a table");
        assert!(app.nav_stack.is_empty() && app.filter_buffer.is_empty() && app.modal.is_none());
    }
}

/// D4: boundaries. One cluster has nowhere to go, so no picker; no clusters at
/// all must not panic either.
#[tokio::test]
async fn journey_zero_and_one_cluster_never_open_a_picker() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    for contexts in [vec![], vec![context_named("test-cluster")]] {
        app.contexts = contexts;
        time_out(&mut app);
        app.cluster_unreachable = false;
        app.handle_cluster_info_failure("connection refused");
        assert!(app.modal.is_none());
    }
}

/// D6: the user is told which cluster is unreachable, and the picker's own
/// list names the way out.
#[tokio::test]
async fn journey_the_unreachable_cluster_is_named_to_the_user() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    two_clusters(&mut app);
    time_out(&mut app);
    app.handle_cluster_info_failure("connection refused");
    let (msg, _, _) = app.toast.clone().expect("a toast");
    assert!(msg.contains("test-cluster") && msg.contains("unreachable"), "{msg}");
    let screen = common::render_app(&mut app, 120, 40);
    assert!(screen.contains("staging"), "the other cluster is offered:\n{screen}");
}

/// D7: the picker on a small terminal and a large one.
#[tokio::test]
async fn journey_the_picker_renders_at_80x24_and_200x50() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    two_clusters(&mut app);
    time_out(&mut app);
    app.handle_cluster_info_failure("connection refused");
    for (w, h) in [(80, 24), (200, 50), (40, 10)] {
        let screen = common::render_app(&mut app, w, h);
        assert!(!screen.trim().is_empty(), "{w}x{h} drew nothing");
    }
}
