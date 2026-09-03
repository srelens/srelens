use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Table, Row, Cell},
    Frame,
};

use crate::theme::Theme;

/// Render centered modal rect helper
pub fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

pub fn render_help_modal(f: &mut Frame, area: Rect) {
    let modal_area = centered_rect(75, 80, area);
    f.render_widget(Clear, modal_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::ACCENT))
        .title(" SRElens & k9s Keybindings Cheat Sheet (Press Esc to close) ");

    let inner = block.inner(modal_area);
    f.render_widget(block, modal_area);

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(10),
            Constraint::Length(1),
        ])
        .split(inner);

    let rows = vec![
        Row::new(vec![
            Cell::from(Span::styled("Navigation & Views", Style::default().fg(Theme::CYAN).add_modifier(Modifier::BOLD))),
            Cell::from(""),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  : <command>", Theme::key_hint_key())),
            Cell::from("Open command prompt (:pod, :deploy, :svc, :no, :ns, :helm, :pf, :ai, :ctx, :q)"),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  Ctrl+c", Theme::key_hint_key())),
            Cell::from("Exit / kill srelens-tui immediately"),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  / <filter>", Theme::key_hint_key())),
            Cell::from("Filter table rows by substring or regex"),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  j / k / ↑ / ↓", Theme::key_hint_key())),
            Cell::from("Navigate up / down through list items"),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  g / G", Theme::key_hint_key())),
            Cell::from("Jump to top / bottom of current view"),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  Ctrl+u / Ctrl+d", Theme::key_hint_key())),
            Cell::from("Half-page scroll up / down"),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  Enter", Theme::key_hint_key())),
            Cell::from("Drill down (Pod -> Containers; Workload -> Pods; Node -> Pods; CRD -> Instances)"),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  Esc", Theme::key_hint_key())),
            Cell::from("Pop view back / Clear filter / Close modal"),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  Ctrl+a", Theme::key_hint_key())),
            Cell::from("Toggle between active namespace and all namespaces (0)"),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  Space", Theme::key_hint_key())),
            Cell::from("Mark / select item for bulk operations"),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("Resource Actions", Style::default().fg(Theme::CYAN).add_modifier(Modifier::BOLD))),
            Cell::from(""),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  x", Theme::key_hint_key())),
            Cell::from("Open Quick Actions & Incident Palette (AI prompt, restart, scale, tree, logs)"),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  t", Theme::key_hint_key())),
            Cell::from("Resource Relationship Tree (:tree) — ownerReferences, dependents & cross-links"),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  l", Theme::key_hint_key())),
            Cell::from("View Logs (options: p previous, t timestamps, w wrap, f follow, s save)"),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  s", Theme::key_hint_key())),
            Cell::from("Interactive Shell / Exec into pod container or debug container"),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  y / v", Theme::key_hint_key())),
            Cell::from("View YAML manifest"),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  d", Theme::key_hint_key())),
            Cell::from("Describe resource"),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  e", Theme::key_hint_key())),
            Cell::from("Edit resource YAML in $EDITOR with dry-run diff & server-side apply"),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  Ctrl+d", Theme::key_hint_key())),
            Cell::from("Delete selected resource (with confirmation)"),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  Ctrl+r", Theme::key_hint_key())),
            Cell::from("Rollout restart workload (Deployment / StatefulSet / DaemonSet)"),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  Ctrl+s", Theme::key_hint_key())),
            Cell::from("Scale workload replica count"),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  Shift+f / Ctrl+f", Theme::key_hint_key())),
            Cell::from("Start Port Forwarding to Pod / Service"),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  c", Theme::key_hint_key())),
            Cell::from("Copy resource name (or all marked names) to clipboard"),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  C / Shift+c", Theme::key_hint_key())),
            Cell::from("Copy resource full YAML to clipboard"),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  Ctrl+y", Theme::key_hint_key())),
            Cell::from("Copy resource srelens:// deep link URL"),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  Mouse drag", Theme::key_hint_key())),
            Cell::from("Select text in any view; release to copy to clipboard"),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  c / u (Nodes)", Theme::key_hint_key())),
            Cell::from("Cordon / Drain Node"),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  t", Theme::key_hint_key())),
            Cell::from("Trigger CronJob / Job manual run"),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("SRElens Superpowers", Style::default().fg(Theme::CYAN).add_modifier(Modifier::BOLD))),
            Cell::from(""),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  Tab / :ai", Theme::key_hint_key())),
            Cell::from("Open SRElens AI Assistant to investigate incidents & cluster state"),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  :helm", Theme::key_hint_key())),
            Cell::from("Helm 3 release management (history, values, manifest diff, rollback)"),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  :tb", Theme::key_hint_key())),
            Cell::from("Toolbox diagnostics (kubectl, helm, krew, plugins status)"),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  :pf", Theme::key_hint_key())),
            Cell::from("Port Forwards manager (active connections, transfer metrics)"),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  F1 - F10 / :ctx", Theme::key_hint_key())),
            Cell::from("Quick-switch Kubernetes cluster contexts"),
        ]),
    ];

    let table = Table::new(rows, [Constraint::Length(25), Constraint::Min(40)])
        .column_spacing(2);

    f.render_widget(table, sections[1]);
}
