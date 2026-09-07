//! Rendering tests for the TUI's chrome: modals, the help overlay, the header,
//! the status bar, and the AI assistant view.
//!
//! Every test renders through ratatui's `TestBackend` (see `common`) and
//! asserts on the screen text, or drives `AssistantViewState` directly and
//! asserts on the resulting state.

mod common;

use std::cell::RefCell;
use std::time::Duration;

use crossterm::event::KeyCode;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use srelens_tui::ai_skills::CavemanLevel;
use srelens_tui::app::ActiveView;
use srelens_tui::commands::command_suggestions;
use srelens_tui::ui::dialogs::{
    render_modal, ContainerAction, ContextPickerItem, Modal, QuickActionId, QuickActionItem,
};
use srelens_tui::ui::header::{render_header, ContextChipInfo, HeaderProps};
use srelens_tui::ui::help::{centered_rect, render_help_modal};
use srelens_tui::ui::statusbar::{render_statusbar, InputMode, StatusBarProps};
use srelens_tui::views::assistant_view::{
    format_message_content, format_message_content_with_width, parse_inline_markdown,
    render_assistant_view, render_markdown_table, wrap_line, AssistantViewState, ChatMessage,
    TokenUsage, ToolCallRecord, ToolCallStatus,
};
use srelens_tui::views::metrics_panel_view::MetricsPanelState;
use srelens_tui::views::reason_rail::ReasonTally;
use srelens_tui::AiSettings;

// ───────────────────────── helpers ─────────────────────────

fn modal_text(width: u16, height: u16, modal: &Modal) -> String {
    common::render_text(width, height, |f| render_modal(f, f.area(), modal))
}

fn line_text(line: &Line) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

/// Run the markdown formatter over `content` and return one plain string per
/// produced line.
fn md(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    format_message_content(&mut out, content);
    out.iter().map(line_text).collect()
}

fn assistant_text(width: u16, height: u16, state: &AssistantViewState) -> String {
    let settings = AiSettings::default();
    common::render_text(width, height, |f| {
        render_assistant_view(f, f.area(), state, &settings)
    })
}

fn msg(role: &str, content: &str) -> ChatMessage {
    ChatMessage {
        role: role.to_string(),
        content: content.to_string(),
        timestamp: "10:00:00".to_string(),
        tool_calls: Vec::new(),
        token_usage: None,
    }
}

fn tool(id: &str, name: &str, args: &str, status: ToolCallStatus) -> ToolCallRecord {
    ToolCallRecord {
        id: id.to_string(),
        tool: name.to_string(),
        args_summary: args.to_string(),
        status,
    }
}

/// A fresh assistant state whose greeting has a fixed timestamp so rendered
/// rows are deterministic.
fn fresh_state() -> AssistantViewState {
    let mut state = AssistantViewState::new();
    state.messages[0].timestamp = "10:00:00".to_string();
    state
}

/// The `(Ns elapsed)` count from the busy line that starts with `prefix`.
/// Panics with the frame if the line or the count is not there, so a renderer
/// that stops showing elapsed time fails loudly rather than silently.
fn elapsed_seconds(text: &str, prefix: &str) -> u64 {
    let line = text
        .lines()
        .find(|l| l.contains(prefix))
        .unwrap_or_else(|| panic!("no busy line starting {prefix:?}: {text}"));
    let (_, after) = line
        .split_once(&format!("{prefix} ("))
        .unwrap_or_else(|| panic!("no elapsed badge on {line:?}"));
    let (count, _) = after
        .split_once("s elapsed)")
        .unwrap_or_else(|| panic!("no elapsed badge on {line:?}"));
    count
        .parse()
        .unwrap_or_else(|e| panic!("elapsed count {count:?} is not a number: {e}"))
}

fn status_props(mode: &InputMode) -> StatusBarProps<'_> {
    StatusBarProps {
        mode,
        command_input: "",
        filter_input: "",
        matched_count: 0,
        total_count: 0,
        is_text_search: false,
        toast: None,
        custom_hints: None,
        suggestions: None,
        close_pf_button: None,
        close_pf_rect: None,
    }
}

fn statusbar_text(props: StatusBarProps) -> String {
    let lines = common::render_lines(200, 24, |f| {
        render_statusbar(f, Rect::new(0, 22, 200, 2), props)
    });
    lines[22..].join(
        "
",
    )
}

fn header_props<'a>(contexts: &'a [ContextChipInfo]) -> HeaderProps<'a> {
    HeaderProps {
        context: "prod-east",
        cluster: "prod-east-cluster",
        server: "https://10.0.0.1:6443",
        namespace: "default",
        version: "v1.29.2",
        node_count: 3,
        pod_count: 42,
        is_connected: true,
        active_view_name: "Pods",
        contexts,
        context_chip_rects: None,
    }
}

fn header_text(width: u16, height: u16, props: HeaderProps) -> String {
    common::render_text(width, height, |f| {
        render_header(f, Rect::new(0, 0, width, height), props)
    })
}

fn chip(index: usize, name: &str, is_current: bool, is_local: bool) -> ContextChipInfo {
    ContextChipInfo {
        name: name.to_string(),
        is_current,
        is_local,
        index,
    }
}

fn ctx_item(
    name: &str,
    cluster: &str,
    ns: &str,
    is_local: bool,
    provider: Option<&str>,
    file: &str,
) -> ContextPickerItem {
    ContextPickerItem {
        name: name.to_string(),
        cluster: cluster.to_string(),
        server: format!("https://{}.example:6443", cluster),
        namespace: ns.to_string(),
        is_local,
        provider: provider.map(str::to_string),
        source_file: file.to_string(),
    }
}

fn sample_contexts() -> Vec<ContextPickerItem> {
    vec![
        ctx_item(
            "prod-east",
            "prod-east-cluster",
            "payments",
            false,
            Some("gke"),
            "/home/sre/.kube/config",
        ),
        ctx_item(
            "dev-local",
            "kind-dev",
            "",
            true,
            None,
            "/home/sre/.kube/kind.yaml",
        ),
        ctx_item("staging", "stg", "web", false, None, "/etc/kube/staging"),
    ]
}

fn action(id: QuickActionId, key: &str, title: &str, desc: &str) -> QuickActionItem {
    QuickActionItem {
        id,
        key_hint: key.to_string(),
        title: title.to_string(),
        description: desc.to_string(),
    }
}

fn sample_actions() -> Vec<QuickActionItem> {
    vec![
        action(
            QuickActionId::AskAi,
            "a",
            "Ask AI",
            "Investigate this resource with the assistant",
        ),
        action(
            QuickActionId::ViewLogs,
            "l",
            "View Logs",
            "Stream container logs",
        ),
        action(
            QuickActionId::Delete,
            "^d",
            "Delete",
            "Remove the resource (confirmation required)",
        ),
    ]
}

fn confirm(destructive: bool) -> Modal {
    Modal::Confirm {
        title: if destructive {
            "Delete Pod".into()
        } else {
            "Restart Deployment".into()
        },
        message: if destructive {
            "Delete pod web-0 in namespace default?".into()
        } else {
            "Rollout restart deployment web?".into()
        },
        action_name: if destructive {
            "Delete".into()
        } else {
            "Restart".into()
        },
        is_destructive: destructive,
    }
}

fn every_modal() -> Vec<Modal> {
    vec![
        confirm(true),
        Modal::Scale {
            workload_name: "web".into(),
            current_replicas: 3,
            input: "5".into(),
        },
        Modal::PortForward {
            pod_name: "web-0".into(),
            namespace: "default".into(),
            container_port: 8080,
            local_port_input: "8080".into(),
            kind: "Pod".into(),
        },
        Modal::ContainerPicker {
            pod_name: "web-0".into(),
            namespace: Some("default".into()),
            containers: vec!["app".into(), "sidecar".into()],
            selected_idx: 0,
            action: ContainerAction::Logs,
        },
        Modal::ContextPicker {
            contexts: sample_contexts(),
            current_context: "prod-east".into(),
            selected_idx: 0,
            filter: String::new(),
        },
        Modal::NamespacePicker {
            namespaces: vec!["default".into(), "kube-system".into()],
            current_namespace: "default".into(),
            selected_idx: 0,
            filter: String::new(),
        },
        Modal::ActionPalette {
            resource_kind: "Pod".into(),
            resource_name: "web-0".into(),
            namespace: None,
            actions: sample_actions(),
            selected_idx: 0,
            filter: String::new(),
        },
        Modal::MetricsTimeline(MetricsPanelState::new(
            "Pod".into(),
            "web-0".into(),
            Some("default".into()),
            vec![],
        )),
        Modal::ReasonRail {
            tallies: vec![ReasonTally {
                reason: "BackOff".into(),
                count: 3,
                event_type: "Warning".into(),
            }],
            selected_idx: 0,
            active_filter: None,
        },
    ]
}

// ───────────────────────── dialogs: Confirm ─────────────────────────

#[test]
fn the_delete_confirmation_modal_names_the_resource_and_both_choices() {
    let text = modal_text(120, 40, &confirm(true));
    assert!(text.contains(" Delete Pod "), "{text}");
    assert!(
        text.contains("Delete pod web-0 in namespace default?"),
        "{text}"
    );
    assert!(
        text.contains("Press [Enter/y] to Delete  |  [Esc/n] to Cancel"),
        "{text}"
    );
}

#[test]
fn a_non_destructive_confirmation_uses_its_own_action_verb() {
    let text = modal_text(120, 40, &confirm(false));
    assert!(text.contains(" Restart Deployment "), "{text}");
    assert!(text.contains("Rollout restart deployment web?"), "{text}");
    assert!(text.contains("[Enter/y] to Restart"), "{text}");
}

#[test]
fn a_long_confirmation_message_wraps_inside_the_modal() {
    let modal = Modal::Confirm {
        title: "Drain Node".into(),
        message: "Drain node worker-1? Every pod on it will be evicted and rescheduled elsewhere in the cluster.".into(),
        action_name: "Drain".into(),
        is_destructive: true,
    };
    let lines = common::render_lines(100, 40, |f| render_modal(f, f.area(), &modal));
    let body: Vec<&String> = lines
        .iter()
        .filter(|l| l.contains("Drain node") || l.contains("evicted") || l.contains("cluster."))
        .collect();
    assert!(
        body.len() >= 2,
        "message should span several rows: {lines:?}"
    );
    assert!(
        lines.iter().any(|l| l.contains("[Enter/y] to Drain")),
        "{lines:?}"
    );
}

// ───────────────────────── dialogs: Scale / PortForward ─────────────────────────

#[test]
fn the_scale_modal_shows_current_replicas_and_the_typed_target() {
    let modal = Modal::Scale {
        workload_name: "web".into(),
        current_replicas: 3,
        input: "5".into(),
    };
    let text = modal_text(120, 40, &modal);
    assert!(text.contains(" Scale Workload: web "), "{text}");
    assert!(text.contains("Current replicas: 3"), "{text}");
    assert!(text.contains(" Desired Replicas "), "{text}");
    assert!(text.contains("5█"), "{text}");
    assert!(text.contains("[Enter] Apply  [Esc] Cancel"), "{text}");
}

#[test]
fn an_empty_scale_input_shows_only_the_cursor() {
    let modal = Modal::Scale {
        workload_name: "api".into(),
        current_replicas: 0,
        input: String::new(),
    };
    let lines = common::render_lines(120, 40, |f| render_modal(f, f.area(), &modal));
    let cursor_row = lines.iter().find(|l| l.contains('█')).expect("cursor row");
    assert!(
        cursor_row.trim_matches(|c| c == '│' || c == ' ') == "█",
        "{cursor_row:?}"
    );
    assert!(
        lines.iter().any(|l| l.contains("Current replicas: 0")),
        "{lines:?}"
    );
}

#[test]
fn the_port_forward_modal_shows_the_target_port_and_the_local_port_being_typed() {
    let modal = Modal::PortForward {
        pod_name: "web-0".into(),
        namespace: "default".into(),
        container_port: 8080,
        local_port_input: "80".into(),
        kind: "Pod".into(),
    };
    let text = modal_text(120, 40, &modal);
    assert!(
        text.contains(" Start Port Forward: web-0 (default) "),
        "{text}"
    );
    assert!(text.contains("Target container port: 8080"), "{text}");
    assert!(text.contains(" Local Port (127.0.0.1) "), "{text}");
    assert!(text.contains("80█"), "{text}");
    assert!(text.contains("[Enter] Forward  [Esc] Cancel"), "{text}");
}

// ───────────────────────── dialogs: ContainerPicker ─────────────────────────

#[test]
fn the_container_picker_titles_itself_after_the_action_and_marks_the_selection() {
    let mut modal = Modal::ContainerPicker {
        pod_name: "web-0".into(),
        namespace: Some("default".into()),
        containers: vec!["app".into(), "sidecar".into(), "init-db".into()],
        selected_idx: 1,
        action: ContainerAction::Logs,
    };
    let text = modal_text(120, 40, &modal);
    assert!(text.contains(" View Logs for Container (web-0) "), "{text}");
    assert!(text.contains("  app"), "{text}");
    assert!(text.contains("▶ sidecar"), "{text}");
    assert!(text.contains("  init-db"), "{text}");

    if let Modal::ContainerPicker {
        action,
        selected_idx,
        ..
    } = &mut modal
    {
        *action = ContainerAction::Shell;
        *selected_idx = 2;
    }
    let text = modal_text(120, 40, &modal);
    assert!(
        text.contains(" Exec Shell into Container (web-0) "),
        "{text}"
    );
    assert!(text.contains("▶ init-db"), "{text}");
    assert!(!text.contains("▶ sidecar"), "{text}");
}

// ───────────────────────── dialogs: ContextPicker ─────────────────────────

#[test]
fn the_context_picker_lists_every_context_with_badges_and_marks_the_active_one() {
    let modal = Modal::ContextPicker {
        contexts: sample_contexts(),
        current_context: "prod-east".into(),
        selected_idx: 1,
        filter: String::new(),
    };
    let text = modal_text(140, 40, &modal);
    assert!(
        text.contains(
            " Switch Kubernetes Context (Type to filter, ↑/↓ Navigate, Enter Switch, Esc Close) "
        ),
        "{text}"
    );
    assert!(text.contains(" Filter Contexts "), "{text}");
    assert!(
        text.contains(" / █"),
        "empty filter renders just the cursor: {text}"
    );
    assert!(
        text.contains("★ [gke] prod-east (active) ns:[payments]"),
        "{text}"
    );
    assert!(
        text.contains("cluster: prod-east-cluster  •  file: config"),
        "{text}"
    );
    assert!(text.contains("▶ [local] dev-local"), "{text}");
    assert!(!text.contains("dev-local (active)"), "{text}");
    assert!(
        text.contains("cluster: kind-dev  •  file: kind.yaml"),
        "{text}"
    );
    assert!(text.contains("  [remote] staging ns:[web]"), "{text}");
    assert!(
        text.contains("Showing 3/3 contexts  •  Enter: Switch  Esc: Cancel"),
        "{text}"
    );
}

#[test]
fn the_context_picker_filter_matches_name_cluster_provider_and_file_case_insensitively() {
    let mut modal = Modal::ContextPicker {
        contexts: sample_contexts(),
        current_context: "prod-east".into(),
        selected_idx: 0,
        filter: "GKE".into(),
    };
    let text = modal_text(140, 40, &modal);
    assert!(text.contains(" / GKE█"), "{text}");
    assert!(text.contains("prod-east"), "{text}");
    assert!(!text.contains("dev-local"), "{text}");
    assert!(text.contains("Showing 1/3 contexts"), "{text}");

    for (filter, expected) in [
        ("stg", "staging"),
        ("kind.yaml", "dev-local"),
        ("dev-lo", "dev-local"),
    ] {
        if let Modal::ContextPicker { filter: f, .. } = &mut modal {
            *f = filter.to_string();
        }
        let text = modal_text(140, 40, &modal);
        assert!(
            text.contains(expected),
            "filter {filter:?} should keep {expected}: {text}"
        );
        assert!(
            text.contains("Showing 1/3 contexts"),
            "filter {filter:?}: {text}"
        );
    }

    if let Modal::ContextPicker { filter: f, .. } = &mut modal {
        *f = "nothing-matches".to_string();
    }
    let text = modal_text(140, 40, &modal);
    assert!(text.contains("Showing 0/3 contexts"), "{text}");
}

// ───────────────────────── dialogs: NamespacePicker ─────────────────────────

#[test]
fn the_namespace_picker_stars_the_active_namespace_and_arrows_the_selection() {
    let modal = Modal::NamespacePicker {
        namespaces: vec![
            "default".into(),
            "kube-system".into(),
            "kube-public".into(),
            "payments".into(),
        ],
        current_namespace: "default".into(),
        selected_idx: 1,
        filter: String::new(),
    };
    let text = modal_text(140, 40, &modal);
    assert!(
        text.contains(" Switch Namespace (0: All Namespaces, ↑/↓ Navigate, Enter Select) "),
        "{text}"
    );
    assert!(text.contains(" Filter "), "{text}");
    assert!(text.contains("★ default"), "{text}");
    assert!(text.contains("▶ kube-system"), "{text}");
    assert!(text.contains("  kube-public"), "{text}");
    assert!(text.contains("  payments"), "{text}");
}

#[test]
fn the_namespace_picker_filter_narrows_the_list_and_reindexes_the_selection() {
    let modal = Modal::NamespacePicker {
        namespaces: vec![
            "default".into(),
            "kube-system".into(),
            "kube-public".into(),
            "payments".into(),
        ],
        current_namespace: "default".into(),
        selected_idx: 0,
        filter: "kube".into(),
    };
    let text = modal_text(140, 40, &modal);
    assert!(text.contains("kube█"), "{text}");
    assert!(text.contains("▶ kube-system"), "{text}");
    assert!(text.contains("  kube-public"), "{text}");
    assert!(!text.contains("default"), "{text}");
    assert!(!text.contains("payments"), "{text}");
}

// ───────────────────────── dialogs: ActionPalette ─────────────────────────

#[test]
fn the_action_palette_shows_every_action_with_its_key_and_description() {
    let modal = Modal::ActionPalette {
        resource_kind: "Pod".into(),
        resource_name: "web-0".into(),
        namespace: Some("default".into()),
        actions: sample_actions(),
        selected_idx: 0,
        filter: String::new(),
    };
    let text = modal_text(140, 40, &modal);
    assert!(text.contains(" ⚡  Actions: Pod/web-0 (default) [Type to filter, ↑/↓ Navigate, Enter Run, Esc Close] "), "{text}");
    assert!(text.contains(" Filter Actions "), "{text}");
    assert!(text.contains("▶ [a] Ask AI"), "{text}");
    assert!(
        text.contains("Investigate this resource with the assistant"),
        "{text}"
    );
    assert!(text.contains("  [l] View Logs"), "{text}");
    assert!(text.contains("  [^d] Delete"), "{text}");
    assert!(
        text.contains("Showing 3/3 actions  •  Enter: Run Action  Esc: Cancel"),
        "{text}"
    );
}

#[test]
fn the_action_palette_drops_the_namespace_suffix_for_cluster_scoped_resources_and_filters_by_description(
) {
    let mut modal = Modal::ActionPalette {
        resource_kind: "Node".into(),
        resource_name: "worker-1".into(),
        namespace: None,
        actions: sample_actions(),
        selected_idx: 0,
        filter: "stream".into(),
    };
    let text = modal_text(140, 40, &modal);
    assert!(
        text.contains(" ⚡  Actions: Node/worker-1 [Type to filter"),
        "{text}"
    );
    assert!(text.contains(" / stream█"), "{text}");
    assert!(text.contains("▶ [l] View Logs"), "{text}");
    assert!(!text.contains("Ask AI"), "{text}");
    assert!(text.contains("Showing 1/3 actions"), "{text}");

    if let Modal::ActionPalette { filter, .. } = &mut modal {
        *filter = "^D".to_string();
    }
    let text = modal_text(140, 40, &modal);
    assert!(
        text.contains("▶ [^d] Delete"),
        "key hints match case-insensitively: {text}"
    );
    assert!(text.contains("Showing 1/3 actions"), "{text}");
}

// ───────────────────────── dialogs: delegated modals ─────────────────────────

#[test]
fn the_metrics_timeline_modal_is_delegated_to_the_metrics_panel() {
    let modal = Modal::MetricsTimeline(MetricsPanelState::new(
        "Pod".into(),
        "web-0".into(),
        Some("default".into()),
        vec![],
    ));
    let text = modal_text(120, 40, &modal);
    assert!(text.contains("Live Metrics Timeline"), "{text}");
    assert!(text.contains("Collecting metrics-server data"), "{text}");
}

#[test]
fn the_reason_rail_modal_is_delegated_to_the_reason_rail_widget() {
    let modal = Modal::ReasonRail {
        tallies: vec![
            ReasonTally {
                reason: "BackOff".into(),
                count: 3,
                event_type: "Warning".into(),
            },
            ReasonTally {
                reason: "Scheduled".into(),
                count: 1,
                event_type: "Normal".into(),
            },
        ],
        selected_idx: 0,
        active_filter: Some("BackOff".into()),
    };
    let text = modal_text(120, 40, &modal);
    assert!(text.contains("Event Reasons"), "{text}");
    assert!(text.contains("BackOff"), "{text}");
    assert!(text.contains("Scheduled"), "{text}");
}

#[test]
fn every_modal_still_draws_its_frame_on_a_cramped_terminal() {
    for modal in every_modal() {
        for (w, h) in [(30u16, 8u16), (24, 6)] {
            let text = modal_text(w, h, &modal);
            assert!(
                text.contains('┌') || text.contains('─'),
                "{w}x{h} {modal:?}: {text:?}"
            );
        }
        // Too small for any frame at all: must simply not panic.
        modal_text(12, 4, &modal);
    }
}

// ───────────────────────── help ─────────────────────────

#[test]
fn centered_rect_carves_a_proportional_rectangle_out_of_the_middle() {
    let r = centered_rect(50, 50, Rect::new(0, 0, 100, 40));
    assert_eq!(r, Rect::new(25, 10, 50, 20));
    let tiny = centered_rect(50, 50, Rect::new(0, 0, 1, 1));
    assert!(tiny.width <= 1 && tiny.height <= 1);
}

#[test]
fn the_help_modal_lists_every_section_and_its_key_bindings() {
    let text = common::render_text(160, 60, |f| render_help_modal(f, f.area()));
    assert!(
        text.contains(" SRElens & k9s Keybindings Cheat Sheet (Press Esc to close) "),
        "{text}"
    );
    for section in [
        "Navigation & Views",
        "Resource Actions",
        "SRElens Superpowers",
    ] {
        assert!(text.contains(section), "missing section {section}: {text}");
    }
    for (key, desc) in [
        (": <command>", "Open command prompt"),
        ("Ctrl+c", "Exit / kill srelens-tui immediately"),
        ("/ <filter>", "Filter table rows by substring or regex"),
        ("j / k / ↑ / ↓", "Navigate up / down"),
        ("g / G", "Jump to top / bottom"),
        ("Ctrl+u / Ctrl+d", "Half-page scroll"),
        ("Enter", "Drill down"),
        ("Esc", "Pop view back"),
        (
            "Ctrl+a",
            "Toggle between active namespace and all namespaces",
        ),
        ("Space", "Mark / select item"),
        ("x", "Quick Actions & Incident Palette"),
        ("t", "Resource Relationship Tree"),
        ("l", "View Logs"),
        ("s", "Interactive Shell"),
        ("y / v", "View YAML manifest"),
        ("d", "Describe resource"),
        ("e", "Edit resource YAML in $EDITOR"),
        ("Ctrl+d", "Delete selected resource"),
        ("Ctrl+r", "Rollout restart workload"),
        ("Ctrl+s", "Scale workload replica count"),
        ("Shift+f / Ctrl+f", "Start Port Forwarding"),
        ("c", "Copy resource name"),
        ("C / Shift+c", "Copy resource full YAML"),
        ("Ctrl+y", "Copy resource srelens:// deep link URL"),
        ("Mouse drag", "Select text in any view"),
        ("c / u (Nodes)", "Cordon / Drain Node"),
        ("Tab / :ai", "Open SRElens AI Assistant"),
        (":helm", "Helm 3 release management"),
        (":tb", "Toolbox diagnostics"),
        (":pf", "Port Forwards manager"),
        (
            "F1 - F10 / :ctx",
            "Quick-switch Kubernetes cluster contexts",
        ),
    ] {
        let row = text
            .lines()
            .find(|l| l.contains(key) && l.contains(desc))
            .unwrap_or_else(|| panic!("no row pairs {key:?} with {desc:?}:\n{text}"));
        assert!(
            row.find(key).unwrap() < row.find(desc).unwrap(),
            "key column precedes description: {row}"
        );
    }
}

#[test]
fn the_help_modal_truncates_from_the_bottom_when_the_terminal_is_short() {
    let text = common::render_text(120, 24, |f| render_help_modal(f, f.area()));
    assert!(text.contains("Cheat Sheet"), "{text}");
    assert!(text.contains("Navigation & Views"), "{text}");
    assert!(text.contains(": <command>"), "{text}");
    assert!(
        !text.contains("SRElens Superpowers"),
        "later sections fall off a 24-row terminal: {text}"
    );
    assert!(!text.contains("F1 - F10"), "{text}");
}

#[test]
fn the_help_modal_degrades_to_a_border_on_a_tiny_terminal() {
    let text = common::render_text(20, 6, |f| render_help_modal(f, f.area()));
    assert!(
        (text.contains('┌') && text.contains('┘')) || (text.contains('╭') && text.contains('╰')),
        "{text:?}"
    );
}

// ───────────────────────── statusbar ─────────────────────────

#[test]
fn command_mode_echoes_the_prompt_with_a_cursor_and_no_popup_without_suggestions() {
    let mode = InputMode::Command;
    let mut props = status_props(&mode);
    props.command_input = "po";
    let text = statusbar_text(props);
    assert!(text.contains(":po█"), "{text}");
    assert!(!text.contains("Commands (Tab to complete"), "{text}");

    let mode = InputMode::Command;
    let mut props = status_props(&mode);
    props.command_input = "zzz";
    let empty: Vec<(srelens_tui::commands::DynamicCommandDef, usize)> = Vec::new();
    props.suggestions = Some((&empty, 0));
    let text = statusbar_text(props);
    assert!(text.contains(":zzz█"), "{text}");
    assert!(
        !text.contains("Commands (Tab to complete"),
        "an empty suggestion list draws no popup: {text}"
    );
}

#[test]
fn command_mode_pops_up_the_matching_commands_above_the_bar_and_arrows_the_selection() {
    let suggs = command_suggestions("po");
    assert!(
        suggs.len() >= 2,
        "the registry should offer several matches for 'po'"
    );
    let selected = 1;
    let mode = InputMode::Command;
    let mut props = status_props(&mode);
    props.command_input = "po";
    props.suggestions = Some((&suggs, selected));
    let lines = common::render_lines(80, 24, |f| {
        render_statusbar(f, Rect::new(0, 22, 80, 2), props)
    });
    let text = lines.join("\n");
    assert!(
        text.contains("Commands") && text.contains("Tab") && text.contains("Enter"),
        "{text}"
    );

    let sel_name = &suggs[selected].0.name;
    let sel_row = lines
        .iter()
        .find(|l| l.contains("▶ "))
        .expect("a selected row");
    assert!(
        sel_row.contains(sel_name.as_str()),
        "selected row {sel_row:?} names {sel_name}"
    );
    assert!(
        sel_row.contains(&suggs[selected].0.description),
        "{sel_row}"
    );
    let first = &suggs[0].0;
    let first_row = lines
        .iter()
        .find(|l| l.contains(first.name.as_str()) && !l.contains("▶ "))
        .expect("unselected first row");
    if !first.aliases.is_empty() {
        assert!(
            first_row.contains(&format!("({})", first.aliases.join(", "))),
            "{first_row}"
        );
    }
    let popup_row = lines
        .iter()
        .position(|l| l.contains("Commands"))
        .unwrap();
    assert!(popup_row < 22, "popup opens above the bar");
    assert!(lines[23].starts_with(":po█"), "{:?}", lines[23]);
}

#[test]
fn regex_filter_mode_shows_the_match_ratio_and_apply_hint() {
    let mode = InputMode::Filter;
    let mut props = status_props(&mode);
    props.filter_input = "web-.*";
    props.matched_count = 3;
    props.total_count = 10;
    let text = statusbar_text(props);
    assert!(
        text.contains("Filter (regex): /web-.*█  [3/10]  (Enter to apply, Esc to clear)"),
        "{text}"
    );
}

#[test]
fn text_search_mode_shows_a_match_count_and_next_prev_hint() {
    let mode = InputMode::Filter;
    let mut props = status_props(&mode);
    props.filter_input = "OOM";
    props.matched_count = 4;
    props.total_count = 900;
    props.is_text_search = true;
    let text = statusbar_text(props);
    assert!(
        text.contains("Search: /OOM█  [4 matches]  (Enter to finish, n/N next/prev, Esc to clear)"),
        "{text}"
    );
    assert!(!text.contains("900"), "{text}");
}

#[test]
fn normal_mode_shows_the_default_key_palette() {
    let mode = InputMode::Normal;
    let text = statusbar_text(status_props(&mode));
    for hint in [
        "<:> Cmd",
        "</> Filter",
        "<c> CopyURL",
        "<l> Logs",
        "<s> Shell",
        "<f>/<F> PortForward",
        "<d> Describe",
    ] {
        assert!(text.contains(hint), "missing {hint}: {text}");
    }
    assert!(!text.contains("Filter:"), "{text}");
    assert!(!text.contains('➜'), "{text}");
}

#[test]
fn normal_mode_prefixes_a_toast_and_appends_the_active_filter() {
    let mode = InputMode::Normal;
    let mut props = status_props(&mode);
    props.toast = Some(("Copied web-0", Style::default()));
    props.filter_input = "web";
    props.matched_count = 2;
    props.total_count = 9;
    let text = statusbar_text(props);
    assert!(text.starts_with("─"), "{text}");
    assert!(
        text.contains("Copied web-0 │ <:> Cmd") && (text.contains('➜') || text.contains('●')),
        "{text}"
    );
    assert!(text.contains(" | Filter: \"web\" [2/9]"), "{text}");
}

#[test]
fn normal_mode_uses_custom_hints_when_a_view_supplies_them() {
    let mode = InputMode::Normal;
    let mut props = status_props(&mode);
    let hints: &[(&str, &str)] = &[("<Esc>", "Back"), ("<w>", "Wrap")];
    props.custom_hints = Some(hints);
    let text = statusbar_text(props);
    assert!(text.contains("<Esc> Back <w> Wrap"), "{text}");
    assert!(!text.contains("<:> Cmd"), "{text}");
}

// ───────────────────────── header ─────────────────────────

#[test]
fn a_two_row_header_shows_brand_chips_stats_and_the_active_context_line() {
    let contexts = vec![
        chip(1, "prod-east", true, false),
        chip(2, "dev-local", false, true),
    ];
    let props = header_props(&contexts);
    let lines = common::render_lines(120, 3, |f| render_header(f, Rect::new(0, 0, 120, 3), props));
    assert!(lines[0].starts_with("⚡  SRELENS"), "{:?}", lines[0]);
    assert!(
        lines[0].contains("[● 1: prod-east] [2: dev-local]"),
        "{:?}",
        lines[0]
    );
    assert!(
        lines[0].ends_with("● v1.29.2 Nodes: 3 Pods: 42"),
        "{:?}",
        lines[0]
    );
    assert!(
        lines[1].starts_with("Ctx: prod-east  NS: [default]  View: Pods"),
        "{:?}",
        lines[1]
    );
    assert!(
        lines[1].contains("<:ctx> All Ctx  <F1-F10> Hotkeys"),
        "{:?}",
        lines[1]
    );
    assert!(lines[2].starts_with("───"), "bottom border: {:?}", lines[2]);
}

#[test]
fn an_empty_namespace_reads_as_all_and_a_bare_version_gains_a_v_prefix() {
    let mut props = header_props(&[]);
    props.namespace = "";
    props.version = "1.30.1";
    props.is_connected = false;
    let text = header_text(120, 3, props);
    assert!(text.contains("NS: [all]"), "{text}");
    assert!(text.contains("● v1.30.1 Nodes: 3 Pods: 42"), "{text}");

    let mut props = header_props(&[]);
    props.version = "unknown";
    let text = header_text(120, 3, props);
    assert!(
        text.contains("● unknown Nodes: 3"),
        "a version with no dots is left alone: {text}"
    );
}

#[test]
fn context_chips_record_their_click_rects_and_collapse_into_an_overflow_chip() {
    let contexts = vec![
        chip(1, "alpha-cluster-01", true, false),
        chip(2, "bravo-cluster-02", false, false),
        chip(3, "charlie-cluster-03", false, false),
    ];
    let rects = RefCell::new(Vec::new());
    let mut props = header_props(&contexts);
    props.context = "alpha-cluster-01";
    props.context_chip_rects = Some(&rects);
    // The chip column runs from x=11 (after the brand) to width-34 (before
    // the stats). At width 106 that is x=11..72: the 23-wide current chip and
    // the 21-wide second chip fit, the 23-wide third does not, and the
    // 11-wide overflow marker takes its place.
    let lines = common::render_lines(106, 3, |f| render_header(f, Rect::new(0, 0, 106, 3), props));
    assert!(
        lines[0].contains("[● 1: alpha-cluster-01] [2: bravo-cluster-02] [+1 (:ctx)]"),
        "{:?}",
        lines[0]
    );
    assert!(!lines[0].contains("charlie"), "{:?}", lines[0]);

    let recorded = rects.borrow();
    let names: Vec<&str> = recorded.iter().map(|(_, n)| n.as_str()).collect();
    assert_eq!(names, vec!["alpha-cluster-01", "bravo-cluster-02", ":ctx"]);
    assert_eq!(recorded[0].0, Rect::new(11, 0, 23, 1));
    assert_eq!(recorded[1].0, Rect::new(35, 0, 21, 1));
    assert_eq!(recorded[2].0, Rect::new(57, 0, 11, 1));
}

#[test]
fn a_chip_that_does_not_fit_takes_the_overflow_marker_with_it() {
    let contexts = vec![
        chip(1, "alpha-cluster-01", true, false),
        chip(2, "bravo-cluster-02", false, false),
        chip(3, "charlie-cluster-03", false, false),
    ];
    let props = header_props(&contexts);
    // Four columns narrower and the overflow marker no longer fits either, so
    // the row stops after the two chips that do.
    let lines = common::render_lines(100, 3, |f| render_header(f, Rect::new(0, 0, 100, 3), props));
    assert!(
        lines[0].contains("[● 1: alpha-cluster-01] [2: bravo-cluster-02]"),
        "{:?}",
        lines[0]
    );
    assert!(
        !lines[0].contains("charlie") && !lines[0].contains("(:ctx)"),
        "{:?}",
        lines[0]
    );
}

#[test]
fn the_overflow_chip_is_dropped_when_even_it_would_not_fit() {
    let contexts = vec![
        chip(1, "production-cluster-eu-west", true, false),
        chip(2, "staging", false, false),
    ];
    let rects = RefCell::new(vec![(Rect::default(), "stale".to_string())]);
    let mut props = header_props(&contexts);
    props.context_chip_rects = Some(&rects);
    // Width 80 leaves 35 columns: the 31-wide first chip fits, then only 4
    // columns remain, too few for "[+1 (:ctx)]".
    let lines = common::render_lines(80, 3, |f| render_header(f, Rect::new(0, 0, 80, 3), props));
    assert!(
        lines[0].contains("[● 1: production-cluster-eu-west]"),
        "{:?}",
        lines[0]
    );
    assert!(!lines[0].contains("staging"), "{:?}", lines[0]);
    assert!(!lines[0].contains("(:ctx)"), "{:?}", lines[0]);
    let recorded = rects.borrow();
    assert_eq!(
        recorded.len(),
        1,
        "stale rects are cleared before drawing: {recorded:?}"
    );
    assert_eq!(recorded[0].1, "production-cluster-eu-west");
}

#[test]
fn chips_are_skipped_entirely_when_there_are_none_or_no_room_for_them() {
    // No contexts to draw: the rest of the header still renders.
    let rects = RefCell::new(Vec::new());
    let mut props = header_props(&[]);
    props.context_chip_rects = Some(&rects);
    let text = header_text(120, 3, props);
    assert!(text.starts_with("⚡  SRELENS"), "{text}");
    assert!(text.contains("Ctx: prod-east"), "{text}");
    assert!(!text.contains(" (:ctx)]"), "{text}");
    assert!(rects.borrow().is_empty());

    // A header only a few columns wide: the chip strip is under five cells, so
    // nothing is drawn there and no click target is recorded.
    let contexts = vec![chip(1, "prod", true, false)];
    let rects = RefCell::new(Vec::new());
    let mut props = header_props(&contexts);
    props.context_chip_rects = Some(&rects);
    let text = header_text(4, 3, props);
    assert!(!text.contains("prod"), "{text}");
    assert!(rects.borrow().is_empty());
}

#[test]
fn a_single_row_header_collapses_everything_onto_one_line() {
    let contexts = vec![chip(1, "prod-east", true, false)];
    let mut props = header_props(&contexts);
    props.namespace = "";
    props.version = "1.28.0";
    let lines = common::render_lines(120, 2, |f| render_header(f, Rect::new(0, 0, 120, 2), props));
    assert!(lines[0].starts_with("⚡  SRELENS"), "{:?}", lines[0]);
    assert!(
        lines[0].contains("Ctx: prod-east NS: [all] View: Pods"),
        "{:?}",
        lines[0]
    );
    assert!(
        lines[0].ends_with("● v1.28.0 Nodes: 3 Pods: 42"),
        "{:?}",
        lines[0]
    );
    assert!(
        !lines[0].contains("[● 1: prod-east]"),
        "no chip row in compact mode: {:?}",
        lines[0]
    );
    assert!(lines[1].starts_with("───"), "{:?}", lines[1]);
}

// ───────────────────────── assistant: title bar ─────────────────────────

#[test]
fn the_assistant_title_names_the_provider_model_and_key_hints() {
    let state = fresh_state();
    let text = assistant_text(200, 30, &state);
    let settings = AiSettings::default();
    let model = settings.get_model(settings.default_provider);
    assert!(text.contains(&format!(" SRElens AI Assistant[Anthropic (Claude) - {model}] [<c> Copy, <Ctrl+t> Tools, <Ctrl+e> Save, <Ctrl+l> Clear, <Ctrl+s> Settings, <Esc> Back] ")), "{text}");
    assert!(text.contains("SRElens [10:00:00]:"), "{text}");
    assert!(
        text.contains("Hello! I am your SRElens AI Assistant."),
        "{text}"
    );
    assert!(text.contains(" Ask Assistant (Type '/' for SRE Playbooks, ↑/↓ History, <c> Copy, <Ctrl+s> Settings) "), "{text}");
}

#[test]
fn the_assistant_title_reflects_context_caveman_tokens_selection_and_folded_tools() {
    let mut state = AssistantViewState::for_context("prod-east");
    state.messages[0].timestamp = "10:00:00".into();
    assert!(state.messages[0]
        .content
        .contains("for context 'prod-east'"));
    state.caveman_level = Some(CavemanLevel::WenyanUltra);
    state.expand_tools = true;
    state.start_selection(0, 0);
    state.update_selection(0, 3);
    state.finish_selection(0, 3);
    let mut usage_msg = msg("assistant", "done");
    usage_msg.token_usage = Some(TokenUsage {
        total_tokens: 1234,
        ..Default::default()
    });
    state.messages.push(usage_msg);

    let text = assistant_text(220, 30, &state);
    assert!(
        text.contains(
            " SRElens AI Assistant @prod-east  [🦖  CAVEMAN: WENYAN-ULTRA][Anthropic (Claude) - "
        ),
        "{text}"
    );
    assert!(
        text.contains("[⚡  1,234 tokens, <c> Copy Selection, <Ctrl+t> Fold Tools, <Ctrl+e> Save"),
        "{text}"
    );
}

// ───────────────────────── assistant: tool chips ─────────────────────────

fn state_with_tools(expand: bool) -> AssistantViewState {
    let mut state = fresh_state();
    let mut m = msg("assistant", "Found it.");
    m.tool_calls = vec![
        tool(
            "1",
            "srelens-k8s.getObject",
            "pod/web-0",
            ToolCallStatus::Success,
        ),
        tool(
            "2",
            "srelens-k8s.getObject",
            "pod/web-1",
            ToolCallStatus::Success,
        ),
        tool(
            "3",
            "srelens-k8s.getObject",
            "pod/web-2",
            ToolCallStatus::Success,
        ),
        tool("4", "k8s_listPods", "", ToolCallStatus::Success),
        tool("5", "hookAdditionalContexts", "", ToolCallStatus::Success),
        tool("6", "bash", &"k".repeat(70), ToolCallStatus::Success),
    ];
    state.messages.push(m);
    state.expand_tools = expand;
    state
}

#[test]
fn a_collapsed_tool_chip_groups_repeated_tools_and_hides_internal_ones() {
    let state = state_with_tools(false);
    let text = assistant_text(160, 30, &state);
    assert!(text.contains("▶ ⚙ 5 tools queried: getObject (×3), listPods, bash [✓ ok]  (click or <Ctrl+t> to expand)"), "{text}");
    assert!(!text.contains("hookAdditionalContexts"), "{text}");
    assert!(
        !text.contains("pod/web-0"),
        "collapsed chips hide per-tool args: {text}"
    );
    let chip_lines = state.tool_chip_lines.borrow();
    assert_eq!(
        chip_lines.as_slice(),
        &[4],
        "the chip sits right under the second message header: {chip_lines:?}"
    );
}

#[test]
fn an_expanded_tool_chip_lists_each_call_and_truncates_long_arguments() {
    let state = state_with_tools(true);
    let text = assistant_text(160, 30, &state);
    assert!(
        text.contains("▼ ⚙ 5 tools queried (click or <Ctrl+t> to collapse): [✓ ok]"),
        "{text}"
    );
    assert!(text.contains("⚙ getObject pod/web-0 [✓ ok]"), "{text}");
    assert!(text.contains("⚙ getObject pod/web-2 [✓ ok]"), "{text}");
    assert!(
        text.contains("⚙ listPods [✓ ok]"),
        "empty args add no preview: {text}"
    );
    assert!(
        text.contains(&format!("⚙ bash {}... [✓ ok]", "k".repeat(62))),
        "{text}"
    );
    assert!(!text.contains(&"k".repeat(63)), "{text}");
}

#[test]
fn the_overall_tool_badge_prefers_running_over_error_over_ok() {
    let mut state = fresh_state();
    let mut m = msg("assistant", "");
    m.tool_calls = vec![
        tool("1", "bash", "ls", ToolCallStatus::Error("boom".into())),
        tool("2", "read", "f", ToolCallStatus::Running),
    ];
    state.messages.push(m);
    state.expand_tools = true;
    let text = assistant_text(160, 30, &state);
    assert!(
        text.contains("2 tools queried (click or <Ctrl+t> to collapse): [⠋ running]"),
        "{text}"
    );
    assert!(text.contains("⚙ bash ls [✗ error]"), "{text}");
    assert!(text.contains("⚙ read f [⠋ running]"), "{text}");

    state.finish_tool_call("2", ToolCallStatus::Success);
    state.expand_tools = false;
    let text = assistant_text(160, 30, &state);
    assert!(
        text.contains("2 tools queried: bash, read [✗ error]"),
        "{text}"
    );
}

// ───────────────────────── assistant: usage footer, busy line ─────────────────────────

#[test]
fn the_token_footer_formats_thousands_cache_and_duration() {
    let mut state = fresh_state();
    let mut m = msg("assistant", "ok");
    m.token_usage = Some(TokenUsage {
        prompt_tokens: 1000,
        completion_tokens: 500,
        cached_tokens: 200,
        total_tokens: 1500,
        duration_ms: Some(1234),
    });
    state.messages.push(m);
    let text = assistant_text(160, 30, &state);
    assert!(
        text.contains("⚡  1,500 tokens (1,000 prompt, 500 completion • 200 cached) • 1.2s"),
        "{text}"
    );

    state.set_token_usage(TokenUsage {
        prompt_tokens: 10,
        completion_tokens: 5,
        cached_tokens: 0,
        total_tokens: 15,
        duration_ms: Some(850),
    });
    let text = assistant_text(160, 30, &state);
    assert!(
        text.contains("⚡  15 tokens (10 prompt, 5 completion) • 850ms"),
        "{text}"
    );

    state.set_token_usage(TokenUsage {
        prompt_tokens: 1,
        completion_tokens: 1,
        total_tokens: 2,
        ..Default::default()
    });
    let lines = common::render_lines(160, 30, |f| {
        render_assistant_view(f, f.area(), &state, &AiSettings::default())
    });
    // The title bar carries a token hint too, so match the footer's own shape.
    let footer = lines
        .iter()
        .find(|l| l.contains("2 tokens (1 prompt"))
        .expect("footer");
    assert!(
        footer
            .trim_end_matches(['│', ' '])
            .ends_with("(1 prompt, 1 completion)"),
        "no cache and no duration means no badges: {footer:?}"
    );
}

#[test]
fn a_busy_assistant_shows_a_spinner_status_and_elapsed_seconds() {
    let mut state = fresh_state();
    state.is_busy = true;
    state.spinner_frame = 3;
    let text = assistant_text(160, 30, &state);
    assert!(
        text.contains("⠸ Consulting AI provider & cluster state... (0s elapsed)"),
        "{text}"
    );

    // Seed a known age instead of reading the clock and asserting an exact
    // count: on a loaded runner a second can pass between `Instant::now()`
    // and the render, and a CORRECT renderer would then print 8s and fail an
    // equality check. Assert the floor instead — that the elapsed seconds are
    // the seeded age or more, which a renderer stuck at 0 still fails.
    state.set_status("Running kubectl get pods".into());
    state.busy_start = std::time::Instant::now().checked_sub(Duration::from_secs(7));
    assert!(
        state.busy_start.is_some(),
        "the clock must support a 7s offset"
    );
    state.tick();
    let text = assistant_text(160, 30, &state);
    let seconds = elapsed_seconds(&text, "⠼ Running kubectl get pods");
    assert!(seconds >= 7, "expected at least the seeded 7s: {text}");

    state.finish_turn();
    assert!(!state.is_busy && state.busy_start.is_none());
    state.tick();
    assert_eq!(
        state.spinner_frame, 4,
        "the spinner only advances while busy"
    );
}

// ───────────────────────── assistant: scrolling ─────────────────────────

/// True when some row of the frame carries exactly `body` between the
/// assistant block's side borders. Rows are border-terminated, so a plain
/// `contains("question 1\n")` never matches.
fn has_body_row(lines: &[String], body: &str) -> bool {
    lines.iter().any(|l| l.trim_matches('│').trim() == body)
}

fn long_conversation() -> AssistantViewState {
    let mut state = fresh_state();
    for i in 1..=40 {
        state.messages.push(msg("user", &format!("question {i}")));
    }
    state
}

#[test]
fn following_the_bottom_shows_the_newest_message_and_the_default_input_title() {
    let state = long_conversation();
    let lines = common::render_lines(120, 24, |f| {
        render_assistant_view(f, f.area(), &state, &AiSettings::default())
    });
    let text = lines.join("\n");
    assert!(has_body_row(&lines, "question 40"), "{text}");
    assert!(!has_body_row(&lines, "question 1"), "{text}");
    assert!(
        text.contains("Ask Assistant (Type '/' for SRE Playbooks"),
        "{text}"
    );
    assert!(state.last_max_scroll.get() > 0);
    assert_eq!(state.last_total_lines.get(), 1 + 2 + 1 + 40 * 3 + 2, "greeting (header, 2 wrapped content rows at this width, blank) + 40 × (header, content, blank) + 2 breathing rows");
}

#[test]
fn scrolling_up_shows_the_oldest_lines_and_a_line_counter_in_the_input_title() {
    let mut state = long_conversation();
    state.last_max_scroll.set(999);
    state.scroll_to_top();
    let lines = common::render_lines(120, 24, |f| {
        render_assistant_view(f, f.area(), &state, &AiSettings::default())
    });
    let text = lines.join("\n");
    assert!(has_body_row(&lines, "question 1"), "{text}");
    assert!(!text.contains("question 40"), "{text}");
    let total = state.last_total_lines.get();
    assert!(
        text.contains(&format!(
            "Ask Assistant (<End> Follow bottom, <PageUp>/<PageDown> Scroll) [Line 1/{total}]"
        )),
        "{text}"
    );

    state.scroll_down(5);
    assert!(!state.auto_scroll);
    let text = assistant_text(120, 24, &state);
    assert!(text.contains(&format!("[Line 6/{total}]")), "{text}");

    state.scroll_down(10_000);
    assert!(
        state.auto_scroll,
        "scrolling past the end re-engages follow mode"
    );
    let text = assistant_text(120, 24, &state);
    assert!(text.contains("question 40"), "{text}");
    assert!(!text.contains("Follow bottom"), "{text}");
}

#[test]
fn an_explicit_offset_at_or_past_the_end_drops_the_line_counter() {
    let mut state = long_conversation();
    assistant_text(120, 24, &state);
    state.auto_scroll = false;
    state.scroll_offset = state.last_max_scroll.get() + 50;
    let text = assistant_text(120, 24, &state);
    assert!(text.contains("question 40"), "{text}");
    assert!(
        text.contains("Ask Assistant (Type '/' for SRE Playbooks"),
        "{text}"
    );
}

// ───────────────────────── assistant: input box ─────────────────────────

#[test]
fn a_multi_line_prompt_grows_the_input_box_and_keeps_the_cursor_line_visible() {
    let mut state = fresh_state();
    state.input = (1..=8)
        .map(|i| format!("line{i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let lines = common::render_lines(120, 30, |f| {
        render_assistant_view(f, f.area(), &state, &AiSettings::default())
    });
    let text = lines.join("\n");
    assert!(text.contains("line8█"), "{text}");
    assert!(text.contains("line3"), "{text}");
    assert!(
        !text.contains("line1\n") && !text.contains("line2"),
        "the first rows scroll out of an 8-row box: {text}"
    );
    let title_row = lines
        .iter()
        .position(|l| l.contains("Ask Assistant"))
        .unwrap();
    assert_eq!(
        lines.len() - 1 - title_row,
        8,
        "input box is capped at 8 rows (title, 6 text rows, bottom border) inside the outer border"
    );
}

#[test]
fn a_long_single_line_prompt_wraps_inside_the_input_box() {
    let mut state = fresh_state();
    state.input = "a".repeat(150);
    let text = assistant_text(60, 24, &state);
    assert!(text.contains(&"a".repeat(56)), "{text}");
    assert!(text.contains(&format!("{}█", "a".repeat(38))), "{text}");
}

// ───────────────────────── assistant: slash suggestions ─────────────────────────

#[test]
fn typing_a_slash_pops_up_every_playbook_with_the_first_selected() {
    let mut state = fresh_state();
    state.input = "/".into();
    state.update_slash_suggestions();
    let n = state.slash_suggestions.len();
    assert!(n > 7, "enough playbooks to need windowing");
    let text = assistant_text(140, 40, &state);
    assert!(
        text.contains(&format!(
            " ⚡  SRE Playbooks & Slash Commands (1/{n}) [<Tab>/<Enter>: Apply, ↑/↓: Select] "
        )),
        "{text}"
    );
    assert!(
        text.contains(" ▶ /crashloop [Pod] - Triage pod stuck in CrashLoopBackOff"),
        "{text}"
    );
    assert!(text.contains("   /pending"), "{text}");
    assert!(
        text.contains(
            " Ask Assistant (⚡  SRE Playbooks: <Tab>/<Enter> Apply, ↑/↓ Select, <Esc> Dismiss) "
        ),
        "{text}"
    );
    assert!(
        text.contains("   /summarise - "),
        "the seventh row still fits: {text}"
    );
    // The greeting itself names /caveman, so look for its popup row instead.
    assert!(
        !text.contains("/caveman ["),
        "only the first seven playbooks fit the popup: {text}"
    );
    assert!(!text.contains("/clear - "), "{text}");
}

#[test]
fn the_suggestion_popup_windows_around_a_selection_past_the_seventh_row() {
    let mut state = fresh_state();
    state.input = "/".into();
    state.update_slash_suggestions();
    let n = state.slash_suggestions.len();
    // Stepping up from the first row wraps around to the last one.
    state.slash_suggestion_up();
    assert_eq!(state.slash_suggestion_idx, n - 1);
    let text = assistant_text(140, 40, &state);
    assert!(
        text.contains(&format!("Slash Commands ({n}/{n})")),
        "{text}"
    );
    assert!(
        text.contains(" ▶ /caveman [lite|full|ultra|wenyan|off] - "),
        "{text}"
    );
    assert!(text.contains("   /settings - "), "{text}");
    assert!(
        !text.contains("/crashloop"),
        "the window slid past the first rows: {text}"
    );

    // And stepping down from the last row wraps back to the first.
    state.slash_suggestion_down();
    assert_eq!(state.slash_suggestion_idx, 0);
    let text = assistant_text(140, 40, &state);
    assert!(text.contains(&format!("Slash Commands (1/{n})")), "{text}");
    assert!(text.contains(" ▶ /crashloop [Pod] - "), "{text}");

    // A prefix narrows the popup, and it matches a playbook's name as well as
    // its command: "cl" reaches /summarise through its name "cluster-summary".
    state.input = "/cl".into();
    state.update_slash_suggestions();
    let text = assistant_text(140, 40, &state);
    assert!(text.contains("Slash Commands (1/2)"), "{text}");
    assert!(text.contains(" ▶ /summarise - "), "{text}");
    assert!(
        text.contains("   /clear - Clear conversation history"),
        "a utility without a target has no bracket: {text}"
    );
    assert!(!text.contains("/clear ["), "{text}");
    assert!(!text.contains("/crashloop"), "{text}");

    state.slash_suggestion_down();
    assert!(state.apply_selected_slash_suggestion());
    assert_eq!(state.input, "/clear ");
    assert!(state.slash_suggestions.is_empty());
    state.update_slash_suggestions();
    assert!(
        state.slash_suggestions.is_empty(),
        "a completed command with a space offers no suggestions"
    );
    assert!(!state.apply_selected_slash_suggestion());
}

// ───────────────────────── assistant: selection ─────────────────────────

#[test]
fn a_selection_spanning_rows_is_highlighted_and_extracted_from_the_rendered_rows() {
    let mut state = fresh_state();
    state.messages.push(msg("user", "hello world"));
    assistant_text(160, 30, &state);
    {
        let plain = state.plain_lines.borrow();
        assert_eq!(plain[0], "SRElens [10:00:00]:");
        assert_eq!(plain[3], "You [10:00:00]:");
        assert_eq!(plain[4], "hello world");
    }
    state.start_selection(3, 4);
    state.update_selection(4, 5);
    state.finish_selection(4, 5);
    assert_eq!(
        state.get_selected_text().as_deref(),
        Some("[10:00:00]:\nhello")
    );
    let text = assistant_text(160, 30, &state);
    assert!(text.contains("<c> Copy Selection"), "{text}");
    assert!(
        text.contains("hello world"),
        "highlighting keeps the text intact: {text}"
    );

    // Dragging backwards is normalised, so the same text comes out.
    state.start_selection(4, 5);
    state.finish_selection(3, 4);
    assert_eq!(
        state.get_selected_text().as_deref(),
        Some("[10:00:00]:\nhello")
    );
    assistant_text(160, 30, &state);

    // Selecting the whole of a middle row exercises the open-ended column range.
    state.start_selection(0, 2);
    state.finish_selection(4, 3);
    assert_eq!(state.get_selected_text().as_deref(), Some("Elens [10:00:00]:\nHello! I am your SRElens AI Assistant. Type '/' for SRE playbooks or '/caveman' for token compression, or ask any question:\n\nYou [10:00:00]:\nhel"));
    assistant_text(160, 30, &state);

    state.clear_selection();
    assert!(state.selection.is_none() && !state.is_selecting);
}

#[test]
fn a_click_without_a_drag_leaves_no_selection() {
    let mut state = fresh_state();
    state.start_selection(2, 7);
    assert!(state.is_selecting);
    state.finish_selection(2, 7);
    assert!(!state.is_selecting);
    assert!(state.selection.is_none());
    assert!(state.get_selected_text().is_none());

    state.update_selection(5, 5);
    assert!(
        state.selection.is_none(),
        "moving without a press does nothing"
    );
}

// ───────────────────────── assistant: conversation state ─────────────────────────

#[test]
fn clearing_a_conversation_resets_everything_and_keeps_the_context_in_the_greeting() {
    let mut state = AssistantViewState::for_context("stage");
    state.start_turn("why is web-0 crashing?".into());
    state.append_stream_chunk("Because");
    state.scroll_up(3);
    state.set_status("thinking".into());
    state.input = "draft".into();
    state.clear_conversation();
    assert_eq!(state.messages.len(), 1);
    assert!(state.messages[0].content.starts_with(
        "Hello! I am your SRElens AI Assistant for context 'stage'. I can analyze pod crashes"
    ));
    assert!(state.input.is_empty() && !state.is_busy && state.busy_status.is_empty());
    assert!(state.auto_scroll && state.scroll_offset == 0);

    let mut plain = AssistantViewState::new();
    plain.clear_conversation();
    assert!(plain.messages[0]
        .content
        .starts_with("Hello! I am your SRElens AI Assistant. I can analyze pod crashes"));
}

#[test]
fn exporting_to_markdown_records_roles_tools_statuses_and_usage() {
    let mut state = fresh_state();
    state.messages.push(msg("user", "why?"));
    let mut reply = msg("assistant", "Because of OOM.");
    reply.timestamp = String::new();
    reply.tool_calls = vec![
        tool("1", "bash", "kubectl top", ToolCallStatus::Running),
        tool("2", "read", "pod.yaml", ToolCallStatus::Success),
        tool("3", "exec", "sh", ToolCallStatus::Error("exit 1".into())),
        tool("4", "getMcpTools", "", ToolCallStatus::Success),
    ];
    reply.token_usage = Some(TokenUsage {
        prompt_tokens: 2000,
        completion_tokens: 100,
        cached_tokens: 1500,
        total_tokens: 2100,
        duration_ms: Some(4200),
    });
    state.messages.push(reply);
    let mut note = msg("system", "   ");
    note.token_usage = Some(TokenUsage {
        total_tokens: 7,
        duration_ms: Some(12),
        ..Default::default()
    });
    state.messages.push(note);

    let md = state.export_to_markdown("Anthropic", "claude-x");
    assert!(
        md.starts_with("# SRElens AI Assistant Conversation Export\n\n- **Exported At**: "),
        "{md}"
    );
    assert!(
        md.contains("- **Provider**: Anthropic\n- **Model**: claude-x\n\n---\n\n"),
        "{md}"
    );
    assert!(
        md.contains("### 🤖 SRElens Assistant [10:00:00]\n\nHello!"),
        "{md}"
    );
    assert!(md.contains("### 👤 You [10:00:00]\n\nwhy?\n\n---"), "{md}");
    assert!(md.contains("### 🤖 SRElens Assistant\n\n#### Executed Tools:\n- `bash`: `kubectl top` [running]\n- `read`: `pod.yaml` [ok]\n- `exec`: `sh` [exit 1]\n\nBecause of OOM.\n\n*⚡ 2,100 tokens (2,000 prompt, 100 completion • 1,500 cached) • 4.2s*\n\n---"), "{md}");
    assert!(!md.contains("getMcpTools"), "{md}");
    assert!(
        md.contains(
            "### ℹ️ System [10:00:00]\n\n*⚡ 7 tokens (0 prompt, 0 completion) • 12ms*\n\n---"
        ),
        "blank content is skipped: {md}"
    );
}

#[test]
fn saving_a_conversation_to_an_explicit_path_writes_the_markdown_there() {
    let state = fresh_state();
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("chrome_tests");
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let path = dir.join("export.md");
    let written = state
        .save_conversation_to_file("Anthropic", "claude-x", Some(path.to_str().unwrap()))
        .expect("save");
    assert_eq!(written, path);
    let body = std::fs::read_to_string(&path).expect("read back");
    assert!(body.contains("- **Model**: claude-x"), "{body}");
    assert!(
        body.contains("Hello! I am your SRElens AI Assistant."),
        "{body}"
    );
    let _ = std::fs::remove_file(&path);

    let missing = dir.join("no-such-dir").join("export.md");
    let err = state
        .save_conversation_to_file("p", "m", Some(missing.to_str().unwrap()))
        .unwrap_err();
    assert!(err.starts_with("Failed to save conversation: "), "{err}");
}

// ───────────────────────── assistant: markdown blocks ─────────────────────────

#[test]
fn headings_of_every_depth_get_their_own_marker() {
    let lines = md("# Title\n## Section\n### Point\n#### Detail\nplain");
    assert_eq!(
        lines,
        vec![
            "",
            "  ▌ Title",
            "",
            "  ▌ Section",
            "  ● Point",
            "    Detail",
            "plain"
        ]
    );
}

#[test]
fn horizontal_rules_blockquotes_and_lists_are_rendered_with_glyphs() {
    let lines = md("---\n***\n> quoted **loud**\n- top\n  - nested\n* star\n+ plus\n1. first\n12. twelfth\nv1. not a list\nend.");
    assert!(lines[0].starts_with("  ──────"), "{lines:?}");
    assert!(lines[1].starts_with("  ──────"), "{lines:?}");
    assert_eq!(lines[2], "  ▎ quoted loud");
    assert_eq!(lines[3], "  • top");
    assert_eq!(lines[4], "    ◦ nested");
    assert_eq!(lines[5], "  • star");
    assert_eq!(lines[6], "  • plus");
    assert_eq!(lines[7], "  1. first");
    assert_eq!(lines[8], "  12. twelfth");
    assert_eq!(lines[9], "v1. not a list");
    assert_eq!(lines[10], "end.");
}

#[test]
fn a_dash_line_with_other_characters_is_not_a_rule() {
    let lines = md("--- not a rule\n---");
    assert_eq!(lines[0], "--- not a rule");
    assert!(lines[1].starts_with("  ─────"), "{lines:?}");
}

#[test]
fn fenced_code_blocks_are_framed_with_the_language_and_colour_comments_and_keys() {
    let lines = md("```yaml\n# a comment\nreplicas: 3\n  // c-style\nhttp://example.com:8080/x\nplain text\n```\nafter");
    assert_eq!(
        lines[0],
        format!("  ┌── yaml {}", "─".repeat(65 - "── yaml ".len()))
    );
    assert_eq!(lines[1], "  │ # a comment");
    assert_eq!(lines[2], "  │ replicas: 3");
    assert_eq!(lines[3], "  │   // c-style");
    assert_eq!(lines[4], "  │ http://example.com:8080/x");
    assert_eq!(lines[5], "  │ plain text");
    assert_eq!(lines[6], format!("  └{}", "─".repeat(66)));
    assert_eq!(lines[7], "after");

    let mut out = Vec::new();
    format_message_content(&mut out, "```yaml\nkey: value\n```");
    let key_line = &out[1];
    let contents: Vec<&str> = key_line.spans.iter().map(|s| s.content.as_ref()).collect();
    assert_eq!(
        contents,
        vec!["  ", "│ ", "key:", " value"],
        "key and value are separate spans"
    );
}

#[test]
fn an_unterminated_or_language_less_fence_still_closes_its_frame() {
    let lines = md("```\necho hi");
    assert!(lines[0].starts_with("  ┌── code ─"), "{lines:?}");
    assert_eq!(lines[1], "  │ echo hi");
    assert!(lines[2].starts_with("  └──"), "{lines:?}");
    assert_eq!(lines.len(), 3);
}

// ───────────────────────── assistant: markdown tables ─────────────────────────

fn table_lines(header: &str, rows: &[&str], width: Option<usize>) -> Vec<String> {
    let mut out = Vec::new();
    render_markdown_table(&mut out, header, rows, width);
    out.iter().map(line_text).collect()
}

#[test]
fn a_markdown_table_is_boxed_with_cleaned_cells() {
    let lines = table_lines(
        "| `Name` | 'Status' | \"Mem\" |",
        &["| web-0 | Ready | 512MiB |", "| data-1 | 3.5 |"],
        None,
    );
    assert_eq!(lines[0], "  ┌────────┬────────┬────────┐");
    assert_eq!(lines[1], "  │ Name   │ Status │ Mem    │");
    assert_eq!(lines[2], "  ├────────┼────────┼────────┤");
    assert_eq!(lines[3], "  │ web-0  │ Ready  │ 512MiB │");
    assert_eq!(
        lines[4], "  │ data-1 │ 3.5    │        │",
        "a short row is padded with empty cells"
    );
    assert_eq!(lines[5], "  └────────┴────────┴────────┘");
}

#[test]
fn a_narrow_width_shrinks_the_widest_column_and_wraps_its_cells() {
    let header = "| Pod | Message |";
    let rows = ["| api-0 | container restarted, exit code 137/OOM-killed by kernel |"];
    let wide = table_lines(header, &rows, None);
    assert!(
        wide[3].contains("container restarted, exit code 137/OOM-killed by kernel"),
        "{wide:?}"
    );
    assert_eq!(wide.len(), 5);

    let narrow = table_lines(header, &rows, Some(40));
    assert!(
        narrow.len() > 5,
        "the message cell wraps across rows: {narrow:?}"
    );
    let widest = narrow.iter().map(|l| l.chars().count()).max().unwrap();
    assert!(widest <= 40, "{narrow:?}");
    assert!(narrow[3].starts_with("  │ api-0 │ container"), "{narrow:?}");
    assert!(
        narrow[4].starts_with("  │       │ "),
        "continuation rows leave the first cell blank: {narrow:?}"
    );
    let joined: String = narrow[3..narrow.len() - 1]
        .iter()
        .map(|l| l.split('│').nth(2).unwrap().trim().to_string())
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(
        joined,
        "container restarted, exit code 137/OOM-killed by kernel"
    );
}

#[test]
fn a_hopelessly_narrow_width_leaves_the_columns_at_their_minimum() {
    let lines = table_lines("| a | b | c |", &["| x | y | z |"], Some(5));
    assert_eq!(
        lines[1], "  │ a   │ b   │ c   │",
        "columns keep their 3-cell floor when the budget is below 4 per column"
    );
}

#[test]
fn an_unbreakable_cell_is_split_at_the_column_width() {
    let lines = table_lines("| Id |", &["| abcdefghijklmnopqrstuvwxyz |"], Some(20));
    let cells: Vec<String> = lines[3..lines.len() - 1]
        .iter()
        .map(|l| l.split('│').nth(1).unwrap().trim().to_string())
        .collect();
    assert!(cells.len() >= 2, "{lines:?}");
    assert_eq!(cells.concat(), "abcdefghijklmnopqrstuvwxyz");

    let with_blank = table_lines("| Note |", &["| first\n\nsecond |"], None);
    assert!(with_blank.len() >= 5, "{with_blank:?}");
}

#[test]
fn a_table_inside_a_message_is_detected_and_a_lone_pipe_row_is_not() {
    let lines = md("Summary:\n| Pod | Ready |\n|-----|:-----:|\n| web-0 | True |\n| web-1 | False |\ndone\n| orphan |");
    assert_eq!(lines[0], "Summary:");
    assert!(lines[1].starts_with("  ┌"), "{lines:?}");
    assert!(lines[2].contains("│ Pod   │ Ready │"), "{lines:?}");
    assert!(lines[4].contains("│ web-0 │ True  │"), "{lines:?}");
    assert!(lines[5].contains("│ web-1 │ False │"), "{lines:?}");
    assert!(lines[6].starts_with("  └"), "{lines:?}");
    assert_eq!(lines[7], "done");
    assert_eq!(
        lines[8], "| orphan |",
        "a pipe row with no separator beneath it is plain text"
    );

    let mut out = Vec::new();
    format_message_content_with_width(&mut out, "| A | B |\n|---|---|\n| 1 | 2 |", Some(200));
    assert!(line_text(&out[0]).starts_with("  ┌"));
}

#[test]
fn table_rows_are_never_wrapped_but_long_words_are_split_at_the_width() {
    let boxed = Line::from("  │ a very long table row that would otherwise wrap around │");
    assert_eq!(wrap_line(boxed, 10).len(), 1);

    let word = Line::from("x".repeat(45));
    let wrapped: Vec<String> = wrap_line(word, 20).iter().map(line_text).collect();
    assert_eq!(wrapped, vec!["x".repeat(20), "x".repeat(20), "x".repeat(5)]);

    let mixed: Vec<String> = wrap_line(Line::from(format!("short {} tail", "y".repeat(30))), 12)
        .iter()
        .map(line_text)
        .collect();
    assert_eq!(
        mixed,
        vec!["short ", "yyyyyyyyyyyy", "yyyyyyyyyyyy", "yyyyyy tail"]
    );

    assert_eq!(wrap_line(Line::from("untouched"), 0).len(), 1);
    assert_eq!(line_text(&wrap_line(Line::from(""), 5)[0]), "");
}

// ───────────────────────── assistant: inline markdown ─────────────────────────

fn span_texts(input: &str) -> Vec<String> {
    parse_inline_markdown(input)
        .iter()
        .map(|s| s.content.to_string())
        .collect()
}

#[test]
fn inline_markdown_splits_bold_italic_links_strikethrough_and_code_into_spans() {
    assert_eq!(
        span_texts("see ***both*** and **bold** and *it* and [docs](https://k8s.io) and ~~gone~~ and `code` end"),
        vec![
            "see ", "both", " and ", "bold", " and ", "it", " and ", "docs", " (https://k8s.io)", " and ", "gone", " and ", "code", " end",
        ]
    );
}

#[test]
fn unterminated_inline_markers_consume_to_the_end_of_the_line() {
    assert_eq!(
        span_texts("***open"),
        vec!["op", "en"],
        "the bold-italic scanner stops two chars early when unterminated"
    );
    assert_eq!(
        span_texts("**open"),
        vec!["ope", "n"],
        "the bold scanner stops one char early"
    );
    assert_eq!(span_texts("*open"), vec!["open"]);
    assert_eq!(span_texts("~~open"), vec!["ope", "n"]);
    assert_eq!(span_texts("`open"), vec!["open"]);
}

#[test]
fn brackets_without_a_url_and_escaped_asterisks_stay_literal() {
    assert_eq!(span_texts("[not a link] here"), vec!["[not a link] here"]);
    assert_eq!(span_texts("[label](no close"), vec!["[label](no close"]);
    assert_eq!(span_texts("[label] (spaced)"), vec!["[label] (spaced)"]);
    assert_eq!(span_texts("2 \\* 3 = 6"), vec!["2 \\* 3 = 6"]);
    assert_eq!(span_texts("a~b"), vec!["a~b"]);
}

// ───────────────────────── through the App ─────────────────────────

#[tokio::test]
async fn the_app_draws_the_help_overlay_and_closes_it_on_escape() {
    let (mut app, _rx) = common::app().await;
    app.show_help = true;
    let text = common::render_app(&mut app, 160, 60);
    assert!(text.contains("Keybindings Cheat Sheet"), "{text}");
    assert!(text.contains("SRElens Superpowers"), "{text}");
    app.handle_key_event(common::key(KeyCode::Esc)).await;
    assert!(!app.show_help);
    let text = common::render_app(&mut app, 160, 60);
    assert!(!text.contains("Keybindings Cheat Sheet"), "{text}");
}

#[tokio::test]
async fn the_app_draws_whatever_modal_is_open_over_the_current_view() {
    let (mut app, _rx) = common::app().await;
    app.modal = Some(confirm(true));
    let text = common::render_app(&mut app, 120, 40);
    assert!(text.contains(" Delete Pod "), "{text}");
    assert!(text.contains("[Enter/y] to Delete"), "{text}");

    app.modal = Some(Modal::Scale {
        workload_name: "web".into(),
        current_replicas: 2,
        input: "4".into(),
    });
    let text = common::render_app(&mut app, 120, 40);
    assert!(text.contains(" Scale Workload: web "), "{text}");
    assert!(text.contains("4█"), "{text}");
}

#[tokio::test]
async fn the_app_renders_the_assistant_view_with_the_active_context_in_its_title() {
    let (mut app, _rx) = common::app().await;
    app.active_view = ActiveView::Assistant;
    app.assistant_state.caveman_level = Some(CavemanLevel::Lite);
    let text = common::render_app(&mut app, 200, 40);
    assert!(
        text.contains(" SRElens AI Assistant @test-cluster  [🦖  CAVEMAN: LITE]["),
        "{text}"
    );
    assert!(text.contains("for context 'test-cluster'"), "{text}");
    common::type_str(&mut app, "/oo").await;
    let text = common::render_app(&mut app, 200, 40);
    assert!(
        text.contains("/oom"),
        "typing a slash prefix opens the playbook popup: {text}"
    );
    assert!(text.contains("/oo█"), "{text}");
}
