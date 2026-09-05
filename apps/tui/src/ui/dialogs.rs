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
        Modal::Scale { workload_name, current_replicas, input } => {
            let modal_area = centered_rect(45, 25, area);
            f.render_widget(Clear, modal_area);

            let block = Block::default()
                .borders(Borders::ALL)
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
        Modal::PortForward { pod_name, namespace, container_port, local_port_input } => {
            let modal_area = centered_rect(50, 30, area);
            f.render_widget(Clear, modal_area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Theme::ACCENT))
                .title(format!(" Start Port Forward: {} ({}) ", pod_name, namespace));

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

            let items: Vec<ListItem> = filtered
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let is_active = c.name == *current_context;
                    let is_sel = i == *selected_idx;
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

            let items: Vec<ListItem> = all_ns
                .iter()
                .enumerate()
                .map(|(i, name)| {
                    let is_active = name == current_namespace;
                    let is_sel = i == *selected_idx;
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

            let items: Vec<ListItem> = filtered
                .iter()
                .enumerate()
                .map(|(i, a)| {
                    let is_sel = i == *selected_idx;
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
    }
}
