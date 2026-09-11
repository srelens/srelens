use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Tabs, Wrap},
    Frame,
};
use srelens_kube::helm::{HelmReleaseDetail, HelmRevision};

use crate::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelmDetailTab {
    Overview = 0,
    ValuesDiff = 1,
    Revisions = 2,
    Manifest = 3,
    Notes = 4,
}

impl HelmDetailTab {
    pub fn title(&self) -> &'static str {
        match self {
            Self::Overview => "1: Overview",
            Self::ValuesDiff => "2: Values Diff",
            Self::Revisions => "3: Revisions",
            Self::Manifest => "4: Manifest",
            Self::Notes => "5: Notes",
        }
    }

    pub fn from_index(idx: usize) -> Self {
        match idx {
            0 => Self::Overview,
            1 => Self::ValuesDiff,
            2 => Self::Revisions,
            3 => Self::Manifest,
            4 => Self::Notes,
            _ => Self::Overview,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValuesDiffMode {
    CustomVsComputed,
    CustomVsDefault,
    RevisionVsPrevious,
}

#[derive(Debug, Clone)]
pub enum DiffKind {
    Same,
    Add,
    Remove,
}

#[derive(Debug, Clone)]
pub struct DiffLine {
    pub kind: DiffKind,
    pub text: String,
    pub line_num_left: Option<usize>,
    pub line_num_right: Option<usize>,
}

pub struct HelmDetailViewState {
    pub release_name: String,
    pub namespace: String,
    pub active_tab: HelmDetailTab,
    pub is_loading: bool,
    pub error: Option<String>,
    pub detail: Option<HelmReleaseDetail>,
    pub previous_detail: Option<HelmReleaseDetail>,
    pub selected_revision_idx: usize,
    pub scroll_offset: usize,
    pub values_diff_mode: ValuesDiffMode,
    pub filter_query: String,
    pub search_query: String,
    pub search_matches: Vec<usize>,
    pub current_match_idx: Option<usize>,
}

impl HelmDetailViewState {
    pub fn new(name: String, namespace: String) -> Self {
        Self {
            release_name: name,
            namespace,
            active_tab: HelmDetailTab::Overview,
            is_loading: true,
            error: None,
            detail: None,
            previous_detail: None,
            selected_revision_idx: 0,
            scroll_offset: 0,
            values_diff_mode: ValuesDiffMode::CustomVsComputed,
            filter_query: String::new(),
            search_query: String::new(),
            search_matches: Vec::new(),
            current_match_idx: None,
        }
    }

    pub fn set_detail(&mut self, detail: HelmReleaseDetail) {
        self.detail = Some(detail);
        self.is_loading = false;
        self.error = None;
        if !self.search_query.is_empty() {
            self.recompute_search_matches();
        }
    }

    pub fn set_previous_detail(&mut self, detail: HelmReleaseDetail) {
        self.previous_detail = Some(detail);
    }

    pub fn set_error(&mut self, err: String) {
        self.error = Some(err);
        self.is_loading = false;
    }

    pub fn next_tab(&mut self) {
        let curr = self.active_tab as usize;
        self.active_tab = HelmDetailTab::from_index((curr + 1) % 5);
        self.scroll_offset = 0;
        self.recompute_search_matches();
    }

    pub fn prev_tab(&mut self) {
        let curr = self.active_tab as usize;
        self.active_tab = HelmDetailTab::from_index(if curr == 0 { 4 } else { curr - 1 });
        self.scroll_offset = 0;
        self.recompute_search_matches();
    }

    pub fn set_tab(&mut self, tab: HelmDetailTab) {
        self.active_tab = tab;
        self.scroll_offset = 0;
        self.recompute_search_matches();
    }

    pub fn toggle_diff_mode(&mut self) {
        self.values_diff_mode = match self.values_diff_mode {
            ValuesDiffMode::CustomVsComputed => ValuesDiffMode::CustomVsDefault,
            ValuesDiffMode::CustomVsDefault => ValuesDiffMode::RevisionVsPrevious,
            ValuesDiffMode::RevisionVsPrevious => ValuesDiffMode::CustomVsComputed,
        };
        self.scroll_offset = 0;
        self.recompute_search_matches();
    }

    pub fn text_lines_for_active_tab(&self) -> Vec<String> {
        match self.active_tab {
            HelmDetailTab::Manifest => self
                .detail
                .as_ref()
                .map(|d| d.manifest.lines().map(super::sanitize_span_text).collect())
                .unwrap_or_default(),
            HelmDetailTab::Notes => self
                .detail
                .as_ref()
                .map(|d| d.notes.lines().map(super::sanitize_span_text).collect())
                .unwrap_or_default(),
            HelmDetailTab::ValuesDiff => self
                .compute_values_diff()
                .into_iter()
                .map(|dl| super::sanitize_span_text(&dl.text))
                .collect(),
            HelmDetailTab::Overview | HelmDetailTab::Revisions => Vec::new(),
        }
    }

    pub fn total_lines_for_active_tab(&self) -> usize {
        match self.active_tab {
            HelmDetailTab::Manifest => self.detail.as_ref().map(|d| d.manifest.lines().count()).unwrap_or(0),
            HelmDetailTab::Notes => self.detail.as_ref().map(|d| d.notes.lines().count()).unwrap_or(0),
            HelmDetailTab::ValuesDiff => self.compute_values_diff().len(),
            HelmDetailTab::Revisions => self.detail.as_ref().map(|d| d.history.len()).unwrap_or(0),
            HelmDetailTab::Overview => 0,
        }
    }

    pub fn manifest_line_count(&self) -> usize {
        self.detail.as_ref().map(|d| d.manifest.lines().count()).unwrap_or(0)
    }

    pub fn set_search_query(&mut self, query: &str) {
        self.search_query = query.to_string();
        self.filter_query = query.to_string();
        if query.is_empty() {
            self.search_matches.clear();
            self.current_match_idx = None;
            return;
        }

        let lines = self.text_lines_for_active_tab();
        let q = query.to_lowercase();
        self.search_matches = lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.to_lowercase().contains(&q))
            .map(|(i, _)| i)
            .collect();

        if !self.search_matches.is_empty() {
            self.current_match_idx = Some(0);
            self.scroll_offset = self.search_matches[0];
        } else {
            self.current_match_idx = None;
        }
    }

    pub fn recompute_search_matches(&mut self) {
        if self.search_query.is_empty() {
            self.search_matches.clear();
            self.current_match_idx = None;
            return;
        }

        let lines = self.text_lines_for_active_tab();
        let q = self.search_query.to_lowercase();
        self.search_matches = lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.to_lowercase().contains(&q))
            .map(|(i, _)| i)
            .collect();

        if !self.search_matches.is_empty() {
            let next_idx = self.current_match_idx.unwrap_or(0).min(self.search_matches.len() - 1);
            self.current_match_idx = Some(next_idx);
            self.scroll_offset = self.search_matches[next_idx];
        } else {
            self.current_match_idx = None;
        }
    }

    pub fn next_match(&mut self) {
        if self.search_matches.is_empty() {
            return;
        }
        let next_idx = match self.current_match_idx {
            Some(curr) => (curr + 1) % self.search_matches.len(),
            None => 0,
        };
        self.current_match_idx = Some(next_idx);
        self.scroll_offset = self.search_matches[next_idx];
    }

    pub fn prev_match(&mut self) {
        if self.search_matches.is_empty() {
            return;
        }
        let prev_idx = match self.current_match_idx {
            Some(0) | None => self.search_matches.len().saturating_sub(1),
            Some(curr) => curr - 1,
        };
        self.current_match_idx = Some(prev_idx);
        self.scroll_offset = self.search_matches[prev_idx];
    }

    pub fn clear_search(&mut self) {
        self.search_query.clear();
        self.filter_query.clear();
        self.search_matches.clear();
        self.current_match_idx = None;
    }

    pub fn scroll_down(&mut self, n: usize) {
        self.scroll_offset += n;
    }

    pub fn scroll_up(&mut self, n: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(n);
    }

    pub fn scroll_to_top(&mut self) {
        self.scroll_offset = 0;
    }

    pub fn scroll_to_bottom(&mut self) {
        let total = self.total_lines_for_active_tab();
        self.scroll_offset = total.saturating_sub(1);
    }

    pub fn select_next_revision(&mut self) {
        if let Some(ref d) = self.detail {
            if !d.history.is_empty() && self.selected_revision_idx + 1 < d.history.len() {
                self.selected_revision_idx += 1;
            }
        }
    }

    pub fn select_prev_revision(&mut self) {
        if self.selected_revision_idx > 0 {
            self.selected_revision_idx -= 1;
        }
    }

    pub fn selected_revision(&self) -> Option<&HelmRevision> {
        let d = self.detail.as_ref()?;
        d.history.get(self.selected_revision_idx)
    }

    pub fn compute_values_diff(&self) -> Vec<DiffLine> {
        let (left, right) = match self.values_diff_mode {
            ValuesDiffMode::CustomVsComputed => {
                let custom_values = self.detail.as_ref().map(|d| d.values_yaml.as_str()).unwrap_or("");
                let computed_values = self.detail.as_ref().map(|d| d.computed_values_yaml.as_str()).unwrap_or("");
                (custom_values, computed_values)
            }
            ValuesDiffMode::CustomVsDefault => {
                let default_values = self.detail.as_ref().map(|d| d.chart_values_yaml.as_str()).unwrap_or("");
                let custom_values = self.detail.as_ref().map(|d| d.values_yaml.as_str()).unwrap_or("");
                (default_values, custom_values)
            }
            ValuesDiffMode::RevisionVsPrevious => {
                let prev = self.previous_detail.as_ref().map(|d| d.values_yaml.as_str()).unwrap_or("");
                let curr = self.detail.as_ref().map(|d| d.values_yaml.as_str()).unwrap_or("");
                (prev, curr)
            }
        };

        let diff = similar::TextDiff::from_lines(left, right);
        let mut lines = Vec::new();
        let mut left_line = 1;
        let mut right_line = 1;

        for change in diff.iter_all_changes() {
            let text = change.value().trim_end_matches('\n').to_string();
            match change.tag() {
                similar::ChangeTag::Equal => {
                    lines.push(DiffLine {
                        kind: DiffKind::Same,
                        text,
                        line_num_left: Some(left_line),
                        line_num_right: Some(right_line),
                    });
                    left_line += 1;
                    right_line += 1;
                }
                similar::ChangeTag::Delete => {
                    lines.push(DiffLine {
                        kind: DiffKind::Remove,
                        text,
                        line_num_left: Some(left_line),
                        line_num_right: None,
                    });
                    left_line += 1;
                }
                similar::ChangeTag::Insert => {
                    lines.push(DiffLine {
                        kind: DiffKind::Add,
                        text,
                        line_num_left: None,
                        line_num_right: Some(right_line),
                    });
                    right_line += 1;
                }
            }
        }
        lines
    }

    pub fn parse_manifest_resource_counts(&self) -> Vec<(String, usize)> {
        let Some(ref d) = self.detail else { return Vec::new() };
        let mut map: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        for doc in d.manifest.split("---") {
            for line in doc.lines() {
                let trimmed = line.trim();
                if let Some(rest) = trimmed.strip_prefix("kind:") {
                    let k = rest.trim().to_string();
                    if !k.is_empty() {
                        *map.entry(k).or_insert(0) += 1;
                    }
                    break;
                }
            }
        }
        map.into_iter().collect()
    }
}

pub fn render_helm_detail_view(f: &mut Frame, area: Rect, state: &HelmDetailViewState) {
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Tab navigation bar
            Constraint::Min(10),   // Active tab contents
            Constraint::Length(1), // Bottom hints
        ])
        .split(area);

    render_tab_strip(f, main_chunks[0], state);

    if state.is_loading {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(Theme::border_type())
            .border_style(Style::default().fg(Theme::border()));
        let inner = block.inner(main_chunks[1]);
        f.render_widget(block, main_chunks[1]);
        let msg = Paragraph::new(format!("⟳ Loading Helm release details for '{}/{}'...", state.namespace, state.release_name))
            .style(Style::default().fg(Theme::cyan()));
        f.render_widget(msg, inner);
        return;
    }

    if let Some(ref err) = state.error {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(Theme::border_type())
            .border_style(Style::default().fg(Theme::border()));
        let inner = block.inner(main_chunks[1]);
        f.render_widget(block, main_chunks[1]);
        let msg = Paragraph::new(format!("⚠ Failed to load release: {}", err))
            .style(Style::default().fg(Theme::red()));
        f.render_widget(msg, inner);
        return;
    }

    match state.active_tab {
        HelmDetailTab::Overview => render_overview_tab(f, main_chunks[1], state),
        HelmDetailTab::ValuesDiff => render_values_diff_tab(f, main_chunks[1], state),
        HelmDetailTab::Revisions => render_revisions_tab(f, main_chunks[1], state),
        HelmDetailTab::Manifest => render_manifest_tab(f, main_chunks[1], state),
        HelmDetailTab::Notes => render_notes_tab(f, main_chunks[1], state),
    }

    render_bottom_hints(f, main_chunks[2], state);
}

fn render_tab_strip(f: &mut Frame, area: Rect, state: &HelmDetailViewState) {
    let rev_str = state.detail.as_ref().map(|d| format!("v{}", d.revision)).unwrap_or_default();
    let status_str = state.detail.as_ref().map(|d| d.status.as_str()).unwrap_or("loading");
    let title = format!(" ⎈ Helm Release: {}/{} [{}] ({}) ", state.namespace, state.release_name, rev_str, status_str);

    let titles: Vec<Line> = [
        HelmDetailTab::Overview,
        HelmDetailTab::ValuesDiff,
        HelmDetailTab::Revisions,
        HelmDetailTab::Manifest,
        HelmDetailTab::Notes,
    ]
    .iter()
    .map(|tab| {
        let is_selected = *tab == state.active_tab;
        let style = if is_selected {
            Style::default().fg(Theme::accent()).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Theme::dim())
        };
        Line::from(vec![Span::styled(format!(" {} ", tab.title()), style)])
    })
    .collect();

    let tabs = Tabs::new(titles)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(Theme::border_type())
                .border_style(Style::default().fg(Theme::border()))
                .title(Span::styled(title, Theme::title())),
        )
        .highlight_style(
            Style::default()
                .fg(Theme::sel_fg())
                .bg(Theme::sel_bg())
                .add_modifier(Modifier::BOLD),
        )
        .select(state.active_tab as usize);

    f.render_widget(tabs, area);
}

fn render_overview_tab(f: &mut Frame, area: Rect, state: &HelmDetailViewState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(Theme::border_type())
        .border_style(Style::default().fg(Theme::border()))
        .title(Span::styled(" Release Overview & Topology Breakdown ", Theme::title()));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let Some(ref d) = state.detail else { return };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(8), // Release details table/cards
            Constraint::Length(1), // Separator
            Constraint::Min(6),   // Rendered Kubernetes resources
        ])
        .split(inner);

    let status_style = if d.status == "deployed" {
        Theme::status_ok()
    } else if d.status.contains("fail") {
        Theme::status_error()
    } else {
        Theme::status_warn()
    };

    let meta_lines = vec![
        Line::from(vec![
            Span::styled("  Release Name  : ", Style::default().fg(Theme::dim())),
            Span::styled(&d.name, Style::default().fg(Theme::fg()).add_modifier(Modifier::BOLD)),
            Span::raw("    "),
            Span::styled("Namespace     : ", Style::default().fg(Theme::dim())),
            Span::styled(&d.namespace, Style::default().fg(Theme::cyan())),
        ]),
        Line::from(vec![
            Span::styled("  Status        : ", Style::default().fg(Theme::dim())),
            Span::styled(&d.status, status_style.add_modifier(Modifier::BOLD)),
            Span::raw("        "),
            Span::styled("Revision      : ", Style::default().fg(Theme::dim())),
            Span::styled(format!("{} (latest)", d.revision), Style::default().fg(Theme::fg())),
        ]),
        Line::from(vec![
            Span::styled("  Chart         : ", Style::default().fg(Theme::dim())),
            Span::styled(format!("{}-{}", d.chart, d.chart_version), Style::default().fg(Theme::accent())),
            Span::raw("    "),
            Span::styled("App Version   : ", Style::default().fg(Theme::dim())),
            Span::styled(&d.app_version, Style::default().fg(Theme::fg())),
        ]),
        Line::from(vec![
            Span::styled("  Last Updated  : ", Style::default().fg(Theme::dim())),
            Span::styled(&d.updated, Style::default().fg(Theme::dim())),
        ]),
        Line::from(vec![
            Span::styled("  Revisions     : ", Style::default().fg(Theme::dim())),
            Span::styled(format!("{} revisions recorded in history", d.history.len()), Style::default().fg(Theme::dim())),
        ]),
    ];

    let meta_para = Paragraph::new(meta_lines);
    f.render_widget(meta_para, chunks[0]);

    // Resource breakdown from manifest
    let res_counts = state.parse_manifest_resource_counts();
    let mut count_lines = Vec::new();
    count_lines.push(Line::from(vec![
        Span::styled("  Kubernetes Resources in Manifest:", Style::default().fg(Theme::accent()).add_modifier(Modifier::BOLD)),
    ]));
    count_lines.push(Line::from(""));

    if res_counts.is_empty() {
        count_lines.push(Line::from(vec![
            Span::styled("    (No resources detected in rendered manifest)", Style::default().fg(Theme::dim())),
        ]));
    } else {
        for (kind, count) in res_counts {
            count_lines.push(Line::from(vec![
                Span::styled(format!("    • {:<20} : ", kind), Style::default().fg(Theme::fg())),
                Span::styled(format!("{}", count), Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD)),
            ]));
        }
    }

    let res_para = Paragraph::new(count_lines);
    f.render_widget(res_para, chunks[2]);
}

fn render_values_diff_tab(f: &mut Frame, area: Rect, state: &HelmDetailViewState) {
    let mode_desc = match state.values_diff_mode {
        ValuesDiffMode::CustomVsComputed => "[Mode: User Values (helm get values) vs Computed Values (helm get values --all)]",
        ValuesDiffMode::CustomVsDefault => "[Mode: User Values vs Chart Defaults]",
        ValuesDiffMode::RevisionVsPrevious => "[Mode: Current Revision vs Previous Revision Values]",
    };

    let search_badge = if !state.search_query.is_empty() {
        if state.search_matches.is_empty() {
            format!(" [Search: \"{}\" (0 matches)]", state.search_query)
        } else {
            format!(
                " [Search: \"{}\" ({}/{} matches, n/N)]",
                state.search_query,
                state.current_match_idx.map(|i| i + 1).unwrap_or(0),
                state.search_matches.len()
            )
        }
    } else {
        String::new()
    };

    let title = format!(" Values Diff {} (Press <m> to toggle mode){} ", mode_desc, search_badge);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(Theme::border_type())
        .border_style(Style::default().fg(Theme::border()))
        .title(Span::styled(title, Theme::title()));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let diff_lines = state.compute_values_diff();
    if diff_lines.is_empty() {
        let msg = Paragraph::new("No values diff detected (values identical or empty).")
            .style(Style::default().fg(Theme::dim()));
        f.render_widget(msg, inner);
        return;
    }

    let match_style = Style::default()
        .bg(Theme::YELLOW)
        .fg(Color::Rgb(20, 20, 20))
        .add_modifier(Modifier::BOLD);

    let query_lower = state.search_query.to_lowercase();
    let viewport_height = inner.height as usize;
    let visible_lines: Vec<Line> = diff_lines
        .iter()
        .skip(state.scroll_offset)
        .take(viewport_height)
        .map(|dl| {
            let (prefix, style) = match dl.kind {
                DiffKind::Same => ("   ", Style::default().fg(Theme::dim())),
                DiffKind::Add => (" + ", Style::default().fg(Theme::green())),
                DiffKind::Remove => (" - ", Style::default().fg(Theme::red())),
            };

            let left_str = dl.line_num_left.map(|n| format!("{:>4}", n)).unwrap_or_else(|| "    ".to_string());
            let right_str = dl.line_num_right.map(|n| format!("{:>4}", n)).unwrap_or_else(|| "    ".to_string());

            let clean_text = super::sanitize_span_text(&dl.text);
            let mut spans = vec![
                Span::styled(format!("{} {} ", left_str, right_str), Style::default().fg(Theme::dim())),
                Span::styled(prefix, style.add_modifier(Modifier::BOLD)),
            ];

            let has_match = !query_lower.is_empty()
                && clean_text.to_lowercase().contains(&query_lower);

            if has_match {
                let highlighted = super::highlight_text_matches(&clean_text, &state.search_query, style, match_style);
                spans.extend(highlighted);
            } else {
                spans.push(Span::styled(clean_text, style));
            }

            Line::from(spans)
        })
        .collect();

    let para = Paragraph::new(visible_lines);
    f.render_widget(para, inner);
}

fn render_revisions_tab(f: &mut Frame, area: Rect, state: &HelmDetailViewState) {
    let title = " Revision History (<j/k> Select  <r> Rollback to Revision N  <v>/<Enter> Inspect Revision) ";
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(Theme::border_type())
        .border_style(Style::default().fg(Theme::border()))
        .title(Span::styled(title, Theme::title()));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let Some(ref d) = state.detail else { return };
    if d.history.is_empty() {
        let msg = Paragraph::new("No revision history available.").style(Style::default().fg(Theme::dim()));
        f.render_widget(msg, inner);
        return;
    }

    let headers = Row::new(vec![
        Cell::from("REV").style(Theme::table_header()),
        Cell::from("STATUS").style(Theme::table_header()),
        Cell::from("UPDATED").style(Theme::table_header()),
        Cell::from("CHART VERSION").style(Theme::table_header()),
        Cell::from("DESCRIPTION").style(Theme::table_header()),
    ])
    .height(1)
    .bottom_margin(1);

    let rows: Vec<Row> = d
        .history
        .iter()
        .enumerate()
        .map(|(i, rev)| {
            let is_selected = i == state.selected_revision_idx;
            let status_style = if rev.status == "deployed" {
                Theme::status_ok()
            } else if rev.status.contains("fail") {
                Theme::status_error()
            } else {
                Theme::status_warn()
            };

            let row_style = if is_selected {
                Theme::selected_row()
            } else {
                Style::default()
            };

            let is_current = rev.revision == d.revision;
            let rev_display = if is_current {
                format!("{} (current)", rev.revision)
            } else {
                rev.revision.to_string()
            };

            Row::new(vec![
                Cell::from(rev_display),
                Cell::from(rev.status.as_str()).style(status_style),
                Cell::from(rev.updated.as_str()),
                Cell::from(rev.chart_version.as_str()),
                Cell::from(rev.description.as_str()),
            ])
            .style(row_style)
        })
        .collect();

    let mut max_rev = "REVISION".len();
    let mut max_status = "STATUS".len();
    let mut max_updated = "UPDATED".len();
    let mut max_chart = "CHART VERSION".len();
    let mut max_desc = "DESCRIPTION".len();

    for rev in &d.history {
        let is_current = rev.revision == d.revision;
        let rev_len = if is_current {
            format!("{} (current)", rev.revision).len()
        } else {
            rev.revision.to_string().len()
        };
        max_rev = max_rev.max(rev_len);
        max_status = max_status.max(rev.status.len());
        max_updated = max_updated.max(rev.updated.len());
        max_chart = max_chart.max(rev.chart_version.len());
        max_desc = max_desc.max(rev.description.len());
    }

    let widths = [
        Constraint::Length((max_rev + 1) as u16),
        Constraint::Length((max_status + 1) as u16),
        Constraint::Length((max_updated + 1) as u16),
        Constraint::Length((max_chart + 1) as u16),
        Constraint::Length((max_desc + 1) as u16),
    ];

    let table = Table::new(rows, widths).header(headers);
    f.render_widget(table, inner);
}

fn render_manifest_tab(f: &mut Frame, area: Rect, state: &HelmDetailViewState) {
    let search_badge = if !state.search_query.is_empty() {
        if state.search_matches.is_empty() {
            format!(" [Search: \"{}\" (0 matches)]", state.search_query)
        } else {
            format!(
                " [Search: \"{}\" ({}/{} matches, n/N)]",
                state.search_query,
                state.current_match_idx.map(|i| i + 1).unwrap_or(0),
                state.search_matches.len()
            )
        }
    } else {
        String::new()
    };

    let title = format!(" Rendered Kubernetes Manifests (YAML) (<y> Copy  </> Search){} ", search_badge);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(Theme::border_type())
        .border_style(Style::default().fg(Theme::border()))
        .title(Span::styled(title, Theme::title()));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let Some(ref d) = state.detail else { return };
    if d.manifest.is_empty() {
        let msg = Paragraph::new("Manifest is empty.").style(Style::default().fg(Theme::dim()));
        f.render_widget(msg, inner);
        return;
    }

    let match_style = Style::default()
        .bg(Theme::YELLOW)
        .fg(Color::Rgb(20, 20, 20))
        .add_modifier(Modifier::BOLD);

    let query_lower = state.search_query.to_lowercase();
    let viewport_height = inner.height as usize;
    let lines: Vec<Line> = d
        .manifest
        .lines()
        .skip(state.scroll_offset)
        .take(viewport_height)
        .enumerate()
        .map(|(i, l)| {
            let line_idx = state.scroll_offset + i + 1;
            let style = if l.starts_with("kind:") || l.starts_with("apiVersion:") {
                Style::default().fg(Theme::accent()).add_modifier(Modifier::BOLD)
            } else if l.starts_with("---") {
                Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD)
            } else if l.trim_start().starts_with('#') {
                Style::default().fg(Theme::dim())
            } else if l.contains(':') {
                Style::default().fg(Theme::fg())
            } else {
                Style::default().fg(Theme::dim())
            };

            let clean_l = super::sanitize_span_text(l);
            let mut spans = vec![
                Span::styled(format!("{:>5} │ ", line_idx), Style::default().fg(Theme::dim())),
            ];

            let has_match = !query_lower.is_empty()
                && clean_l.to_lowercase().contains(&query_lower);

            if has_match {
                let highlighted = super::highlight_text_matches(&clean_l, &state.search_query, style, match_style);
                spans.extend(highlighted);
            } else {
                spans.push(Span::styled(clean_l, style));
            }

            Line::from(spans)
        })
        .collect();

    let para = Paragraph::new(lines);
    f.render_widget(para, inner);
}

fn render_notes_tab(f: &mut Frame, area: Rect, state: &HelmDetailViewState) {
    let search_badge = if !state.search_query.is_empty() {
        if state.search_matches.is_empty() {
            format!(" [Search: \"{}\" (0 matches)]", state.search_query)
        } else {
            format!(
                " [Search: \"{}\" ({}/{} matches, n/N)]",
                state.search_query,
                state.current_match_idx.map(|i| i + 1).unwrap_or(0),
                state.search_matches.len()
            )
        }
    } else {
        String::new()
    };

    let title = format!(" Chart Release Notes (NOTES.txt) (<y> Copy  </> Search){} ", search_badge);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(Theme::border_type())
        .border_style(Style::default().fg(Theme::border()))
        .title(Span::styled(title, Theme::title()));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let Some(ref d) = state.detail else { return };
    if d.notes.is_empty() {
        let msg = Paragraph::new("No release notes (NOTES.txt) provided by chart.")
            .style(Style::default().fg(Theme::dim()));
        f.render_widget(msg, inner);
        return;
    }

    let match_style = Style::default()
        .bg(Theme::YELLOW)
        .fg(Color::Rgb(20, 20, 20))
        .add_modifier(Modifier::BOLD);

    let query_lower = state.search_query.to_lowercase();
    let viewport_height = inner.height as usize;
    let lines: Vec<Line> = d
        .notes
        .lines()
        .skip(state.scroll_offset)
        .take(viewport_height)
        .map(|l| {
            let clean_l = super::sanitize_span_text(l);
            let has_match = !query_lower.is_empty()
                && clean_l.to_lowercase().contains(&query_lower);

            if has_match {
                let highlighted = super::highlight_text_matches(
                    &clean_l,
                    &state.search_query,
                    Style::default().fg(Theme::fg()),
                    match_style,
                );
                Line::from(highlighted)
            } else {
                Line::from(Span::styled(clean_l, Style::default().fg(Theme::fg())))
            }
        })
        .collect();

    let para = Paragraph::new(lines);
    f.render_widget(para, inner);
}

fn render_bottom_hints(f: &mut Frame, area: Rect, state: &HelmDetailViewState) {
    let hints = match state.active_tab {
        HelmDetailTab::Overview => "<Tab> Switch Tab  <1-5> Jump Tab  <Esc> Back to Releases",
        HelmDetailTab::ValuesDiff => "<Tab> Switch Tab  <m> Toggle Diff Mode  </> Search  <n/N> Next/Prev  <j/k> Scroll  <g/G> Top/Bottom  <Esc> Back",
        HelmDetailTab::Revisions => "<Tab> Switch Tab  <j/k> Select Rev  <r> Rollback to Selected  <Enter>/<v> View  <Esc> Back",
        HelmDetailTab::Manifest => "<Tab> Switch Tab  <j/k> Scroll  <g/G> Top/Bottom  <y> Copy  </> Search  <n/N> Next/Prev  <Esc> Back",
        HelmDetailTab::Notes => "<Tab> Switch Tab  <j/k> Scroll  <g/G> Top/Bottom  <y> Copy  </> Search  <n/N> Next/Prev  <Esc> Back",
    };

    let p = Paragraph::new(format!(" {}", hints))
        .style(Style::default().fg(Theme::dim()));
    f.render_widget(p, area);
}
