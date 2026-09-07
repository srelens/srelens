use std::cell::Cell;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Gauge, Paragraph};
use ratatui::Frame;

use srelens_kube::gpu_info::{format_vram_mib, GpuClusterInfo, GpuNodeInfo, GpuPodItem};
use crate::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuPane {
    Nodes,
    Pods,
}

#[derive(Debug, Clone)]
pub struct GpuViewState {
    pub cluster_info: Option<GpuClusterInfo>,
    pub selected_node_idx: usize,
    pub selected_pod_idx: usize,
    pub focused_pane: GpuPane,
    pub is_loading: bool,
    pub error: Option<String>,
    pub last_left_pane_rect: Cell<Rect>,
    pub last_pods_pane_rect: Cell<Rect>,
    pub last_nodes_start_idx: Cell<usize>,
    pub last_pods_start_idx: Cell<usize>,
}

impl Default for GpuViewState {
    fn default() -> Self {
        Self::new()
    }
}

impl GpuViewState {
    pub fn new() -> Self {
        Self {
            cluster_info: None,
            selected_node_idx: 0,
            selected_pod_idx: 0,
            focused_pane: GpuPane::Nodes,
            is_loading: true,
            error: None,
            last_left_pane_rect: Cell::new(Rect::default()),
            last_pods_pane_rect: Cell::new(Rect::default()),
            last_nodes_start_idx: Cell::new(0),
            last_pods_start_idx: Cell::new(0),
        }
    }

    pub fn set_info(&mut self, info: GpuClusterInfo) {
        self.is_loading = false;
        self.error = None;
        if self.selected_node_idx >= info.nodes.len() && !info.nodes.is_empty() {
            self.selected_node_idx = info.nodes.len() - 1;
        }
        self.clamp_pod_selection(&info);
        self.cluster_info = Some(info);
    }

    pub fn set_error(&mut self, err: String) {
        self.is_loading = false;
        self.error = Some(err);
    }

    fn clamp_pod_selection(&mut self, info: &GpuClusterInfo) {
        if let Some(node) = info.nodes.get(self.selected_node_idx) {
            if self.selected_pod_idx >= node.pods.len() && !node.pods.is_empty() {
                self.selected_pod_idx = node.pods.len() - 1;
            } else if node.pods.is_empty() {
                self.selected_pod_idx = 0;
            }
        } else {
            self.selected_pod_idx = 0;
        }
    }

    pub fn selected_node(&self) -> Option<&GpuNodeInfo> {
        self.cluster_info
            .as_ref()
            .and_then(|ci| ci.nodes.get(self.selected_node_idx))
    }

    pub fn selected_pod(&self) -> Option<&GpuPodItem> {
        self.selected_node().and_then(|n| n.pods.get(self.selected_pod_idx))
    }

    pub fn select_next_node(&mut self) {
        if let Some(ci) = &self.cluster_info {
            if !ci.nodes.is_empty() && self.selected_node_idx + 1 < ci.nodes.len() {
                self.selected_node_idx += 1;
                self.selected_pod_idx = 0;
            }
        }
    }

    pub fn select_prev_node(&mut self) {
        if self.selected_node_idx > 0 {
            self.selected_node_idx -= 1;
            self.selected_pod_idx = 0;
        }
    }

    pub fn select_first_node(&mut self) {
        self.selected_node_idx = 0;
        self.selected_pod_idx = 0;
    }

    pub fn select_last_node(&mut self) {
        if let Some(ci) = &self.cluster_info {
            if !ci.nodes.is_empty() {
                self.selected_node_idx = ci.nodes.len() - 1;
                self.selected_pod_idx = 0;
            }
        }
    }

    pub fn select_next_pod(&mut self) {
        if let Some(node) = self.selected_node() {
            if !node.pods.is_empty() && self.selected_pod_idx + 1 < node.pods.len() {
                self.selected_pod_idx += 1;
            }
        }
    }

    pub fn select_prev_pod(&mut self) {
        if self.selected_pod_idx > 0 {
            self.selected_pod_idx -= 1;
        }
    }

    pub fn select_first_pod(&mut self) {
        self.selected_pod_idx = 0;
    }

    pub fn select_last_pod(&mut self) {
        if let Some(node) = self.selected_node() {
            if !node.pods.is_empty() {
                self.selected_pod_idx = node.pods.len() - 1;
            }
        }
    }

    pub fn toggle_pane(&mut self) {
        self.focused_pane = match self.focused_pane {
            GpuPane::Nodes => GpuPane::Pods,
            GpuPane::Pods => GpuPane::Nodes,
        };
    }

    pub fn set_node_by_index(&mut self, idx: usize) {
        if let Some(ci) = &self.cluster_info {
            if idx < ci.nodes.len() {
                self.selected_node_idx = idx;
                self.selected_pod_idx = 0;
            }
        }
    }

    pub fn set_pod_by_index(&mut self, idx: usize) {
        if let Some(node) = self.selected_node() {
            if idx < node.pods.len() {
                self.selected_pod_idx = idx;
            }
        }
    }
}

pub fn render(f: &mut Frame, area: Rect, state: &GpuViewState) {
    // Dynamic width for left pane based on the longest node name
    let max_node_name_len = state
        .cluster_info
        .as_ref()
        .map(|ci| {
            ci.nodes
                .iter()
                .map(|n| n.name.len())
                .max()
                .unwrap_or(18)
        })
        .unwrap_or(18)
        .max("NODE".len());

    // Left pane needs:
    // border (2) + prefix (1) + node_name (max_node_name_len) + space (1) +
    // status (10) + gpus (8) + vram (8) = max_node_name_len + 30
    let needed_left_width = (max_node_name_len + 30) as u16;
    let left_width = if area.width > 90 {
        // Reserve at least 48 cols for right details pane if space allows
        needed_left_width.min(area.width.saturating_sub(48)).max(44)
    } else {
        needed_left_width.min(area.width.saturating_sub(30)).max(36)
    };

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(left_width), // Left pane: GPU nodes list (dynamically sized)
            Constraint::Min(40),            // Right pane: Node metrics & GPU Pods
        ])
        .split(area);

    state.last_left_pane_rect.set(chunks[0]);

    render_nodes_list(f, chunks[0], state, max_node_name_len);
    render_details_pane(f, chunks[1], state);
}

fn render_nodes_list(f: &mut Frame, area: Rect, state: &GpuViewState, max_node_name_len: usize) {
    let is_focused = state.focused_pane == GpuPane::Nodes;
    let node_count = state.cluster_info.as_ref().map(|ci| ci.nodes.len()).unwrap_or(0);

    let border_color = if is_focused {
        Theme::ACCENT
    } else {
        Theme::BORDER
    };

    let title_style = if is_focused {
        Style::default().fg(Theme::ACCENT).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Theme::fg()).add_modifier(Modifier::BOLD)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(format!(" ⚡ GPU NODES ({}) ", node_count), title_style));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if state.is_loading {
        let p = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled("  ⚡ Querying cluster GPU nodes...", Style::default().fg(Theme::CYAN))),
        ]);
        f.render_widget(p, inner);
        return;
    }

    if let Some(err) = &state.error {
        let p = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(format!("  ✖ Error: {}", err), Style::default().fg(Theme::RED))),
            Line::from(Span::styled("  Press 'r' to retry.", Style::default().fg(Theme::DIM))),
        ]);
        f.render_widget(p, inner);
        return;
    }

    let nodes = state.cluster_info.as_ref().map(|ci| &ci.nodes[..]).unwrap_or(&[]);
    if nodes.is_empty() {
        let p = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled("  ⊘ No GPU nodes detected.", Style::default().fg(Theme::YELLOW))),
            Line::from(""),
            Line::from(Span::styled("  No nodes in this cluster report", Style::default().fg(Theme::DIM))),
            Line::from(Span::styled("  nvidia.com/gpu, amd.com/gpu, or", Style::default().fg(Theme::DIM))),
            Line::from(Span::styled("  accelerator device labels.", Style::default().fg(Theme::DIM))),
        ]);
        f.render_widget(p, inner);
        return;
    }

    // Available rows for list items
    let visible_rows = inner.height.saturating_sub(2) as usize;
    if visible_rows == 0 {
        return;
    }

    let selected = state.selected_node_idx;
    let mut start_idx = state.last_nodes_start_idx.get();
    if selected < start_idx {
        start_idx = selected;
    } else if selected >= start_idx + visible_rows {
        start_idx = selected + 1 - visible_rows;
    }
    state.last_nodes_start_idx.set(start_idx);

    let mut lines = Vec::new();

    let available_node_col = (inner.width as usize).saturating_sub(28);
    let node_col_width = max_node_name_len.min(available_node_col).max(12);

    // Header row
    lines.push(Line::from(vec![
        Span::styled(format!(" {:<width$} ", "NODE", width = node_col_width), Theme::table_header()),
        Span::styled(format!("{:<9} ", "STATUS"), Theme::table_header()),
        Span::styled(format!("{:<7} ", "GPUS"), Theme::table_header()),
        Span::styled(format!("{:<8}", "VRAM"), Theme::table_header()),
    ]));
    lines.push(Line::from(Span::styled("─".repeat(inner.width as usize), Style::default().fg(Theme::BORDER))));

    let end_idx = (start_idx + visible_rows).min(nodes.len());
    for i in start_idx..end_idx {
        let node = &nodes[i];
        let is_sel = i == selected;

        let status_span = if node.unschedulable {
            Span::styled("⊘ Cordon ", Style::default().fg(Theme::YELLOW))
        } else if node.status == "Ready" {
            Span::styled("● Ready  ", Style::default().fg(Theme::GREEN))
        } else {
            Span::styled("✖ NotRdy ", Style::default().fg(Theme::RED))
        };

        let gpus_str = format!("{}/{}", node.gpu_requests, node.gpu_capacity);
        let vram_str = if let Some(tot) = node.vram_capacity_total_mib {
            format!("{}/{}G", node.vram_requests_total_mib / 1024, tot / 1024)
        } else {
            "-".to_string()
        };

        let row_style = if is_sel {
            if is_focused {
                Style::default().bg(Theme::SEL_BG).fg(Theme::SEL_FG).add_modifier(Modifier::BOLD)
            } else {
                Style::default().bg(Color::Rgb(30, 41, 59)).fg(Theme::SEL_FG).add_modifier(Modifier::BOLD)
            }
        } else {
            Style::default().fg(Theme::fg())
        };

        let prefix = if is_sel { ">" } else { " " };
        let trunc_name = if node.name.len() > node_col_width {
            format!("{}…", &node.name[..node_col_width.saturating_sub(1)])
        } else {
            node.name.clone()
        };

        lines.push(Line::from(vec![
            Span::styled(format!("{}{:<width$} ", prefix, trunc_name, width = node_col_width), row_style),
            status_span,
            Span::styled(format!(" {:<6} ", gpus_str), row_style),
            Span::styled(format!("{:<8}", vram_str), row_style),
        ]));
    }

    let p = Paragraph::new(lines);
    f.render_widget(p, inner);
}

fn render_details_pane(f: &mut Frame, area: Rect, state: &GpuViewState) {
    let node_opt = state.selected_node();
    if node_opt.is_none() {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Theme::BORDER))
            .title(Span::styled(" GPU CONSUMPTION & WORKLOADS ", Style::default().fg(Theme::BORDER)));
        let inner = block.inner(area);
        f.render_widget(block, area);

        let p = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled("  Select a GPU node on the left to inspect its GPU & VRAM allocation.", Style::default().fg(Theme::DIM))),
        ]);
        f.render_widget(p, inner);
        return;
    }

    let node = node_opt.unwrap();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(8), // Top: Node GPU metrics & allocation gauges
            Constraint::Min(6),    // Bottom: Pods requesting GPU table
        ])
        .split(area);

    render_node_gpu_summary(f, chunks[0], node);
    render_node_pods_table(f, chunks[1], state, node);
}

fn render_node_gpu_summary(f: &mut Frame, area: Rect, node: &GpuNodeInfo) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::BORDER))
        .title(Span::styled(
            format!(" 🖥️  NODE GPU CONSUMPTION: {} ", node.name),
            Style::default().fg(Theme::ACCENT).add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(area);
    f.render_widget(block, area);

    // Inner layout: Line 1 info, Row 2/3 Gauges, Row 4 summary
    let inner_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Hardware details
            Constraint::Length(1), // Spacing
            Constraint::Length(1), // GPU count gauge
            Constraint::Length(1), // VRAM gauge
            Constraint::Length(1), // Summary footer
            Constraint::Min(0),
        ])
        .split(inner);

    // 1. Hardware details line
    let model = node.gpu_model.as_deref().unwrap_or("Unknown GPU");
    let driver = node.gpu_driver_version.as_deref().unwrap_or("-");
    let cuda = node.gpu_cuda_version.as_deref().unwrap_or("-");
    let itype = &node.instance_type;

    let info_line = Line::from(vec![
        Span::styled("Model: ", Theme::header_label()),
        Span::styled(format!("{}  ", model), Theme::header_val()),
        Span::styled("Driver: ", Theme::header_label()),
        Span::styled(format!("{}  ", driver), Theme::header_val()),
        Span::styled("CUDA: ", Theme::header_label()),
        Span::styled(format!("{}  ", cuda), Theme::header_val()),
        Span::styled("Instance: ", Theme::header_label()),
        Span::styled(itype, Theme::header_val()),
    ]);
    f.render_widget(Paragraph::new(info_line), inner_chunks[0]);

    // 2. GPU Count Allocation Gauge
    let gpu_cap = node.gpu_capacity.max(1);
    let gpu_pct = ((node.gpu_requests as f64 / gpu_cap as f64) * 100.0).round() as u16;
    let gpu_color = if gpu_pct >= 90 {
        Theme::RED
    } else if gpu_pct >= 70 {
        Theme::YELLOW
    } else {
        Theme::GREEN
    };

    let gpu_gauge_label = format!("GPUs Allocated: {} / {} ({:.0}%)", node.gpu_requests, node.gpu_capacity, gpu_pct);
    let gpu_gauge = Gauge::default()
        .gauge_style(Style::default().fg(gpu_color))
        .label(Span::styled(gpu_gauge_label, Style::default().fg(Theme::SEL_FG).add_modifier(Modifier::BOLD)))
        .percent(gpu_pct.min(100));
    f.render_widget(gpu_gauge, inner_chunks[2]);

    // 3. VRAM Allocation Gauge
    if let Some(tot_vram) = node.vram_capacity_total_mib {
        let vram_cap = tot_vram.max(1);
        let vram_pct = ((node.vram_requests_total_mib as f64 / vram_cap as f64) * 100.0).round() as u16;
        let vram_color = if vram_pct >= 90 {
            Theme::RED
        } else if vram_pct >= 70 {
            Theme::YELLOW
        } else {
            Theme::CYAN
        };

        let vram_gauge_label = format!(
            "VRAM Allocated: {} / {} ({:.1}%)",
            format_vram_mib(node.vram_requests_total_mib),
            format_vram_mib(tot_vram),
            (node.vram_requests_total_mib as f64 / vram_cap as f64) * 100.0
        );

        let vram_gauge = Gauge::default()
            .gauge_style(Style::default().fg(vram_color))
            .label(Span::styled(vram_gauge_label, Style::default().fg(Theme::SEL_FG).add_modifier(Modifier::BOLD)))
            .percent(vram_pct.min(100));
        f.render_widget(vram_gauge, inner_chunks[3]);
    } else {
        let vram_label = Line::from(Span::styled(
            format!("VRAM Requested: {} (Total capacity not reported by node labels)", format_vram_mib(node.vram_requests_total_mib)),
            Style::default().fg(Theme::YELLOW),
        ));
        f.render_widget(Paragraph::new(vram_label), inner_chunks[3]);
    }

    // 4. Summary footer stats
    let free_gpus = node.gpu_capacity.saturating_sub(node.gpu_requests);
    let free_vram_str = if let Some(tot) = node.vram_capacity_total_mib {
        format_vram_mib(tot.saturating_sub(node.vram_requests_total_mib))
    } else {
        "-".to_string()
    };

    let summary_line = Line::from(vec![
        Span::styled("Pods on GPU: ", Theme::header_label()),
        Span::styled(format!("{}   ", node.pods.len()), Theme::header_val()),
        Span::styled("Available GPUs: ", Theme::header_label()),
        Span::styled(format!("{}   ", free_gpus), Style::default().fg(Theme::GREEN).add_modifier(Modifier::BOLD)),
        Span::styled("Available VRAM: ", Theme::header_label()),
        Span::styled(free_vram_str, Style::default().fg(Theme::CYAN).add_modifier(Modifier::BOLD)),
    ]);
    f.render_widget(Paragraph::new(summary_line), inner_chunks[4]);
}

fn render_node_pods_table(f: &mut Frame, area: Rect, state: &GpuViewState, node: &GpuNodeInfo) {
    let is_focused = state.focused_pane == GpuPane::Pods;
    let border_color = if is_focused {
        Theme::ACCENT
    } else {
        Theme::BORDER
    };

    let title_style = if is_focused {
        Style::default().fg(Theme::ACCENT).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Theme::fg()).add_modifier(Modifier::BOLD)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(
            format!(" 📦 PODS ASKING FOR GPU / VRAM ({}) ", node.pods.len()),
            title_style,
        ));

    let inner = block.inner(area);
    f.render_widget(block, area);

    state.last_pods_pane_rect.set(inner);

    if node.pods.is_empty() {
        let p = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled("  ✓ No pods currently requesting GPU on this node.", Style::default().fg(Theme::GREEN))),
            Line::from(""),
            Line::from(Span::styled(
                format!("  All {} GPUs ({}) are free and ready to accept workloads.",
                    node.gpu_capacity,
                    node.vram_capacity_total_mib.map(format_vram_mib).unwrap_or_else(|| "-".to_string())
                ),
                Style::default().fg(Theme::DIM)
            )),
        ]);
        f.render_widget(p, inner);
        return;
    }

    let visible_rows = inner.height.saturating_sub(2) as usize;
    if visible_rows == 0 {
        return;
    }

    let selected = state.selected_pod_idx;
    let mut start_idx = state.last_pods_start_idx.get();
    if selected < start_idx {
        start_idx = selected;
    } else if selected >= start_idx + visible_rows {
        start_idx = selected + 1 - visible_rows;
    }
    state.last_pods_start_idx.set(start_idx);

    let mut lines = Vec::new();

    let pod_col_width = (inner.width as usize).saturating_sub(74).max(28);

    // Table Header
    lines.push(Line::from(vec![
        Span::styled(format!(" {:<15} ", "NAMESPACE"), Theme::table_header()),
        Span::styled(format!("{:<width$} ", "POD NAME", width = pod_col_width), Theme::table_header()),
        Span::styled(format!("{:<11} ", "STATUS"), Theme::table_header()),
        Span::styled(format!("{:<7} ", "GPUS"), Theme::table_header()),
        Span::styled(format!("{:<12} ", "VRAM REQ"), Theme::table_header()),
        Span::styled(format!("{:<7} ", "READY"), Theme::table_header()),
        Span::styled(format!("{:<9} ", "RESTARTS"), Theme::table_header()),
        Span::styled(format!("{:<6}", "AGE"), Theme::table_header()),
    ]));
    lines.push(Line::from(Span::styled("─".repeat(inner.width as usize), Style::default().fg(Theme::BORDER))));

    let end_idx = (start_idx + visible_rows).min(node.pods.len());
    for i in start_idx..end_idx {
        let pod = &node.pods[i];
        let is_sel = i == selected;

        let row_style = if is_sel {
            if is_focused {
                Style::default().bg(Theme::SEL_BG).fg(Theme::SEL_FG).add_modifier(Modifier::BOLD)
            } else {
                Style::default().bg(Color::Rgb(30, 41, 59)).fg(Theme::SEL_FG).add_modifier(Modifier::BOLD)
            }
        } else {
            Style::default().fg(Theme::fg())
        };

        let status_color = if pod.phase == "Running" {
            Theme::GREEN
        } else if pod.phase == "Completed" || pod.phase == "Succeeded" {
            Theme::CYAN
        } else {
            Theme::RED
        };

        let prefix = if is_sel { ">" } else { " " };
        let trunc_ns = if pod.namespace.len() > 15 {
            format!("{}…", &pod.namespace[..14])
        } else {
            pod.namespace.clone()
        };

        let trunc_name = if pod.name.len() > pod_col_width {
            format!("{}…", &pod.name[..pod_col_width.saturating_sub(1)])
        } else {
            pod.name.clone()
        };

        let vram_str = format_vram_mib(pod.vram_requests_mib);

        lines.push(Line::from(vec![
            Span::styled(format!("{}{:<15} ", prefix, trunc_ns), row_style),
            Span::styled(format!("{:<width$} ", trunc_name, width = pod_col_width), row_style),
            Span::styled(format!("{:<11} ", pod.phase), Style::default().fg(status_color)),
            Span::styled(format!("{:<7} ", pod.gpu_requests), row_style),
            Span::styled(format!("{:<12} ", vram_str), Style::default().fg(Theme::CYAN).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{:<7} ", pod.ready_containers), row_style),
            Span::styled(format!("{:<9} ", pod.restarts), row_style),
            Span::styled(format!("{:<6}", pod.age), Style::default().fg(Theme::DIM)),
        ]));
    }

    let p = Paragraph::new(lines);
    f.render_widget(p, inner);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gpu_view_state_navigation_and_selection() {
        let mut state = GpuViewState::new();
        assert!(state.is_loading);

        let pod = GpuPodItem {
            name: "llama3-70b-0".to_string(),
            namespace: "ai-gen".to_string(),
            phase: "Running".to_string(),
            gpu_requests: 4,
            vram_requests_mib: 327680,
            ready_containers: "1/1".to_string(),
            restarts: 0,
            age: "5d".to_string(),
            containers: vec!["engine".to_string()],
        };

        let node1 = GpuNodeInfo {
            name: "gpu-node-alpha".to_string(),
            status: "Ready".to_string(),
            unschedulable: false,
            roles: "worker".to_string(),
            instance_type: "p4de.24xlarge".to_string(),
            gpu_model: Some("NVIDIA A100 SXM4 80GB".to_string()),
            gpu_driver_version: Some("535.129.03".to_string()),
            gpu_cuda_version: Some("12.2".to_string()),
            gpu_capacity: 8,
            gpu_allocatable: 8,
            gpu_requests: 4,
            vram_per_gpu_mib: Some(81920),
            vram_capacity_total_mib: Some(655360),
            vram_requests_total_mib: 327680,
            pods: vec![pod.clone()],
        };

        let node2 = GpuNodeInfo {
            name: "gpu-node-beta".to_string(),
            status: "Ready".to_string(),
            unschedulable: false,
            roles: "worker".to_string(),
            instance_type: "g4dn.xlarge".to_string(),
            gpu_model: Some("Tesla T4".to_string()),
            gpu_driver_version: Some("535.129.03".to_string()),
            gpu_cuda_version: Some("12.2".to_string()),
            gpu_capacity: 1,
            gpu_allocatable: 1,
            gpu_requests: 0,
            vram_per_gpu_mib: Some(15360),
            vram_capacity_total_mib: Some(15360),
            vram_requests_total_mib: 0,
            pods: vec![],
        };

        let info = GpuClusterInfo {
            nodes: vec![node1, node2],
            total_gpu_nodes: 2,
            total_gpus: 9,
            total_allocated_gpus: 4,
            total_vram_mib: 670720,
            total_allocated_vram_mib: 327680,
            total_gpu_pods: 1,
        };

        state.set_info(info);
        assert!(!state.is_loading);
        assert_eq!(state.selected_node().unwrap().name, "gpu-node-alpha");
        assert_eq!(state.selected_pod().unwrap().name, "llama3-70b-0");

        // Navigate to next node
        state.select_next_node();
        assert_eq!(state.selected_node().unwrap().name, "gpu-node-beta");
        assert!(state.selected_pod().is_none()); // node2 has 0 pods

        // Navigate back to prev node
        state.select_prev_node();
        assert_eq!(state.selected_node().unwrap().name, "gpu-node-alpha");
        assert_eq!(state.selected_pod().unwrap().name, "llama3-70b-0");

        // Toggle pane focus
        assert_eq!(state.focused_pane, GpuPane::Nodes);
        state.toggle_pane();
        assert_eq!(state.focused_pane, GpuPane::Pods);
    }

    #[test]
    fn test_gpu_view_render_dynamic_node_col_width() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let mut state = GpuViewState::new();
        let long_node_name = "data-processing-stage-gpu-s79cj".to_string();
        let node = GpuNodeInfo {
            name: long_node_name.clone(),
            status: "Ready".to_string(),
            unschedulable: false,
            roles: "worker".to_string(),
            instance_type: "p4de.24xlarge".to_string(),
            gpu_model: Some("NVIDIA A100 SXM4 80GB".to_string()),
            gpu_driver_version: Some("535.129.03".to_string()),
            gpu_cuda_version: Some("12.2".to_string()),
            gpu_capacity: 8,
            gpu_allocatable: 8,
            gpu_requests: 0,
            vram_per_gpu_mib: Some(81920),
            vram_capacity_total_mib: Some(655360),
            vram_requests_total_mib: 0,
            pods: vec![],
        };

        let info = GpuClusterInfo {
            nodes: vec![node],
            total_gpu_nodes: 1,
            total_gpus: 8,
            total_allocated_gpus: 0,
            total_vram_mib: 655360,
            total_allocated_vram_mib: 0,
            total_gpu_pods: 0,
        };
        state.set_info(info);

        let backend = TestBackend::new(140, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, f.area(), &state)).unwrap();

        let buffer = terminal.backend().buffer();
        let content: String = (0..buffer.area.height)
            .map(|y| {
                let mut line = String::new();
                for x in 0..buffer.area.width {
                    line.push_str(buffer[(x, y)].symbol());
                }
                line
            })
            .collect::<Vec<_>>()
            .join("\n");

        // The full long node name must be rendered without being cut/truncated with '…'
        assert!(content.contains("data-processing-stage-gpu-s79cj"), "Rendered output should contain full node name without truncation");
        assert!(!content.contains("data-processing-st…"));
    }
}
