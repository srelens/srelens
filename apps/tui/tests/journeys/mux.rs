//! Journey tests for `:mux` multi-cluster terminal multiplexer.
//!
//! Covers Human QA Matrix dimensions D1–D7:
//! - D1: Input Disorder & Interleaving (Typing in picker, 'D' does not trigger close modal, Tab does not change panes, Ctrl+b cycles focus)
//! - D2: Ambiguity & Collisions (Disambiguating clusters with shared prefixes)
//! - D4: Boundary & Scale Extremes (80x24 narrow terminal dual split, dynamic resizing)
//! - D5: State Transitions & Re-Entry (Open split -> activate -> Shift+D prompt -> cancel/confirm -> restore)
//! - D7: Visual & Perceptual Reality (Active pane indicator ● ACTIVE, dual pane headers, border styling)

use crossterm::event::{KeyCode, KeyModifiers};
use srectl::ui::dialogs::Modal;
use srelens_kube::contexts::ContextDto;

use crate::common::{self, ch, click, ctrl, key, mouse, press, render_app, shift, type_str};

fn ctx(name: &str, cluster: &str, namespace: &str) -> ContextDto {
    ContextDto {
        name: name.to_string(),
        stable_id: format!("file/{}", name),
        key: format!("file/{}", name),
        pinned_id: None,
        cluster: cluster.to_string(),
        server: format!("https://{}.example.invalid", cluster),
        namespace: namespace.to_string(),
        is_current: false,
        is_local: false,
        provider: None,
        source_file: "cluster-config.yaml".to_string(),
        auth_kind: "token".to_string(),
    }
}

/// D1 & D2: Open `:mux`, filter context picker, ensure 'D' types into filter
/// without closing, and activate the right cluster context.
#[tokio::test]
async fn journey_mux_open_filter_and_activate_cluster() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app_with("prod-us-east", "default").await;
    app.contexts = vec![
        ctx("prod-us-east", "prod-cluster", "default"),
        ctx("stage-us-east", "stage-cluster", "stage"),
        ctx("stage-eu-west", "stage-eu-cluster", "stage"),
    ];

    // 1. Enter command mode and type :mux
    press(&mut app, ch(':')).await;
    type_str(&mut app, "mux").await;
    press(&mut app, key(KeyCode::Enter)).await;

    // Split should now be active with Context Picker on the right pane
    let mux = app.mux_state.as_ref().expect("mux should be active");
    assert_eq!(
        mux.focused_pane, 1,
        "right pane should be focused initially for context selection"
    );
    assert!(mux.picker.is_some(), "right pane must show context picker");

    // 2. Type "stage" into filter
    type_str(&mut app, "stage").await;
    let mux = app.mux_state.as_ref().unwrap();
    let picker = mux.picker.as_ref().unwrap();
    assert_eq!(picker.filter, "stage");
    assert_eq!(picker.filtered_items().len(), 2);

    // 3. Type capital 'D' - must NOT trigger close split modal!
    press(&mut app, shift(KeyCode::Char('D'))).await;
    assert!(
        app.modal.is_none(),
        "typing 'D' in picker filter must not open confirm close modal"
    );
    let mux = app.mux_state.as_ref().unwrap();
    let picker = mux.picker.as_ref().unwrap();
    assert_eq!(picker.filter, "stageD");

    // Backspace 'D'
    press(&mut app, key(KeyCode::Backspace)).await;
    let mux = app.mux_state.as_ref().unwrap();
    let picker = mux.picker.as_ref().unwrap();
    assert_eq!(picker.filter, "stage");

    // 4. Press Enter to select the highlighted context ("stage-us-east")
    press(&mut app, key(KeyCode::Enter)).await;

    // Both panes should now be active
    let mux = app.mux_state.as_ref().expect("mux should still be active");
    assert!(
        mux.picker.is_none(),
        "picker must be dismissed after context selection"
    );
    assert_eq!(mux.focused_pane, 1, "right pane should remain focused");
    assert_eq!(
        app.active_context, "stage-us-east",
        "app active context should now be right pane"
    );
    assert_eq!(
        mux.inactive_pane.as_ref().unwrap().context,
        "prod-us-east",
        "inactive pane must preserve left cluster"
    );
}

/// D1: Focus cycling via `<Ctrl+b>` and verifying `<Tab>` does NOT change pane focus.
#[tokio::test]
async fn journey_mux_focus_cycling_and_tab_invariance() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app_with("cluster-left", "default").await;
    app.contexts = vec![
        ctx("cluster-left", "c-left", "default"),
        ctx("cluster-right", "c-right", "default"),
    ];

    // Open mux and navigate down to select the second context ("cluster-right")
    press(&mut app, ch(':')).await;
    type_str(&mut app, "mux").await;
    press(&mut app, key(KeyCode::Enter)).await;
    press(&mut app, key(KeyCode::Down)).await;
    press(&mut app, key(KeyCode::Enter)).await;

    assert_eq!(app.mux_state.as_ref().unwrap().focused_pane, 1);
    assert_eq!(app.active_context, "cluster-right");

    // Press <Tab> -> pane focus MUST NOT change!
    press(&mut app, key(KeyCode::Tab)).await;
    assert_eq!(
        app.mux_state.as_ref().unwrap().focused_pane,
        1,
        "<Tab> must never change pane focus"
    );

    // Press <Ctrl+b> -> cycles focus to Left pane (pane 0)
    press(&mut app, ctrl('b')).await;
    assert_eq!(app.mux_state.as_ref().unwrap().focused_pane, 0);
    assert_eq!(app.active_context, "cluster-left");

    // Press <Ctrl+b> -> cycles focus back to Right pane (pane 1)
    press(&mut app, ctrl('b')).await;
    assert_eq!(app.mux_state.as_ref().unwrap().focused_pane, 1);
    assert_eq!(app.active_context, "cluster-right");
}

/// D4 & D5: Mouse click focus, dynamic resizing via drag & Alt keys, and Shift+D closing.
#[tokio::test]
async fn journey_mux_mouse_click_resizing_and_close_confirmation() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app_with("cluster-left", "default").await;
    app.contexts = vec![
        ctx("cluster-left", "c-left", "default"),
        ctx("cluster-right", "c-right", "default"),
    ];

    // Open mux and select second context
    press(&mut app, ch(':')).await;
    type_str(&mut app, "mux").await;
    press(&mut app, key(KeyCode::Enter)).await;
    press(&mut app, key(KeyCode::Down)).await;
    press(&mut app, key(KeyCode::Enter)).await;

    // Render app at 100x30 to establish pane rects
    let _screen = render_app(&mut app, 100, 30);
    let (left_r, right_r) = app.mux_state.as_ref().unwrap().pane_rects;
    assert_eq!(left_r.width, 50);
    assert_eq!(right_r.width, 50);

    // Click inside left pane (col 20, row 10) -> switches focus to left pane
    app.handle_mouse(click(20, 10)).await;
    assert_eq!(app.mux_state.as_ref().unwrap().focused_pane, 0);
    assert_eq!(app.active_context, "cluster-left");

    // Click inside right pane (col 70, row 10) -> switches focus to right pane
    app.handle_mouse(click(70, 10)).await;
    assert_eq!(app.mux_state.as_ref().unwrap().focused_pane, 1);
    assert_eq!(app.active_context, "cluster-right");

    // Dynamic resizing via mouse drag
    use crossterm::event::{MouseButton, MouseEventKind};
    app.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 35, 10))
        .await;
    assert_eq!(app.mux_state.as_ref().unwrap().split_ratio, 35);

    // Dynamic resizing via keyboard Alt+Left / Alt+Right
    let mut alt_right = key(KeyCode::Right);
    alt_right.modifiers = KeyModifiers::ALT;
    press(&mut app, alt_right).await;
    assert_eq!(app.mux_state.as_ref().unwrap().split_ratio, 40);

    // Press Shift+D to initiate close split
    press(&mut app, shift(KeyCode::Char('D'))).await;
    assert!(matches!(
        app.modal,
        Some(Modal::Confirm { ref action_name, .. }) if action_name == "close_mux"
    ));

    // Cancel modal with 'n'
    press(&mut app, ch('n')).await;
    assert!(app.modal.is_none());
    assert!(
        app.mux_state.is_some(),
        "mux split should remain open after cancelling"
    );

    // Press Shift+D again and confirm with 'y'
    press(&mut app, shift(KeyCode::Char('D'))).await;
    press(&mut app, ch('y')).await;

    assert!(app.modal.is_none());
    assert!(
        app.mux_state.is_none(),
        "mux split must be closed after confirming"
    );
}

/// D7: Visual & Perceptual Reality on narrow 80x24 terminal screen.
#[tokio::test]
async fn journey_mux_visual_rendering_80x24() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app_with("cluster-alpha", "default").await;
    app.contexts = vec![
        ctx("cluster-alpha", "alpha", "default"),
        ctx("cluster-beta", "beta", "default"),
    ];

    press(&mut app, ch(':')).await;
    type_str(&mut app, "mux").await;
    press(&mut app, key(KeyCode::Enter)).await;
    press(&mut app, key(KeyCode::Down)).await;
    press(&mut app, key(KeyCode::Enter)).await;

    // Render narrow 80x24 buffer
    let screen = render_app(&mut app, 80, 24);

    // Assert visual truth in rendered screen buffer
    assert!(
        screen.contains("[1: cluster-alpha]"),
        "Screen should render pane 1 title:\n{screen}"
    );
    assert!(
        screen.contains("[2: cluster-beta]"),
        "Screen should render pane 2 title:\n{screen}"
    );
    assert!(
        screen.contains("● ACTIVE"),
        "Active pane indicator must be rendered:\n{screen}"
    );
}
