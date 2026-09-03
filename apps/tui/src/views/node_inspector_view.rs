use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph},
    Frame,
};

use srelens_kube::node_inspector::{NodeInspectorDetails, NodePodItem};
use crate::theme::Theme;

#[derive(Debug, Clone)]
pub struct NodeInspectorState {
    pub node_name: String,
    pub details: Option<NodeInspectorDetails>,
    pub selected_pod_idx: usize,
    pub scroll_offset: usize,
    pub is_loading: bool,
    pub error: Option<String>,
}

impl NodeInspectorState {
    pub fn new(node_name: String) -> Self {
        Self {
            node_name,
            details: None,
            selected_pod_idx: 0,
            scroll_offset: 0,
            is_loading: true,
            error: None,
        }
    }

    pub fn set_details(&mut self, details: NodeInspectorDetails) {
        self.details = Some(details);
        self.is_loading = false;
        self.error = None;
        if self.selected_pod_idx >= self.pods_len() {
            self.selected_pod_idx = self.pods_len().saturating_sub(1);
        }
    }

    pub fn set_error(&mut self, err: String) {
        self.error = Some(err);
        self.is_loading = false;
    }

    pub fn pods_len(&self) -> usize {
        self.details.as_ref().map(|d| d.pods.len()).unwrap_or(0)
    }

    pub fn selected_pod(&self) -> Option<&NodePodItem> {
        self.details
            .as_ref()
            .and_then(|d| d.pods.get(self.selected_pod_idx))
    }

    pub fn select_next(&mut self) {
        let total = self.pods_len();
        if total > 0 && self.selected_pod_idx + 1 < total {
            self.selected_pod_idx += 1;
        }
    }

    pub fn select_prev(&mut self) {
        if self.selected_pod_idx > 0 {
            self.selected_pod_idx -= 1;
        }
    }

    pub fn select_first(&mut self) {
        self.selected_pod_idx = 0;
        self.scroll_offset = 0;
    }

    pub fn select_last(&mut self) {
        let total = self.pods_len();
        if total > 0 {
            self.selected_pod_idx = total - 1;
        }
    }

    pub fn page_down(&mut self, step: usize) {
        let total = self.pods_len();
        if total > 0 {
            self.selected_pod_idx = (self.selected_pod_idx + step).min(total - 1);
        }
    }

    pub fn page_up(&mut self, step: usize) {
        self.selected_pod_idx = self.selected_pod_idx.saturating_sub(step);
    }
}

pub fn render_node_inspector_view(f: &mut Frame, area: Rect, state: &NodeInspectorState) {
    if state.is_loading && state.details.is_none() {
        let loading_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Theme::BORDER))
            .title(format!(" Node Inspector: {} ", state.node_name));
        let p = Paragraph::new(format!(
            "\n  ⏳ Inspecting node '{}'...\n  Fetching hardware capacity, GPU allocations, and scheduled pods...",
            state.node_name
        ))
        .block(loading_block)
        .style(Style::default().fg(Theme::CYAN));
        f.render_widget(p, area);
        return;
    }

    if let Some(err) = &state.error {
        if state.details.is_none() {
            let error_block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Theme::RED))
                .title(format!(" Node Inspector Error: {} ", state.node_name));
            let p = Paragraph::new(format!(
                "\n  ❌ Failed to inspect node '{}':\n\n  {}\n\n  Press <Esc> or <q> to return, or <r> to retry.",
                state.node_name, err
            ))
            .block(error_block)
            .style(Style::default().fg(Theme::RED));
            f.render_widget(p, area);
            return;
        }
    }

    let Some(d) = &state.details else {
        return;
    };

    // Layout hierarchy:
    // 1. Header Card (height: 4)
    // 2. Resource & GPU Allocation Gauges (height: 4)
    // 3. Conditions & Taints Strip (height: 3)
    // 4. Scheduled Pods Table (min: 8)
    // 5. Footer Key Hints (height: 1)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4), // Header
            Constraint::Length(4), // Gauges
            Constraint::Length(3), // Conditions & Taints
            Constraint::Min(8),    // Pods table
            Constraint::Length(1), // Footer
        ])
        .split(area);

    // --- 1. Header Card ---
    render_header_card(f, chunks[0], d);

    // --- 2. Gauges Area ---
    render_gauges_card(f, chunks[1], d);

    // --- 3. Conditions & Taints Bar ---
    render_conditions_and_taints(f, chunks[2], d);

    // --- 4. Scheduled Pods Table ---
    render_pods_table(f, chunks[3], state, d);

    // --- 5. Footer Shortcuts ---
    render_footer_hints(f, chunks[4], d);
}

fn render_header_card(f: &mut Frame, area: Rect, d: &NodeInspectorDetails) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::BORDER))
        .title(Span::styled(
            format!(" 🖥️  Node: {} ", d.name),
            Style::default().fg(Theme::ACCENT).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let status_color = if d.status == "Ready" {
        Theme::GREEN
    } else {
        Theme::RED
    };

    let mut row1_spans = vec![
        Span::styled(
            format!("● {} ", d.status),
            Style::default().fg(status_color).add_modifier(Modifier::BOLD),
        ),
    ];

    if d.unschedulable {
        row1_spans.push(Span::styled(
            "[CORDONED / UNSCHEDULABLE] ",
            Style::default()
                .fg(Color::Black)
                .bg(Theme::YELLOW)
                .add_modifier(Modifier::BOLD),
        ));
    }

    if d.has_gpu {
        let model = d.gpu_model.as_deref().unwrap_or("GPU Accelerator");
        row1_spans.push(Span::styled(
            format!("[⚡ {}] ", model),
            Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD),
        ));
    }

    row1_spans.push(Span::styled("Role: ", Theme::header_label()));
    row1_spans.push(Span::styled(format!("{}  ", d.roles), Theme::header_val()));

    row1_spans.push(Span::styled("Type: ", Theme::header_label()));
    row1_spans.push(Span::styled(format!("{}  ", d.instance_type), Theme::header_val()));

    if let Some(zone) = &d.zone {
        row1_spans.push(Span::styled("Zone: ", Theme::header_label()));
        row1_spans.push(Span::styled(format!("{}  ", zone), Theme::header_val()));
    }

    if let Some(pool) = &d.nodepool {
        row1_spans.push(Span::styled("Pool: ", Theme::header_label()));
        row1_spans.push(Span::styled(format!("{}  ", pool), Theme::header_val()));
    }

    row1_spans.push(Span::styled("Kubelet: ", Theme::header_label()));
    row1_spans.push(Span::styled(&d.kubelet_version, Theme::header_val()));

    let mut row2_spans = vec![
        Span::styled("OS: ", Theme::header_label()),
        Span::styled(format!("{} ({})  ", d.os_image, d.architecture), Theme::header_val()),
        Span::styled("Kernel: ", Theme::header_label()),
        Span::styled(format!("{}  ", d.kernel_version), Theme::header_val()),
        Span::styled("Runtime: ", Theme::header_label()),
        Span::styled(format!("{}  ", d.container_runtime), Theme::header_val()),
    ];

    if let Some(ip) = &d.internal_ip {
        row2_spans.push(Span::styled("IP: ", Theme::header_label()));
        row2_spans.push(Span::styled(ip, Theme::header_val()));
    }

    let p = Paragraph::new(vec![Line::from(row1_spans), Line::from(row2_spans)]);
    f.render_widget(p, inner);
}

fn render_gauges_card(f: &mut Frame, area: Rect, d: &NodeInspectorDetails) {
    let has_gpu = d.has_gpu;
    let gauge_constraints = if has_gpu {
        vec![
            Constraint::Percentage(25), // CPU
            Constraint::Percentage(25), // Memory
            Constraint::Percentage(25), // Pods
            Constraint::Percentage(25), // GPU
        ]
    } else {
        vec![
            Constraint::Percentage(33), // CPU
            Constraint::Percentage(33), // Memory
            Constraint::Percentage(34), // Pods
        ]
    };

    let gauge_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(gauge_constraints)
        .split(area);

    // 1. CPU Gauge
    let cpu_alloc = d.cpu_allocatable_millicores as f64 / 1000.0;
    let cpu_req = d.cpu_requests_millicores as f64 / 1000.0;
    let cpu_pct = if d.cpu_allocatable_millicores > 0 {
        ((d.cpu_requests_millicores as f64 / d.cpu_allocatable_millicores as f64) * 100.0).round() as u16
    } else {
        0
    };
    let cpu_color = if cpu_pct > 85 {
        Theme::RED
    } else if cpu_pct > 70 {
        Theme::YELLOW
    } else {
        Theme::CYAN
    };

    let cpu_title = format!(" CPU: {:.1}/{:.1} Cores ({}%) ", cpu_req, cpu_alloc, cpu_pct);
    let cpu_gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Theme::BORDER)).title(cpu_title))
        .gauge_style(Style::default().fg(cpu_color))
        .percent(cpu_pct.min(100));
    f.render_widget(cpu_gauge, gauge_chunks[0]);

    // 2. Memory Gauge
    let mem_alloc_gib = d.mem_allocatable_mib as f64 / 1024.0;
    let mem_req_gib = d.mem_requests_mib as f64 / 1024.0;
    let mem_pct = if d.mem_allocatable_mib > 0 {
        ((d.mem_requests_mib as f64 / d.mem_allocatable_mib as f64) * 100.0).round() as u16
    } else {
        0
    };
    let mem_color = if mem_pct > 85 {
        Theme::RED
    } else if mem_pct > 70 {
        Theme::YELLOW
    } else {
        Theme::ACCENT
    };

    let mem_title = format!(" Memory: {:.1}/{:.1} GiB ({}%) ", mem_req_gib, mem_alloc_gib, mem_pct);
    let mem_gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Theme::BORDER)).title(mem_title))
        .gauge_style(Style::default().fg(mem_color))
        .percent(mem_pct.min(100));
    f.render_widget(mem_gauge, gauge_chunks[1]);

    // 3. Pods Gauge
    let pods_alloc = d.pods_allocatable.max(1);
    let pods_pct = ((d.pods_count as f64 / pods_alloc as f64) * 100.0).round() as u16;
    let pods_color = if pods_pct > 85 {
        Theme::RED
    } else if pods_pct > 70 {
        Theme::YELLOW
    } else {
        Theme::GREEN
    };

    let pods_title = format!(" Pods: {}/{} ({}%) ", d.pods_count, d.pods_allocatable, pods_pct);
    let pods_gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Theme::BORDER)).title(pods_title))
        .gauge_style(Style::default().fg(pods_color))
        .percent(pods_pct.min(100));
    f.render_widget(pods_gauge, gauge_chunks[2]);

    // 4. GPU Gauge (if present)
    if has_gpu {
        let gpu_alloc = d.gpu_allocatable_count.max(d.gpu_capacity_count).max(1);
        let gpu_pct = if gpu_alloc > 0 {
            ((d.gpu_requests_count as f64 / gpu_alloc as f64) * 100.0).round() as u16
        } else {
            0
        };
        let gpu_color = if gpu_pct > 90 {
            Theme::RED
        } else if gpu_pct > 75 {
            Theme::YELLOW
        } else {
            Theme::YELLOW
        };

        let model_label = d.gpu_model.as_deref().unwrap_or("GPU");
        let gpu_title = format!(" ⚡ {}: {}/{} ({}%) ", model_label, d.gpu_requests_count, gpu_alloc, gpu_pct);
        let gpu_gauge = Gauge::default()
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Theme::YELLOW))
                    .title(gpu_title),
            )
            .gauge_style(Style::default().fg(gpu_color))
            .percent(gpu_pct.min(100));
        f.render_widget(gpu_gauge, gauge_chunks[3]);
    }
}

fn render_conditions_and_taints(f: &mut Frame, area: Rect, d: &NodeInspectorDetails) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::BORDER))
        .title(" Health Conditions & Taints ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut spans = Vec::new();

    // 1. Conditions
    spans.push(Span::styled("Conditions: ", Style::default().fg(Theme::DIM)));
    for cond in &d.conditions {
        let is_ok = match cond.type_.as_str() {
            "Ready" => cond.status == "True",
            "MemoryPressure" | "DiskPressure" | "PIDPressure" | "NetworkUnavailable" => cond.status == "False",
            _ => true,
        };
        let color = if is_ok { Theme::GREEN } else { Theme::RED };
        spans.push(Span::styled(format!("{}:{} ", cond.type_, cond.status), Style::default().fg(color)));
    }

    spans.push(Span::raw(" │ "));

    // 2. Taints
    spans.push(Span::styled("Taints: ", Style::default().fg(Theme::DIM)));
    if d.taints.is_empty() {
        spans.push(Span::styled("<none>", Style::default().fg(Theme::DIM)));
    } else {
        for (i, t) in d.taints.iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw(" "));
            }
            let taint_str = match &t.value {
                Some(v) => format!("{}={}:{}", t.key, v, t.effect),
                None => format!("{}:{}", t.key, t.effect),
            };
            let color = if t.key.contains("gpu") {
                Theme::YELLOW
            } else if t.effect == "NoExecute" {
                Theme::RED
            } else {
                Theme::CYAN
            };
            spans.push(Span::styled(taint_str, Style::default().fg(color)));
        }
    }

    // Driver & CUDA details if present
    if let (Some(drv), Some(cuda)) = (&d.gpu_driver_version, &d.gpu_cuda_version) {
        spans.push(Span::raw(" │ "));
        spans.push(Span::styled(format!("Driver: {} (CUDA {})", drv, cuda), Style::default().fg(Theme::YELLOW)));
    }

    let p = Paragraph::new(Line::from(spans));
    f.render_widget(p, inner);
}

fn render_pods_table(f: &mut Frame, area: Rect, state: &NodeInspectorState, d: &NodeInspectorDetails) {
    let gpu_pod_count = d.pods.iter().filter(|p| p.gpu_requests > 0).count();
    let table_title = if gpu_pod_count > 0 {
        format!(" Scheduled Pods ({} Total | ⚡ {} GPU Workloads) ", d.pods.len(), gpu_pod_count)
    } else {
        format!(" Scheduled Pods ({} Total) ", d.pods.len())
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::BORDER))
        .title(Span::styled(table_title, Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if d.pods.is_empty() {
        let empty_p = Paragraph::new("\n  No pods currently scheduled on this node.")
            .style(Style::default().fg(Theme::DIM));
        f.render_widget(empty_p, inner);
        return;
    }

    // Column widths:
    // Marker (2), NS (18), Name (32), Status (12), Ready (6), Restarts (5), CPU (10), Mem (10), GPU (12), Age (6)
    let visible_rows = inner.height.saturating_sub(1) as usize; // header takes 1
    if visible_rows == 0 {
        return;
    }

    // Adjust scroll offset to keep selected_pod_idx visible
    let mut scroll = state.scroll_offset;
    if state.selected_pod_idx < scroll {
        scroll = state.selected_pod_idx;
    } else if state.selected_pod_idx >= scroll + visible_rows {
        scroll = state.selected_pod_idx - visible_rows + 1;
    }

    let mut lines = Vec::new();

    // Table Header
    let header_line = Line::from(vec![
        Span::styled("  ", Style::default()),
        Span::styled(format!("{:<18}", "NAMESPACE"), Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)),
        Span::styled(format!("{:<34}", "NAME"), Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)),
        Span::styled(format!("{:<12}", "STATUS"), Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)),
        Span::styled(format!("{:<6}", "READY"), Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)),
        Span::styled(format!("{:<6}", "REST"), Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)),
        Span::styled(format!("{:<10}", "CPU REQ"), Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)),
        Span::styled(format!("{:<10}", "MEM REQ"), Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)),
        Span::styled(format!("{:<12}", "GPU REQ"), Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)),
        Span::styled(format!("{:<6}", "AGE"), Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)),
    ]);
    lines.push(header_line);

    for (idx, pod) in d.pods.iter().enumerate().skip(scroll).take(visible_rows) {
        let is_selected = idx == state.selected_pod_idx;

        let row_bg = if is_selected {
            Theme::SEL_BG
        } else {
            Color::Reset
        };

        let marker = if is_selected { "▶ " } else { "  " };
        let marker_style = if is_selected {
            Style::default().fg(Theme::CYAN).bg(row_bg).add_modifier(Modifier::BOLD)
        } else {
            Style::default().bg(row_bg)
        };

        let ns_str = truncate_str(&pod.namespace, 17);
        let name_str = truncate_str(&pod.name, 33);

        let status_color = match pod.phase.as_str() {
            "Running" => Theme::GREEN,
            "Succeeded" => Theme::CYAN,
            "Pending" => Theme::YELLOW,
            "Failed" | "CrashLoopBackOff" | "Error" => Theme::RED,
            _ => Theme::FG,
        };

        let cpu_str = if pod.cpu_requests_millicores >= 1000 {
            format!("{:.1} cores", pod.cpu_requests_millicores as f64 / 1000.0)
        } else if pod.cpu_requests_millicores > 0 {
            format!("{}m", pod.cpu_requests_millicores)
        } else {
            "-".to_string()
        };

        let mem_str = if pod.mem_requests_mib >= 1024 {
            format!("{:.1} GiB", pod.mem_requests_mib as f64 / 1024.0)
        } else if pod.mem_requests_mib > 0 {
            format!("{} MiB", pod.mem_requests_mib)
        } else {
            "-".to_string()
        };

        let (gpu_str, gpu_style) = if pod.gpu_requests > 0 {
            (
                format!("⚡ {} GPU", pod.gpu_requests),
                Style::default().fg(Theme::YELLOW).bg(row_bg).add_modifier(Modifier::BOLD),
            )
        } else {
            ("-".to_string(), Style::default().fg(Theme::DIM).bg(row_bg))
        };

        let row = Line::from(vec![
            Span::styled(marker, marker_style),
            Span::styled(format!("{:<18}", ns_str), Style::default().fg(Theme::CYAN).bg(row_bg)),
            Span::styled(format!("{:<34}", name_str), Style::default().fg(if is_selected { Theme::ACCENT } else { Theme::FG }).bg(row_bg).add_modifier(if is_selected { Modifier::BOLD } else { Modifier::empty() })),
            Span::styled(format!("{:<12}", pod.phase), Style::default().fg(status_color).bg(row_bg)),
            Span::styled(format!("{:<6}", pod.ready_containers), Style::default().fg(Theme::FG).bg(row_bg)),
            Span::styled(format!("{:<6}", pod.restarts), Style::default().fg(if pod.restarts > 0 { Theme::YELLOW } else { Theme::DIM }).bg(row_bg)),
            Span::styled(format!("{:<10}", cpu_str), Style::default().fg(Theme::FG).bg(row_bg)),
            Span::styled(format!("{:<10}", mem_str), Style::default().fg(Theme::FG).bg(row_bg)),
            Span::styled(format!("{:<12}", gpu_str), gpu_style),
            Span::styled(format!("{:<6}", pod.age), Style::default().fg(Theme::DIM).bg(row_bg)),
        ]);
        lines.push(row);
    }

    let p = Paragraph::new(lines);
    f.render_widget(p, inner);
}

fn render_footer_hints(f: &mut Frame, area: Rect, d: &NodeInspectorDetails) {
    let cordon_hint = if d.unschedulable {
        ("<c>", "Uncordon")
    } else {
        ("<c>", "Cordon")
    };

    let hints: &[(&str, &str)] = &[
        ("<↑/↓>", "Select Pod"),
        ("<Enter>", "Jump"),
        ("<l>", "Logs"),
        ("<d>", "Pod Describe"),
        ("<D>", "Node Describe"),
        ("<y>", "Pod YAML"),
        ("<Y>", "Node YAML"),
        ("<x>", "Actions"),
        cordon_hint,
        ("<s>", "Shell"),
        ("<r>", "Refresh"),
        ("<Esc>", "Back"),
    ];

    let mut spans = Vec::new();
    for (i, (k, label)) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled(*k, Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)));
        spans.push(Span::styled(format!(":{} ", label), Style::default().fg(Theme::DIM)));
    }

    let p = Paragraph::new(Line::from(spans));
    f.render_widget(p, area);
}

fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() > max_len {
        format!("{}…", &s[..max_len.saturating_sub(1)])
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_inspector_state_navigation() {
        let mut state = NodeInspectorState::new("node-1".to_string());
        assert_eq!(state.node_name, "node-1");
        assert!(state.is_loading);
        assert_eq!(state.pods_len(), 0);

        let details = NodeInspectorDetails {
            name: "node-1".to_string(),
            status: "Ready".to_string(),
            unschedulable: false,
            roles: "worker".to_string(),
            instance_type: "m5.xlarge".to_string(),
            zone: Some("us-east-1a".to_string()),
            region: Some("us-east-1".to_string()),
            nodepool: Some("default-pool".to_string()),
            internal_ip: Some("10.0.0.1".to_string()),
            external_ip: None,
            os_image: "Ubuntu".to_string(),
            kernel_version: "5.15".to_string(),
            container_runtime: "containerd".to_string(),
            kubelet_version: "v1.30.0".to_string(),
            architecture: "amd64".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            cpu_capacity_millicores: 4000,
            cpu_allocatable_millicores: 3900,
            cpu_requests_millicores: 1200,
            mem_capacity_mib: 16384,
            mem_allocatable_mib: 15000,
            mem_requests_mib: 4096,
            pods_capacity: 110,
            pods_allocatable: 110,
            pods_count: 2,
            has_gpu: false,
            gpu_model: None,
            gpu_driver_version: None,
            gpu_cuda_version: None,
            gpu_capacity_count: 0,
            gpu_allocatable_count: 0,
            gpu_requests_count: 0,
            gpu_memory_total_mib: None,
            conditions: vec![],
            taints: vec![],
            pods: vec![
                NodePodItem {
                    name: "pod-1".to_string(),
                    namespace: "default".to_string(),
                    phase: "Running".to_string(),
                    ready_containers: "1/1".to_string(),
                    restarts: 0,
                    age: "2d".to_string(),
                    cpu_requests_millicores: 500,
                    mem_requests_mib: 1024,
                    gpu_requests: 0,
                },
                NodePodItem {
                    name: "pod-2".to_string(),
                    namespace: "kube-system".to_string(),
                    phase: "Running".to_string(),
                    ready_containers: "1/1".to_string(),
                    restarts: 1,
                    age: "5d".to_string(),
                    cpu_requests_millicores: 700,
                    mem_requests_mib: 3072,
                    gpu_requests: 0,
                },
            ],
        };

        state.set_details(details);
        assert!(!state.is_loading);
        assert_eq!(state.pods_len(), 2);
        assert_eq!(state.selected_pod_idx, 0);
        assert_eq!(state.selected_pod().unwrap().name, "pod-1");

        state.select_next();
        assert_eq!(state.selected_pod_idx, 1);
        assert_eq!(state.selected_pod().unwrap().name, "pod-2");

        // Bounds check
        state.select_next();
        assert_eq!(state.selected_pod_idx, 1);

        state.select_prev();
        assert_eq!(state.selected_pod_idx, 0);

        state.select_last();
        assert_eq!(state.selected_pod_idx, 1);

        state.select_first();
        assert_eq!(state.selected_pod_idx, 0);
    }
}
