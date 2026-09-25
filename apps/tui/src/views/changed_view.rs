use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Cell, Paragraph, Row, Table, TableState, Wrap},
    Frame,
};
use srelens_kube::changed::{
    AppDeploymentChange, ChangeKind, ChangedTriageReport, FailureCategory, IncidentStatus,
    InfraChangeItem, PodIncidentDetail, RolloutStatus,
};
use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::theme::Theme;
use crate::views::sanitize_span_text;

/// The diagnostic card is drawn below the table only when the workspace is
/// at least this tall; below it the footer carries the card's keys instead.
pub const CARD_MIN_HEIGHT: u16 = 26;

/// Symptom groups shown on the card before "+K other symptoms".
const MAX_SYMPTOM_GROUPS: usize = 3;

/// Pod names shown per symptom group before "+N more".
const MAX_SAMPLE_PODS: usize = 2;

/// Failing pods that share one symptom, e.g. 147 pods all
/// "CrashLoopBackOff | exited with code 1".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymptomGroup {
    pub status: String,
    pub detail: String,
    pub count: usize,
    /// Up to [`MAX_SAMPLE_PODS`] pod names, in first-seen order.
    pub sample_pods: Vec<String>,
}

impl SymptomGroup {
    /// One line: the pod itself when it is alone, else the count and a
    /// couple of names.
    pub fn describe(&self) -> String {
        let what = if self.detail.is_empty() {
            self.status.clone()
        } else {
            format!("{} | {}", self.status, self.detail)
        };
        if self.count == 1 {
            let pod = self
                .sample_pods
                .first()
                .map(String::as_str)
                .unwrap_or("pod");
            return format!("{pod}: {what}");
        }
        let mut names = self.sample_pods.join(", ");
        let rest = self.count.saturating_sub(self.sample_pods.len());
        if rest > 0 {
            names.push_str(&format!(", +{rest} more"));
        }
        format!("{what} — {} pods ({names})", self.count)
    }
}

/// Group failing pods by what is wrong with them, keeping first-seen order,
/// so a deployment of 150 identical crash-looping pods reads as one line.
pub fn group_pod_symptoms(symptoms: &[PodIncidentDetail]) -> Vec<SymptomGroup> {
    let mut groups: Vec<SymptomGroup> = Vec::new();
    for ps in symptoms {
        match groups
            .iter_mut()
            .find(|g| g.status == ps.status && g.detail == ps.detail_message)
        {
            Some(g) => {
                g.count += 1;
                if g.sample_pods.len() < MAX_SAMPLE_PODS {
                    g.sample_pods.push(ps.pod_name.clone());
                }
            }
            None => groups.push(SymptomGroup {
                status: ps.status.clone(),
                detail: ps.detail_message.clone(),
                count: 1,
                sample_pods: vec![ps.pod_name.clone()],
            }),
        }
    }
    groups
}

/// The dim marker after a workload's name saying why it is listed, when it
/// is not a rollout. A word, so colour is not the only signal.
pub fn change_tag(kind: ChangeKind) -> Option<&'static str> {
    match kind {
        ChangeKind::Rollout => None,
        ChangeKind::Scaled => Some(" (scaled)"),
        ChangeKind::FailingOnly => Some(" (unchanged)"),
    }
}

/// When the row's change happened. Older reports carry no `changed_age`;
/// fall back to the rollout age they did carry.
fn row_age(d: &AppDeploymentChange) -> &str {
    if d.changed_age.is_empty() {
        &d.deployed_age
    } else {
        &d.changed_age
    }
}

/// A column exactly as wide as its longest value, and never narrower than
/// its header, so no value is cut ("external-secrets" in a 14-wide column
/// read "external-secre"). The flexible ROOT CAUSE / MESSAGE column takes
/// what is left.
fn column_width(header: &str, values: impl Iterator<Item = usize>) -> u16 {
    values
        .max()
        .unwrap_or(0)
        .max(header.chars().count())
        .min(u16::MAX as usize) as u16
}

/// Push one blank line unless the card already ends with one, so sections
/// are spaced once whichever of them are present.
fn push_gap(lines: &mut Vec<Line<'_>>) {
    if lines.last().is_some_and(|l| l.width() > 0) {
        lines.push(Line::from(""));
    }
}

pub const WINDOWS: &[(&str, Duration)] = &[
    ("15m", Duration::from_secs(900)),
    ("30m", Duration::from_secs(1800)),
    ("1h", Duration::from_secs(3600)),
    ("3h", Duration::from_secs(10800)),
    ("24h", Duration::from_secs(86400)),
];

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

/// Where one Quick AI RCA request stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuickRcaStatus {
    Loading,
    Ready {
        root_cause: String,
        /// Empty when the model ignored the two-line format; the whole
        /// reply is then the root cause.
        action_item: String,
    },
    Error(String),
}

/// A Quick AI RCA for one workload revision, as shown on its card.
#[derive(Debug, Clone)]
pub struct QuickRca {
    /// The pod whose logs were sent, when there were any.
    pub pod_name: Option<String>,
    /// Which provider answered, shown so the reader knows where the
    /// workload's logs went.
    pub provider: String,
    pub status: QuickRcaStatus,
    pub updated_at: Instant,
}

pub struct ChangedViewState {
    pub report: Option<ChangedTriageReport>,
    /// The context `report` was fetched from. The view outlives a context
    /// switch, and regional clusters run workloads of the same name, so the
    /// RCA cache is keyed by it.
    pub context: String,
    /// Quick AI RCA results by [`ChangedViewState::rca_key`]. Kept across
    /// refreshes and navigation; a new revision gets a new key, so an answer
    /// about the previous release is never shown as current.
    pub ai_summaries: HashMap<String, QuickRca>,
    pub selected_idx: usize,
    pub infra_selected_idx: usize,
    pub active_tab: ChangedTab,
    pub is_loading: bool,
    pub error: Option<String>,
    pub filter_query: String,
    pub window_idx: usize,
    pub incident_filter: IncidentFilter,
    /// Also list workloads that did not change in the window but are failing
    /// in it (`u`). Off by default: the window means "changed".
    pub include_failing: bool,
    /// Also list Deployments whose only change is a replica count (`S`).
    /// Off by default: an autoscaled cluster scales something every hour.
    pub include_scaled: bool,
}

impl ChangedViewState {
    pub fn new() -> Self {
        Self {
            report: None,
            context: String::new(),
            ai_summaries: HashMap::new(),
            selected_idx: 0,
            infra_selected_idx: 0,
            active_tab: ChangedTab::Deployments,
            is_loading: true,
            error: None,
            filter_query: String::new(),
            window_idx: 2, // Default to 1h
            incident_filter: IncidentFilter::All,
            include_failing: false,
            include_scaled: false,
        }
    }

    /// Cache key for a workload's Quick AI RCA: cluster, namespace, kind,
    /// name and revision.
    pub fn rca_key(&self, d: &AppDeploymentChange) -> String {
        format!(
            "{}|{}/{}/{}@{}",
            self.context, d.namespace, d.kind, d.app_name, d.current_revision
        )
    }

    pub fn rca_for(&self, d: &AppDeploymentChange) -> Option<&QuickRca> {
        self.ai_summaries.get(&self.rca_key(d))
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

    pub fn toggle_include_scaled(&mut self) {
        self.include_scaled = !self.include_scaled;
        self.selected_idx = 0;
        self.is_loading = true;
    }

    /// The banner's scope label: what the list holds besides rollouts.
    pub fn scope_label(&self) -> &'static str {
        match (self.include_scaled, self.include_failing) {
            (false, false) => "[CHANGED]",
            (true, false) => "[CHANGED + SCALED]",
            (false, true) => "[CHANGED + FAILING]",
            (true, true) => "[CHANGED + SCALED + FAILING]",
        }
    }

    pub fn toggle_include_failing(&mut self) {
        self.include_failing = !self.include_failing;
        self.selected_idx = 0;
        self.is_loading = true;
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
    let (banner_block, health, controls) = summary_banner(state);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(banner_height(&health, &controls, area.width)),
            Constraint::Min(8),    // Main workspace
            Constraint::Length(1), // Footer shortcuts
        ])
        .split(area);

    render_summary_banner(f, chunks[0], banner_block, health, controls);

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
        render_footer(f, chunks[2], state, false);
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
        render_footer(f, chunks[2], state, false);
        return;
    }

    match state.active_tab {
        ChangedTab::Deployments => render_deployments_tab(f, chunks[1], state),
        ChangedTab::Infra => render_infra_tab(f, chunks[1], state),
    }

    let card_visible =
        state.active_tab == ChangedTab::Deployments && chunks[1].height >= CARD_MIN_HEIGHT;
    render_footer(f, chunks[2], state, card_visible);
}

/// The banner's block, its health badges, and its controls (window, filter,
/// scope, infra). They share one line when it fits and take two when not, so
/// the controls are never clipped off a narrow terminal.
fn summary_banner(state: &ChangedViewState) -> (Block<'static>, Line<'static>, Line<'static>) {
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
    ]);

    let controls = Line::from(vec![
        Span::styled(" Window: ", Theme::header_label()),
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
        Span::styled("Scope: ", Theme::header_label()),
        Span::styled(
            state.scope_label(),
            Style::default()
                .fg(Theme::accent())
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

    (block, line1, controls)
}

/// Rows the banner needs at `width`: one content line, or two.
fn banner_height(health: &Line, controls: &Line, width: u16) -> u16 {
    let inner = usize::from(width.saturating_sub(2));
    if health.width() + 2 + controls.width() <= inner {
        3
    } else {
        4
    }
}

fn render_summary_banner(
    f: &mut Frame,
    area: Rect,
    block: Block<'static>,
    health: Line<'static>,
    controls: Line<'static>,
) {
    let lines = if area.height >= 4 {
        vec![health, controls]
    } else {
        let mut spans = health.spans;
        spans.push(Span::raw("  "));
        spans.extend(controls.spans);
        vec![Line::from(spans)]
    };
    let p = Paragraph::new(lines).block(block);
    f.render_widget(p, area);
}

fn render_deployments_tab(f: &mut Frame, area: Rect, state: &ChangedViewState) {
    if area.height >= CARD_MIN_HEIGHT {
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
            match (state.include_scaled, state.include_failing) {
                (false, false) => " Workloads Changed in Window ",
                (true, false) => " Workloads Changed or Scaled in Window ",
                (false, true) => " Workloads Changed or Failing in Window ",
                (true, true) => " Workloads Changed, Scaled or Failing in Window ",
            },
            Theme::table_header(),
        ));

    if deps.is_empty() {
        let msg = if !state.filter_query.is_empty() {
            format!(
                "No workloads matching query '{}'. Press / to change search or Esc to clear.",
                state.filter_query
            )
        } else if state.incident_filter != IncidentFilter::All {
            // Workloads may be there; the filter hides them. Say which.
            format!(
                "No workloads in the window match the {} filter. Press f to cycle it back to ALL.",
                state.incident_filter.label()
            )
        } else {
            let mut msg = format!(
                "No workloads changed{}{} within the last {}. Use [ or ] to broaden the time window",
                if state.include_scaled { ", scaled" } else { "" },
                if state.include_failing { " or failing" } else { "" },
                state.current_window_label()
            );
            let mut more = Vec::new();
            if !state.include_scaled {
                more.push("S to include scaled workloads");
            }
            if !state.include_failing {
                more.push("u to include workloads failing without a change");
            }
            if !more.is_empty() {
                msg.push_str(", or ");
                msg.push_str(&more.join(", or "));
            }
            msg.push('.');
            msg
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
        Cell::from(Span::styled("CHANGED", Theme::table_header())),
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
            let mut name_spans = vec![Span::styled(
                name_display,
                Style::default()
                    .fg(Theme::fg())
                    .add_modifier(Modifier::BOLD),
            )];
            if let Some(tag) = change_tag(d.change_kind) {
                name_spans.push(Span::styled(tag, Style::default().fg(Theme::dim())));
            }
            let name_cell = Cell::from(Line::from(name_spans));
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

            let age_cell = Cell::from(Span::styled(row_age(d), Style::default().fg(Theme::dim())));

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

    // Wide enough for the longest name and its "(sts)"/"(unchanged)"
    // markers, within reason; the ROOT CAUSE column takes the rest.
    let workload_width = deps.iter().map(|d| {
        let kind_tag = match d.kind.as_str() {
            "StatefulSet" | "CronJob" => 5,
            "Job" => 6,
            _ => 0,
        };
        let change = change_tag(d.change_kind).map_or(0, |t| t.chars().count());
        d.app_name.chars().count() + kind_tag + change
    });
    let widths = [
        Constraint::Length(12),
        Constraint::Length(column_width("WORKLOAD", workload_width)),
        Constraint::Length(column_width(
            "NAMESPACE",
            deps.iter().map(|d| d.namespace.chars().count()),
        )),
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
            "Select a workload above to inspect its root cause, GitOps release, and symptoms.",
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
                if g.sync_age == "-" {
                    "never".to_string()
                } else {
                    format!("{} ago", sanitize_span_text(&g.sync_age))
                },
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

    // Why a non-rollout row is listed, with the rollout age for contrast.
    match d.change_kind {
        ChangeKind::Rollout => {}
        ChangeKind::Scaled => lines.push(Line::from(Span::styled(
            format!(
                "{} {} ago; last rollout {} ago (S to hide).",
                d.change_detail.as_deref().unwrap_or("Scaled"),
                row_age(d),
                d.deployed_age
            ),
            Style::default().fg(Theme::dim()),
        ))),
        ChangeKind::FailingOnly => lines.push(Line::from(Span::styled(
            format!(
                "Not changed in the last {}; shown because it is failing now (u to hide).",
                state.current_window_label()
            ),
            Style::default().fg(Theme::dim()),
        ))),
    }

    push_gap(&mut lines);

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

    push_gap(&mut lines);

    // Symptoms, grouped: one line per distinct failure, not one per pod.
    // The full logs are one key away (`l`), so the card carries none.
    if !d.pod_symptoms.is_empty() {
        let groups = group_pod_symptoms(&d.pod_symptoms);
        let n = d.pod_symptoms.len();
        lines.push(Line::from(vec![Span::styled(
            format!(
                "Symptoms ({n} pod{} failing):",
                if n == 1 { "" } else { "s" }
            ),
            Style::default()
                .fg(Theme::red())
                .add_modifier(Modifier::BOLD),
        )]));
        for g in groups.iter().take(MAX_SYMPTOM_GROUPS) {
            lines.push(Line::from(vec![Span::styled(
                sanitize_span_text(&format!("  • {}", g.describe())),
                Style::default().fg(Theme::fg()),
            )]));
        }
        let hidden = groups.len().saturating_sub(MAX_SYMPTOM_GROUPS);
        if hidden > 0 {
            lines.push(Line::from(Span::styled(
                format!(
                    "  +{hidden} other symptom{}",
                    if hidden == 1 { "" } else { "s" }
                ),
                Style::default().fg(Theme::dim()),
            )));
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
    push_gap(&mut lines);

    // Quick AI RCA (on demand, `s`)
    if let Some(rca) = state.rca_for(d) {
        let mut source = rca
            .pod_name
            .clone()
            .unwrap_or_else(|| "no logs".to_string());
        source.push_str(", ");
        source.push_str(&rca.provider);
        let suffix = match rca.status {
            QuickRcaStatus::Ready { .. } => format!(
                " [cached {} ago]",
                srelens_kube::format_age(rca.updated_at.elapsed().as_secs() as i64)
            ),
            _ => String::new(),
        };
        lines.push(Line::from(vec![Span::styled(
            format!("🤖 Quick AI RCA ({}){suffix}:", sanitize_span_text(&source)),
            Style::default()
                .fg(Theme::cyan())
                .add_modifier(Modifier::BOLD),
        )]));
        push_gap(&mut lines);
        match &rca.status {
            QuickRcaStatus::Loading => lines.push(Line::from(Span::styled(
                "   ⚡ Analyzing termination state, events, and error logs...",
                Style::default().fg(Theme::dim()),
            ))),
            QuickRcaStatus::Ready {
                root_cause,
                action_item,
            } => {
                lines.push(Line::from(vec![
                    Span::styled("   • Root Cause: ", Theme::header_label()),
                    Span::styled(
                        sanitize_span_text(root_cause),
                        Style::default().fg(Theme::fg()),
                    ),
                ]));
                if !action_item.is_empty() {
                    lines.push(Line::from(vec![
                        Span::styled("   • Action Item: ", Theme::header_label()),
                        Span::styled(
                            sanitize_span_text(action_item),
                            Style::default().fg(Theme::green()),
                        ),
                    ]));
                }
            }
            QuickRcaStatus::Error(msg) => lines.push(Line::from(Span::styled(
                format!("   ⚠️ {}", sanitize_span_text(msg)),
                Style::default()
                    .fg(Theme::yellow())
                    .add_modifier(Modifier::BOLD),
            ))),
        }
        push_gap(&mut lines);
    }

    // Actions Hint
    lines.push(Line::from(vec![
        Span::styled("Actions: ", Theme::header_label()),
        Span::styled(
            "[Enter/d] Describe   [l] Logs   [y] YAML   [s] Quick AI RCA   [a] Assistant   [r] Refresh",
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
        Constraint::Length(column_width(
            "NAMESPACE",
            infra.iter().map(|i| i.namespace.chars().count()),
        )),
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

/// The footer carries only keys not already on screen. The card's Actions
/// line lists the per-workload keys, so they appear here only when there is
/// no card: on the Infra tab, or when the pane is too short to draw one.
/// Cmd, Help and Back live in the status bar below.
fn render_footer(f: &mut Frame, area: Rect, state: &ChangedViewState, card_visible: bool) {
    let mut keys: Vec<(String, String)> = Vec::new();
    if !card_visible {
        keys.push(("[Enter]".into(), "Describe".into()));
        keys.push(("[y]".into(), "YAML".into()));
        if state.active_tab == ChangedTab::Deployments {
            keys.push(("[l]".into(), "Logs".into()));
            keys.push(("[s]".into(), "Quick RCA".into()));
            keys.push(("[a]".into(), "Assistant".into()));
        }
        keys.push(("[r]".into(), "Refresh".into()));
    }
    keys.push((
        "[[/]]".into(),
        format!("Window ({})", state.current_window_label()),
    ));
    keys.push(("[f]".into(), "Filter".into()));
    keys.push((
        "[u]".into(),
        if state.include_failing {
            "Hide unchanged".into()
        } else {
            "Include failing".into()
        },
    ));
    keys.push((
        "[S]".into(),
        if state.include_scaled {
            "Hide scaled".into()
        } else {
            "Include scaled".into()
        },
    ));
    keys.push(("[Tab]".into(), "Toggle Infra".into()));
    keys.push(("[/]".into(), "Search".into()));

    let mut spans = Vec::new();
    for (key, label) in keys {
        spans.push(Span::styled(
            format!("{key} "),
            Style::default()
                .fg(Theme::accent())
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            format!("{label}  "),
            Style::default().fg(Theme::dim()),
        ));
    }
    let p = Paragraph::new(Line::from(spans)).wrap(Wrap { trim: true });
    f.render_widget(p, area);
}
