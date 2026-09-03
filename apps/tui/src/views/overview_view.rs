use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph, Row, Table, Cell},
    Frame,
};

use crate::theme::Theme;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
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
    pub total_gpus: usize,
    pub allocated_gpus: usize,
    pub total_gpu_mem_mib: i64,
    pub used_gpu_mem_mib: i64,
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

    pub fn with_data(data: ClusterOverviewData) -> Self {
        Self { data }
    }

    pub fn set_data(&mut self, data: ClusterOverviewData) {
        self.data = data;
    }
}

pub fn render_overview_view(f: &mut Frame, area: Rect, state: &OverviewViewState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::BORDER))
        .title(Span::styled(" Cluster Health & Resource Overview [<r> Refresh, <Esc> Back] ", Theme::title()));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let d = &state.data;
    let has_gpus = d.total_gpus > 0;
    let gauges_height = if has_gpus { 10 } else { 7 };

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6),            // Cluster info cards
            Constraint::Length(gauges_height), // Resource allocation gauges (CPU, Mem, [GPU])
            Constraint::Min(8),               // Workload health distribution
        ])
        .split(inner);

    // 1. Cluster info table
    let status_span = if d.is_reachable {
        Span::styled("● REACHABLE / HEALTHY", Theme::status_ok())
    } else {
        Span::styled("● UNREACHABLE", Theme::status_error())
    };

    let k8s_version_display = if d.k8s_version.is_empty() {
        "Connecting...".to_string()
    } else if d.k8s_version.starts_with('v') {
        d.k8s_version.clone()
    } else {
        format!("v{}", d.k8s_version)
    };

    let ready_nodes_display = if d.ready_nodes > 0 || d.node_count > 0 {
        format!("{}/{} Ready", d.ready_nodes, d.node_count)
    } else {
        "Fetching...".to_string()
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
                Cell::from(k8s_version_display).style(Theme::header_val()),
            ]),
            Row::new(vec![
                Cell::from("Server:").style(Theme::header_label()),
                Cell::from(d.server_url.as_str()).style(Theme::header_val()),
                Cell::from("Nodes:").style(Theme::header_label()),
                Cell::from(ready_nodes_display).style(Theme::header_val()),
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

    // 2. Resource gauges (CPU, Memory, optional GPU)
    let gauge_constraints = if has_gpus {
        vec![Constraint::Length(3), Constraint::Length(3), Constraint::Length(3)]
    } else {
        vec![Constraint::Length(3), Constraint::Length(3)]
    };

    let gauge_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(gauge_constraints)
        .split(rows[1]);

    let cpu_pct = if d.total_cpu_millicores > 0 {
        ((d.used_cpu_millicores as f64 / d.total_cpu_millicores as f64) * 100.0) as u16
    } else {
        0
    };

    let cpu_title = if d.total_cpu_millicores >= 1000 {
        format!(
            " CPU Allocation: {:.1} / {:.1} Cores ({}%) ",
            d.used_cpu_millicores as f64 / 1000.0,
            d.total_cpu_millicores as f64 / 1000.0,
            cpu_pct
        )
    } else {
        format!(
            " CPU Allocation: {}m / {}m ({}%) ",
            d.used_cpu_millicores, d.total_cpu_millicores, cpu_pct
        )
    };

    let cpu_gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title(cpu_title))
        .gauge_style(Style::default().fg(if cpu_pct > 85 { Theme::RED } else if cpu_pct > 70 { Theme::YELLOW } else { Theme::CYAN }))
        .percent(cpu_pct.min(100));
    f.render_widget(cpu_gauge, gauge_layout[0]);

    let mem_pct = if d.total_mem_mib > 0 {
        ((d.used_mem_mib as f64 / d.total_mem_mib as f64) * 100.0) as u16
    } else {
        0
    };

    let mem_title = if d.total_mem_mib >= 1024 {
        format!(
            " Memory Allocation: {:.1} / {:.1} GiB ({}%) ",
            d.used_mem_mib as f64 / 1024.0,
            d.total_mem_mib as f64 / 1024.0,
            mem_pct
        )
    } else {
        format!(
            " Memory Allocation: {}MiB / {}MiB ({}%) ",
            d.used_mem_mib, d.total_mem_mib, mem_pct
        )
    };

    let mem_gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title(mem_title))
        .gauge_style(Style::default().fg(if mem_pct > 85 { Theme::RED } else if mem_pct > 70 { Theme::YELLOW } else { Theme::ACCENT }))
        .percent(mem_pct.min(100));
    f.render_widget(mem_gauge, gauge_layout[1]);

    if has_gpus && gauge_layout.len() > 2 {
        let (gpu_title, gpu_pct) = if d.total_gpus > 0 {
            let pct = (((d.allocated_gpus as f64 / d.total_gpus as f64) * 100.0).round() as u32).min(100) as u16;
            let title = if d.used_gpu_mem_mib > 0 {
                let vram_gib = d.used_gpu_mem_mib as f64 / 1024.0;
                if d.total_gpu_mem_mib > 0 {
                    let total_vram = d.total_gpu_mem_mib as f64 / 1024.0;
                    let vram_pct = (((vram_gib / total_vram) * 100.0).round() as u32).min(100);
                    format!(
                        " GPU Allocation: {} / {} GPUs ({}%)  •  VRAM: {:.1} / {:.1} GiB ({}%) ",
                        d.allocated_gpus, d.total_gpus, pct, vram_gib, total_vram, vram_pct
                    )
                } else {
                    format!(
                        " GPU Allocation: {} / {} GPUs ({}%)  •  VRAM Allocated: {:.1} GiB ",
                        d.allocated_gpus, d.total_gpus, pct, vram_gib
                    )
                }
            } else {
                format!(
                    " GPU Allocation: {} / {} GPUs ({}%) ",
                    d.allocated_gpus, d.total_gpus, pct
                )
            };
            (title, pct)
        } else if d.total_gpu_mem_mib > 0 {
            let pct = (((d.used_gpu_mem_mib as f64 / d.total_gpu_mem_mib as f64) * 100.0).round() as u32).min(100) as u16;
            (
                format!(
                    " GPU VRAM Allocation: {:.1} / {:.1} GiB ({}%) ",
                    d.used_gpu_mem_mib as f64 / 1024.0,
                    d.total_gpu_mem_mib as f64 / 1024.0,
                    pct,
                ),
                pct,
            )
        } else {
            (
                format!(" GPU Allocation: {:.1} GiB VRAM allocated ", d.used_gpu_mem_mib as f64 / 1024.0),
                0,
            )
        };

        let gpu_gauge = Gauge::default()
            .block(Block::default().borders(Borders::ALL).title(gpu_title))
            .gauge_style(Style::default().fg(Theme::ACCENT))
            .percent(gpu_pct.min(100));
        f.render_widget(gpu_gauge, gauge_layout[2]);
    }

    // 3. Pods distribution
    let pod_dist_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::BORDER))
        .title(" Workload Health Distribution ");
    let pod_dist_inner = pod_dist_block.inner(rows[2]);
    f.render_widget(pod_dist_block, rows[2]);

    let mut dist_lines = vec![
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
            if d.pending_pods > 0 {
                Span::styled("  (Unschedulable or waiting for resources/PVC)", Style::default().fg(Theme::DIM))
            } else {
                Span::raw("")
            },
        ]),
        Line::from(vec![
            Span::styled("● Failed/Crash: ", Theme::status_error()),
            Span::styled(format!("{}", d.failed_pods), Theme::status_error()),
            if d.failed_pods > 0 {
                Span::styled("  (OOMKilled, CrashLoopBackOff, or Error)", Style::default().fg(Theme::DIM))
            } else {
                Span::raw("")
            },
        ]),
    ];

    if d.total_gpus > 0 || d.used_gpu_mem_mib > 0 {
        let vram_detail = if d.used_gpu_mem_mib > 0 {
            if d.total_gpu_mem_mib > 0 {
                format!("  [VRAM: {:.1} / {:.1} GiB allocated]", d.used_gpu_mem_mib as f64 / 1024.0, d.total_gpu_mem_mib as f64 / 1024.0)
            } else {
                format!("  [VRAM: {:.1} GiB allocated]", d.used_gpu_mem_mib as f64 / 1024.0)
            }
        } else {
            String::new()
        };

        dist_lines.push(Line::from(vec![
            Span::styled("GPU Slices:     ", Theme::header_label()),
            Span::styled(format!("{}/{} Physical GPUs", d.allocated_gpus, d.total_gpus), Style::default().fg(Theme::ACCENT)),
            Span::styled(vram_detail, Style::default().fg(Theme::FG)),
        ]));
    }

    f.render_widget(Paragraph::new(dist_lines), pod_dist_inner);
}
