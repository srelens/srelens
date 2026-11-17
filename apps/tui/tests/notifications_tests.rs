//! The notification center: a history behind the one-slot toast, and a watcher
//! that turns pod transitions into alerts.

mod common;

use serde_json::{json, Value};
use srectl::notifications::{Level, NotificationCenter};

fn flat(name: &str, status: &str, restarts: i64) -> Value {
    json!({ "name": name, "namespace": "prod", "status": status, "restarts": restarts })
}

fn raw_waiting(name: &str, reason: &str, restarts: i64) -> Value {
    json!({
        "metadata": { "name": name, "namespace": "prod" },
        "status": { "phase": "Running", "containerStatuses": [{
            "name": "app", "restartCount": restarts,
            "state": { "waiting": { "reason": reason } }
        }]}
    })
}

fn messages(c: &NotificationCenter) -> Vec<String> {
    c.items().map(|n| n.message.clone()).collect()
}

#[test]
fn history_keeps_every_message_the_toast_would_overwrite() {
    let mut c = NotificationCenter::default();
    c.push(Level::Info, "ui", "first");
    c.push(Level::Error, "ui", "second");
    assert_eq!(messages(&c), vec!["second", "first"], "newest first");
}

#[test]
fn an_identical_repeat_is_counted_not_listed_again() {
    let mut c = NotificationCenter::default();
    c.push(Level::Error, "watch", "Watch error: boom");
    c.push(Level::Error, "watch", "Watch error: boom");
    c.push(Level::Error, "watch", "Watch error: boom");
    assert_eq!(c.items().count(), 1);
    assert_eq!(c.items().next().unwrap().count, 3);
}

#[test]
fn only_warnings_and_errors_count_as_unread_until_opened() {
    let mut c = NotificationCenter::default();
    c.push(Level::Success, "ui", "ok");
    c.push(Level::Info, "ui", "fyi");
    assert_eq!(c.unread(), 0);
    c.push(Level::Warn, "ui", "careful");
    c.push(Level::Error, "ui", "broken");
    assert_eq!(c.unread(), 2);
    c.mark_read();
    assert_eq!(c.unread(), 0);
}

#[test]
fn history_is_bounded() {
    let mut c = NotificationCenter::default();
    for i in 0..500 {
        c.push(Level::Info, "ui", format!("m{i}"));
    }
    assert!(c.items().count() <= 200);
    assert_eq!(c.items().next().unwrap().message, "m499");
}

#[test]
fn the_first_look_at_a_namespace_only_sets_a_baseline() {
    let mut c = NotificationCenter::default();
    c.observe_pods("ctx", "prod", &[flat("web", "CrashLoopBackOff", 9)]);
    assert_eq!(c.items().count(), 0, "already-broken pods do not flood on open");
}

#[test]
fn a_pod_that_starts_crash_looping_raises_an_error() {
    let mut c = NotificationCenter::default();
    c.observe_pods("ctx", "prod", &[flat("web", "Running", 0)]);
    c.observe_pods("ctx", "prod", &[flat("web", "CrashLoopBackOff", 1)]);
    let n = c.items().next().expect("an alert");
    assert_eq!(n.level, Level::Error);
    assert!(n.message.contains("prod/web") && n.message.contains("CrashLoopBackOff"));
    assert_eq!(c.unread(), 1);
}

#[test]
fn the_same_problem_is_not_re_announced_on_every_refresh() {
    let mut c = NotificationCenter::default();
    c.observe_pods("ctx", "prod", &[flat("web", "Running", 0)]);
    c.observe_pods("ctx", "prod", &[flat("web", "ImagePullBackOff", 0)]);
    c.observe_pods("ctx", "prod", &[flat("web", "ImagePullBackOff", 0)]);
    c.observe_pods("ctx", "prod", &[flat("web", "ImagePullBackOff", 0)]);
    assert_eq!(c.items().count(), 1);
}

#[test]
fn recovery_is_reported() {
    let mut c = NotificationCenter::default();
    c.observe_pods("ctx", "prod", &[flat("web", "Running", 0)]);
    c.observe_pods("ctx", "prod", &[flat("web", "CrashLoopBackOff", 1)]);
    c.observe_pods("ctx", "prod", &[flat("web", "Running", 1)]);
    let n = c.items().next().unwrap();
    assert_eq!(n.level, Level::Success);
    assert!(n.message.contains("prod/web") && n.message.contains("recovered"));
}

#[test]
fn a_restart_of_a_running_pod_is_a_warning() {
    let mut c = NotificationCenter::default();
    c.observe_pods("ctx", "prod", &[flat("web", "Running", 0)]);
    c.observe_pods("ctx", "prod", &[flat("web", "Running", 2)]);
    let n = c.items().next().unwrap();
    assert_eq!(n.level, Level::Warn);
    assert!(n.message.contains("restarted"));
}

#[test]
fn a_new_pod_that_is_born_broken_is_reported_after_the_baseline() {
    let mut c = NotificationCenter::default();
    c.observe_pods("ctx", "prod", &[flat("web", "Running", 0)]);
    c.observe_pods(
        "ctx",
        "prod",
        &[flat("web", "Running", 0), flat("job", "Error", 0)],
    );
    assert!(c.items().next().unwrap().message.contains("prod/job"));
}

#[test]
fn raw_pod_objects_are_understood_as_well_as_flat_rows() {
    let mut c = NotificationCenter::default();
    c.observe_pods("ctx", "prod", &[raw_waiting("web", "ContainerCreating", 0)]);
    c.observe_pods("ctx", "prod", &[raw_waiting("web", "CrashLoopBackOff", 3)]);
    let n = c.items().next().unwrap();
    assert_eq!(n.level, Level::Error);
    assert!(n.message.contains("CrashLoopBackOff"));
}

#[test]
fn contexts_are_tracked_separately() {
    let mut c = NotificationCenter::default();
    c.observe_pods("a", "prod", &[flat("web", "Running", 0)]);
    c.observe_pods("b", "prod", &[flat("web", "CrashLoopBackOff", 4)]);
    assert_eq!(c.items().count(), 0, "b's first look is its own baseline");
}

#[test]
fn clear_empties_history_and_unread() {
    let mut c = NotificationCenter::default();
    c.push(Level::Error, "ui", "x");
    c.clear();
    assert_eq!(c.items().count(), 0);
    assert_eq!(c.unread(), 0);
}

// ---------------------------------------------------------------------------
// In the app
// ---------------------------------------------------------------------------

use crossterm::event::KeyCode;
use srectl::commands::CommandTarget;
use srectl::theme::Theme;

#[tokio::test]
async fn every_toast_lands_in_the_history_with_its_level() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    app.set_toast("all good".into(), Theme::status_ok());
    app.set_toast("it broke".into(), Theme::status_error());
    let levels: Vec<Level> = app.notifications.items().map(|n| n.level).collect();
    assert_eq!(levels, vec![Level::Error, Level::Success]);
    assert_eq!(app.notifications.unread(), 1);
}

#[tokio::test]
async fn the_notifications_command_opens_the_history_and_reads_it() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    app.set_toast("it broke".into(), Theme::status_error());
    app.execute_command_target(CommandTarget::Notifications).await;
    assert!(app.show_notifications);

    let screen = common::render_app(&mut app, 120, 40);
    assert!(screen.contains("it broke"), "the overlay lists the message:\n{screen}");
    assert_eq!(app.notifications.unread(), 0, "drawn, so read");

    common::press(&mut app, common::key(KeyCode::Esc)).await;
    assert!(!app.show_notifications);
}

#[tokio::test]
async fn c_clears_the_history_while_it_is_open() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    app.set_toast("it broke".into(), Theme::status_error());
    app.execute_command_target(CommandTarget::Notifications).await;
    common::press(&mut app, common::ch('c')).await;
    assert_eq!(app.notifications.items().count(), 0);
    assert!(app.show_notifications, "clearing does not close it");
}

#[tokio::test]
async fn unread_alerts_show_in_the_status_bar_after_the_toast_expires() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    app.set_toast("it broke".into(), Theme::status_error());
    app.toast = None;
    let screen = common::render_app(&mut app, 160, 40);
    assert!(
        screen.contains("1 alert") && screen.contains(":notifications"),
        "a persistent hint outlives the toast:\n{screen}"
    );
}

#[tokio::test]
async fn a_pod_watch_that_turns_bad_raises_an_alert() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    let ch = "watch:test-cluster:prod:pods".to_string();
    app.handle_stream_event(ch.clone(), json!([flat("web", "Running", 0)]));
    app.handle_stream_event(ch, json!([flat("web", "CrashLoopBackOff", 1)]));
    let n = app.notifications.items().next().expect("an alert");
    assert_eq!(n.level, Level::Error);
    assert!(n.message.contains("prod/web"));
}

#[test]
fn a_pod_flapping_between_failure_states_is_one_alert_not_many() {
    // Kubernetes backs a failing pod off by flipping its status between
    // Error and CrashLoopBackOff (and ErrImagePull / ImagePullBackOff).
    let mut c = NotificationCenter::default();
    c.observe_pods("ctx", "prod", &[flat("web", "Running", 0)]);
    for status in [
        "Error",
        "CrashLoopBackOff",
        "Error",
        "CrashLoopBackOff",
        "Error",
    ] {
        c.observe_pods("ctx", "prod", &[flat("web", status, 1)]);
    }
    assert_eq!(c.items().count(), 1, "{:?}", messages(&c));
}

#[test]
fn a_pod_that_broke_before_the_baseline_stays_quiet_while_it_flaps() {
    let mut c = NotificationCenter::default();
    c.observe_pods("ctx", "prod", &[flat("web", "ErrImagePull", 0)]);
    c.observe_pods("ctx", "prod", &[flat("web", "ImagePullBackOff", 0)]);
    c.observe_pods("ctx", "prod", &[flat("web", "ErrImagePull", 0)]);
    assert_eq!(c.items().count(), 0);
}

#[test]
fn a_flapping_pod_that_recovers_and_breaks_again_alerts_again() {
    let mut c = NotificationCenter::default();
    c.observe_pods("ctx", "prod", &[flat("web", "Running", 0)]);
    c.observe_pods("ctx", "prod", &[flat("web", "Error", 1)]);
    c.observe_pods("ctx", "prod", &[flat("web", "Running", 1)]);
    c.observe_pods("ctx", "prod", &[flat("web", "CrashLoopBackOff", 2)]);
    let levels: Vec<Level> = c.items().map(|n| n.level).collect();
    assert_eq!(levels, vec![Level::Error, Level::Success, Level::Error]);
}

#[tokio::test]
async fn a_new_pod_alert_pops_up_as_a_toast_and_is_listed_once() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    let ch = "watch:test-cluster:prod:pods".to_string();
    app.handle_stream_event(ch.clone(), json!([flat("web", "Running", 0)]));
    assert!(app.toast.is_none(), "the baseline is silent");

    app.handle_stream_event(ch, json!([flat("web", "CrashLoopBackOff", 1)]));
    let (msg, _, style) = app.toast.clone().expect("the alert shows as a toast");
    assert!(msg.contains("prod/web") && msg.contains("CrashLoopBackOff"), "{msg}");
    assert_eq!(style, Theme::status_error());
    assert_eq!(app.notifications.items().count(), 1, "not recorded twice");
}

#[tokio::test]
async fn several_alerts_in_one_refresh_toast_a_summary_and_list_each() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    let ch = "watch:test-cluster:prod:pods".to_string();
    app.handle_stream_event(ch.clone(), json!([flat("a", "Running", 0), flat("b", "Running", 0)]));
    app.handle_stream_event(
        ch,
        json!([flat("a", "CrashLoopBackOff", 1), flat("b", "ImagePullBackOff", 0)]),
    );
    let (msg, _, _) = app.toast.clone().expect("a toast");
    assert!(msg.contains("2"), "a summary names the count: {msg}");
    assert_eq!(app.notifications.items().count(), 2);
}

#[test]
fn the_center_says_whether_an_unread_alert_is_an_error() {
    let mut c = NotificationCenter::default();
    assert!(!c.unread_has_error());
    c.push(Level::Warn, "ui", "careful");
    assert!(!c.unread_has_error(), "warnings alone are not errors");
    c.push(Level::Error, "ui", "broken");
    assert!(c.unread_has_error());
    c.mark_read();
    assert!(!c.unread_has_error());
}

#[test]
fn a_pod_recreated_under_the_same_name_does_not_inherit_the_old_ones_state() {
    let mut c = NotificationCenter::default();
    c.observe_pods("ctx", "prod", &[flat("db-0", "Running", 0)]);
    c.observe_pods("ctx", "prod", &[flat("db-0", "CrashLoopBackOff", 3)]);
    assert_eq!(c.items().count(), 1);

    // db-0 is deleted: the snapshot no longer lists it.
    c.observe_pods("ctx", "prod", &[flat("web", "Running", 0)]);
    // Its healthy replacement must not be reported as a recovery...
    c.observe_pods("ctx", "prod", &[flat("web", "Running", 0), flat("db-0", "Running", 0)]);
    assert_eq!(c.items().count(), 1, "no false recovery for a new pod");

    // ...and a replacement that fails is a new failure, not a suppressed repeat.
    c.observe_pods("ctx", "prod", &[flat("web", "Running", 0)]);
    c.observe_pods("ctx", "prod", &[flat("web", "Running", 0), flat("db-0", "ImagePullBackOff", 0)]);
    assert_eq!(c.items().count(), 2);
    assert_eq!(c.items().next().unwrap().level, Level::Error);
}

#[test]
fn pruning_stays_inside_the_snapshots_namespace() {
    let mut c = NotificationCenter::default();
    c.observe_pods("ctx", "prod", &[flat("a", "Running", 0)]);
    c.observe_pods("ctx", "dev", &[flat("b", "Running", 0)]);
    // A prod snapshot without "a" must not forget dev's "b".
    c.observe_pods("ctx", "prod", &[]);
    c.observe_pods("ctx", "dev", &[flat("b", "CrashLoopBackOff", 1)]);
    assert_eq!(c.items().count(), 1, "dev/b was still known, so this is a real transition");
}

#[tokio::test]
async fn older_alerts_can_be_scrolled_to_and_stay_unread_until_they_are() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    for i in 0..100 {
        app.notifications.push(Level::Error, "ctx", format!("boom-{i:03}"));
    }
    app.execute_command_target(CommandTarget::Notifications).await;
    let screen = common::render_app(&mut app, 120, 24);
    let rows = app.notifications_rows;
    assert!(rows < 100 && screen.contains("boom-099"));
    assert!(!screen.contains("boom-000"), "the oldest is below the fold");
    assert_eq!(app.notifications.unread(), 100 - rows, "only what was drawn is read");
    assert!(app.notifications.unread_has_error());

    common::press(&mut app, common::key(KeyCode::PageDown)).await;
    let screen = common::render_app(&mut app, 120, 24);
    assert!(!screen.contains("boom-099"), "paged past the newest:\n{screen}");
    assert_eq!(app.notifications.unread(), 100 - 2 * rows);

    common::press(&mut app, common::ch('G')).await;
    let screen = common::render_app(&mut app, 120, 24);
    assert!(screen.contains("boom-000"), "End reaches the oldest:\n{screen}");
    assert_eq!(app.notifications.unread(), 0);

    // Pushing past the end does not wind up a hidden counter.
    for _ in 0..30 {
        common::press(&mut app, common::key(KeyCode::Down)).await;
    }
    common::press(&mut app, common::ch('k')).await;
    let screen = common::render_app(&mut app, 120, 24);
    assert!(screen.contains("boom-001"), "one step up shows one newer entry:\n{screen}");

    common::press(&mut app, common::key(KeyCode::Home)).await;
    let screen = common::render_app(&mut app, 120, 24);
    assert!(screen.contains("boom-099"));
}
