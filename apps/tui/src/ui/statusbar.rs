use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

use crate::commands::{command_suggestions, CommandDef};
use crate::theme::Theme;

#[derive(Debug, Clone, PartialEq)]
pub enum InputMode {
    Normal,
    Command, // `:` mode
    Filter,  // `/` mode
}

pub struct StatusBarProps<'a> {
    pub mode: &'a InputMode,
    pub command_input: &'a str,
    pub filter_input: &'a str,
    pub matched_count: usize,
    pub total_count: usize,
    pub toast: Option<(&'a str, Style)>,
    pub custom_hints: Option<&'a [(&'a str, &'a str)]>,
    pub suggestions: Option<(&'a [(crate::commands::DynamicCommandDef, usize)], usize)>,
}

pub fn render_statusbar(f: &mut Frame, area: Rect, props: StatusBarProps) {
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(Theme::BORDER));

    let inner = block.inner(area);
    f.render_widget(block, area);

    match props.mode {
        InputMode::Command => {
            // Render command bar with autocomplete popup
            let cmd_text = Line::from(vec![
                Span::styled(":", Theme::prompt()),
                Span::styled(props.command_input, Style::default().fg(Theme::FG)),
                Span::styled("█", Style::default().fg(Theme::CYAN)), // Cursor
            ]);
            f.render_widget(Paragraph::new(cmd_text), inner);

            // Render autocomplete suggestions if typing
            if let Some((suggestions, selected_idx)) = props.suggestions {
                if !suggestions.is_empty() && !props.command_input.is_empty() {
                    let popup_height = (suggestions.len() as u16).min(8) + 2;
                    let popup_area = Rect {
                        x: area.x + 1,
                        y: area.y.saturating_sub(popup_height),
                        width: 60.min(area.width.saturating_sub(2)),
                        height: popup_height,
                    };

                    f.render_widget(Clear, popup_area);

                    let items: Vec<ListItem> = suggestions
                        .iter()
                        .take(8)
                        .enumerate()
                        .map(|(i, (cmd, _))| {
                            let is_sel = i == selected_idx % suggestions.len().max(1);
                            let aliases_str = if cmd.aliases.is_empty() {
                                String::new()
                            } else {
                                format!(" ({})", cmd.aliases.join(", "))
                            };
                            let prefix = if is_sel { "▶ " } else { "  " };
                            let name_style = if is_sel {
                                Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)
                            } else {
                                Style::default().fg(Theme::CYAN).add_modifier(Modifier::BOLD)
                            };

                            let line = Line::from(vec![
                                Span::styled(prefix, name_style),
                                Span::styled(format!(":{}", cmd.name), name_style),
                                Span::styled(aliases_str, Style::default().fg(Theme::DIM)),
                                Span::raw(" - "),
                                Span::styled(&cmd.description, Style::default().fg(Theme::FG)),
                            ]);
                            let item_style = if is_sel {
                                Style::default().bg(Theme::SEL_BG)
                            } else {
                                Style::default()
                            };
                            ListItem::new(line).style(item_style)
                        })
                        .collect();

                    let list = List::new(items).block(
                        Block::default()
                            .borders(Borders::ALL)
                            .border_style(Style::default().fg(Theme::ACCENT))
                            .title(" Commands (Tab to complete, Enter to run) "),
                    );
                    f.render_widget(list, popup_area);
                }
            }
        }
        InputMode::Filter => {
            let filter_text = Line::from(vec![
                Span::styled("Filter (regex): /", Theme::prompt()),
                Span::styled(props.filter_input, Style::default().fg(Theme::FG)),
                Span::styled("█", Style::default().fg(Theme::YELLOW)), // Cursor
                Span::raw("  "),
                Span::styled(
                    format!("[{}/{}]", props.matched_count, props.total_count),
                    Style::default().fg(Theme::DIM),
                ),
                Span::styled("  (Enter to apply, Esc to clear)", Style::default().fg(Theme::DIM)),
            ]);
            f.render_widget(Paragraph::new(filter_text), inner);
        }
        InputMode::Normal => {
            if let Some((msg, style)) = props.toast {
                // Show notification toast message
                let toast_line = Line::from(vec![
                    Span::styled("➜ ", style),
                    Span::styled(msg, style),
                ]);
                f.render_widget(Paragraph::new(toast_line), inner);
            } else {
                // Show k9s shortcut key hints
                let hints = props.custom_hints.unwrap_or(&[
                    ("<:>", "Cmd"),
                    ("</>", "Filter"),
                    ("<l>", "Logs"),
                    ("<s>", "Shell"),
                    ("<y>", "YAML"),
                    ("<e>", "Edit"),
                    ("<d>", "Describe"),
                    ("<^d>", "Delete"),
                    ("<^r>", "Restart"),
                    ("<^s>", "Scale"),
                    ("<^f>", "PF"),
                    ("<?>", "Help"),
                ]);

                let mut spans = Vec::new();
                for (key, desc) in hints {
                    spans.push(Span::styled(*key, Theme::key_hint_key()));
                    spans.push(Span::styled(format!(" {} ", desc), Theme::key_hint_desc()));
                }

                if !props.filter_input.is_empty() {
                    spans.push(Span::raw(" | "));
                    spans.push(Span::styled("Filter: ", Theme::header_label()));
                    spans.push(Span::styled(
                        format!("\"{}\" [{}/{}]", props.filter_input, props.matched_count, props.total_count),
                        Style::default().fg(Theme::YELLOW),
                    ));
                }

                f.render_widget(Paragraph::new(Line::from(spans)), inner);
            }
        }
    }
}
