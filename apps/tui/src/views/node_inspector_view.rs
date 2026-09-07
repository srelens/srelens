use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        canvas::{Canvas, Line as CanvasLine},
        Block, Borders, Gauge, Paragraph,
    },
    Frame,
};

pub use srelens_kube::node_inspector::{NodeInspectorDetails, NodePodItem};
use crate::theme::Theme;
use crate::views::metrics_panel_view::{compute_y_bounds, format_axis_val};

#[derive(Debug, Clone)]
pub struct NodeInspectorState {
    pub node_name: String,
    pub details: Option<NodeInspectorDetails>,
    pub cpu_history: Vec<u64>,
    pub mem_history: Vec<u64>,
    pub selected_pod_idx: usize,
    pub scroll_offset: usize,
    pub is_loading: bool,
    pub error: Option<String>,
    pub last_pods_table_rect: std::cell::Cell<Rect>,
    pub last_scroll_offset: std::cell::Cell<usize>,
}

impl NodeInspectorState {
    pub fn new(node_name: String) -> Self {
        Self {
            node_name,
            details: None,
            cpu_history: Vec::new(),
            mem_history: Vec::new(),
            selected_pod_idx: 0,
            scroll_offset: 0,
            is_loading: true,
            error: None,
            last_pods_table_rect: std::cell::Cell::new(Rect::default()),
            last_scroll_offset: std::cell::Cell::new(0),
        }
    }

    pub fn update_metrics_history(&mut self, cpu: &[u64], mem: &[u64]) {
        self.cpu_history = cpu.to_vec();
        self.mem_history = mem.to_vec();
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
    // 3. Live Metrics Timeline (Canvas line charts for CPU & Memory) (height: 4 if height >= 26)
    // 4. Conditions & Taints Strip (height: 3)
    // 5. Scheduled Pods Table (min: 6)
    // 6. Footer Key Hints (height: 1)
    let show_sparklines = area.height >= 26;
    let sparkline_height = if show_sparklines { 4 } else { 0 };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),                 // Header
            Constraint::Length(4),                 // Gauges
            Constraint::Length(sparkline_height),  // Live Metrics Timeline
            Constraint::Length(3),                 // Conditions & Taints
            Constraint::Min(6),                    // Pods table
            Constraint::Length(1),                 // Footer
        ])
        .split(area);

    // --- 1. Header Card ---
    render_header_card(f, chunks[0], d);

    // --- 2. Gauges Area ---
    render_gauges_card(f, chunks[1], d);

    // --- 3. Live Metrics Timeline ---
    if show_sparklines {
        render_metrics_timeline_card(f, chunks[2], state, d);
    }

    // --- 4. Conditions & Taints Bar ---
    render_conditions_and_taints(f, chunks[3], d);

    // --- 5. Scheduled Pods Table ---
    render_pods_table(f, chunks[4], state, d);

    // --- 6. Footer Shortcuts ---
    render_footer_hints(f, chunks[5], d);
}

fn render_metrics_timeline_card(f: &mut Frame, area: Rect, state: &NodeInspectorState, d: &NodeInspectorDetails) {
    if area.height < 3 || area.width < 20 {
        return;
    }
    let h_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    // 1. CPU Canvas Line Chart
    let cur_cpu = state.cpu_history.last().copied().unwrap_or(d.cpu_requests_millicores.max(0) as u64);
    let min_cpu = state.cpu_history.iter().copied().min().unwrap_or(cur_cpu);
    let peak_cpu = state.cpu_history.iter().copied().max().unwrap_or(cur_cpu);

    let cpu_title = format!(" 📈 CPU Usage Trend [cur: {}m | peak: {}m | alloc: {}m] ", cur_cpu, peak_cpu, d.cpu_allocatable_millicores);
    let cpu_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::BORDER))
        .title(Span::styled(cpu_title, Style::default().fg(Theme::CYAN).add_modifier(Modifier::BOLD)));
    let cpu_inner = cpu_block.inner(h_chunks[0]);
    f.render_widget(cpu_block, h_chunks[0]);

    if state.cpu_history.is_empty() {
        let p = Paragraph::new(Line::from(Span::styled("⚡ Awaiting metrics-server samples...", Style::default().fg(Theme::DIM))));
        f.render_widget(p, cpu_inner);
    } else {
        let splits = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(8), // Y-axis labels & tick marks
                Constraint::Min(10),   // Canvas line plot
            ])
            .split(cpu_inner);

        let (cpu_y_floor, cpu_y_ceil, cpu_y_top, cpu_y_mid, cpu_y_bot) = compute_y_bounds(min_cpu, peak_cpu);

        let y_height = splits[0].height as usize;
        let mut y_spans = Vec::new();
        if y_height >= 3 {
            y_spans.push(Line::from(Span::styled(format!("{:>6} ┤", format_axis_val(cpu_y_top, false)), Style::default().fg(Theme::DIM))));
            let blanks = y_height.saturating_sub(3);
            let top_blanks = blanks / 2;
            let bot_blanks = blanks - top_blanks;
            for _ in 0..top_blanks {
                y_spans.push(Line::from(Span::styled("       │", Style::default().fg(Theme::DIM))));
            }
            y_spans.push(Line::from(Span::styled(format!("{:>6} ┤", format_axis_val(cpu_y_mid, false)), Style::default().fg(Theme::DIM))));
            for _ in 0..bot_blanks {
                y_spans.push(Line::from(Span::styled("       │", Style::default().fg(Theme::DIM))));
            }
            y_spans.push(Line::from(Span::styled(format!("{:>6} ┤", format_axis_val(cpu_y_bot, false)), Style::default().fg(Theme::DIM))));
        } else if y_height >= 2 {
            y_spans.push(Line::from(Span::styled(format!("{:>6} ┤", format_axis_val(cpu_y_top, false)), Style::default().fg(Theme::DIM))));
            for _ in 0..y_height.saturating_sub(2) {
                y_spans.push(Line::from(Span::styled("       │", Style::default().fg(Theme::DIM))));
            }
            y_spans.push(Line::from(Span::styled(format!("{:>6} ┤", format_axis_val(cpu_y_bot, false)), Style::default().fg(Theme::DIM))));
        } else {
            y_spans.push(Line::from(Span::styled(format!("{:>6} ┤", format_axis_val(cpu_y_top, false)), Style::default().fg(Theme::DIM))));
        }
        f.render_widget(Paragraph::new(y_spans), splits[0]);

        let n = state.cpu_history.len();
        let x_max = (n.saturating_sub(1)).max(1) as f64;
        let cpu_pts = state.cpu_history.clone();

        let canvas = Canvas::default()
            .x_bounds([0.0, x_max])
            .y_bounds([cpu_y_floor, cpu_y_ceil])
            .paint(move |ctx| {
                let y_mid_f = (cpu_y_floor + cpu_y_ceil) / 2.0;
                ctx.draw(&CanvasLine {
                    x1: 0.0,
                    y1: y_mid_f,
                    x2: x_max,
                    y2: y_mid_f,
                    color: Color::Rgb(45, 55, 72),
                });

                if cpu_pts.len() == 1 {
                    ctx.draw(&CanvasLine {
                        x1: 0.0,
                        y1: cpu_pts[0] as f64,
                        x2: x_max,
                        y2: cpu_pts[0] as f64,
                        color: Theme::CYAN,
                    });
                } else if cpu_pts.len() > 1 {
                    for i in 0..cpu_pts.len() - 1 {
                        ctx.draw(&CanvasLine {
                            x1: i as f64,
                            y1: cpu_pts[i] as f64,
                            x2: (i + 1) as f64,
                            y2: cpu_pts[i + 1] as f64,
                            color: Theme::CYAN,
                        });
                    }
                }
            });
        f.render_widget(canvas, splits[1]);
    }

    // 2. Memory Canvas Line Chart
    let cur_mem = state.mem_history.last().copied().unwrap_or(d.mem_requests_mib.max(0) as u64);
    let min_mem = state.mem_history.iter().copied().min().unwrap_or(cur_mem);
    let peak_mem = state.mem_history.iter().copied().max().unwrap_or(cur_mem);

    let mem_title = format!(" 📈 Memory Usage Trend [cur: {}MiB | peak: {}MiB | alloc: {}MiB] ", cur_mem, peak_mem, d.mem_allocatable_mib);
    let mem_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::BORDER))
        .title(Span::styled(mem_title, Style::default().fg(Color::Rgb(168, 85, 247)).add_modifier(Modifier::BOLD)));
    let mem_inner = mem_block.inner(h_chunks[1]);
    f.render_widget(mem_block, h_chunks[1]);

    if state.mem_history.is_empty() {
        let p = Paragraph::new(Line::from(Span::styled("⚡ Awaiting metrics-server samples...", Style::default().fg(Theme::DIM))));
        f.render_widget(p, mem_inner);
    } else {
        let splits = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(8), // Y-axis labels & tick marks
                Constraint::Min(10),   // Canvas line plot
            ])
            .split(mem_inner);

        let (mem_y_floor, mem_y_ceil, mem_y_top, mem_y_mid, mem_y_bot) = compute_y_bounds(min_mem, peak_mem);

        let y_height = splits[0].height as usize;
        let mut y_spans = Vec::new();
        if y_height >= 3 {
            y_spans.push(Line::from(Span::styled(format!("{:>6} ┤", format_axis_val(mem_y_top, true)), Style::default().fg(Theme::DIM))));
            let blanks = y_height.saturating_sub(3);
            let top_blanks = blanks / 2;
            let bot_blanks = blanks - top_blanks;
            for _ in 0..top_blanks {
                y_spans.push(Line::from(Span::styled("       │", Style::default().fg(Theme::DIM))));
            }
            y_spans.push(Line::from(Span::styled(format!("{:>6} ┤", format_axis_val(mem_y_mid, true)), Style::default().fg(Theme::DIM))));
            for _ in 0..bot_blanks {
                y_spans.push(Line::from(Span::styled("       │", Style::default().fg(Theme::DIM))));
            }
            y_spans.push(Line::from(Span::styled(format!("{:>6} ┤", format_axis_val(mem_y_bot, true)), Style::default().fg(Theme::DIM))));
        } else if y_height >= 2 {
            y_spans.push(Line::from(Span::styled(format!("{:>6} ┤", format_axis_val(mem_y_top, true)), Style::default().fg(Theme::DIM))));
            for _ in 0..y_height.saturating_sub(2) {
                y_spans.push(Line::from(Span::styled("       │", Style::default().fg(Theme::DIM))));
            }
            y_spans.push(Line::from(Span::styled(format!("{:>6} ┤", format_axis_val(mem_y_bot, true)), Style::default().fg(Theme::DIM))));
        } else {
            y_spans.push(Line::from(Span::styled(format!("{:>6} ┤", format_axis_val(mem_y_top, true)), Style::default().fg(Theme::DIM))));
        }
        f.render_widget(Paragraph::new(y_spans), splits[0]);

        let n = state.mem_history.len();
        let x_max = (n.saturating_sub(1)).max(1) as f64;
        let mem_pts = state.mem_history.clone();

        let canvas = Canvas::default()
            .x_bounds([0.0, x_max])
            .y_bounds([mem_y_floor, mem_y_ceil])
            .paint(move |ctx| {
                let y_mid_f = (mem_y_floor + mem_y_ceil) / 2.0;
                ctx.draw(&CanvasLine {
                    x1: 0.0,
                    y1: y_mid_f,
                    x2: x_max,
                    y2: y_mid_f,
                    color: Color::Rgb(45, 55, 72),
                });

                if mem_pts.len() == 1 {
                    ctx.draw(&CanvasLine {
                        x1: 0.0,
                        y1: mem_pts[0] as f64,
                        x2: x_max,
                        y2: mem_pts[0] as f64,
                        color: Color::Rgb(168, 85, 247),
                    });
                } else if mem_pts.len() > 1 {
                    for i in 0..mem_pts.len() - 1 {
                        ctx.draw(&CanvasLine {
                            x1: i as f64,
                            y1: mem_pts[i] as f64,
                            x2: (i + 1) as f64,
                            y2: mem_pts[i + 1] as f64,
                            color: Color::Rgb(168, 85, 247),
                        });
                    }
                }
            });
        f.render_widget(canvas, splits[1]);
    }
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
        let model_label = d.gpu_model.as_deref().unwrap_or("GPU");

        let (gpu_title, gpu_pct) = if d.gpu_memory_requests_mib > 0 && d.gpu_memory_total_mib.unwrap_or(0) > 0 {
            let total_vram_mib = d.gpu_memory_total_mib.unwrap_or(15360);
            let req_vram_gib = d.gpu_memory_requests_mib as f64 / 1024.0;
            let total_vram_gib = total_vram_mib as f64 / 1024.0;
            let pct = (((d.gpu_memory_requests_mib as f64 / total_vram_mib as f64) * 100.0).round() as u64).min(999) as u16;
            (
                format!(" ⚡ {}: {:.1}/{:.1} GiB VRAM ({}%) ", model_label, req_vram_gib, total_vram_gib, pct),
                pct,
            )
        } else {
            let gpu_alloc = d.gpu_allocatable_count.max(d.gpu_capacity_count).max(1);
            let pct = (((d.gpu_requests_count as f64 / gpu_alloc as f64) * 100.0).round() as u64).min(999) as u16;
            let title = if d.gpu_allocatable_count > 1 {
                format!(" ⚡ {}: {}/{} Slices ({}%) ", model_label, d.gpu_requests_count, gpu_alloc, pct)
            } else {
                format!(" ⚡ {}: {}/{} ({}%) ", model_label, d.gpu_requests_count, gpu_alloc, pct)
            };
            (title, pct)
        };

        let gpu_color = if gpu_pct > 90 {
            Theme::RED
        } else if gpu_pct > 75 {
            Theme::YELLOW
        } else {
            Theme::YELLOW
        };

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

    // Conditions
    spans.push(Span::styled("Conditions: ", Style::default().fg(Theme::CYAN).add_modifier(Modifier::BOLD)));
    if d.conditions.is_empty() {
        spans.push(Span::styled("None", Style::default().fg(Theme::DIM)));
    } else {
        for (i, cond) in d.conditions.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(" ", Style::default()));
            }
            let is_ok = (cond.type_ == "Ready" && cond.status == "True")
                || (cond.type_ != "Ready" && cond.status == "False");
            let style = if is_ok {
                Style::default().fg(Theme::GREEN)
            } else {
                Style::default().fg(Theme::RED).add_modifier(Modifier::BOLD)
            };
            spans.push(Span::styled(format!("{}:{}", cond.type_, cond.status), style));
        }
    }

    spans.push(Span::styled("  │  ", Style::default().fg(Theme::DIM)));

    // Taints
    spans.push(Span::styled("Taints: ", Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)));
    if d.taints.is_empty() {
        spans.push(Span::styled("None", Style::default().fg(Theme::DIM)));
    } else {
        for (i, taint) in d.taints.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(" ", Style::default()));
            }
            let t_str = if let Some(val) = &taint.value {
                format!("{}={}:{}", taint.key, val, taint.effect)
            } else {
                format!("{}:{}", taint.key, taint.effect)
            };
            spans.push(Span::styled(t_str, Style::default().fg(Theme::YELLOW)));
        }
    }

    let p = Paragraph::new(Line::from(spans));
    f.render_widget(p, inner);
}

fn render_pods_table(f: &mut Frame, area: Rect, state: &NodeInspectorState, d: &NodeInspectorDetails) {
    let gpu_pod_count = d.pods.iter().filter(|p| p.gpu_requests > 0 || p.gpu_mem_requests_mib > 0).count();
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

    state.last_pods_table_rect.set(inner);
    state.last_scroll_offset.set(scroll);

    let mut lines = Vec::new();

    // Dynamically calculate column widths based on longest content: max(header, items) + 1
    let mut max_ns = "NAMESPACE".len();
    let mut max_name = "NAME".len();
    let mut max_ip = "IP".len();
    let mut max_status = "STATUS".len();
    let mut max_ready = "READY".len();
    let mut max_rest = "REST".len();
    let mut max_cpu = "CPU REQ".len();
    let mut max_mem = "MEM REQ".len();
    let mut max_gpu = "GPU REQ".len();
    let mut max_age = "AGE".len();

    for pod in &d.pods {
        max_ns = max_ns.max(pod.namespace.len());
        max_name = max_name.max(pod.name.len());
        let ip_len = if pod.pod_ip.is_empty() { 1 } else { pod.pod_ip.len() };
        max_ip = max_ip.max(ip_len);
        max_status = max_status.max(pod.phase.len());
        max_ready = max_ready.max(pod.ready_containers.len());
        let rest_len = if pod.restarts >= 1000 { 5 } else if pod.restarts >= 100 { 3 } else { pod.restarts.to_string().len() };
        max_rest = max_rest.max(rest_len);

        let cpu_len = if pod.cpu_requests_millicores >= 1000 {
            format!("{:.1}c", pod.cpu_requests_millicores as f64 / 1000.0).len()
        } else if pod.cpu_requests_millicores > 0 {
            format!("{}m", pod.cpu_requests_millicores).len()
        } else {
            1
        };
        max_cpu = max_cpu.max(cpu_len);

        let mem_len = if pod.mem_requests_mib >= 1024 {
            format!("{:.1} GiB", pod.mem_requests_mib as f64 / 1024.0).len()
        } else if pod.mem_requests_mib > 0 {
            format!("{} MiB", pod.mem_requests_mib).len()
        } else {
            1
        };
        max_mem = max_mem.max(mem_len);

        let gpu_len = if pod.gpu_mem_requests_mib > 0 {
            if pod.gpu_mem_requests_mib >= 1024 {
                format!("⚡ {:.1} GiB", pod.gpu_mem_requests_mib as f64 / 1024.0).chars().count()
            } else {
                format!("⚡ {} MiB", pod.gpu_mem_requests_mib).chars().count()
            }
        } else if pod.gpu_requests > 0 {
            format!("⚡ {} GPU", pod.gpu_requests).chars().count()
        } else {
            1
        };
        max_gpu = max_gpu.max(gpu_len);
        max_age = max_age.max(pod.age.len());
    }

    let ns_w = max_ns + 1;
    let name_w = max_name + 1;
    let ip_w = max_ip + 1;
    let status_w = max_status + 1;
    let ready_w = max_ready + 1;
    let rest_w = max_rest + 1;
    let cpu_w = max_cpu + 1;
    let mem_w = max_mem + 1;
    let gpu_w = max_gpu + 1;
    let age_w = max_age + 1;

    // Table Header
    let header_line = Line::from(vec![
        Span::styled("  ", Style::default()),
        Span::styled(format!("{:<width$}", "NAMESPACE", width = ns_w), Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)),
        Span::styled(format!("{:<width$}", "NAME", width = name_w), Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)),
        Span::styled(format!("{:<width$}", "IP", width = ip_w), Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)),
        Span::styled(format!("{:<width$}", "STATUS", width = status_w), Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)),
        Span::styled(format!("{:<width$}", "READY", width = ready_w), Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)),
        Span::styled(format!("{:<width$}", "REST", width = rest_w), Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)),
        Span::styled(format!("{:<width$}", "CPU REQ", width = cpu_w), Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)),
        Span::styled(format!("{:<width$}", "MEM REQ", width = mem_w), Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)),
        Span::styled(format!("{:<width$}", "GPU REQ", width = gpu_w), Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)),
        Span::styled(format!("{:<width$}", "AGE", width = age_w), Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)),
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

        let ip_str = if pod.pod_ip.is_empty() { "-" } else { &pod.pod_ip };

        let status_color = match pod.phase.as_str() {
            "Running" => Theme::GREEN,
            "Succeeded" => Theme::CYAN,
            "Pending" => Theme::YELLOW,
            "Failed" | "CrashLoopBackOff" | "Error" => Theme::RED,
            _ => Theme::FG,
        };

        let cpu_str = if pod.cpu_requests_millicores >= 1000 {
            format!("{:.1}c", pod.cpu_requests_millicores as f64 / 1000.0)
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

        let (gpu_str, gpu_style) = if pod.gpu_mem_requests_mib > 0 {
            let vram_str = if pod.gpu_mem_requests_mib >= 1024 {
                format!("⚡ {:.1} GiB", pod.gpu_mem_requests_mib as f64 / 1024.0)
            } else {
                format!("⚡ {} MiB", pod.gpu_mem_requests_mib)
            };
            (
                vram_str,
                Style::default().fg(Theme::YELLOW).bg(row_bg).add_modifier(Modifier::BOLD),
            )
        } else if pod.gpu_requests > 0 {
            (
                format!("⚡ {} GPU", pod.gpu_requests),
                Style::default().fg(Theme::YELLOW).bg(row_bg).add_modifier(Modifier::BOLD),
            )
        } else {
            ("-".to_string(), Style::default().fg(Theme::DIM).bg(row_bg))
        };

        let row = Line::from(vec![
            Span::styled(marker, marker_style),
            Span::styled(format!("{:<width$}", pod.namespace, width = ns_w), Style::default().fg(Theme::CYAN).bg(row_bg)),
            Span::styled(format!("{:<width$}", pod.name, width = name_w), Style::default().fg(if is_selected { Theme::ACCENT } else { Theme::FG }).bg(row_bg).add_modifier(if is_selected { Modifier::BOLD } else { Modifier::empty() })),
            Span::styled(format!("{:<width$}", ip_str, width = ip_w), Style::default().fg(Theme::DIM).bg(row_bg)),
            Span::styled(format!("{:<width$}", pod.phase, width = status_w), Style::default().fg(status_color).bg(row_bg)),
            Span::styled(format!("{:<width$}", pod.ready_containers, width = ready_w), Style::default().fg(Theme::FG).bg(row_bg)),
            Span::styled(format!("{:<width$}", pod.restarts, width = rest_w), Style::default().fg(if pod.restarts > 0 { Theme::YELLOW } else { Theme::DIM }).bg(row_bg)),
            Span::styled(format!("{:<width$}", cpu_str, width = cpu_w), Style::default().fg(Theme::FG).bg(row_bg)),
            Span::styled(format!("{:<width$}", mem_str, width = mem_w), Style::default().fg(Theme::FG).bg(row_bg)),
            Span::styled(format!("{:<width$}", gpu_str, width = gpu_w), gpu_style),
            Span::styled(format!("{:<width$}", pod.age, width = age_w), Style::default().fg(Theme::DIM).bg(row_bg)),
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
            gpu_memory_requests_mib: 0,
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
                    gpu_mem_requests_mib: 0,
                    pod_ip: "10.244.0.10".to_string(),
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
                    gpu_mem_requests_mib: 0,
                    pod_ip: "10.244.0.11".to_string(),
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
