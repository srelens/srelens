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
        contexts: Vec<String>,
        current_context: String,
        selected_idx: usize,
    },
    NamespacePicker {
        namespaces: Vec<String>,
        current_namespace: String,
        selected_idx: usize,
        filter: String,
    },
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
        Modal::ContextPicker { contexts, current_context, selected_idx } => {
            let modal_area = centered_rect(55, 50, area);
            f.render_widget(Clear, modal_area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Theme::ACCENT))
                .title(" Switch Kubernetes Context (↑/↓ Navigate, Enter Switch, Esc Close) ");

            let inner = block.inner(modal_area);
            f.render_widget(block, modal_area);

            let items: Vec<ListItem> = contexts
                .iter()
                .enumerate()
                .map(|(i, name)| {
                    let is_active = name == current_context;
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
            f.render_widget(list, inner);
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

            let mut all_ns = vec!["(all namespaces)".to_string()];
            all_ns.extend(namespaces.iter().filter(|n| n.contains(filter.as_str())).cloned());

            let items: Vec<ListItem> = all_ns
                .iter()
                .enumerate()
                .map(|(i, name)| {
                    let is_active = (i == 0 && current_namespace.is_empty()) || name == current_namespace;
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
    }
}
