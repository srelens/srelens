//! Journey tests for `:mux` multi-cluster terminal multiplexer.
//!
//! Covers Human QA Matrix dimensions D1–D7:
//! - D1: Input Disorder & Interleaving (Typing in picker, 'D' does not trigger close modal, Tab does not change panes, Ctrl+b cycles focus)
//! - D2: Ambiguity & Collisions (Disambiguating clusters with shared prefixes)
//! - D4: Boundary & Scale Extremes (80x24 narrow terminal dual split, dynamic resizing)
//! - D5: State Transitions & Re-Entry (Open split -> activate -> Shift+D prompt -> cancel/confirm -> restore)
//! - D7: Visual & Perceptual Reality (Active pane indicator ● ACTIVE, dual pane headers, border styling)

use crossterm::event::{KeyCode, KeyModifiers};
use srectl::app::ActiveView;
use srectl::ui::dialogs::Modal;
use srectl::ui::statusbar::InputMode;
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

/// D1 & D5: Assistant state isolation and command bar reset on focus cycling.
#[tokio::test]
async fn journey_mux_assistant_state_and_command_bar_isolation() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app_with("cluster-left", "default").await;
    app.contexts = vec![
        ctx("cluster-left", "c-left", "default"),
        ctx("cluster-right", "c-right", "default"),
    ];

    // Open mux and select cluster-right for pane 2
    press(&mut app, ch(':')).await;
    type_str(&mut app, "mux").await;
    press(&mut app, key(KeyCode::Enter)).await;
    press(&mut app, key(KeyCode::Enter)).await;

    assert_eq!(app.mux_state.as_ref().unwrap().focused_pane, 1);
    assert_eq!(app.active_context, "cluster-right");

    // Open assistant in right pane
    press(&mut app, ch(':')).await;
    type_str(&mut app, "assistant").await;
    press(&mut app, key(KeyCode::Enter)).await;
    assert!(matches!(app.active_view, ActiveView::Assistant));

    // Type text into assistant input in right pane
    type_str(&mut app, "query-for-right-cluster").await;
    assert_eq!(app.assistant_state.input, "query-for-right-cluster");
    assert_eq!(app.assistant_state.context_name, "cluster-right");

    // Cycle focus to Left pane (<Ctrl+b>)
    press(&mut app, ctrl('b')).await;
    assert_eq!(app.mux_state.as_ref().unwrap().focused_pane, 0);
    assert_eq!(app.active_context, "cluster-left");

    // Left pane assistant state must NOT share the prompt from right pane!
    assert_ne!(app.assistant_state.input, "query-for-right-cluster");
    assert_eq!(app.assistant_state.input, "");

    // In left pane, open assistant and type a distinct query
    press(&mut app, ch(':')).await;
    type_str(&mut app, "assistant").await;
    press(&mut app, key(KeyCode::Enter)).await;
    assert!(matches!(app.active_view, ActiveView::Assistant));
    type_str(&mut app, "query-for-left-cluster").await;
    assert_eq!(app.assistant_state.input, "query-for-left-cluster");

    // Exit assistant back to table view
    press(&mut app, key(KeyCode::Esc)).await;
    assert!(matches!(app.active_view, ActiveView::Table(_)));

    // Enter command mode and type partial command in left pane
    press(&mut app, ch(':')).await;
    type_str(&mut app, "partial-command").await;
    assert_eq!(app.input_mode, InputMode::Command);
    assert_eq!(app.command_buffer, "partial-command");

    // Switch focus back to right pane (<Ctrl+b>)
    press(&mut app, ctrl('b')).await;
    assert_eq!(app.mux_state.as_ref().unwrap().focused_pane, 1);
    assert_eq!(app.active_context, "cluster-right");

    // Command bar must be cleanly reset to Normal mode with empty buffer!
    assert_eq!(app.input_mode, InputMode::Normal);
    assert!(app.command_buffer.is_empty());

    // Right pane was in Assistant view, its input must be preserved!
    assert!(matches!(app.active_view, ActiveView::Assistant));
    assert_eq!(app.assistant_state.input, "query-for-right-cluster");
    assert_eq!(app.assistant_state.context_name, "cluster-right");

    // Switch back to left pane: opening assistant restores its isolated input!
    press(&mut app, ctrl('b')).await;
    assert_eq!(app.active_context, "cluster-left");
    press(&mut app, ch(':')).await;
    type_str(&mut app, "assistant").await;
    press(&mut app, key(KeyCode::Enter)).await;
    assert_eq!(app.assistant_state.input, "query-for-left-cluster");
}

/// D2 & D6: Modal focus lock and duplicate context collision prevention.
#[tokio::test]
async fn journey_mux_modal_focus_lock_and_collision_guard() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app_with("cluster-left", "default").await;
    app.contexts = vec![
        ctx("cluster-left", "c-left", "default"),
        ctx("cluster-right", "c-right", "default"),
    ];

    // Open mux and select cluster-right
    press(&mut app, ch(':')).await;
    type_str(&mut app, "mux").await;
    press(&mut app, key(KeyCode::Enter)).await;
    press(&mut app, key(KeyCode::Enter)).await;

    // Render to set pane rects
    let _screen = render_app(&mut app, 100, 30);
    assert_eq!(app.mux_state.as_ref().unwrap().focused_pane, 1);

    // Open Shift+D close confirm modal
    press(&mut app, shift(KeyCode::Char('D'))).await;
    assert!(app.modal.is_some());

    // While modal is open, <Ctrl+b> must NOT change pane focus!
    press(&mut app, ctrl('b')).await;
    assert_eq!(
        app.mux_state.as_ref().unwrap().focused_pane,
        1,
        "Focus switch must be locked while modal is open"
    );

    // While modal is open, mouse click on left pane must NOT switch focus
    app.handle_mouse(click(20, 10)).await;
    assert_eq!(
        app.mux_state.as_ref().unwrap().focused_pane,
        1,
        "Mouse click must not switch focus while modal is open"
    );

    // Dismiss modal with 'n'
    press(&mut app, ch('n')).await;
    assert!(app.modal.is_none());

    // Now <Ctrl+b> works normally
    press(&mut app, ctrl('b')).await;
    assert_eq!(app.mux_state.as_ref().unwrap().focused_pane, 0);
    assert_eq!(app.active_context, "cluster-left");

    // Attempting to switch context in left pane to "cluster-right" (already open on right pane)
    // should be blocked by our collision guard!
    app.switch_context("cluster-right".to_string()).await;

    assert_eq!(
        app.active_context, "cluster-left",
        "Switching to context active in other pane must be refused"
    );
    assert!(
        app.toast
            .as_ref()
            .unwrap()
            .0
            .contains("already open in the other pane"),
        "Toast must explain why context switch was refused"
    );
}

/// D3: Background stream routing into inactive pane without active namespace corruption.
#[tokio::test]
async fn journey_mux_inactive_stream_and_tick_routing() {
    use serde_json::json;

    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app_with("cluster-left", "default").await;
    app.contexts = vec![
        ctx("cluster-left", "c-left", "default"),
        ctx("cluster-right", "c-right", "prod-ns"),
    ];

    // Open mux and select cluster-right
    press(&mut app, ch(':')).await;
    type_str(&mut app, "mux").await;
    press(&mut app, key(KeyCode::Enter)).await;
    press(&mut app, key(KeyCode::Enter)).await;

    // Focused pane is 1 (cluster-right), inactive pane is 0 (cluster-left)
    assert_eq!(app.active_context, "cluster-right");
    let inactive = app
        .mux_state
        .as_mut()
        .unwrap()
        .inactive_pane
        .as_mut()
        .unwrap();
    inactive.current_watch_channel = Some("watch:cluster-left:default:pods".to_string());
    inactive.filter_buffer = "filter-term".to_string();

    // Stream event for inactive pane arrives!
    let pod_data = json!([
        {"name": "pod-1", "ready": "1/1", "status": "Running", "restarts": "0", "age": "1d"},
        {"name": "pod-2-filter-term", "ready": "1/1", "status": "Running", "restarts": "0", "age": "2d"}
    ]);
    app.handle_stream_event("watch:cluster-left:default:pods".to_string(), pod_data);

    // Verify inactive pane table got the items filtered with its OWN filter_buffer
    let inactive = app
        .mux_state
        .as_ref()
        .unwrap()
        .inactive_pane
        .as_ref()
        .unwrap();
    if let ActiveView::Table(ref t) = inactive.active_view {
        assert_eq!(
            t.raw_items.len(),
            2,
            "inactive pane must store all raw items"
        );
        assert_eq!(
            t.filtered_indices.len(),
            1,
            "inactive pane must filter with its own filter_buffer"
        );
        assert_eq!(
            t.raw_items[t.filtered_indices[0]]["name"],
            "pod-2-filter-term"
        );
    } else {
        panic!("inactive active_view should be Table");
    }

    // Verify active pane's table was NOT affected or corrupted
    if let ActiveView::Table(ref t) = app.active_view {
        assert!(
            t.raw_items.is_empty(),
            "active pane's table must remain unaffected"
        );
    }

    // Namespace discovery for inactive context
    let ns_data = json!([
        {"name": "kube-system"},
        {"name": "default"},
        {"name": "monitoring"}
    ]);
    app.handle_stream_event("discovery:namespaces:cluster-left".to_string(), ns_data);
    let inactive = app
        .mux_state
        .as_ref()
        .unwrap()
        .inactive_pane
        .as_ref()
        .unwrap();
    assert_eq!(
        inactive.namespaces,
        vec!["default", "kube-system", "monitoring"]
    );

    // handle_tick ticks inactive pane without panicking
    app.handle_tick();
}

/// D2 & D5: AI Assistant Settings Isolation between Left and Right panes.
#[tokio::test]
async fn journey_mux_ai_settings_isolation() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app_with("cluster-left", "default").await;
    app.contexts = vec![
        ctx("cluster-left", "left", "default"),
        ctx("cluster-right", "right", "default"),
    ];

    // Open mux split with cluster-right
    press(&mut app, ch(':')).await;
    type_str(&mut app, "mux").await;
    press(&mut app, key(KeyCode::Enter)).await;
    press(&mut app, key(KeyCode::Down)).await;
    press(&mut app, key(KeyCode::Enter)).await;

    assert_eq!(app.active_context, "cluster-right");
    assert_eq!(app.mux_state.as_ref().unwrap().focused_pane, 1);

    // 1. Configure AI settings in Right pane: set Cursor agent with custom model
    app.ai_settings.agent = srelens_agent::adapter::AgentKind::Cursor;
    app.ai_settings.set_agent_model(
        srelens_agent::adapter::AgentKind::Cursor,
        Some("claude-3-5-sonnet".to_string()),
    );

    // 2. Switch focus to Left pane via Ctrl+b
    press(&mut app, ctrl('b')).await;
    assert_eq!(app.active_context, "cluster-left");
    assert_eq!(app.mux_state.as_ref().unwrap().focused_pane, 0);

    // Left pane must retain default Srelens agent and NOT inherit Cursor
    assert_eq!(
        app.ai_settings.agent,
        srelens_agent::adapter::AgentKind::Srelens,
        "Left pane must not inherit right pane's agent change"
    );

    // Configure Left pane to use Gemini provider with gemini-1.5-pro
    app.ai_settings.default_provider = srectl::ai_config::AiProvider::Gemini;
    app.ai_settings
        .models
        .insert("gemini".to_string(), "gemini-1.5-pro".to_string());

    // 3. Switch focus back to Right pane via Ctrl+b
    press(&mut app, ctrl('b')).await;
    assert_eq!(app.active_context, "cluster-right");
    assert_eq!(app.mux_state.as_ref().unwrap().focused_pane, 1);

    // Right pane must preserve its Cursor agent and NOT have Gemini
    assert_eq!(
        app.ai_settings.agent,
        srelens_agent::adapter::AgentKind::Cursor,
        "Right pane must retain its Cursor agent"
    );
    assert_ne!(
        app.ai_settings.default_provider,
        srectl::ai_config::AiProvider::Gemini,
        "Right pane must not inherit left pane's Gemini provider"
    );

    // Open assistant view in right pane
    press(&mut app, ch(':')).await;
    type_str(&mut app, "assistant").await;
    press(&mut app, key(KeyCode::Enter)).await;

    // Switch to left pane and open assistant view as well
    press(&mut app, ctrl('b')).await;
    assert_eq!(app.active_context, "cluster-left");
    press(&mut app, ch(':')).await;
    type_str(&mut app, "assistant").await;
    press(&mut app, key(KeyCode::Enter)).await;

    // 4. Render split screen and verify distinct visual headers for both panes
    let screen = render_app(&mut app, 140, 30);
    assert!(
        screen.contains("Google Gemini") && screen.contains("gemini-1.5-pro"),
        "Left pane assistant banner must show Google Gemini and gemini-1.5-pro:\n{screen}"
    );
    assert!(
        screen.contains("Cursor") && screen.contains("claude-3-5-sonnet"),
        "Right pane assistant banner must show Cursor and claude-3-5-sonnet:\n{screen}"
    );
}

/// D5 & D6: Shift+D closes the currently focused pane (Left pane close promotion).
#[tokio::test]
async fn journey_mux_close_focused_left_pane() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app_with("cluster-left", "default").await;
    app.contexts = vec![
        ctx("cluster-left", "left", "default"),
        ctx("cluster-right", "right", "default"),
    ];

    // Open mux split with cluster-right
    press(&mut app, ch(':')).await;
    type_str(&mut app, "mux").await;
    press(&mut app, key(KeyCode::Enter)).await;
    press(&mut app, key(KeyCode::Down)).await;
    press(&mut app, key(KeyCode::Enter)).await;

    assert_eq!(app.active_context, "cluster-right");
    assert_eq!(app.mux_state.as_ref().unwrap().focused_pane, 1);

    // Switch focus to Left pane (pane 0)
    press(&mut app, ctrl('b')).await;
    assert_eq!(app.active_context, "cluster-left");
    assert_eq!(app.mux_state.as_ref().unwrap().focused_pane, 0);

    // Press Shift+D to initiate close while focused on Left pane
    press(&mut app, shift(KeyCode::Char('D'))).await;
    assert!(matches!(
        app.modal,
        Some(Modal::Confirm { ref action_name, .. }) if action_name == "close_mux"
    ));

    // Confirm close with 'y'
    press(&mut app, ch('y')).await;
    assert!(app.modal.is_none());
    assert!(app.mux_state.is_none(), "Mux split must be closed");

    // The focused Left pane was closed; Right pane survives and is now full screen
    assert_eq!(
        app.active_context, "cluster-right",
        "Right pane must survive when Left pane is closed"
    );
    assert!(
        app.toast
            .as_ref()
            .unwrap()
            .0
            .contains("Closed pane 1: 'cluster-left'"),
        "Toast must report closing pane 1 cluster-left"
    );
}

/// D5 & D6: Shift+D closes the currently focused pane (Right pane close).
#[tokio::test]
async fn journey_mux_close_focused_right_pane() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app_with("cluster-left", "default").await;
    app.contexts = vec![
        ctx("cluster-left", "left", "default"),
        ctx("cluster-right", "right", "default"),
    ];

    // Open mux split with cluster-right
    press(&mut app, ch(':')).await;
    type_str(&mut app, "mux").await;
    press(&mut app, key(KeyCode::Enter)).await;
    press(&mut app, key(KeyCode::Down)).await;
    press(&mut app, key(KeyCode::Enter)).await;

    assert_eq!(app.active_context, "cluster-right");
    assert_eq!(app.mux_state.as_ref().unwrap().focused_pane, 1);

    // Press Shift+D while focused on Right pane
    press(&mut app, shift(KeyCode::Char('D'))).await;
    assert!(matches!(
        app.modal,
        Some(Modal::Confirm { ref action_name, .. }) if action_name == "close_mux"
    ));

    // Confirm close with 'y'
    press(&mut app, ch('y')).await;
    assert!(app.modal.is_none());
    assert!(app.mux_state.is_none(), "Mux split must be closed");

    // The focused Right pane was closed; Left pane survives and is restored
    assert_eq!(
        app.active_context, "cluster-left",
        "Left pane must survive when Right pane is closed"
    );
    assert!(
        app.toast
            .as_ref()
            .unwrap()
            .0
            .contains("Closed pane 2: 'cluster-right'"),
        "Toast must report closing pane 2 cluster-right"
    );
}
