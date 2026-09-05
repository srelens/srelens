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
    pub is_text_search: bool,
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
            if let Some((suggs, selected_idx)) = props.suggestions {
                if !suggs.is_empty() {
                    let popup_height = (suggs.len() as u16 + 2).min(8);
                    let popup_area = Rect {
                        x: area.x + 2,
                        y: area.y.saturating_sub(popup_height),
                        width: area.width.saturating_sub(4).min(65),
                        height: popup_height,
                    };
                    f.render_widget(Clear, popup_area);

                    let items: Vec<ListItem> = suggs
                        .iter()
                        .enumerate()
                        .map(|(i, (cmd, _score))| {
                            let is_selected = i == selected_idx;
                            let prefix = if is_selected { "▶ " } else { "  " };
                            let alias_str = if !cmd.aliases.is_empty() {
                                format!(" ({})", cmd.aliases.join(", "))
                            } else {
                                String::new()
                            };
                            let line = Line::from(vec![
                                Span::styled(
                                    format!("{}{}{:<20}", prefix, cmd.name, alias_str),
                                    if is_selected {
                                        Style::default().fg(Theme::CYAN).add_modifier(Modifier::BOLD)
                                    } else {
                                        Style::default().fg(Theme::FG)
                                    },
                                ),
                                Span::styled(
                                    format!("  {}", cmd.description),
                                    Style::default().fg(Theme::DIM),
                                ),
                            ]);
                            ListItem::new(line).style(if is_selected {
                                Theme::selected_row()
                            } else {
                                Style::default()
                            })
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
            let (label, stats, hint) = if props.is_text_search {
                (
                    "Search: /",
                    format!("[{} matches]", props.matched_count),
                    "  (Enter to finish, n/N next/prev, Esc to clear)",
                )
            } else {
                (
                    "Filter (regex): /",
                    format!("[{}/{}]", props.matched_count, props.total_count),
                    "  (Enter to apply, Esc to clear)",
                )
            };
            let filter_text = Line::from(vec![
                Span::styled(label, Theme::prompt()),
                Span::styled(props.filter_input, Style::default().fg(Theme::FG)),
                Span::styled("█", Style::default().fg(Theme::YELLOW)), // Cursor
                Span::raw("  "),
                Span::styled(stats, Style::default().fg(Theme::DIM)),
                Span::styled(hint, Style::default().fg(Theme::DIM)),
            ]);
            f.render_widget(Paragraph::new(filter_text), inner);
        }
        InputMode::Normal => {
            let default_hints: &[(&str, &str)] = &[
                ("<:>", "Cmd"),
                ("</>", "Filter"),
                ("<c>", "CopyURL"),
                ("<l>", "Logs"),
                ("<s>", "Shell"),
                ("<f>/<F>", "PortForward"),
                ("<d>", "Describe"),
                ("<y>", "YAML"),
                ("<e>", "Edit"),
                ("<^d>", "Delete"),
                ("<^r>", "Restart"),
                ("<^s>", "Scale"),
                ("<?>", "Help"),
            ];
            let hints = props.custom_hints.unwrap_or(default_hints);

            let mut spans = Vec::new();

            // Render notification toast without hiding keystroke palette
            if let Some((msg, style)) = props.toast {
                spans.push(Span::styled("➜ ", style));
                spans.push(Span::styled(format!("{} ", msg), style));
                spans.push(Span::styled("│ ", Style::default().fg(Theme::BORDER)));
            }

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
