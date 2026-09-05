use ratatui::{
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
    Frame,
};
use serde::{Deserialize, Serialize};

use crate::theme::Theme;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelmReleaseItem {
    pub name: String,
    pub namespace: String,
    pub revision: i64,
    pub status: String,
    pub chart: String,
    #[serde(rename = "appVersion")]
    pub app_version: String,
    pub updated: String,
}

pub struct HelmViewState {
    pub releases: Vec<HelmReleaseItem>,
    pub selected_idx: usize,
}

impl HelmViewState {
    pub fn new() -> Self {
        Self {
            releases: Vec::new(),
            selected_idx: 0,
        }
    }

    pub fn set_releases(&mut self, releases: Vec<HelmReleaseItem>) {
        self.releases = releases;
        if self.selected_idx >= self.releases.len() {
            self.selected_idx = self.releases.len().saturating_sub(1);
        }
    }

    pub fn select_next(&mut self) {
        if !self.releases.is_empty() && self.selected_idx + 1 < self.releases.len() {
            self.selected_idx += 1;
        }
    }

    pub fn select_prev(&mut self) {
        if self.selected_idx > 0 {
            self.selected_idx -= 1;
        }
    }

    pub fn selected_release(&self) -> Option<&HelmReleaseItem> {
        self.releases.get(self.selected_idx)
    }
}

pub fn render_helm_view(f: &mut Frame, area: Rect, state: &HelmViewState) {
    let title = format!(" Helm 3 Releases [{}] (<v> Values <y> Manifest <d> History <ctrl-d> Uninstall <Esc> Back) ", state.releases.len());

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::BORDER))
        .title(Span::styled(title, Theme::title()));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if state.releases.is_empty() {
        let empty_msg = Paragraph::new(
            "No Helm releases found in current namespace.",
        )
        .style(Style::default().fg(Theme::DIM));
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

    let rows: Vec<Row> = state
        .releases
        .iter()
        .enumerate()
        .map(|(i, rel)| {
            let is_selected = i == state.selected_idx;
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

            Row::new(vec![
                Cell::from(rel.namespace.as_str()),
                Cell::from(rel.name.as_str()),
                Cell::from(rel.revision.to_string()),
                Cell::from(rel.status.as_str()).style(status_style),
                Cell::from(rel.chart.as_str()),
                Cell::from(rel.app_version.as_str()),
                Cell::from(rel.updated.as_str()),
            ])
            .style(row_style)
        })
        .collect();

    let widths = [
        Constraint::Length(18),
        Constraint::Min(25),
        Constraint::Length(10),
        Constraint::Length(14),
        Constraint::Length(25),
        Constraint::Length(15),
        Constraint::Length(25),
    ];

    let table = Table::new(rows, widths).header(headers);
    f.render_widget(table, inner);
}
