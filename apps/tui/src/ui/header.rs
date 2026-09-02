use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::theme::Theme;

pub struct HeaderProps<'a> {
    pub context: &'a str,
    pub cluster: &'a str,
    pub server: &'a str,
    pub namespace: &'a str,
    pub version: &'a str,
    pub node_count: usize,
    pub pod_count: usize,
    pub is_connected: bool,
    pub active_view_name: &'a str,
}

pub fn render_header(f: &mut Frame, area: Rect, props: HeaderProps) {
    let block = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(Theme::BORDER));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(12),  // Brand "SRELENS"
            Constraint::Min(30),     // Cluster / Context / Namespace info
            Constraint::Length(35),  // Status / Health metrics
        ])
        .split(inner);

    // 1. Logo / Brand
    let brand = Paragraph::new(Line::from(vec![
        Span::styled("⚡ ", Style::default().fg(Theme::YELLOW)),
        Span::styled("SRELENS", Style::default().fg(Theme::ACCENT).add_modifier(Modifier::BOLD)),
    ]));
    f.render_widget(brand, chunks[0]);

    // 2. Context, Namespace, View title
    let ns_display = if props.namespace.is_empty() {
        "all"
    } else {
        props.namespace
    };

    let status_dot = if props.is_connected {
        Span::styled("● ", Style::default().fg(Theme::GREEN))
    } else {
        Span::styled("● ", Style::default().fg(Theme::RED))
    };

    let cluster_line = Line::from(vec![
        Span::styled("Ctx: ", Theme::header_label()),
        Span::styled(props.context, Theme::header_val()),
        Span::raw("  "),
        Span::styled("NS: ", Theme::header_label()),
        Span::styled(format!("[{}]", ns_display), Style::default().fg(Theme::CYAN).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled("View: ", Theme::header_label()),
        Span::styled(props.active_view_name, Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)),
    ]);
    f.render_widget(Paragraph::new(cluster_line), chunks[1]);

    // 3. Cluster stats & Version
    let version_clean = if props.version.starts_with('v') {
        props.version.to_string()
    } else if props.version.contains('.') {
        format!("v{}", props.version)
    } else {
        props.version.to_string()
    };

    let stats_line = Line::from(vec![
        status_dot,
        Span::styled(format!("{} ", version_clean), Theme::header_label()),
        Span::styled("Nodes: ", Theme::header_label()),
        Span::styled(format!("{} ", props.node_count), Theme::header_val()),
        Span::styled("Pods: ", Theme::header_label()),
        Span::styled(format!("{}", props.pod_count), Theme::header_val()),
    ]);
    f.render_widget(Paragraph::new(stats_line).alignment(ratatui::layout::Alignment::Right), chunks[2]);
}
