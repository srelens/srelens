use ratatui::{
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
    Frame,
};
use srelens_kube::argo::ArgoApplication;

use crate::theme::Theme;

pub struct ArgoViewState {
    pub applications: Vec<ArgoApplication>,
    pub all_applications: Vec<ArgoApplication>,
    pub selected_idx: usize,
    pub is_loading: bool,
    pub error: Option<String>,
    pub filter_query: String,
    pub is_remote_hub: bool,
    pub hub_context_name: Option<String>,
    pub show_all_hub_apps: bool,
}

impl ArgoViewState {
    pub fn new() -> Self {
        Self {
            applications: Vec::new(),
            all_applications: Vec::new(),
            selected_idx: 0,
            is_loading: true,
            error: None,
            filter_query: String::new(),
            is_remote_hub: false,
            hub_context_name: None,
            show_all_hub_apps: false,
        }
    }

    pub fn displayed_applications(&self) -> &[ArgoApplication] {
        if self.is_remote_hub && self.show_all_hub_apps {
            &self.all_applications
        } else {
            &self.applications
        }
    }

    pub fn toggle_show_all(&mut self) {
        self.show_all_hub_apps = !self.show_all_hub_apps;
        self.selected_idx = 0;
    }

    pub fn set_applications(
        &mut self,
        apps: Vec<ArgoApplication>,
        all_apps: Vec<ArgoApplication>,
        is_remote_hub: bool,
        hub_context_name: Option<String>,
    ) {
        if self.applications == apps
            && self.all_applications == all_apps
            && self.is_remote_hub == is_remote_hub
            && self.hub_context_name == hub_context_name
        {
            self.is_loading = false;
            self.error = None;
            return;
        }
        let sel_target = self
            .selected_application()
            .map(|a| (a.name.clone(), a.namespace.clone()));
        self.applications = apps;
        self.all_applications = all_apps;
        self.is_remote_hub = is_remote_hub;
        self.hub_context_name = hub_context_name;
        self.is_loading = false;
        self.error = None;
        let indices = self.filtered_indices();
        if let Some((name, ns)) = sel_target {
            if let Some(pos) = indices.iter().position(|&idx| {
                self.displayed_applications()
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
        let apps = self.displayed_applications();
        if self.filter_query.is_empty() {
            return (0..apps.len()).collect();
        }
        let q = self.filter_query.to_lowercase();
        apps.iter()
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
        self.displayed_applications().get(idx)
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
        _ if status.is_empty() || status.eq_ignore_ascii_case("unknown") => {
            ("- Unknown", Style::default().fg(Theme::dim()))
        }
        _ => ("✖ Error", Theme::status_error()),
    }
}

pub(crate) fn repo_basename(repo_url: &str) -> &str {
    let s = repo_url.trim_end_matches('/');
    let s = s.strip_suffix(".git").unwrap_or(s);
    if let Some(pos) = s.rfind(|c| c == '/' || c == ':') {
        &s[pos + 1..]
    } else {
        s
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
    let displayed = state.displayed_applications();
    let filtered = state.filtered_indices();
    let count_text = if state.filter_query.is_empty() {
        format!("{}", displayed.len())
    } else {
        format!("{}/{}", filtered.len(), displayed.len())
    };

    let hub_tag = if state.is_remote_hub {
        let name = state.hub_context_name.as_deref().unwrap_or("Hub");
        if state.show_all_hub_apps {
            format!(" [Hub: {} · All Hub Apps] ", name)
        } else {
            format!(" [Hub: {} · Spoke Filtered] ", name)
        }
    } else {
        String::new()
    };

    let toggle_hint = if state.is_remote_hub {
        if state.show_all_hub_apps {
            " <a> Current Spoke "
        } else {
            " <a> View All Hub Apps "
        }
    } else {
        ""
    };

    let title = format!(
        " 🐙 ArgoCD Applications [{}] {}(<Enter> Details  <x> Actions / AI{} <s> Sync  <p> Toggle Auto-Sync  <R> Hard Refresh  <g> Git  <c> Config Hub  <r> Reload  <Esc> Back) ",
        count_text, hub_tag, toggle_hint
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
        let msg = if err.contains("No ArgoCD deployment in this cluster")
            || err.contains("no argocd deployment")
        {
            format!("⚠ {}", err)
        } else {
            format!("⚠ Failed to load ArgoCD applications: {}", err)
        };
        let lines = vec![
            Line::from(vec![Span::styled(
                msg,
                Style::default()
                    .fg(Theme::red())
                    .add_modifier(Modifier::BOLD),
            )]),
            Line::from(vec![
                Span::styled(
                    "Hint: In a Hub-and-Spoke setup, press ",
                    Style::default().fg(Theme::dim()),
                ),
                Span::styled(
                    "<c>",
                    Style::default()
                        .fg(Theme::yellow())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    " to configure the ArgoCD Hub context or external kubeconfig in ':config'.",
                    Style::default().fg(Theme::dim()),
                ),
            ]),
        ];
        let p = Paragraph::new(lines).wrap(ratatui::widgets::Wrap { trim: true });
        f.render_widget(p, inner);
        return;
    }

    if displayed.is_empty() {
        let msg = if state.is_remote_hub {
            if !state.all_applications.is_empty() && !state.show_all_hub_apps {
                "No ArgoCD applications found targeting this cluster on the Hub. Press <a> to view all Hub applications."
            } else {
                "No ArgoCD applications found targeting this cluster on the Hub."
            }
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

    let mut max_dest = "DEST CLUSTER".len();
    let mut max_dest_ns = "DEST NS".len();
    let mut max_ns = "NAMESPACE".len();
    let mut max_name = "APPLICATION".len();
    let mut max_project = "PROJECT".len();
    let mut max_sync = "SYNC".len();
    let mut max_health = "HEALTH".len();
    let mut max_auto_sync = "AUTO-SYNC".len();
    let mut max_last_sync = "LAST SYNC".len();
    let mut max_age = "AGE".len();

    for &real_idx in &filtered {
        let app = &displayed[real_idx];
        let (sync_text, _) = sync_status_badge(&app.sync_status);
        let (health_text, _) = health_status_badge(&app.health_status);
        let auto_sync_len = if app.auto_sync_enabled { 7 } else { 6 };
        let last_sync_len = if app.last_sync_time.is_empty() {
            1
        } else {
            format_iso_age(&app.last_sync_time).len()
        };
        let age_len = format_iso_age(&app.created_at).len();

        let dest_str = if !app.destination_name.is_empty() {
            app.destination_name.as_str()
        } else if !app.destination_server.is_empty() {
            app.destination_server.as_str()
        } else {
            "-"
        };

        max_dest = max_dest.max(dest_str.len());
        let dest_ns_str = if app.destination_namespace.is_empty() {
            "-"
        } else {
            app.destination_namespace.as_str()
        };
        max_dest_ns = max_dest_ns.max(dest_ns_str.len());
        max_ns = max_ns.max(app.namespace.len());
        max_name = max_name.max(app.name.len());
        max_project = max_project.max(app.project.len());
        max_sync = max_sync.max(sync_text.chars().count());
        max_health = max_health.max(health_text.chars().count());
        max_auto_sync = max_auto_sync.max(auto_sync_len);
        max_last_sync = max_last_sync.max(last_sync_len);
        max_age = max_age.max(age_len);
    }

    let col_first = (if is_remote { max_dest } else { max_ns } + 2) as u16;
    let col_second = (if is_remote { max_dest_ns } else { max_name } + 2) as u16;
    let col_third = (if is_remote { max_name } else { max_project } + 2) as u16;
    let col_sync = (max_sync + 2) as u16;
    let col_health = (max_health + 2) as u16;
    let col_auto = (max_auto_sync + 2) as u16;
    let col_last_sync = (max_last_sync + 2) as u16;
    let col_age = (max_age + 2) as u16;

    let primary_width = col_first + col_second + col_third + col_sync + col_health;
    let secondary_width = col_auto + col_last_sync + col_age;
    let total_all = primary_width + secondary_width;

    let avail_w = inner.width;
    let show_secondary = avail_w >= total_all;
    let show_source = avail_w >= total_all + 15;

    let mut header_cells = if is_remote {
        vec![
            Cell::from("DEST CLUSTER").style(Theme::table_header()),
            Cell::from("DEST NS").style(Theme::table_header()),
            Cell::from("APPLICATION").style(Theme::table_header()),
            Cell::from("SYNC").style(Theme::table_header()),
            Cell::from("HEALTH").style(Theme::table_header()),
        ]
    } else {
        vec![
            Cell::from("NAMESPACE").style(Theme::table_header()),
            Cell::from("APPLICATION").style(Theme::table_header()),
            Cell::from("PROJECT").style(Theme::table_header()),
            Cell::from("SYNC").style(Theme::table_header()),
            Cell::from("HEALTH").style(Theme::table_header()),
        ]
    };

    if show_source {
        header_cells.push(Cell::from("SOURCE / PATH").style(Theme::table_header()));
    }
    if show_secondary {
        header_cells.push(Cell::from("AUTO-SYNC").style(Theme::table_header()));
        header_cells.push(Cell::from("LAST SYNC").style(Theme::table_header()));
        header_cells.push(Cell::from("AGE").style(Theme::table_header()));
    }
    let headers = Row::new(header_cells).height(1).bottom_margin(1);

    let rows: Vec<Row> = filtered
        .iter()
        .enumerate()
        .map(|(display_idx, &real_idx)| {
            let app = &displayed[real_idx];
            let is_selected = display_idx == state.selected_idx;

            let (sync_text, sync_style) = sync_status_badge(&app.sync_status);
            let (health_text, health_style) = health_status_badge(&app.health_status);

            let row_style = if is_selected {
                Theme::selected_row()
            } else {
                Style::default()
            };

            let mut cells = if is_remote {
                let dest = if !app.destination_name.is_empty() {
                    app.destination_name.as_str()
                } else if !app.destination_server.is_empty() {
                    app.destination_server.as_str()
                } else {
                    "-"
                };
                let dest_ns = if app.destination_namespace.is_empty() {
                    "-"
                } else {
                    app.destination_namespace.as_str()
                };

                vec![
                    Cell::from(dest),
                    Cell::from(dest_ns),
                    Cell::from(app.name.as_str()),
                    Cell::from(sync_text).style(sync_style),
                    Cell::from(health_text).style(health_style),
                ]
            } else {
                vec![
                    Cell::from(app.namespace.as_str()),
                    Cell::from(app.name.as_str()),
                    Cell::from(app.project.as_str()),
                    Cell::from(sync_text).style(sync_style),
                    Cell::from(health_text).style(health_style),
                ]
            };

            if show_source {
                let repo = repo_basename(&app.repo_url);
                let source_display = if !repo.is_empty() {
                    if app.path.is_empty() {
                        format!("{}@{}", repo, app.target_revision)
                    } else {
                        format!("{}@{}:{}", repo, app.target_revision, app.path)
                    }
                } else if app.path.is_empty() {
                    app.target_revision.clone()
                } else {
                    format!("{}:{}", app.target_revision, app.path)
                };
                cells.push(Cell::from(source_display));
            }

            if show_secondary {
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

                cells.push(Cell::from(auto_sync_str).style(auto_sync_style));
                cells.push(Cell::from(last_sync));
                cells.push(Cell::from(age));
            }

            Row::new(cells).style(row_style)
        })
        .collect();

    let mut widths = vec![
        Constraint::Length(col_first),
        Constraint::Length(col_second),
        Constraint::Length(col_third),
        Constraint::Length(col_sync),
        Constraint::Length(col_health),
    ];
    if show_source {
        widths.push(Constraint::Fill(1));
    }
    if show_secondary {
        widths.push(Constraint::Length(col_auto));
        widths.push(Constraint::Length(col_last_sync));
        widths.push(Constraint::Length(col_age));
    }

    let table = Table::new(rows, widths).header(headers);
    f.render_widget(table, inner);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_repo_basename() {
        assert_eq!(
            repo_basename("https://github.com/argoproj/argocd-example-apps.git"),
            "argocd-example-apps"
        );
        assert_eq!(
            repo_basename("git@github.com:argoproj/argocd-example-apps.git"),
            "argocd-example-apps"
        );
        assert_eq!(repo_basename("https://gitlab.com/org/repo/"), "repo");
        assert_eq!(repo_basename("custom-repo"), "custom-repo");
        assert_eq!(repo_basename(""), "");
    }

    #[test]
    fn test_sync_status_badge() {
        let (label, _) = sync_status_badge("Synced");
        assert_eq!(label, "● Synced");
        let (label, _) = sync_status_badge("OutOfSync");
        assert_eq!(label, "▲ OutOfSync");
        let (label, _) = sync_status_badge("");
        assert_eq!(label, "- Unknown");
        let (label, _) = sync_status_badge("Unknown");
        assert_eq!(label, "- Unknown");
        let (label, _) = sync_status_badge("unknown");
        assert_eq!(label, "- Unknown");
        let (label, _) = sync_status_badge("Failed");
        assert_eq!(label, "✖ Error");
    }

    #[test]
    fn test_health_status_badge() {
        let (label, _) = health_status_badge("Healthy");
        assert_eq!(label, "● Healthy");
        let (label, _) = health_status_badge("Progressing");
        assert_eq!(label, "⟳ Progressing");
        let (label, _) = health_status_badge("Degraded");
        assert_eq!(label, "✖ Degraded");
        let (label, _) = health_status_badge("Missing");
        assert_eq!(label, "✖ Missing");
        let (label, _) = health_status_badge("Suspended");
        assert_eq!(label, "⏸ Suspended");
        let (label, _) = health_status_badge("");
        assert_eq!(label, "- Unknown");
        let (label, _) = health_status_badge("Unknown");
        assert_eq!(label, "? Unknown");
        let (label, _) = health_status_badge("Custom");
        assert_eq!(label, "? Unknown");
    }

    #[test]
    fn test_format_iso_age() {
        assert_eq!(format_iso_age(""), "-");
        assert_eq!(format_iso_age("invalid-iso-date"), "-");
        let now_iso = chrono::Utc::now().to_rfc3339();
        let formatted = format_iso_age(&now_iso);
        assert!(!formatted.is_empty() && formatted != "-");
    }

    #[test]
    fn test_argo_view_state_navigation_and_filtering() {
        let mut state = ArgoViewState::new();
        assert!(state.is_loading);
        assert_eq!(state.selected_idx, 0);
        assert!(state.displayed_applications().is_empty());
        assert!(state.selected_application().is_none());

        let mut app1 = ArgoApplication::from_json(&serde_json::json!({
            "metadata": { "name": "frontend", "namespace": "argocd" },
            "spec": { "project": "web", "destination": { "name": "prod", "namespace": "prod" } },
            "status": { "sync": { "status": "Synced" }, "health": { "status": "Healthy" } }
        }));
        app1.name = "frontend".to_string();

        let mut app2 = ArgoApplication::from_json(&serde_json::json!({
            "metadata": { "name": "backend", "namespace": "argocd" },
            "spec": { "project": "api", "destination": { "name": "prod", "namespace": "prod" } },
            "status": { "sync": { "status": "OutOfSync" }, "health": { "status": "Degraded" } }
        }));
        app2.name = "backend".to_string();

        state.set_applications(
            vec![app1.clone()],
            vec![app1.clone(), app2.clone()],
            true,
            Some("hub-ctx".to_string()),
        );
        assert!(!state.is_loading);
        assert_eq!(state.displayed_applications().len(), 1);
        assert_eq!(state.selected_application().unwrap().name, "frontend");

        // Toggle show all hub apps
        state.toggle_show_all();
        assert!(state.show_all_hub_apps);
        assert_eq!(state.displayed_applications().len(), 2);

        // Selection navigation
        state.select_next();
        assert_eq!(state.selected_idx, 1);
        assert_eq!(state.selected_application().unwrap().name, "backend");
        state.select_next(); // At end
        assert_eq!(state.selected_idx, 1);
        state.select_prev();
        assert_eq!(state.selected_idx, 0);
        state.select_prev(); // At start
        assert_eq!(state.selected_idx, 0);

        // Filter query
        state.filter_query = "back".to_string();
        let filtered = state.filtered_indices();
        assert_eq!(filtered, vec![1]);
        assert_eq!(state.selected_application().unwrap().name, "backend");

        // Filter with no match
        state.filter_query = "nonexistent-query".to_string();
        assert!(state.filtered_indices().is_empty());
        assert!(state.selected_application().is_none());

        // Error state
        state.set_error("Failed to connect".to_string());
        assert_eq!(state.error.as_deref(), Some("Failed to connect"));
        assert!(!state.is_loading);
    }

    #[test]
    fn test_render_argo_view_states() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();

        // 1. Loading state
        let mut state = ArgoViewState::new();
        terminal
            .draw(|f| {
                render_argo_view(f, f.area(), &state);
            })
            .unwrap();

        // 2. Error state
        state.set_error("Failed to connect to cluster".to_string());
        terminal
            .draw(|f| {
                render_argo_view(f, f.area(), &state);
            })
            .unwrap();

        // 3. Empty state
        state.set_applications(vec![], vec![], false, None);
        terminal
            .draw(|f| {
                render_argo_view(f, f.area(), &state);
            })
            .unwrap();

        // 4. Populated table state with remote hub
        let app = ArgoApplication::from_json(&serde_json::json!({
            "metadata": { "name": "web-app", "namespace": "argocd" },
            "spec": {
                "project": "default",
                "source": { "repoURL": "https://github.com/org/repo", "targetRevision": "main" },
                "destination": { "name": "prod", "namespace": "default" }
            },
            "status": {
                "sync": { "status": "Synced" },
                "health": { "status": "Healthy" }
            }
        }));
        state.set_applications(
            vec![app.clone()],
            vec![app],
            true,
            Some("hub-ctx".to_string()),
        );
        terminal
            .draw(|f| {
                render_argo_view(f, f.area(), &state);
            })
            .unwrap();
    }
}
