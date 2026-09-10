use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

use crate::theme::Theme;
use crate::tui_config::TuiConfig;
use crate::ui::command_popup_rect;

pub const SAMPLE_SUGGESTIONS: &[(&str, &[&str], &str)] = &[
    ("pods", &["po"], "Pods in namespace or cluster-wide"),
    ("deployments", &["deploy"], "Deployments controller"),
    ("services", &["svc"], "Kubernetes Services"),
    ("configmaps", &["cm"], "ConfigMaps key-value configurations"),
    ("secrets", &["sec"], "Kubernetes Secrets (masked)"),
    ("nodes", &["no"], "Cluster worker and control-plane nodes"),
    ("events", &["ev"], "Recent cluster events and warnings"),
    ("helm", &["releases"], "Helm release revisions and values"),
    ("assistant", &["ai", "chat"], "SRElens AI Assistant Chat"),
    ("config", &["tui"], "TUI Configuration & Popup Size"),
    ("toolbox", &["tools"], "Diagnostics (kubectl, helm, krew)"),
    ("overview", &[], "Cluster health and resource totals"),
    ("quit", &["q", "exit"], "Quit SRElens"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuiConfigViewState {
    pub selected_field: usize, // 0 = Width, 1 = Visible Rows
}

impl Default for TuiConfigViewState {
    fn default() -> Self {
        Self::new()
    }
}

impl TuiConfigViewState {
    pub fn new() -> Self {
        Self { selected_field: 0 }
    }

    pub fn select_next_field(&mut self) {
        self.selected_field = (self.selected_field + 1) % 2;
    }

    pub fn select_prev_field(&mut self) {
        if self.selected_field == 0 {
            self.selected_field = 1;
        } else {
            self.selected_field -= 1;
        }
    }

    pub fn adjust_current(&mut self, delta: i32, config: &mut TuiConfig) {
        match self.selected_field {
            0 => {
                let current = config.command_popup_max_width as i32;
                let next = (current + delta * 5).clamp(40, 200) as u16;
                config.command_popup_max_width = next;
            }
            1 => {
                let current = config.command_popup_max_visible as i32;
                let next = (current + delta).clamp(3, 20) as usize;
                config.command_popup_max_visible = next;
            }
            _ => {}
        }
        let _ = config.save();
    }

    pub fn reset_defaults(&mut self, config: &mut TuiConfig) {
        *config = TuiConfig::default();
        let _ = config.save();
    }
}

fn render_slider(value: f32, total_width: usize) -> String {
    let clamped = value.clamp(0.0, 1.0);
    let filled = (clamped * total_width as f32).round() as usize;
    let empty = total_width.saturating_sub(filled);
    format!("[{}{}]", "█".repeat(filled), "░".repeat(empty))
}

pub fn render_tui_config_view(
    f: &mut Frame,
    area: Rect,
    state: &TuiConfigViewState,
    config: &TuiConfig,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::accent()))
        .title(Span::styled(" TUI Configuration ", Theme::title()));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Top description
            Constraint::Min(10),   // Controls and Live Preview
            Constraint::Length(2), // Bottom key hints
        ])
        .split(inner);

    // 1. Top Header Description
    let desc_lines = vec![
        Line::from(vec![
            Span::styled("TUI Display & Layout Options: ", Theme::header_label()),
            Span::styled(
                "Configure command menu dimensions and popup sizing.",
                Style::default().fg(Theme::fg()).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![Span::styled(
            "Settings are saved automatically to ~/.config/srelens/tui.json and applied immediately.",
            Style::default().fg(Theme::dim()),
        )]),
    ];
    f.render_widget(Paragraph::new(desc_lines), chunks[0]);

    // 2. Middle Region: Controls (left) & Live Preview (right) or stacked if narrow
    let (controls_area, preview_area) = if chunks[1].width >= 90 {
        let h_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
            .split(chunks[1]);
        (h_chunks[0], h_chunks[1])
    } else {
        let v_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(12), Constraint::Min(8)])
            .split(chunks[1]);
        (v_chunks[0], v_chunks[1])
    };

    // Setting cards
    let control_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6), // Width card
            Constraint::Length(6), // Visible rows card
            Constraint::Min(0),
        ])
        .split(controls_area);

    // Setting 0: Command Popup Max Width
    let is_width_selected = state.selected_field == 0;
    let width_border_color = if is_width_selected {
        Theme::cyan()
    } else {
        Theme::border()
    };
    let width_title = if is_width_selected {
        " ▶ Command Popup Max Width "
    } else {
        "   Command Popup Max Width "
    };
    let width_block = Block::default()
        .borders(Borders::ALL)
        .border_type(Theme::border_type())
        .border_style(Style::default().fg(width_border_color))
        .title(Span::styled(
            width_title,
            if is_width_selected {
                Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Theme::fg())
            },
        ));
    let width_inner = width_block.inner(control_chunks[0]);
    f.render_widget(width_block, control_chunks[0]);

    let width_val = config.command_popup_max_width;
    let width_pct = (width_val.saturating_sub(40) as f32) / (160.0);
    let slider_width = (width_inner.width.saturating_sub(4) as usize).min(24).max(10);
    let width_lines = vec![
        Line::from(vec![
            Span::styled("Width: ", Style::default().fg(Theme::dim())),
            Span::styled(
                format!("{width_val:>3} cols "),
                Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD),
            ),
            Span::styled(render_slider(width_pct, slider_width), Style::default().fg(Theme::accent())),
            Span::styled(" [40..=200]", Style::default().fg(Theme::dim())),
        ]),
        Line::from(vec![
            Span::styled("Step: ±5  |  Use ", Style::default().fg(Theme::dim())),
            Span::styled("h/l", Style::default().fg(Theme::yellow())),
            Span::styled(" or ", Style::default().fg(Theme::dim())),
            Span::styled("←/→", Style::default().fg(Theme::yellow())),
            Span::styled(" or ", Style::default().fg(Theme::dim())),
            Span::styled("-/+", Style::default().fg(Theme::yellow())),
            Span::styled(" to adjust", Style::default().fg(Theme::dim())),
        ]),
    ];
    f.render_widget(Paragraph::new(width_lines), width_inner);

    // Setting 1: Command Popup Max Visible Rows
    let is_rows_selected = state.selected_field == 1;
    let rows_border_color = if is_rows_selected {
        Theme::cyan()
    } else {
        Theme::border()
    };
    let rows_title = if is_rows_selected {
        " ▶ Command Popup Max Visible Rows "
    } else {
        "   Command Popup Max Visible Rows "
    };
    let rows_block = Block::default()
        .borders(Borders::ALL)
        .border_type(Theme::border_type())
        .border_style(Style::default().fg(rows_border_color))
        .title(Span::styled(
            rows_title,
            if is_rows_selected {
                Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Theme::fg())
            },
        ));
    let rows_inner = rows_block.inner(control_chunks[1]);
    f.render_widget(rows_block, control_chunks[1]);

    let rows_val = config.command_popup_max_visible;
    let rows_pct = (rows_val.saturating_sub(3) as f32) / (17.0);
    let slider_rows = (rows_inner.width.saturating_sub(4) as usize).min(24).max(10);
    let rows_lines = vec![
        Line::from(vec![
            Span::styled("Rows:  ", Style::default().fg(Theme::dim())),
            Span::styled(
                format!("{rows_val:>3} rows "),
                Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD),
            ),
            Span::styled(render_slider(rows_pct, slider_rows), Style::default().fg(Theme::accent())),
            Span::styled(" [3..=20]", Style::default().fg(Theme::dim())),
        ]),
        Line::from(vec![
            Span::styled("Step: ±1  |  Use ", Style::default().fg(Theme::dim())),
            Span::styled("h/l", Style::default().fg(Theme::yellow())),
            Span::styled(" or ", Style::default().fg(Theme::dim())),
            Span::styled("←/→", Style::default().fg(Theme::yellow())),
            Span::styled(" or ", Style::default().fg(Theme::dim())),
            Span::styled("-/+", Style::default().fg(Theme::yellow())),
            Span::styled(" to adjust", Style::default().fg(Theme::dim())),
        ]),
    ];
    f.render_widget(Paragraph::new(rows_lines), rows_inner);

    // 3. Live Preview of Command Popup
    let preview_block = Block::default()
        .borders(Borders::ALL)
        .border_type(Theme::border_type())
        .border_style(Style::default().fg(Theme::border()))
        .title(Span::styled(
            " Live Preview: Command Popup (:) ",
            Style::default().fg(Theme::accent()).add_modifier(Modifier::BOLD),
        ));
    let preview_inner = preview_block.inner(preview_area);
    f.render_widget(preview_block, preview_area);

    if preview_inner.height >= 4 && preview_inner.width >= 20 {
        // Simulated statusbar at bottom of preview area
        let sim_bar_y = preview_inner.y + preview_inner.height.saturating_sub(1);
        let sim_bar_area = Rect {
            x: preview_inner.x,
            y: sim_bar_y,
            width: preview_inner.width,
            height: 1,
        };
        let sim_cmd_line = Line::from(vec![
            Span::styled(":", Theme::prompt()),
            Span::styled("po", Style::default().fg(Theme::fg())),
            Span::styled("█", Style::default().fg(Theme::cyan())),
        ]);
        f.render_widget(Paragraph::new(sim_cmd_line), sim_bar_area);

        // Compute popup area relative to sim_bar_area using command_popup_rect
        let calc_popup = command_popup_rect(
            sim_bar_area,
            SAMPLE_SUGGESTIONS.len(),
            config.command_popup_max_width,
            config.command_popup_max_visible,
        );

        // Clamp popup vertically so it stays within preview_inner
        let max_avail_height = preview_inner.height.saturating_sub(1);
        let popup_height = calc_popup.height.min(max_avail_height);
        let popup_y = sim_bar_y.saturating_sub(popup_height).max(preview_inner.y);
        let popup_width = calc_popup.width.min(preview_inner.width.saturating_sub(2));

        let popup_area = Rect {
            x: preview_inner.x + 1,
            y: popup_y,
            width: popup_width,
            height: popup_height,
        };

        f.render_widget(Clear, popup_area);

        let max_visible = config.command_popup_max_visible;
        let visible_count = SAMPLE_SUGGESTIONS.len().min(max_visible);
        let visible_slice = &SAMPLE_SUGGESTIONS[0..visible_count];

        let items: Vec<ListItem> = visible_slice
            .iter()
            .enumerate()
            .map(|(i, (name, aliases, desc))| {
                let is_selected = i == 0;
                let prefix = if is_selected { "▶ " } else { "  " };
                let alias_str = if !aliases.is_empty() {
                    format!(" ({})", aliases.join(", "))
                } else {
                    String::new()
                };
                let line = Line::from(vec![
                    Span::styled(
                        format!("{}{}{:<18}", prefix, name, alias_str),
                        if is_selected {
                            Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Theme::fg())
                        },
                    ),
                    Span::styled(format!("  {}", desc), Style::default().fg(Theme::dim())),
                ]);
                ListItem::new(line).style(if is_selected {
                    Theme::selected_row()
                } else {
                    Style::default()
                })
            })
            .collect();

        let title = if SAMPLE_SUGGESTIONS.len() > max_visible {
            format!(
                " Commands [1/{}] (Tab: complete, Enter: run) ",
                SAMPLE_SUGGESTIONS.len()
            )
        } else {
            " Commands (Tab to complete, Enter to run) ".to_string()
        };

        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(Theme::border_type())
                .border_style(Style::default().fg(Theme::accent()))
                .title(title),
        );
        f.render_widget(list, popup_area);
    }

    // 4. Bottom Key Hints
    let hints_line = Line::from(vec![
        Span::styled("<j/k or ↑/↓> ", Style::default().fg(Theme::yellow()).add_modifier(Modifier::BOLD)),
        Span::styled("Select  ", Style::default().fg(Theme::dim())),
        Span::styled("<h/l, ←/→, -/+> ", Style::default().fg(Theme::yellow()).add_modifier(Modifier::BOLD)),
        Span::styled("Adjust  ", Style::default().fg(Theme::dim())),
        Span::styled("<r> ", Style::default().fg(Theme::yellow()).add_modifier(Modifier::BOLD)),
        Span::styled("Reset Defaults  ", Style::default().fg(Theme::dim())),
        Span::styled("<Esc or q> ", Style::default().fg(Theme::yellow()).add_modifier(Modifier::BOLD)),
        Span::styled("Back", Style::default().fg(Theme::dim())),
    ]);
    f.render_widget(Paragraph::new(hints_line), chunks[2]);
}
