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
    NodeSsh {
        node_name: String,
        destination_input: String,
        cursor_pos: usize,
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
        update_available: Option<String>,
    },
    AddCluster {
        input: String,
        cursor_pos: usize,
        error_message: Option<String>,
        preview_contexts: Vec<String>,
    },
    Diagnosis {
        resource_kind: String,
        resource_name: String,
        namespace: Option<String>,
        report: srelens_kube::diagnose::DiagnosticReport,
        scroll_offset: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickActionId {
    DiagnoseResource,
    AskAi,
    PlaybookCrashLoop,
    PlaybookPending,
    PlaybookOom,
    PlaybookRollout,
    PlaybookEndpoints,
    PlaybookNodePressure,
    PlaybookArgoProgressing,
    ArgoDetails,
    ArgoSync,
    ArgoRefresh,
    ArgoOpenGit,
    RelationshipTree,
    ViewLogs,
    OpenShell,
    NodeSsh,
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

pub fn format_action_display(action_name: &str) -> String {
    if let Some(payload_str) = action_name.strip_prefix("argo_sync:") {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(payload_str) {
            if val.get("prune").and_then(|p| p.as_bool()).unwrap_or(false) {
                return "Sync with Prune".to_string();
            }
        }
        return "Sync Application".to_string();
    }
    if let Some(payload_str) = action_name.strip_prefix("argo_toggle_auto:") {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(payload_str) {
            if val.get("enable").and_then(|e| e.as_bool()).unwrap_or(false) {
                return "Enable Auto-Sync".to_string();
            } else {
                return "Pause Auto-Sync".to_string();
            }
        }
        return "Toggle Auto-Sync".to_string();
    }
    if action_name.starts_with("helm-rollback:") {
        return "Rollback Release".to_string();
    }
    if action_name.starts_with("helm-uninstall:") {
        return "Uninstall Release".to_string();
    }
    if action_name.starts_with("stop-pf:") {
        return "Stop Port Forward".to_string();
    }
    if action_name.starts_with("delete:") || action_name.starts_with("bulk_delete:") {
        return "Delete".to_string();
    }
    if action_name.starts_with("restart:") {
        return "Restart".to_string();
    }
    if action_name.starts_with("drain:") {
        return "Drain Node".to_string();
    }
    if action_name.starts_with("cordon:") {
        return "Cordon Node".to_string();
    }
    if action_name.starts_with("uncordon:") {
        return "Uncordon Node".to_string();
    }
    action_name.to_string()
}

pub fn render_modal(f: &mut Frame, area: Rect, modal: &Modal) {
    match modal {
        Modal::Confirm {
            title,
            message,
            action_name,
            is_destructive,
        } => {
            let modal_area = centered_rect(50, 30, area);
            f.render_widget(Clear, modal_area);

            let border_color = if *is_destructive {
                Theme::RED
            } else {
                Theme::ACCENT
            };
            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(Theme::border_type())
                .border_style(Style::default().fg(border_color))
                .title(format!(" {} ", title));

            let inner = block.inner(modal_area);
            f.render_widget(block, modal_area);

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(3), Constraint::Length(2)])
                .split(inner);

            let msg_widget = Paragraph::new(message.as_str())
                .wrap(Wrap { trim: true })
                .alignment(Alignment::Center);
            f.render_widget(msg_widget, chunks[0]);

            let display_action = format_action_display(action_name);
            let prompt_line = Line::from(vec![
                Span::styled("Press ", Style::default().fg(Theme::DIM)),
                Span::styled(
                    "[Enter/y]",
                    Style::default()
                        .fg(if *is_destructive {
                            Theme::RED
                        } else {
                            Theme::GREEN
                        })
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" to {}", display_action),
                    Style::default().fg(Theme::FG),
                ),
                Span::styled("  |  ", Style::default().fg(Theme::DIM)),
                Span::styled(
                    "[Esc/n]",
                    Style::default()
                        .fg(Theme::YELLOW)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" to Cancel", Style::default().fg(Theme::DIM)),
            ]);
            let prompt_widget = Paragraph::new(prompt_line).alignment(Alignment::Center);
            f.render_widget(prompt_widget, chunks[1]);
        }
        Modal::InputConfirm {
            title,
            message,
            action_name: _,
            required_input,
            current_input,
            is_destructive,
        } => {
            let modal_area = centered_rect(55, 34, area);
            f.render_widget(Clear, modal_area);

            let border_color = if *is_destructive {
                Theme::red()
            } else {
                Theme::accent()
            };
            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(Theme::border_type())
                .border_style(Style::default().fg(border_color))
                .title(Span::styled(
                    format!(" {} ", title),
                    Style::default()
                        .fg(border_color)
                        .add_modifier(Modifier::BOLD),
                ));

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
                .border_style(Style::default().fg(if is_matched {
                    Theme::green()
                } else {
                    Theme::yellow()
                }))
                .title(format!(" Type \"{}\" to confirm ", required_input));

            let input_widget = Paragraph::new(format!("{}█", current_input))
                .style(
                    Style::default()
                        .fg(if is_matched {
                            Theme::green()
                        } else {
                            Theme::fg()
                        })
                        .add_modifier(Modifier::BOLD),
                )
                .alignment(Alignment::Center)
                .block(input_block);
            f.render_widget(input_widget, chunks[1]);

            let prompt_line = Line::from(vec![
                Span::styled("Press ", Style::default().fg(Theme::dim())),
                Span::styled(
                    "[Enter]",
                    Style::default()
                        .fg(if is_matched {
                            Theme::green()
                        } else {
                            Theme::dim()
                        })
                        .add_modifier(if is_matched {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ),
                Span::styled(
                    if is_matched {
                        " Confirm"
                    } else {
                        " (type word to enable)"
                    },
                    Style::default().fg(if is_matched {
                        Theme::fg()
                    } else {
                        Theme::dim()
                    }),
                ),
                Span::styled("  |  ", Style::default().fg(Theme::dim())),
                Span::styled(
                    "[Esc]",
                    Style::default()
                        .fg(Theme::yellow())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" Cancel", Style::default().fg(Theme::dim())),
            ]);
            let prompt_widget = Paragraph::new(prompt_line).alignment(Alignment::Center);
            f.render_widget(prompt_widget, chunks[2]);
        }
        Modal::Scale {
            workload_name,
            current_replicas,
            input,
        } => {
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
            ]))
            .alignment(Alignment::Center);
            f.render_widget(hints, chunks[2]);
        }
        Modal::NodeSsh {
            node_name,
            destination_input,
            cursor_pos,
        } => {
            let modal_area = centered_rect(55, 30, area);
            f.render_widget(Clear, modal_area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(Theme::border_type())
                .border_style(Style::default().fg(Theme::ACCENT))
                .title(format!(" 🔑 SSH into Node: {} ", node_name));

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

            let info = Paragraph::new("Direct SSH to host OS (works when kubelet is down)")
                .style(Style::default().fg(Theme::DIM))
                .alignment(Alignment::Center);
            f.render_widget(info, chunks[0]);

            let input_block = Block::default()
                .borders(Borders::ALL)
                .border_type(Theme::border_type())
                .border_style(Style::default().fg(Theme::CYAN))
                .title(" Destination (IP, hostname, or user@host) ");

            let chars: Vec<char> = destination_input.chars().collect();
            let pos = (*cursor_pos).min(chars.len());
            let before: String = chars[..pos].iter().collect();
            let after: String = chars[pos..].iter().collect();
            let display_text = format!("{}█{}", before, after);

            let input_widget = Paragraph::new(display_text)
                .style(Style::default().fg(Theme::FG).add_modifier(Modifier::BOLD))
                .alignment(Alignment::Center)
                .block(input_block);
            f.render_widget(input_widget, chunks[1]);

            let hints = Paragraph::new(Line::from(vec![
                Span::styled("[Enter]", Theme::key_hint_key()),
                Span::styled(" Connect  ", Theme::key_hint_desc()),
                Span::styled("[Esc]", Theme::key_hint_key()),
                Span::styled(" Cancel", Theme::key_hint_desc()),
            ]))
            .alignment(Alignment::Center);
            f.render_widget(hints, chunks[2]);
        }
        Modal::PortForward {
            pod_name,
            namespace,
            container_port,
            local_port_input,
            kind,
        } => {
            let modal_area = centered_rect(50, 30, area);
            f.render_widget(Clear, modal_area);

            let title_str = if kind.is_empty() || kind == "Pod" {
                format!(" Start Port Forward: {} ({}) ", pod_name, namespace)
            } else {
                format!(
                    " Start Port Forward: {}/{} ({}) ",
                    kind, pod_name, namespace
                )
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
            ]))
            .alignment(Alignment::Center);
            f.render_widget(hints, chunks[2]);
        }
        Modal::ContainerPicker {
            pod_name,
            containers,
            selected_idx,
            action,
            ..
        } => {
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
        Modal::ContextPicker {
            contexts,
            current_context,
            selected_idx,
            filter,
        } => {
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
                Span::styled(
                    filter.as_str(),
                    Style::default().fg(Theme::FG).add_modifier(Modifier::BOLD),
                ),
                Span::styled("█", Style::default().fg(Theme::CYAN)),
            ]))
            .block(search_block);
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
                            || c.provider
                                .as_deref()
                                .unwrap_or("")
                                .to_lowercase()
                                .contains(&lower_filter)
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

                    let prefix = if is_active {
                        "★ "
                    } else if is_sel {
                        "▶ "
                    } else {
                        "  "
                    };

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
                        Span::styled(
                            prefix,
                            if is_sel {
                                Theme::selected_row()
                            } else {
                                Style::default().fg(color)
                            },
                        ),
                        Span::styled(provider_badge, Style::default().fg(Theme::DIM)),
                        Span::styled(
                            c.name.clone(),
                            Style::default().fg(color).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            active_badge,
                            Style::default()
                                .fg(Theme::GREEN)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(ns_info, Style::default().fg(Theme::CYAN)),
                    ]);

                    let file_name = std::path::Path::new(&c.source_file)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(&c.source_file);

                    let line2 = Line::from(vec![
                        Span::raw("    "),
                        Span::styled(
                            format!("cluster: {}  •  file: {}", c.cluster, file_name),
                            Style::default().fg(Theme::DIM),
                        ),
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
                Span::styled(
                    format!(
                        " Showing {}/{} contexts  •  ",
                        filtered.len(),
                        contexts.len()
                    ),
                    Style::default().fg(Theme::DIM),
                ),
                Span::styled(
                    "Enter",
                    Style::default()
                        .fg(Theme::ACCENT)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(": Switch  ", Style::default().fg(Theme::DIM)),
                Span::styled(
                    "Ctrl+i",
                    Style::default()
                        .fg(Theme::CYAN)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(": Import  ", Style::default().fg(Theme::DIM)),
                Span::styled(
                    "Esc",
                    Style::default()
                        .fg(Theme::YELLOW)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(": Cancel", Style::default().fg(Theme::DIM)),
            ]))
            .alignment(Alignment::Center);
            f.render_widget(footer, chunks[2]);
        }
        Modal::NamespacePicker {
            namespaces,
            current_namespace,
            selected_idx,
            filter,
        } => {
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
                .constraints([Constraint::Length(3), Constraint::Min(5)])
                .split(inner);

            let filter_block = Block::default()
                .borders(Borders::ALL)
                .border_type(Theme::border_type())
                .border_style(Style::default().fg(Theme::CYAN))
                .title(" Filter ");
            let filter_widget = Paragraph::new(format!("{}█", filter)).block(filter_block);
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
                    let prefix = if is_active {
                        "★ "
                    } else if is_sel {
                        "▶ "
                    } else {
                        "  "
                    };
                    let style = if is_sel {
                        Theme::selected_row()
                    } else if is_active {
                        Style::default()
                            .fg(Theme::GREEN)
                            .add_modifier(Modifier::BOLD)
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

            let ns_str = namespace
                .as_deref()
                .map(|n| format!(" ({})", n))
                .unwrap_or_default();
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
                Span::styled(
                    filter.as_str(),
                    Style::default().fg(Theme::FG).add_modifier(Modifier::BOLD),
                ),
                Span::styled("█", Style::default().fg(Theme::CYAN)),
            ]))
            .block(search_block);
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
                        Span::styled(
                            prefix,
                            if is_sel {
                                Theme::selected_row()
                            } else {
                                Style::default().fg(Theme::ACCENT)
                            },
                        ),
                        Span::styled(
                            format!("[{}] ", a.key_hint),
                            Style::default()
                                .fg(Theme::CYAN)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            a.title.clone(),
                            Style::default().fg(Theme::FG).add_modifier(Modifier::BOLD),
                        ),
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
                Span::styled(
                    format!(" Showing {}/{} actions  •  ", filtered.len(), actions.len()),
                    Style::default().fg(Theme::DIM),
                ),
                Span::styled(
                    "Enter",
                    Style::default()
                        .fg(Theme::ACCENT)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(": Run Action  ", Style::default().fg(Theme::DIM)),
                Span::styled(
                    "Esc",
                    Style::default()
                        .fg(Theme::YELLOW)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(": Cancel", Style::default().fg(Theme::DIM)),
            ]))
            .alignment(Alignment::Center);
            f.render_widget(footer, chunks[2]);
        }
        Modal::MetricsTimeline(state) => {
            crate::views::metrics_panel_view::render_metrics_panel_modal(f, area, state);
        }
        Modal::ReasonRail {
            tallies,
            selected_idx,
            active_filter,
        } => {
            crate::views::reason_rail::render_reason_rail_modal(
                f,
                area,
                tallies,
                *selected_idx,
                active_filter.as_deref(),
            );
        }
        Modal::ThemePicker {
            selected_idx,
            initial_theme_idx: _,
        } => {
            let modal_area = centered_rect(75, 75, area);
            f.render_widget(Clear, modal_area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(Theme::border_type())
                .border_style(Style::default().fg(Theme::accent()))
                .title(Span::styled(
                    " Themes & Visual Styles (↑/↓ or j/k Preview • Enter Select • Esc Cancel) ",
                    Style::default()
                        .fg(Theme::accent())
                        .add_modifier(Modifier::BOLD),
                ));

            let inner = block.inner(modal_area);
            f.render_widget(block, modal_area);

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(6), Constraint::Length(1)])
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
                        Span::styled(
                            cursor,
                            if is_sel {
                                Style::default()
                                    .fg(Theme::cyan())
                                    .add_modifier(Modifier::BOLD)
                            } else {
                                Style::default().fg(Theme::dim())
                            },
                        ),
                        Span::styled(
                            format!("{:<20}", p.display_name),
                            if is_sel {
                                Style::default()
                                    .fg(Theme::fg())
                                    .add_modifier(Modifier::BOLD)
                            } else {
                                Style::default().fg(Theme::fg())
                            },
                        ),
                        Span::raw(" "),
                        Span::styled("■ ", Style::default().fg(p.accent)),
                        Span::styled("■ ", Style::default().fg(p.cyan)),
                        Span::styled("■ ", Style::default().fg(p.green)),
                        Span::styled("■ ", Style::default().fg(p.yellow)),
                        Span::styled("■ ", Style::default().fg(p.red)),
                        Span::raw(" "),
                    ];

                    if !visual_badge.is_empty() {
                        spans.push(Span::styled(
                            visual_badge,
                            Style::default()
                                .fg(Theme::cyan())
                                .add_modifier(Modifier::BOLD),
                        ));
                    }

                    if is_current {
                        spans.push(Span::styled(
                            "✔ ACTIVE ",
                            Style::default().fg(p.green).add_modifier(Modifier::BOLD),
                        ));
                    }

                    spans.push(Span::styled(
                        format!("— {}", p.description),
                        Style::default().fg(Theme::dim()),
                    ));

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
                Span::styled(
                    active_name,
                    Style::default()
                        .fg(Theme::accent())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "  •  [Enter] Save  [Esc] Cancel",
                    Style::default().fg(Theme::dim()),
                ),
            ]))
            .alignment(Alignment::Center);
            f.render_widget(footer, chunks[1]);
        }
        Modal::FeatureBanner {
            show_on_startup,
            update_available,
        } => {
            render_feature_banner_modal(f, area, *show_on_startup, update_available.as_deref());
        }
        Modal::AddCluster {
            input,
            cursor_pos,
            error_message,
            preview_contexts,
        } => {
            render_add_cluster_modal(
                f,
                area,
                input,
                *cursor_pos,
                error_message.as_deref(),
                preview_contexts,
            );
        }
        Modal::Diagnosis {
            resource_kind,
            resource_name,
            namespace,
            report,
            scroll_offset,
        } => {
            render_diagnosis_modal(
                f,
                area,
                resource_kind,
                resource_name,
                namespace.as_deref(),
                report,
                *scroll_offset,
            );
        }
    }
}

pub fn render_diagnosis_modal(
    f: &mut Frame,
    area: Rect,
    resource_kind: &str,
    resource_name: &str,
    namespace: Option<&str>,
    report: &srelens_kube::diagnose::DiagnosticReport,
    scroll_offset: usize,
) {
    use srelens_kube::diagnose::{DiagnosticVerdict, SignalSeverity};

    let modal_area = centered_rect(75, 75, area);
    f.render_widget(Clear, modal_area);

    let ns_str = namespace.map(|n| format!(" -n {}", n)).unwrap_or_default();
    let title = format!(
        " ⚡ Root-Cause Diagnosis: {}/{}{} ",
        resource_kind, resource_name, ns_str
    );

    let (border_color, verdict_badge, verdict_color) = match report.verdict {
        DiagnosticVerdict::OOMKilled => (Theme::RED, " OOMKilled ", Theme::RED),
        DiagnosticVerdict::CrashLoopBackOff => (Theme::RED, " CrashLoopBackOff ", Theme::RED),
        DiagnosticVerdict::ConfigError => (Theme::RED, " ConfigError ", Theme::RED),
        DiagnosticVerdict::ImagePullFailed => (Theme::RED, " ImagePullFailed ", Theme::RED),
        DiagnosticVerdict::SchedulingFailed => (Theme::YELLOW, " SchedulingFailed ", Theme::YELLOW),
        DiagnosticVerdict::Evicted => (Theme::RED, " Evicted ", Theme::RED),
        DiagnosticVerdict::Failed => (Theme::RED, " Failed ", Theme::RED),
        DiagnosticVerdict::NodeLost => (Theme::YELLOW, " NodeLost ", Theme::YELLOW),
        DiagnosticVerdict::Healthy => (Theme::GREEN, " Healthy ", Theme::GREEN),
        DiagnosticVerdict::Unknown => (Theme::DIM, " Unknown ", Theme::DIM),
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(Theme::border_type())
        .border_style(Style::default().fg(border_color))
        .title(title);

    let inner = block.inner(modal_area);
    f.render_widget(block, modal_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Verdict & Summary
            Constraint::Length(4), // Remediation
            Constraint::Min(4),    // Signals & Traces
            Constraint::Length(1), // Footer keys
        ])
        .split(inner);

    // 1. Verdict & Summary
    let verdict_spans = vec![
        Span::styled(
            verdict_badge,
            Style::default()
                .fg(Color::Black)
                .bg(verdict_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            &report.summary,
            Style::default().fg(Theme::FG).add_modifier(Modifier::BOLD),
        ),
    ];
    let verdict_block = Block::default()
        .borders(Borders::ALL)
        .border_type(Theme::border_type())
        .border_style(Style::default().fg(Theme::border()))
        .title(" Verdict ");
    f.render_widget(
        Paragraph::new(Line::from(verdict_spans)).block(verdict_block),
        chunks[0],
    );

    // 2. Recommended Remediation
    let remediation_text = if report.remediation.is_empty() {
        "No immediate remediation required."
    } else {
        &report.remediation
    };
    let rem_block = Block::default()
        .borders(Borders::ALL)
        .border_type(Theme::border_type())
        .border_style(Style::default().fg(Theme::CYAN))
        .title(" 💡 Recommended Remediation ");
    let rem_p = Paragraph::new(remediation_text)
        .style(Style::default().fg(Theme::CYAN))
        .wrap(Wrap { trim: true })
        .block(rem_block);
    f.render_widget(rem_p, chunks[1]);

    // 3. Correlated Signals
    let mut signal_lines = Vec::new();
    if report.signals.is_empty() {
        signal_lines.push(Line::from(Span::styled(
            "No abnormal signals or warning events detected.",
            Style::default().fg(Theme::DIM),
        )));
    } else {
        for sig in &report.signals {
            let (icon, icon_color) = match sig.severity {
                SignalSeverity::Error => ("✖ ", Theme::RED),
                SignalSeverity::Warning => ("▲ ", Theme::YELLOW),
                SignalSeverity::Info => ("ℹ ", Theme::CYAN),
            };
            signal_lines.push(Line::from(vec![
                Span::styled(
                    icon,
                    Style::default().fg(icon_color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    &sig.title,
                    Style::default().fg(Theme::FG).add_modifier(Modifier::BOLD),
                ),
            ]));
            if let Some(ref detail) = sig.detail {
                signal_lines.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(detail, Style::default().fg(Theme::DIM)),
                ]));
            }
            signal_lines.push(Line::raw(""));
        }
    }

    let signals_block = Block::default()
        .borders(Borders::ALL)
        .border_type(Theme::border_type())
        .border_style(Style::default().fg(Theme::border()))
        .title(format!(
            " 🔍 Correlated Signals ({}) ",
            report.signals.len()
        ));
    let signals_p = Paragraph::new(signal_lines)
        .scroll((scroll_offset as u16, 0))
        .wrap(Wrap { trim: false })
        .block(signals_block);
    f.render_widget(signals_p, chunks[2]);

    // 4. Footer shortcuts
    let footer_line = Line::from(vec![
        Span::styled(
            "[Esc/q]",
            Style::default()
                .fg(Theme::YELLOW)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Close  │  ", Style::default().fg(Theme::DIM)),
        Span::styled(
            "[l]",
            Style::default()
                .fg(Theme::CYAN)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Logs  │  ", Style::default().fg(Theme::DIM)),
        Span::styled(
            "[d]",
            Style::default()
                .fg(Theme::CYAN)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Describe  │  ", Style::default().fg(Theme::DIM)),
        Span::styled(
            "[e]",
            Style::default()
                .fg(Theme::CYAN)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Edit YAML  │  ", Style::default().fg(Theme::DIM)),
        Span::styled(
            "[a]",
            Style::default()
                .fg(Theme::GREEN)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Ask AI  │  ", Style::default().fg(Theme::DIM)),
        Span::styled(
            "[↑/↓]",
            Style::default()
                .fg(Theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Scroll", Style::default().fg(Theme::DIM)),
    ]);
    f.render_widget(
        Paragraph::new(footer_line).alignment(Alignment::Center),
        chunks[3],
    );
}

pub fn render_add_cluster_modal(
    f: &mut Frame,
    area: Rect,
    input: &str,
    cursor_pos: usize,
    error_message: Option<&str>,
    preview_contexts: &[String],
) {
    let modal_area = centered_rect(65, 45, area);
    f.render_widget(Clear, modal_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(Theme::border_type())
        .border_style(Style::default().fg(Theme::ACCENT))
        .title(" Import Cluster / Kubeconfig (:import, :add-cluster) ");

    let inner = block.inner(modal_area);
    f.render_widget(block, modal_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Input box
            Constraint::Min(3),    // Preview / Error status box
            Constraint::Length(1), // Footer help
        ])
        .split(inner);

    // 1. Input box
    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_type(Theme::border_type())
        .border_style(Style::default().fg(Theme::CYAN))
        .title(" Kubeconfig File Path or Raw YAML (<Ctrl+v> to Paste) ");

    let display_input = if input.contains('\n') {
        let lines: Vec<&str> = input.lines().collect();
        format!(
            "[Pasted YAML: {} lines, {} bytes] (Press Enter to import)",
            lines.len(),
            input.len()
        )
    } else {
        let chars: Vec<char> = input.chars().collect();
        let idx = cursor_pos.min(chars.len());
        let before: String = chars[..idx].iter().collect();
        let after: String = chars[idx..].iter().collect();
        format!("{}█{}", before, after)
    };

    let input_para = Paragraph::new(Line::from(vec![
        Span::styled(" > ", Style::default().fg(Theme::DIM)),
        Span::styled(
            display_input,
            Style::default().fg(Theme::FG).add_modifier(Modifier::BOLD),
        ),
    ]))
    .block(input_block);
    f.render_widget(input_para, chunks[0]);

    // 2. Preview / Status box
    let mut status_lines = Vec::new();
    if let Some(err) = error_message {
        status_lines.push(Line::from(vec![
            Span::styled(
                "✗ ",
                Style::default().fg(Theme::RED).add_modifier(Modifier::BOLD),
            ),
            Span::styled(err, Style::default().fg(Theme::RED)),
        ]));
    } else if !preview_contexts.is_empty() {
        status_lines.push(Line::from(vec![
            Span::styled(
                "✓ Detected Context(s): ",
                Style::default()
                    .fg(Theme::GREEN)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                preview_contexts.join(", "),
                Style::default()
                    .fg(Theme::CYAN)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        status_lines.push(Line::from(vec![Span::styled(
            "  Ready to import into SRElens managed kubeconfigs.",
            Style::default().fg(Theme::DIM),
        )]));
    } else if input.trim().is_empty() {
        status_lines.push(Line::from(vec![Span::styled(
            "• Enter a path to a kubeconfig file (e.g. ~/Downloads/cluster.yaml)",
            Style::default().fg(Theme::DIM),
        )]));
        status_lines.push(Line::from(vec![Span::styled(
            "• Or press <Ctrl+v> to paste raw kubeconfig YAML directly from clipboard",
            Style::default().fg(Theme::DIM),
        )]));
    } else {
        status_lines.push(Line::from(vec![Span::styled(
            "Press <Enter> to parse and import...",
            Style::default().fg(Theme::YELLOW),
        )]));
    }

    let status_block = Block::default()
        .borders(Borders::ALL)
        .border_type(Theme::border_type())
        .border_style(Style::default().fg(if error_message.is_some() {
            Theme::RED
        } else {
            Theme::BORDER
        }))
        .title(" Status & Context Preview ");
    f.render_widget(Paragraph::new(status_lines).block(status_block), chunks[1]);

    // 3. Footer
    let footer = Paragraph::new(Line::from(vec![
        Span::styled(
            "<Ctrl+v>",
            Style::default()
                .fg(Theme::CYAN)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Paste  |  ", Theme::header_label()),
        Span::styled(
            "<Enter>",
            Style::default()
                .fg(Theme::GREEN)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Import & Connect  |  ", Theme::header_label()),
        Span::styled(
            "<Ctrl+w>",
            Style::default()
                .fg(Theme::YELLOW)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Rubout  |  ", Theme::header_label()),
        Span::styled(
            "<Esc>",
            Style::default().fg(Theme::RED).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Cancel", Theme::header_label()),
    ]))
    .alignment(Alignment::Center);
    f.render_widget(footer, chunks[2]);
}

pub fn render_feature_banner_modal(
    f: &mut Frame,
    area: Rect,
    show_on_startup: bool,
    update_available: Option<&str>,
) {
    let modal_width = (area.width.saturating_sub(4))
        .clamp(48, 118)
        .min(area.width);
    let modal_height = (area.height.saturating_sub(2))
        .clamp(18, 29)
        .min(area.height);
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
            Style::default()
                .fg(Theme::cyan())
                .add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(modal_area);
    f.render_widget(block, modal_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Top description & update alert
            Constraint::Min(13),   // Features list
            Constraint::Length(3), // Checkbox and key hints
        ])
        .split(inner);

    let inner_w = chunks[1].width as usize;

    // 1. Header description & update indicator
    let mut header_lines = vec![Line::from(vec![Span::styled(
        "Kubernetes control room with high-velocity SRE troubleshooting capabilities.",
        Style::default()
            .fg(Theme::fg())
            .add_modifier(Modifier::BOLD),
    )])];

    if let Some(ver) = update_available {
        header_lines.push(Line::from(vec![
            Span::styled(
                "▲ UPDATE AVAILABLE: ",
                Style::default()
                    .fg(Theme::yellow())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("v{} is available! ", ver),
                Style::default()
                    .fg(Theme::yellow())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("• Run '", Style::default().fg(Theme::dim())),
            Span::styled(
                "srelens-tui update",
                Style::default()
                    .fg(Theme::cyan())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "' in terminal (or press ",
                Style::default().fg(Theme::dim()),
            ),
            Span::styled(
                "u",
                Style::default()
                    .fg(Theme::yellow())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(")", Style::default().fg(Theme::dim())),
        ]));
    } else {
        header_lines.push(Line::from(vec![
            Span::styled(
                format!("Version v{} • Type ", env!("CARGO_PKG_VERSION")),
                Style::default().fg(Theme::dim()),
            ),
            Span::styled(
                ":update",
                Style::default()
                    .fg(Theme::cyan())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" or press ", Style::default().fg(Theme::dim())),
            Span::styled(
                "u",
                Style::default()
                    .fg(Theme::yellow())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" to check for updates", Style::default().fg(Theme::dim())),
        ]));
    }

    let header_hint = if inner_w >= 108 {
        "Key built-in features you should know (press [0-9, b, i, u] to jump directly, or type ':' for command prompt):"
    } else if inner_w >= 80 {
        "Key built-in features (press [0-9, b, i, u] to jump directly, or ':' for commands):"
    } else {
        "Key features (press [0-9, b, i, u] to jump, ':' for commands):"
    };

    header_lines.push(Line::from(vec![Span::styled(
        header_hint,
        Style::default().fg(Theme::dim()),
    )]));
    f.render_widget(Paragraph::new(header_lines), chunks[0]);

    // 2. Feature highlights
    let update_desc = if let Some(ver) = update_available {
        format!("▲ New version v{} available! Run 'srelens-tui update'", ver)
    } else {
        "Check for new releases & update binary ('srelens-tui update')".to_string()
    };

    let features: [(&str, &str, &str, String, &str); 13] = [
        (
            "[1]",
            ":helm",
            "[Helm 3]",
            "Helm 3 release revisions, rollback status, values & manifests".to_string(),
            ":helm [ns]",
        ),
        (
            "[2]",
            ":overview",
            "[Cluster]",
            "Cluster overview, health summary & node/pod capacity".to_string(),
            ":overview",
        ),
        (
            "[3]",
            ":gpuinfo",
            "[Hardware]",
            "GPU hardware inspector, specs & per-pod VRAM allocations".to_string(),
            ":gpuinfo",
        ),
        (
            "[4]",
            ":workloads",
            "[Workload]",
            "Unified workloads view (Pods, Deployments, STS, DS, Jobs)".to_string(),
            ":workloads [ns]",
        ),
        (
            "[5]",
            ":argo",
            "[GitOps]",
            "ArgoCD applications, sync status, drift & GitOps control".to_string(),
            ":argo [ns]",
        ),
        (
            "[6]",
            ":ai",
            "[AI Assistant]",
            "Interactive AI troubleshooting chat for automated RCA".to_string(),
            ":ai",
        ),
        (
            "[7]",
            ":ai-settings",
            "[AI Config]",
            "Configure AI providers (Claude, OpenAI, Gemini), models & keys".to_string(),
            ":ai-settings",
        ),
        (
            "[8]",
            ":config",
            "[Lens Settings]",
            "Lens settings: popup width, visible rows, text scale & banner".to_string(),
            ":config",
        ),
        (
            "[9]",
            ":banner",
            "[Guide]",
            "Re-display this feature highlights banner & startup guide".to_string(),
            ":banner",
        ),
        (
            "[0]",
            ":nodes",
            "[Node SSH]",
            "Direct SSH to host OS for node recovery (<S> on node)".to_string(),
            ":nodes -> <S>",
        ),
        (
            "[b]",
            ":bgp",
            "[BGP Peering]",
            "BGP control plane, live peering topology & route VIPs".to_string(),
            ":bgp",
        ),
        (
            "[i]",
            ":import",
            "[Add Cluster]",
            "Import Kubernetes cluster / kubeconfig from clipboard or file path".to_string(),
            ":import",
        ),
        ("[u]", ":update", "[Self Update]", update_desc, ":update"),
    ];

    let items: Vec<ListItem> = features
        .iter()
        .map(|(num, cmd, cat, desc, syntax)| {
            let (num_style, cmd_style) = if *cmd == ":update" && update_available.is_some() {
                (
                    Style::default()
                        .fg(Theme::yellow())
                        .add_modifier(Modifier::BOLD),
                    Style::default()
                        .fg(Theme::yellow())
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                (
                    Style::default()
                        .fg(Theme::cyan())
                        .add_modifier(Modifier::BOLD),
                    Style::default()
                        .fg(Theme::yellow())
                        .add_modifier(Modifier::BOLD),
                )
            };

            let prefix_w = 4 + 13 + 15; // num (4) + cmd (13) + cat (15)
            let mut spans = vec![
                Span::styled(format!("{num} "), num_style),
                Span::styled(format!("{:<13}", cmd), cmd_style),
                Span::styled(
                    format!("{:<15}", cat),
                    Style::default()
                        .fg(Theme::accent())
                        .add_modifier(Modifier::BOLD),
                ),
            ];

            if inner_w >= 110 {
                let syntax_str = format!("  ({})", syntax);
                let avail_desc = inner_w.saturating_sub(prefix_w + syntax_str.len());
                let clean_desc = if desc.chars().count() > avail_desc && avail_desc > 3 {
                    let s: String = desc.chars().take(avail_desc - 3).collect();
                    format!("{}...", s)
                } else {
                    desc.clone()
                };
                spans.push(Span::styled(
                    format!("{:<avail_desc$}", clean_desc),
                    Style::default().fg(Theme::fg()),
                ));
                spans.push(Span::styled(syntax_str, Style::default().fg(Theme::dim())));
            } else {
                let avail_desc = inner_w.saturating_sub(prefix_w);
                let clean_desc = if desc.chars().count() > avail_desc && avail_desc > 3 {
                    let s: String = desc.chars().take(avail_desc - 3).collect();
                    format!("{}...", s)
                } else {
                    desc.clone()
                };
                spans.push(Span::styled(clean_desc, Style::default().fg(Theme::fg())));
            }

            ListItem::new(Line::from(spans))
        })
        .collect();

    f.render_widget(List::new(items), chunks[1]);

    // 3. Footer with Startup Checkbox & Key hints
    let checkbox_spans = if show_on_startup {
        vec![
            Span::styled(
                " [●] ",
                Style::default()
                    .fg(Theme::cyan())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "Show this feature banner on startup",
                Style::default()
                    .fg(Theme::fg())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" (Press ", Style::default().fg(Theme::dim())),
            Span::styled(
                "t",
                Style::default()
                    .fg(Theme::yellow())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" to toggle)", Style::default().fg(Theme::dim())),
        ]
    } else {
        vec![
            Span::styled(" [○] ", Style::default().fg(Theme::dim())),
            Span::styled(
                "Show this feature banner on startup",
                Style::default().fg(Theme::dim()),
            ),
            Span::styled(" (Currently ", Style::default().fg(Theme::dim())),
            Span::styled(
                "Disabled",
                Style::default()
                    .fg(Theme::yellow())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" • Press ", Style::default().fg(Theme::dim())),
            Span::styled(
                "t",
                Style::default()
                    .fg(Theme::yellow())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" to enable)", Style::default().fg(Theme::dim())),
        ]
    };

    let jump_hint = "0-9, b, i, u";
    let footer_spans = if inner_w >= 98 {
        vec![
            Span::styled(" Press ", Style::default().fg(Theme::dim())),
            Span::styled(
                "Enter",
                Style::default()
                    .fg(Theme::yellow())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(", ", Style::default().fg(Theme::dim())),
            Span::styled(
                "Esc",
                Style::default()
                    .fg(Theme::yellow())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(", or ", Style::default().fg(Theme::dim())),
            Span::styled(
                "q",
                Style::default()
                    .fg(Theme::yellow())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" to dismiss  |  Press ", Style::default().fg(Theme::dim())),
            Span::styled(
                jump_hint,
                Style::default()
                    .fg(Theme::cyan())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" to jump directly  |  ", Style::default().fg(Theme::dim())),
            Span::styled(":banner", Style::default().fg(Theme::accent())),
            Span::styled(" to reopen anytime", Style::default().fg(Theme::dim())),
        ]
    } else if inner_w >= 75 {
        vec![
            Span::styled(" Press ", Style::default().fg(Theme::dim())),
            Span::styled(
                "Enter",
                Style::default()
                    .fg(Theme::yellow())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(", ", Style::default().fg(Theme::dim())),
            Span::styled(
                "Esc",
                Style::default()
                    .fg(Theme::yellow())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(", or ", Style::default().fg(Theme::dim())),
            Span::styled(
                "q",
                Style::default()
                    .fg(Theme::yellow())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" to dismiss  |  Press ", Style::default().fg(Theme::dim())),
            Span::styled(
                jump_hint,
                Style::default()
                    .fg(Theme::cyan())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" to jump  |  ", Style::default().fg(Theme::dim())),
            Span::styled(":banner", Style::default().fg(Theme::accent())),
            Span::styled(" to reopen", Style::default().fg(Theme::dim())),
        ]
    } else {
        vec![
            Span::styled(
                "Esc",
                Style::default()
                    .fg(Theme::yellow())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" dismiss | ", Style::default().fg(Theme::dim())),
            Span::styled(
                jump_hint,
                Style::default()
                    .fg(Theme::cyan())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" jump | ", Style::default().fg(Theme::dim())),
            Span::styled(":banner", Style::default().fg(Theme::accent())),
            Span::styled(" reopen", Style::default().fg(Theme::dim())),
        ]
    };

    let footer_lines = vec![
        Line::from(vec![Span::styled(
            "─".repeat(inner_w),
            Style::default().fg(Theme::border()),
        )]),
        Line::from(checkbox_spans),
        Line::from(footer_spans),
    ];

    f.render_widget(Paragraph::new(footer_lines), chunks[2]);
}
