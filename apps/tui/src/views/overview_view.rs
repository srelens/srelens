use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph, Row, Table, Cell},
    Frame,
};

use crate::theme::Theme;

#[derive(Debug, Clone, Default)]
pub struct ClusterOverviewData {
    pub context_name: String,
    pub cluster_name: String,
    pub server_url: String,
    pub k8s_version: String,
    pub is_reachable: bool,
    pub node_count: usize,
    pub ready_nodes: usize,
    pub total_pods: usize,
    pub running_pods: usize,
    pub pending_pods: usize,
    pub failed_pods: usize,
    pub total_cpu_millicores: i64,
    pub used_cpu_millicores: i64,
    pub total_mem_mib: i64,
    pub used_mem_mib: i64,
}

pub struct OverviewViewState {
    pub data: ClusterOverviewData,
}

impl OverviewViewState {
    pub fn new() -> Self {
        Self {
            data: ClusterOverviewData::default(),
        }
    }

    pub fn set_data(&mut self, data: ClusterOverviewData) {
        self.data = data;
    }
}

pub fn render_overview_view(f: &mut Frame, area: Rect, state: &OverviewViewState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::BORDER))
        .title(Span::styled(" Cluster Health & Resource Overview (<Esc> Back) ", Theme::title()));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6), // Cluster info cards
            Constraint::Length(8), // Resource allocation gauges (CPU, Mem, Pods)
            Constraint::Min(10),   // Workload health distribution
        ])
        .split(inner);

    // 1. Cluster info table
    let d = &state.data;
    let status_span = if d.is_reachable {
        Span::styled("● REACHABLE / HEALTHY", Theme::status_ok())
    } else {
        Span::styled("● UNREACHABLE", Theme::status_error())
    };

    let info_table = Table::new(
        vec![
            Row::new(vec![
                Cell::from("Context:").style(Theme::header_label()),
                Cell::from(d.context_name.as_str()).style(Theme::header_val()),
                Cell::from("Status:").style(Theme::header_label()),
                Cell::from(status_span),
            ]),
            Row::new(vec![
                Cell::from("Cluster:").style(Theme::header_label()),
                Cell::from(d.cluster_name.as_str()).style(Theme::header_val()),
                Cell::from("K8s Version:").style(Theme::header_label()),
                Cell::from(format!("v{}", d.k8s_version)).style(Theme::header_val()),
            ]),
            Row::new(vec![
                Cell::from("Server:").style(Theme::header_label()),
                Cell::from(d.server_url.as_str()).style(Theme::header_val()),
                Cell::from("Nodes:").style(Theme::header_label()),
                Cell::from(format!("{}/{} Ready", d.ready_nodes, d.node_count)).style(Theme::header_val()),
            ]),
        ],
        [
            Constraint::Length(12),
            Constraint::Min(30),
            Constraint::Length(14),
            Constraint::Min(25),
        ],
    );
    f.render_widget(info_table, rows[0]);

    // 2. Resource gauges (CPU & Memory)
    let gauge_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
        ])
        .split(rows[1]);

    let cpu_pct = if d.total_cpu_millicores > 0 {
        ((d.used_cpu_millicores as f64 / d.total_cpu_millicores as f64) * 100.0) as u16
    } else {
        0
    };

    let cpu_gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title(format!(" CPU Allocation: {}m / {}m ({}%) ", d.used_cpu_millicores, d.total_cpu_millicores, cpu_pct)))
        .gauge_style(Style::default().fg(if cpu_pct > 80 { Theme::RED } else { Theme::CYAN }))
        .percent(cpu_pct.min(100));
    f.render_widget(cpu_gauge, gauge_layout[0]);

    let mem_pct = if d.total_mem_mib > 0 {
        ((d.used_mem_mib as f64 / d.total_mem_mib as f64) * 100.0) as u16
    } else {
        0
    };

    let mem_gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title(format!(" Memory Allocation: {}MiB / {}MiB ({}%) ", d.used_mem_mib, d.total_mem_mib, mem_pct)))
        .gauge_style(Style::default().fg(if mem_pct > 80 { Theme::RED } else { Theme::ACCENT }))
        .percent(mem_pct.min(100));
    f.render_widget(mem_gauge, gauge_layout[1]);

    // 3. Pods distribution
    let pod_dist_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::BORDER))
        .title(" Pod Health Distribution ");
    let pod_dist_inner = pod_dist_block.inner(rows[2]);
    f.render_widget(pod_dist_block, rows[2]);

    let dist_lines = vec![
        Line::from(vec![
            Span::styled("Total Pods:     ", Theme::header_label()),
            Span::styled(format!("{}", d.total_pods), Theme::header_val()),
        ]),
        Line::from(vec![
            Span::styled("● Running:      ", Theme::status_ok()),
            Span::styled(format!("{}", d.running_pods), Theme::status_ok()),
        ]),
        Line::from(vec![
            Span::styled("● Pending:      ", Theme::status_warn()),
            Span::styled(format!("{}", d.pending_pods), Theme::status_warn()),
        ]),
        Line::from(vec![
            Span::styled("● Failed/Crash: ", Theme::status_error()),
            Span::styled(format!("{}", d.failed_pods), Theme::status_error()),
        ]),
    ];
    f.render_widget(Paragraph::new(dist_lines), pod_dist_inner);
}
