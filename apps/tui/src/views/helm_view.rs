use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap},
    Frame,
};
use serde::{Deserialize, Serialize};

use crate::theme::Theme;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelmReleaseItem {
    pub name: String,
    pub namespace: String,
    pub revision: i64,
    pub status: String,
    pub chart: String,
    pub chart_version: String,
    #[serde(rename = "appVersion")]
    pub app_version: String,
    pub updated: String,
}

impl From<srelens_kube::helm::HelmReleaseSummary> for HelmReleaseItem {
    fn from(s: srelens_kube::helm::HelmReleaseSummary) -> Self {
        Self {
            name: s.name,
            namespace: s.namespace,
            revision: s.revision,
            status: s.status,
            chart: s.chart,
            chart_version: s.chart_version,
            app_version: s.app_version,
            updated: s.updated,
        }
    }
}

pub struct HelmViewState {
    pub releases: Vec<HelmReleaseItem>,
    pub selected_idx: usize,
    pub is_loading: bool,
    pub error: Option<String>,
    pub filter_query: String,
}

impl HelmViewState {
    pub fn new() -> Self {
        Self {
            releases: Vec::new(),
            selected_idx: 0,
            is_loading: true,
            error: None,
            filter_query: String::new(),
        }
    }

    pub fn set_releases(&mut self, releases: Vec<HelmReleaseItem>) {
        if self.releases == releases {
            self.is_loading = false;
            self.error = None;
            return;
        }
        let sel_target = self.selected_release().map(|r| (r.name.clone(), r.namespace.clone()));
        self.releases = releases;
        self.is_loading = false;
        self.error = None;
        let indices = self.filtered_indices();
        if let Some((name, ns)) = sel_target {
            if let Some(pos) = indices.iter().position(|&idx| {
                self.releases.get(idx).map(|r| r.name == name && r.namespace == ns).unwrap_or(false)
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
            return (0..self.releases.len()).collect();
        }
        let q = self.filter_query.to_lowercase();
        self.releases
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                r.name.to_lowercase().contains(&q)
                    || r.namespace.to_lowercase().contains(&q)
                    || r.chart.to_lowercase().contains(&q)
                    || r.status.to_lowercase().contains(&q)
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

    pub fn selected_release(&self) -> Option<&HelmReleaseItem> {
        let indices = self.filtered_indices();
        let idx = *indices.get(self.selected_idx)?;
        self.releases.get(idx)
    }
}

pub fn render_helm_view(f: &mut Frame, area: Rect, state: &HelmViewState) {
    let filtered = state.filtered_indices();
    let count_text = if state.filter_query.is_empty() {
        format!("{}", state.releases.len())
    } else {
        format!("{}/{}", filtered.len(), state.releases.len())
    };

    let title = format!(
        " ⎈ Helm 3 Releases [{}] (<Enter> Deep Inspector  <v> Values  <y> Manifest  <d> History  <R> Refresh  <r> Rollback  <ctrl-d> Uninstall  <Esc> Back) ",
        count_text
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(Theme::border_type())
        .border_style(Style::default().fg(Theme::border()))
        .title(Span::styled(title, Theme::title()));

    let mut inner = block.inner(area);
    f.render_widget(block, area);

    if state.is_loading {
        let loading_msg = Paragraph::new("⟳ Loading Helm releases from cluster...")
            .style(Style::default().fg(Theme::cyan()));
        f.render_widget(loading_msg, inner);
        return;
    }

    if let Some(ref err) = state.error {
        let stale = !state.releases.is_empty();
        let message = if stale {
            format!("Refresh failed; rows are stale: {err}")
        } else {
            format!("Failed to load Helm releases: {err}")
        };
        let error = Paragraph::new(message)
            .wrap(Wrap { trim: true })
            .style(Style::default().fg(Theme::red()));
        let recovery = Paragraph::new(if stale {
            "Press R to retry. Rollback is disabled."
        } else {
            "Press R to retry."
        }).wrap(Wrap { trim: true }).style(Style::default().fg(Theme::red()));
        let recovery_height = recovery.line_count(inner.width).min(inner.height as usize) as u16;
        // Keep the table header, its margin and at least one cached release visible.
        let table_height = if stale { 3.min(inner.height.saturating_sub(recovery_height)) } else { 0 };
        let available = inner.height.saturating_sub(recovery_height + table_height);
        let error_lines = error.line_count(inner.width);
        let error_height = error_lines.min(available as usize) as u16;
        let truncated = error_lines > error_height as usize && error_height > 0;
        let marker_height = u16::from(truncated);
        let regions = Layout::vertical([
            Constraint::Length(error_height.saturating_sub(marker_height)),
            Constraint::Length(marker_height),
            Constraint::Length(recovery_height),
            Constraint::Min(table_height),
        ]).split(inner);
        f.render_widget(error, regions[0]);
        if truncated {
            f.render_widget(Paragraph::new("… error truncated").style(Style::default().fg(Theme::red())), regions[1]);
        }
        f.render_widget(recovery, regions[2]);
        if !stale { return; }
        inner = regions[3];
    }

    if state.releases.is_empty() {
        let empty_msg = Paragraph::new("No Helm releases found in current namespace.")
            .style(Style::default().fg(Theme::dim()));
        f.render_widget(empty_msg, inner);
        return;
    }

    if filtered.is_empty() {
        let empty_msg = Paragraph::new(format!("No releases matching filter '{}'", state.filter_query))
            .style(Style::default().fg(Theme::dim()));
        f.render_widget(empty_msg, inner);
        return;
    }

    let headers = Row::new(vec![
        Cell::from("NAMESPACE").style(Theme::table_header()),
        Cell::from("NAME").style(Theme::table_header()),
        Cell::from("REVISION").style(Theme::table_header()),
        Cell::from("STATUS").style(Theme::table_header()),
        Cell::from("CHART").style(Theme::table_header()),
        Cell::from("APP VERSION").style(Theme::table_header()),
        Cell::from("UPDATED").style(Theme::table_header()),
    ])
    .height(1)
    .bottom_margin(1);

    let rows: Vec<Row> = filtered
        .iter()
        .enumerate()
        .map(|(display_idx, &real_idx)| {
            let rel = &state.releases[real_idx];
            let is_selected = display_idx == state.selected_idx;
            let status_style = if rel.status == "deployed" {
                Theme::status_ok()
            } else if rel.status.contains("fail") {
                Theme::status_error()
            } else {
                Theme::status_warn()
            };

            let row_style = if is_selected {
                Theme::selected_row()
            } else {
                Style::default()
            };

            let chart_display = if rel.chart_version.is_empty() {
                rel.chart.clone()
            } else {
                format!("{}-{}", rel.chart, rel.chart_version)
            };

            Row::new(vec![
                Cell::from(rel.namespace.as_str()),
                Cell::from(rel.name.as_str()),
                Cell::from(rel.revision.to_string()),
                Cell::from(rel.status.as_str()).style(status_style),
                Cell::from(chart_display),
                Cell::from(rel.app_version.as_str()),
                Cell::from(rel.updated.as_str()),
            ])
            .style(row_style)
        })
        .collect();

    let mut max_ns = "NAMESPACE".len();
    let mut max_name = "NAME".len();
    let mut max_rev = "REVISION".len();
    let mut max_status = "STATUS".len();
    let mut max_chart = "CHART".len();
    let mut max_app_v = "APP VERSION".len();
    let mut max_updated = "UPDATED".len();

    for &idx in &filtered {
        let rel = &state.releases[idx];
        max_ns = max_ns.max(rel.namespace.len());
        max_name = max_name.max(rel.name.len());
        max_rev = max_rev.max(rel.revision.to_string().len());
        max_status = max_status.max(rel.status.len());
        let chart_display_len = if rel.chart_version.is_empty() {
            rel.chart.len()
        } else {
            rel.chart.len() + 1 + rel.chart_version.len()
        };
        max_chart = max_chart.max(chart_display_len);
        max_app_v = max_app_v.max(rel.app_version.len());
        max_updated = max_updated.max(rel.updated.len());
    }

    let widths = [
        Constraint::Length((max_ns + 1) as u16),
        Constraint::Min((max_name + 1) as u16),
        Constraint::Length((max_rev + 1) as u16),
        Constraint::Length((max_status + 1) as u16),
        Constraint::Length((max_chart + 1) as u16),
        Constraint::Length((max_app_v + 1) as u16),
        Constraint::Length((max_updated + 1) as u16),
    ];

    let table = Table::new(rows, widths).header(headers);
    f.render_widget(table, inner);
}
