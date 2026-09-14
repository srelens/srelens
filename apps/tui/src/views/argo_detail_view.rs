use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, Tabs, Wrap},
    Frame,
};
use srelens_kube::argo::{ArgoApplication, ArgoResourceItem, ArgoSyncHistoryItem};

use crate::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgoDetailTab {
    Overview = 0,
    ManagedResources = 1,
    Drift = 2,
    RevisionHistory = 3,
}

impl ArgoDetailTab {
    pub fn title(&self, drift_count: usize) -> String {
        match self {
            Self::Overview => "1: Overview".to_string(),
            Self::ManagedResources => "2: Managed Resources".to_string(),
            Self::Drift => {
                if drift_count > 0 {
                    format!("3: Drift / Out-of-Sync ({}) ⚠", drift_count)
                } else {
                    "3: Drift / Out-of-Sync (0)".to_string()
                }
            }
            Self::RevisionHistory => "4: Revision History".to_string(),
        }
    }

    pub fn from_index(idx: usize) -> Self {
        match idx {
            0 => Self::Overview,
            1 => Self::ManagedResources,
            2 => Self::Drift,
            3 => Self::RevisionHistory,
            _ => Self::Overview,
        }
    }
}

pub struct ArgoDetailViewState {
    pub app_name: String,
    pub app_namespace: String,
    pub hub_context: Option<String>,
    pub active_tab: ArgoDetailTab,
    pub is_loading: bool,
    pub error: Option<String>,
    pub application: Option<ArgoApplication>,
    pub selected_resource_idx: usize,
    pub selected_drift_idx: usize,
    pub selected_history_idx: usize,
    pub scroll_offset: usize,
    pub search_query: String,
}

impl ArgoDetailViewState {
    pub fn new(name: String, namespace: String, hub_context: Option<String>) -> Self {
        Self {
            app_name: name,
            app_namespace: namespace,
            hub_context,
            active_tab: ArgoDetailTab::Overview,
            is_loading: true,
            error: None,
            application: None,
            selected_resource_idx: 0,
            selected_drift_idx: 0,
            selected_history_idx: 0,
            scroll_offset: 0,
            search_query: String::new(),
        }
    }

    pub fn set_application(&mut self, app: ArgoApplication) {
        self.application = Some(app);
        self.is_loading = false;
        self.error = None;
    }

    pub fn set_error(&mut self, err: String) {
        self.error = Some(err);
        self.is_loading = false;
    }

    pub fn next_tab(&mut self) {
        let curr = self.active_tab as usize;
        self.active_tab = ArgoDetailTab::from_index((curr + 1) % 4);
        self.scroll_offset = 0;
    }

    pub fn prev_tab(&mut self) {
        let curr = self.active_tab as usize;
        self.active_tab = ArgoDetailTab::from_index(if curr == 0 { 3 } else { curr - 1 });
        self.scroll_offset = 0;
    }

    pub fn set_tab(&mut self, tab: ArgoDetailTab) {
        self.active_tab = tab;
        self.scroll_offset = 0;
    }

    pub fn drift_items(&self) -> Vec<&ArgoResourceItem> {
        let Some(ref app) = self.application else {
            return Vec::new();
        };
        app.resources
            .iter()
            .filter(|r| r.status != "Synced" || r.health != "Healthy")
            .collect()
    }

    pub fn select_next(&mut self) {
        match self.active_tab {
            ArgoDetailTab::Overview => {
                self.scroll_offset = self.scroll_offset.saturating_add(1);
            }
            ArgoDetailTab::ManagedResources => {
                if let Some(ref app) = self.application {
                    if !app.resources.is_empty()
                        && self.selected_resource_idx + 1 < app.resources.len()
                    {
                        self.selected_resource_idx += 1;
                    }
                }
            }
            ArgoDetailTab::Drift => {
                let count = self.drift_items().len();
                if count > 0 && self.selected_drift_idx + 1 < count {
                    self.selected_drift_idx += 1;
                }
            }
            ArgoDetailTab::RevisionHistory => {
                if let Some(ref app) = self.application {
                    if !app.sync_history.is_empty()
                        && self.selected_history_idx + 1 < app.sync_history.len()
                    {
                        self.selected_history_idx += 1;
                    }
                }
            }
        }
    }

    pub fn select_prev(&mut self) {
        match self.active_tab {
            ArgoDetailTab::Overview => {
                self.scroll_offset = self.scroll_offset.saturating_sub(1);
            }
            ArgoDetailTab::ManagedResources => {
                if self.selected_resource_idx > 0 {
                    self.selected_resource_idx -= 1;
                }
            }
            ArgoDetailTab::Drift => {
                if self.selected_drift_idx > 0 {
                    self.selected_drift_idx -= 1;
                }
            }
            ArgoDetailTab::RevisionHistory => {
                if self.selected_history_idx > 0 {
                    self.selected_history_idx -= 1;
                }
            }
        }
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

pub fn render_argo_detail_view(f: &mut Frame, area: Rect, state: &ArgoDetailViewState) {
    let drift_count = state.drift_items().len();

    let title = format!(
        " 🐙 ArgoCD Application: {}/{} (<1-4> Tabs  <s> Sync  <p> Auto-Sync  <R> Hard Refresh  <g> Git  <r> Reload  <Esc> Back) ",
        state.app_namespace, state.app_name
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(Theme::border_type())
        .border_style(Style::default().fg(Theme::border()))
        .title(Span::styled(title, Theme::title()));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if state.is_loading && state.application.is_none() {
        let loading = Paragraph::new(format!(
            "⟳ Fetching details for ArgoCD application '{}/{}'...",
            state.app_namespace, state.app_name
        ))
        .style(Style::default().fg(Theme::cyan()));
        f.render_widget(loading, inner);
        return;
    }

    if let Some(ref err) = state.error {
        let err_msg = Paragraph::new(format!("⚠ Error fetching application: {}", err))
            .style(Style::default().fg(Theme::red()));
        f.render_widget(err_msg, inner);
        return;
    }

    let Some(ref app) = state.application else {
        let not_found = Paragraph::new("Application details not available.")
            .style(Style::default().fg(Theme::dim()));
        f.render_widget(not_found, inner);
        return;
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(inner);

    // Tab bar
    let tab_titles: Vec<Line> = vec![
        Line::from(ArgoDetailTab::Overview.title(drift_count)),
        Line::from(ArgoDetailTab::ManagedResources.title(drift_count)),
        Line::from(ArgoDetailTab::Drift.title(drift_count)),
        Line::from(ArgoDetailTab::RevisionHistory.title(drift_count)),
    ];

    let tabs = Tabs::new(tab_titles)
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(Style::default().fg(Theme::border())),
        )
        .select(state.active_tab as usize)
        .style(Style::default().fg(Theme::dim()))
        .highlight_style(
            Style::default()
                .fg(Theme::accent())
                .add_modifier(Modifier::BOLD),
        );
    f.render_widget(tabs, chunks[0]);

    match state.active_tab {
        ArgoDetailTab::Overview => render_overview_tab(f, chunks[1], app, state),
        ArgoDetailTab::ManagedResources => render_resources_tab(f, chunks[1], app, state),
        ArgoDetailTab::Drift => render_drift_tab(f, chunks[1], app, state),
        ArgoDetailTab::RevisionHistory => render_history_tab(f, chunks[1], app, state),
    }
}

fn render_overview_tab(
    f: &mut Frame,
    area: Rect,
    app: &ArgoApplication,
    state: &ArgoDetailViewState,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(7),
            Constraint::Length(5),
            Constraint::Length(5),
            Constraint::Min(6),
        ])
        .split(area);

    // 1. App & Status block
    let (sync_badge, sync_style) = sync_status_badge(&app.sync_status);
    let (health_badge, health_style) = health_status_badge(&app.health_status);

    let app_info = vec![
        Line::from(vec![
            Span::styled("Application: ", Theme::header_label()),
            Span::styled(&app.name, Style::default().add_modifier(Modifier::BOLD)),
            Span::raw("    "),
            Span::styled("Argo Namespace: ", Theme::header_label()),
            Span::styled(&app.namespace, Style::default().fg(Theme::cyan())),
            Span::raw("    "),
            Span::styled("Project: ", Theme::header_label()),
            Span::styled(&app.project, Style::default()),
        ]),
        Line::from(vec![
            Span::styled("Sync Status: ", Theme::header_label()),
            Span::styled(sync_badge, sync_style),
            Span::raw(" (Revision: "),
            Span::styled(
                if app.sync_revision.is_empty() {
                    "-"
                } else {
                    &app.sync_revision
                },
                Style::default().fg(Theme::dim()),
            ),
            Span::raw(")"),
            Span::raw("    "),
            Span::styled("Health: ", Theme::header_label()),
            Span::styled(health_badge, health_style),
            Span::raw("  "),
            Span::styled(&app.health_message, Theme::dim()),
        ]),
        Line::from(vec![
            Span::styled("Created At: ", Theme::header_label()),
            Span::styled(
                if app.created_at.is_empty() {
                    "-"
                } else {
                    &app.created_at
                },
                Style::default(),
            ),
            Span::raw("    "),
            Span::styled("Last Sync: ", Theme::header_label()),
            Span::styled(
                if app.last_sync_time.is_empty() {
                    "-"
                } else {
                    &app.last_sync_time
                },
                Style::default(),
            ),
        ]),
        Line::from(vec![
            Span::styled("Auto-Sync: ", Theme::header_label()),
            Span::styled(
                if app.auto_sync_enabled {
                    "Enabled"
                } else {
                    "Paused / Manual"
                },
                if app.auto_sync_enabled {
                    Style::default().fg(Theme::green())
                } else {
                    Style::default().fg(Theme::yellow())
                },
            ),
            Span::raw("    "),
            Span::styled("Self-Heal: ", Theme::header_label()),
            Span::styled(
                if app.self_heal_enabled {
                    "Enabled"
                } else {
                    "Disabled"
                },
                Style::default(),
            ),
            Span::raw("    "),
            Span::styled("Prune: ", Theme::header_label()),
            Span::styled(
                if app.prune_enabled {
                    "Enabled"
                } else {
                    "Disabled"
                },
                Style::default(),
            ),
        ]),
    ];

    let p1 = Paragraph::new(app_info).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Theme::border()))
            .title(Span::styled(" Status & Policies ", Theme::title())),
    );
    f.render_widget(p1, chunks[0]);

    // 2. Source & Destination
    let dest_name = if !app.destination_name.is_empty() {
        &app.destination_name
    } else {
        "in-cluster"
    };
    let dest_server = if !app.destination_server.is_empty() {
        &app.destination_server
    } else {
        "-"
    };

    let source_dest = vec![
        Line::from(vec![
            Span::styled("Git Repo: ", Theme::header_label()),
            Span::styled(&app.repo_url, Style::default().fg(Theme::cyan())),
            Span::raw("    "),
            Span::styled("Target Revision: ", Theme::header_label()),
            Span::styled(&app.target_revision, Style::default()),
            Span::raw("    "),
            Span::styled("Path: ", Theme::header_label()),
            Span::styled(&app.path, Style::default()),
        ]),
        Line::from(vec![
            Span::styled("Target Cluster: ", Theme::header_label()),
            Span::styled(dest_name, Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" ("),
            Span::styled(dest_server, Theme::dim()),
            Span::raw(")    "),
            Span::styled("Target Namespace: ", Theme::header_label()),
            Span::styled(&app.destination_namespace, Style::default().fg(Theme::cyan())),
        ]),
    ];

    let p2 = Paragraph::new(source_dest).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Theme::border()))
            .title(Span::styled(" Source & Destination ", Theme::title())),
    );
    f.render_widget(p2, chunks[1]);

    // 3. Operation status
    let op_phase = if app.operation_phase.is_empty() {
        "Succeeded"
    } else {
        &app.operation_phase
    };
    let op_style = match op_phase {
        "Succeeded" => Theme::status_ok(),
        "Failed" | "Error" => Theme::status_error(),
        _ => Theme::status_warn(),
    };

    let op_info = vec![
        Line::from(vec![
            Span::styled("Phase: ", Theme::header_label()),
            Span::styled(op_phase, op_style),
            Span::raw("    "),
            Span::styled("Finished: ", Theme::header_label()),
            Span::styled(
                if app.last_sync_time.is_empty() {
                    "-"
                } else {
                    &app.last_sync_time
                },
                Style::default(),
            ),
        ]),
        Line::from(vec![
            Span::styled("Message: ", Theme::header_label()),
            Span::styled(
                if app.operation_message.is_empty() {
                    "No operation errors"
                } else {
                    &app.operation_message
                },
                if app.operation_phase == "Failed" {
                    Style::default().fg(Theme::red())
                } else {
                    Style::default().fg(Theme::dim())
                },
            ),
        ]),
    ];

    let p3 = Paragraph::new(op_info).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Theme::border()))
            .title(Span::styled(" Last Synchronization Operation ", Theme::title())),
    );
    f.render_widget(p3, chunks[2]);

    // 4. Action Levers summary
    let actions = vec![
        Line::from(vec![
            Span::styled("<s>", Style::default().fg(Theme::accent()).add_modifier(Modifier::BOLD)),
            Span::raw(" Trigger Sync (with prune option)        "),
            Span::styled("<p>", Style::default().fg(Theme::accent()).add_modifier(Modifier::BOLD)),
            Span::raw(format!(" {} Auto-Sync (SRE lever)        ", if app.auto_sync_enabled { "Pause" } else { "Resume" })),
            Span::styled("<R>", Style::default().fg(Theme::accent()).add_modifier(Modifier::BOLD)),
            Span::raw(" Hard Refresh (force Git fetch)"),
        ]),
        Line::from(vec![
            Span::styled("<g>", Style::default().fg(Theme::accent()).add_modifier(Modifier::BOLD)),
            Span::raw(" Open Git Repository in browser          "),
            Span::styled("<2>", Style::default().fg(Theme::accent()).add_modifier(Modifier::BOLD)),
            Span::raw(format!(" Managed Resources ({})             ", app.resources.len())),
            Span::styled("<3>", Style::default().fg(Theme::accent()).add_modifier(Modifier::BOLD)),
            Span::raw(format!(" Drift Inspection ({})", state.drift_items().len())),
        ]),
    ];

    let p4 = Paragraph::new(actions).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Theme::border()))
            .title(Span::styled(" Emergency SRE Incident Levers & Navigation ", Theme::title())),
    );
    f.render_widget(p4, chunks[3]);
}

fn render_resources_tab(
    f: &mut Frame,
    area: Rect,
    app: &ArgoApplication,
    state: &ArgoDetailViewState,
) {
    if app.resources.is_empty() {
        let empty = Paragraph::new("No managed Kubernetes resources reported by ArgoCD.")
            .style(Style::default().fg(Theme::dim()));
        f.render_widget(empty, area);
        return;
    }

    let headers = Row::new(vec![
        Cell::from("KIND").style(Theme::table_header()),
        Cell::from("NAMESPACE").style(Theme::table_header()),
        Cell::from("NAME").style(Theme::table_header()),
        Cell::from("STATUS").style(Theme::table_header()),
        Cell::from("HEALTH").style(Theme::table_header()),
        Cell::from("HOOK").style(Theme::table_header()),
        Cell::from("MESSAGE").style(Theme::table_header()),
    ])
    .height(1)
    .bottom_margin(1);

    let rows: Vec<Row> = app
        .resources
        .iter()
        .enumerate()
        .map(|(idx, res)| {
            let is_selected = idx == state.selected_resource_idx;
            let (sync_badge, sync_style) = sync_status_badge(&res.status);
            let (health_badge, health_style) = health_status_badge(&res.health);

            let row_style = if is_selected {
                Theme::selected_row()
            } else {
                Style::default()
            };

            Row::new(vec![
                Cell::from(res.kind.as_str()),
                Cell::from(res.namespace.as_str()),
                Cell::from(res.name.as_str()),
                Cell::from(sync_badge).style(sync_style),
                Cell::from(health_badge).style(health_style),
                Cell::from(res.hook.as_deref().unwrap_or("-")),
                Cell::from(res.message.as_str()),
            ])
            .style(row_style)
        })
        .collect();

    let widths = [
        Constraint::Length(16),
        Constraint::Length(16),
        Constraint::Length(28),
        Constraint::Length(14),
        Constraint::Length(16),
        Constraint::Length(10),
        Constraint::Min(30),
    ];

    let table = Table::new(rows, widths).header(headers);
    f.render_widget(table, area);
}

fn render_drift_tab(
    f: &mut Frame,
    area: Rect,
    _app: &ArgoApplication,
    state: &ArgoDetailViewState,
) {
    let drift = state.drift_items();
    if drift.is_empty() {
        let msg = vec![
            Line::from(""),
            Line::from(Span::styled(
                "  ✔ No Drift Detected",
                Style::default()
                    .fg(Theme::green())
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "  All managed Kubernetes resources match the desired Git state and report Healthy.",
                Theme::dim(),
            )),
        ];
        let p = Paragraph::new(msg);
        f.render_widget(p, area);
        return;
    }

    let headers = Row::new(vec![
        Cell::from("KIND").style(Theme::table_header()),
        Cell::from("NAMESPACE").style(Theme::table_header()),
        Cell::from("NAME").style(Theme::table_header()),
        Cell::from("STATUS").style(Theme::table_header()),
        Cell::from("HEALTH").style(Theme::table_header()),
        Cell::from("DRIFT / ERROR REASON").style(Theme::table_header()),
    ])
    .height(1)
    .bottom_margin(1);

    let rows: Vec<Row> = drift
        .iter()
        .enumerate()
        .map(|(idx, res)| {
            let is_selected = idx == state.selected_drift_idx;
            let (sync_badge, sync_style) = sync_status_badge(&res.status);
            let (health_badge, health_style) = health_status_badge(&res.health);

            let row_style = if is_selected {
                Theme::selected_row()
            } else {
                Style::default()
            };

            let reason = if !res.message.is_empty() {
                res.message.as_str()
            } else if res.status == "OutOfSync" {
                "Resource configuration drifted from Git commit"
            } else {
                "-"
            };

            Row::new(vec![
                Cell::from(res.kind.as_str()),
                Cell::from(res.namespace.as_str()),
                Cell::from(res.name.as_str()),
                Cell::from(sync_badge).style(sync_style),
                Cell::from(health_badge).style(health_style),
                Cell::from(reason),
            ])
            .style(row_style)
        })
        .collect();

    let widths = [
        Constraint::Length(16),
        Constraint::Length(16),
        Constraint::Length(28),
        Constraint::Length(14),
        Constraint::Length(16),
        Constraint::Min(40),
    ];

    let table = Table::new(rows, widths).header(headers);
    f.render_widget(table, area);
}

fn render_history_tab(
    f: &mut Frame,
    area: Rect,
    app: &ArgoApplication,
    state: &ArgoDetailViewState,
) {
    if app.sync_history.is_empty() {
        let empty = Paragraph::new("No synchronization history available.")
            .style(Style::default().fg(Theme::dim()));
        f.render_widget(empty, area);
        return;
    }

    let headers = Row::new(vec![
        Cell::from("ID").style(Theme::table_header()),
        Cell::from("REVISION").style(Theme::table_header()),
        Cell::from("DEPLOYED AT").style(Theme::table_header()),
        Cell::from("REPO URL").style(Theme::table_header()),
        Cell::from("PATH").style(Theme::table_header()),
    ])
    .height(1)
    .bottom_margin(1);

    let rows: Vec<Row> = app
        .sync_history
        .iter()
        .enumerate()
        .map(|(idx, hist)| {
            let is_selected = idx == state.selected_history_idx;
            let row_style = if is_selected {
                Theme::selected_row()
            } else {
                Style::default()
            };

            Row::new(vec![
                Cell::from(hist.id.to_string()),
                Cell::from(hist.revision.as_str()),
                Cell::from(hist.deployed_at.as_str()),
                Cell::from(hist.repo_url.as_str()),
                Cell::from(hist.path.as_str()),
            ])
            .style(row_style)
        })
        .collect();

    let widths = [
        Constraint::Length(6),
        Constraint::Length(14),
        Constraint::Length(25),
        Constraint::Min(35),
        Constraint::Length(20),
    ];

    let table = Table::new(rows, widths).header(headers);
    f.render_widget(table, area);
}
