use ratatui::{
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    text::Span,
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
    Frame,
};
use srelens_kube::argo::ArgoApplication;

use crate::theme::Theme;

pub struct ArgoViewState {
    pub applications: Vec<ArgoApplication>,
    pub selected_idx: usize,
    pub is_loading: bool,
    pub error: Option<String>,
    pub filter_query: String,
    pub is_remote_hub: bool,
    pub hub_context_name: Option<String>,
}

impl ArgoViewState {
    pub fn new() -> Self {
        Self {
            applications: Vec::new(),
            selected_idx: 0,
            is_loading: true,
            error: None,
            filter_query: String::new(),
            is_remote_hub: false,
            hub_context_name: None,
        }
    }

    pub fn set_applications(
        &mut self,
        apps: Vec<ArgoApplication>,
        is_remote_hub: bool,
        hub_context_name: Option<String>,
    ) {
        if self.applications == apps
            && self.is_remote_hub == is_remote_hub
            && self.hub_context_name == hub_context_name
        {
            self.is_loading = false;
            self.error = None;
            return;
        }
        let sel_target = self.selected_application().map(|a| (a.name.clone(), a.namespace.clone()));
        self.applications = apps;
        self.is_remote_hub = is_remote_hub;
        self.hub_context_name = hub_context_name;
        self.is_loading = false;
        self.error = None;
        let indices = self.filtered_indices();
        if let Some((name, ns)) = sel_target {
            if let Some(pos) = indices.iter().position(|&idx| {
                self.applications
                    .get(idx)
                    .map(|a| a.name == name && a.namespace == ns)
                    .unwrap_or(false)
            }) {
                self.selected_idx = pos;
                return;
            }
        }
        let count = indices.len();
        if self.selected_idx >= count {
            self.selected_idx = count.saturating_sub(1);
        }
    }

    pub fn set_error(&mut self, err: String) {
        self.error = Some(err);
        self.is_loading = false;
    }

    pub fn filtered_indices(&self) -> Vec<usize> {
        if self.filter_query.is_empty() {
            return (0..self.applications.len()).collect();
        }
        let q = self.filter_query.to_lowercase();
        self.applications
            .iter()
            .enumerate()
            .filter(|(_, a)| {
                a.name.to_lowercase().contains(&q)
                    || a.namespace.to_lowercase().contains(&q)
                    || a.project.to_lowercase().contains(&q)
                    || a.destination_name.to_lowercase().contains(&q)
                    || a.destination_server.to_lowercase().contains(&q)
                    || a.destination_namespace.to_lowercase().contains(&q)
                    || a.sync_status.to_lowercase().contains(&q)
                    || a.health_status.to_lowercase().contains(&q)
                    || a.repo_url.to_lowercase().contains(&q)
                    || a.path.to_lowercase().contains(&q)
            })
            .map(|(i, _)| i)
            .collect()
    }

    pub fn select_next(&mut self) {
        let indices = self.filtered_indices();
        if !indices.is_empty() && self.selected_idx + 1 < indices.len() {
            self.selected_idx += 1;
        }
    }

    pub fn select_prev(&mut self) {
        if self.selected_idx > 0 {
            self.selected_idx -= 1;
        }
    }

    pub fn selected_application(&self) -> Option<&ArgoApplication> {
        let indices = self.filtered_indices();
        let idx = *indices.get(self.selected_idx)?;
        self.applications.get(idx)
    }
}

fn format_iso_age(iso: &str) -> String {
    if iso.is_empty() {
        return "-".to_string();
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(iso) {
        let now = chrono::Utc::now();
        let secs = (now - dt.with_timezone(&chrono::Utc)).num_seconds();
        srelens_kube::format_age(secs)
    } else {
        "-".to_string()
    }
}

fn sync_status_badge(status: &str) -> (&'static str, Style) {
    match status {
        "Synced" => ("● Synced", Theme::status_ok()),
        "OutOfSync" => ("▲ OutOfSync", Theme::status_warn()),
        _ if status.is_empty() => ("- Unknown", Style::default().fg(Theme::dim())),
        _ => ("✖ Error", Theme::status_error()),
    }
}

fn health_status_badge(health: &str) -> (&'static str, Style) {
    match health {
        "Healthy" => ("● Healthy", Theme::status_ok()),
        "Progressing" => ("⟳ Progressing", Style::default().fg(Theme::cyan())),
        "Degraded" => ("✖ Degraded", Theme::status_error()),
        "Missing" => ("✖ Missing", Theme::status_error()),
        "Suspended" => ("⏸ Suspended", Style::default().fg(Theme::dim())),
        _ if health.is_empty() => ("- Unknown", Style::default().fg(Theme::dim())),
        _ => ("? Unknown", Style::default().fg(Theme::dim())),
    }
}

pub fn render_argo_view(f: &mut Frame, area: Rect, state: &ArgoViewState) {
    let filtered = state.filtered_indices();
    let count_text = if state.filter_query.is_empty() {
        format!("{}", state.applications.len())
    } else {
        format!("{}/{}", filtered.len(), state.applications.len())
    };

    let hub_tag = if state.is_remote_hub {
        let name = state.hub_context_name.as_deref().unwrap_or("Hub");
        format!(" [Hub: {}] ", name)
    } else {
        String::new()
    };

    let title = format!(
        " 🐙 ArgoCD Applications [{}] {}(<Enter> Details  <s> Sync  <p> Toggle Auto-Sync  <R> Hard Refresh  <g> Git  <r> Reload  <Esc> Back) ",
        count_text, hub_tag
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(Theme::border_type())
        .border_style(Style::default().fg(Theme::border()))
        .title(Span::styled(title, Theme::title()));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if state.is_loading {
        let loading_msg = Paragraph::new("⟳ Loading ArgoCD applications...")
            .style(Style::default().fg(Theme::cyan()));
        f.render_widget(loading_msg, inner);
        return;
    }

    if let Some(ref err) = state.error {
        let error_msg = Paragraph::new(format!("⚠ Failed to load ArgoCD applications: {}", err))
            .style(Style::default().fg(Theme::red()));
        f.render_widget(error_msg, inner);
        return;
    }

    if state.applications.is_empty() {
        let msg = if state.is_remote_hub {
            "No ArgoCD applications found targeting this cluster on the Hub."
        } else {
            "No ArgoCD applications found in cluster."
        };
        let empty_msg = Paragraph::new(msg).style(Style::default().fg(Theme::dim()));
        f.render_widget(empty_msg, inner);
        return;
    }

    if filtered.is_empty() {
        let empty_msg = Paragraph::new(format!(
            "No applications matching filter '{}'",
            state.filter_query
        ))
        .style(Style::default().fg(Theme::dim()));
        f.render_widget(empty_msg, inner);
        return;
    }

    let is_remote = state.is_remote_hub;

    let headers = if is_remote {
        Row::new(vec![
            Cell::from("DEST CLUSTER").style(Theme::table_header()),
            Cell::from("DEST NS").style(Theme::table_header()),
            Cell::from("APPLICATION").style(Theme::table_header()),
            Cell::from("SYNC").style(Theme::table_header()),
            Cell::from("HEALTH").style(Theme::table_header()),
            Cell::from("SOURCE / PATH").style(Theme::table_header()),
            Cell::from("AUTO-SYNC").style(Theme::table_header()),
            Cell::from("LAST SYNC").style(Theme::table_header()),
            Cell::from("AGE").style(Theme::table_header()),
        ])
    } else {
        Row::new(vec![
            Cell::from("NAMESPACE").style(Theme::table_header()),
            Cell::from("APPLICATION").style(Theme::table_header()),
            Cell::from("PROJECT").style(Theme::table_header()),
            Cell::from("SYNC").style(Theme::table_header()),
            Cell::from("HEALTH").style(Theme::table_header()),
            Cell::from("SOURCE / PATH").style(Theme::table_header()),
            Cell::from("AUTO-SYNC").style(Theme::table_header()),
            Cell::from("LAST SYNC").style(Theme::table_header()),
            Cell::from("AGE").style(Theme::table_header()),
        ])
    }
    .height(1)
    .bottom_margin(1);

    let rows: Vec<Row> = filtered
        .iter()
        .enumerate()
        .map(|(display_idx, &real_idx)| {
            let app = &state.applications[real_idx];
            let is_selected = display_idx == state.selected_idx;

            let (sync_text, sync_style) = sync_status_badge(&app.sync_status);
            let (health_text, health_style) = health_status_badge(&app.health_status);

            let row_style = if is_selected {
                Theme::selected_row()
            } else {
                Style::default()
            };

            let source_display = if app.path.is_empty() {
                app.target_revision.clone()
            } else {
                format!("{}:{}", app.target_revision, app.path)
            };

            let auto_sync_str = if app.auto_sync_enabled {
                "Enabled"
            } else {
                "Paused"
            };
            let auto_sync_style = if app.auto_sync_enabled {
                Style::default().fg(Theme::green())
            } else {
                Style::default().fg(Theme::yellow())
            };

            let last_sync = if app.last_sync_time.is_empty() {
                "-".to_string()
            } else {
                format_iso_age(&app.last_sync_time)
            };

            let age = format_iso_age(&app.created_at);

            if is_remote {
                let dest = if !app.destination_name.is_empty() {
                    app.destination_name.as_str()
                } else if !app.destination_server.is_empty() {
                    app.destination_server.as_str()
                } else {
                    "-"
                };

                Row::new(vec![
                    Cell::from(dest),
                    Cell::from(app.destination_namespace.as_str()),
                    Cell::from(app.name.as_str()),
                    Cell::from(sync_text).style(sync_style),
                    Cell::from(health_text).style(health_style),
                    Cell::from(source_display),
                    Cell::from(auto_sync_str).style(auto_sync_style),
                    Cell::from(last_sync),
                    Cell::from(age),
                ])
                .style(row_style)
            } else {
                Row::new(vec![
                    Cell::from(app.namespace.as_str()),
                    Cell::from(app.name.as_str()),
                    Cell::from(app.project.as_str()),
                    Cell::from(sync_text).style(sync_style),
                    Cell::from(health_text).style(health_style),
                    Cell::from(source_display),
                    Cell::from(auto_sync_str).style(auto_sync_style),
                    Cell::from(last_sync),
                    Cell::from(age),
                ])
                .style(row_style)
            }
        })
        .collect();

    let widths = if is_remote {
        [
            Constraint::Length(16),
            Constraint::Length(14),
            Constraint::Min(20),
            Constraint::Length(14),
            Constraint::Length(16),
            Constraint::Min(25),
            Constraint::Length(11),
            Constraint::Length(11),
            Constraint::Length(7),
        ]
    } else {
        [
            Constraint::Length(14),
            Constraint::Min(20),
            Constraint::Length(12),
            Constraint::Length(14),
            Constraint::Length(16),
            Constraint::Min(25),
            Constraint::Length(11),
            Constraint::Length(11),
            Constraint::Length(7),
        ]
    };

    let table = Table::new(rows, widths).header(headers);
    f.render_widget(table, inner);
}
