use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::theme::Theme;

#[derive(Debug, Clone, PartialEq)]
pub struct ContextChipInfo {
    pub name: String,
    pub is_current: bool,
    pub is_local: bool,
    pub index: usize,
}

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
    pub contexts: &'a [ContextChipInfo],
    pub context_chip_rects: Option<&'a std::cell::RefCell<Vec<(Rect, String)>>>,
}

pub fn render_header(f: &mut Frame, area: Rect, props: HeaderProps) {
    let block = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(Theme::BORDER));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if let Some(rects) = props.context_chip_rects {
        rects.borrow_mut().clear();
    }

    if inner.height >= 2 {
        let v_rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Row 0: Brand | Context Hotbar Chips | Stats
                Constraint::Length(1), // Row 1: Active Ctx / NS / View | Hints
            ])
            .split(inner);

        // Row 0:
        let r0_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(11), // Brand "⚡ SRELENS"
                Constraint::Min(20),    // Context Hotbar Chips
                Constraint::Length(34), // Stats
            ])
            .split(v_rows[0]);

        // Brand
        let brand = Paragraph::new(Line::from(vec![
            Span::styled("⚡ ", Style::default().fg(Theme::YELLOW)),
            Span::styled("SRELENS", Style::default().fg(Theme::ACCENT).add_modifier(Modifier::BOLD)),
        ]));
        f.render_widget(brand, r0_chunks[0]);

        // Contexts Hotbar
        render_context_chips(f, r0_chunks[1], props.contexts, props.context_chip_rects);

        // Stats
        render_stats(f, r0_chunks[2], &props);

        // Row 1: Active context details & navigation hints
        let r1_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Min(35),    // Active Ctx / NS / View
                Constraint::Length(38), // Hints
            ])
            .split(v_rows[1]);

        let ns_display = if props.namespace.is_empty() {
            "all"
        } else {
            props.namespace
        };

        let active_ctx_color = Theme::context_color(props.context, props.contexts.iter().find(|c| c.name == props.context).map(|c| c.is_local).unwrap_or(false));

        let active_line = Line::from(vec![
            Span::styled("Ctx: ", Theme::header_label()),
            Span::styled(props.context, Style::default().fg(active_ctx_color).add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled("NS: ", Theme::header_label()),
            Span::styled(format!("[{}]", ns_display), Style::default().fg(Theme::CYAN).add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled("View: ", Theme::header_label()),
            Span::styled(props.active_view_name, Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)),
        ]);
        f.render_widget(Paragraph::new(active_line), r1_chunks[0]);

        let hints_line = Line::from(vec![
            Span::styled("<:ctx>", Style::default().fg(Theme::CYAN).add_modifier(Modifier::BOLD)),
            Span::styled(" All Ctx  ", Theme::header_label()),
            Span::styled("<F1-F10>", Style::default().fg(Theme::CYAN).add_modifier(Modifier::BOLD)),
            Span::styled(" Hotkeys  ", Theme::header_label()),
            Span::styled("<?>", Style::default().fg(Theme::CYAN).add_modifier(Modifier::BOLD)),
            Span::styled(" Help", Theme::header_label()),
        ]);
        f.render_widget(Paragraph::new(hints_line).alignment(ratatui::layout::Alignment::Right), r1_chunks[1]);
    } else {
        // Fallback for compact single-row header
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(11),
                Constraint::Min(30),
                Constraint::Length(34),
            ])
            .split(inner);

        let brand = Paragraph::new(Line::from(vec![
            Span::styled("⚡ ", Style::default().fg(Theme::YELLOW)),
            Span::styled("SRELENS", Style::default().fg(Theme::ACCENT).add_modifier(Modifier::BOLD)),
        ]));
        f.render_widget(brand, chunks[0]);

        let ns_display = if props.namespace.is_empty() { "all" } else { props.namespace };
        let active_ctx_color = Theme::context_color(props.context, false);
        let cluster_line = Line::from(vec![
            Span::styled("Ctx: ", Theme::header_label()),
            Span::styled(props.context, Style::default().fg(active_ctx_color).add_modifier(Modifier::BOLD)),
            Span::raw(" "),
            Span::styled("NS: ", Theme::header_label()),
            Span::styled(format!("[{}]", ns_display), Style::default().fg(Theme::CYAN).add_modifier(Modifier::BOLD)),
            Span::raw(" "),
            Span::styled("View: ", Theme::header_label()),
            Span::styled(props.active_view_name, Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)),
        ]);
        f.render_widget(Paragraph::new(cluster_line), chunks[1]);
        render_stats(f, chunks[2], &props);
    }
}

fn render_stats(f: &mut Frame, area: Rect, props: &HeaderProps) {
    let status_dot = if props.is_connected {
        Span::styled("● ", Style::default().fg(Theme::GREEN))
    } else {
        Span::styled("● ", Style::default().fg(Theme::RED))
    };

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
    f.render_widget(Paragraph::new(stats_line).alignment(ratatui::layout::Alignment::Right), area);
}

fn render_context_chips(
    f: &mut Frame,
    area: Rect,
    contexts: &[ContextChipInfo],
    rects: Option<&std::cell::RefCell<Vec<(Rect, String)>>>,
) {
    if contexts.is_empty() || area.width < 5 {
        return;
    }

    let mut spans = Vec::new();
    let mut current_col: u16 = area.x;
    let max_x = area.x + area.width;

    for (idx, ctx) in contexts.iter().enumerate() {
        let color = Theme::context_color(&ctx.name, ctx.is_local);
        let chip_label = if ctx.is_current {
            format!("[● {}: {}]", ctx.index, ctx.name)
        } else {
            format!("[{}: {}]", ctx.index, ctx.name)
        };

        let chip_w = unicode_width::UnicodeWidthStr::width(chip_label.as_str()) as u16;

        if current_col + chip_w > max_x {
            let remaining = contexts.len().saturating_sub(idx);
            let overflow_label = format!("[+{} (:ctx)]", remaining);
            let overflow_w = unicode_width::UnicodeWidthStr::width(overflow_label.as_str()) as u16;
            if current_col + overflow_w <= max_x {
                if let Some(r) = rects {
                    r.borrow_mut().push((Rect { x: current_col, y: area.y, width: overflow_w, height: 1 }, ":ctx".to_string()));
                }
                spans.push(Span::styled(overflow_label, Style::default().fg(Theme::CYAN).add_modifier(Modifier::BOLD)));
            }
            break;
        }

        if let Some(r) = rects {
            r.borrow_mut().push((Rect { x: current_col, y: area.y, width: chip_w, height: 1 }, ctx.name.clone()));
        }

        if ctx.is_current {
            spans.push(Span::styled(
                chip_label,
                Style::default().fg(color).bg(Color::Rgb(35, 40, 50)).add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(
                chip_label,
                Style::default().fg(color),
            ));
        }

        spans.push(Span::raw(" "));
        current_col += chip_w + 1;
    }

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}
