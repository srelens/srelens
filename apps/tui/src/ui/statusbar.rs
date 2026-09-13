use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

use crate::commands::{command_suggestions, CommandDef};
use crate::theme::Theme;
use crate::tui_config::CommandPopupDensity;

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
    pub command_popup_max_width: Option<u16>,
    pub command_popup_max_visible: Option<usize>,
    pub command_popup_density: Option<CommandPopupDensity>,
}

pub fn command_popup_rect(
    area: Rect,
    item_count: usize,
    max_width: u16,
    max_visible: usize,
    density: CommandPopupDensity,
) -> Rect {
    let visible_count = item_count.min(max_visible);
    let item_h = density.item_height();
    let content_height = visible_count.min(u16::MAX as usize) as u16;
    let popup_height = content_height.saturating_mul(item_h).saturating_add(2).min(area.y);
    let popup_width = area.width.saturating_sub(4).min(max_width);
    Rect {
        x: area.x + 2,
        y: area.y.saturating_sub(popup_height),
        width: popup_width,
        height: popup_height,
    }
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
                    let max_width = props.command_popup_max_width.unwrap_or(65);
                    let max_visible = props.command_popup_max_visible.unwrap_or(6);
                    let density = props.command_popup_density.unwrap_or_default();
                    let density = if area.y.saturating_sub(2) < density.item_height() {
                        CommandPopupDensity::Compact
                    } else { density };
                    let popup_area = command_popup_rect(area, suggs.len(), max_width, max_visible, density);
                    if popup_area.height < 3 || popup_area.width < 3 { return; }
                    f.render_widget(Clear, popup_area);
                    let inner_height = popup_area.height.saturating_sub(2);
                    let item_lines = density.item_height();
                    let max_fits = (inner_height / item_lines) as usize;
                    let visible_count = suggs.len().min(max_visible).min(max_fits.max(1));

                    // Compute window offset to keep selected_idx visible
                    let scroll_offset = if selected_idx >= visible_count {
                        selected_idx + 1 - visible_count
                    } else {
                        0
                    };

                    let visible_slice = &suggs[scroll_offset..(scroll_offset + visible_count).min(suggs.len())];
                    let popup_inner_w = popup_area.width.saturating_sub(2) as usize;

                    let items: Vec<ListItem> = visible_slice
                        .iter()
                        .enumerate()
                        .map(|(rel_i, (cmd, _score))| {
                            let abs_i = scroll_offset + rel_i;
                            let is_selected = abs_i == selected_idx;
                            let prefix = if is_selected { "▶ " } else { "  " };

                            match density {
                                CommandPopupDensity::ExtraLarge => {
                                    // 3-line spacious card layout with maximum readability & detail
                                    let alias_str = if !cmd.aliases.is_empty() {
                                        format!(" ({})", cmd.aliases.join(", "))
                                    } else {
                                        String::new()
                                    };
                                    let cat_badge = format!("[{}]", cmd.category());
                                    let name_text = format!("{}{}{}", prefix, cmd.name.to_uppercase(), alias_str);

                                    let name_len = name_text.chars().count();
                                    let badge_len = cat_badge.chars().count();
                                    let spacer_len = popup_inner_w.saturating_sub(name_len + badge_len + 1).max(2);
                                    let spacer = " ".repeat(spacer_len);

                                    let line1 = Line::from(vec![
                                        Span::styled(
                                            name_text,
                                            if is_selected {
                                                Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD)
                                            } else {
                                                Style::default().fg(Theme::fg()).add_modifier(Modifier::BOLD)
                                            },
                                        ),
                                        Span::raw(spacer),
                                        Span::styled(
                                            cat_badge,
                                            if is_selected {
                                                Style::default().fg(Theme::accent()).add_modifier(Modifier::BOLD)
                                            } else {
                                                Style::default().fg(Theme::dim())
                                            },
                                        ),
                                    ]);

                                    let line2 = Line::from(vec![
                                        Span::raw("    "),
                                        Span::styled(
                                            cmd.description.clone(),
                                            if is_selected {
                                                Style::default().fg(Theme::fg())
                                            } else {
                                                Style::default().fg(Theme::dim())
                                            },
                                        ),
                                    ]);

                                    let syntax = cmd.syntax_hint();
                                    let mut line3_spans = vec![
                                        Span::raw("    "),
                                        Span::styled("Usage: ", Style::default().fg(Theme::dim())),
                                        Span::styled(
                                            if !syntax.is_empty() { syntax.to_string() } else { format!(":{}", cmd.name) },
                                            if is_selected {
                                                Style::default().fg(Theme::yellow()).add_modifier(Modifier::BOLD)
                                            } else {
                                                Style::default().fg(Theme::yellow())
                                            },
                                        ),
                                    ];
                                    if !cmd.aliases.is_empty() {
                                        line3_spans.push(Span::styled("  |  Aliases: ", Style::default().fg(Theme::dim())));
                                        line3_spans.push(Span::styled(
                                            cmd.aliases.join(", "),
                                            if is_selected {
                                                Style::default().fg(Theme::fg())
                                            } else {
                                                Style::default().fg(Theme::dim())
                                            },
                                        ));
                                    }
                                    let line3 = Line::from(line3_spans);

                                    ListItem::new(vec![line1, line2, line3]).style(if is_selected {
                                        Theme::selected_row()
                                    } else {
                                        Style::default()
                                    })
                                }
                                CommandPopupDensity::Large => {
                                    // 2-line spacious card layout with bold large typography
                                    let alias_str = if !cmd.aliases.is_empty() {
                                        format!(" ({})", cmd.aliases.join(", "))
                                    } else {
                                        String::new()
                                    };
                                    let cat_badge = format!("[{}]", cmd.category());
                                    let name_text = format!("{}{}{}", prefix, cmd.name.to_uppercase(), alias_str);

                                    let name_len = name_text.chars().count();
                                    let badge_len = cat_badge.chars().count();
                                    let spacer_len = popup_inner_w.saturating_sub(name_len + badge_len + 1).max(2);
                                    let spacer = " ".repeat(spacer_len);

                                    let line1 = Line::from(vec![
                                        Span::styled(
                                            name_text,
                                            if is_selected {
                                                Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD)
                                            } else {
                                                Style::default().fg(Theme::fg()).add_modifier(Modifier::BOLD)
                                            },
                                        ),
                                        Span::raw(spacer),
                                        Span::styled(
                                            cat_badge,
                                            if is_selected {
                                                Style::default().fg(Theme::accent()).add_modifier(Modifier::BOLD)
                                            } else {
                                                Style::default().fg(Theme::dim())
                                            },
                                        ),
                                    ]);

                                    let mut line2_spans = vec![
                                        Span::raw("    "),
                                        Span::styled(
                                            cmd.description.clone(),
                                            if is_selected {
                                                Style::default().fg(Theme::fg())
                                            } else {
                                                Style::default().fg(Theme::dim())
                                            },
                                        ),
                                    ];

                                    let syntax = cmd.syntax_hint();
                                    if !syntax.is_empty() && popup_inner_w >= 65 {
                                        line2_spans.push(Span::styled("  •  ", Style::default().fg(Theme::dim())));
                                        line2_spans.push(Span::styled(
                                            syntax.to_string(),
                                            if is_selected {
                                                Style::default().fg(Theme::yellow()).add_modifier(Modifier::BOLD)
                                            } else {
                                                Style::default().fg(Theme::yellow())
                                            },
                                        ));
                                    }

                                    let line2 = Line::from(line2_spans);

                                    ListItem::new(vec![line1, line2]).style(if is_selected {
                                        Theme::selected_row()
                                    } else {
                                        Style::default()
                                    })
                                }
                                CommandPopupDensity::Standard => {
                                    // 1-line enriched layout with category badge and syntax hint
                                    let alias_str = if !cmd.aliases.is_empty() {
                                        format!(" ({})", cmd.aliases.join(", "))
                                    } else {
                                        String::new()
                                    };
                                    let name_col = format!("{}{}{}", prefix, cmd.name, alias_str);
                                    let pad_width = if popup_inner_w >= 90 { 26 } else { 20 };
                                    let padded_name = format!("{:<pad_width$}", name_col, pad_width = pad_width);

                                    let mut spans = vec![
                                        Span::styled(
                                            padded_name,
                                            if is_selected {
                                                Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD)
                                            } else {
                                                Style::default().fg(Theme::fg()).add_modifier(Modifier::BOLD)
                                            },
                                        ),
                                    ];

                                    if popup_inner_w >= 60 {
                                        spans.push(Span::styled(
                                            format!("[{}] ", cmd.category()),
                                            if is_selected {
                                                Style::default().fg(Theme::accent())
                                            } else {
                                                Style::default().fg(Theme::dim())
                                            },
                                        ));
                                    }

                                    spans.push(Span::styled(
                                        format!(" {}", cmd.description),
                                        if is_selected {
                                            Style::default().fg(Theme::fg())
                                        } else {
                                            Style::default().fg(Theme::dim())
                                        },
                                    ));

                                    let syntax = cmd.syntax_hint();
                                    if !syntax.is_empty() && popup_inner_w >= 85 {
                                        spans.push(Span::styled("  |  ", Style::default().fg(Theme::dim())));
                                        spans.push(Span::styled(
                                            syntax.to_string(),
                                            if is_selected {
                                                Style::default().fg(Theme::yellow())
                                            } else {
                                                Style::default().fg(Theme::dim())
                                            },
                                        ));
                                    }

                                    ListItem::new(Line::from(spans)).style(if is_selected {
                                        Theme::selected_row()
                                    } else {
                                        Style::default()
                                    })
                                }
                                CommandPopupDensity::Compact => {
                                    // 1-line compact layout: clean, condensed
                                    let alias_str = if !cmd.aliases.is_empty() {
                                        format!(" ({})", cmd.aliases.join(", "))
                                    } else {
                                        String::new()
                                    };
                                    let name_col = format!("{}{}{}", prefix, cmd.name, alias_str);
                                    let pad_width = if popup_inner_w >= 90 { 24 } else { 18 };
                                    let padded_name = format!("{:<pad_width$}", name_col, pad_width = pad_width);

                                    let mut spans = vec![
                                        Span::styled(
                                            padded_name,
                                            if is_selected {
                                                Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD)
                                            } else {
                                                Style::default().fg(Theme::fg())
                                            },
                                        ),
                                        Span::styled(
                                            format!(" {}", cmd.description),
                                            if is_selected {
                                                Style::default().fg(Theme::fg())
                                            } else {
                                                Style::default().fg(Theme::dim())
                                            },
                                        ),
                                    ];

                                    let syntax = cmd.syntax_hint();
                                    if !syntax.is_empty() && popup_inner_w >= 95 {
                                        spans.push(Span::styled("  |  ", Style::default().fg(Theme::dim())));
                                        spans.push(Span::styled(
                                            syntax.to_string(),
                                            if is_selected {
                                                Style::default().fg(Theme::yellow())
                                            } else {
                                                Style::default().fg(Theme::dim())
                                            },
                                        ));
                                    }

                                    ListItem::new(Line::from(spans)).style(if is_selected {
                                        Theme::selected_row()
                                    } else {
                                        Style::default()
                                    })
                                }
                            }
                        })
                        .collect();

                    let title = if suggs.len() > visible_count {
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
                ("<d>", "Describe"),
                ("<y>", "YAML"),
                ("<e>", "Edit"),
                ("<^d>", "Delete"),
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
                let label = if props.is_text_search { "Search: " } else { "Filter: " };
                spans.push(Span::styled(label, Theme::header_label()));
                let stats = if props.is_text_search {
                    if props.matched_count == 0 {
                        format!("\"{}\" [0 matches]", props.filter_input)
                    } else if props.matched_count == 1 {
                        format!("\"{}\" [1 match, n/N]", props.filter_input)
                    } else {
                        format!("\"{}\" [{} matches, n/N]", props.filter_input, props.matched_count)
                    }
                } else {
                    format!("\"{}\" [{}/{}]", props.filter_input, props.matched_count, props.total_count)
                };
                spans.push(Span::styled(
                    stats,
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
