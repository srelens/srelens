use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};

use crate::theme::Theme;
use crate::ui::help::centered_rect;

#[derive(Debug, Clone)]
pub enum Modal {
    Confirm {
        title: String,
        message: String,
        action_name: String,
        is_destructive: bool,
    },
    InputConfirm {
        title: String,
        message: String,
        action_name: String,
        required_input: String,
        current_input: String,
        is_destructive: bool,
    },
    Scale {
        workload_name: String,
        current_replicas: i32,
        input: String,
    },
    PortForward {
        pod_name: String,
        namespace: String,
        container_port: u16,
        local_port_input: String,
        kind: String,
    },
    ContainerPicker {
        pod_name: String,
        namespace: Option<String>,
        containers: Vec<String>,
        selected_idx: usize,
        action: ContainerAction,
    },
    ContextPicker {
        contexts: Vec<ContextPickerItem>,
        current_context: String,
        selected_idx: usize,
        filter: String,
    },
    NamespacePicker {
        namespaces: Vec<String>,
        current_namespace: String,
        selected_idx: usize,
        filter: String,
    },
    ActionPalette {
        resource_kind: String,
        resource_name: String,
        namespace: Option<String>,
        actions: Vec<QuickActionItem>,
        selected_idx: usize,
        filter: String,
    },
    MetricsTimeline(crate::views::metrics_panel_view::MetricsPanelState),
    ReasonRail {
        tallies: Vec<crate::views::reason_rail::ReasonTally>,
        selected_idx: usize,
        active_filter: Option<String>,
    },
    ThemePicker {
        selected_idx: usize,
        initial_theme_idx: usize,
    },
    FeatureBanner {
        show_on_startup: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickActionId {
    AskAi,
    PlaybookCrashLoop,
    PlaybookPending,
    PlaybookOom,
    PlaybookRollout,
    PlaybookEndpoints,
    PlaybookNodePressure,
    RelationshipTree,
    ViewLogs,
    OpenShell,
    PortForward,
    StopPortForward,
    Describe,
    ViewYaml,
    EditYaml,
    Scale,
    RolloutRestart,
    JumpToPods,
    InspectNode,
    CordonNode,
    DrainNode,
    Delete,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QuickActionItem {
    pub id: QuickActionId,
    pub key_hint: String,
    pub title: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContextPickerItem {
    pub name: String,
    pub cluster: String,
    pub server: String,
    pub namespace: String,
    pub is_local: bool,
    pub provider: Option<String>,
    pub source_file: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ContainerAction {
    Logs,
    Shell,
}

pub fn render_modal(f: &mut Frame, area: Rect, modal: &Modal) {
    match modal {
        Modal::Confirm { title, message, action_name, is_destructive } => {
            let modal_area = centered_rect(50, 30, area);
            f.render_widget(Clear, modal_area);

            let border_color = if *is_destructive { Theme::RED } else { Theme::ACCENT };
            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(Theme::border_type())
                .border_style(Style::default().fg(border_color))
                .title(format!(" {} ", title));

            let inner = block.inner(modal_area);
            f.render_widget(block, modal_area);

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(3),
                    Constraint::Length(2),
                ])
                .split(inner);

            let msg_widget = Paragraph::new(message.as_str())
                .wrap(Wrap { trim: true })
                .alignment(Alignment::Center);
            f.render_widget(msg_widget, chunks[0]);

            let prompt_line = Line::from(vec![
                Span::styled("Press ", Style::default().fg(Theme::DIM)),
                Span::styled("[Enter/y]", Style::default().fg(if *is_destructive { Theme::RED } else { Theme::GREEN }).add_modifier(Modifier::BOLD)),
                Span::styled(format!(" to {}", action_name), Style::default().fg(Theme::FG)),
                Span::styled("  |  ", Style::default().fg(Theme::DIM)),
                Span::styled("[Esc/n]", Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)),
                Span::styled(" to Cancel", Style::default().fg(Theme::DIM)),
            ]);
            let prompt_widget = Paragraph::new(prompt_line).alignment(Alignment::Center);
            f.render_widget(prompt_widget, chunks[1]);
        }
        Modal::InputConfirm { title, message, action_name: _, required_input, current_input, is_destructive } => {
            let modal_area = centered_rect(55, 34, area);
            f.render_widget(Clear, modal_area);

            let border_color = if *is_destructive { Theme::red() } else { Theme::accent() };
            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(Theme::border_type())
                .border_style(Style::default().fg(border_color))
                .title(Span::styled(format!(" {} ", title), Style::default().fg(border_color).add_modifier(Modifier::BOLD)));

            let inner = block.inner(modal_area);
            f.render_widget(block, modal_area);

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(3),
                    Constraint::Length(3),
                    Constraint::Length(2),
                ])
                .split(inner);

            let msg_widget = Paragraph::new(message.as_str())
                .wrap(Wrap { trim: true })
                .alignment(Alignment::Center);
            f.render_widget(msg_widget, chunks[0]);

            let is_matched = current_input == required_input;
            let input_block = Block::default()
                .borders(Borders::ALL)
                .border_type(Theme::border_type())
                .border_style(Style::default().fg(if is_matched { Theme::green() } else { Theme::yellow() }))
                .title(format!(" Type \"{}\" to confirm ", required_input));

            let input_widget = Paragraph::new(format!("{}█", current_input))
                .style(Style::default().fg(if is_matched { Theme::green() } else { Theme::fg() }).add_modifier(Modifier::BOLD))
                .alignment(Alignment::Center)
                .block(input_block);
            f.render_widget(input_widget, chunks[1]);

            let prompt_line = Line::from(vec![
                Span::styled("Press ", Style::default().fg(Theme::dim())),
                Span::styled("[Enter]", Style::default().fg(if is_matched { Theme::green() } else { Theme::dim() }).add_modifier(if is_matched { Modifier::BOLD } else { Modifier::empty() })),
                Span::styled(if is_matched { " Confirm" } else { " (type word to enable)" }, Style::default().fg(if is_matched { Theme::fg() } else { Theme::dim() })),
                Span::styled("  |  ", Style::default().fg(Theme::dim())),
                Span::styled("[Esc]", Style::default().fg(Theme::yellow()).add_modifier(Modifier::BOLD)),
                Span::styled(" Cancel", Style::default().fg(Theme::dim())),
            ]);
            let prompt_widget = Paragraph::new(prompt_line).alignment(Alignment::Center);
            f.render_widget(prompt_widget, chunks[2]);
        }
        Modal::Scale { workload_name, current_replicas, input } => {
            let modal_area = centered_rect(45, 25, area);
            f.render_widget(Clear, modal_area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(Theme::border_type())
                .border_style(Style::default().fg(Theme::ACCENT))
                .title(format!(" Scale Workload: {} ", workload_name));

            let inner = block.inner(modal_area);
            f.render_widget(block, modal_area);

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Length(3),
                    Constraint::Length(2),
                ])
                .split(inner);

            let info = Paragraph::new(format!("Current replicas: {}", current_replicas))
                .alignment(Alignment::Center);
            f.render_widget(info, chunks[0]);

            let input_block = Block::default()
                .borders(Borders::ALL)
                .border_type(Theme::border_type())
                .border_style(Style::default().fg(Theme::CYAN))
                .title(" Desired Replicas ");
            let input_widget = Paragraph::new(format!("{}█", input))
                .style(Style::default().fg(Theme::FG).add_modifier(Modifier::BOLD))
                .alignment(Alignment::Center)
                .block(input_block);
            f.render_widget(input_widget, chunks[1]);

            let hints = Paragraph::new(Line::from(vec![
                Span::styled("[Enter]", Theme::key_hint_key()),
                Span::styled(" Apply  ", Theme::key_hint_desc()),
                Span::styled("[Esc]", Theme::key_hint_key()),
                Span::styled(" Cancel", Theme::key_hint_desc()),
            ])).alignment(Alignment::Center);
            f.render_widget(hints, chunks[2]);
        }
        Modal::PortForward { pod_name, namespace, container_port, local_port_input, kind } => {
            let modal_area = centered_rect(50, 30, area);
            f.render_widget(Clear, modal_area);

            let title_str = if kind.is_empty() || kind == "Pod" {
                format!(" Start Port Forward: {} ({}) ", pod_name, namespace)
            } else {
                format!(" Start Port Forward: {}/{} ({}) ", kind, pod_name, namespace)
            };
            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(Theme::border_type())
                .border_style(Style::default().fg(Theme::ACCENT))
                .title(title_str);

            let inner = block.inner(modal_area);
            f.render_widget(block, modal_area);

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Length(3),
                    Constraint::Length(2),
                ])
                .split(inner);

            let info = Paragraph::new(format!("Target container port: {}", container_port))
                .alignment(Alignment::Center);
            f.render_widget(info, chunks[0]);

            let input_block = Block::default()
                .borders(Borders::ALL)
                .border_type(Theme::border_type())
                .border_style(Style::default().fg(Theme::CYAN))
                .title(" Local Port (127.0.0.1) ");
            let input_widget = Paragraph::new(format!("{}█", local_port_input))
                .style(Style::default().fg(Theme::FG).add_modifier(Modifier::BOLD))
                .alignment(Alignment::Center)
                .block(input_block);
            f.render_widget(input_widget, chunks[1]);

            let hints = Paragraph::new(Line::from(vec![
                Span::styled("[Enter]", Theme::key_hint_key()),
                Span::styled(" Forward  ", Theme::key_hint_desc()),
                Span::styled("[Esc]", Theme::key_hint_key()),
                Span::styled(" Cancel", Theme::key_hint_desc()),
            ])).alignment(Alignment::Center);
            f.render_widget(hints, chunks[2]);
        }
        Modal::ContainerPicker { pod_name, containers, selected_idx, action, .. } => {
            let modal_area = centered_rect(45, 40, area);
            f.render_widget(Clear, modal_area);

            let action_title = match action {
                ContainerAction::Logs => "View Logs for Container",
                ContainerAction::Shell => "Exec Shell into Container",
            };

            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(Theme::border_type())
                .border_style(Style::default().fg(Theme::ACCENT))
                .title(format!(" {} ({}) ", action_title, pod_name));

            let inner = block.inner(modal_area);
            f.render_widget(block, modal_area);

            let items: Vec<ListItem> = containers
                .iter()
                .enumerate()
                .map(|(i, name)| {
                    let is_sel = i == *selected_idx;
                    let style = if is_sel {
                        Theme::selected_row()
                    } else {
                        Style::default().fg(Theme::FG)
                    };
                    let prefix = if is_sel { "▶ " } else { "  " };
                    ListItem::new(format!("{}{}", prefix, name)).style(style)
                })
                .collect();

            let list = List::new(items);
            f.render_widget(list, inner);
        }
        Modal::ContextPicker { contexts, current_context, selected_idx, filter } => {
            let modal_area = centered_rect(65, 60, area);
            f.render_widget(Clear, modal_area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(Theme::border_type())
                .border_style(Style::default().fg(Theme::ACCENT))
                .title(" Switch Kubernetes Context (Type to filter, ↑/↓ Navigate, Enter Switch, Esc Close) ");

            let inner = block.inner(modal_area);
            f.render_widget(block, modal_area);

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3), // Search input box
                    Constraint::Min(5),    // Filtered list
                    Constraint::Length(1), // Footer hint
                ])
                .split(inner);

            // 1. Search input box
            let search_block = Block::default()
                .borders(Borders::ALL)
                .border_type(Theme::border_type())
                .border_style(Style::default().fg(Theme::CYAN))
                .title(" Filter Contexts ");
            let search_para = Paragraph::new(Line::from(vec![
                Span::styled(" / ", Style::default().fg(Theme::DIM)),
                Span::styled(filter.as_str(), Style::default().fg(Theme::FG).add_modifier(Modifier::BOLD)),
                Span::styled("█", Style::default().fg(Theme::CYAN)),
            ])).block(search_block);
            f.render_widget(search_para, chunks[0]);

            // 2. Filter contexts
            let lower_filter = filter.to_lowercase();
            let filtered: Vec<&ContextPickerItem> = contexts
                .iter()
                .filter(|c| {
                    if lower_filter.is_empty() {
                        true
                    } else {
                        c.name.to_lowercase().contains(&lower_filter)
                            || c.cluster.to_lowercase().contains(&lower_filter)
                            || c.provider.as_deref().unwrap_or("").to_lowercase().contains(&lower_filter)
                            || c.source_file.to_lowercase().contains(&lower_filter)
                    }
                })
                .collect();

            let visible_items = (chunks[1].height as usize / 2).max(1);
            let sel = (*selected_idx).min(filtered.len().saturating_sub(1));
            let start_idx = if sel >= visible_items {
                sel + 1 - visible_items
            } else {
                0
            };
            let end_idx = (start_idx + visible_items).min(filtered.len());

            let items: Vec<ListItem> = filtered[start_idx..end_idx]
                .iter()
                .enumerate()
                .map(|(rel_i, c)| {
                    let i = start_idx + rel_i;
                    let is_active = c.name == *current_context;
                    let is_sel = i == sel;
                    let color = Theme::context_color(&c.name, c.is_local);

                    let prefix = if is_active { "★ " } else if is_sel { "▶ " } else { "  " };

                    let provider_badge = if let Some(p) = &c.provider {
                        format!("[{}] ", p)
                    } else if c.is_local {
                        "[local] ".to_string()
                    } else {
                        "[remote] ".to_string()
                    };

                    let active_badge = if is_active { " (active)" } else { "" };

                    let ns_info = if !c.namespace.is_empty() {
                        format!(" ns:[{}]", c.namespace)
                    } else {
                        String::new()
                    };

                    let line1 = Line::from(vec![
                        Span::styled(prefix, if is_sel { Theme::selected_row() } else { Style::default().fg(color) }),
                        Span::styled(provider_badge, Style::default().fg(Theme::DIM)),
                        Span::styled(c.name.clone(), Style::default().fg(color).add_modifier(Modifier::BOLD)),
                        Span::styled(active_badge, Style::default().fg(Theme::GREEN).add_modifier(Modifier::BOLD)),
                        Span::styled(ns_info, Style::default().fg(Theme::CYAN)),
                    ]);

                    let file_name = std::path::Path::new(&c.source_file)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(&c.source_file);

                    let line2 = Line::from(vec![
                        Span::raw("    "),
                        Span::styled(format!("cluster: {}  •  file: {}", c.cluster, file_name), Style::default().fg(Theme::DIM)),
                    ]);

                    let style = if is_sel {
                        Theme::selected_row()
                    } else {
                        Style::default()
                    };

                    ListItem::new(vec![line1, line2]).style(style)
                })
                .collect();

            let list = List::new(items);
            f.render_widget(list, chunks[1]);

            // 3. Footer hint
            let footer = Paragraph::new(Line::from(vec![
                Span::styled(format!(" Showing {}/{} contexts  •  ", filtered.len(), contexts.len()), Style::default().fg(Theme::DIM)),
                Span::styled("Enter", Style::default().fg(Theme::ACCENT).add_modifier(Modifier::BOLD)),
                Span::styled(": Switch  ", Style::default().fg(Theme::DIM)),
                Span::styled("Esc", Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)),
                Span::styled(": Cancel", Style::default().fg(Theme::DIM)),
            ])).alignment(Alignment::Center);
            f.render_widget(footer, chunks[2]);
        }
        Modal::NamespacePicker { namespaces, current_namespace, selected_idx, filter } => {
            let modal_area = centered_rect(50, 55, area);
            f.render_widget(Clear, modal_area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(Theme::border_type())
                .border_style(Style::default().fg(Theme::ACCENT))
                .title(" Switch Namespace (0: All Namespaces, ↑/↓ Navigate, Enter Select) ");

            let inner = block.inner(modal_area);
            f.render_widget(block, modal_area);

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Min(5),
                ])
                .split(inner);

            let filter_block = Block::default()
                .borders(Borders::ALL)
                .border_type(Theme::border_type())
                .border_style(Style::default().fg(Theme::CYAN))
                .title(" Filter ");
            let filter_widget = Paragraph::new(format!("{}█", filter))
                .block(filter_block);
            f.render_widget(filter_widget, chunks[0]);

            let all_ns: Vec<String> = namespaces
                .iter()
                .filter(|n| n.contains(filter.as_str()))
                .cloned()
                .collect();

            let visible_height = (chunks[1].height as usize).max(1);
            let sel = (*selected_idx).min(all_ns.len().saturating_sub(1));
            let start_idx = if sel >= visible_height {
                sel + 1 - visible_height
            } else {
                0
            };
            let end_idx = (start_idx + visible_height).min(all_ns.len());

            let items: Vec<ListItem> = all_ns[start_idx..end_idx]
                .iter()
                .enumerate()
                .map(|(rel_i, name)| {
                    let i = start_idx + rel_i;
                    let is_active = name == current_namespace;
                    let is_sel = i == sel;
                    let prefix = if is_active { "★ " } else if is_sel { "▶ " } else { "  " };
                    let style = if is_sel {
                        Theme::selected_row()
                    } else if is_active {
                        Style::default().fg(Theme::GREEN).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Theme::FG)
                    };
                    ListItem::new(format!("{}{}", prefix, name)).style(style)
                })
                .collect();

            let list = List::new(items);
            f.render_widget(list, chunks[1]);
        }
        Modal::ActionPalette {
            resource_kind,
            resource_name,
            namespace,
            actions,
            selected_idx,
            filter,
        } => {
            let modal_area = centered_rect(65, 65, area);
            f.render_widget(Clear, modal_area);

            let ns_str = namespace.as_deref().map(|n| format!(" ({})", n)).unwrap_or_default();
            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(Theme::border_type())
                .border_style(Style::default().fg(Theme::ACCENT))
                .title(Span::styled(
                    format!(" ⚡ Actions: {}/{}{} [Type to filter, ↑/↓ Navigate, Enter Run, Esc Close] ", resource_kind, resource_name, ns_str),
                    Theme::title(),
                ));

            let inner = block.inner(modal_area);
            f.render_widget(block, modal_area);

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3), // Search box
                    Constraint::Min(5),    // Actions list
                    Constraint::Length(1), // Footer hint
                ])
                .split(inner);

            // 1. Search box
            let search_block = Block::default()
                .borders(Borders::ALL)
                .border_type(Theme::border_type())
                .border_style(Style::default().fg(Theme::CYAN))
                .title(" Filter Actions ");
            let search_para = Paragraph::new(Line::from(vec![
                Span::styled(" / ", Style::default().fg(Theme::DIM)),
                Span::styled(filter.as_str(), Style::default().fg(Theme::FG).add_modifier(Modifier::BOLD)),
                Span::styled("█", Style::default().fg(Theme::CYAN)),
            ])).block(search_block);
            f.render_widget(search_para, chunks[0]);

            // 2. Filtered actions
            let lower_filter = filter.to_lowercase();
            let filtered: Vec<&QuickActionItem> = actions
                .iter()
                .filter(|a| {
                    if lower_filter.is_empty() {
                        true
                    } else {
                        a.title.to_lowercase().contains(&lower_filter)
                            || a.key_hint.to_lowercase().contains(&lower_filter)
                            || a.description.to_lowercase().contains(&lower_filter)
                    }
                })
                .collect();

            let visible_items = (chunks[1].height as usize / 2).max(1);
            let sel = (*selected_idx).min(filtered.len().saturating_sub(1));
            let start_idx = if sel >= visible_items {
                sel + 1 - visible_items
            } else {
                0
            };
            let end_idx = (start_idx + visible_items).min(filtered.len());

            let items: Vec<ListItem> = filtered[start_idx..end_idx]
                .iter()
                .enumerate()
                .map(|(rel_i, a)| {
                    let i = start_idx + rel_i;
                    let is_sel = i == sel;
                    let prefix = if is_sel { "▶ " } else { "  " };

                    let line1 = Line::from(vec![
                        Span::styled(prefix, if is_sel { Theme::selected_row() } else { Style::default().fg(Theme::ACCENT) }),
                        Span::styled(format!("[{}] ", a.key_hint), Style::default().fg(Theme::CYAN).add_modifier(Modifier::BOLD)),
                        Span::styled(a.title.clone(), Style::default().fg(Theme::FG).add_modifier(Modifier::BOLD)),
                    ]);

                    let line2 = Line::from(vec![
                        Span::raw("      "),
                        Span::styled(a.description.clone(), Style::default().fg(Theme::DIM)),
                    ]);

                    let style = if is_sel {
                        Theme::selected_row()
                    } else {
                        Style::default()
                    };

                    ListItem::new(vec![line1, line2]).style(style)
                })
                .collect();

            let list = List::new(items);
            f.render_widget(list, chunks[1]);

            // 3. Footer hint
            let footer = Paragraph::new(Line::from(vec![
                Span::styled(format!(" Showing {}/{} actions  •  ", filtered.len(), actions.len()), Style::default().fg(Theme::DIM)),
                Span::styled("Enter", Style::default().fg(Theme::ACCENT).add_modifier(Modifier::BOLD)),
                Span::styled(": Run Action  ", Style::default().fg(Theme::DIM)),
                Span::styled("Esc", Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)),
                Span::styled(": Cancel", Style::default().fg(Theme::DIM)),
            ])).alignment(Alignment::Center);
            f.render_widget(footer, chunks[2]);
        }
        Modal::MetricsTimeline(state) => {
            crate::views::metrics_panel_view::render_metrics_panel_modal(f, area, state);
        }
        Modal::ReasonRail { tallies, selected_idx, active_filter } => {
            crate::views::reason_rail::render_reason_rail_modal(f, area, tallies, *selected_idx, active_filter.as_deref());
        }
        Modal::ThemePicker { selected_idx, initial_theme_idx: _ } => {
            let modal_area = centered_rect(75, 75, area);
            f.render_widget(Clear, modal_area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(Theme::border_type())
                .border_style(Style::default().fg(Theme::accent()))
                .title(Span::styled(" Themes & Visual Styles (↑/↓ or j/k Preview • Enter Select • Esc Cancel) ", Style::default().fg(Theme::accent()).add_modifier(Modifier::BOLD)));

            let inner = block.inner(modal_area);
            f.render_widget(block, modal_area);

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(6),
                    Constraint::Length(1),
                ])
                .split(inner);

            let active_idx = Theme::active_index();
            let items: Vec<ListItem> = Theme::all_themes()
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let is_sel = i == *selected_idx;
                    let is_current = i == active_idx;
                    let cursor = if is_sel { "▶ " } else { "  " };

                    let visual_badge = match p.header_style {
                        crate::theme::HeaderStyle::FinoTime => "[Fino • Live Clock • Tree] ",
                        crate::theme::HeaderStyle::Minimal => "[Minimalist Chrome] ",
                        crate::theme::HeaderStyle::Standard => {
                            if p.border_type == ratatui::widgets::BorderType::Thick {
                                "[Thick Neon Borders] "
                            } else {
                                ""
                            }
                        }
                    };

                    let mut spans = vec![
                        Span::styled(cursor, if is_sel { Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD) } else { Style::default().fg(Theme::dim()) }),
                        Span::styled(format!("{:<20}", p.display_name), if is_sel { Style::default().fg(Theme::fg()).add_modifier(Modifier::BOLD) } else { Style::default().fg(Theme::fg()) }),
                        Span::raw(" "),
                        Span::styled("■ ", Style::default().fg(p.accent)),
                        Span::styled("■ ", Style::default().fg(p.cyan)),
                        Span::styled("■ ", Style::default().fg(p.green)),
                        Span::styled("■ ", Style::default().fg(p.yellow)),
                        Span::styled("■ ", Style::default().fg(p.red)),
                        Span::raw(" "),
                    ];

                    if !visual_badge.is_empty() {
                        spans.push(Span::styled(visual_badge, Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD)));
                    }

                    if is_current {
                        spans.push(Span::styled("✔ ACTIVE ", Style::default().fg(p.green).add_modifier(Modifier::BOLD)));
                    }

                    spans.push(Span::styled(format!("— {}", p.description), Style::default().fg(Theme::dim())));

                    let row_style = if is_sel {
                        Theme::selected_row()
                    } else {
                        Style::default()
                    };

                    ListItem::new(Line::from(spans)).style(row_style)
                })
                .collect();

            let list = List::new(items);
            f.render_widget(list, chunks[0]);

            let active_name = Theme::active_palette().display_name;
            let footer = Paragraph::new(Line::from(vec![
                Span::styled("Live Previewing: ", Style::default().fg(Theme::dim())),
                Span::styled(active_name, Style::default().fg(Theme::accent()).add_modifier(Modifier::BOLD)),
                Span::styled("  •  [Enter] Save  [Esc] Cancel", Style::default().fg(Theme::dim())),
            ])).alignment(Alignment::Center);
            f.render_widget(footer, chunks[1]);
        }
        Modal::FeatureBanner { show_on_startup } => {
            render_feature_banner_modal(f, area, *show_on_startup);
        }
    }
}

pub fn render_feature_banner_modal(f: &mut Frame, area: Rect, show_on_startup: bool) {
    let modal_width = (area.width.saturating_sub(4)).min(94).max(48);
    let modal_height = (area.height.saturating_sub(2)).min(21).max(14);
    let modal_x = area.x + (area.width.saturating_sub(modal_width)) / 2;
    let modal_y = area.y + (area.height.saturating_sub(modal_height)) / 2;
    let modal_area = Rect::new(modal_x, modal_y, modal_width, modal_height);

    f.render_widget(Clear, modal_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(Theme::border_type())
        .border_style(Style::default().fg(Theme::cyan()))
        .title(Span::styled(
            " ✨ Welcome to SRElens — Feature Highlights ✨ ",
            Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(modal_area);
    f.render_widget(block, modal_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // Top description
            Constraint::Min(7),   // Features list
            Constraint::Length(3), // Checkbox and key hints
        ])
        .split(inner);

    // 1. Header description
    let header_lines = vec![
        Line::from(vec![
            Span::styled(
                "Kubernetes control room with high-velocity SRE troubleshooting capabilities.",
                Style::default().fg(Theme::fg()).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                "Key built-in features you should know (press [1-7] to jump directly, or type ':' for command prompt):",
                Style::default().fg(Theme::dim()),
            ),
        ]),
    ];
    f.render_widget(Paragraph::new(header_lines), chunks[0]);

    // 2. Feature highlights
    let features: &[(&str, &str, &str, &str, &str)] = &[
        ("[1]", ":helm",        "[Helm 3]",        "Helm 3 release revisions, rollback status, values & manifests", ":helm [ns]"),
        ("[2]", ":overview",    "[Cluster]",       "Cluster overview, health summary & node/pod capacity",          ":overview"),
        ("[3]", ":gpuinfo",     "[Hardware]",      "GPU hardware inspector, specs & per-pod VRAM allocations",     ":gpuinfo"),
        ("[4]", ":workloads",   "[Workload]",      "Unified workloads view (Pods, Deployments, STS, DS, Jobs)",     ":workloads [ns]"),
        ("[5]", ":ai",          "[AI Assistant]",  "Interactive AI troubleshooting chat for automated RCA",         ":ai"),
        ("[6]", ":ai-settings", "[AI Config]",     "Configure AI providers (Claude, OpenAI, Gemini), models & keys", ":ai-settings"),
        ("[7]", ":config",      "[Lens Settings]", "Lens settings: popup width, visible rows, text scale & banner",  ":config"),
    ];

    let inner_w = chunks[1].width as usize;
    let items: Vec<ListItem> = features
        .iter()
        .map(|(num, cmd, cat, desc, syntax)| {
            let mut spans = vec![
                Span::styled(format!("{num} "), Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{:<13}", cmd), Style::default().fg(Theme::yellow()).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{:<15}", cat), Style::default().fg(Theme::accent()).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{:<desc_len$}", desc, desc_len = if inner_w >= 85 { 44 } else { 32 }), Style::default().fg(Theme::fg())),
            ];
            if inner_w >= 80 {
                spans.push(Span::styled(format!("  ({})", syntax), Style::default().fg(Theme::dim())));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    f.render_widget(List::new(items), chunks[1]);

    // 3. Footer with Startup Checkbox & Key hints
    let checkbox_spans = if show_on_startup {
        vec![
            Span::styled(" [●] ", Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD)),
            Span::styled("Show this feature banner on startup", Style::default().fg(Theme::fg()).add_modifier(Modifier::BOLD)),
            Span::styled(" (Press ", Style::default().fg(Theme::dim())),
            Span::styled("t", Style::default().fg(Theme::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled(" to toggle)", Style::default().fg(Theme::dim())),
        ]
    } else {
        vec![
            Span::styled(" [○] ", Style::default().fg(Theme::dim())),
            Span::styled("Show this feature banner on startup", Style::default().fg(Theme::dim())),
            Span::styled(" (Currently ", Style::default().fg(Theme::dim())),
            Span::styled("Disabled", Style::default().fg(Theme::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled(" • Press ", Style::default().fg(Theme::dim())),
            Span::styled("t", Style::default().fg(Theme::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled(" to enable)", Style::default().fg(Theme::dim())),
        ]
    };

    let footer_lines = vec![
        Line::from(vec![
            Span::styled("─".repeat(inner_w.min(90)), Style::default().fg(Theme::border())),
        ]),
        Line::from(checkbox_spans),
        Line::from(vec![
            Span::styled(" Press ", Style::default().fg(Theme::dim())),
            Span::styled("Enter", Style::default().fg(Theme::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled(", ", Style::default().fg(Theme::dim())),
            Span::styled("Esc", Style::default().fg(Theme::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled(", or ", Style::default().fg(Theme::dim())),
            Span::styled("q", Style::default().fg(Theme::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled(" to dismiss  |  Press ", Style::default().fg(Theme::dim())),
            Span::styled("1-7", Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD)),
            Span::styled(" to jump directly  |  ", Style::default().fg(Theme::dim())),
            Span::styled(":banner", Style::default().fg(Theme::accent())),
            Span::styled(" to reopen anytime", Style::default().fg(Theme::dim())),
        ]),
    ];

    f.render_widget(Paragraph::new(footer_lines), chunks[2]);
}
