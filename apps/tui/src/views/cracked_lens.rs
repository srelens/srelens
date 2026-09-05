use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::theme::Theme;

/// Renders a centered cracked / broken ASCII magnifying lens widget
/// when the Kubernetes cluster is unreachable after timeout or error.
pub fn render_cracked_lens(
    f: &mut Frame,
    area: Rect,
    context: &str,
    cluster: &str,
    server: &str,
    elapsed_secs: u64,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::RED))
        .title(Span::styled(" Cluster Unreachable ", Style::default().fg(Theme::RED).add_modifier(Modifier::BOLD)));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.height < 10 || inner.width < 30 {
        let compact_msg = vec![
            Line::from(vec![
                Span::styled("✖ Cluster Unreachable ", Style::default().fg(Theme::RED).add_modifier(Modifier::BOLD)),
                Span::styled(format!("(timed out after {}s)", elapsed_secs), Style::default().fg(Theme::DIM)),
            ]),
            Line::from(vec![
                Span::styled("Context: ", Theme::header_label()),
                Span::styled(context, Style::default().fg(Theme::YELLOW)),
            ]),
            Line::from(vec![
                Span::styled("Press <r> to retry or <Ctrl+x> to switch context", Style::default().fg(Theme::CYAN)),
            ]),
        ];
        f.render_widget(Paragraph::new(compact_msg).alignment(Alignment::Center), inner);
        return;
    }

    // ASCII Art for Cracked Lens:
    // A high-definition double-rim magnifying glass with realistic lightning fracture & glares
    let rim_style = Style::default().fg(Theme::RED);
    let crack_style = Style::default()
        .fg(Color::Rgb(255, 215, 0))
        .add_modifier(Modifier::BOLD);
    let glare_style = Style::default()
        .fg(Theme::CYAN)
        .add_modifier(Modifier::DIM);
    let handle_style = Style::default().fg(Theme::DIM);

    let cracked_lens_lines = vec![
        Line::from(Span::styled("             .--------------------.", rim_style)),
        Line::from(Span::styled("          .-'  .----------------.  `-.", rim_style)),
        Line::from(vec![
            Span::styled("        .'   .' ", rim_style),
            Span::styled("//", glare_style),
            Span::raw(" "),
            Span::styled("\\", crack_style),
            Span::raw("             "),
            Span::styled("`.   `.", rim_style),
        ]),
        Line::from(vec![
            Span::styled("       /    /   ", rim_style),
            Span::styled("//", glare_style),
            Span::raw("  "),
            Span::styled("\\     /\\", crack_style),
            Span::raw("       "),
            Span::styled("\\    \\", rim_style),
        ]),
        Line::from(vec![
            Span::styled("      ;    ;         ", rim_style),
            Span::styled("\\   /  \\", crack_style),
            Span::raw("       "),
            Span::styled(";    ;", rim_style),
        ]),
        Line::from(vec![
            Span::styled("     /    /           ", rim_style),
            Span::styled("\\_/    \\", crack_style),
            Span::raw("       "),
            Span::styled("\\    \\", rim_style),
        ]),
        Line::from(vec![
            Span::styled("    |    |             ", rim_style),
            Span::styled("/      \\", crack_style),
            Span::raw("       "),
            Span::styled("|    |", rim_style),
        ]),
        Line::from(vec![
            Span::styled("    |    |       ", rim_style),
            Span::styled("/\\   /   /\\   \\", crack_style),
            Span::raw("      "),
            Span::styled("|    |", rim_style),
        ]),
        Line::from(vec![
            Span::styled("    |    |      ", rim_style),
            Span::styled("/  \\_/   /  \\   \\_", crack_style),
            Span::raw("    "),
            Span::styled("|    |", rim_style),
        ]),
        Line::from(vec![
            Span::styled("     \\    \\    ", rim_style),
            Span::styled("/        /    \\    \\", crack_style),
            Span::raw("  "),
            Span::styled("/    /", rim_style),
        ]),
        Line::from(vec![
            Span::styled("      ;    ;  ", rim_style),
            Span::styled("/        /      \\", crack_style),
            Span::raw("    "),
            Span::styled(";    ;", rim_style),
        ]),
        Line::from(vec![
            Span::styled("       \\    \\/", rim_style),
            Span::styled("        /        \\", crack_style),
            Span::raw("  "),
            Span::styled("/    /", rim_style),
        ]),
        Line::from(vec![
            Span::styled("        `.   `.      ", rim_style),
            Span::styled("/", crack_style),
            Span::raw("          "),
            Span::styled(".'   .'", rim_style),
        ]),
        Line::from(vec![
            Span::styled("          `-._ `----", rim_style),
            Span::styled("/", crack_style),
            Span::styled("-----------' _.-'", rim_style),
        ]),
        Line::from(vec![
            Span::styled("              `----", rim_style),
            Span::styled("/", crack_style),
            Span::styled("-------------'   ", rim_style),
            Span::styled("\\", handle_style),
        ]),
        Line::from(vec![
            Span::raw("                                      "),
            Span::styled("\\ \\", handle_style),
        ]),
        Line::from(vec![
            Span::raw("                                       "),
            Span::styled("\\ \\", handle_style),
        ]),
        Line::from(vec![
            Span::raw("                                        "),
            Span::styled("\\_\\", handle_style),
        ]),
    ];

    let mut diag_lines = vec![
        Line::from(vec![
            Span::styled("✖ CLUSTER UNREACHABLE ", Style::default().fg(Theme::RED).add_modifier(Modifier::BOLD)),
            Span::styled(format!("(no response after {}s)", elapsed_secs), Style::default().fg(Theme::DIM)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Context:  ", Theme::header_label()),
            Span::styled(context, Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)),
        ]),
    ];

    if !cluster.is_empty() {
        diag_lines.push(Line::from(vec![
            Span::styled("Cluster:  ", Theme::header_label()),
            Span::styled(cluster, Style::default().fg(Theme::FG)),
        ]));
    }
    if !server.is_empty() {
        diag_lines.push(Line::from(vec![
            Span::styled("Server:   ", Theme::header_label()),
            Span::styled(server, Style::default().fg(Theme::CYAN)),
        ]));
    }

    diag_lines.push(Line::from(""));
    diag_lines.push(Line::from(vec![
        Span::styled("Quick Actions:", Style::default().fg(Theme::ACCENT).add_modifier(Modifier::BOLD)),
    ]));
    diag_lines.push(Line::from(vec![
        Span::styled("  [r]          ", Style::default().fg(Theme::CYAN).add_modifier(Modifier::BOLD)),
        Span::styled("Retry connection", Theme::header_label()),
    ]));
    diag_lines.push(Line::from(vec![
        Span::styled("  [Ctrl+x]     ", Style::default().fg(Theme::CYAN).add_modifier(Modifier::BOLD)),
        Span::styled("Switch context (:ctx)", Theme::header_label()),
    ]));
    diag_lines.push(Line::from(vec![
        Span::styled("  [:]          ", Style::default().fg(Theme::CYAN).add_modifier(Modifier::BOLD)),
        Span::styled("Open command bar", Theme::header_label()),
    ]));
    diag_lines.push(Line::from(vec![
        Span::styled("  [?]          ", Style::default().fg(Theme::CYAN).add_modifier(Modifier::BOLD)),
        Span::styled("Open help menu", Theme::header_label()),
    ]));

    let total_art_h = cracked_lens_lines.len() as u16;
    let total_diag_h = diag_lines.len() as u16;
    let required_h = total_art_h.max(total_diag_h);

    let v_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(inner.height.saturating_sub(required_h) / 2),
            Constraint::Length(required_h),
            Constraint::Min(0),
        ])
        .split(inner);

    let content_area = v_chunks[1];
    let total_art_w: u16 = 44;
    let spacing: u16 = 4;
    let min_diag_w: u16 = 40;
    let total_min_w = total_art_w + spacing + min_diag_w;

    if content_area.width >= total_min_w {
        let pad = (content_area.width.saturating_sub(total_min_w + 10)) / 2;
        let h_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(pad),
                Constraint::Length(total_art_w),
                Constraint::Length(spacing),
                Constraint::Min(min_diag_w),
                Constraint::Length(pad),
            ])
            .split(content_area);

        f.render_widget(Paragraph::new(cracked_lens_lines), h_chunks[1]);
        f.render_widget(Paragraph::new(diag_lines), h_chunks[3]);
    } else {
        // Narrow terminal: stack vertically
        let mut combined = cracked_lens_lines;
        combined.push(Line::from(""));
        combined.extend(diag_lines);
        f.render_widget(Paragraph::new(combined).alignment(Alignment::Center), inner);
    }
}
