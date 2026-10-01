use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Cell, Paragraph, Row, Table, TableState, Wrap},
    Frame,
};
use srelens_kube::changed::{
    AppDeploymentChange, ArgoCoverage, ArgoCoverageState, ChangeKind, ChangedTriageReport,
    FailureCategory, IncidentStatus, InfraChangeItem, PodIncidentDetail, RolloutStatus,
};
use srelens_registry::github::RolloutCause;
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
pub fn change_tag(_kind: ChangeKind) -> Option<&'static str> {
    None
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

/// The formatted description of the most recent change event in the window,
/// e.g. "📦 Rollout (12m)", "📈 Scaled 14→18 (47s)", "⚠️ Failing (2m)".
pub fn recent_change_str(d: &AppDeploymentChange) -> String {
    let age = if d.changed_age.is_empty() {
        &d.deployed_age
    } else {
        &d.changed_age
    };
    match d.change_kind {
        ChangeKind::Rollout => {
            if age.is_empty() || age == "-" {
                "📦 Rollout".to_string()
            } else {
                format!("📦 Rollout ({age})")
            }
        }
        ChangeKind::Scaled => {
            if let Some(ref detail) = d.change_detail {
                if age.is_empty() || age == "-" {
                    format!("📈 {detail}")
                } else {
                    format!("📈 {detail} ({age})")
                }
            } else if age.is_empty() || age == "-" {
                "📈 Scaled".to_string()
            } else {
                format!("📈 Scaled ({age})")
            }
        }
        ChangeKind::FailingOnly => {
            if d.incident_status == IncidentStatus::Healthy {
                "-".to_string()
            } else if age.is_empty() || age == "-" {
                "⚠️ Failing".to_string()
            } else {
                format!("⚠️ Failing ({age})")
            }
        }
    }
}

/// A column exactly as wide as its longest value, and never narrower than
/// its header, so no value is cut ("external-secrets" in a 14-wide column
/// read "external-secre"). The flexible ROOT CAUSE / MESSAGE column takes
/// what is left.
fn column_width(header: &str, values: impl Iterator<Item = usize>) -> u16 {
    let header_width = unicode_width::UnicodeWidthStr::width(header);
    values
        .max()
        .unwrap_or(0)
        .max(header_width)
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
pub enum TriageBandFocus {
    Incidents,
    Timeline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IncidentFilter {
    All,
    CrashingOnly,
    OomOnly,
    ErrorOnly,
    PendingOnly,
    ProbeOnly,
    RollingOnly,
}

impl IncidentFilter {
    pub fn label(&self) -> &'static str {
        match self {
            Self::All => "ALL",
            Self::CrashingOnly => "CRASH",
            Self::OomOnly => "OOM",
            Self::ErrorOnly => "ERROR",
            Self::PendingOnly => "PENDING (INFRA)",
            Self::ProbeOnly => "PROBE",
            Self::RollingOnly => "ROLLING",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            Self::All => Self::CrashingOnly,
            Self::CrashingOnly => Self::OomOnly,
            Self::OomOnly => Self::ErrorOnly,
            Self::ErrorOnly => Self::PendingOnly,
            Self::PendingOnly => Self::ProbeOnly,
            Self::ProbeOnly => Self::RollingOnly,
            Self::RollingOnly => Self::All,
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

/// Where one GitHub lookup of a rollout's cause stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CauseLookup {
    Loading,
    Ready(RolloutCause),
    Failed(String),
}

/// What to ask GitHub about one row's Argo sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CauseAsk {
    /// Repo, both revisions and path: an answer about fixed commits, so it
    /// is valid for as long as the view lives.
    pub key: String,
    pub repo_url: String,
    pub revision: String,
    pub previous: Option<String>,
    pub path: String,
}

/// The GitHub question a row's Argo sync poses. `None` when the row names
/// no sync, so there is nothing to ask; `Err` when there is one but GitHub
/// cannot explain it, with the reason to show instead.
pub fn cause_ask(d: &AppDeploymentChange) -> Option<Result<CauseAsk, String>> {
    let r = d.gitops.as_ref()?.rollout.as_ref()?;
    if r.is_chart {
        return Some(Err(format!(
            "chart source, chart version {}: no git history to look up",
            r.revision
        )));
    }
    if srelens_registry::github::parse_github_repo(&r.repo_url).is_none() {
        return Some(Err(format!("not a github.com repository ({})", r.repo_url)));
    }
    Some(Ok(CauseAsk {
        key: format!(
            "{}|{}|{}|{}",
            r.repo_url,
            r.previous_revision.as_deref().unwrap_or(""),
            r.revision,
            r.path
        ),
        repo_url: r.repo_url.clone(),
        revision: r.revision.clone(),
        previous: r.previous_revision.clone(),
        path: r.path.clone(),
    }))
}

pub struct ChangedViewState {
    pub report: Option<ChangedTriageReport>,
    /// GitHub answers by [`CauseAsk::key`], fetched for the selected row
    /// only. Failed ones are dropped on refresh, so `r` retries them.
    pub causes: HashMap<String, CauseLookup>,
    /// The context `report` was fetched from. The view outlives a context
    /// switch, and regional clusters run workloads of the same name, so the
    /// RCA cache is keyed by it.
    pub context: String,
    /// Quick AI RCA results by [`ChangedViewState::rca_key`]. Kept across
    /// refreshes and navigation; a new revision gets a new key, so an answer
    /// about the previous release is never shown as current.
    pub ai_summaries: HashMap<String, QuickRca>,
    pub selected_idx: usize,
    pub band_focus: TriageBandFocus,
    pub incident_selected_idx: usize,
    pub timeline_selected_idx: usize,
    pub show_healthy_in_timeline: bool,
    pub infra_selected_idx: usize,
    pub active_tab: ChangedTab,
    pub is_loading: bool,
    pub error: Option<String>,
    pub filter_query: String,
    pub window_idx: usize,
    pub incident_filter: IncidentFilter,
    /// Also list workloads that did not change in the window but are failing
    /// in it (`u`). On by default: active failures are never hidden.
    pub include_failing: bool,
    /// Also list Deployments whose only change is a replica count (`S`).
    /// On by default: scaling surges are included in triage.
    pub include_scaled: bool,
    /// Whether to display the SRE scope and triage guide banner (`b` / `?`).
    pub show_guide_banner: bool,
    /// Whether an asynchronous background or manual triage refresh is in flight.
    pub is_refreshing: bool,
    /// Timestamp of when the triage report was last successfully updated.
    pub last_refreshed_at: Option<std::time::Instant>,
}

impl ChangedViewState {
    pub fn new() -> Self {
        Self {
            report: None,
            causes: HashMap::new(),
            context: String::new(),
            ai_summaries: HashMap::new(),
            selected_idx: 0,
            band_focus: TriageBandFocus::Incidents,
            incident_selected_idx: 0,
            timeline_selected_idx: 0,
            show_healthy_in_timeline: true,
            infra_selected_idx: 0,
            active_tab: ChangedTab::Deployments,
            is_loading: true,
            error: None,
            filter_query: String::new(),
            window_idx: 2, // Default to 1h
            incident_filter: IncidentFilter::All,
            include_failing: true,
            include_scaled: false,
            show_guide_banner: true,
            is_refreshing: false,
            last_refreshed_at: None,
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
        self.incident_selected_idx = 0;
        self.timeline_selected_idx = 0;
        self.band_focus = TriageBandFocus::Incidents;
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
        self.incident_selected_idx = 0;
        self.timeline_selected_idx = 0;
        self.band_focus = TriageBandFocus::Incidents;
        self.infra_selected_idx = 0;
        self.is_loading = true;
    }

    pub fn set_window_by_str(&mut self, s: &str) {
        let clean = s.trim().to_ascii_lowercase();
        for (i, (lbl, _)) in WINDOWS.iter().enumerate() {
            if *lbl == clean || clean.starts_with(lbl) {
                self.window_idx = i;
                self.selected_idx = 0;
                self.incident_selected_idx = 0;
                self.timeline_selected_idx = 0;
                self.band_focus = TriageBandFocus::Incidents;
                self.infra_selected_idx = 0;
                self.is_loading = true;
                return;
            }
        }
    }

    pub fn cycle_filter(&mut self) {
        self.incident_filter = self.incident_filter.next();
        self.selected_idx = 0;
        self.incident_selected_idx = 0;
        self.timeline_selected_idx = 0;
        self.band_focus = TriageBandFocus::Incidents;
    }

    pub fn toggle_include_scaled(&mut self) {
        self.include_scaled = !self.include_scaled;
        self.selected_idx = 0;
        self.incident_selected_idx = 0;
        self.timeline_selected_idx = 0;
        self.band_focus = TriageBandFocus::Incidents;
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
        self.incident_selected_idx = 0;
        self.timeline_selected_idx = 0;
        self.band_focus = TriageBandFocus::Incidents;
        self.is_loading = true;
    }

    pub fn toggle_tab(&mut self) {
        self.active_tab = match self.active_tab {
            ChangedTab::Deployments => ChangedTab::Infra,
            ChangedTab::Infra => ChangedTab::Deployments,
        };
    }

    pub fn clamp_selection(&mut self) {
        let inc_count = self.filtered_incidents().len();
        if self.incident_selected_idx >= inc_count {
            self.incident_selected_idx = inc_count.saturating_sub(1);
        }
        let tl_count = self.filtered_timeline_deployments().len();
        if self.timeline_selected_idx >= tl_count {
            self.timeline_selected_idx = tl_count.saturating_sub(1);
        }
        let dep_count = self.filtered_deployments().len();
        if self.selected_idx >= dep_count {
            self.selected_idx = dep_count.saturating_sub(1);
        }
        self.sync_band_from_selected_idx();

        let infra_count = self.filtered_infra().len();
        if self.infra_selected_idx >= infra_count {
            self.infra_selected_idx = infra_count.saturating_sub(1);
        }
    }

    pub fn set_report(&mut self, report: ChangedTriageReport) {
        self.report = Some(report);
        self.is_loading = false;
        self.error = None;
        self.causes
            .retain(|_, c| !matches!(c, CauseLookup::Failed(_)));
        self.clamp_selection();
    }

    /// What `g` opens for the selected row: its pull request (a person's
    /// before a bot's), else a commit that reached the branch without one,
    /// else the sync's compare view, with the words for the toast. `Err`
    /// says why there is nothing to open.
    pub fn cause_link(&self) -> Result<(String, String), String> {
        let d = self
            .selected_deployment()
            .ok_or_else(|| "Select a workload first".to_string())?;
        self.cause_link_for(d)
    }

    pub fn cause_link_for(&self, d: &AppDeploymentChange) -> Result<(String, String), String> {
        let ask = match cause_ask(d) {
            Some(Ok(ask)) => ask,
            Some(Err(reason)) => return Err(reason),
            None => return Err(format!("No Argo sync to trace for {}", d.app_name)),
        };
        let c = match self.causes.get(&ask.key) {
            Some(CauseLookup::Ready(c)) => c,
            Some(CauseLookup::Failed(e)) => return Err(format!("GitHub: {e}")),
            _ => return Err("Still looking up the pull requests on GitHub".to_string()),
        };
        let pr = c.pulls.iter().find(|p| !p.is_bot).or(c.pulls.first());
        if let Some(pr) = pr.filter(|p| !p.html_url.is_empty()) {
            let more = match c.pulls.len() {
                1 => String::new(),
                n => format!(" (1 of {n} in this sync)"),
            };
            return Ok((pr.html_url.clone(), format!("PR #{}{more}", pr.number)));
        }
        let direct = c
            .commits
            .iter()
            .find(|cm| c.direct_commits.contains(&cm.sha) && !cm.html_url.is_empty());
        if let Some(cm) = direct {
            return Ok((
                cm.html_url.clone(),
                format!("commit {}", short_sha(&cm.sha)),
            ));
        }
        match &c.compare_url {
            Some(url) => Ok((url.clone(), "the sync's compare view".to_string())),
            None => Err("GitHub named no pull request for this sync".to_string()),
        }
    }

    /// The GitHub question the selected Deployments-tab row poses and has
    /// not asked yet.
    pub fn next_cause_ask(&self) -> Option<CauseAsk> {
        if self.active_tab != ChangedTab::Deployments {
            return None;
        }
        let ask = cause_ask(self.selected_deployment()?)?.ok()?;
        (!self.causes.contains_key(&ask.key)).then_some(ask)
    }

    pub fn set_error(&mut self, err: String) {
        self.error = Some(err);
        self.is_loading = false;
    }

    pub fn matches_query(&self, d: &AppDeploymentChange, q: &str) -> bool {
        if q.is_empty() {
            return true;
        }
        d.app_name.to_ascii_lowercase().contains(q)
            || d.namespace.to_ascii_lowercase().contains(q)
            || d.image_diff.to_ascii_lowercase().contains(q)
            || d.failure_detail.to_ascii_lowercase().contains(q)
            || d.current_images
                .iter()
                .any(|img| img.to_ascii_lowercase().contains(q))
            || d.gitops
                .as_ref()
                .map(|g| {
                    g.app_name.to_ascii_lowercase().contains(q)
                        || g.sync_revision.to_ascii_lowercase().contains(q)
                        || g.target_revision.to_ascii_lowercase().contains(q)
                })
                .unwrap_or(false)
    }

    pub fn filtered_incidents(&self) -> Vec<&AppDeploymentChange> {
        let Some(report) = &self.report else {
            return Vec::new();
        };
        let q = self.filter_query.trim().to_ascii_lowercase();
        report
            .deployments
            .iter()
            .filter(|d| d.is_incident())
            .filter(|d| match self.incident_filter {
                IncidentFilter::All => true,
                IncidentFilter::CrashingOnly => d.incident_status == IncidentStatus::CrashLoop,
                IncidentFilter::OomOnly => d.incident_status == IncidentStatus::OomKilled,
                IncidentFilter::ErrorOnly => {
                    d.incident_status == IncidentStatus::ConfigError
                        || d.incident_status == IncidentStatus::ImageError
                }
                IncidentFilter::PendingOnly => d.incident_status == IncidentStatus::Pending,
                IncidentFilter::ProbeOnly => d.incident_status == IncidentStatus::ProbeFailure,
                IncidentFilter::RollingOnly => {
                    d.incident_status == IncidentStatus::Rolling
                        || d.incident_status == IncidentStatus::Stalled
                }
            })
            .filter(|d| self.matches_query(d, &q))
            .collect()
    }

    pub fn filtered_timeline_deployments(&self) -> Vec<&AppDeploymentChange> {
        let Some(report) = &self.report else {
            return Vec::new();
        };
        let q = self.filter_query.trim().to_ascii_lowercase();
        report
            .deployments
            .iter()
            .filter(|d| d.change_kind == ChangeKind::Rollout)
            .filter(|d| match self.incident_filter {
                IncidentFilter::All => true,
                IncidentFilter::CrashingOnly => d.incident_status == IncidentStatus::CrashLoop,
                IncidentFilter::OomOnly => d.incident_status == IncidentStatus::OomKilled,
                IncidentFilter::ErrorOnly => {
                    d.incident_status == IncidentStatus::ConfigError
                        || d.incident_status == IncidentStatus::ImageError
                }
                IncidentFilter::PendingOnly => d.incident_status == IncidentStatus::Pending,
                IncidentFilter::ProbeOnly => d.incident_status == IncidentStatus::ProbeFailure,
                IncidentFilter::RollingOnly => {
                    d.incident_status == IncidentStatus::Rolling
                        || d.incident_status == IncidentStatus::Stalled
                }
            })
            .filter(|d| self.matches_query(d, &q))
            .collect()
    }

    pub fn effective_band_focus(&self) -> TriageBandFocus {
        if self.filtered_incidents().is_empty() {
            TriageBandFocus::Timeline
        } else if self.filtered_timeline_deployments().is_empty() {
            TriageBandFocus::Incidents
        } else {
            self.band_focus
        }
    }

    pub fn toggle_band_focus(&mut self) {
        if self.filtered_incidents().is_empty() {
            self.band_focus = TriageBandFocus::Timeline;
            self.sync_selected_idx();
            return;
        }
        self.band_focus = match self.effective_band_focus() {
            TriageBandFocus::Incidents => TriageBandFocus::Timeline,
            TriageBandFocus::Timeline => TriageBandFocus::Incidents,
        };
        self.sync_selected_idx();
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
                IncidentFilter::ProbeOnly => d.incident_status == IncidentStatus::ProbeFailure,
                IncidentFilter::RollingOnly => {
                    d.incident_status == IncidentStatus::Rolling
                        || d.incident_status == IncidentStatus::Stalled
                }
            })
            .filter(|d| self.matches_query(d, &q))
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
        let all = self.filtered_deployments();
        if let Some(d) = all.get(self.selected_idx).copied() {
            return Some(d);
        }
        match self.effective_band_focus() {
            TriageBandFocus::Incidents => {
                let inc = self.filtered_incidents();
                if let Some(d) = inc.get(self.incident_selected_idx).copied() {
                    return Some(d);
                }
                self.filtered_timeline_deployments()
                    .get(self.timeline_selected_idx)
                    .copied()
            }
            TriageBandFocus::Timeline => {
                let tl = self.filtered_timeline_deployments();
                if let Some(d) = tl.get(self.timeline_selected_idx).copied() {
                    return Some(d);
                }
                self.filtered_incidents()
                    .get(self.incident_selected_idx)
                    .copied()
            }
        }
    }

    pub fn sync_selected_idx(&mut self) {
        let target_opt = match self.effective_band_focus() {
            TriageBandFocus::Incidents => self
                .filtered_incidents()
                .get(self.incident_selected_idx)
                .map(|d| (d.app_name.clone(), d.namespace.clone(), d.kind.clone())),
            TriageBandFocus::Timeline => self
                .filtered_timeline_deployments()
                .get(self.timeline_selected_idx)
                .map(|d| (d.app_name.clone(), d.namespace.clone(), d.kind.clone())),
        };

        if let Some((app_name, namespace, kind)) = target_opt {
            let all = self.filtered_deployments();
            if let Some(pos) = all
                .iter()
                .position(|d| d.app_name == app_name && d.namespace == namespace && d.kind == kind)
            {
                self.selected_idx = pos;
            }
        }
    }

    pub fn sync_band_from_selected_idx(&mut self) {
        let (is_incident, app_name, namespace, kind) =
            match self.filtered_deployments().get(self.selected_idx) {
                Some(target) => (
                    target.is_incident(),
                    target.app_name.clone(),
                    target.namespace.clone(),
                    target.kind.clone(),
                ),
                None => return,
            };

        if is_incident && !self.filtered_incidents().is_empty() {
            self.band_focus = TriageBandFocus::Incidents;
            if let Some(pos) = self
                .filtered_incidents()
                .iter()
                .position(|d| d.app_name == app_name && d.namespace == namespace && d.kind == kind)
            {
                self.incident_selected_idx = pos;
            }
        } else {
            self.band_focus = TriageBandFocus::Timeline;
            if let Some(pos) = self
                .filtered_timeline_deployments()
                .iter()
                .position(|d| d.app_name == app_name && d.namespace == namespace && d.kind == kind)
            {
                self.timeline_selected_idx = pos;
            }
        }
    }

    pub fn selected_infra(&self) -> Option<&InfraChangeItem> {
        let infra = self.filtered_infra();
        infra.get(self.infra_selected_idx).copied()
    }

    pub fn select_next(&mut self) {
        match self.active_tab {
            ChangedTab::Deployments => {
                match self.effective_band_focus() {
                    TriageBandFocus::Incidents => {
                        let inc_count = self.filtered_incidents().len();
                        if self.incident_selected_idx + 1 < inc_count {
                            self.incident_selected_idx += 1;
                        } else if !self.filtered_timeline_deployments().is_empty() {
                            self.band_focus = TriageBandFocus::Timeline;
                            self.timeline_selected_idx = 0;
                        }
                    }
                    TriageBandFocus::Timeline => {
                        let tl_count = self.filtered_timeline_deployments().len();
                        if tl_count > 0 && self.timeline_selected_idx + 1 < tl_count {
                            self.timeline_selected_idx += 1;
                        }
                    }
                }
                self.sync_selected_idx();
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
                match self.effective_band_focus() {
                    TriageBandFocus::Incidents => {
                        if self.incident_selected_idx > 0 {
                            self.incident_selected_idx -= 1;
                        }
                    }
                    TriageBandFocus::Timeline => {
                        if self.timeline_selected_idx > 0 {
                            self.timeline_selected_idx -= 1;
                        } else if !self.filtered_incidents().is_empty() {
                            self.band_focus = TriageBandFocus::Incidents;
                            self.incident_selected_idx =
                                self.filtered_incidents().len().saturating_sub(1);
                        }
                    }
                }
                self.sync_selected_idx();
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
            ChangedTab::Deployments => {
                if !self.filtered_incidents().is_empty() {
                    self.band_focus = TriageBandFocus::Incidents;
                    self.incident_selected_idx = 0;
                } else {
                    self.band_focus = TriageBandFocus::Timeline;
                    self.timeline_selected_idx = 0;
                }
                self.sync_selected_idx();
            }
            ChangedTab::Infra => self.infra_selected_idx = 0,
        }
    }

    pub fn select_last(&mut self) {
        match self.active_tab {
            ChangedTab::Deployments => {
                if !self.filtered_timeline_deployments().is_empty() {
                    self.band_focus = TriageBandFocus::Timeline;
                    self.timeline_selected_idx =
                        self.filtered_timeline_deployments().len().saturating_sub(1);
                } else if !self.filtered_incidents().is_empty() {
                    self.band_focus = TriageBandFocus::Incidents;
                    self.incident_selected_idx = self.filtered_incidents().len().saturating_sub(1);
                }
                self.sync_selected_idx();
            }
            ChangedTab::Infra => {
                let count = self.filtered_infra().len();
                self.infra_selected_idx = count.saturating_sub(1);
            }
        }
    }
}

/// SRE Scope & Triage Guide banner explaining [CHANGED], [S] Scaled, [u] Failing, and [Tab] Infra.
pub fn render_guide_banner(f: &mut Frame, area: Rect, state: &ChangedViewState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(Theme::border_type())
        .border_style(Style::default().fg(Theme::border()))
        .title(Span::styled(
            " 💡 SRE Scope & Triage Guide [b/? to hide] ",
            Style::default()
                .fg(Theme::accent())
                .add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let line1 = Line::from(vec![
        Span::styled(
            " [t] Focus: ",
            Style::default()
                .fg(Theme::accent())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            match state.effective_band_focus() {
                TriageBandFocus::Incidents => "INCIDENTS BAND",
                TriageBandFocus::Timeline => "TIMELINE BAND",
            },
            Style::default()
                .fg(Theme::fg())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            " (↑/↓ across bands)  │  ",
            Style::default().fg(Theme::dim()),
        ),
        Span::styled(
            "[w] Window: ",
            Style::default()
                .fg(Theme::cyan())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            state.current_window_label(),
            Style::default().fg(Theme::fg()),
        ),
        Span::styled("  │  ", Style::default().fg(Theme::dim())),
        Span::styled(
            "[f] Filter: ",
            Style::default()
                .fg(Theme::yellow())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            state.incident_filter.label(),
            Style::default().fg(Theme::fg()),
        ),
        Span::styled("  │  ", Style::default().fg(Theme::dim())),
        Span::styled(
            "[Tab] Infra: ",
            Style::default()
                .fg(Theme::accent())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{} non-deployment warning(s)", state.filtered_infra().len()),
            Style::default().fg(Theme::fg()),
        ),
    ]);

    let line2 = Line::from(vec![
        Span::styled(" 💡 SRE Triage: ", Style::default().fg(Theme::dim())),
        Span::styled(
            "Top band isolates active incidents. Bottom band isolates recent rollouts.",
            Style::default().fg(Theme::dim()),
        ),
    ]);

    let lines = if inner.height >= 2 {
        vec![line1, line2]
    } else {
        vec![line1]
    };

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

pub fn render_guide_banner_preview(f: &mut Frame, area: Rect, enabled: bool) {
    if enabled {
        let dummy_state = ChangedViewState {
            show_guide_banner: true,
            ..ChangedViewState::new()
        };
        render_guide_banner(f, area, &dummy_state);
    } else {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(Theme::border_type())
            .border_style(Style::default().fg(Theme::border()))
            .title(Span::styled(
                " 💡 SRE Scope & Triage Guide (Disabled) ",
                Style::default().fg(Theme::dim()),
            ));
        let inner = block.inner(area);
        f.render_widget(block, area);

        let lines = vec![
            Line::from(vec![
                Span::styled("Banner is currently ", Style::default().fg(Theme::dim())),
                Span::styled(
                    "Disabled",
                    Style::default()
                        .fg(Theme::yellow())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    ". Press Space/Enter to enable it on startup.",
                    Style::default().fg(Theme::dim()),
                ),
            ]),
            Line::from(""),
            Line::from(vec![Span::styled(
                "When enabled, the triage guide sits directly above the :changed table to explain:",
                Style::default().fg(Theme::fg()),
            )]),
            Line::from(vec![
                Span::styled("• ", Style::default().fg(Theme::cyan())),
                Span::styled(
                    "[CHANGED]: ",
                    Style::default()
                        .fg(Theme::cyan())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "Workloads with actual rollouts, image upgrades, and config diffs.",
                    Style::default().fg(Theme::dim()),
                ),
            ]),
            Line::from(vec![
                Span::styled("• ", Style::default().fg(Theme::yellow())),
                Span::styled(
                    "[S] Scaled: ",
                    Style::default()
                        .fg(Theme::yellow())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "HPA horizontal autoscaling events, replica count edits, and scale-to-zero.",
                    Style::default().fg(Theme::dim()),
                ),
            ]),
            Line::from(vec![
                Span::styled("• ", Style::default().fg(Theme::red())),
                Span::styled(
                    "[u] Failing: ",
                    Style::default()
                        .fg(Theme::red())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "Older unchanged workloads that are failing or producing warning events.",
                    Style::default().fg(Theme::dim()),
                ),
            ]),
            Line::from(vec![
                Span::styled("• ", Style::default().fg(Theme::accent())),
                Span::styled(
                    "[Tab] Infra: ",
                    Style::default()
                        .fg(Theme::accent())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "Cluster-wide non-deployment infrastructure alerts (CNI, Ingress, Istio, Nodes).",
                    Style::default().fg(Theme::dim()),
                ),
            ]),
        ];
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
    }
}

pub fn render_changed_view(f: &mut Frame, area: Rect, state: &ChangedViewState) {
    let (banner_block, health, controls) = summary_banner(state);
    let b_height = banner_height(&health, &controls, area.width);
    let guide_h = if state.show_guide_banner && area.height >= 28 {
        if area.height >= 34 {
            4
        } else {
            3
        }
    } else {
        0
    };
    let card_visible = state.active_tab == ChangedTab::Deployments
        && state
            .report
            .as_ref()
            .map_or(false, |r| !r.deployments.is_empty())
        && area.height.saturating_sub(b_height + guide_h + 1) >= CARD_MIN_HEIGHT;
    let footer_h = footer_height(area.width, state, card_visible);

    let constraints = if guide_h > 0 {
        vec![
            Constraint::Length(b_height),
            Constraint::Length(guide_h),
            Constraint::Min(8),           // Main workspace
            Constraint::Length(footer_h), // Footer shortcuts
        ]
    } else {
        vec![
            Constraint::Length(b_height),
            Constraint::Min(8),           // Main workspace
            Constraint::Length(footer_h), // Footer shortcuts
        ]
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    render_summary_banner(f, chunks[0], banner_block, health, controls);

    let (main_chunk, footer_chunk) = if guide_h > 0 {
        render_guide_banner(f, chunks[1], state);
        (chunks[2], chunks[3])
    } else {
        (chunks[1], chunks[2])
    };

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
        f.render_widget(loading_p, main_chunk);
        render_footer(f, footer_chunk, state, false);
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
        f.render_widget(err_p, main_chunk);
        render_footer(f, footer_chunk, state, false);
        return;
    }

    match state.active_tab {
        ChangedTab::Deployments => render_deployments_tab(f, main_chunk, state),
        ChangedTab::Infra => render_infra_tab(f, main_chunk, state),
    }

    let card_visible =
        state.active_tab == ChangedTab::Deployments && main_chunk.height >= CARD_MIN_HEIGHT;
    render_footer(f, footer_chunk, state, card_visible);
}

/// The banner's block, its health badges, and its controls (window, filter,
/// scope, infra). They share one line when it fits and take two when not, so
/// the controls are never clipped off a narrow terminal.
fn summary_banner(state: &ChangedViewState) -> (Block<'static>, Line<'static>, Line<'static>) {
    let (crashing, oom, error, pending, probe_failures, rolling) = if let Some(r) = &state.report {
        (
            r.summary.crashing_count,
            r.summary.oom_count,
            r.summary.error_count,
            r.summary.pending_count,
            r.summary.probe_failure_count,
            r.summary.rolling_count,
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
    } else if error > 0 || pending > 0 || probe_failures > 0 {
        Theme::status_warn()
    } else if rolling > 0 {
        Style::default().fg(Theme::cyan())
    } else {
        Theme::status_ok()
    };

    let title_badge = match state.active_tab {
        ChangedTab::Deployments => {
            " 🚨 SRE INCIDENT INVESTIGATOR & CHANGE TRIAGE [Tab: Workloads] "
        }
        ChangedTab::Infra => " 📦 SRE INCIDENT INVESTIGATOR — INFRASTRUCTURE CHANGES [Tab: Infra] ",
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
            format!(" 🩺 PROBE: {probe_failures} "),
            if probe_failures > 0 {
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
        Span::styled("Infra: ", Theme::header_label()),
        Span::styled(
            format!("{infra_count}"),
            Style::default()
                .fg(Theme::fg())
                .add_modifier(Modifier::BOLD),
        ),
        if state.is_refreshing {
            Span::styled(
                "  │  ⟳ Refreshing...",
                Style::default()
                    .fg(Theme::cyan())
                    .add_modifier(Modifier::BOLD),
            )
        } else if let Some(last) = state.last_refreshed_at {
            let secs = last.elapsed().as_secs();
            Span::styled(
                format!("  │  ⟳ Live (5s) [{}s ago]", secs),
                Style::default().fg(Theme::dim()),
            )
        } else {
            Span::styled("  │  ⟳ Live (5s)", Style::default().fg(Theme::dim()))
        },
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
        // Split vertically into upper dual-band tables (48%) and lower diagnostic card (52%)
        let sub = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(48), Constraint::Percentage(52)])
            .split(area);
        render_dual_band_tables(f, sub[0], state);
        render_deployment_diagnostic_card(f, sub[1], state);
    } else {
        render_dual_band_tables(f, area, state);
    }
}

fn render_dual_band_tables(f: &mut Frame, area: Rect, state: &ChangedViewState) {
    let incidents = state.filtered_incidents();
    let timeline = state.filtered_timeline_deployments();

    if incidents.is_empty() {
        let sub = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(4)])
            .split(area);
        render_zero_incidents_banner(f, sub[0], state);
        render_timeline_table(f, sub[1], state, &timeline);
    } else {
        let inc_h = if incidents.len() <= 4 {
            (incidents.len() as u16 + 3).min(area.height.saturating_sub(5))
        } else {
            area.height / 2
        };
        let sub = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(inc_h.max(4)), Constraint::Min(4)])
            .split(area);
        render_incidents_table(f, sub[0], state, &incidents);
        render_timeline_table(f, sub[1], state, &timeline);
    }
}

fn render_zero_incidents_banner(f: &mut Frame, area: Rect, state: &ChangedViewState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Theme::status_ok())
        .title(Span::styled(
            " ✨ Workload Incidents: 0 Active (Cluster Healthy) ",
            Theme::status_ok().add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let line = Line::from(vec![
        Span::styled("  🟢 ", Theme::status_ok()),
        Span::styled(
            format!(
                "No active incidents detected in {} window. All workloads running & ready.",
                state.current_window_label()
            ),
            Style::default()
                .fg(Theme::fg())
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    f.render_widget(Paragraph::new(vec![line]), inner);
}

fn build_workload_row(
    d: &AppDeploymentChange,
    is_selected: bool,
    is_band_focused: bool,
) -> Row<'static> {
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
        IncidentStatus::ProbeFailure => Cell::from(Span::styled(
            " 🩺 PROBE FAIL ",
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
        d.namespace.clone(),
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

    let short_detail =
        srelens_kube::changed::condense_diagnostic(d.failure_category, &d.failure_detail);
    let detail_text = if d.failure_category != FailureCategory::None || !short_detail.is_empty() {
        format!("{} {}", d.failure_category.badge(), short_detail)
            .trim()
            .to_string()
    } else if !d.image_diff.is_empty() {
        format!("📷 {}", d.image_diff)
    } else {
        "🟢 Healthy".to_string()
    };
    let detail_style = match d.failure_category {
        FailureCategory::Compute
        | FailureCategory::Storage
        | FailureCategory::Network
        | FailureCategory::Probe => Style::default()
            .fg(Theme::yellow())
            .add_modifier(Modifier::BOLD),
        FailureCategory::App | FailureCategory::Image => Style::default()
            .fg(Theme::red())
            .add_modifier(Modifier::BOLD),
        FailureCategory::None => Style::default().fg(Theme::green()),
    };
    let detail_cell = Cell::from(Span::styled(sanitize_span_text(&detail_text), detail_style));

    let deployed_cell = Cell::from(Span::styled(
        d.deployed_age.clone(),
        Style::default().fg(Theme::dim()),
    ));

    let recent_change_text = recent_change_str(d);
    let recent_change_style = match d.change_kind {
        ChangeKind::Rollout => Style::default().fg(Theme::cyan()),
        ChangeKind::Scaled => Style::default().fg(Theme::yellow()),
        ChangeKind::FailingOnly => {
            if d.incident_status != IncidentStatus::Healthy {
                Style::default().fg(Theme::red())
            } else {
                Style::default().fg(Theme::dim())
            }
        }
    };
    let recent_change_cell = Cell::from(Span::styled(
        sanitize_span_text(&recent_change_text),
        recent_change_style,
    ));

    let row = Row::new(vec![
        status_cell,
        name_cell,
        ns_cell,
        rev_cell,
        ready_cell,
        recent_change_cell,
        deployed_cell,
        detail_cell,
    ]);

    if is_selected {
        if is_band_focused {
            row.style(Theme::selected_row())
        } else {
            row.style(Style::default().bg(Theme::sel_bg()))
        }
    } else {
        row
    }
}

fn compute_workload_widths(deps: &[&AppDeploymentChange]) -> [Constraint; 8] {
    let workload_widths = deps.iter().map(|d| {
        let kind_tag = match d.kind.as_str() {
            "CronJob" => 5,
            "StatefulSet" | "Job" => 6,
            _ => 0,
        };
        let change =
            change_tag(d.change_kind).map_or(0, |t| unicode_width::UnicodeWidthStr::width(t));
        unicode_width::UnicodeWidthStr::width(d.app_name.as_str()) + kind_tag + change
    });
    let ns_widths = deps
        .iter()
        .map(|d| unicode_width::UnicodeWidthStr::width(d.namespace.as_str()));
    let rev_widths = deps.iter().map(|d| {
        if let Some(ref g) = d.gitops {
            format!("r{} ({})", d.current_revision, g.sync_revision).len()
        } else {
            format!("r{}", d.current_revision).len()
        }
    });
    let recent_change_widths = deps
        .iter()
        .map(|d| unicode_width::UnicodeWidthStr::width(recent_change_str(d).as_str()));

    [
        Constraint::Length(12),
        Constraint::Length(column_width("WORKLOAD", workload_widths)),
        Constraint::Length(column_width("NAMESPACE", ns_widths)),
        Constraint::Length(column_width("REVISION / GITOPS", rev_widths)),
        Constraint::Length(7),
        Constraint::Length(column_width("RECENT CHANGE", recent_change_widths)),
        Constraint::Length(8),
        Constraint::Min(28),
    ]
}

fn workload_header() -> Row<'static> {
    let header_cells = [
        Cell::from(Span::styled("STATUS", Theme::table_header())),
        Cell::from(Span::styled("WORKLOAD", Theme::table_header())),
        Cell::from(Span::styled("NAMESPACE", Theme::table_header())),
        Cell::from(Span::styled("REVISION / GITOPS", Theme::table_header())),
        Cell::from(Span::styled("READY", Theme::table_header())),
        Cell::from(Span::styled("RECENT CHANGE", Theme::table_header())),
        Cell::from(Span::styled("DEPLOYED", Theme::table_header())),
        Cell::from(Span::styled(
            "ROOT CAUSE / SRE DIAGNOSTIC",
            Theme::table_header(),
        )),
    ];
    Row::new(header_cells).height(1).bottom_margin(0)
}

fn render_incidents_table(
    f: &mut Frame,
    area: Rect,
    state: &ChangedViewState,
    incidents: &[&AppDeploymentChange],
) {
    let is_focused = state.effective_band_focus() == TriageBandFocus::Incidents;
    let border_style = if is_focused {
        Style::default()
            .fg(Theme::red())
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Theme::border())
    };
    let title = if is_focused {
        format!(
            " 🚨 Active Workload Incidents ({}) [FOCUSED] ",
            incidents.len()
        )
    } else {
        format!(
            " 🚨 Active Workload Incidents ({}) [Press 't' to focus] ",
            incidents.len()
        )
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        .title(Span::styled(
            title,
            if is_focused {
                Style::default()
                    .fg(Theme::red())
                    .add_modifier(Modifier::BOLD)
            } else {
                Theme::table_header()
            },
        ));

    let selected_pos = incidents.iter().position(|d| {
        state.selected_deployment().map_or(false, |sel| {
            sel.app_name == d.app_name && sel.namespace == d.namespace && sel.kind == d.kind
        })
    });

    let rows: Vec<Row> = incidents
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let is_row_selected = selected_pos == Some(i);
            build_workload_row(d, is_row_selected, is_focused)
        })
        .collect();

    let widths = compute_workload_widths(incidents);
    let table = Table::new(rows, widths)
        .header(workload_header())
        .block(block)
        .column_spacing(1);

    let mut table_state = TableState::default();
    table_state.select(selected_pos.or(Some(state.incident_selected_idx)));
    f.render_stateful_widget(table, area, &mut table_state);
}

fn render_timeline_table(
    f: &mut Frame,
    area: Rect,
    state: &ChangedViewState,
    timeline: &[&AppDeploymentChange],
) {
    let is_focused = state.effective_band_focus() == TriageBandFocus::Timeline;
    let border_style = if is_focused {
        Style::default().fg(Theme::border_focus())
    } else {
        Style::default().fg(Theme::border())
    };
    let title = if is_focused {
        format!(
            " 🕒 Timeline: Recent Rollouts [{}] [FOCUSED] ",
            state.current_window_label()
        )
    } else {
        format!(
            " 🕒 Timeline: Recent Rollouts [{}] [Press 't' to focus] ",
            state.current_window_label()
        )
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        .title(Span::styled(
            title,
            if is_focused {
                Style::default()
                    .fg(Theme::accent())
                    .add_modifier(Modifier::BOLD)
            } else {
                Theme::table_header()
            },
        ));

    if timeline.is_empty() {
        let msg = if !state.filter_query.is_empty() {
            format!(
                "No rollouts matching query '{}'. Press / to change search or Esc to clear.",
                state.filter_query
            )
        } else if state.incident_filter != IncidentFilter::All {
            format!(
                "No rollouts in the window match the {} filter. Press f to cycle it back to ALL.",
                state.incident_filter.label()
            )
        } else {
            format!(
                "✨ 0 Rollouts in the last {}. No workloads deployed in this window.",
                state.current_window_label()
            )
        };
        let p = Paragraph::new(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(msg, Style::default().fg(Theme::dim())),
        ]))
        .block(block);
        f.render_widget(p, area);
        return;
    }

    let selected_pos = timeline.iter().position(|d| {
        state.selected_deployment().map_or(false, |sel| {
            sel.app_name == d.app_name && sel.namespace == d.namespace && sel.kind == d.kind
        })
    });

    let rows: Vec<Row> = timeline
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let is_row_selected = selected_pos == Some(i);
            build_workload_row(d, is_row_selected, is_focused)
        })
        .collect();

    let widths = compute_workload_widths(timeline);
    let table = Table::new(rows, widths)
        .header(workload_header())
        .block(block)
        .column_spacing(1);

    let mut table_state = TableState::default();
    table_state.select(selected_pos.or(Some(state.timeline_selected_idx)));
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

    // Line 1b: ArgoCD Rollout in Window, when the Why block below does not
    // already name the sync.
    let names_sync = d.gitops.as_ref().is_some_and(|g| g.rollout.is_some());
    if let (Some(argo_msg), false) = (&d.argo_rollout_in_window, names_sync) {
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

    // How much of Argo the GitOps fields below were matched against.
    if let Some(line) = state
        .report
        .as_ref()
        .and_then(|r| argo_coverage_line(&r.argo))
    {
        lines.push(line);
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
        if let Some(ref msg) = g
            .sync_message
            .as_deref()
            .filter(|m| !m.starts_with("successfully synced"))
        {
            let (label, color) = if g.is_health_message {
                ("   Health: ", Theme::yellow())
            } else {
                ("   Sync Error: ", Theme::red())
            };
            lines.push(Line::from(vec![
                Span::styled(label, Theme::header_label()),
                Span::styled(
                    sanitize_span_text(msg),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
            ]));
        }
    }
    lines.extend(why_lines(d, state));

    // Why a non-rollout row is listed, with the rollout age for contrast.
    match d.change_kind {
        ChangeKind::Rollout => {}
        ChangeKind::Scaled => lines.push(Line::from(Span::styled(
            format!(
                "{} {} ago; last rollout {} ago.",
                d.change_detail.as_deref().unwrap_or("Scaled"),
                row_age(d),
                d.deployed_age
            ),
            Style::default().fg(Theme::dim()),
        ))),
        ChangeKind::FailingOnly => {
            let reason = if d.incident_status == IncidentStatus::Healthy {
                format!(
                    "Not changed in the last {}; warning events occurred in this window.",
                    state.current_window_label()
                )
            } else {
                format!(
                    "Not changed in the last {}; actively failing in this window.",
                    state.current_window_label()
                )
            };
            lines.push(Line::from(Span::styled(
                reason,
                Style::default().fg(Theme::dim()),
            )));
        }
    }

    push_gap(&mut lines);

    // Line 3: Root Cause & Failure Detail
    let cat_style = match d.failure_category {
        FailureCategory::Compute
        | FailureCategory::Storage
        | FailureCategory::Network
        | FailureCategory::Probe => Style::default()
            .fg(Theme::yellow())
            .add_modifier(Modifier::BOLD),
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
    let mut actions = String::from(
        "[Enter/d] Describe   [l] Logs   [y] YAML   [s] Quick AI RCA   [a] Assistant   [r] Refresh",
    );
    if state.cause_link_for(d).is_ok() {
        actions.push_str("   [g] Open PR");
    }
    lines.push(Line::from(vec![
        Span::styled("Actions: ", Theme::header_label()),
        Span::styled(
            actions,
            Style::default()
                .fg(Theme::accent())
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    let p = Paragraph::new(lines).block(block).wrap(Wrap { trim: true });
    f.render_widget(p, area);
}

/// "Argo: …" on the card: how many Applications the GitOps fields were
/// matched against, from where, and how old they are. `None` when no Argo
/// manages the cluster, since there is nothing to qualify.
pub fn argo_coverage_line(c: &ArgoCoverage) -> Option<Line<'static>> {
    let from = match &c.hub {
        Some(hub) => format!("from hub {hub}"),
        None => "on this cluster".to_string(),
    };
    let age = c
        .fetched_at
        .as_deref()
        .and_then(|t| t.parse::<srelens_kube::k8s_openapi::jiff::Timestamp>().ok())
        .map(|t| {
            let secs = srelens_kube::k8s_openapi::jiff::Timestamp::now()
                .duration_since(t)
                .as_secs();
            format!(", as of {} ago", srelens_kube::format_age(secs.max(0)))
        })
        .unwrap_or_default();
    let n = c.apps_loaded;
    let dim = Style::default().fg(Theme::dim());
    let warn = Style::default().fg(Theme::yellow());
    let (text, style) = match c.state {
        ArgoCoverageState::NoArgo => return None,
        ArgoCoverageState::Complete => (format!("{n} apps {from}{age}"), dim),
        ArgoCoverageState::Stale => {
            let failed = c
                .error
                .as_deref()
                .map(|e| format!("; last refresh: {e}"))
                .unwrap_or_default();
            (format!("{n} apps {from}{age}{failed}"), warn)
        }
        ArgoCoverageState::Partial => (
            format!("loading {from} ({n} apps for this cluster so far)"),
            warn,
        ),
        ArgoCoverageState::Unavailable => (
            format!(
                "unavailable: {}",
                c.error.as_deref().unwrap_or("unknown error")
            ),
            Style::default()
                .fg(Theme::yellow())
                .add_modifier(Modifier::BOLD),
        ),
    };
    Some(Line::from(vec![
        Span::styled("Argo: ", Theme::header_label()),
        Span::styled(sanitize_span_text(&text), style),
    ]))
}

/// Pull requests and commits the Why block lists before counting the rest.
const MAX_CAUSE_LINES: usize = 5;

fn short_sha(rev: &str) -> String {
    rev.chars().take(7).collect()
}

fn indented(text: &str, style: Style) -> Line<'static> {
    Line::from(vec![
        Span::raw("   "),
        Span::styled(sanitize_span_text(text), style),
    ])
}

/// Why the workload rolled out: a restart the cluster recorded, or the Argo
/// sync behind it and the pull requests GitHub names for it — or what could
/// not be found out, said as such.
pub fn why_lines(d: &AppDeploymentChange, state: &ChangedViewState) -> Vec<Line<'static>> {
    let label = || Span::styled("Why: ", Theme::header_label());
    let dim = Style::default().fg(Theme::dim());
    let warn = Style::default()
        .fg(Theme::yellow())
        .add_modifier(Modifier::BOLD);
    let strong = Style::default()
        .fg(Theme::fg())
        .add_modifier(Modifier::BOLD);
    let said = |text: &str, style: Style| {
        vec![Line::from(vec![
            label(),
            Span::styled(sanitize_span_text(text), style),
        ])]
    };

    if let Some(cause) = &d.local_cause {
        return said(cause, strong);
    }
    if let Some(msg) = &d.gitops_unresolved {
        return said(msg, warn);
    }
    let Some(g) = &d.gitops else {
        return Vec::new();
    };
    let Some(r) = &g.rollout else {
        return match &g.rollout_unmatched {
            Some(why) => said(why, dim),
            None => Vec::new(),
        };
    };

    let rev = |s: &str| {
        if r.is_chart {
            s.to_string()
        } else {
            short_sha(s)
        }
    };
    let mut sync = format!("Argo sync #{} to {}", r.history_id, rev(&r.revision));
    if let Some(prev) = &r.previous_revision {
        sync.push_str(&format!(" from {}", rev(prev)));
    }
    if let Some(who) = &r.initiated_by {
        sync.push_str(&format!(", by {who}"));
    }
    if let Ok(at) = r
        .deployed_at
        .parse::<srelens_kube::k8s_openapi::jiff::Timestamp>()
    {
        let secs = srelens_kube::k8s_openapi::jiff::Timestamp::now()
            .duration_since(at)
            .as_secs();
        sync.push_str(&format!(", {} ago", srelens_kube::format_age(secs.max(0))));
    }
    if r.approximate {
        sync.push_str(" (latest sync in the window)");
    }
    let via = match g.matched_by.as_str() {
        "trackingId" => "tracking id",
        "resources" => "app resources",
        "label" => "instance label",
        other => other,
    };
    sync.push_str(&format!(" [via {via}]"));
    let mut lines = said(
        &sync,
        Style::default()
            .fg(Theme::cyan())
            .add_modifier(Modifier::BOLD),
    );

    let ask = match cause_ask(d) {
        Some(Ok(ask)) => ask,
        Some(Err(reason)) => {
            lines.push(indented(&reason, dim));
            return lines;
        }
        None => return lines,
    };
    match state.causes.get(&ask.key) {
        None | Some(CauseLookup::Loading) => lines.push(indented(
            "GitHub: looking up the pull requests behind this sync...",
            dim,
        )),
        Some(CauseLookup::Failed(e)) => {
            lines.push(indented(&format!("GitHub: {e}. [r] to retry."), warn))
        }
        Some(CauseLookup::Ready(c)) => lines.extend(cause_lines(c, d, strong, dim, warn)),
    }
    lines
}

fn cause_lines(
    c: &RolloutCause,
    d: &AppDeploymentChange,
    strong: Style,
    dim: Style,
    warn: Style,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    if c.rolled_back {
        out.push(indented(
            "Rollback: this sync undid the changes below",
            warn,
        ));
    }
    let image_changed = !d.previous_images.is_empty() && d.previous_images != d.current_images;

    let mut items: Vec<(String, Style)> = Vec::new();
    for p in &c.pulls {
        if p.is_bot {
            let mut text = format!("Bot PR #{} \"{}\" by {}", p.number, p.title, p.user);
            if image_changed {
                text.push_str(&format!(" (image {})", d.image_diff));
            }
            items.push((text, Style::default().fg(Theme::fg())));
        } else {
            items.push((
                format!("PR #{} \"{}\" by {}", p.number, p.title, p.user),
                strong,
            ));
        }
    }
    for sha in &c.direct_commits {
        let Some(cm) = c.commits.iter().find(|x| &x.sha == sha) else {
            continue;
        };
        let what = if cm.is_bot {
            "Bot commit"
        } else {
            "Direct commit"
        };
        items.push((
            format!(
                "{what} {} \"{}\" by {} (no PR)",
                short_sha(&cm.sha),
                cm.subject,
                cm.author
            ),
            Style::default().fg(Theme::fg()),
        ));
    }

    if items.is_empty() {
        let place = if c.path.is_empty() {
            String::new()
        } else {
            format!(" under {}", c.path)
        };
        let range = match &c.previous_revision {
            Some(p) => format!("between {} and {}", short_sha(p), short_sha(&c.revision)),
            None => format!("in {}", short_sha(&c.revision)),
        };
        out.push(indented(&format!("GitHub: no commits{place} {range}"), dim));
    }
    let hidden = items.len().saturating_sub(MAX_CAUSE_LINES);
    for (text, style) in items.into_iter().take(MAX_CAUSE_LINES) {
        out.push(indented(&text, style));
    }
    if hidden > 0 {
        let link = c
            .compare_url
            .as_deref()
            .map(|u| format!(": {u}"))
            .unwrap_or_default();
        out.push(indented(&format!("+{hidden} more{link}"), dim));
    }
    if c.other_commits > 0 {
        out.push(indented(
            &format!(
                "{} other commit{} in this range changed other paths",
                c.other_commits,
                if c.other_commits == 1 { "" } else { "s" }
            ),
            dim,
        ));
    }
    if c.truncated {
        out.push(indented(
            "GitHub returned part of this range; the list may be incomplete",
            warn,
        ));
    }
    out
}

/// Wrap message text into multiple lines bounded by `max_width`.
/// Breaks on word boundaries where possible; breaks words that exceed `max_width`.
pub fn wrap_message_text(text: &str, max_width: usize) -> Vec<String> {
    let sanitized = sanitize_span_text(text);
    if sanitized.is_empty() {
        return vec![String::new()];
    }
    let max_w = max_width.max(15);
    let mut lines = Vec::new();
    let mut current_line = String::new();
    let mut current_w = 0;

    for word in sanitized.split_whitespace() {
        let word_w = unicode_width::UnicodeWidthStr::width(word);
        if current_line.is_empty() {
            if word_w <= max_w {
                current_line.push_str(word);
                current_w = word_w;
            } else {
                // Word itself exceeds max_w; split by chars
                let mut chunk = String::new();
                let mut chunk_w = 0;
                for ch in word.chars() {
                    let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1);
                    if chunk_w + cw > max_w && !chunk.is_empty() {
                        lines.push(chunk);
                        chunk = String::new();
                        chunk_w = 0;
                    }
                    chunk.push(ch);
                    chunk_w += cw;
                }
                if !chunk.is_empty() {
                    current_line = chunk;
                    current_w = chunk_w;
                }
            }
        } else if current_w + 1 + word_w <= max_w {
            current_line.push(' ');
            current_line.push_str(word);
            current_w += 1 + word_w;
        } else {
            lines.push(current_line);
            current_line = String::new();
            current_w = 0;

            if word_w <= max_w {
                current_line.push_str(word);
                current_w = word_w;
            } else {
                let mut chunk = String::new();
                let mut chunk_w = 0;
                for ch in word.chars() {
                    let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1);
                    if chunk_w + cw > max_w && !chunk.is_empty() {
                        lines.push(chunk);
                        chunk = String::new();
                        chunk_w = 0;
                    }
                    chunk.push(ch);
                    chunk_w += cw;
                }
                if !chunk.is_empty() {
                    current_line = chunk;
                    current_w = chunk_w;
                }
            }
        }
    }

    if !current_line.is_empty() {
        lines.push(current_line);
    }

    if lines.is_empty() {
        vec![String::new()]
    } else {
        lines
    }
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
        let msg = if !state.filter_query.is_empty() {
            format!(
                "No infrastructure changes matching query '{}'. Press / to change search or Esc to clear.",
                state.filter_query
            )
        } else {
            format!(
                "No infrastructure changes or warning events detected in the last {}.",
                state.current_window_label()
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
        Cell::from(Span::styled("AGE", Theme::table_header())),
        Cell::from(Span::styled("KIND", Theme::table_header())),
        Cell::from(Span::styled("NAMESPACE", Theme::table_header())),
        Cell::from(Span::styled("NAME", Theme::table_header())),
        Cell::from(Span::styled("REASON", Theme::table_header())),
        Cell::from(Span::styled("COUNT", Theme::table_header())),
        Cell::from(Span::styled("MESSAGE", Theme::table_header())),
    ];
    let header = Row::new(header_cells).height(1).bottom_margin(0);

    let ns_col_width = column_width(
        "NAMESPACE",
        infra.iter().map(|i| i.namespace.chars().count()),
    );
    // Fixed columns: 8 (AGE) + 14 (KIND) + ns_col_width + 22 (NAME) + 16 (REASON) + 6 (COUNT) + 6 (spacing) + 2 (borders)
    let fixed_width = 8 + 14 + ns_col_width + 22 + 16 + 6 + 6 + 2;
    let msg_col_width = (area.width.saturating_sub(fixed_width as u16) as usize).max(30);

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

            let wrapped_lines = wrap_message_text(&item.message, msg_col_width);
            let row_height = wrapped_lines.len().max(1) as u16;
            let text_lines: Vec<Line> = wrapped_lines
                .into_iter()
                .map(|line| Line::from(Span::styled(line, Style::default().fg(Theme::fg()))))
                .collect();
            let msg_cell = Cell::from(Text::from(text_lines));

            let row = Row::new(vec![
                age_cell,
                kind_cell,
                ns_cell,
                name_cell,
                reason_cell,
                count_cell,
                msg_cell,
            ])
            .height(row_height);

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
        Constraint::Length(ns_col_width),
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
fn footer_line(state: &ChangedViewState, card_visible: bool) -> Line<'static> {
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
    keys.push(("[Tab]".into(), "Toggle Infra".into()));
    keys.push((
        "[b]".into(),
        if state.show_guide_banner {
            "Hide guide".into()
        } else {
            "Show guide".into()
        },
    ));
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
    Line::from(spans)
}

fn footer_height(width: u16, state: &ChangedViewState, card_visible: bool) -> u16 {
    if width == 0 {
        return 1;
    }
    let w = usize::from(width);
    let line = footer_line(state, card_visible);
    let len = line.width();
    let rows = (len + w - 1) / w;
    (rows as u16).clamp(1, 3)
}

fn render_footer(f: &mut Frame, area: Rect, state: &ChangedViewState, card_visible: bool) {
    let line = footer_line(state, card_visible);
    let p = Paragraph::new(line).wrap(Wrap { trim: true });
    f.render_widget(p, area);
}
