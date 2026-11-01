//! Journeys through alerts: the toast, the header badge and `:notifications`.
//!
//! D1 input disorder, D3 late arrivals, D4 boundaries, D5 re-entry, D6
//! truthfulness and D7 narrow screens apply. D2 (ambiguity) does not: there is
//! one command with fixed aliases and no names to collide.
use crossterm::event::KeyCode;
use serde_json::{json, Value};
use srectl::notifications::Level;
use srectl::theme::Theme;

use crate::common::{self, ch, context_named, key, press, run_command};

fn pod(name: &str, status: &str) -> Value {
    json!({ "name": name, "namespace": "prod", "status": status, "restarts": 0 })
}

fn watch(ctx: &str) -> String {
    format!("watch:{ctx}:prod:pods")
}

/// D1: a person opens the alerts, mashes keys that mean nothing there, clears,
/// closes with `q`, and reopens. Nothing sticks half-open, and a second `:alerts`
/// right after closing works.
#[tokio::test]
async fn journey_alerts_overlay_survives_key_mashing_and_reopening() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    app.set_toast("it broke".into(), Theme::status_error());

    run_command(&mut app, "alerts").await;
    assert!(app.show_notifications);
    for k in [KeyCode::Down, KeyCode::Up, KeyCode::Tab, KeyCode::Enter, KeyCode::Char(' '), KeyCode::Char('j')] {
        press(&mut app, key(k)).await;
    }
    assert!(app.show_notifications, "keys that mean nothing here do not close it");
    press(&mut app, ch('c')).await;
    assert_eq!(app.notifications.items().count(), 0);
    press(&mut app, ch('q')).await;
    assert!(!app.show_notifications);

    run_command(&mut app, "notifications").await;
    assert!(app.show_notifications, "reopens straight after closing");
}

/// D3: a pod breaks while the overlay is open. It lands in the list at once,
/// and after closing the badge is not left claiming it was read.
#[tokio::test]
async fn journey_an_alert_arriving_while_the_overlay_is_open_is_not_lost() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    app.handle_stream_event(watch("test-cluster"), json!([pod("web", "Running")]));
    run_command(&mut app, "notifications").await;

    app.handle_stream_event(watch("test-cluster"), json!([pod("web", "CrashLoopBackOff")]));
    let screen = common::render_app(&mut app, 140, 40);
    assert!(screen.contains("prod/web"), "listed while open:\n{screen}");

    press(&mut app, key(KeyCode::Esc)).await;
    assert_eq!(app.notifications.unread(), 1, "arrived after it was read, so still unread");
}

/// D4: nothing at all to report, and far too much.
#[tokio::test]
async fn journey_an_empty_history_and_a_flood_both_render() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    run_command(&mut app, "alerts").await;
    assert!(common::render_app(&mut app, 120, 30).contains("Nothing to report"));

    for i in 0..1000 {
        app.notifications.push(Level::Error, "x", format!("failure {i}"));
    }
    assert!(app.notifications.items().count() <= 200, "history is bounded");
    let screen = common::render_app(&mut app, 120, 30);
    assert!(screen.contains("failure 999"), "newest is shown:\n{screen}");
}

/// D5: alerts follow the cluster you are on. Pods first seen on the other
/// cluster only set its baseline, and one that then breaks alerts there.
#[tokio::test]
async fn journey_switching_clusters_gives_the_new_one_its_own_baseline() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    app.contexts = vec![context_named("test-cluster"), context_named("staging")];
    app.switch_context("staging".into()).await;

    app.handle_stream_event(watch("staging"), json!([pod("api", "CrashLoopBackOff")]));
    assert_eq!(app.notifications.items().filter(|n| n.message.contains("api")).count(), 0,
        "already broken when first seen: no alert");

    app.handle_stream_event(watch("staging"), json!([pod("api", "Running")]));
    app.handle_stream_event(watch("staging"), json!([pod("api", "Error")]));
    assert!(app.notifications.items().any(|n| n.message.contains("prod/api") && n.level == Level::Error));

    let before = app.notifications.items().count();
    app.handle_stream_event(watch("test-cluster"), json!([pod("old", "Running")]));
    app.handle_stream_event(watch("test-cluster"), json!([pod("old", "Error")]));
    assert_eq!(app.notifications.items().count(), before, "the cluster we left raises nothing");
}

/// D6: failures are reported as failures, with their reason.
#[tokio::test]
async fn journey_a_watch_error_reaches_the_toast_and_the_history() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    app.current_watch_channel = Some(watch("test-cluster"));
    app.handle_stream_event(watch("test-cluster"), json!({ "error": "connection refused" }));
    let (msg, _, _) = app.toast.clone().expect("a toast");
    assert!(msg.contains("Watch error") && msg.contains("connection refused"), "{msg}");
    assert!(app.notifications.items().any(|n| n.message.contains("connection refused")));
}

/// D7: the header badge and the overlay on a small and a large terminal.
#[tokio::test]
async fn journey_badge_and_overlay_render_at_80x24_and_200x50() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    app.set_toast("it broke".into(), Theme::status_error());
    for (w, h) in [(80, 24), (200, 50), (40, 10)] {
        assert!(!common::render_app(&mut app, w, h).trim().is_empty());
    }
    assert!(common::render_app(&mut app, 120, 30).contains("! 1"), "the badge is in the header");
    run_command(&mut app, "alerts").await;
    for (w, h) in [(80, 24), (200, 50), (40, 10)] {
        assert!(!common::render_app(&mut app, w, h).trim().is_empty());
    }
}
