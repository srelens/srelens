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

    pub fn selected_resource(&self) -> Option<&ArgoResourceItem> {
        let app = self.application.as_ref()?;
        match self.active_tab {
            ArgoDetailTab::ManagedResources => app.resources.get(self.selected_resource_idx),
            ArgoDetailTab::Drift => self.drift_items().get(self.selected_drift_idx).copied(),
            _ => None,
        }
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
    render_argo_detail_view_with(f, area, state, false);
}

/// [`render_argo_detail_view`], also advertising `<a> ArgoCD` when an ArgoCD
/// UI URL is configured and `a` has somewhere to open.
pub fn render_argo_detail_view_with(
    f: &mut Frame,
    area: Rect,
    state: &ArgoDetailViewState,
    has_argo_ui: bool,
) {
    let drift_count = state.drift_items().len();

    let extra_hints = match state.active_tab {
        ArgoDetailTab::ManagedResources | ArgoDetailTab::Drift => {
            "  <Enter/d> Describe  <y> YAML  <x> Actions / AI "
        }
        _ => "  <x> Actions / AI ",
    };

    let title = format!(
        " 🐙 ArgoCD Application: {}/{} (<1-4> Tabs  <s> Sync  <p> Auto-Sync  <R> Hard Refresh  <g> Git  {}<r> Reload{}<Esc> Back) ",
        state.app_namespace,
        state.app_name,
        if has_argo_ui { "<a> ArgoCD  " } else { "" },
        extra_hints
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
        .wrap(Wrap { trim: true })
        .style(Style::default().fg(Theme::cyan()));
        f.render_widget(loading, inner);
        return;
    }

    if let Some(ref err) = state.error {
        let err_msg = Paragraph::new(format!("⚠ Error fetching application: {}", err))
            .wrap(Wrap { trim: true })
            .style(Style::default().fg(Theme::red()));
        f.render_widget(err_msg, inner);
        return;
    }

    let Some(ref app) = state.application else {
        let not_found = Paragraph::new("Application details not available.")
            .wrap(Wrap { trim: true })
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
        .style(Style::default().fg(Theme::label()))
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
            Span::styled(" (Revision: ", Theme::header_label()),
            Span::styled(
                if app.sync_revision.is_empty() {
                    "-"
                } else {
                    &app.sync_revision
                },
                Style::default().fg(Theme::label()),
            ),
            Span::styled(")", Theme::header_label()),
            Span::raw("    "),
            Span::styled("Health: ", Theme::header_label()),
            Span::styled(health_badge, health_style),
            Span::raw("  "),
            Span::styled(&app.health_message, Style::default().fg(Theme::label())),
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

    let p1 = Paragraph::new(app_info).wrap(Wrap { trim: true }).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Theme::border()))
            .title(Span::styled(" Status & Policies ", Theme::title())),
    );

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
            Span::styled(" (", Theme::header_label()),
            Span::styled(dest_server, Style::default().fg(Theme::label())),
            Span::styled(")    ", Theme::header_label()),
            Span::styled("Target Namespace: ", Theme::header_label()),
            Span::styled(
                if app.destination_namespace.is_empty() {
                    "(cluster-scoped / none)"
                } else {
                    app.destination_namespace.as_str()
                },
                Style::default().fg(Theme::cyan()),
            ),
        ]),
    ];

    let p2 = Paragraph::new(source_dest).wrap(Wrap { trim: true }).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Theme::border()))
            .title(Span::styled(" Source & Destination ", Theme::title())),
    );

    // 3. Last Operation Details
    let op_phase = if app.operation_phase.is_empty() {
        "None"
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
                    Style::default().fg(Theme::label())
                },
            ),
        ]),
    ];

    let p3 = Paragraph::new(op_info).wrap(Wrap { trim: true }).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Theme::border()))
            .title(Span::styled(
                " Last Synchronization Operation ",
                Theme::title(),
            )),
    );

    // 4. Action Levers summary
    let actions = vec![
        Line::from(vec![
            Span::styled(
                "<s>",
                Style::default()
                    .fg(Theme::accent())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" Trigger Sync (with prune option)        "),
            Span::styled(
                "<p>",
                Style::default()
                    .fg(Theme::accent())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                " {} Auto-Sync (SRE lever)        ",
                if app.auto_sync_enabled {
                    "Pause"
                } else {
                    "Resume"
                }
            )),
            Span::styled(
                "<R>",
                Style::default()
                    .fg(Theme::accent())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" Hard Refresh (force Git fetch)"),
        ]),
        Line::from(vec![
            Span::styled(
                "<g>",
                Style::default()
                    .fg(Theme::accent())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" Open Git Repository in browser          "),
            Span::styled(
                "<2>",
                Style::default()
                    .fg(Theme::accent())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                " Managed Resources ({})             ",
                app.resources.len()
            )),
            Span::styled(
                "<3>",
                Style::default()
                    .fg(Theme::accent())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(" Drift Inspection ({})", state.drift_items().len())),
        ]),
    ];

    let p4 = Paragraph::new(actions).wrap(Wrap { trim: true }).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Theme::border()))
            .title(Span::styled(
                " Emergency SRE Incident Levers & Navigation ",
                Theme::title(),
            )),
    );

    // Cards keep their old minimum so a short message still looks the same,
    // and grow when a health message, Git URL, or sync error wraps.
    let text_width = area.width.saturating_sub(2).max(1);
    let h1 = (p1.line_count(text_width) as u16).max(7);
    let h2 = (p2.line_count(text_width) as u16).max(5);
    let h3 = (p3.line_count(text_width) as u16).max(5);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(h1),
            Constraint::Length(h2),
            Constraint::Length(h3),
            Constraint::Min(6),
        ])
        .split(area);
    f.render_widget(p1, chunks[0]);
    f.render_widget(p2, chunks[1]);
    f.render_widget(p3, chunks[2]);
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
            .wrap(Wrap { trim: true })
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

    let mut max_kind = "KIND".len();
    let mut max_ns = "NAMESPACE".len();
    let mut max_name = "NAME".len();
    let mut max_status = "STATUS".len();
    let mut max_health = "HEALTH".len();
    let mut max_hook = "HOOK".len();
    let mut max_msg = "MESSAGE".len();

    for res in &app.resources {
        let (sync_badge, _) = sync_status_badge(&res.status);
        let (health_badge, _) = health_status_badge(&res.health);
        let hook_str = res.hook.as_deref().unwrap_or("-");

        max_kind = max_kind.max(res.kind.len());
        max_ns = max_ns.max(res.namespace.len());
        max_name = max_name.max(res.name.len());
        max_status = max_status.max(sync_badge.chars().count());
        max_health = max_health.max(health_badge.chars().count());
        max_hook = max_hook.max(hook_str.len());
        max_msg = max_msg.max(res.message.len());
    }

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
        Constraint::Length((max_kind + 2) as u16),
        Constraint::Length((max_ns + 2) as u16),
        Constraint::Length((max_name + 2) as u16),
        Constraint::Length((max_status + 2) as u16),
        Constraint::Length((max_health + 2) as u16),
        Constraint::Length((max_hook + 2) as u16),
        Constraint::Min((max_msg + 2).max(20) as u16),
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
        let p = Paragraph::new(msg).wrap(Wrap { trim: true });
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

    let mut max_kind = "KIND".len();
    let mut max_ns = "NAMESPACE".len();
    let mut max_name = "NAME".len();
    let mut max_status = "STATUS".len();
    let mut max_health = "HEALTH".len();
    let mut max_reason = "DRIFT / ERROR REASON".len();

    for res in &drift {
        let (sync_badge, _) = sync_status_badge(&res.status);
        let (health_badge, _) = health_status_badge(&res.health);
        let reason_len = if !res.message.is_empty() {
            res.message.len()
        } else if res.status == "OutOfSync" {
            "Resource configuration drifted from Git commit".len()
        } else {
            1
        };

        max_kind = max_kind.max(res.kind.len());
        max_ns = max_ns.max(res.namespace.len());
        max_name = max_name.max(res.name.len());
        max_status = max_status.max(sync_badge.chars().count());
        max_health = max_health.max(health_badge.chars().count());
        max_reason = max_reason.max(reason_len);
    }

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
        Constraint::Length((max_kind + 2) as u16),
        Constraint::Length((max_ns + 2) as u16),
        Constraint::Length((max_name + 2) as u16),
        Constraint::Length((max_status + 2) as u16),
        Constraint::Length((max_health + 2) as u16),
        Constraint::Min((max_reason + 2).max(25) as u16),
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
            .wrap(Wrap { trim: true })
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

    let mut max_id = "ID".len();
    let mut max_rev = "REVISION".len();
    let mut max_dep = "DEPLOYED AT".len();
    let mut max_repo = "REPO URL".len();
    let mut max_path = "PATH".len();

    for hist in &app.sync_history {
        max_id = max_id.max(hist.id.to_string().len());
        max_rev = max_rev.max(hist.revision.len());
        max_dep = max_dep.max(hist.deployed_at.len());
        max_repo = max_repo.max(hist.repo_url.len());
        max_path = max_path.max(hist.path.len());
    }

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
        Constraint::Length((max_id + 2) as u16),
        Constraint::Length((max_rev + 2) as u16),
        Constraint::Length((max_dep + 2) as u16),
        Constraint::Length((max_repo + 2) as u16),
        Constraint::Min((max_path + 2) as u16),
    ];

    let table = Table::new(rows, widths).header(headers);
    f.render_widget(table, area);
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(health_status_badge("Healthy").0, "● Healthy");
        assert_eq!(health_status_badge("Progressing").0, "⟳ Progressing");
        assert_eq!(health_status_badge("Degraded").0, "✖ Degraded");
        assert_eq!(health_status_badge("Missing").0, "✖ Missing");
        assert_eq!(health_status_badge("Suspended").0, "⏸ Suspended");
        assert_eq!(health_status_badge("").0, "- Unknown");
        assert_eq!(health_status_badge("Unknown").0, "? Unknown");
        assert_eq!(health_status_badge("Other").0, "? Unknown");
    }

    #[test]
    fn test_argo_detail_tab_transitions_and_titles() {
        assert_eq!(ArgoDetailTab::Overview.title(0), "1: Overview");
        assert_eq!(
            ArgoDetailTab::ManagedResources.title(0),
            "2: Managed Resources"
        );
        assert_eq!(ArgoDetailTab::Drift.title(0), "3: Drift / Out-of-Sync (0)");
        assert_eq!(
            ArgoDetailTab::Drift.title(3),
            "3: Drift / Out-of-Sync (3) ⚠"
        );
        assert_eq!(
            ArgoDetailTab::RevisionHistory.title(0),
            "4: Revision History"
        );

        assert_eq!(ArgoDetailTab::from_index(0), ArgoDetailTab::Overview);
        assert_eq!(
            ArgoDetailTab::from_index(1),
            ArgoDetailTab::ManagedResources
        );
        assert_eq!(ArgoDetailTab::from_index(2), ArgoDetailTab::Drift);
        assert_eq!(ArgoDetailTab::from_index(3), ArgoDetailTab::RevisionHistory);
        assert_eq!(ArgoDetailTab::from_index(99), ArgoDetailTab::Overview);
    }

    #[test]
    fn test_argo_detail_view_state_navigation_and_selection() {
        let mut state = ArgoDetailViewState::new(
            "my-app".to_string(),
            "argocd".to_string(),
            Some("hub".to_string()),
        );
        assert!(state.is_loading);
        assert_eq!(state.active_tab, ArgoDetailTab::Overview);
        assert!(state.drift_items().is_empty());
        assert!(state.selected_resource().is_none());

        // Tab cycling
        state.next_tab();
        assert_eq!(state.active_tab, ArgoDetailTab::ManagedResources);
        state.next_tab();
        assert_eq!(state.active_tab, ArgoDetailTab::Drift);
        state.next_tab();
        assert_eq!(state.active_tab, ArgoDetailTab::RevisionHistory);
        state.next_tab();
        assert_eq!(state.active_tab, ArgoDetailTab::Overview);
        state.prev_tab();
        assert_eq!(state.active_tab, ArgoDetailTab::RevisionHistory);
        state.set_tab(ArgoDetailTab::ManagedResources);
        assert_eq!(state.active_tab, ArgoDetailTab::ManagedResources);

        // Populate application
        let app = ArgoApplication::from_json(&serde_json::json!({
            "metadata": { "name": "my-app", "namespace": "argocd" },
            "spec": {
                "project": "default",
                "source": { "repoURL": "https://github.com/org/repo", "targetRevision": "v1.0" },
                "destination": { "name": "spoke-1", "namespace": "default" }
            },
            "status": {
                "resources": [
                    { "group": "apps", "version": "v1", "kind": "Deployment", "namespace": "default", "name": "web", "status": "Synced", "health": { "status": "Healthy" } },
                    { "group": "", "version": "v1", "kind": "Service", "namespace": "default", "name": "web-svc", "status": "OutOfSync", "health": { "status": "Degraded" } }
                ],
                "history": [
                    { "id": 1, "revision": "rev-1", "deployedAt": "2026-03-01T00:00:00Z", "source": { "repoURL": "https://github.com/org/repo", "targetRevision": "v1.0", "path": "deploy" } },
                    { "id": 2, "revision": "rev-2", "deployedAt": "2026-03-02T00:00:00Z", "source": { "repoURL": "https://github.com/org/repo", "targetRevision": "v2.0", "path": "deploy" } }
                ]
            }
        }));
        state.set_application(app);
        assert!(!state.is_loading);

        // Drift items: only the OutOfSync/Degraded item
        let drift = state.drift_items();
        assert_eq!(drift.len(), 1);
        assert_eq!(drift[0].name, "web-svc");

        // Managed resources selection
        state.set_tab(ArgoDetailTab::ManagedResources);
        assert_eq!(state.selected_resource().unwrap().name, "web");
        state.select_next();
        assert_eq!(state.selected_resource().unwrap().name, "web-svc");
        state.select_next(); // At end
        assert_eq!(state.selected_resource().unwrap().name, "web-svc");
        state.select_prev();
        assert_eq!(state.selected_resource().unwrap().name, "web");
        state.select_prev(); // At start
        assert_eq!(state.selected_resource().unwrap().name, "web");

        // Drift selection
        state.set_tab(ArgoDetailTab::Drift);
        assert_eq!(state.selected_resource().unwrap().name, "web-svc");
        state.select_next();
        assert_eq!(state.selected_resource().unwrap().name, "web-svc");
        state.select_prev();
        assert_eq!(state.selected_resource().unwrap().name, "web-svc");

        // Revision history selection
        state.set_tab(ArgoDetailTab::RevisionHistory);
        assert_eq!(state.selected_history_idx, 0);
        state.select_next();
        assert_eq!(state.selected_history_idx, 1);
        state.select_next();
        assert_eq!(state.selected_history_idx, 1);
        state.select_prev();
        assert_eq!(state.selected_history_idx, 0);

        // Overview scroll
        state.set_tab(ArgoDetailTab::Overview);
        state.select_next();
        assert_eq!(state.scroll_offset, 1);
        state.select_prev();
        assert_eq!(state.scroll_offset, 0);

        // Error state
        state.set_error("Application deleted".to_string());
        assert_eq!(state.error.as_deref(), Some("Application deleted"));
    }

    #[test]
    fn test_render_argo_detail_view_all_tabs_and_states() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let backend = TestBackend::new(140, 40);
        let mut terminal = Terminal::new(backend).unwrap();

        // 1. Loading state
        let mut state = ArgoDetailViewState::new("app".to_string(), "argocd".to_string(), None);
        terminal
            .draw(|f| {
                render_argo_detail_view(f, f.area(), &state);
            })
            .unwrap();

        // 2. Error state
        state.set_error("Network timeout".to_string());
        terminal
            .draw(|f| {
                render_argo_detail_view(f, f.area(), &state);
            })
            .unwrap();

        // 3. Render all 4 populated tabs
        let app = ArgoApplication::from_json(&serde_json::json!({
            "metadata": { "name": "my-app", "namespace": "argocd", "creationTimestamp": "2026-03-01T00:00:00Z" },
            "spec": {
                "project": "core",
                "source": { "repoURL": "https://github.com/org/repo", "targetRevision": "main", "path": "apps/web" },
                "destination": { "name": "prod-cluster", "server": "https://10.0.0.1:6443", "namespace": "web" },
                "syncPolicy": { "automated": { "prune": true, "selfHeal": true } }
            },
            "status": {
                "sync": { "status": "OutOfSync", "revision": "abcdef1" },
                "health": { "status": "Degraded", "message": "1 replica unavailable" },
                "operationState": { "phase": "Failed", "message": "hook failed", "finishedAt": "2026-03-01T01:00:00Z" },
                "resources": [
                    { "group": "apps", "version": "v1", "kind": "Deployment", "namespace": "web", "name": "web-deployment", "status": "OutOfSync", "health": { "status": "Degraded", "message": "waiting" } }
                ],
                "history": [
                    { "id": 1, "revision": "rev-1", "deployedAt": "2026-03-01T00:00:00Z", "source": { "repoURL": "https://github.com/org/repo", "targetRevision": "main", "path": "apps/web" } }
                ]
            }
        }));
        state.set_application(app);

        // Tab 1: Overview
        state.set_tab(ArgoDetailTab::Overview);
        terminal
            .draw(|f| {
                render_argo_detail_view(f, f.area(), &state);
            })
            .unwrap();

        // Tab 2: Managed Resources
        state.set_tab(ArgoDetailTab::ManagedResources);
        terminal
            .draw(|f| {
                render_argo_detail_view(f, f.area(), &state);
            })
            .unwrap();

        // Tab 3: Drift
        state.set_tab(ArgoDetailTab::Drift);
        terminal
            .draw(|f| {
                render_argo_detail_view(f, f.area(), &state);
            })
            .unwrap();

        // Tab 4: Revision History
        state.set_tab(ArgoDetailTab::RevisionHistory);
        terminal
            .draw(|f| {
                render_argo_detail_view(f, f.area(), &state);
            })
            .unwrap();
    }
}
