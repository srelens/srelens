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
    pub close_pf_button: Option<(&'a str, Style)>,
    pub close_pf_rect: Option<&'a std::cell::RefCell<Option<Rect>>>,
}

pub fn render_statusbar(f: &mut Frame, area: Rect, props: StatusBarProps) {
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(Theme::border()));

    let inner = block.inner(area);
    f.render_widget(block, area);

    match props.mode {
        InputMode::Command => {
            // Render command bar with autocomplete popup
            let prompt_prefix = Theme::prompt_glyph();
            let prompt_str = if prompt_prefix == ":" {
                ":".to_string()
            } else {
                format!("{}:", prompt_prefix)
            };
            let cmd_text = Line::from(vec![
                Span::styled(prompt_str, Theme::prompt()),
                Span::styled(props.command_input, Style::default().fg(Theme::fg())),
                Span::styled("█", Style::default().fg(Theme::cyan())), // Cursor
            ]);
            f.render_widget(Paragraph::new(cmd_text), inner);

            // Render autocomplete suggestions if typing
            if let Some((suggs, selected_idx)) = props.suggestions {
                if !suggs.is_empty() {
                    let max_visible = 6usize;
                    let visible_count = suggs.len().min(max_visible);
                    let popup_height = (visible_count as u16 + 2).min(8);
                    let popup_area = Rect {
                        x: area.x + 2,
                        y: area.y.saturating_sub(popup_height),
                        width: area.width.saturating_sub(4).min(65),
                        height: popup_height,
                    };
                    f.render_widget(Clear, popup_area);

                    // Compute window offset to keep selected_idx visible
                    let scroll_offset = if selected_idx >= max_visible {
                        selected_idx + 1 - max_visible
                    } else {
                        0
                    };

                    let visible_slice = &suggs[scroll_offset..(scroll_offset + visible_count).min(suggs.len())];

                    let items: Vec<ListItem> = visible_slice
                        .iter()
                        .enumerate()
                        .map(|(rel_i, (cmd, _score))| {
                            let abs_i = scroll_offset + rel_i;
                            let is_selected = abs_i == selected_idx;
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
                                        Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD)
                                    } else {
                                        Style::default().fg(Theme::fg())
                                    },
                                ),
                                Span::styled(
                                    format!("  {}", cmd.description),
                                    Style::default().fg(Theme::dim()),
                                ),
                            ]);
                            ListItem::new(line).style(if is_selected {
                                Theme::selected_row()
                            } else {
                                Style::default()
                            })
                        })
                        .collect();

                    let title = if suggs.len() > max_visible {
                        format!(" Commands [{}/{}] (Tab: complete, Enter: run) ", selected_idx + 1, suggs.len())
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
            }
        }
        InputMode::Filter => {
            let prompt_prefix = Theme::prompt_glyph();
            let base_label = if props.is_text_search {
                "Search: /"
            } else {
                "Filter (regex): /"
            };
            let label = if prompt_prefix != ":" {
                format!("{}{}", prompt_prefix, base_label)
            } else {
                base_label.to_string()
            };
            let (stats, hint) = if props.is_text_search {
                (
                    format!("[{} matches]", props.matched_count),
                    "  (Enter to finish, n/N next/prev, Esc to clear)",
                )
            } else {
                (
                    format!("[{}/{}]", props.matched_count, props.total_count),
                    "  (Enter to apply, Esc to clear)",
                )
            };
            let filter_text = Line::from(vec![
                Span::styled(label, Theme::prompt()),
                Span::styled(props.filter_input, Style::default().fg(Theme::fg())),
                Span::styled("█", Style::default().fg(Theme::yellow())), // Cursor
                Span::raw("  "),
                Span::styled(stats, Style::default().fg(Theme::dim())),
                Span::styled(hint, Style::default().fg(Theme::dim())),
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
                spans.push(Span::styled(format!("{} ", Theme::bullet_glyph()), style));
                spans.push(Span::styled(format!("{} ", msg), style));
                spans.push(Span::styled("│ ", Style::default().fg(Theme::border())));
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
                    Style::default().fg(Theme::yellow()),
                ));
            }

            if let Some((btn_label, btn_style)) = props.close_pf_button {
                let btn_text = format!(" [ {} ] ", btn_label);
                let btn_width = (btn_text.len() as u16).min(inner.width);
                let btn_x = inner.x + inner.width.saturating_sub(btn_width);
                let btn_area = Rect {
                    x: btn_x,
                    y: inner.y,
                    width: btn_width,
                    height: inner.height.min(1),
                };
                if let Some(rect_cell) = props.close_pf_rect {
                    *rect_cell.borrow_mut() = Some(btn_area);
                }

                let hints_area = Rect {
                    x: inner.x,
                    y: inner.y,
                    width: inner.width.saturating_sub(btn_width),
                    height: inner.height,
                };
                f.render_widget(Paragraph::new(Line::from(spans)), hints_area);
                f.render_widget(Paragraph::new(Line::from(vec![
                    Span::styled(btn_text, btn_style),
                ])), btn_area);
            } else {
                if let Some(rect_cell) = props.close_pf_rect {
                    *rect_cell.borrow_mut() = None;
                }
                f.render_widget(Paragraph::new(Line::from(spans)), inner);
            }
        }
    }

    if props.mode != &InputMode::Normal {
        if let Some(rect_cell) = props.close_pf_rect {
            *rect_cell.borrow_mut() = None;
        }
    }
}
