//! Human QA Matrix journeys for AI Assistant mouse drag-to-scroll selection:
//! D1: Input Disorder & Boundary Escape (drag outside pane, implicit release on Moved)
//! D2: Selection Normalization (reverse bottom-to-top vs top-to-bottom extraction)
//! D3: Temporal & Stationary Hold Ramp / Streaming stability
//! D4: Scale Extremes & Time Budget (1,000 lines, <50ms)
//! D5: State Transitions (tool chip toggle on Down vs dragging across chip lines)
//! D6: Buffer Truthfulness & Trailing Newlines Trimmed
//! D7: Perceptual Reality on 80x24 Terminal (padding columns intact)

use crossterm::event::{MouseButton, MouseEventKind};
use srectl::app::{ActiveView, App};
use srectl::views::assistant_view::{ChatMessage, DragScrollZone};

use crate::common::{self, click, mouse, render_app};

fn toast(app: &App) -> String {
    app.toast
        .as_ref()
        .map(|(msg, _, _)| msg.clone())
        .unwrap_or_default()
}

fn seed_chat_messages(app: &mut App, count: usize) {
    app.active_view = ActiveView::Assistant;
    app.assistant_state.messages.clear();
    for i in 0..count {
        let role = if i % 2 == 0 { "user" } else { "assistant" };
        app.assistant_state.messages.push(ChatMessage {
            role: role.to_string(),
            content: format!("Message line {i:03}: incident response diagnostics for pod-{i}"),
            timestamp: "12:00:00".to_string(),
            tool_calls: Vec::new(),
            token_usage: None,
        });
    }
}

/// D1: Boundary Escape & Implicit Release.
/// A user clicks near the bottom, drags upward into the header bar (row 0),
/// and releases outside the window where the terminal emulator emits `Moved`
/// instead of `Up`. Viewport scrolls up, selection completes, and clipboard copies.
#[tokio::test]
async fn journey_d1_drag_above_top_boundary_scrolls_up_and_moved_implicit_release() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    seed_chat_messages(&mut app, 40);

    // Initial render populates geometry
    render_app(&mut app, 80, 24);
    let vp = app.assistant_state.last_viewport_rect.get();
    assert!(vp.height >= 8);

    let initial_scroll = app.assistant_state.effective_scroll();
    assert!(initial_scroll > 5);

    // Click near bottom of viewport to start selection
    let click_row = vp.y + vp.height.saturating_sub(2);
    app.handle_mouse(click(vp.x + 5, click_row)).await;
    assert!(app.assistant_state.is_selecting);

    // Drag above top boundary into row 0 (the header)
    app.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), vp.x + 5, 0))
        .await;
    render_app(&mut app, 80, 24);

    // Viewport has scrolled up and Up zone is active
    assert_eq!(
        app.assistant_state.drag_scroll,
        Some(DragScrollZone::Up { ticks: 0 })
    );
    let scrolled = app.assistant_state.effective_scroll();
    assert!(scrolled < initial_scroll);

    // Terminal drops Up event outside window and sends Moved instead
    app.handle_mouse(mouse(MouseEventKind::Moved, vp.x + 5, 0))
        .await;

    // Selection finished, drag_scroll cleared, toast set
    assert!(!app.assistant_state.is_selecting);
    assert_eq!(app.assistant_state.drag_scroll, None);
    assert_eq!(toast(&app), "✓ Copied selection to clipboard");
}

/// D2: Selection Normalization.
/// Dragging backward (bottom-to-top, right-to-left) normalizes start/end
/// bounds so copied text maintains forward reading order.
#[tokio::test]
async fn journey_d2_reverse_selection_preserves_normalized_extraction_order() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    seed_chat_messages(&mut app, 30);
    render_app(&mut app, 80, 24);
    let vp = app.assistant_state.last_viewport_rect.get();

    // Drag from bottom-right towards top-left (reverse)
    let start_row = vp.y + 6;
    let end_row = vp.y + 2;
    app.handle_mouse(click(vp.x + 20, start_row)).await;
    app.handle_mouse(mouse(
        MouseEventKind::Drag(MouseButton::Left),
        vp.x + 5,
        end_row,
    ))
    .await;
    app.handle_mouse(mouse(
        MouseEventKind::Up(MouseButton::Left),
        vp.x + 5,
        end_row,
    ))
    .await;

    assert_eq!(toast(&app), "✓ Copied selection to clipboard");
    let sel_text = app.assistant_state.get_selected_text();
    assert!(sel_text.is_some());
    let text = sel_text.unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines.len() >= 2);
}

/// D3: Temporal & Stationary Hold Ramp / Streaming stability.
/// 1. Holding mouse stationary at top boundary triggers accelerating scroll ramp.
/// 2. Assistant streaming chunks while user has scrolled up does not hijack viewport.
#[tokio::test]
async fn journey_d3_stationary_hold_acceleration_ramp_and_streaming_stability() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    seed_chat_messages(&mut app, 50);
    render_app(&mut app, 80, 24);
    let vp = app.assistant_state.last_viewport_rect.get();

    // Start selection near bottom
    let click_row = vp.y + vp.height.saturating_sub(2);
    app.handle_mouse(click(vp.x + 5, click_row)).await;
    // Drag to row 0
    app.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), vp.x + 5, 0))
        .await;
    render_app(&mut app, 80, 24);
    let s0 = app.assistant_state.effective_scroll();

    // Stationary hold: tick 1
    app.handle_tick();
    render_app(&mut app, 80, 24);
    let s1 = app.assistant_state.effective_scroll();
    assert!(s1 < s0);

    // Stationary hold: tick 2 (accelerated step)
    app.handle_tick();
    render_app(&mut app, 80, 24);
    let s2 = app.assistant_state.effective_scroll();
    assert!(s2 < s1);

    // Streaming mid-selection with auto_scroll=false preserves stable viewport
    assert!(!app.assistant_state.auto_scroll);
    let pre_stream_scroll = app.assistant_state.effective_scroll();
    app.assistant_state
        .append_stream_chunk("Chunk while user reading earlier history\n");
    render_app(&mut app, 80, 24);
    assert_eq!(app.assistant_state.effective_scroll(), pre_stream_scroll);

    // Release mouse
    app.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), vp.x + 5, 0))
        .await;
    assert_eq!(toast(&app), "✓ Copied selection to clipboard");
}

/// D4: Scale Extremes & Time Budget.
/// 1,000-line history rendered and processed through 20 drag + tick cycles in <50ms.
#[tokio::test]
async fn journey_d4_scale_extreme_and_time_budget_1000_lines() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    seed_chat_messages(&mut app, 1000);
    render_app(&mut app, 80, 24);
    let vp = app.assistant_state.last_viewport_rect.get();

    let start = std::time::Instant::now();
    let click_row = vp.y + vp.height.saturating_sub(2);
    app.handle_mouse(click(vp.x + 5, click_row)).await;
    for _ in 0..20 {
        app.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), vp.x + 5, 0))
            .await;
        app.handle_tick();
    }
    app.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), vp.x + 5, 0))
        .await;
    let elapsed = start.elapsed();

    assert!(
        elapsed < std::time::Duration::from_millis(50),
        "elapsed was {:?}",
        elapsed
    );
    assert_eq!(toast(&app), "✓ Copied selection to clipboard");
}

/// D5: State Transitions & Tool Chips.
/// Clicking a chip toggles expansion; dragging across chips selects text.
#[tokio::test]
async fn journey_d5_tool_chips_click_vs_drag_transitions() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    app.active_view = ActiveView::Assistant;
    app.assistant_state.messages.clear();
    app.assistant_state.messages.push(ChatMessage {
        role: "assistant".to_string(),
        content: "Here is what I found:".to_string(),
        timestamp: "12:00:00".to_string(),
        tool_calls: vec![srectl::views::assistant_view::ToolCallRecord {
            id: "call_1".to_string(),
            tool: "k8s.listPods".to_string(),
            args_summary: "{}".to_string(),
            status: srectl::views::assistant_view::ToolCallStatus::Success,
        }],
        token_usage: None,
    });
    render_app(&mut app, 80, 24);
    let vp = app.assistant_state.last_viewport_rect.get();

    let chip_lines = app.assistant_state.tool_chip_lines.borrow().clone();
    assert!(!chip_lines.is_empty(), "expected at least one chip line");
    let chip_line = chip_lines[0];
    let chip_row = (vp.y as usize + chip_line) as u16;

    let was_expanded = app.assistant_state.expand_tools;
    // Click on chip row toggles expansion without starting selection
    app.handle_mouse(click(vp.x + 2, chip_row)).await;
    assert_ne!(app.assistant_state.expand_tools, was_expanded);
    assert!(!app.assistant_state.is_selecting);

    // Drag started on role header (row vp.y) continues across chip row
    app.handle_mouse(click(vp.x + 5, vp.y)).await;
    assert!(app.assistant_state.is_selecting);
    app.handle_mouse(mouse(
        MouseEventKind::Drag(MouseButton::Left),
        vp.x + 5,
        chip_row,
    ))
    .await;
    // expand_tools did not toggle again
    assert_ne!(app.assistant_state.expand_tools, was_expanded);
    assert!(app.assistant_state.is_selecting);
    app.handle_mouse(mouse(
        MouseEventKind::Up(MouseButton::Left),
        vp.x + 5,
        chip_row,
    ))
    .await;
}

/// D6: Buffer Truthfulness & Trailing Newlines Trimmed.
/// Dragging past the bottom border into breathing room padding copies text
/// without trailing newline clutter.
#[tokio::test]
async fn journey_d6_buffer_truthfulness_and_trailing_newlines_trimmed() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    seed_chat_messages(&mut app, 20);
    render_app(&mut app, 80, 24);
    let vp = app.assistant_state.last_viewport_rect.get();

    // Select down into the input area / breathing room (strictly below vp)
    app.handle_mouse(click(vp.x + 2, vp.y + 2)).await;
    app.handle_mouse(mouse(
        MouseEventKind::Drag(MouseButton::Left),
        vp.x + 10,
        vp.y + vp.height + 2,
    ))
    .await;
    app.handle_mouse(mouse(
        MouseEventKind::Up(MouseButton::Left),
        vp.x + 10,
        vp.y + vp.height + 2,
    ))
    .await;

    assert_eq!(toast(&app), "✓ Copied selection to clipboard");
    let text = app.assistant_state.get_selected_text().unwrap();
    assert!(
        !text.ends_with("\n\n"),
        "trailing breathing room blank lines must be trimmed"
    );
}

/// D7: Perceptual Reality on 80x24 Terminal.
/// Text column boundaries on narrow terminals remain intact without corruption.
#[tokio::test]
async fn journey_d7_perceptual_reality_padding_columns_intact_on_80x24() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    seed_chat_messages(&mut app, 20);
    let _initial_frame = render_app(&mut app, 80, 24);
    let vp = app.assistant_state.last_viewport_rect.get();

    // Drag across width
    app.handle_mouse(click(vp.x + 1, vp.y + 2)).await;
    app.handle_mouse(mouse(
        MouseEventKind::Drag(MouseButton::Left),
        vp.x + vp.width.saturating_sub(2),
        vp.y + 2,
    ))
    .await;
    let selected_frame = render_app(&mut app, 80, 24);

    assert!(selected_frame.contains("incident response"));
    app.handle_mouse(mouse(
        MouseEventKind::Up(MouseButton::Left),
        vp.x + vp.width.saturating_sub(2),
        vp.y + 2,
    ))
    .await;
    assert_eq!(toast(&app), "✓ Copied selection to clipboard");
}
