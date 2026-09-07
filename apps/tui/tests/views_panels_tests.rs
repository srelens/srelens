//! Integration tests for the smaller panel views under `src/views/`: their
//! `*State` constructors and mutators, and what `render_*` draws for each
//! branch (loading, error, empty, populated, wide, narrow).
//!
//! Nothing here touches a cluster, spawns a process, or depends on what is
//! installed on the machine. States are built through the public API and
//! rendered through ratatui's `TestBackend`.

mod common;

use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Position;
use ratatui::style::{Color, Style};
use ratatui::{Frame, Terminal};
use serde_json::{json, Value};
use srelens_kube::lineage::{LineageNode, LineageRelation};
use srelens_kube::metrics::MetricSample;
use srelens_tui::commands::{CrdMeta, PrinterColumn, ResourceKind};
use srelens_tui::theme::Theme;
use srelens_tui::views::cracked_lens::render_cracked_lens;
use srelens_tui::views::describe_view::{render_describe_view, DescribeViewState};
use srelens_tui::views::helm_view::{render_helm_view, HelmReleaseItem, HelmViewState};
use srelens_tui::views::logs_view::{render_logs_view, LogsViewState};
use srelens_tui::views::metrics_panel_view::{
    render_metrics_panel_modal, MetricsPanelState, MetricsTimeRange,
};
use srelens_tui::views::port_forward_view::{
    render_port_forward_view, PortForwardEntry, PortForwardViewState,
};
use srelens_tui::views::reason_rail::{
    render_reason_rail_modal, render_reason_rail_widget, tally_event_reasons, ReasonTally,
};
use srelens_tui::views::resource_table::{
    default_columns_for_kind, eval_crd_json_path, extract_field_str, is_event_warning_or_failure,
    render_resource_table, ResourceTableState, WorkloadSegment,
};
use srelens_tui::views::toolbox_view::{render_toolbox_view, ToolStatusItem, ToolboxViewState};
use srelens_tui::views::tree_view::{render_tree_view, TreeViewState};
use srelens_tui::views::{highlight_text_matches, sanitize_span_text};

/// Render one frame and hand back the raw buffer so a test can inspect cell
/// styles (the colour a status cell was drawn in), not only the text.
fn render_buffer<F>(width: u16, height: u16, draw: F) -> Buffer
where
    F: FnOnce(&mut Frame),
{
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal.draw(draw).expect("draw");
    terminal.backend().buffer().clone()
}

/// The row text of a buffer, trailing spaces trimmed.
fn buffer_row(buf: &Buffer, y: u16) -> String {
    let mut line = String::new();
    for x in 0..buf.area.width {
        line.push_str(buf[(x, y)].symbol());
    }
    line.trim_end().to_string()
}

/// Foreground colour of the first cell on row `y` whose text starts `needle`.
fn fg_of(buf: &Buffer, y: u16, needle: &str) -> Option<Color> {
    let row = buffer_row(buf, y);
    let col = row.find(needle)?;
    // `find` returns a byte offset; rows here are ASCII up to the needle
    // except for box-drawing borders, so count chars instead.
    let x = row[..col].chars().count() as u16;
    buf.cell(Position::new(x, y)).and_then(|c| c.style().fg)
}

/// Screen column (not byte offset) at which `needle` starts in `row`.
fn col_of(row: &str, needle: &str) -> u16 {
    let byte = row
        .find(needle)
        .unwrap_or_else(|| panic!("{needle:?} not in {row:?}"));
    row[..byte].chars().count() as u16
}

fn row_containing<'a>(lines: &'a [String], needle: &str) -> Option<(usize, &'a String)> {
    lines.iter().enumerate().find(|(_, l)| l.contains(needle))
}

// ---------------------------------------------------------------------------
// views/mod.rs: sanitize_span_text and highlight_text_matches
// ---------------------------------------------------------------------------

#[test]
fn sanitize_span_text_passes_plain_text_through_unchanged() {
    assert_eq!(sanitize_span_text("plain text 123"), "plain text 123");
    assert_eq!(sanitize_span_text(""), "");
}

#[test]
fn sanitize_span_text_expands_tabs_to_eight_column_stops() {
    assert_eq!(sanitize_span_text("\tx"), "        x");
    assert_eq!(sanitize_span_text("abc\tx"), "abc     x");
    assert_eq!(sanitize_span_text("12345678\tx"), "12345678        x");
}

#[test]
fn sanitize_span_text_drops_csi_and_two_byte_escape_sequences() {
    // CSI: ESC [ params final-byte.
    assert_eq!(sanitize_span_text("\u{1b}[1;32mok\u{1b}[0m"), "ok");
    // A non-CSI escape is ESC plus exactly one byte.
    assert_eq!(sanitize_span_text("a\u{1b}Mb"), "ab");
    // A trailing bare ESC has nothing to swallow and simply disappears.
    assert_eq!(sanitize_span_text("tail\u{1b}"), "tail");
}

#[test]
fn sanitize_span_text_flattens_newlines_and_removes_other_controls() {
    assert_eq!(sanitize_span_text("a\nb"), "a b");
    assert_eq!(sanitize_span_text("a\r\nb\u{7}\u{0}c"), "a bc");
    // A newline counts as one column for the following tab stop.
    assert_eq!(sanitize_span_text("\n\tx"), "        x");
}

#[test]
fn highlight_text_matches_returns_one_base_span_for_an_empty_query() {
    let spans = highlight_text_matches("hello", "", Style::default(), Style::default());
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].content, "hello");
}

#[test]
fn highlight_text_matches_splits_around_every_case_insensitive_hit() {
    let base = Style::default().fg(Color::White);
    let hit = Style::default().fg(Color::Yellow);
    let spans = highlight_text_matches("Error: an ERROR again", "error", base, hit);
    let parts: Vec<(&str, Option<Color>)> = spans
        .iter()
        .map(|s| (s.content.as_ref(), s.style.fg))
        .collect();
    assert_eq!(
        parts,
        vec![
            ("Error", Some(Color::Yellow)),
            (": an ", Some(Color::White)),
            ("ERROR", Some(Color::Yellow)),
            (" again", Some(Color::White)),
        ]
    );
}

#[test]
fn highlight_text_matches_falls_back_to_the_whole_text_when_nothing_matches() {
    let base = Style::default().fg(Color::White);
    let hit = Style::default().fg(Color::Yellow);
    let spans = highlight_text_matches("nothing here", "zzz", base, hit);
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].content, "nothing here");
    assert_eq!(spans[0].style.fg, Some(Color::White));
}

#[test]
fn highlight_text_matches_handles_a_match_that_ends_the_text() {
    let spans = highlight_text_matches("abcXYZ", "xyz", Style::default(), Style::default());
    let parts: Vec<&str> = spans.iter().map(|s| s.content.as_ref()).collect();
    assert_eq!(parts, vec!["abc", "XYZ"]);
}

// ---------------------------------------------------------------------------
// logs_view
// ---------------------------------------------------------------------------

fn logs() -> LogsViewState {
    LogsViewState::new(
        "web-0".to_string(),
        "default".to_string(),
        Some("app".to_string()),
        "chan-1".to_string(),
    )
}

#[test]
fn logs_view_starts_empty_and_following() {
    let state = logs();
    assert!(state.lines.is_empty());
    assert!(state.follow);
    assert!(!state.timestamps);
    assert!(!state.previous);
    assert!(!state.wrap);
    assert_eq!(state.scroll_offset, 0);
    assert!(state.search_query.is_empty());
    assert!(state.current_match_idx.is_none());
}

#[test]
fn logs_view_push_line_keeps_the_bottom_in_view_while_following() {
    let mut state = logs();
    for i in 0..5 {
        state.push_line(format!("line {i}"));
    }
    assert_eq!(state.lines.len(), 5);
    assert_eq!(state.scroll_offset, 4);
}

#[test]
fn logs_view_stops_following_when_the_user_scrolls_up() {
    let mut state = logs();
    for i in 0..10 {
        state.push_line(format!("line {i}"));
    }
    state.scroll_up(3);
    assert!(!state.follow);
    assert_eq!(state.scroll_offset, 6);
    // New lines no longer drag the viewport down.
    state.push_line("line 10".to_string());
    assert_eq!(state.scroll_offset, 6);
    // Scrolling up past the top clamps at zero.
    state.scroll_up(100);
    assert_eq!(state.scroll_offset, 0);
}

#[test]
fn logs_view_scroll_down_is_bounded_by_the_last_line() {
    let mut state = logs();
    for i in 0..4 {
        state.push_line(format!("line {i}"));
    }
    state.scroll_top();
    assert_eq!(state.scroll_offset, 0);
    assert!(!state.follow);
    state.scroll_down(2);
    assert_eq!(state.scroll_offset, 2);
    // A step that would run past the end is ignored entirely.
    state.scroll_down(5);
    assert_eq!(state.scroll_offset, 2);
}

#[test]
fn logs_view_toggle_follow_jumps_back_to_the_bottom() {
    let mut state = logs();
    for i in 0..6 {
        state.push_line(format!("line {i}"));
    }
    state.scroll_top();
    state.toggle_follow();
    assert!(state.follow);
    assert_eq!(state.scroll_offset, 5);
    state.toggle_follow();
    assert!(!state.follow);
}

#[test]
fn logs_view_flag_toggles_flip_each_flag_independently() {
    let mut state = logs();
    state.toggle_timestamps();
    state.toggle_previous();
    state.toggle_wrap();
    assert!(state.timestamps && state.previous && state.wrap);
    state.toggle_timestamps();
    assert!(!state.timestamps && state.previous && state.wrap);
}

#[test]
fn logs_view_search_finds_matches_and_cycles_through_them() {
    let mut state = logs();
    state.push_line("boot ok".to_string());
    state.push_line("ERROR one".to_string());
    state.push_line("fine".to_string());
    state.push_line("error two".to_string());

    state.set_search_query("error");
    assert_eq!(state.search_matches, vec![1, 3]);
    assert_eq!(state.current_match_idx, Some(0));
    assert_eq!(state.scroll_offset, 1);
    assert!(!state.follow);

    state.next_match();
    assert_eq!(state.current_match_idx, Some(1));
    assert_eq!(state.scroll_offset, 3);
    state.next_match();
    assert_eq!(state.current_match_idx, Some(0), "next wraps around");

    state.prev_match();
    assert_eq!(
        state.current_match_idx,
        Some(1),
        "prev from first wraps to last"
    );
    state.prev_match();
    assert_eq!(state.current_match_idx, Some(0));

    state.clear_search();
    assert!(state.search_query.is_empty());
    assert!(state.search_matches.is_empty());
    assert!(state.current_match_idx.is_none());
}

#[test]
fn logs_view_search_with_no_hits_or_empty_query_clears_the_cursor() {
    let mut state = logs();
    state.push_line("only line".to_string());
    state.set_search_query("nomatch");
    assert!(state.search_matches.is_empty());
    assert!(state.current_match_idx.is_none());
    // next/prev on no matches are no-ops.
    state.next_match();
    state.prev_match();
    assert!(state.current_match_idx.is_none());

    state.set_search_query("line");
    assert_eq!(state.current_match_idx, Some(0));
    state.set_search_query("");
    assert!(state.search_matches.is_empty());
    assert!(state.current_match_idx.is_none());

    // With no current index, next/prev start from the ends.
    state.set_search_query("line");
    state.current_match_idx = None;
    state.next_match();
    assert_eq!(state.current_match_idx, Some(0));
    state.current_match_idx = None;
    state.prev_match();
    assert_eq!(state.current_match_idx, Some(0));
}

#[test]
fn logs_view_save_to_file_writes_every_line_to_the_temp_dir() {
    let mut state = logs();
    state.push_line("first".to_string());
    state.push_line("second".to_string());
    let path = state.save_to_file().expect("saved");
    let contents = std::fs::read_to_string(&path).expect("readable");
    assert_eq!(contents, "first\nsecond");
    assert!(path.contains("web-0-"));
    assert!(path.ends_with("-logs.txt"));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn logs_view_renders_a_waiting_placeholder_when_there_are_no_lines() {
    let state = logs();
    let text = common::render_text(120, 40, |f| render_logs_view(f, f.area(), &state));
    assert!(
        text.contains("Logs: web-0 (default/app) [Ftpw] [1/0 lines]"),
        "{text}"
    );
    assert!(text.contains("Waiting for logs..."));
}

#[test]
fn logs_view_renders_numbered_lines_and_the_flag_letters_when_following() {
    let mut state =
        LogsViewState::new("api".to_string(), "prod".to_string(), None, "c".to_string());
    for i in 1..=3 {
        state.push_line(format!("message {i}"));
    }
    state.toggle_timestamps();
    state.toggle_previous();
    state.toggle_wrap();
    let text = common::render_text(120, 40, |f| render_logs_view(f, f.area(), &state));
    assert!(
        text.contains("Logs: api (prod/all) [FTPW] [3/3 lines]"),
        "{text}"
    );
    assert!(text.contains("    1 │ message 1"));
    assert!(text.contains("    3 │ message 3"));
}

#[test]
fn logs_view_shows_the_tail_while_following_and_the_offset_when_not() {
    let mut state = logs();
    for i in 1..=50 {
        state.push_line(format!("line {i}"));
    }
    // 10 rows tall: 8 inner rows -> the last 8 lines while following.
    let text = common::render_text(60, 10, |f| render_logs_view(f, f.area(), &state));
    assert!(text.contains("   50 │ line 50"), "{text}");
    assert!(!text.contains("   42 │ line 42"), "{text}");
    assert!(text.contains("   43 │ line 43"), "{text}");

    state.scroll_top();
    state.scroll_down(4);
    let text = common::render_text(60, 10, |f| render_logs_view(f, f.area(), &state));
    assert!(text.contains("[5/50 lines]"), "{text}");
    assert!(text.contains("    5 │ line 5"), "{text}");
    assert!(!text.contains("    4 │ line 4"), "{text}");
}

#[test]
fn logs_view_colours_lines_by_severity_and_highlights_search_hits() {
    let mut state = logs();
    state.push_line("something FATAL happened".to_string());
    state.push_line("a warning here".to_string());
    state.push_line("info: started".to_string());
    state.push_line("debug: details".to_string());
    state.push_line("plain".to_string());
    state.scroll_top();

    let buf = render_buffer(80, 12, |f| render_logs_view(f, f.area(), &state));
    let red = Theme::RED;
    let yellow = Theme::YELLOW;
    let dim = Theme::DIM;
    assert_eq!(fg_of(&buf, 1, "something"), Some(red));
    assert_eq!(fg_of(&buf, 2, "a warning"), Some(yellow));
    assert_eq!(fg_of(&buf, 3, "info:"), Some(Theme::FG));
    assert_eq!(fg_of(&buf, 4, "debug:"), Some(dim));
    assert_eq!(fg_of(&buf, 5, "plain"), Some(Theme::FG));

    state.set_search_query("WARN");
    let buf = render_buffer(140, 12, |f| render_logs_view(f, f.area(), &state));
    let title = buffer_row(&buf, 0);
    assert!(
        title.contains("[Search: \"WARN\" (1/1 matches, n/N)]"),
        "{title}"
    );
    // The viewport now starts at the match; the hit itself is drawn on the
    // yellow match background, the rest of the line in the base colour.
    let row = buffer_row(&buf, 1);
    assert!(row.contains("a warning here"), "{row}");
    let x = col_of(&row, "warn");
    assert_eq!(
        buf.cell(Position::new(x, 1)).unwrap().style().bg,
        Some(yellow)
    );
    let x_a = col_of(&row, "a warning");
    assert_ne!(
        buf.cell(Position::new(x_a, 1)).unwrap().style().bg,
        Some(yellow)
    );
}

#[test]
fn logs_view_title_reports_zero_matches_and_wraps_long_lines_when_asked() {
    let mut state = logs();
    state.push_line("x".repeat(100));
    state.set_search_query("nothing");
    let lines = common::render_lines(60, 20, |f| render_logs_view(f, f.area(), &state));
    // The title is clipped to the width; the badge is still part of it.
    let title_text = common::render_text(200, 20, |f| render_logs_view(f, f.area(), &state));
    assert!(
        title_text.contains("[Search: \"nothing\" (0 matches)]"),
        "{title_text}"
    );
    // Without wrap the 100-char line is cut at the border.
    assert!(lines[1].starts_with("│    1 │ xxxx"), "{}", lines[1]);
    assert!(
        lines[2].trim_matches(|c| c == '│' || c == ' ').is_empty(),
        "{}",
        lines[2]
    );

    state.toggle_wrap();
    let wrapped = common::render_lines(60, 20, |f| render_logs_view(f, f.area(), &state));
    assert!(
        wrapped[2].contains("xxxx"),
        "wrapped continuation: {}",
        wrapped[2]
    );
}

// ---------------------------------------------------------------------------
// describe_view
// ---------------------------------------------------------------------------

const DESCRIBE: &str = "Name:         web-0\nNamespace:    default\nLabels:       app=web\n\nContainers:\n  app:\n    Image:  nginx\nplain trailing line\nEvents:       <none>";

fn describe() -> DescribeViewState {
    DescribeViewState::new(
        "web-0".to_string(),
        "Pod".to_string(),
        Some("default".to_string()),
        DESCRIBE.to_string(),
    )
}

#[test]
fn describe_view_splits_content_into_lines_at_construction() {
    let state = describe();
    assert_eq!(state.lines.len(), 9);
    assert_eq!(state.lines[0], "Name:         web-0");
    assert_eq!(state.content, DESCRIBE);
    assert_eq!(state.scroll_offset, 0);
}

#[test]
fn describe_view_search_cycles_and_clears() {
    let mut state = describe();
    state.set_search_query("APP");
    assert_eq!(state.search_matches, vec![2, 5]);
    assert_eq!(state.current_match_idx, Some(0));
    assert_eq!(state.scroll_offset, 2);

    state.next_match();
    assert_eq!(state.scroll_offset, 5);
    state.next_match();
    assert_eq!(state.current_match_idx, Some(0));
    state.prev_match();
    assert_eq!(state.current_match_idx, Some(1));
    state.prev_match();
    assert_eq!(state.current_match_idx, Some(0));

    state.set_search_query("zzz");
    assert!(state.search_matches.is_empty());
    assert!(state.current_match_idx.is_none());
    state.next_match();
    state.prev_match();
    assert!(state.current_match_idx.is_none());

    state.set_search_query("");
    assert!(state.search_matches.is_empty());

    state.set_search_query("name");
    state.clear_search();
    assert!(state.search_query.is_empty() && state.search_matches.is_empty());
    assert!(state.current_match_idx.is_none());
}

#[test]
fn describe_view_scrolling_is_clamped_to_the_content() {
    let mut state = describe();
    state.scroll_down(3);
    assert_eq!(state.scroll_offset, 3);
    state.scroll_down(100);
    assert_eq!(state.scroll_offset, 3, "overshoot is ignored");
    state.scroll_up(1);
    assert_eq!(state.scroll_offset, 2);
    state.scroll_up(50);
    assert_eq!(state.scroll_offset, 0);
    state.scroll_bottom();
    assert_eq!(state.scroll_offset, 8);
    state.scroll_top();
    assert_eq!(state.scroll_offset, 0);

    let mut empty = DescribeViewState::new("x".into(), "Pod".into(), None, String::new());
    empty.scroll_bottom();
    assert_eq!(empty.scroll_offset, 0);
}

#[test]
fn describe_view_renders_title_with_namespace_and_styled_key_value_lines() {
    let state = describe();
    let buf = render_buffer(120, 40, |f| render_describe_view(f, f.area(), &state));
    let title = buffer_row(&buf, 0);
    assert!(
        title
            .contains("Describe: Pod/web-0 (default) (Line 1/9) [c: Copy] [/: Search] [Esc: Back]"),
        "{title}"
    );
    // "Name:" is the cyan key, the value is the base colour.
    assert_eq!(fg_of(&buf, 1, "Name:"), Some(Theme::CYAN));
    assert_eq!(fg_of(&buf, 1, "web-0"), Some(Theme::FG));
    // A line without a colon is drawn plain.
    assert_eq!(fg_of(&buf, 8, "plain trailing"), Some(Theme::FG));
    assert!(buffer_row(&buf, 9).contains("Events:       <none>"));
}

#[test]
fn describe_view_renders_without_namespace_and_from_the_scroll_offset() {
    let mut state = DescribeViewState::new(
        "node-a".into(),
        "Node".into(),
        None,
        (1..=30)
            .map(|i| format!("k{i}: v{i}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    state.scroll_down(10);
    let lines = common::render_lines(60, 20, |f| render_describe_view(f, f.area(), &state));
    assert!(
        lines[0].contains("Describe: Node/node-a  (Line 11/30)"),
        "{}",
        lines[0]
    );
    assert!(lines[1].contains("k11: v11"), "{}", lines[1]);
    assert!(!lines.iter().any(|l| l.contains("k10: v10")));
}

#[test]
fn describe_view_shows_the_search_badge_and_highlights_the_hit() {
    let mut state = describe();
    state.set_search_query("nginx");
    let buf = render_buffer(120, 40, |f| render_describe_view(f, f.area(), &state));
    let title = buffer_row(&buf, 0);
    assert!(
        title.contains("[Search: \"nginx\" (1/1 matches, n/N)]"),
        "{title}"
    );
    // Scrolled to the match: first body row is the Image line.
    let row = buffer_row(&buf, 1);
    assert!(row.contains("Image:  nginx"), "{row}");
    let x = col_of(&row, "nginx");
    assert_eq!(
        buf.cell(Position::new(x, 1)).unwrap().style().bg,
        Some(Theme::YELLOW)
    );
    assert_ne!(
        buf.cell(Position::new(x - 1, 1)).unwrap().style().bg,
        Some(Theme::YELLOW)
    );

    state.set_search_query("absent");
    let text = common::render_text(120, 40, |f| render_describe_view(f, f.area(), &state));
    assert!(text.contains("[Search: \"absent\" (0 matches)]"), "{text}");
}

// ---------------------------------------------------------------------------
// tree_view
// ---------------------------------------------------------------------------

fn lineage() -> LineageNode {
    let mut deploy = LineageNode::new(
        "Deployment",
        "web",
        Some("prod".into()),
        LineageRelation::Owner,
    );
    deploy.status = Some("3/3 ready".into());
    let mut rs = LineageNode::new(
        "ReplicaSet",
        "web-abc",
        Some("prod".into()),
        LineageRelation::Owner,
    );
    let mut pod = LineageNode::new(
        "Pod",
        "web-abc-1",
        Some("prod".into()),
        LineageRelation::Target,
    );
    pod.status = Some("Running".into());
    pod.details = Some("node-1".into());
    let mut ctr = LineageNode::new("Container", "app", None, LineageRelation::Child);
    ctr.status = Some("CrashLoopBackOff".into());
    let mut cm = LineageNode::new(
        "ConfigMap",
        "web-cfg",
        Some("prod".into()),
        LineageRelation::Config,
    );
    cm.status = Some("Pending".into());
    let svc = LineageNode::new(
        "Service",
        "web-svc",
        Some("prod".into()),
        LineageRelation::Service,
    );
    let ing = LineageNode::new(
        "Ingress",
        "web-ing",
        Some("prod".into()),
        LineageRelation::Ingress,
    );
    let sec = LineageNode::new(
        "Secret",
        "web-tls",
        Some("prod".into()),
        LineageRelation::Secret,
    );
    let pvc = LineageNode::new(
        "PersistentVolumeClaim",
        "data",
        Some("prod".into()),
        LineageRelation::Storage,
    );
    let node = LineageNode::new("Node", "node-1", None, LineageRelation::Node);
    let other = LineageNode::new("Widget", "w", None, LineageRelation::Child);
    pod.children = vec![ctr, cm, sec, pvc, node, other];
    rs.children.push(pod);
    deploy.children = vec![rs, svc, ing];
    deploy
}

#[test]
fn tree_view_flattens_with_branch_glyphs_and_selects_the_target() {
    let mut state = TreeViewState::new("Pod".into(), "web-abc-1".into(), Some("prod".into()));
    assert!(state.is_loading);
    state.set_tree(lineage());
    assert!(!state.is_loading);
    assert!(state.error.is_none());
    assert_eq!(state.nodes.len(), 11);
    assert_eq!(state.nodes[0].prefix, "");
    assert_eq!(state.nodes[1].prefix, "├── ");
    assert_eq!(state.nodes[2].prefix, "│   └── ");
    assert_eq!(state.nodes[3].prefix, "│       ├── ");
    assert_eq!(state.nodes[8].prefix, "│       └── ");
    assert_eq!(state.nodes[9].prefix, "├── ");
    assert_eq!(state.nodes[10].prefix, "└── ");
    assert_eq!(state.selected_idx, 2);
    assert_eq!(state.selected_node().unwrap().name, "web-abc-1");
    assert_eq!(state.nodes[2].depth, 2);
    assert!(state.nodes[2].is_last_child);
}

#[test]
fn tree_view_navigation_clamps_at_both_ends() {
    let mut state = TreeViewState::new("Pod".into(), "web-abc-1".into(), None);
    state.set_tree(lineage());
    state.select_first();
    assert_eq!(state.selected_idx, 0);
    state.select_prev();
    assert_eq!(state.selected_idx, 0);
    state.select_last();
    assert_eq!(state.selected_idx, 10);
    state.select_next();
    assert_eq!(state.selected_idx, 10);
    state.select_prev();
    state.select_next();
    assert_eq!(state.selected_idx, 10);

    let mut empty = TreeViewState::new("Pod".into(), "x".into(), None);
    empty.select_last();
    empty.select_next();
    assert_eq!(empty.selected_idx, 0);
    assert!(empty.selected_node().is_none());
}

#[test]
fn tree_view_tree_as_text_lists_every_node_with_badge_and_status() {
    let mut state = TreeViewState::new("Pod".into(), "web-abc-1".into(), None);
    state.set_tree(lineage());
    let text = state.tree_as_text();
    assert!(text.starts_with("Resource Relationship Tree for Pod/web-abc-1\n"));
    assert!(text.contains(&"=".repeat(50)));
    assert!(text.contains("Deployment/web [OWNER] [status: 3/3 ready] \n"));
    assert!(text.contains("│   └── Pod/web-abc-1 [TARGET] [status: Running] node-1\n"));
    assert!(text.contains("├── Service/web-svc [SERVICE] [status: -] \n"));
}

#[test]
fn tree_view_renders_loading_error_and_empty_branches() {
    let mut state = TreeViewState::new("Pod".into(), "web-0".into(), Some("ns".into()));
    let text = common::render_text(120, 40, |f| render_tree_view(f, f.area(), &state));
    assert!(
        text.contains("Resource Relationship Tree: Pod/web-0 ns"),
        "{text}"
    );
    assert!(text.contains("Resolving resource lineage..."));
    assert!(text.contains("linked resources for Pod/web-0..."));

    state.set_error("forbidden: pods is forbidden".into());
    assert!(!state.is_loading);
    let text = common::render_text(120, 40, |f| render_tree_view(f, f.area(), &state));
    assert!(
        text.contains("Failed to resolve lineage: forbidden: pods is forbidden"),
        "{text}"
    );

    state.error = None;
    let text = common::render_text(120, 40, |f| render_tree_view(f, f.area(), &state));
    assert!(
        text.contains("No relationships or lineage found for this resource."),
        "{text}"
    );
}

#[test]
fn tree_view_renders_every_node_kind_with_its_badges_and_status_colour() {
    let mut state = TreeViewState::new("Pod".into(), "web-abc-1".into(), Some("prod".into()));
    state.set_tree(lineage());
    let buf = render_buffer(120, 40, |f| render_tree_view(f, f.area(), &state));
    let rows: Vec<String> = (0..buf.area.height).map(|y| buffer_row(&buf, y)).collect();

    let (y, deploy) = row_containing(&rows, "Deployment/web [OWNER]").unwrap();
    assert!(deploy.contains("[● 3/3 ready]"), "{deploy}");
    assert_eq!(fg_of(&buf, y as u16, "[● 3/3"), Theme::status_ok().fg);

    let (y, pod) = row_containing(&rows, "Pod/web-abc-1 [TARGET]").unwrap();
    assert!(pod.contains("[● Running] (node-1)"), "{pod}");
    // Selected row: the name is drawn in the selection foreground.
    assert_eq!(fg_of(&buf, y as u16, "web-abc-1"), Some(Theme::SEL_FG));

    let (y, ctr) = row_containing(&rows, "Container/app [CHILD]").unwrap();
    assert!(ctr.contains("[● CrashLoopBackOff]"), "{ctr}");
    assert_eq!(fg_of(&buf, y as u16, "[● Crash"), Theme::status_error().fg);

    let (y, cm) = row_containing(&rows, "ConfigMap/web-cfg [CONFIG]").unwrap();
    assert!(cm.contains("[● Pending]"), "{cm}");
    assert_eq!(fg_of(&buf, y as u16, "[● Pending"), Theme::status_warn().fg);

    assert!(row_containing(&rows, "Secret/web-tls [SECRET]").is_some());
    assert!(row_containing(&rows, "PersistentVolumeClaim/data [STORAGE]").is_some());
    assert!(row_containing(&rows, "Node/node-1 [NODE]").is_some());
    assert!(row_containing(&rows, "Widget/w [CHILD]").is_some());
    assert!(row_containing(&rows, "Service/web-svc [SERVICE]").is_some());
    assert!(row_containing(&rows, "Ingress/web-ing [INGRESS]").is_some());
}

#[test]
fn tree_view_scrolls_so_a_far_selection_stays_visible_on_a_short_screen() {
    let mut root = LineageNode::new("Deployment", "big", None, LineageRelation::Owner);
    for i in 0..40 {
        root.children.push(LineageNode::new(
            "Pod",
            format!("pod-{i:02}"),
            None,
            LineageRelation::Child,
        ));
    }
    let mut state = TreeViewState::new("Deployment".into(), "big".into(), None);
    state.set_tree(root);
    state.select_last();
    // 20 rows -> 18 inner rows; selection 40 -> start at 40 - 9 = 31.
    let lines = common::render_lines(60, 20, |f| render_tree_view(f, f.area(), &state));
    assert!(lines.iter().any(|l| l.contains("Pod/pod-39")), "{lines:?}");
    assert!(lines.iter().any(|l| l.contains("Pod/pod-30")), "{lines:?}");
    assert!(!lines.iter().any(|l| l.contains("Pod/pod-29")), "{lines:?}");
    // The root is scrolled out of the body (it still appears in the title).
    assert!(
        !lines[1..].iter().any(|l| l.contains("Deployment/big")),
        "{lines:?}"
    );
}

// ---------------------------------------------------------------------------
// toolbox_view (hand-built state: the constructor probes the machine)
// ---------------------------------------------------------------------------

fn tool(
    name: &str,
    installed: bool,
    required: bool,
    version: Option<&str>,
    path: Option<&str>,
) -> ToolStatusItem {
    ToolStatusItem {
        name: name.to_string(),
        installed,
        version: version.map(str::to_string),
        path: path.map(str::to_string),
        required,
    }
}

fn toolbox() -> ToolboxViewState {
    ToolboxViewState {
        tools: vec![
            tool(
                "kubectl",
                true,
                true,
                Some("v1.30.2"),
                Some("/usr/local/bin/kubectl"),
            ),
            tool("helm", false, true, None, None),
            tool("krew", false, false, None, None),
        ],
        selected_idx: 0,
    }
}

#[test]
fn toolbox_view_selection_moves_within_the_tool_list() {
    let mut state = toolbox();
    state.select_prev();
    assert_eq!(state.selected_idx, 0);
    state.select_next();
    state.select_next();
    assert_eq!(state.selected_idx, 2);
    state.select_next();
    assert_eq!(state.selected_idx, 2);
    state.select_prev();
    assert_eq!(state.selected_idx, 1);

    let mut empty = ToolboxViewState {
        tools: vec![],
        selected_idx: 0,
    };
    empty.select_next();
    assert_eq!(empty.selected_idx, 0);
}

#[test]
fn toolbox_view_renders_one_status_per_tool_with_the_right_colour() {
    let state = toolbox();
    let buf = render_buffer(120, 20, |f| render_toolbox_view(f, f.area(), &state));
    let rows: Vec<String> = (0..buf.area.height).map(|y| buffer_row(&buf, y)).collect();
    assert!(
        rows[0].contains("SRElens Toolbox & CLI Environment Diagnostics (<Esc> Back)"),
        "{}",
        rows[0]
    );
    assert!(
        rows[1].contains("TOOL")
            && rows[1].contains("STATUS")
            && rows[1].contains("VERSION")
            && rows[1].contains("PATH")
    );

    let (y, kubectl) = row_containing(&rows, "kubectl").unwrap();
    assert!(kubectl.contains("● INSTALLED"), "{kubectl}");
    assert!(
        kubectl.contains("v1.30.2") && kubectl.contains("/usr/local/bin/kubectl"),
        "{kubectl}"
    );
    assert_eq!(fg_of(&buf, y as u16, "● INSTALLED"), Theme::status_ok().fg);

    let (y, helm) = row_containing(&rows, "helm").unwrap();
    assert!(helm.contains("● MISSING (REQUIRED)"), "{helm}");
    assert_eq!(fg_of(&buf, y as u16, "● MISSING"), Theme::status_error().fg);

    let (y, krew) = row_containing(&rows, "krew").unwrap();
    assert!(krew.contains("○ NOT INSTALLED"), "{krew}");
    assert_eq!(fg_of(&buf, y as u16, "○ NOT"), Theme::status_warn().fg);
    // Missing version and path render as a dash.
    assert!(
        krew.matches(" - ").count() >= 1 || krew.ends_with('-') || krew.contains("-  "),
        "{krew}"
    );
}

#[test]
fn toolbox_view_highlights_the_selected_row_even_on_a_narrow_screen() {
    let mut state = toolbox();
    state.select_next();
    let buf = render_buffer(60, 20, |f| render_toolbox_view(f, f.area(), &state));
    let rows: Vec<String> = (0..buf.area.height).map(|y| buffer_row(&buf, y)).collect();
    let (y, _) = row_containing(&rows, "helm").unwrap();
    assert_eq!(
        buf.cell(Position::new(2, y as u16)).unwrap().style().bg,
        Theme::selected_row().bg
    );
    let (y0, _) = row_containing(&rows, "kubectl").unwrap();
    assert_ne!(
        buf.cell(Position::new(2, y0 as u16)).unwrap().style().bg,
        Theme::selected_row().bg
    );
}

// ---------------------------------------------------------------------------
// port_forward_view
// ---------------------------------------------------------------------------

fn forward(id: &str, status: &str, local: u16, rx: u64, tx: u64) -> PortForwardEntry {
    PortForwardEntry {
        id: id.to_string(),
        context: "prod".to_string(),
        namespace: "default".to_string(),
        target_type: "pod".to_string(),
        target_name: format!("web-{id}"),
        local_port: local,
        container_port: 8080,
        active_connections: 2,
        bytes_rx: rx,
        bytes_tx: tx,
        status: status.to_string(),
    }
}

#[test]
fn port_forward_view_set_forwards_clamps_the_selection() {
    let mut state = PortForwardViewState::new();
    assert!(state.forwards.is_empty());
    assert!(state.selected_forward().is_none());

    state.set_forwards(vec![
        forward("a", "active", 8080, 0, 0),
        forward("b", "active", 8081, 0, 0),
        forward("c", "error", 8082, 0, 0),
    ]);
    state.select_next();
    state.select_next();
    state.select_next();
    assert_eq!(state.selected_idx, 2);
    assert_eq!(state.selected_forward().unwrap().id, "c");

    state.set_forwards(vec![forward("a", "active", 8080, 0, 0)]);
    assert_eq!(state.selected_idx, 0);
    state.select_prev();
    assert_eq!(state.selected_idx, 0);

    state.set_forwards(vec![]);
    assert_eq!(state.selected_idx, 0);
    state.select_next();
    assert_eq!(state.selected_idx, 0);
}

#[test]
fn port_forward_view_renders_the_empty_placeholder() {
    let state = PortForwardViewState::new();
    let text = common::render_text(120, 20, |f| render_port_forward_view(f, f.area(), &state));
    assert!(
        text.contains(
            "Active Port Forwards [0] (<d> Stop Forward <shift-f> New Forward <Esc> Back)"
        ),
        "{text}"
    );
    assert!(
        text.contains("No active port forwards. Select a Pod or Service and press <shift-f>"),
        "{text}"
    );
}

#[test]
fn port_forward_view_renders_rows_with_human_byte_counts_and_status_colours() {
    let mut state = PortForwardViewState::new();
    state.set_forwards(vec![
        forward("a", "active", 8080, 512, 1536),
        forward("b", "running", 9090, 3 * 1024 * 1024, 0),
        forward("c", "error", 7070, 1024, 2048),
    ]);
    state.select_next();
    let buf = render_buffer(140, 20, |f| render_port_forward_view(f, f.area(), &state));
    let rows: Vec<String> = (0..buf.area.height).map(|y| buffer_row(&buf, y)).collect();
    assert!(rows[0].contains("Active Port Forwards [3]"), "{}", rows[0]);
    assert!(
        rows[1].contains("STATUS") && rows[1].contains("LOCAL PORT") && rows[1].contains("RX / TX"),
        "{}",
        rows[1]
    );

    let (y, a) = row_containing(&rows, "127.0.0.1:8080").unwrap();
    assert!(
        a.contains("● active")
            && a.contains("pod/web-a")
            && a.contains("8080")
            && a.contains("512B / 1.5KB"),
        "{a}"
    );
    assert_eq!(fg_of(&buf, y as u16, "● active"), Theme::status_ok().fg);

    let (y, b) = row_containing(&rows, "127.0.0.1:9090").unwrap();
    assert!(b.contains("3.0MB / 0B"), "{b}");
    assert_eq!(fg_of(&buf, y as u16, "● running"), Theme::status_ok().fg);
    assert_eq!(
        buf.cell(Position::new(2, y as u16)).unwrap().style().bg,
        Theme::selected_row().bg
    );

    let (y, c) = row_containing(&rows, "127.0.0.1:7070").unwrap();
    assert!(c.contains("1.0KB / 2.0KB"), "{c}");
    assert_eq!(fg_of(&buf, y as u16, "● error"), Theme::status_warn().fg);
}

/// At a narrow width this table is over-constrained — its seven columns ask
/// for 115 cells and cannot all fit — and WHICH columns survive is not
/// stable between runs (see the note in the PR: cassowary resolves a
/// degenerate system by an order that Rust's per-process hash seed varies).
/// So this asserts only what a narrow render is actually guaranteed to
/// produce: the block, its count, and the flexible TARGET column, which is
/// the one `Constraint::Min` and therefore always allotted. Anything about
/// STATUS or the local address at this width would be a coin flip.
#[test]
fn port_forward_view_at_a_narrow_width_keeps_its_block_and_flexible_column() {
    let mut state = PortForwardViewState::new();
    state.set_forwards(vec![forward("a", "active", 8080, 0, 0)]);
    let text = common::render_text(60, 20, |f| render_port_forward_view(f, f.area(), &state));
    assert!(text.contains("Active Port Forwards [1]"), "{text}");
    assert!(text.contains("TARGET"), "{text}");
    assert!(text.contains("pod/web-a"), "{text}");
}

/// The same view given room for every column: here the layout is determinate,
/// so the status and the local address can be asserted.
#[test]
fn port_forward_view_shows_status_and_local_port_once_every_column_fits() {
    let mut state = PortForwardViewState::new();
    state.set_forwards(vec![forward("a", "active", 8080, 0, 0)]);
    let text = common::render_text(130, 20, |f| render_port_forward_view(f, f.area(), &state));
    assert!(text.contains("● active"), "{text}");
    assert!(text.contains("127.0.0.1:8080"), "{text}");
    assert!(text.contains("pod/web-a"), "{text}");
}

// ---------------------------------------------------------------------------
// helm_view
// ---------------------------------------------------------------------------

fn release(name: &str, status: &str, revision: i64) -> HelmReleaseItem {
    HelmReleaseItem {
        name: name.to_string(),
        namespace: "platform".to_string(),
        revision,
        status: status.to_string(),
        chart: format!("{name}-1.2.3"),
        chart_version: "1.2.3".to_string(),
        app_version: "2.0".to_string(),
        updated: "2026-09-01 10:00".to_string(),
    }
}

#[test]
fn helm_view_set_releases_clamps_the_selection() {
    let mut state = HelmViewState::new();
    assert!(state.selected_release().is_none());
    state.set_releases(vec![
        release("a", "deployed", 1),
        release("b", "failed", 2),
        release("c", "pending-install", 3),
    ]);
    state.select_next();
    state.select_next();
    state.select_next();
    assert_eq!(state.selected_release().unwrap().name, "c");
    state.set_releases(vec![release("a", "deployed", 1)]);
    assert_eq!(state.selected_idx, 0);
    state.select_prev();
    assert_eq!(state.selected_idx, 0);
    state.set_releases(vec![]);
    state.select_next();
    assert_eq!(state.selected_idx, 0);
}

#[test]
fn helm_view_renders_the_empty_placeholder() {
    let mut state = HelmViewState::new();
    state.set_releases(vec![]);
    let text = common::render_text(160, 20, |f| render_helm_view(f, f.area(), &state));
    assert!(
        text.contains("Helm 3 Releases [0]") && text.contains("Uninstall") && text.contains("Back"),
        "{text}"
    );
    assert!(
        text.contains("No Helm releases found in current namespace."),
        "{text}"
    );
}

#[test]
fn helm_view_renders_rows_and_colours_status_by_outcome() {
    let mut state = HelmViewState::new();
    state.set_releases(vec![
        release("ingress", "deployed", 4),
        release("db", "failed", 2),
        release("cache", "superseded", 7),
    ]);
    state.select_next();
    let buf = render_buffer(140, 20, |f| render_helm_view(f, f.area(), &state));
    let rows: Vec<String> = (0..buf.area.height).map(|y| buffer_row(&buf, y)).collect();
    assert!(rows[0].contains("Helm 3 Releases [3]"), "{}", rows[0]);
    assert!(
        rows[1].contains("NAMESPACE")
            && rows[1].contains("REVISION")
            && rows[1].contains("APP VERSION")
            && rows[1].contains("UPDATED")
    );

    let (y, ingress) = row_containing(&rows, "ingress-1.2.3").unwrap();
    assert!(
        ingress.contains("platform")
            && ingress.contains("deployed")
            && ingress.contains("2026-09-01 10:00"),
        "{ingress}"
    );
    assert_eq!(fg_of(&buf, y as u16, "deployed"), Theme::status_ok().fg);

    let (y, db) = row_containing(&rows, "db-1.2.3").unwrap();
    assert!(db.contains("failed") && db.contains(" 2 "), "{db}");
    assert_eq!(fg_of(&buf, y as u16, "failed"), Theme::status_error().fg);
    assert_eq!(
        buf.cell(Position::new(2, y as u16)).unwrap().style().bg,
        Theme::selected_row().bg
    );

    let (y, cache) = row_containing(&rows, "cache-1.2.3").unwrap();
    assert!(cache.contains("superseded"), "{cache}");
    assert_eq!(fg_of(&buf, y as u16, "superseded"), Theme::status_warn().fg);
}

#[test]
fn helm_view_narrow_screen_keeps_namespace_and_name() {
    let mut state = HelmViewState::new();
    state.set_releases(vec![release("ingress", "deployed", 4)]);
    let text = common::render_text(60, 20, |f| render_helm_view(f, f.area(), &state));
    assert!(text.contains("platform"), "{text}");
    assert!(text.contains("ingress"), "{text}");
}

// ---------------------------------------------------------------------------
// reason_rail
// ---------------------------------------------------------------------------

fn tallies() -> Vec<ReasonTally> {
    vec![
        ReasonTally {
            reason: "BackOff".into(),
            count: 5,
            event_type: "Warning".into(),
        },
        ReasonTally {
            reason: "Scheduled".into(),
            count: 3,
            event_type: "Normal".into(),
        },
        ReasonTally {
            reason: "Pulled".into(),
            count: 1,
            event_type: "Normal".into(),
        },
    ]
}

#[test]
fn tally_event_reasons_defaults_missing_fields_and_skips_blank_reasons() {
    let events = vec![
        json!({ "type": "Warning" }),
        json!({ "reason": "   " }),
        json!({ "reason": " Pulled " }),
        json!({ "reason": "Pulled", "type": "Normal" }),
        json!({ "reason": "Killing", "type": "Warning" }),
    ];
    let t = tally_event_reasons(&events);
    assert_eq!(t.len(), 3);
    assert_eq!(t[0].reason, "Pulled");
    assert_eq!(t[0].count, 2);
    assert_eq!(
        t[0].event_type, "Normal",
        "type comes from the first sighting"
    );
    assert_eq!(t[1].reason, "Unknown");
    assert_eq!(t[1].event_type, "Warning");
    assert_eq!(t[2].reason, "Killing");
}

#[test]
fn reason_rail_widget_shows_the_empty_placeholder_and_the_focus_dependent_title() {
    let text = common::render_text(40, 8, |f| {
        render_reason_rail_widget(f, f.area(), &[], 0, false, None)
    });
    assert!(text.contains("Event Reasons [Tab/<R> Focus]"), "{text}");
    assert!(text.contains("No events in scope"), "{text}");

    let text = common::render_text(60, 8, |f| {
        render_reason_rail_widget(f, f.area(), &[], 0, true, None)
    });
    assert!(
        text.contains("Event Reasons [↑/↓ Navigate, Enter Pick, Esc Back]"),
        "{text}"
    );
}

#[test]
fn reason_rail_widget_marks_the_cursor_the_active_filter_and_warning_dots() {
    let t = tallies();
    let buf = render_buffer(40, 8, |f| {
        render_reason_rail_widget(f, f.area(), &t, 1, true, Some("Pulled"))
    });
    let rows: Vec<String> = (0..buf.area.height).map(|y| buffer_row(&buf, y)).collect();
    assert!(rows[1].contains("  ● BackOff (5)"), "{}", rows[1]);
    assert_eq!(fg_of(&buf, 1, "BackOff"), Some(Theme::YELLOW));
    assert!(rows[2].contains("▸ ○ Scheduled (3)"), "{}", rows[2]);
    assert_eq!(fg_of(&buf, 2, "Scheduled"), Some(Theme::SEL_FG));
    assert_eq!(
        buf.cell(Position::new(1, 2)).unwrap().style().bg,
        Theme::selected_row().bg
    );
    assert!(rows[3].contains("  ○ Pulled (1) ✔"), "{}", rows[3]);
    assert_eq!(fg_of(&buf, 3, "Pulled"), Some(Theme::CYAN));

    // Unfocused: no cursor is drawn even for the selected index.
    let text = common::render_text(40, 8, |f| {
        render_reason_rail_widget(f, f.area(), &t, 1, false, None)
    });
    assert!(!text.contains("▸"), "{text}");
    assert!(text.contains("○ Scheduled (3)"), "{text}");
}

#[test]
fn reason_rail_widget_scrolls_to_keep_a_far_selection_visible() {
    let many: Vec<ReasonTally> = (0..30)
        .map(|i| ReasonTally {
            reason: format!("Reason{i:02}"),
            count: 30 - i,
            event_type: "Normal".into(),
        })
        .collect();
    // 8 rows -> 6 visible; selection 20 -> offset 17.
    let lines = common::render_lines(40, 8, |f| {
        render_reason_rail_widget(f, f.area(), &many, 20, true, None)
    });
    assert!(lines.iter().any(|l| l.contains("Reason17")), "{lines:?}");
    assert!(
        lines.iter().any(|l| l.contains("▸ ○ Reason20")),
        "{lines:?}"
    );
    assert!(lines.iter().any(|l| l.contains("Reason22")), "{lines:?}");
    assert!(!lines.iter().any(|l| l.contains("Reason16")), "{lines:?}");
    assert!(!lines.iter().any(|l| l.contains("Reason23")), "{lines:?}");
}

#[test]
fn reason_rail_modal_centres_a_focused_rail_and_shrinks_on_small_screens() {
    let t = tallies();
    let lines = common::render_lines(120, 40, |f| {
        render_reason_rail_modal(f, f.area(), &t, 0, None)
    });
    // 60 wide, 20 tall, centred: rows 10..30, columns 30..90.
    assert!(lines[10].starts_with(&" ".repeat(30)), "{}", lines[10]);
    assert!(
        lines[10].contains("Event Reasons [↑/↓ Navigate"),
        "{}",
        lines[10]
    );
    assert!(lines[11].contains("▸ ● BackOff (5)"), "{}", lines[11]);
    assert!(lines[29].trim_start().starts_with('└'), "{}", lines[29]);
    assert!(lines[9].is_empty() && lines[30].is_empty());

    let lines = common::render_lines(30, 10, |f| {
        render_reason_rail_modal(f, f.area(), &t, 2, Some("Pulled"))
    });
    assert!(lines[2].starts_with("  ┌"), "{}", lines[2]);
    assert!(
        lines.iter().any(|l| l.contains("▸ ○ Pulled (1) ✔")),
        "{lines:?}"
    );
}

// ---------------------------------------------------------------------------
// cracked_lens
// ---------------------------------------------------------------------------

#[test]
fn cracked_lens_uses_the_compact_message_when_there_is_no_room_for_art() {
    let text = common::render_text(50, 8, |f| {
        render_cracked_lens(f, f.area(), "prod", "", "", 30)
    });
    assert!(text.contains("Cluster Unreachable"), "{text}");
    assert!(text.contains("(timed out after 30s)"), "{text}");
    assert!(text.contains("Context: prod"), "{text}");
    assert!(
        text.contains("Press <r> to retry or <Ctrl+x> to switch context"),
        "{text}"
    );
    assert!(!text.contains("Quick Actions"), "{text}");

    // Tall enough but too narrow is also compact: the art and diagnostics
    // are skipped. (The centred, unwrapped compact lines are wider than 26
    // columns, so ratatui draws none of them here; only the block title
    // survives. See the report on cracked_lens.rs.)
    let text = common::render_text(28, 30, |f| {
        render_cracked_lens(f, f.area(), "prod", "", "", 5)
    });
    assert!(text.contains("Cluster Unreachable"), "{text}");
    assert!(!text.contains("Quick Actions"), "{text}");
    assert!(!text.contains(".----"), "{text}");
}

#[test]
fn cracked_lens_draws_art_beside_diagnostics_on_a_wide_screen() {
    let lines = common::render_lines(140, 40, |f| {
        render_cracked_lens(
            f,
            f.area(),
            "prod-eu",
            "eu-cluster",
            "https://10.0.0.1:6443",
            45,
        )
    });
    let text = lines.join("\n");
    assert!(
        text.contains("Context:  prod-eu") || text.contains("prod-eu"),
        "{text}"
    );
    assert!(text.contains("Cluster:  eu-cluster"), "{text}");
    assert!(text.contains("Server:   https://10.0.0.1:6443"), "{text}");
    assert!(text.contains("Quick Actions:"), "{text}");
    assert!(text.contains("Retry connection"), "{text}");
    assert!(text.contains("Switch context (:ctx)"), "{text}");
    // Side by side: the art's top rim and the diagnostics share rows.
    let (art_row, _) = row_containing(&lines, ".--------------------.").unwrap();
    let (cluster_row, _) = row_containing(&lines, "Cluster:  eu-cluster").unwrap();
    assert!(
        (art_row as i32 - cluster_row as i32).abs() < 12,
        "{art_row} vs {cluster_row}"
    );
    let (_, side) = row_containing(&lines, "Cluster:  eu-cluster").unwrap();
    assert!(
        side.find("Cluster:").unwrap() > 44,
        "diagnostics are right of the art: {side}"
    );
}

#[test]
fn cracked_lens_stacks_art_above_diagnostics_on_a_narrow_screen_and_omits_blank_fields() {
    let lines = common::render_lines(60, 40, |f| {
        render_cracked_lens(f, f.area(), "dev", "", "", 12)
    });
    let text = lines.join("\n");
    assert!(text.contains("Quick Actions:"), "{text}");
    assert!(!text.contains("Cluster:"), "{text}");
    assert!(!text.contains("Server:"), "{text}");
    let (art_row, _) = row_containing(&lines, ".--------------------.").unwrap();
    let (qa_row, _) = row_containing(&lines, "Quick Actions:").unwrap();
    assert!(
        qa_row > art_row + 10,
        "diagnostics come after the art: {art_row} < {qa_row}"
    );
}

// ---------------------------------------------------------------------------
// metrics_panel_view
// ---------------------------------------------------------------------------

fn sample(ts_ms: u64, cpu: u64, mem: u64) -> MetricSample {
    MetricSample {
        timestamp_epoch_ms: ts_ms,
        cpu_millicores: cpu,
        memory_mib: mem,
    }
}

#[test]
fn metrics_panel_state_cycles_ranges_and_replaces_samples() {
    let mut state =
        MetricsPanelState::new("Pod".into(), "web-0".into(), Some("default".into()), vec![]);
    assert_eq!(state.range, MetricsTimeRange::FiveMin);
    state.cycle_time_range();
    assert_eq!(state.range, MetricsTimeRange::TenMin);
    state.cycle_time_range();
    state.cycle_time_range();
    assert_eq!(state.range, MetricsTimeRange::OneHour);
    state.cycle_time_range();
    assert_eq!(state.range, MetricsTimeRange::FiveMin);
    assert_eq!(MetricsTimeRange::ThirtyMin.label(), "30m");
    assert_eq!(MetricsTimeRange::OneHour.window_ms(), 3_600_000);

    state.update_samples(&[sample(1_000, 10, 20)]);
    assert_eq!(state.samples.len(), 1);
    assert_eq!(state.samples[0].cpu_millicores, 10);
}

#[test]
fn metrics_panel_modal_shows_the_collecting_placeholder_without_samples() {
    let state = MetricsPanelState::new("Node".into(), "node-1".into(), None, vec![]);
    let text = common::render_text(120, 40, |f| render_metrics_panel_modal(f, f.area(), &state));
    assert!(
        text.contains("Live Metrics Timeline — Node: node-1"),
        "{text}"
    );
    assert!(text.contains("Time Range:"), "{text}");
    assert!(text.contains("[1: 5m]"), "{text}");
    assert!(text.contains("(0 samples in buffer)"), "{text}");
    assert!(text.contains("Collecting metrics-server data..."), "{text}");
    assert!(!text.contains("CPU Usage:"), "{text}");
}

#[test]
fn metrics_panel_modal_summarises_only_the_samples_inside_the_window() {
    let now = 10_000_000u64;
    let samples = vec![
        // 20 minutes old: outside a 5m window, inside a 30m window.
        sample(now - 20 * 60 * 1000, 900, 900),
        sample(now - 60 * 1000, 100, 200),
        sample(now, 300, 400),
    ];
    let mut state = MetricsPanelState::new(
        "Pod".into(),
        "web-0".into(),
        Some("default".into()),
        samples,
    );
    let text = common::render_text(120, 40, |f| render_metrics_panel_modal(f, f.area(), &state));
    assert!(
        text.contains("Live Metrics Timeline — Pod: default/web-0"),
        "{text}"
    );
    assert!(text.contains("(3 samples in buffer)"), "{text}");
    assert!(
        text.contains("CPU Usage: 300m")
            && text.contains("min: 100m")
            && text.contains("avg: 200m")
            && text.contains("peak: 300m"),
        "{text}"
    );
    assert!(
        text.contains("Memory Usage: 400 MiB")
            && text.contains("min: 200 MiB")
            && text.contains("avg: 300 MiB")
            && text.contains("peak: 400 MiB"),
        "{text}"
    );

    state.cycle_time_range();
    state.cycle_time_range();
    assert_eq!(state.range, MetricsTimeRange::ThirtyMin);
    let text = common::render_text(120, 40, |f| render_metrics_panel_modal(f, f.area(), &state));
    assert!(
        text.contains("CPU Usage: 300m")
            && text.contains("min: 100m")
            && text.contains("avg: 433m")
            && text.contains("peak: 900m"),
        "{text}"
    );
    assert!(text.contains("[3: 30m]"), "{text}");
}

// ---------------------------------------------------------------------------
// resource_table: columns, segments, state mutators
// ---------------------------------------------------------------------------

fn names(kind: &ResourceKind) -> Vec<&'static str> {
    default_columns_for_kind(kind)
        .iter()
        .map(|c| c.name)
        .collect()
}

#[test]
fn default_columns_match_each_resource_kind() {
    assert_eq!(
        names(&ResourceKind::Workloads),
        [
            "NAMESPACE",
            "KIND",
            "NAME",
            "READY",
            "STATUS",
            "RESTARTS",
            "CPU",
            "MEM",
            "AGE",
            "IMAGE"
        ]
    );
    assert_eq!(
        names(&ResourceKind::Pods),
        [
            "NAMESPACE",
            "NAME",
            "READY",
            "STATUS",
            "RESTARTS",
            "CPU",
            "MEM",
            "IP",
            "NODE",
            "AGE"
        ]
    );
    assert_eq!(
        names(&ResourceKind::Deployments),
        [
            "NAMESPACE",
            "NAME",
            "READY",
            "UP-TO-DATE",
            "AVAILABLE",
            "AGE"
        ]
    );
    assert_eq!(
        names(&ResourceKind::StatefulSets),
        ["NAMESPACE", "NAME", "READY", "AGE"]
    );
    assert_eq!(
        names(&ResourceKind::DaemonSets),
        [
            "NAMESPACE",
            "NAME",
            "DESIRED",
            "CURRENT",
            "READY",
            "UP-TO-DATE",
            "AVAILABLE",
            "AGE"
        ]
    );
    assert_eq!(
        names(&ResourceKind::Jobs),
        ["NAMESPACE", "NAME", "COMPLETIONS", "DURATION", "AGE"]
    );
    assert_eq!(
        names(&ResourceKind::CronJobs),
        [
            "NAMESPACE",
            "NAME",
            "SCHEDULE",
            "SUSPEND",
            "ACTIVE",
            "LAST SCHEDULE",
            "AGE"
        ]
    );
    assert_eq!(
        names(&ResourceKind::Services),
        [
            "NAMESPACE",
            "NAME",
            "TYPE",
            "CLUSTER-IP",
            "EXTERNAL-IP",
            "PORTS",
            "AGE"
        ]
    );
    assert_eq!(
        names(&ResourceKind::Ingresses),
        [
            "NAMESPACE",
            "NAME",
            "CLASS",
            "HOSTS",
            "ADDRESS",
            "PORTS",
            "AGE"
        ]
    );
    assert_eq!(
        names(&ResourceKind::EndpointSlices),
        [
            "NAMESPACE",
            "NAME",
            "ADDRESS-TYPE",
            "PORTS",
            "ENDPOINTS",
            "AGE"
        ]
    );
    assert_eq!(
        names(&ResourceKind::ConfigMaps),
        ["NAMESPACE", "NAME", "DATA", "AGE"]
    );
    assert_eq!(
        names(&ResourceKind::Secrets),
        ["NAMESPACE", "NAME", "TYPE", "DATA", "AGE"]
    );
    assert_eq!(
        names(&ResourceKind::PersistentVolumeClaims),
        [
            "NAMESPACE",
            "NAME",
            "STATUS",
            "VOLUME",
            "CAPACITY",
            "ACCESS MODES",
            "STORAGECLASS",
            "AGE"
        ]
    );
    assert_eq!(
        names(&ResourceKind::PersistentVolumes),
        [
            "NAME",
            "CAPACITY",
            "ACCESS MODES",
            "RECLAIM POLICY",
            "STATUS",
            "CLAIM",
            "STORAGECLASS",
            "AGE"
        ]
    );
    assert_eq!(
        names(&ResourceKind::StorageClasses),
        [
            "NAME",
            "PROVISIONER",
            "RECLAIMPOLICY",
            "VOLUMEBINDINGMODE",
            "ALLOWEXPANSION",
            "AGE"
        ]
    );
    assert_eq!(
        names(&ResourceKind::Nodes),
        ["NAME", "STATUS", "ROLES", "VERSION", "CPU", "MEMORY", "PODS", "AGE"]
    );
    assert_eq!(names(&ResourceKind::Namespaces), ["NAME", "STATUS", "AGE"]);
    assert_eq!(
        names(&ResourceKind::Events),
        [
            "NAMESPACE",
            "LAST SEEN",
            "TYPE",
            "REASON",
            "OBJECT",
            "MESSAGE"
        ]
    );
    assert_eq!(
        names(&ResourceKind::CustomResourceDefinitions),
        ["NAME", "GROUP", "VERSION", "SCOPE", "AGE"]
    );
    // Kinds without a bespoke layout share the generic three-column one.
    assert_eq!(
        names(&ResourceKind::ServiceAccounts),
        ["NAMESPACE", "NAME", "AGE"]
    );
    assert_eq!(
        names(&ResourceKind::ResourceQuotas),
        ["NAMESPACE", "NAME", "AGE"]
    );
}

fn crd(namespaced: bool, columns: &[(&str, &str, i32)]) -> ResourceKind {
    ResourceKind::CustomResource(CrdMeta {
        crd_name: "widgets.example.io".into(),
        group: "example.io".into(),
        version: "v1".into(),
        kind: "Widget".into(),
        plural: "widgets".into(),
        singular: "widget".into(),
        namespaced,
        short_names: vec![],
        printer_columns: columns
            .iter()
            .map(|(name, path, priority)| PrinterColumn {
                name: name.to_string(),
                json_path: path.to_string(),
                col_type: "string".into(),
                priority: *priority,
                description: None,
            })
            .collect(),
    })
}

#[test]
fn custom_resource_columns_follow_the_crd_printer_columns() {
    // No printer columns: NAMESPACE (namespaced), NAME, AGE.
    assert_eq!(names(&crd(true, &[])), ["NAMESPACE", "NAME", "AGE"]);
    assert_eq!(names(&crd(false, &[])), ["NAME", "AGE"]);

    // Printer columns: priority > 0 is skipped, AGE is appended only when the
    // CRD does not declare one, and every known heading gets its width.
    let kind = crd(
        false,
        &[
            ("Ready", ".status.ready", 0),
            ("Status", ".status.phase", 0),
            ("StoreType", ".spec.store.type", 0),
            ("Store", ".spec.store.name", 0),
            ("Refresh Interval", ".spec.refresh", 0),
            ("Last Sync", ".status.lastSync", 0),
            ("Hidden", ".spec.hidden", 1),
            ("Custom", ".spec.custom", 0),
        ],
    );
    assert_eq!(
        names(&kind),
        [
            "NAME",
            "READY",
            "STATUS",
            "STORETYPE",
            "STORE",
            "REFRESH INTERVAL",
            "LAST SYNC",
            "CUSTOM",
            "AGE"
        ]
    );
    let cols = default_columns_for_kind(&kind);
    assert_eq!(cols[1].key, "printer:.status.ready");
    assert_eq!(cols[1].width, ratatui::layout::Constraint::Length(8));
    assert_eq!(cols[4].width, ratatui::layout::Constraint::Min(22));
    assert_eq!(cols[7].width, ratatui::layout::Constraint::Length(16));

    let with_age = crd(true, &[("Age", ".metadata.creationTimestamp", 0)]);
    assert_eq!(names(&with_age), ["NAMESPACE", "NAME", "AGE"]);
    assert_eq!(
        default_columns_for_kind(&with_age)[2].key,
        "printer:.metadata.creationTimestamp"
    );
}

#[test]
fn workload_segments_cycle_in_order_and_name_themselves() {
    let all = WorkloadSegment::all();
    assert_eq!(all.len(), 6);
    assert_eq!(all[0], WorkloadSegment::All);
    let mut seg = WorkloadSegment::default();
    let mut seen = Vec::new();
    for _ in 0..6 {
        seen.push(seg.display_name());
        seg = seg.next();
    }
    assert_eq!(
        seen,
        [
            "All",
            "Deployment",
            "StatefulSet",
            "DaemonSet",
            "Pod",
            "CronJob"
        ]
    );
    assert_eq!(seg, WorkloadSegment::All, "cycles back to All");
}

fn pods(n: usize) -> Vec<Value> {
    (0..n)
        .map(|i| json!({ "name": format!("pod-{i:02}"), "namespace": if i % 2 == 0 { "a" } else { "b" }, "kind": "Pod", "status": "Running" }))
        .collect()
}

#[test]
fn table_selection_jumps_and_pages_within_the_filtered_rows() {
    let mut t = ResourceTableState::new(ResourceKind::Pods);
    assert!(t.is_loading);
    // Empty table: everything is a no-op.
    t.select_bottom();
    t.page_down(5);
    t.page_up(5);
    t.select_next();
    assert_eq!(t.selected_idx, 0);
    assert!(t.selected_item().is_none());

    t.set_items(pods(10), "");
    assert!(!t.is_loading);
    t.select_bottom();
    assert_eq!(t.selected_idx, 9);
    t.select_top();
    assert_eq!(t.selected_idx, 0);
    t.page_down(4);
    assert_eq!(t.selected_idx, 4);
    t.page_down(100);
    assert_eq!(t.selected_idx, 9);
    t.page_up(3);
    assert_eq!(t.selected_idx, 6);
    t.page_up(100);
    assert_eq!(t.selected_idx, 0);
    t.select_prev();
    assert_eq!(t.selected_idx, 0);
    assert_eq!(t.selected_resource_name().as_deref(), Some("pod-00"));
    assert_eq!(t.selected_namespace().as_deref(), Some("a"));
    assert_eq!(t.selected_resource_kind().as_deref(), Some("Pod"));
}

#[test]
fn table_marks_toggle_per_raw_item_and_survive_refiltering() {
    let mut t = ResourceTableState::new(ResourceKind::Pods);
    t.set_items(pods(4), "");
    t.select_next();
    t.toggle_mark_selected();
    assert!(t.marked_indices.contains(&1));
    t.toggle_mark_selected();
    assert!(!t.marked_indices.contains(&1));
    t.toggle_mark_selected();
    // Filter down to namespace b: pod-01 and pod-03.
    t.apply_filter("^b$");
    assert_eq!(t.filtered_indices, vec![1, 3]);
    assert!(t.marked_indices.contains(&1));
    // Marking on an empty filter result is a no-op.
    t.apply_filter("nothing-matches");
    assert!(t.filtered_indices.is_empty());
    t.toggle_mark_selected();
    assert_eq!(t.marked_indices.len(), 1);
}

#[test]
fn table_filter_falls_back_to_substring_when_the_regex_is_invalid() {
    let mut t = ResourceTableState::new(ResourceKind::Pods);
    t.set_items(
        vec![
            json!({ "name": "svc(1)", "namespace": "x" }),
            json!({ "name": "other", "namespace": "y" }),
        ],
        "",
    );
    t.apply_filter("svc(");
    assert_eq!(t.filtered_indices, vec![0]);
    // Namespace is searched too, and the selection is clamped when it falls
    // outside the new result set.
    t.apply_filter("");
    t.select_bottom();
    t.apply_filter("^x$");
    assert_eq!(t.filtered_indices, vec![0]);
    assert_eq!(t.selected_idx, 0);
    // Metadata-shaped items are searched by metadata.name/namespace.
    t.set_items(
        vec![json!({ "metadata": { "name": "meta-pod", "namespace": "kube-system" } })],
        "meta",
    );
    assert_eq!(t.filtered_indices, vec![0]);
    t.apply_filter("KUBE-SYS");
    assert_eq!(t.filtered_indices, vec![0]);
}

fn events() -> Vec<Value> {
    vec![
        json!({ "namespace": "a", "type": "Normal", "reason": "Scheduled", "object": "Pod/web-0", "message": "ok" }),
        json!({ "namespace": "a", "type": "Warning", "reason": "BackOff", "object": "Pod/web-1", "message": "Back-off restarting" }),
        json!({ "namespace": "b", "type": "Normal", "reason": "Pulled", "involvedObject": { "kind": "Pod", "name": "web-2" }, "message": "image pulled" }),
        json!({ "namespace": "b", "type": "Normal", "reason": "BackOff", "object": "Pod/web-3", "message": "again" }),
    ]
}

#[test]
fn table_event_triage_and_reason_filters_narrow_the_rows() {
    let mut t = ResourceTableState::new(ResourceKind::Events);
    t.set_items(events(), "");
    assert_eq!(t.filtered_indices.len(), 4);

    assert!(t.toggle_warning_triage(""));
    assert_eq!(
        t.filtered_indices,
        vec![1, 3],
        "warning type or critical reason"
    );
    assert!(!t.toggle_warning_triage(""));
    assert_eq!(t.filtered_indices.len(), 4);

    t.select_bottom();
    t.set_reason_filter(Some("backoff".into()), "");
    assert_eq!(
        t.filtered_indices,
        vec![1, 3],
        "reason match is case-insensitive"
    );
    assert_eq!(t.selected_idx, 0);
    t.set_reason_filter(None, "");
    assert_eq!(t.filtered_indices.len(), 4);

    assert!(!t.reason_rail_focused);
    t.toggle_reason_rail_focus();
    assert!(t.reason_rail_focused);
    t.select_next_reason(2);
    t.select_next_reason(2);
    assert_eq!(t.selected_reason_idx, 1);
    t.select_next_reason(0);
    assert_eq!(t.selected_reason_idx, 1);
    t.select_prev_reason();
    t.select_prev_reason();
    assert_eq!(t.selected_reason_idx, 0);
}

#[test]
fn table_reason_and_triage_filters_only_apply_to_events() {
    let mut t = ResourceTableState::new(ResourceKind::Pods);
    t.set_items(pods(3), "");
    t.toggle_warning_triage("");
    t.set_reason_filter(Some("BackOff".into()), "");
    assert_eq!(t.filtered_indices.len(), 3);
}

#[test]
fn table_workload_segment_filters_by_kind() {
    let mut t = ResourceTableState::new(ResourceKind::Workloads);
    t.set_items(
        vec![
            json!({ "name": "d", "kind": "Deployment", "namespace": "x" }),
            json!({ "name": "s", "kind": "StatefulSet", "namespace": "x" }),
            json!({ "name": "p", "kind": "Pod", "namespace": "x" }),
        ],
        "",
    );
    t.workload_segment = WorkloadSegment::StatefulSet;
    t.apply_filter("");
    assert_eq!(t.filtered_indices, vec![1]);
    t.workload_segment = WorkloadSegment::CronJob;
    t.apply_filter("");
    assert!(t.filtered_indices.is_empty());
}

// ---------------------------------------------------------------------------
// resource_table: field extraction
// ---------------------------------------------------------------------------

#[test]
fn extract_field_str_handles_bools_arrays_and_nested_containers() {
    let item = json!({
        "suspend": true,
        "hosts": ["a.example", "b.example"],
        "emptyArr": [1, 2],
        "spec": { "schedule": "*/5 * * * *", "StorageClass": "fast" },
        "status": { "Phase": "Bound" },
        "metadata": { "uid": "abc" },
        "count": 7,
    });
    assert_eq!(extract_field_str(&item, "suspend"), "true");
    assert_eq!(extract_field_str(&item, "hosts"), "a.example, b.example");
    assert_eq!(
        extract_field_str(&item, "emptyArr"),
        "-",
        "non-string arrays are not rendered"
    );
    assert_eq!(extract_field_str(&item, "count"), "7");
    assert_eq!(extract_field_str(&item, "schedule"), "*/5 * * * *");
    assert_eq!(
        extract_field_str(&item, "storageClass"),
        "fast",
        "case-insensitive inside spec"
    );
    assert_eq!(extract_field_str(&item, "phase"), "Bound");
    assert_eq!(extract_field_str(&item, "uid"), "abc");
    assert_eq!(extract_field_str(&item, "missing"), "-");
    assert_eq!(
        extract_field_str(&item, "SUSPEND"),
        "true",
        "case-insensitive top level"
    );
}

#[test]
fn extract_field_str_resolves_service_addresses_from_spec_and_status() {
    let lb = json!({
        "spec": { "type": "LoadBalancer", "clusterIP": "10.0.0.1" },
        "status": { "loadBalancer": { "ingress": [{ "ip": "1.2.3.4" }, { "hostname": "lb.example" }] } },
    });
    assert_eq!(extract_field_str(&lb, "clusterIP"), "10.0.0.1");
    assert_eq!(extract_field_str(&lb, "externalIP"), "1.2.3.4, lb.example");

    let ext = json!({ "spec": { "type": "ClusterIP", "externalIPs": ["9.9.9.9"] } });
    assert_eq!(extract_field_str(&ext, "externalIP"), "9.9.9.9");

    let pending = json!({ "type": "LoadBalancer" });
    assert_eq!(extract_field_str(&pending, "externalIP"), "<pending>");
    let none = json!({ "type": "NodePort" });
    assert_eq!(extract_field_str(&none, "externalIP"), "<none>");
    let untyped = json!({ "name": "x" });
    assert_eq!(extract_field_str(&untyped, "externalIP"), "-");
}

#[test]
fn extract_field_str_resolves_pod_fields_from_raw_manifests() {
    let raw = json!({
        "kind": "Pod",
        "spec": {
            "nodeName": "node-7",
            "containers": [{ "image": "nginx:1.27", "resources": { "requests": { "cpu": "250m", "memory": "128Mi" } } }],
        },
        "status": { "phase": "Pending", "podIP": "10.1.1.1" },
    });
    assert_eq!(extract_field_str(&raw, "status"), "Pending");
    assert_eq!(extract_field_str(&raw, "nodeName"), "node-7");
    assert_eq!(extract_field_str(&raw, "podIp"), "10.1.1.1");
    assert_eq!(extract_field_str(&raw, "cpu"), "250m");
    assert_eq!(extract_field_str(&raw, "memory"), "128Mi");
    assert_eq!(
        extract_field_str(&raw, "restarts"),
        "—",
        "a typed item with no restarts shows an em dash"
    );
    assert_eq!(extract_field_str(&raw, "ready"), "—");
    assert_eq!(
        extract_field_str(&raw, "image"),
        "—",
        "pod images live at spec.containers, not the template path"
    );

    let summary = json!({ "kind": "Pod", "node": "n1", "podIP": "10.2.2.2", "cpuUsage": "5m", "memUsage": "9Mi", "restarts": 3, "ready": "1/1", "image": "busybox" });
    assert_eq!(extract_field_str(&summary, "node"), "n1");
    assert_eq!(extract_field_str(&summary, "ip"), "10.2.2.2");
    assert_eq!(extract_field_str(&summary, "cpu"), "5m");
    assert_eq!(extract_field_str(&summary, "mem"), "9Mi");
    assert_eq!(extract_field_str(&summary, "restarts"), "3");
    assert_eq!(extract_field_str(&summary, "ready"), "1/1");
    assert_eq!(extract_field_str(&summary, "image"), "busybox");

    let deploy = json!({ "kind": "Deployment", "spec": { "template": { "spec": { "containers": [{ "image": "api:2" }] } } } });
    assert_eq!(extract_field_str(&deploy, "image"), "api:2");
    assert_eq!(extract_field_str(&deploy, "cpu"), "—");
    assert_eq!(extract_field_str(&deploy, "memory"), "—");

    let untyped = json!({ "name": "x" });
    for key in [
        "restarts", "cpu", "memory", "image", "ready", "status", "nodeName", "podIp",
    ] {
        assert_eq!(extract_field_str(&untyped, key), "-", "{key}");
    }
}

#[test]
fn extract_field_str_formats_event_objects_and_type_aliases() {
    let with_obj = json!({ "object": "Pod/web-0", "type_": "Warning" });
    assert_eq!(extract_field_str(&with_obj, "involvedObject"), "Pod/web-0");
    assert_eq!(extract_field_str(&with_obj, "type"), "Warning");
    assert_eq!(extract_field_str(&with_obj, "type_"), "Warning");

    let inv = json!({ "involvedObject": { "kind": "Node", "name": "n1" } });
    assert_eq!(extract_field_str(&inv, "object"), "Node/n1");
    let empty_inv = json!({ "involvedObject": {} });
    assert_eq!(extract_field_str(&empty_inv, "object"), "-");
}

#[test]
fn is_event_warning_or_failure_checks_type_reason_and_message() {
    assert!(is_event_warning_or_failure(&json!({ "type_": "warning" })));
    assert!(is_event_warning_or_failure(
        &json!({ "type": "Normal", "reason": "BackOff" })
    ));
    assert!(is_event_warning_or_failure(
        &json!({ "type": "Normal", "reason": "Pulled", "message": "container was OOMKilled" })
    ));
    assert!(is_event_warning_or_failure(&json!({ "reason": "Killing" })));
    assert!(!is_event_warning_or_failure(
        &json!({ "type": "Normal", "reason": "Scheduled", "message": "all good" })
    ));
    assert!(!is_event_warning_or_failure(&json!({})));
}

#[test]
fn eval_crd_json_path_walks_fields_indexes_and_filters() {
    let obj = json!({
        "status": {
            "phase": "Ready",
            "ready": true,
            "count": 3,
            "conditions": [
                { "type": "Ready", "status": "True" },
                { "type": "Synced", "status": "False" },
            ],
            "tags": ["a", "", "b"],
            "nested": { "x": 1 },
            "nothing": null,
            "empty": "",
            "created": "2020-01-01T00:00:00Z",
        },
        "spec": { "items": [{ "name": "first" }, { "name": "second" }], "odd.key": "v" },
    });
    assert_eq!(eval_crd_json_path(&obj, ""), "-");
    assert_eq!(eval_crd_json_path(&obj, ".status.phase"), "Ready");
    assert_eq!(eval_crd_json_path(&obj, "status.ready"), "True");
    assert_eq!(eval_crd_json_path(&obj, ".status.count"), "3");
    assert_eq!(eval_crd_json_path(&obj, ".spec.items[1].name"), "second");
    assert_eq!(eval_crd_json_path(&obj, ".spec.items[5].name"), "-");
    assert_eq!(eval_crd_json_path(&obj, ".spec[\"odd.key\"]"), "v");
    assert_eq!(
        eval_crd_json_path(&obj, ".status.conditions[?(@.type==\"Ready\")].status"),
        "True"
    );
    assert_eq!(
        eval_crd_json_path(&obj, ".status.conditions[?(@.type=='Synced')].status"),
        "False"
    );
    assert_eq!(
        eval_crd_json_path(&obj, ".status.conditions.[?(@.type==\"Ready\")].status"),
        "True",
        "filter with no leading field"
    );
    assert_eq!(
        eval_crd_json_path(&obj, ".status.conditions[?(@.type==\"Missing\")].status"),
        "-"
    );
    assert_eq!(
        eval_crd_json_path(&obj, ".status.phase[?(@.type==\"Ready\")]"),
        "-",
        "filter on a non-array"
    );
    assert_eq!(eval_crd_json_path(&obj, ".status.tags"), "a, b");
    assert_eq!(eval_crd_json_path(&obj, ".status.nested"), "-");
    assert_eq!(eval_crd_json_path(&obj, ".status.nothing"), "-");
    assert_eq!(eval_crd_json_path(&obj, ".status.empty"), "-");
    assert_eq!(eval_crd_json_path(&obj, ".status.missing.deeper"), "-");
    assert_eq!(
        eval_crd_json_path(&obj, ".status.ready[0]"),
        "-",
        "index into a non-array"
    );
    let age = eval_crd_json_path(&obj, ".status.created");
    assert_ne!(
        age, "2020-01-01T00:00:00Z",
        "RFC3339 timestamps become an age"
    );
    assert_ne!(age, "-");
    // A printer key routes through the same evaluator.
    assert_eq!(extract_field_str(&obj, "printer:.status.phase"), "Ready");
}

// ---------------------------------------------------------------------------
// resource_table: rendering
// ---------------------------------------------------------------------------

#[test]
fn table_renders_loading_and_empty_states() {
    let mut t = ResourceTableState::new(ResourceKind::Pods);
    let text = common::render_text(120, 40, |f| render_resource_table(f, f.area(), &t));
    assert!(text.contains(" Pods [Loading...] "), "{text}");
    assert!(text.contains("Loading Pods from cluster API..."), "{text}");
    assert_eq!(t.last_area_width.get(), 120);

    t.set_items(vec![], "");
    let text = common::render_text(60, 20, |f| render_resource_table(f, f.area(), &t));
    assert!(text.contains(" Pods [0] "), "{text}");
    assert!(text.contains("No Pods found in this scope."), "{text}");
    assert_eq!(t.last_area_width.get(), 60);
}

#[test]
fn table_renders_headers_rows_marks_and_the_filtered_count_badge() {
    let mut t = ResourceTableState::new(ResourceKind::Pods);
    t.set_items(
        vec![
            json!({ "name": "web-0", "namespace": "prod", "ready": "1/1", "status": "Running", "restarts": 0, "podIp": "10.0.0.1", "nodeName": "n1", "age": "2d" }),
            json!({ "name": "web-1", "namespace": "prod", "ready": "0/1", "status": "CrashLoopBackOff", "restarts": 12, "age": "1h" }),
            json!({ "name": "job-x", "namespace": "batch", "status": "Pending" }),
        ],
        "",
    );
    t.select_next();
    t.toggle_mark_selected();
    t.apply_filter("web");
    let buf = render_buffer(160, 40, |f| render_resource_table(f, f.area(), &t));
    let rows: Vec<String> = (0..buf.area.height).map(|y| buffer_row(&buf, y)).collect();
    assert!(rows[0].contains(" Pods [2/3] "), "{}", rows[0]);
    for h in [
        "NAMESPACE",
        "NAME",
        "READY",
        "STATUS",
        "RESTARTS",
        "CPU",
        "MEM",
        "IP",
        "NODE",
        "AGE",
    ] {
        assert!(rows[1].contains(h), "header {h} in {}", rows[1]);
    }
    let (y0, r0) = row_containing(&rows, "web-0").unwrap();
    assert!(
        r0.contains("prod")
            && r0.contains("1/1")
            && r0.contains("Running")
            && r0.contains("10.0.0.1")
            && r0.contains("n1")
            && r0.contains("2d"),
        "{r0}"
    );
    assert_eq!(fg_of(&buf, y0 as u16, "Running"), Theme::status_ok().fg);
    let (y1, r1) = row_containing(&rows, "✔ web-1").unwrap();
    // STATUS is a fixed 14-column cell, so the 16-char phase is clipped.
    assert!(
        r1.contains("CrashLoopBackO") && r1.contains("12") && r1.contains("0/1"),
        "{r1}"
    );
    assert_eq!(
        fg_of(&buf, y1 as u16, "CrashLoop"),
        Theme::status_error().fg
    );
    // The marked row is also the selected one, so it takes the selection bg.
    assert_eq!(
        buf.cell(Position::new(2, y1 as u16)).unwrap().style().bg,
        Theme::selected_row().bg
    );
    assert!(!rows.iter().any(|l| l.contains("job-x")));
    assert_eq!(t.last_start_idx.get(), 0);
    assert_eq!(t.last_viewport_rect.get().width, 158);
}

#[test]
fn table_marked_but_unselected_row_uses_the_marked_background() {
    let mut t = ResourceTableState::new(ResourceKind::Namespaces);
    t.set_items(
        vec![
            json!({ "name": "default", "status": "Active" }),
            json!({ "name": "kube-system", "status": "Terminating" }),
        ],
        "",
    );
    t.toggle_mark_selected();
    t.select_next();
    let buf = render_buffer(80, 10, |f| render_resource_table(f, f.area(), &t));
    let rows: Vec<String> = (0..buf.area.height).map(|y| buffer_row(&buf, y)).collect();
    let (y0, r0) = row_containing(&rows, "✔ default").unwrap();
    assert!(r0.contains("Active"), "{r0}");
    assert_eq!(
        buf.cell(Position::new(2, y0 as u16)).unwrap().style().bg,
        Theme::marked_row().bg
    );
    let (y1, _) = row_containing(&rows, "kube-system").unwrap();
    assert_eq!(
        buf.cell(Position::new(2, y1 as u16)).unwrap().style().bg,
        Theme::selected_row().bg
    );
    assert_eq!(
        fg_of(&buf, y1 as u16, "Terminating"),
        Theme::status_warn().fg
    );
}

#[test]
fn table_scrolls_to_keep_a_far_selection_in_a_short_viewport() {
    let mut t = ResourceTableState::new(ResourceKind::Pods);
    t.set_items(pods(40), "");
    t.select_bottom();
    // 12 rows: inner 10, minus header+margin -> 8 visible; start = 39 - 4 = 35.
    let lines = common::render_lines(120, 12, |f| render_resource_table(f, f.area(), &t));
    assert!(lines.iter().any(|l| l.contains("pod-39")), "{lines:?}");
    assert!(lines.iter().any(|l| l.contains("pod-35")), "{lines:?}");
    assert!(!lines.iter().any(|l| l.contains("pod-34")), "{lines:?}");
    assert_eq!(t.last_start_idx.get(), 35);
    // Alternating stripes: the even display row gets the stripe background.
    let buf = render_buffer(120, 12, |f| render_resource_table(f, f.area(), &t));
    let rows: Vec<String> = (0..buf.area.height).map(|y| buffer_row(&buf, y)).collect();
    let (y36, _) = row_containing(&rows, "pod-36").unwrap();
    let (y37, _) = row_containing(&rows, "pod-37").unwrap();
    assert_eq!(
        buf.cell(Position::new(2, y36 as u16)).unwrap().style().bg,
        Some(Color::Rgb(22, 24, 30))
    );
    assert_ne!(
        buf.cell(Position::new(2, y37 as u16)).unwrap().style().bg,
        Some(Color::Rgb(22, 24, 30))
    );
}

#[test]
fn events_table_shows_the_reason_rail_only_when_wide_enough() {
    let mut t = ResourceTableState::new(ResourceKind::Events);
    t.set_items(events(), "");
    // Exactly at the 110-column threshold the rail appears (32 columns wide,
    // so its title is clipped) and the table columns are squeezed.
    let wide = common::render_lines(110, 30, |f| render_resource_table(f, f.area(), &t));
    let text = wide.join("\n");
    assert!(text.contains("Event Reasons [Tab/<R>"), "{text}");
    assert!(text.contains("● BackOff (2)"), "{text}");
    assert!(text.contains("○ Scheduled (1)"), "{text}");
    assert!(text.contains("○ Pulled (1)"), "{text}");
    assert!(
        wide[1].contains("TYPE")
            && wide[1].contains("REASON")
            && wide[1].contains("OBJECT")
            && wide[1].contains("MESSAGE"),
        "{}",
        wide[1]
    );

    let roomy = common::render_lines(180, 30, |f| render_resource_table(f, f.area(), &t));
    assert!(
        roomy[1].contains("NAMESPACE") && roomy[1].contains("LAST SEEN"),
        "{}",
        roomy[1]
    );
    let text = roomy.join("\n");
    assert!(
        text.contains("Pod/web-2"),
        "involvedObject is formatted: {text}"
    );
    assert!(
        text.contains("Pod/web-0"),
        "object string is used verbatim: {text}"
    );

    let narrow = common::render_text(109, 30, |f| render_resource_table(f, f.area(), &t));
    assert!(!narrow.contains("Event Reasons"), "{narrow}");
    assert!(narrow.contains(" Events [4] "), "{narrow}");
}

#[test]
fn events_table_title_carries_triage_and_reason_badges_and_colours_the_type() {
    let mut t = ResourceTableState::new(ResourceKind::Events);
    t.set_items(events(), "");
    t.toggle_warning_triage("");
    t.set_reason_filter(Some("BackOff".into()), "");
    let buf = render_buffer(160, 30, |f| render_resource_table(f, f.area(), &t));
    let rows: Vec<String> = (0..buf.area.height).map(|y| buffer_row(&buf, y)).collect();
    // Both BackOff events survive triage (the reason itself is critical).
    assert!(
        rows[0].contains(" Events [2/4] [TRIAGE: ON] [Reason: BackOff] "),
        "{}",
        rows[0]
    );
    assert_eq!(
        fg_of(&buf, 0, "Events ["),
        Some(Color::Yellow),
        "triage title is yellow"
    );
    assert_eq!(
        buf.cell(Position::new(0, 0)).unwrap().style().fg,
        Some(Color::Yellow),
        "triage border is yellow"
    );
    let (y, row) = row_containing(&rows, "Back-off restarting").unwrap();
    assert!(row.contains("Warning") && row.contains("BackOff"), "{row}");
    assert_eq!(fg_of(&buf, y as u16, "Warning"), Theme::status_warn().fg);
    assert_eq!(fg_of(&buf, y as u16, "BackOff"), Theme::status_error().fg);
    // The rail shows the active filter tick and the focused title when focused.
    assert!(rows.iter().any(|l| l.contains("BackOff (2) ✔")), "{rows:?}");
    t.toggle_reason_rail_focus();
    let buf = render_buffer(160, 30, |f| render_resource_table(f, f.area(), &t));
    assert_eq!(
        buf.cell(Position::new(0, 0)).unwrap().style().fg,
        Some(Theme::BORDER),
        "border returns to normal while the rail is focused"
    );
    // The 32-column rail clips its title, but the focused variant starts
    // with the navigation hint where the unfocused one says "Tab/<R>".
    assert!(
        buffer_row(&buf, 0).contains("Event Reasons [↑/↓"),
        "{}",
        buffer_row(&buf, 0)
    );
    assert!(
        !buffer_row(&buf, 0).contains("Tab/<R>"),
        "{}",
        buffer_row(&buf, 0)
    );
}

#[test]
fn workloads_table_title_shows_the_segment_and_kind_column() {
    let mut t = ResourceTableState::new(ResourceKind::Workloads);
    t.set_items(
        vec![
            json!({ "name": "api", "kind": "Deployment", "namespace": "prod", "ready": "2/2", "status": "Available", "image": "api:1" }),
            json!({ "name": "api-0", "kind": "Pod", "namespace": "prod", "ready": "1/1", "status": "Running" }),
        ],
        "",
    );
    let text = common::render_text(160, 20, |f| render_resource_table(f, f.area(), &t));
    assert!(
        text.contains(" Workloads [Segment: All (Tab to cycle)] [2] "),
        "{text}"
    );
    assert!(text.contains("KIND") && text.contains("IMAGE"), "{text}");
    assert!(
        text.contains("Deployment") && text.contains("api:1"),
        "{text}"
    );
    t.workload_segment = WorkloadSegment::Pod;
    t.apply_filter("");
    let text = common::render_text(160, 20, |f| render_resource_table(f, f.area(), &t));
    assert!(
        text.contains(" Workloads [Segment: Pod (Tab to cycle)] [1/2] "),
        "{text}"
    );
    assert!(!text.contains("api:1"), "{text}");
}

#[test]
fn custom_resource_table_colours_printer_status_columns() {
    let kind = crd(
        true,
        &[
            ("Ready", ".status.ready", 0),
            ("Health", ".status.health", 0),
            ("Sync", ".status.sync", 0),
        ],
    );
    let mut t = ResourceTableState::new(kind);
    t.set_items(
        vec![
            json!({ "metadata": { "name": "w1", "namespace": "ns" }, "status": { "ready": "True", "health": "Healthy", "sync": "Synced" } }),
            json!({ "metadata": { "name": "w2", "namespace": "ns" }, "status": { "ready": "False", "health": "Degraded", "sync": "OutOfSync" } }),
        ],
        "",
    );
    let buf = render_buffer(120, 20, |f| render_resource_table(f, f.area(), &t));
    let rows: Vec<String> = (0..buf.area.height).map(|y| buffer_row(&buf, y)).collect();
    assert!(rows[0].contains(" Widget [2] "), "{}", rows[0]);
    assert!(
        rows[1].contains("READY")
            && rows[1].contains("HEALTH")
            && rows[1].contains("SYNC")
            && rows[1].contains("AGE"),
        "{}",
        rows[1]
    );
    let (y1, r1) = row_containing(&rows, "w1").unwrap();
    assert!(
        r1.contains("True") && r1.contains("Healthy") && r1.contains("Synced"),
        "{r1}"
    );
    assert_eq!(fg_of(&buf, y1 as u16, "True"), Theme::status_ok().fg);
    // "Healthy" is not a status word the theme knows, so it falls back to the
    // plain foreground rather than the selected-row foreground: proof the
    // HEALTH column went through status_style even on the selected row.
    assert_eq!(fg_of(&buf, y1 as u16, "Healthy"), Some(Theme::FG));
    assert_ne!(
        fg_of(&buf, y1 as u16, "w1"),
        Some(Theme::FG),
        "the name cell is selection-styled"
    );
    let (y2, r2) = row_containing(&rows, "w2").unwrap();
    assert!(r2.contains("Degraded"), "{r2}");
    assert_eq!(fg_of(&buf, y2 as u16, "False"), Theme::status_error().fg);
    assert_eq!(fg_of(&buf, y2 as u16, "Degraded"), Theme::status_error().fg);
}
