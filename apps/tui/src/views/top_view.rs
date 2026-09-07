use std::collections::HashMap;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Row, Table};
use ratatui::Frame;

use crate::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopTab {
    Pods,
    Nodes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopSortBy {
    Cpu,
    Memory,
}

#[derive(Debug, Clone)]
pub struct TopPodRow {
    pub namespace: String,
    pub name: String,
    pub cpu_millicores: i64,
    pub cpu_req_millicores: i64,
    pub cpu_lim_millicores: i64,
    pub mem_mib: i64,
    pub mem_req_mib: i64,
    pub mem_lim_mib: i64,
}

impl TopPodRow {
    pub fn cpu_req_pct(&self) -> Option<f64> {
        if self.cpu_req_millicores > 0 {
            Some((self.cpu_millicores as f64 / self.cpu_req_millicores as f64) * 100.0)
        } else {
            None
        }
    }

    pub fn cpu_lim_pct(&self) -> Option<f64> {
        if self.cpu_lim_millicores > 0 {
            Some((self.cpu_millicores as f64 / self.cpu_lim_millicores as f64) * 100.0)
        } else {
            None
        }
    }

    pub fn mem_req_pct(&self) -> Option<f64> {
        if self.mem_req_mib > 0 {
            Some((self.mem_mib as f64 / self.mem_req_mib as f64) * 100.0)
        } else {
            None
        }
    }

    pub fn mem_lim_pct(&self) -> Option<f64> {
        if self.mem_lim_mib > 0 {
            Some((self.mem_mib as f64 / self.mem_lim_mib as f64) * 100.0)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone)]
pub struct TopNodeRow {
    pub name: String,
    pub status: String,
    pub cpu_millicores: i64,
    pub cpu_alloc_millicores: i64,
    pub mem_mib: i64,
    pub mem_alloc_mib: i64,
}

impl TopNodeRow {
    pub fn cpu_alloc_pct(&self) -> Option<f64> {
        if self.cpu_alloc_millicores > 0 {
            Some((self.cpu_millicores as f64 / self.cpu_alloc_millicores as f64) * 100.0)
        } else {
            None
        }
    }

    pub fn mem_alloc_pct(&self) -> Option<f64> {
        if self.mem_alloc_mib > 0 {
            Some((self.mem_mib as f64 / self.mem_alloc_mib as f64) * 100.0)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone)]
pub struct TopViewState {
    pub active_tab: TopTab,
    pub sort_by: TopSortBy,
    pub sort_ascending: bool,
    pub selected_idx: usize,
    pub scroll_offset: usize,
    pub pods: Vec<TopPodRow>,
    pub nodes: Vec<TopNodeRow>,
    pub filter: String,
    pub active_port_forwards: HashMap<(String, String), Vec<(u16, u16, String)>>,
}

impl Default for TopViewState {
    fn default() -> Self {
        Self::new(TopTab::Pods)
    }
}

impl TopViewState {
    pub fn new(tab: TopTab) -> Self {
        Self {
            active_tab: tab,
            sort_by: TopSortBy::Cpu,
            sort_ascending: false,
            selected_idx: 0,
            scroll_offset: 0,
            pods: Vec::new(),
            nodes: Vec::new(),
            filter: String::new(),
            active_port_forwards: HashMap::new(),
        }
    }

    pub fn extract_pod_resources(val: &serde_json::Value) -> (i64, i64, i64, i64) {
        let mut req_cpu = val.get("cpuReqMillicores").and_then(|v| v.as_i64()).unwrap_or(0);
        let mut lim_cpu = val.get("cpuLimMillicores").and_then(|v| v.as_i64()).unwrap_or(0);
        let mut req_mem = val.get("memReqMiB").and_then(|v| v.as_i64()).unwrap_or(0);
        let mut lim_mem = val.get("memLimMiB").and_then(|v| v.as_i64()).unwrap_or(0);

        if req_cpu == 0 && lim_cpu == 0 && req_mem == 0 && lim_mem == 0 {
            if let Some(conts) = val.pointer("/spec/containers").and_then(|v| v.as_array()) {
                for c in conts {
                    if let Some(c_req_cpu) = c.pointer("/resources/requests/cpu").and_then(|v| v.as_str()) {
                        req_cpu += srelens_kube::metrics::cpu_millicores(c_req_cpu);
                    }
                    if let Some(c_lim_cpu) = c.pointer("/resources/limits/cpu").and_then(|v| v.as_str()) {
                        lim_cpu += srelens_kube::metrics::cpu_millicores(c_lim_cpu);
                    }
                    if let Some(c_req_mem) = c.pointer("/resources/requests/memory").and_then(|v| v.as_str()) {
                        req_mem += srelens_kube::metrics::mem_mib(c_req_mem);
                    }
                    if let Some(c_lim_mem) = c.pointer("/resources/limits/memory").and_then(|v| v.as_str()) {
                        lim_mem += srelens_kube::metrics::mem_mib(c_lim_mem);
                    }
                }
            }
        }
        (req_cpu, lim_cpu, req_mem, lim_mem)
    }

    pub fn set_data(&mut self, pods: Vec<TopPodRow>, nodes: Vec<TopNodeRow>) {
        self.pods = pods;
        self.nodes = nodes;
        self.sort_current();
        self.clamp_selection();
    }

    pub fn toggle_tab(&mut self) {
        self.active_tab = match self.active_tab {
            TopTab::Pods => TopTab::Nodes,
            TopTab::Nodes => TopTab::Pods,
        };
        self.selected_idx = 0;
        self.scroll_offset = 0;
        self.sort_current();
    }

    pub fn set_tab(&mut self, tab: TopTab) {
        if self.active_tab != tab {
            self.active_tab = tab;
            self.selected_idx = 0;
            self.scroll_offset = 0;
            self.sort_current();
        }
    }

    pub fn set_sort_by(&mut self, sort: TopSortBy) {
        if self.sort_by == sort {
            self.sort_ascending = !self.sort_ascending;
        } else {
            self.sort_by = sort;
            self.sort_ascending = false;
        }
        self.sort_current();
    }

    pub fn toggle_sort_direction(&mut self) {
        self.sort_ascending = !self.sort_ascending;
        self.sort_current();
    }

    pub fn sort_current(&mut self) {
        let asc = self.sort_ascending;
        match self.active_tab {
            TopTab::Pods => {
                match self.sort_by {
                    TopSortBy::Cpu => {
                        self.pods.sort_by(|a, b| {
                            let cmp = a.cpu_millicores.cmp(&b.cpu_millicores);
                            if asc { cmp } else { cmp.reverse() }
                        });
                    }
                    TopSortBy::Memory => {
                        self.pods.sort_by(|a, b| {
                            let cmp = a.mem_mib.cmp(&b.mem_mib);
                            if asc { cmp } else { cmp.reverse() }
                        });
                    }
                }
            }
            TopTab::Nodes => {
                match self.sort_by {
                    TopSortBy::Cpu => {
                        self.nodes.sort_by(|a, b| {
                            let cmp = a.cpu_millicores.cmp(&b.cpu_millicores);
                            if asc { cmp } else { cmp.reverse() }
                        });
                    }
                    TopSortBy::Memory => {
                        self.nodes.sort_by(|a, b| {
                            let cmp = a.mem_mib.cmp(&b.mem_mib);
                            if asc { cmp } else { cmp.reverse() }
                        });
                    }
                }
            }
        }
    }

    pub fn visible_count(&self) -> usize {
        match self.active_tab {
            TopTab::Pods => self.filtered_pods().len(),
            TopTab::Nodes => self.filtered_nodes().len(),
        }
    }

    pub fn filtered_pods(&self) -> Vec<&TopPodRow> {
        let q = self.filter.to_lowercase();
        self.pods
            .iter()
            .filter(|p| {
                if q.is_empty() {
                    true
                } else {
                    p.name.to_lowercase().contains(&q) || p.namespace.to_lowercase().contains(&q)
                }
            })
            .collect()
    }

    pub fn filtered_nodes(&self) -> Vec<&TopNodeRow> {
        let q = self.filter.to_lowercase();
        self.nodes
            .iter()
            .filter(|n| {
                if q.is_empty() {
                    true
                } else {
                    n.name.to_lowercase().contains(&q)
                }
            })
            .collect()
    }

    pub fn selected_pod(&self) -> Option<&TopPodRow> {
        if self.active_tab != TopTab::Pods {
            return None;
        }
        let filtered = self.filtered_pods();
        filtered.get(self.selected_idx).copied()
    }

    pub fn selected_node(&self) -> Option<&TopNodeRow> {
        if self.active_tab != TopTab::Nodes {
            return None;
        }
        let filtered = self.filtered_nodes();
        filtered.get(self.selected_idx).copied()
    }

    pub fn clamp_selection(&mut self) {
        let count = self.visible_count();
        if count == 0 {
            self.selected_idx = 0;
            self.scroll_offset = 0;
        } else if self.selected_idx >= count {
            self.selected_idx = count - 1;
        }
    }

    pub fn select_next(&mut self) {
        let count = self.visible_count();
        if count > 0 && self.selected_idx + 1 < count {
            self.selected_idx += 1;
        }
    }

    pub fn select_prev(&mut self) {
        if self.selected_idx > 0 {
            self.selected_idx -= 1;
        }
    }

    pub fn select_first(&mut self) {
        self.selected_idx = 0;
    }

    pub fn select_last(&mut self) {
        let count = self.visible_count();
        if count > 0 {
            self.selected_idx = count - 1;
        }
    }

    pub fn scroll_page_down(&mut self, page_size: usize) {
        let count = self.visible_count();
        if count > 0 {
            self.selected_idx = (self.selected_idx + page_size).min(count - 1);
        }
    }

    pub fn scroll_page_up(&mut self, page_size: usize) {
        self.selected_idx = self.selected_idx.saturating_sub(page_size);
    }
}

pub fn format_cpu(m: i64) -> String {
    if m >= 1000 {
        format!("{:.2}c", m as f64 / 1000.0)
    } else {
        format!("{}m", m)
    }
}

pub fn format_mem(mib: i64) -> String {
    if mib >= 1024 {
        format!("{:.2}Gi", mib as f64 / 1024.0)
    } else {
        format!("{}Mi", mib)
    }
}

fn pct_style(pct: f64) -> Style {
    if pct >= 90.0 {
        Style::default().fg(Theme::red()).add_modifier(Modifier::BOLD)
    } else if pct >= 70.0 {
        Style::default().fg(Theme::yellow())
    } else {
        Style::default().fg(Theme::green())
    }
}

fn format_pct_span(opt_pct: Option<f64>) -> Span<'static> {
    match opt_pct {
        Some(pct) => Span::styled(format!("{:>4.0}%", pct), pct_style(pct)),
        None => Span::styled("   –", Style::default().fg(Theme::dim())),
    }
}

pub fn render_top_view(f: &mut Frame, area: Rect, state: &TopViewState) {
    f.render_widget(Clear, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Top header / tab bar
            Constraint::Min(5),    // Table body
        ])
        .split(area);

    // 1. Header with Tabs & Controls
    let sort_label = match state.sort_by {
        TopSortBy::Cpu => "CPU",
        TopSortBy::Memory => "MEM",
    };
    let dir_symbol = if state.sort_ascending { "▲" } else { "▼" };

    let pods_tab_style = if state.active_tab == TopTab::Pods {
        Style::default().fg(Theme::sel_fg()).bg(Theme::sel_bg()).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Theme::dim())
    };

    let nodes_tab_style = if state.active_tab == TopTab::Nodes {
        Style::default().fg(Theme::sel_fg()).bg(Theme::sel_bg()).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Theme::dim())
    };

    let filter_badge = if !state.filter.is_empty() {
        format!(" [Filter: \"{}\"]", state.filter)
    } else {
        String::new()
    };

    let count = state.visible_count();
    let header_line = Line::from(vec![
        Span::raw(" "),
        Span::styled(" [1] Pods ", pods_tab_style),
        Span::raw(" "),
        Span::styled(" [2] Nodes ", nodes_tab_style),
        Span::raw("  │  "),
        Span::styled(format!("Sort: {} {}", sort_label, dir_symbol), Style::default().fg(Theme::yellow()).add_modifier(Modifier::BOLD)),
        Span::styled(" (<c> CPU <m> MEM <S> Rev)  │  ", Style::default().fg(Theme::dim())),
        Span::styled(format!("Total: {} items{}", count, filter_badge), Style::default().fg(Theme::fg())),
        Span::styled("  │  <Tab> Toggle  <l> Logs  <d> Describe  <Esc> Back ", Style::default().fg(Theme::dim())),
    ]);

    let header_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::border_focus()))
        .title(Span::styled(" Top Hotspots (k9s style) ", Theme::title()));

    let inner_header = header_block.inner(chunks[0]);
    f.render_widget(header_block, chunks[0]);
    f.render_widget(ratatui::widgets::Paragraph::new(header_line), inner_header);

    // 2. Table Body
    let table_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::border()));

    let inner_table = table_block.inner(chunks[1]);
    f.render_widget(table_block, chunks[1]);

    let visible_rows = inner_table.height.saturating_sub(1) as usize; // minus 1 for header
    let sel = state.selected_idx;

    match state.active_tab {
        TopTab::Pods => {
            let pods = state.filtered_pods();
            if pods.is_empty() {
                let empty_msg = Line::from(vec![
                    Span::styled("No pod metrics available. (Ensure metrics-server is installed and pods are running)", Style::default().fg(Theme::dim())),
                ]);
                f.render_widget(ratatui::widgets::Paragraph::new(empty_msg), inner_table);
                return;
            }

            let start_idx = if sel >= visible_rows {
                sel.saturating_sub(visible_rows / 2).min(pods.len().saturating_sub(visible_rows))
            } else {
                0
            };
            let end_idx = (start_idx + visible_rows).min(pods.len());

            let header_style = Style::default().fg(Theme::yellow()).add_modifier(Modifier::BOLD);
            let header = Row::new(vec![
                Span::styled("NAMESPACE", header_style),
                Span::styled("NAME", header_style),
                Span::styled("CPU", header_style),
                Span::styled("% REQ", header_style),
                Span::styled("% LIM", header_style),
                Span::styled("MEMORY", header_style),
                Span::styled("% REQ", header_style),
                Span::styled("% LIM", header_style),
            ]).bottom_margin(0);

            let rows: Vec<Row> = pods[start_idx..end_idx]
                .iter()
                .enumerate()
                .map(|(i, pod)| {
                    let global_idx = start_idx + i;
                    let is_selected = global_idx == sel;

                    let row_style = if is_selected {
                        Theme::selected_row()
                    } else {
                        Style::default().bg(Theme::bg())
                    };

                    let name_cell = if let Some(forwards) = state.active_port_forwards.get(&(pod.namespace.clone(), pod.name.clone())) {
                        if !forwards.is_empty() {
                            let pf_str = forwards
                                .iter()
                                .map(|(loc, rem, _)| {
                                    if loc == rem {
                                        format!("{}", loc)
                                    } else {
                                        format!("{}→{}", loc, rem)
                                    }
                                })
                                .collect::<Vec<_>>()
                                .join(",");
                            Cell::from(Line::from(vec![
                                Span::styled(pod.name.clone(), Style::default().fg(Theme::fg())),
                                Span::raw(" "),
                                Span::styled(format!("[PF: {}]", pf_str), Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD)),
                            ]))
                        } else {
                            Cell::from(Span::styled(pod.name.clone(), Style::default().fg(Theme::fg())))
                        }
                    } else {
                        Cell::from(Span::styled(pod.name.clone(), Style::default().fg(Theme::fg())))
                    };

                    let cells = vec![
                        Cell::from(Span::styled(pod.namespace.clone(), Style::default().fg(Theme::dim()))),
                        name_cell,
                        Cell::from(Span::styled(format_cpu(pod.cpu_millicores), Style::default().fg(Theme::fg()))),
                        Cell::from(format_pct_span(pod.cpu_req_pct())),
                        Cell::from(format_pct_span(pod.cpu_lim_pct())),
                        Cell::from(Span::styled(format_mem(pod.mem_mib), Style::default().fg(Theme::fg()))),
                        Cell::from(format_pct_span(pod.mem_req_pct())),
                        Cell::from(format_pct_span(pod.mem_lim_pct())),
                    ];

                    Row::new(cells).style(row_style)
                })
                .collect();

            let widths = [
                Constraint::Length(18), // Namespace
                Constraint::Min(25),    // Name
                Constraint::Length(10), // CPU
                Constraint::Length(8),  // % REQ
                Constraint::Length(8),  // % LIM
                Constraint::Length(10), // Memory
                Constraint::Length(8),  // % REQ
                Constraint::Length(8),  // % LIM
            ];

            let table = Table::new(rows, widths).header(header);
            f.render_widget(table, inner_table);
        }
        TopTab::Nodes => {
            let nodes = state.filtered_nodes();
            if nodes.is_empty() {
                let empty_msg = Line::from(vec![
                    Span::styled("No node metrics available. (Ensure metrics-server is installed and nodes are ready)", Style::default().fg(Theme::dim())),
                ]);
                f.render_widget(ratatui::widgets::Paragraph::new(empty_msg), inner_table);
                return;
            }

            let start_idx = if sel >= visible_rows {
                sel.saturating_sub(visible_rows / 2).min(nodes.len().saturating_sub(visible_rows))
            } else {
                0
            };
            let end_idx = (start_idx + visible_rows).min(nodes.len());

            let header_style = Style::default().fg(Theme::yellow()).add_modifier(Modifier::BOLD);
            let header = Row::new(vec![
                Span::styled("NAME", header_style),
                Span::styled("STATUS", header_style),
                Span::styled("CPU USAGE", header_style),
                Span::styled("CPU ALLOC", header_style),
                Span::styled("% CPU", header_style),
                Span::styled("MEM USAGE", header_style),
                Span::styled("MEM ALLOC", header_style),
                Span::styled("% MEM", header_style),
            ]).bottom_margin(0);

            let rows: Vec<Row> = nodes[start_idx..end_idx]
                .iter()
                .enumerate()
                .map(|(i, node)| {
                    let global_idx = start_idx + i;
                    let is_selected = global_idx == sel;

                    let row_style = if is_selected {
                        Theme::selected_row()
                    } else {
                        Style::default().bg(Theme::bg())
                    };

                    let status_style = if node.status.eq_ignore_ascii_case("Ready") {
                        Style::default().fg(Theme::green())
                    } else {
                        Style::default().fg(Theme::red())
                    };

                    let cells = vec![
                        Span::styled(node.name.clone(), Style::default().fg(Theme::fg())),
                        Span::styled(node.status.clone(), status_style),
                        Span::styled(format_cpu(node.cpu_millicores), Style::default().fg(Theme::fg())),
                        Span::styled(format_cpu(node.cpu_alloc_millicores), Style::default().fg(Theme::dim())),
                        format_pct_span(node.cpu_alloc_pct()),
                        Span::styled(format_mem(node.mem_mib), Style::default().fg(Theme::fg())),
                        Span::styled(format_mem(node.mem_alloc_mib), Style::default().fg(Theme::dim())),
                        format_pct_span(node.mem_alloc_pct()),
                    ];

                    Row::new(cells).style(row_style)
                })
                .collect();

            let widths = [
                Constraint::Min(25),    // Name
                Constraint::Length(12), // Status
                Constraint::Length(12), // CPU Usage
                Constraint::Length(12), // CPU Alloc
                Constraint::Length(8),  // % CPU
                Constraint::Length(12), // MEM Usage
                Constraint::Length(12), // MEM Alloc
                Constraint::Length(8),  // % MEM
            ];

            let table = Table::new(rows, widths).header(header);
            f.render_widget(table, inner_table);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn test_top_pod_calculations_and_sorting() {
        let mut state = TopViewState::new(TopTab::Pods);
        let pods = vec![
            TopPodRow {
                namespace: "default".to_string(),
                name: "api-gateway".to_string(),
                cpu_millicores: 800,
                cpu_req_millicores: 1000,
                cpu_lim_millicores: 2000,
                mem_mib: 1024,
                mem_req_mib: 2048,
                mem_lim_mib: 4096,
            },
            TopPodRow {
                namespace: "default".to_string(),
                name: "heavy-worker".to_string(),
                cpu_millicores: 1600,
                cpu_req_millicores: 1000,
                cpu_lim_millicores: 2000,
                mem_mib: 3000,
                mem_req_mib: 2048,
                mem_lim_mib: 4096,
            },
        ];

        state.set_data(pods, vec![]);
        // Default sort is CPU descending
        assert_eq!(state.filtered_pods()[0].name, "heavy-worker");
        assert_eq!(state.filtered_pods()[1].name, "api-gateway");

        // Percentage calculations
        let top = state.filtered_pods()[0];
        assert_eq!(top.cpu_req_pct(), Some(160.0));
        assert_eq!(top.cpu_lim_pct(), Some(80.0));

        // Switch sort to Memory
        state.set_sort_by(TopSortBy::Memory);
        assert_eq!(state.filtered_pods()[0].name, "heavy-worker");

        // Reverse sort
        state.toggle_sort_direction();
        assert_eq!(state.filtered_pods()[0].name, "api-gateway");
    }

    #[test]
    fn test_top_nodes_rendering() {
        let mut state = TopViewState::new(TopTab::Nodes);
        let nodes = vec![
            TopNodeRow {
                name: "node-worker-1".to_string(),
                status: "Ready".to_string(),
                cpu_millicores: 3200,
                cpu_alloc_millicores: 4000,
                mem_mib: 12000,
                mem_alloc_mib: 16000,
            },
        ];
        state.set_data(vec![], nodes);

        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render_top_view(f, area, &state);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let rendered: String = (0..buffer.area.height)
            .flat_map(|y| {
                let mut line = String::new();
                for x in 0..buffer.area.width {
                    line.push_str(buffer[(x, y)].symbol());
                }
                line.push('\n');
                line.into_bytes()
            })
            .map(|b| b as char)
            .collect();

        assert!(rendered.contains("Top Hotspots"));
        assert!(rendered.contains("node-worker-1"));
        assert!(rendered.contains("Ready"));
        assert!(rendered.contains("3.20c") || rendered.contains("3200m"));
    }
}
