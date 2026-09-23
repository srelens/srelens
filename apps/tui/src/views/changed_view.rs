use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Cell, Paragraph, Row, Table, TableState, Wrap},
    Frame,
};
use srelens_kube::changed::{
    AppDeploymentChange, ChangedTriageReport, FailureCategory, IncidentStatus, InfraChangeItem,
    RolloutStatus,
};
use std::time::Duration;

use crate::theme::Theme;
use crate::views::sanitize_span_text;

pub const WINDOWS: &[(&str, Duration)] = &[
    ("15m", Duration::from_secs(900)),
    ("30m", Duration::from_secs(1800)),
    ("1h", Duration::from_secs(3600)),
    ("3h", Duration::from_secs(10800)),
    ("24h", Duration::from_secs(86400)),
];

pub type VerdictFilter = IncidentFilter;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IncidentFilter {
    All,
    CrashingOnly,
    OomOnly,
    ErrorOnly,
    PendingOnly,
    RollingOnly,
    HealthyOnly,
}

impl IncidentFilter {
    pub fn label(&self) -> &'static str {
        match self {
            Self::All => "ALL",
            Self::CrashingOnly => "CRASH",
            Self::OomOnly => "OOM",
            Self::ErrorOnly => "ERROR",
            Self::PendingOnly => "PENDING (INFRA)",
            Self::RollingOnly => "ROLLING",
            Self::HealthyOnly => "HEALTHY",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            Self::All => Self::CrashingOnly,
            Self::CrashingOnly => Self::OomOnly,
            Self::OomOnly => Self::ErrorOnly,
            Self::ErrorOnly => Self::PendingOnly,
            Self::PendingOnly => Self::RollingOnly,
            Self::RollingOnly => Self::HealthyOnly,
            Self::HealthyOnly => Self::All,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangedTab {
    Deployments,
    Infra,
}

pub struct ChangedViewState {
    pub report: Option<ChangedTriageReport>,
    pub selected_idx: usize,
    pub infra_selected_idx: usize,
    pub active_tab: ChangedTab,
    pub is_loading: bool,
    pub error: Option<String>,
    pub filter_query: String,
    pub window_idx: usize,
    pub incident_filter: IncidentFilter,
}

impl ChangedViewState {
    pub fn new() -> Self {
        Self {
            report: None,
            selected_idx: 0,
            infra_selected_idx: 0,
            active_tab: ChangedTab::Deployments,
            is_loading: true,
            error: None,
            filter_query: String::new(),
            window_idx: 2, // Default to 1h
            incident_filter: IncidentFilter::All,
        }
    }

    pub fn current_window(&self) -> Duration {
        WINDOWS[self.window_idx.min(WINDOWS.len() - 1)].1
    }

    pub fn current_window_label(&self) -> &'static str {
        WINDOWS[self.window_idx.min(WINDOWS.len() - 1)].0
    }

    pub fn next_window(&mut self) {
        if self.window_idx + 1 < WINDOWS.len() {
            self.window_idx += 1;
        } else {
            self.window_idx = 0;
        }
        self.selected_idx = 0;
        self.infra_selected_idx = 0;
        self.is_loading = true;
    }

    pub fn prev_window(&mut self) {
        if self.window_idx > 0 {
            self.window_idx -= 1;
        } else {
            self.window_idx = WINDOWS.len() - 1;
        }
        self.selected_idx = 0;
        self.infra_selected_idx = 0;
        self.is_loading = true;
    }

    pub fn set_window_by_str(&mut self, s: &str) {
        let clean = s.trim().to_ascii_lowercase();
        for (i, (lbl, _)) in WINDOWS.iter().enumerate() {
            if *lbl == clean || clean.starts_with(lbl) {
                self.window_idx = i;
                self.selected_idx = 0;
                self.infra_selected_idx = 0;
                self.is_loading = true;
                return;
            }
        }
    }

    pub fn cycle_filter(&mut self) {
        self.incident_filter = self.incident_filter.next();
        self.selected_idx = 0;
    }

    pub fn cycle_verdict_filter(&mut self) {
        self.cycle_filter();
    }

    pub fn toggle_tab(&mut self) {
        self.active_tab = match self.active_tab {
            ChangedTab::Deployments => ChangedTab::Infra,
            ChangedTab::Infra => ChangedTab::Deployments,
        };
    }

    pub fn set_report(&mut self, report: ChangedTriageReport) {
        self.report = Some(report);
        self.is_loading = false;
        self.error = None;
        let dep_count = self.filtered_deployments().len();
        if self.selected_idx >= dep_count {
            self.selected_idx = dep_count.saturating_sub(1);
        }
        let infra_count = self.filtered_infra().len();
        if self.infra_selected_idx >= infra_count {
            self.infra_selected_idx = infra_count.saturating_sub(1);
        }
    }

    pub fn set_error(&mut self, err: String) {
        self.error = Some(err);
        self.is_loading = false;
    }

    pub fn filtered_deployments(&self) -> Vec<&AppDeploymentChange> {
        let Some(report) = &self.report else {
            return Vec::new();
        };
        let q = self.filter_query.trim().to_ascii_lowercase();
        report
            .deployments
            .iter()
            .filter(|d| match self.incident_filter {
                IncidentFilter::All => true,
                IncidentFilter::CrashingOnly => d.incident_status == IncidentStatus::CrashLoop,
                IncidentFilter::OomOnly => d.incident_status == IncidentStatus::OomKilled,
                IncidentFilter::ErrorOnly => {
                    d.incident_status == IncidentStatus::ConfigError
                        || d.incident_status == IncidentStatus::ImageError
                }
                IncidentFilter::PendingOnly => d.incident_status == IncidentStatus::Pending,
                IncidentFilter::RollingOnly => {
                    d.incident_status == IncidentStatus::Rolling
                        || d.incident_status == IncidentStatus::Stalled
                }
                IncidentFilter::HealthyOnly => d.incident_status == IncidentStatus::Healthy,
            })
            .filter(|d| {
                if q.is_empty() {
                    return true;
                }
                d.app_name.to_ascii_lowercase().contains(&q)
                    || d.namespace.to_ascii_lowercase().contains(&q)
                    || d.image_diff.to_ascii_lowercase().contains(&q)
                    || d.failure_detail.to_ascii_lowercase().contains(&q)
                    || d.current_images
                        .iter()
                        .any(|img| img.to_ascii_lowercase().contains(&q))
                    || d.gitops
                        .as_ref()
                        .map(|g| {
                            g.app_name.to_ascii_lowercase().contains(&q)
                                || g.sync_revision.to_ascii_lowercase().contains(&q)
                                || g.target_revision.to_ascii_lowercase().contains(&q)
                        })
                        .unwrap_or(false)
            })
            .collect()
    }

    pub fn filtered_infra(&self) -> Vec<&InfraChangeItem> {
        let Some(report) = &self.report else {
            return Vec::new();
        };
        let q = self.filter_query.trim().to_ascii_lowercase();
        report
            .infra_changes
            .iter()
            .filter(|item| {
                if q.is_empty() {
                    return true;
                }
                item.name.to_ascii_lowercase().contains(&q)
                    || item.namespace.to_ascii_lowercase().contains(&q)
                    || item.kind.to_ascii_lowercase().contains(&q)
                    || item.reason.to_ascii_lowercase().contains(&q)
                    || item.message.to_ascii_lowercase().contains(&q)
            })
            .collect()
    }

    pub fn selected_deployment(&self) -> Option<&AppDeploymentChange> {
        let deps = self.filtered_deployments();
        deps.get(self.selected_idx).copied()
    }

    pub fn selected_infra(&self) -> Option<&InfraChangeItem> {
        let infra = self.filtered_infra();
        infra.get(self.infra_selected_idx).copied()
    }

    pub fn select_next(&mut self) {
        match self.active_tab {
            ChangedTab::Deployments => {
                let count = self.filtered_deployments().len();
                if count > 0 && self.selected_idx + 1 < count {
                    self.selected_idx += 1;
                }
            }
            ChangedTab::Infra => {
                let count = self.filtered_infra().len();
                if count > 0 && self.infra_selected_idx + 1 < count {
                    self.infra_selected_idx += 1;
                }
            }
        }
    }

    pub fn select_prev(&mut self) {
        match self.active_tab {
            ChangedTab::Deployments => {
                if self.selected_idx > 0 {
                    self.selected_idx -= 1;
                }
            }
            ChangedTab::Infra => {
                if self.infra_selected_idx > 0 {
                    self.infra_selected_idx -= 1;
                }
            }
        }
    }

    pub fn select_first(&mut self) {
        match self.active_tab {
            ChangedTab::Deployments => self.selected_idx = 0,
            ChangedTab::Infra => self.infra_selected_idx = 0,
        }
    }

    pub fn select_last(&mut self) {
        match self.active_tab {
            ChangedTab::Deployments => {
                let count = self.filtered_deployments().len();
                self.selected_idx = count.saturating_sub(1);
            }
            ChangedTab::Infra => {
                let count = self.filtered_infra().len();
                self.infra_selected_idx = count.saturating_sub(1);
            }
        }
    }
}

pub fn render_changed_view(f: &mut Frame, area: Rect, state: &ChangedViewState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Summary banner
            Constraint::Min(8),    // Main workspace
            Constraint::Length(1), // Footer shortcuts
        ])
        .split(area);

    render_summary_banner(f, chunks[0], state);

    if state.is_loading && state.report.is_none() {
        let loading_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Theme::border()))
            .title(Span::styled(" SRE Incident Investigation ", Theme::title()));
        let loading_p =
            Paragraph::new(vec![
                Line::from(""),
                Line::from(vec![
                Span::styled("⚡ ", Style::default().fg(Theme::cyan())),
                Span::styled(
                    "Analyzing deployments, GitOps releases, error logs, and pending blockers...",
                    Style::default().fg(Theme::fg()).add_modifier(Modifier::BOLD),
                ),
            ]),
            ])
            .alignment(Alignment::Center)
            .block(loading_block)
            .wrap(Wrap { trim: true });
        f.render_widget(loading_p, chunks[1]);
        render_footer(f, chunks[2], state);
        return;
    }

    if let Some(err) = &state.error {
        let err_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Theme::status_error())
            .title(Span::styled(" Investigation Error ", Theme::status_error()));
        let err_p = Paragraph::new(format!("Failed to analyze cluster changes: {err}"))
            .style(Theme::status_error())
            .block(err_block)
            .wrap(Wrap { trim: true });
        f.render_widget(err_p, chunks[1]);
        render_footer(f, chunks[2], state);
        return;
    }

    match state.active_tab {
        ChangedTab::Deployments => render_deployments_tab(f, chunks[1], state),
        ChangedTab::Infra => render_infra_tab(f, chunks[1], state),
    }

    render_footer(f, chunks[2], state);
}

fn render_summary_banner(f: &mut Frame, area: Rect, state: &ChangedViewState) {
    let (crashing, oom, error, pending, rolling, healthy) = if let Some(r) = &state.report {
        (
            r.summary.crashing_count,
            r.summary.oom_count,
            r.summary.error_count,
            r.summary.pending_count,
            r.summary.rolling_count,
            r.summary.healthy_count,
        )
    } else {
        (0, 0, 0, 0, 0, 0)
    };

    let infra_count = state
        .report
        .as_ref()
        .map(|r| r.infra_changes.len())
        .unwrap_or(0);

    let border_style = if crashing > 0 || oom > 0 {
        Theme::status_error()
    } else if error > 0 || pending > 0 {
        Theme::status_warn()
    } else if rolling > 0 {
        Style::default().fg(Theme::cyan())
    } else if healthy > 0 {
        Theme::status_ok()
    } else {
        Style::default().fg(Theme::border())
    };

    let title_badge = match state.active_tab {
        ChangedTab::Deployments => {
            " 🚨 CHANGED & TRIAGE — POST-PAGE INCIDENT INVESTIGATOR [Tab: Deployments] "
        }
        ChangedTab::Infra => " 📦 CHANGED & TRIAGE — INFRASTRUCTURE CHANGES [Tab: Infra] ",
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        .title(Span::styled(title_badge, Theme::title()));

    let line1 = Line::from(vec![
        Span::styled(" Workload Health: ", Theme::header_label()),
        Span::styled(
            format!(" 💥 CRASH: {crashing} "),
            if crashing > 0 {
                Style::default()
                    .bg(Theme::red())
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Theme::dim())
            },
        ),
        Span::raw("  "),
        Span::styled(
            format!(" 💀 OOM: {oom} "),
            if oom > 0 {
                Style::default()
                    .bg(Theme::red())
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Theme::dim())
            },
        ),
        Span::raw("  "),
        Span::styled(
            format!(" ⚠️ ERROR: {error} "),
            if error > 0 {
                Style::default()
                    .bg(Theme::yellow())
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Theme::dim())
            },
        ),
        Span::raw("  "),
        Span::styled(
            format!(" ⏳ PENDING: {pending} "),
            if pending > 0 {
                Style::default()
                    .bg(Theme::yellow())
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Theme::dim())
            },
        ),
        Span::raw("  "),
        Span::styled(
            format!(" 🔄 ROLLING: {rolling} "),
            if rolling > 0 {
                Style::default()
                    .bg(Theme::cyan())
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Theme::dim())
            },
        ),
        Span::raw("  "),
        Span::styled(
            format!(" 🟢 HEALTHY: {healthy} "),
            if healthy > 0 {
                Style::default()
                    .bg(Theme::green())
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Theme::dim())
            },
        ),
        Span::raw("  │  "),
        Span::styled("Window: ", Theme::header_label()),
        Span::styled(
            format!("[{}]", state.current_window_label()),
            Style::default()
                .fg(Theme::accent())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  │  "),
        Span::styled("Filter: ", Theme::header_label()),
        Span::styled(
            format!("[{}]", state.incident_filter.label()),
            Style::default()
                .fg(Theme::cyan())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  │  "),
        Span::styled("Infra: ", Theme::header_label()),
        Span::styled(
            format!("{infra_count}"),
            Style::default()
                .fg(Theme::fg())
                .add_modifier(Modifier::BOLD),
        ),
    ]);

    let p = Paragraph::new(vec![line1])
        .block(block)
        .wrap(Wrap { trim: true });
    f.render_widget(p, area);
}

fn render_deployments_tab(f: &mut Frame, area: Rect, state: &ChangedViewState) {
    if area.height >= 26 {
        // Split vertically into upper table (45%) and lower diagnostic card (55%)
        let sub = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
            .split(area);
        render_deployments_table(f, sub[0], state);
        render_deployment_diagnostic_card(f, sub[1], state);
    } else {
        render_deployments_table(f, area, state);
    }
}

fn render_deployments_table(f: &mut Frame, area: Rect, state: &ChangedViewState) {
    let deps = state.filtered_deployments();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Theme::border()))
        .title(Span::styled(
            " Workloads Changed in Window ",
            Theme::table_header(),
        ));

    if deps.is_empty() {
        let msg = if state.filter_query.is_empty() {
            format!(
                "No workloads modified within the last {}. Use [ or ] to broaden the time window.",
                state.current_window_label()
            )
        } else {
            format!(
                "No workloads matching query '{}'. Press / to change search or Esc to clear.",
                state.filter_query
            )
        };
        let p = Paragraph::new(msg)
            .style(Style::default().fg(Theme::dim()))
            .block(block)
            .wrap(Wrap { trim: true });
        f.render_widget(p, area);
        return;
    }

    let header_cells = [
        Cell::from(Span::styled("STATUS", Theme::table_header())),
        Cell::from(Span::styled("WORKLOAD", Theme::table_header())),
        Cell::from(Span::styled("NAMESPACE", Theme::table_header())),
        Cell::from(Span::styled("REVISION / GITOPS", Theme::table_header())),
        Cell::from(Span::styled("READY", Theme::table_header())),
        Cell::from(Span::styled("ROOT CAUSE / DETAIL", Theme::table_header())),
        Cell::from(Span::styled("AGE", Theme::table_header())),
    ];
    let header = Row::new(header_cells).height(1).bottom_margin(0);

    let rows: Vec<Row> = deps
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let is_selected = i == state.selected_idx;

            let status_cell = match d.incident_status {
                IncidentStatus::CrashLoop => Cell::from(Span::styled(
                    " 💥 CRASH ",
                    Style::default()
                        .bg(Theme::red())
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                )),
                IncidentStatus::OomKilled => Cell::from(Span::styled(
                    " 💀 OOM ",
                    Style::default()
                        .bg(Theme::red())
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                )),
                IncidentStatus::ConfigError => Cell::from(Span::styled(
                    " ⚠️ ERROR ",
                    Style::default()
                        .bg(Theme::yellow())
                        .fg(Color::Black)
                        .add_modifier(Modifier::BOLD),
                )),
                IncidentStatus::ImageError => Cell::from(Span::styled(
                    " 🚫 IMAGE ",
                    Style::default()
                        .bg(Theme::red())
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                )),
                IncidentStatus::Pending => Cell::from(Span::styled(
                    " ⏳ PENDING ",
                    Style::default()
                        .bg(Theme::yellow())
                        .fg(Color::Black)
                        .add_modifier(Modifier::BOLD),
                )),
                IncidentStatus::Stalled => Cell::from(Span::styled(
                    " 🚫 STALLED ",
                    Style::default()
                        .bg(Theme::red())
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                )),
                IncidentStatus::Rolling => Cell::from(Span::styled(
                    " 🔄 ROLLING ",
                    Style::default()
                        .bg(Theme::cyan())
                        .fg(Color::Black)
                        .add_modifier(Modifier::BOLD),
                )),
                IncidentStatus::Healthy => Cell::from(Span::styled(
                    " 🟢 HEALTHY ",
                    Style::default()
                        .bg(Theme::green())
                        .fg(Color::Black)
                        .add_modifier(Modifier::BOLD),
                )),
                IncidentStatus::ScaledDown => {
                    Cell::from(Span::styled(" ⚪ OFF ", Style::default().fg(Theme::dim())))
                }
                IncidentStatus::Unknown => {
                    Cell::from(Span::styled(" - ", Style::default().fg(Theme::dim())))
                }
            };

            let name_display = match d.kind.as_str() {
                "StatefulSet" => format!("{} (sts)", d.app_name),
                "CronJob" => format!("{} (cj)", d.app_name),
                "Job" => format!("{} (job)", d.app_name),
                _ => d.app_name.clone(),
            };
            let name_cell = Cell::from(Span::styled(
                name_display,
                Style::default()
                    .fg(Theme::fg())
                    .add_modifier(Modifier::BOLD),
            ));
            let ns_cell = Cell::from(Span::styled(
                &d.namespace,
                Style::default().fg(Theme::dim()),
            ));

            let gitops_str = if let Some(ref g) = d.gitops {
                format!("r{} ({})", d.current_revision, g.sync_revision)
            } else {
                format!("r{}", d.current_revision)
            };
            let rev_cell = Cell::from(Span::styled(gitops_str, Style::default().fg(Theme::cyan())));

            let ready_str = format!("{}/{}", d.ready_replicas, d.desired_replicas);
            let ready_style = if d.ready_replicas < d.desired_replicas {
                Style::default()
                    .fg(Theme::red())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Theme::green())
            };
            let ready_cell = Cell::from(Span::styled(ready_str, ready_style));

            let detail_text = format!("{} {}", d.failure_category.badge(), d.failure_detail);
            let detail_style = match d.failure_category {
                FailureCategory::Compute | FailureCategory::Storage | FailureCategory::Network => {
                    Style::default()
                        .fg(Theme::yellow())
                        .add_modifier(Modifier::BOLD)
                }
                FailureCategory::App | FailureCategory::Image => Style::default()
                    .fg(Theme::red())
                    .add_modifier(Modifier::BOLD),
                FailureCategory::None => Style::default().fg(Theme::green()),
            };
            let detail_cell =
                Cell::from(Span::styled(sanitize_span_text(&detail_text), detail_style));

            let age_cell = Cell::from(Span::styled(
                &d.deployed_age,
                Style::default().fg(Theme::dim()),
            ));

            let row = Row::new(vec![
                status_cell,
                name_cell,
                ns_cell,
                rev_cell,
                ready_cell,
                detail_cell,
                age_cell,
            ]);

            if is_selected {
                row.style(Theme::selected_row())
            } else {
                row
            }
        })
        .collect();

    let widths = [
        Constraint::Length(12),
        Constraint::Length(22),
        Constraint::Length(14),
        Constraint::Length(16),
        Constraint::Length(9),
        Constraint::Min(30),
        Constraint::Length(8),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .column_spacing(1);

    let mut table_state = TableState::default();
    table_state.select(Some(state.selected_idx));
    f.render_stateful_widget(table, area, &mut table_state);
}

fn render_deployment_diagnostic_card(f: &mut Frame, area: Rect, state: &ChangedViewState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Theme::border_focus()))
        .title(Span::styled(
            " 🔍 Incident Diagnostic & Root Cause Investigator ",
            Theme::title(),
        ));

    let Some(d) = state.selected_deployment() else {
        let p = Paragraph::new(
            "Select a workload above to inspect root cause, GitOps commits, and error logs.",
        )
        .style(Style::default().fg(Theme::dim()))
        .block(block)
        .wrap(Wrap { trim: true });
        f.render_widget(p, area);
        return;
    };

    let mut lines = Vec::new();

    // Line 1: Workload Identity & Replicas
    lines.push(Line::from(vec![
        Span::styled("Workload: ", Theme::header_label()),
        Span::styled(
            format!("{}/{} ({})", d.namespace, d.app_name, d.kind),
            Style::default()
                .fg(Theme::cyan())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("   "),
        Span::styled("Revision: ", Theme::header_label()),
        Span::styled(
            format!("rev {}", d.current_revision),
            Style::default()
                .fg(Theme::accent())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("   "),
        Span::styled("Replicas: ", Theme::header_label()),
        Span::styled(
            format!(
                "Desired: {}, Updated: {}, Ready: {}, Available: {}",
                d.desired_replicas, d.updated_replicas, d.ready_replicas, d.available_replicas
            ),
            Theme::header_val(),
        ),
        Span::raw("   "),
        Span::styled("Deployed: ", Theme::header_label()),
        Span::styled(
            format!("{} ago", d.deployed_age),
            Style::default().fg(Theme::dim()),
        ),
    ]));

    // Line 1b: ArgoCD Rollout in Window (if detected)
    if let Some(ref argo_msg) = d.argo_rollout_in_window {
        lines.push(Line::from(vec![
            Span::styled("🐙 ArgoCD Rollout: ", Theme::header_label()),
            Span::styled(
                sanitize_span_text(argo_msg),
                Style::default()
                    .fg(Theme::cyan())
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    }

    // Line 2: GitOps / ArgoCD Panel (if available)
    if let Some(ref g) = d.gitops {
        let sync_style = if g.sync_status == "Synced" && g.health_status == "Healthy" {
            Style::default().fg(Theme::green())
        } else {
            Style::default()
                .fg(Theme::yellow())
                .add_modifier(Modifier::BOLD)
        };
        lines.push(Line::from(vec![
            Span::styled("🐙 GitOps Release: ", Theme::header_label()),
            Span::styled(format!("{} ({})", g.app_name, g.sync_status), sync_style),
            Span::raw("   "),
            Span::styled("Git: ", Theme::header_label()),
            Span::styled(
                format!("{} ({})", g.sync_revision, g.target_revision),
                Style::default()
                    .fg(Theme::accent())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("   "),
            Span::styled("Synced: ", Theme::header_label()),
            Span::styled(
                format!("{} ago", g.sync_age),
                Style::default().fg(Theme::dim()),
            ),
        ]));
        if let Some(ref msg) = g.sync_message {
            lines.push(Line::from(vec![
                Span::styled("   Sync Error: ", Theme::header_label()),
                Span::styled(
                    sanitize_span_text(msg),
                    Style::default()
                        .fg(Theme::red())
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
        }
    }

    // Line 3: Root Cause & Failure Detail
    let cat_style = match d.failure_category {
        FailureCategory::Compute | FailureCategory::Storage | FailureCategory::Network => {
            Style::default()
                .fg(Theme::yellow())
                .add_modifier(Modifier::BOLD)
        }
        FailureCategory::App | FailureCategory::Image => Style::default()
            .fg(Theme::red())
            .add_modifier(Modifier::BOLD),
        FailureCategory::None => Style::default().fg(Theme::green()),
    };
    lines.push(Line::from(vec![
        Span::styled("Root Cause: ", Theme::header_label()),
        Span::styled(format!("{} ", d.failure_category.badge()), cat_style),
        Span::styled(
            sanitize_span_text(&d.failure_detail),
            Style::default()
                .fg(Theme::fg())
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    // Line 4: Symptoms (multi-line pod breakdown)
    if !d.pod_symptoms.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            "Symptoms:",
            Style::default()
                .fg(Theme::red())
                .add_modifier(Modifier::BOLD),
        )]));
        for ps in &d.pod_symptoms {
            let text = if !ps.detail_message.is_empty() {
                format!("  • {}: {} | {}", ps.pod_name, ps.status, ps.detail_message)
            } else {
                format!("  • {}: {}", ps.pod_name, ps.status)
            };
            lines.push(Line::from(vec![Span::styled(
                sanitize_span_text(&text),
                Style::default().fg(Theme::fg()),
            )]));
        }
    } else if !d.primary_symptoms.is_empty() {
        lines.push(Line::from(vec![
            Span::styled(
                "Symptoms: ",
                Style::default()
                    .fg(Theme::red())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                sanitize_span_text(&d.primary_symptoms.join(" | ")),
                Style::default().fg(Theme::fg()),
            ),
        ]));
    }

    // Line 5: Inline Error Log Snippet (if available) with empty line before and after
    if let Some(ref log_lines) = d.error_log_snippet {
        let first_pod = d
            .pod_symptoms
            .first()
            .map(|ps| ps.pod_name.as_str())
            .or_else(|| d.failing_pod_names.first().map(|s| s.as_str()))
            .unwrap_or("pod");
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            format!("📜 Error Log Snippet ({first_pod}):"),
            Style::default()
                .fg(Theme::yellow())
                .add_modifier(Modifier::BOLD),
        )]));
        for line in log_lines.iter().take(4) {
            lines.push(Line::from(vec![
                Span::styled("   │ ", Style::default().fg(Theme::dim())),
                Span::styled(sanitize_span_text(line), Style::default().fg(Theme::red())),
            ]));
        }
        lines.push(Line::from(""));
    }

    // Line 6+: Correlated Events
    if !d.top_events.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            "Correlated Events: ",
            Theme::header_label(),
        )]));
        for ev in d.top_events.iter().take(2) {
            let ev_style = if ev.type_ == "Warning" {
                Style::default().fg(Theme::yellow())
            } else {
                Style::default().fg(Theme::dim())
            };
            lines.push(Line::from(vec![
                Span::raw("  • "),
                Span::styled(
                    format!("[{}] ", ev.reason),
                    ev_style.add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    sanitize_span_text(&ev.message),
                    Style::default().fg(Theme::fg()),
                ),
                Span::styled(
                    format!(" (x{}, {})", ev.count, ev.age),
                    Style::default().fg(Theme::dim()),
                ),
            ]));
        }
    }

    // Actions Hint
    lines.push(Line::from(vec![
        Span::styled("Actions: ", Theme::header_label()),
        Span::styled(
            "[l] Full Logs   [y] YAML Diff   [d] Describe   [a] AI RCA   [r] Refresh",
            Style::default()
                .fg(Theme::accent())
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    let p = Paragraph::new(lines).block(block).wrap(Wrap { trim: true });
    f.render_widget(p, area);
}

fn render_infra_tab(f: &mut Frame, area: Rect, state: &ChangedViewState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Theme::border()))
        .title(Span::styled(
            " Non-Deployment Cluster Changes & Warnings ",
            Theme::table_header(),
        ));

    let infra = state.filtered_infra();
    if infra.is_empty() {
        let msg = format!(
            "No infrastructure changes or warning events detected in the last {}.",
            state.current_window_label()
        );
        let p = Paragraph::new(msg)
            .style(Style::default().fg(Theme::dim()))
            .block(block)
            .wrap(Wrap { trim: true });
        f.render_widget(p, area);
        return;
    }

    let header_cells = [
        Cell::from(Span::styled("AGE", Theme::table_header())),
        Cell::from(Span::styled("KIND", Theme::table_header())),
        Cell::from(Span::styled("NAMESPACE", Theme::table_header())),
        Cell::from(Span::styled("NAME", Theme::table_header())),
        Cell::from(Span::styled("REASON", Theme::table_header())),
        Cell::from(Span::styled("COUNT", Theme::table_header())),
        Cell::from(Span::styled("MESSAGE", Theme::table_header())),
    ];
    let header = Row::new(header_cells).height(1).bottom_margin(0);

    let rows: Vec<Row> = infra
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let is_selected = i == state.infra_selected_idx;

            let age_cell = Cell::from(Span::styled(&item.age, Style::default().fg(Theme::dim())));
            let kind_cell =
                Cell::from(Span::styled(&item.kind, Style::default().fg(Theme::cyan())));
            let ns_cell = Cell::from(Span::styled(
                &item.namespace,
                Style::default().fg(Theme::dim()),
            ));
            let name_cell = Cell::from(Span::styled(
                &item.name,
                Style::default()
                    .fg(Theme::fg())
                    .add_modifier(Modifier::BOLD),
            ));

            let reason_style = if item.is_warning {
                Style::default()
                    .fg(Theme::yellow())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Theme::dim())
            };
            let reason_cell = Cell::from(Span::styled(&item.reason, reason_style));

            let count_cell = Cell::from(Span::styled(
                format!("{}", item.count),
                Style::default().fg(Theme::dim()),
            ));
            let msg_cell = Cell::from(Span::styled(
                sanitize_span_text(&item.message),
                Style::default().fg(Theme::fg()),
            ));

            let row = Row::new(vec![
                age_cell,
                kind_cell,
                ns_cell,
                name_cell,
                reason_cell,
                count_cell,
                msg_cell,
            ]);

            if is_selected {
                row.style(Theme::selected_row())
            } else {
                row
            }
        })
        .collect();

    let widths = [
        Constraint::Length(8),
        Constraint::Length(14),
        Constraint::Length(14),
        Constraint::Length(22),
        Constraint::Length(16),
        Constraint::Length(6),
        Constraint::Min(30),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .column_spacing(1);

    let mut table_state = TableState::default();
    table_state.select(Some(state.infra_selected_idx));
    f.render_stateful_widget(table, area, &mut table_state);
}

fn render_footer(f: &mut Frame, area: Rect, state: &ChangedViewState) {
    let spans = vec![
        Span::styled(
            "[Enter] ",
            Style::default()
                .fg(Theme::cyan())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("Inspect  ", Style::default().fg(Theme::dim())),
        Span::styled(
            "[l] ",
            Style::default()
                .fg(Theme::cyan())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("Logs  ", Style::default().fg(Theme::dim())),
        Span::styled(
            "[r] ",
            Style::default()
                .fg(Theme::cyan())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("Refresh  ", Style::default().fg(Theme::dim())),
        Span::styled(
            "[y] ",
            Style::default()
                .fg(Theme::cyan())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("YAML  ", Style::default().fg(Theme::dim())),
        Span::styled(
            "[a] ",
            Style::default()
                .fg(Theme::cyan())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("AI RCA  ", Style::default().fg(Theme::dim())),
        Span::styled(
            "[[/]] ",
            Style::default()
                .fg(Theme::accent())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("Window ({})  ", state.current_window_label()),
            Style::default().fg(Theme::dim()),
        ),
        Span::styled(
            "[f] ",
            Style::default()
                .fg(Theme::accent())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("Filter  ", Style::default().fg(Theme::dim())),
        Span::styled(
            "[Tab] ",
            Style::default()
                .fg(Theme::accent())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("Toggle Infra  ", Style::default().fg(Theme::dim())),
        Span::styled(
            "[/] ",
            Style::default()
                .fg(Theme::cyan())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("Search", Style::default().fg(Theme::dim())),
    ];
    let p = Paragraph::new(Line::from(spans)).wrap(Wrap { trim: true });
    f.render_widget(p, area);
}
