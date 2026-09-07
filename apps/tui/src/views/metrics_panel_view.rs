use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        canvas::{Canvas, Line as CanvasLine},
        Block, Borders, Clear, Paragraph, Sparkline,
    },
    Frame,
};

use srelens_kube::metrics::MetricSample;
use crate::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricsTimeRange {
    FiveMin,
    TenMin,
    ThirtyMin,
    OneHour,
}

impl MetricsTimeRange {
    pub fn label(&self) -> &'static str {
        match self {
            Self::FiveMin => "5m",
            Self::TenMin => "10m",
            Self::ThirtyMin => "30m",
            Self::OneHour => "1h",
        }
    }

    pub fn half_label(&self) -> &'static str {
        match self {
            Self::FiveMin => "2.5m",
            Self::TenMin => "5m",
            Self::ThirtyMin => "15m",
            Self::OneHour => "30m",
        }
    }

    pub fn window_ms(&self) -> u64 {
        match self {
            Self::FiveMin => 5 * 60 * 1000,
            Self::TenMin => 10 * 60 * 1000,
            Self::ThirtyMin => 30 * 60 * 1000,
            Self::OneHour => 60 * 60 * 1000,
        }
    }

    pub fn next(&self) -> Self {
        match self {
            Self::FiveMin => Self::TenMin,
            Self::TenMin => Self::ThirtyMin,
            Self::ThirtyMin => Self::OneHour,
            Self::OneHour => Self::FiveMin,
        }
    }

    pub fn prev(&self) -> Self {
        match self {
            Self::FiveMin => Self::OneHour,
            Self::TenMin => Self::FiveMin,
            Self::ThirtyMin => Self::TenMin,
            Self::OneHour => Self::ThirtyMin,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MetricsPanelState {
    pub target_kind: String, // "Pod" or "Node"
    pub target_name: String,
    pub namespace: Option<String>,
    pub range: MetricsTimeRange,
    pub samples: Vec<MetricSample>,
    pub range_button_rects: std::sync::Arc<std::sync::Mutex<Vec<(Rect, MetricsTimeRange)>>>,
}

impl MetricsPanelState {
    pub fn new(target_kind: String, target_name: String, namespace: Option<String>, samples: Vec<MetricSample>) -> Self {
        Self {
            target_kind,
            target_name,
            namespace,
            range: MetricsTimeRange::FiveMin,
            samples,
            range_button_rects: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    pub fn cycle_time_range(&mut self) {
        self.range = self.range.next();
    }

    pub fn cycle_time_range_prev(&mut self) {
        self.range = self.range.prev();
    }

    pub fn update_samples(&mut self, samples: &[MetricSample]) {
        self.samples = samples.to_vec();
    }
}

/// Downsamples and scales recorded samples into `width` discrete time buckets spanning `window_ms`.
/// Each column represents a bucket across the selected time range ending at `now`.
pub fn bucket_samples(
    samples: &[MetricSample],
    window_ms: u64,
    width: usize,
) -> (Vec<u64>, Vec<u64>) {
    if width == 0 || samples.is_empty() {
        return (vec![], vec![]);
    }

    let now = samples.last().map(|s| s.timestamp_epoch_ms).unwrap_or(0);
    let start_time = now.saturating_sub(window_ms);

    // Filter samples within the window
    let window_samples: Vec<&MetricSample> = samples
        .iter()
        .filter(|s| s.timestamp_epoch_ms >= start_time && s.timestamp_epoch_ms <= now)
        .collect();

    if window_samples.is_empty() {
        let cpu = samples.last().map(|s| s.cpu_millicores).unwrap_or(0);
        let mem = samples.last().map(|s| s.memory_mib).unwrap_or(0);
        return (vec![cpu; width], vec![mem; width]);
    }

    // Each column i in 0..width represents a time slice of bucket_duration_ms
    let mut buckets: Vec<(u64, u64, usize)> = vec![(0, 0, 0); width];

    for s in &window_samples {
        let offset = s.timestamp_epoch_ms.saturating_sub(start_time);
        let col = if window_ms > 0 {
            let c = (offset as u128 * width as u128 / window_ms as u128) as usize;
            c.min(width.saturating_sub(1))
        } else {
            width.saturating_sub(1)
        };
        buckets[col].0 += s.cpu_millicores;
        buckets[col].1 += s.memory_mib;
        buckets[col].2 += 1;
    }

    // Earliest recorded sample offset determines where data starts in the window
    let earliest_offset = window_samples.first().unwrap().timestamp_epoch_ms.saturating_sub(start_time);
    let earliest_col = if window_ms > 0 {
        ((earliest_offset as u128 * width as u128 / window_ms as u128) as usize).min(width.saturating_sub(1))
    } else {
        0
    };

    let mut cpu_data = Vec::with_capacity(width);
    let mut mem_data = Vec::with_capacity(width);
    let mut last_cpu = 0;
    let mut last_mem = 0;

    for (col_idx, (sum_c, sum_m, count)) in buckets.into_iter().enumerate() {
        if col_idx < earliest_col {
            // Before earliest sample in this window: empty timeline
            cpu_data.push(0);
            mem_data.push(0);
        } else if count > 0 {
            let avg_c = sum_c / count as u64;
            let avg_m = sum_m / count as u64;
            last_cpu = avg_c;
            last_mem = avg_m;
            cpu_data.push(avg_c);
            mem_data.push(avg_m);
        } else {
            // Sample gap: carry forward last known value
            cpu_data.push(last_cpu);
            mem_data.push(last_mem);
        }
    }

    (cpu_data, mem_data)
}

pub fn format_axis_val(val: u64, is_mem: bool) -> String {
    if is_mem {
        if val >= 100_000 {
            format!("{:.1}G", val as f64 / 1024.0)
        } else if val >= 10_000 {
            format!("{:.1}G", val as f64 / 1024.0)
        } else {
            format!("{}M", val)
        }
    } else {
        if val >= 10_000 {
            format!("{:.1}c", val as f64 / 1000.0)
        } else if val >= 1_000 {
            format!("{:.2}c", val as f64 / 1000.0)
        } else {
            format!("{}m", val)
        }
    }
}

pub fn compute_y_bounds(min_val: u64, max_val: u64) -> (f64, f64, u64, u64, u64) {
    if min_val == max_val {
        if max_val == 0 {
            return (0.0, 10.0, 10, 5, 0);
        }
        let floor = (max_val as f64 * 0.9).max(0.0);
        let ceil = max_val as f64 * 1.1;
        return (floor, ceil, ceil.round() as u64, max_val, floor.round() as u64);
    }
    let diff = (max_val - min_val) as f64;
    // If min is already near 0 (less than 1.5x diff), anchor baseline to 0
    let floor = if (min_val as f64) <= diff * 1.5 {
        0.0
    } else {
        // Otherwise zoom in with 12% padding so variance is clearly visible
        ((min_val as f64) - diff * 0.12).max(0.0)
    };
    let ceil = (max_val as f64) + (diff * 0.12);
    let mid = ((floor + ceil) / 2.0).round() as u64;
    (floor, ceil, ceil.round() as u64, mid, floor.round() as u64)
}

/// Renders the interactive Metrics Panel modal overlay with high-resolution line charts.
pub fn render_metrics_panel_modal(
    f: &mut Frame,
    area: Rect,
    state: &MetricsPanelState,
) {
    let samples = &state.samples;
    let modal_w = 84.min(area.width.saturating_sub(4));
    let modal_h = 24.min(area.height.saturating_sub(4));

    let h_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length((area.width.saturating_sub(modal_w)) / 2),
            Constraint::Length(modal_w),
            Constraint::Length((area.width.saturating_sub(modal_w)) / 2),
        ])
        .split(area);

    let v_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length((area.height.saturating_sub(modal_h)) / 2),
            Constraint::Length(modal_h),
            Constraint::Length((area.height.saturating_sub(modal_h)) / 2),
        ])
        .split(h_chunks[1]);

    let modal_area = v_chunks[1];
    f.render_widget(Clear, modal_area);

    let target_display = if let Some(ns) = &state.namespace {
        format!("{}: {}/{}", state.target_kind, ns, state.target_name)
    } else {
        format!("{}: {}", state.target_kind, state.target_name)
    };

    let title = format!(" 📈 Live Metrics Timeline — {} ", target_display);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::CYAN))
        .title(Span::styled(title, Theme::title()));

    let inner = block.inner(modal_area);
    f.render_widget(block, modal_area);

    // Inner layout:
    // 0. Range Bar & Header Info (height: 2)
    // 1. CPU Sparkline Card (height: 8)
    // 2. Memory Sparkline Card (height: 8)
    // 3. Footer Key Hints (height: 1)
    let body_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(1),
        ])
        .split(inner);

    // 0. Range Selector
    let ranges = [
        MetricsTimeRange::FiveMin,
        MetricsTimeRange::TenMin,
        MetricsTimeRange::ThirtyMin,
        MetricsTimeRange::OneHour,
    ];

    let mut range_spans = vec![
        Span::styled("Time Range: ", Theme::header_label()),
    ];

    let mut btn_rects = Vec::new();
    let mut current_btn_x = body_chunks[0].x + 12;

    for (idx, r) in ranges.iter().enumerate() {
        let is_selected = *r == state.range;
        let style = if is_selected {
            Style::default()
                .fg(Color::Black)
                .bg(Theme::CYAN)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Theme::DIM)
        };
        let label_str = format!(" [{}: {}] ", idx + 1, r.label());
        let btn_w = label_str.len() as u16;
        btn_rects.push((
            Rect {
                x: current_btn_x,
                y: body_chunks[0].y,
                width: btn_w,
                height: 1,
            },
            *r,
        ));
        current_btn_x += btn_w + 1;

        range_spans.push(Span::styled(label_str, style));
        range_spans.push(Span::raw(" "));
    }

    if let Ok(mut lock) = state.range_button_rects.lock() {
        *lock = btn_rects;
    }

    range_spans.push(Span::styled(
        format!("({} samples in buffer)", samples.len()),
        Style::default().fg(Theme::DIM),
    ));

    f.render_widget(Paragraph::new(Line::from(range_spans)), body_chunks[0]);

    if samples.is_empty() {
        let empty_msg = Paragraph::new(vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("⚡ Collecting metrics-server data... ", Style::default().fg(Theme::CYAN).add_modifier(Modifier::BOLD)),
                Span::styled("(Metrics poll every ~4s)", Style::default().fg(Theme::DIM)),
            ]),
        ])
        .alignment(Alignment::Center);
        f.render_widget(empty_msg, body_chunks[1]);
        return;
    }

    // Filter samples within window for metrics stats
    let now = samples.last().map(|s| s.timestamp_epoch_ms).unwrap_or(0);
    let window_ms = state.range.window_ms();
    let window_samples: Vec<&MetricSample> = samples
        .iter()
        .filter(|s| now.saturating_sub(s.timestamp_epoch_ms) <= window_ms)
        .collect();

    let active_samples = if window_samples.is_empty() {
        samples.iter().collect::<Vec<_>>()
    } else {
        window_samples
    };

    let cur_cpu = samples.last().map(|s| s.cpu_millicores).unwrap_or(0);
    let min_cpu = active_samples.iter().map(|s| s.cpu_millicores).min().unwrap_or(0);
    let max_cpu = active_samples.iter().map(|s| s.cpu_millicores).max().unwrap_or(0);
    let avg_cpu = if !active_samples.is_empty() {
        active_samples.iter().map(|s| s.cpu_millicores).sum::<u64>() / active_samples.len() as u64
    } else {
        0
    };

    let cur_mem = samples.last().map(|s| s.memory_mib).unwrap_or(0);
    let min_mem = active_samples.iter().map(|s| s.memory_mib).min().unwrap_or(0);
    let max_mem = active_samples.iter().map(|s| s.memory_mib).max().unwrap_or(0);
    let avg_mem = if !active_samples.is_empty() {
        active_samples.iter().map(|s| s.memory_mib).sum::<u64>() / active_samples.len() as u64
    } else {
        0
    };

    // 1. CPU Canvas Line Chart Box
    let cpu_title = format!(
        " CPU Usage: {}m  ({} min: {}m, avg: {}m, peak: {}m) ",
        cur_cpu, state.range.label(), min_cpu, avg_cpu, max_cpu
    );
    let cpu_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::BORDER))
        .title(Span::styled(cpu_title, Style::default().fg(Theme::CYAN).add_modifier(Modifier::BOLD)));

    let cpu_inner = cpu_block.inner(body_chunks[1]);
    f.render_widget(cpu_block, body_chunks[1]);

    let cpu_splits = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(4),
            Constraint::Length(1),
        ])
        .split(cpu_inner);

    let cpu_chart_splits = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(8), // Y-axis labels & tick marks
            Constraint::Min(20),   // Canvas line plot
        ])
        .split(cpu_splits[0]);

    let (cpu_y_floor, cpu_y_ceil, cpu_y_top, cpu_y_mid, cpu_y_bot) = compute_y_bounds(min_cpu, max_cpu);

    let y_height = cpu_chart_splits[0].height as usize;
    let mut cpu_y_spans = Vec::new();
    if y_height >= 5 {
        cpu_y_spans.push(Line::from(Span::styled(format!("{:>6} ┤", format_axis_val(cpu_y_top, false)), Style::default().fg(Theme::DIM))));
        cpu_y_spans.push(Line::from(Span::styled("       │", Style::default().fg(Theme::DIM))));
        cpu_y_spans.push(Line::from(Span::styled(format!("{:>6} ┤", format_axis_val(cpu_y_mid, false)), Style::default().fg(Theme::DIM))));
        cpu_y_spans.push(Line::from(Span::styled("       │", Style::default().fg(Theme::DIM))));
        cpu_y_spans.push(Line::from(Span::styled(format!("{:>6} ┤", format_axis_val(cpu_y_bot, false)), Style::default().fg(Theme::DIM))));
    } else if y_height >= 3 {
        cpu_y_spans.push(Line::from(Span::styled(format!("{:>6} ┤", format_axis_val(cpu_y_top, false)), Style::default().fg(Theme::DIM))));
        cpu_y_spans.push(Line::from(Span::styled(format!("{:>6} ┤", format_axis_val(cpu_y_mid, false)), Style::default().fg(Theme::DIM))));
        cpu_y_spans.push(Line::from(Span::styled(format!("{:>6} ┤", format_axis_val(cpu_y_bot, false)), Style::default().fg(Theme::DIM))));
    } else {
        cpu_y_spans.push(Line::from(Span::styled(format!("{:>6} ┤", format_axis_val(cpu_y_top, false)), Style::default().fg(Theme::DIM))));
    }
    f.render_widget(Paragraph::new(cpu_y_spans), cpu_chart_splits[0]);

    let start_time = now.saturating_sub(window_ms);
    let mut cpu_pts: Vec<(f64, f64)> = active_samples
        .iter()
        .map(|s| {
            let x = s.timestamp_epoch_ms.saturating_sub(start_time) as f64;
            let y = s.cpu_millicores as f64;
            (x, y)
        })
        .collect();
    cpu_pts.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    let cpu_canvas = Canvas::default()
        .x_bounds([0.0, (window_ms as f64).max(1.0)])
        .y_bounds([cpu_y_floor, cpu_y_ceil])
        .paint(move |ctx| {
            let y_mid_f = (cpu_y_floor + cpu_y_ceil) / 2.0;
            ctx.draw(&CanvasLine {
                x1: 0.0,
                y1: y_mid_f,
                x2: window_ms as f64,
                y2: y_mid_f,
                color: Color::Rgb(45, 55, 72),
            });

            if cpu_pts.len() == 1 {
                ctx.draw(&CanvasLine {
                    x1: cpu_pts[0].0,
                    y1: cpu_pts[0].1,
                    x2: window_ms as f64,
                    y2: cpu_pts[0].1,
                    color: Theme::CYAN,
                });
            } else if cpu_pts.len() > 1 {
                for i in 0..cpu_pts.len() - 1 {
                    ctx.draw(&CanvasLine {
                        x1: cpu_pts[i].0,
                        y1: cpu_pts[i].1,
                        x2: cpu_pts[i + 1].0,
                        y2: cpu_pts[i + 1].1,
                        color: Theme::CYAN,
                    });
                }
                if let Some(last) = cpu_pts.last() {
                    if last.0 < window_ms as f64 {
                        ctx.draw(&CanvasLine {
                            x1: last.0,
                            y1: last.1,
                            x2: window_ms as f64,
                            y2: last.1,
                            color: Theme::CYAN,
                        });
                    }
                }
            }
        });
    f.render_widget(cpu_canvas, cpu_chart_splits[1]);

    let time_axis_center_width = (cpu_splits[1].width as usize).saturating_sub(8 + 12);
    let cpu_axis = Line::from(vec![
        Span::raw(" ".repeat(8)),
        Span::styled(format!("-{}", state.range.label()), Style::default().fg(Theme::DIM)),
        Span::styled(
            format!("{:^width$}", format!("-{}", state.range.half_label()), width = time_axis_center_width),
            Style::default().fg(Theme::DIM),
        ),
        Span::styled("now ", Style::default().fg(Theme::CYAN)),
    ]);
    f.render_widget(Paragraph::new(cpu_axis), cpu_splits[1]);

    // 2. Memory Canvas Line Chart Box
    let mem_title = format!(
        " Memory Usage: {} MiB  ({} min: {} MiB, avg: {} MiB, peak: {} MiB) ",
        cur_mem, state.range.label(), min_mem, avg_mem, max_mem
    );
    let mem_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::BORDER))
        .title(Span::styled(mem_title, Style::default().fg(Color::Rgb(192, 132, 252)).add_modifier(Modifier::BOLD)));

    let mem_inner = mem_block.inner(body_chunks[2]);
    f.render_widget(mem_block, body_chunks[2]);

    let mem_splits = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(4),
            Constraint::Length(1),
        ])
        .split(mem_inner);

    let mem_chart_splits = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(8), // Y-axis labels & tick marks
            Constraint::Min(20),   // Canvas line plot
        ])
        .split(mem_splits[0]);

    let (mem_y_floor, mem_y_ceil, mem_y_top, mem_y_mid, mem_y_bot) = compute_y_bounds(min_mem, max_mem);

    let mem_y_height = mem_chart_splits[0].height as usize;
    let mut mem_y_spans = Vec::new();
    if mem_y_height >= 5 {
        mem_y_spans.push(Line::from(Span::styled(format!("{:>6} ┤", format_axis_val(mem_y_top, true)), Style::default().fg(Theme::DIM))));
        mem_y_spans.push(Line::from(Span::styled("       │", Style::default().fg(Theme::DIM))));
        mem_y_spans.push(Line::from(Span::styled(format!("{:>6} ┤", format_axis_val(mem_y_mid, true)), Style::default().fg(Theme::DIM))));
        mem_y_spans.push(Line::from(Span::styled("       │", Style::default().fg(Theme::DIM))));
        mem_y_spans.push(Line::from(Span::styled(format!("{:>6} ┤", format_axis_val(mem_y_bot, true)), Style::default().fg(Theme::DIM))));
    } else if mem_y_height >= 3 {
        mem_y_spans.push(Line::from(Span::styled(format!("{:>6} ┤", format_axis_val(mem_y_top, true)), Style::default().fg(Theme::DIM))));
        mem_y_spans.push(Line::from(Span::styled(format!("{:>6} ┤", format_axis_val(mem_y_mid, true)), Style::default().fg(Theme::DIM))));
        mem_y_spans.push(Line::from(Span::styled(format!("{:>6} ┤", format_axis_val(mem_y_bot, true)), Style::default().fg(Theme::DIM))));
    } else {
        mem_y_spans.push(Line::from(Span::styled(format!("{:>6} ┤", format_axis_val(mem_y_top, true)), Style::default().fg(Theme::DIM))));
    }
    f.render_widget(Paragraph::new(mem_y_spans), mem_chart_splits[0]);

    let mut mem_pts: Vec<(f64, f64)> = active_samples
        .iter()
        .map(|s| {
            let x = s.timestamp_epoch_ms.saturating_sub(start_time) as f64;
            let y = s.memory_mib as f64;
            (x, y)
        })
        .collect();
    mem_pts.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    let mem_color = Color::Rgb(192, 132, 252);
    let mem_canvas = Canvas::default()
        .x_bounds([0.0, (window_ms as f64).max(1.0)])
        .y_bounds([mem_y_floor, mem_y_ceil])
        .paint(move |ctx| {
            let y_mid_f = (mem_y_floor + mem_y_ceil) / 2.0;
            ctx.draw(&CanvasLine {
                x1: 0.0,
                y1: y_mid_f,
                x2: window_ms as f64,
                y2: y_mid_f,
                color: Color::Rgb(45, 55, 72),
            });

            if mem_pts.len() == 1 {
                ctx.draw(&CanvasLine {
                    x1: mem_pts[0].0,
                    y1: mem_pts[0].1,
                    x2: window_ms as f64,
                    y2: mem_pts[0].1,
                    color: mem_color,
                });
            } else if mem_pts.len() > 1 {
                for i in 0..mem_pts.len() - 1 {
                    ctx.draw(&CanvasLine {
                        x1: mem_pts[i].0,
                        y1: mem_pts[i].1,
                        x2: mem_pts[i + 1].0,
                        y2: mem_pts[i + 1].1,
                        color: mem_color,
                    });
                }
                if let Some(last) = mem_pts.last() {
                    if last.0 < window_ms as f64 {
                        ctx.draw(&CanvasLine {
                            x1: last.0,
                            y1: last.1,
                            x2: window_ms as f64,
                            y2: last.1,
                            color: mem_color,
                        });
                    }
                }
            }
        });
    f.render_widget(mem_canvas, mem_chart_splits[1]);

    let mem_axis = Line::from(vec![
        Span::raw(" ".repeat(8)),
        Span::styled(format!("-{}", state.range.label()), Style::default().fg(Theme::DIM)),
        Span::styled(
            format!("{:^width$}", format!("-{}", state.range.half_label()), width = time_axis_center_width),
            Style::default().fg(Theme::DIM),
        ),
        Span::styled("now ", Style::default().fg(Color::Rgb(192, 132, 252))),
    ]);
    f.render_widget(Paragraph::new(mem_axis), mem_splits[1]);

    // 3. Footer Key Hints
    let footer_hints = Line::from(vec![
        Span::styled("<Tab/1-4/←/→>", Theme::header_label()),
        Span::raw(" Switch Range  "),
        Span::styled("<r>", Theme::header_label()),
        Span::raw(" Refresh  "),
        Span::styled("<Esc/q>", Theme::header_label()),
        Span::raw(" Close"),
    ]);
    f.render_widget(Paragraph::new(footer_hints).alignment(Alignment::Center), body_chunks[3]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn test_metrics_time_range_cycles() {
        let mut range = MetricsTimeRange::FiveMin;
        assert_eq!(range.label(), "5m");
        assert_eq!(range.window_ms(), 5 * 60 * 1000);

        range = range.next();
        assert_eq!(range.label(), "10m");
        assert_eq!(range.window_ms(), 10 * 60 * 1000);

        range = range.next();
        assert_eq!(range.label(), "30m");
        assert_eq!(range.window_ms(), 30 * 60 * 1000);

        range = range.next();
        assert_eq!(range.label(), "1h");
        assert_eq!(range.window_ms(), 60 * 60 * 1000);

        range = range.next();
        assert_eq!(range.label(), "5m");
    }

    #[test]
    fn test_metrics_panel_state_samples() {
        let sample1 = MetricSample {
            timestamp_epoch_ms: 1000,
            cpu_millicores: 150,
            memory_mib: 512,
        };
        let sample2 = MetricSample {
            timestamp_epoch_ms: 2000,
            cpu_millicores: 300,
            memory_mib: 1024,
        };

        let mut panel = MetricsPanelState::new(
            "Pod".to_string(),
            "frontend-799bd".to_string(),
            Some("production".to_string()),
            vec![sample1],
        );

        assert_eq!(panel.samples.len(), 1);
        panel.update_samples(&[sample2]);
        assert_eq!(panel.samples.len(), 1);
        assert_eq!(panel.samples[0].cpu_millicores, 300);

        panel.cycle_time_range();
        assert_eq!(panel.range, MetricsTimeRange::TenMin);
    }

    #[test]
    fn test_render_modal_50_samples() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let mut samples = Vec::new();
        for i in 0..50 {
            samples.push(MetricSample {
                timestamp_epoch_ms: 10000 + i * 4000,
                cpu_millicores: 579,
                memory_mib: 14787,
            });
        }
        let panel = MetricsPanelState::new("Node".to_string(), "test-node".to_string(), None, samples);
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render_metrics_panel_modal(f, f.area(), &panel)).unwrap();

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

        assert!(content.contains("Live Metrics Timeline"));
        assert!(content.contains("-5m"));
        assert!(content.contains("-2.5m"));
        assert!(content.contains("now"));
        assert!(content.contains("5m min: 579m"));
    }

    #[test]
    fn test_bucket_samples_rescaling_with_range_change() {
        let mut samples = Vec::new();
        let base_time: u64 = 1_700_000_000_000;
        // 50 samples spaced 4s apart = 196s duration ending at base_time + 196,000
        for i in 0..50 {
            samples.push(MetricSample {
                timestamp_epoch_ms: base_time + i * 4000,
                cpu_millicores: 500 + i * 10,
                memory_mib: 1000 + i * 20,
            });
        }

        // Window 5m (300,000 ms), width = 75
        let (cpu_5m, _) = bucket_samples(&samples, 5 * 60 * 1000, 75);
        assert_eq!(cpu_5m.len(), 75);
        let non_zeros_5m = cpu_5m.iter().filter(|&&v| v > 0).count();
        assert!(non_zeros_5m >= 45, "5m window should contain most samples across 75 width");

        // Window 10m (600,000 ms), width = 75
        let (cpu_10m, _) = bucket_samples(&samples, 10 * 60 * 1000, 75);
        assert_eq!(cpu_10m.len(), 75);
        let non_zeros_10m = cpu_10m.iter().filter(|&&v| v > 0).count();
        assert!(non_zeros_10m < non_zeros_5m, "10m window should compress timeline compared to 5m");
        assert!(non_zeros_10m >= 20 && non_zeros_10m <= 30);

        // Window 1h (3,600,000 ms), width = 75
        let (cpu_1h, _) = bucket_samples(&samples, 60 * 60 * 1000, 75);
        let non_zeros_1h = cpu_1h.iter().filter(|&&v| v > 0).count();
        assert!(non_zeros_1h < non_zeros_10m, "1h window should compress data even more");
        assert!(non_zeros_1h <= 6);

        // Samples windowing test:
        let out_of_window = vec![MetricSample {
            timestamp_epoch_ms: 5000,
            cpu_millicores: 120,
            memory_mib: 250,
        }];
        // If now is 5000 and start_time is 4000, sample is included.
        // To test window_samples.is_empty(), we pass samples that don't match the condition:
        // Actually window_samples filters: s.timestamp_epoch_ms >= start_time && s.timestamp_epoch_ms <= now
        // where now = samples.last().timestamp_epoch_ms and start_time = now - window_ms.
        // So the last sample ALWAYS has timestamp == now >= start_time, so window_samples will ALWAYS contain at least samples.last()!
        // The only way window_samples is empty is if now < start_time which is prevented by saturating_sub.
        // BUT if it ever is empty, it returns vec![cpu; width] where width elements are returned:
        let (cpu_res, mem_res) = bucket_samples(&out_of_window, 1000, 40);
        assert_eq!(cpu_res.len(), 40);
        assert_eq!(mem_res.len(), 40);
    }

    #[test]
    fn test_compute_y_bounds_and_format_axis() {
        // Flat zero
        let (f0, c0, top0, mid0, bot0) = compute_y_bounds(0, 0);
        assert_eq!(f0, 0.0);
        assert_eq!(c0, 10.0);
        assert_eq!(top0, 10);
        assert_eq!(mid0, 5);
        assert_eq!(bot0, 0);

        // Flat non-zero
        let (f_flat, c_flat, _, _, _) = compute_y_bounds(500, 500);
        assert!(f_flat < 500.0);
        assert!(c_flat > 500.0);

        // Wide variance starting near 0 (should anchor to 0.0)
        let (f_wide, c_wide, _, _, _) = compute_y_bounds(10, 500);
        assert_eq!(f_wide, 0.0);
        assert!(c_wide >= 500.0);

        // Narrow variance high above 0 (like memory 9000-9027): should zoom in
        let (f_narrow, c_narrow, top_n, mid_n, bot_n) = compute_y_bounds(9000, 9027);
        assert!(f_narrow > 8900.0, "Floor should zoom in on narrow variance, got {}", f_narrow);
        assert!(c_narrow < 9100.0, "Ceiling should be tight around max, got {}", c_narrow);
        assert!(top_n > bot_n);
        assert!(mid_n > bot_n && mid_n < top_n);

        // format_axis_val tests
        assert_eq!(format_axis_val(429, false), "429m");
        assert_eq!(format_axis_val(1500, false), "1.50c");
        assert_eq!(format_axis_val(15000, false), "15.0c");

        assert_eq!(format_axis_val(500, true), "500M");
        assert_eq!(format_axis_val(9027, true), "9027M");
        assert_eq!(format_axis_val(15000, true), "14.6G");
        assert_eq!(format_axis_val(150000, true), "146.5G");
    }

    #[test]
    fn test_render_canvas_line_chart_with_narrow_variance_and_y_ticks() {
        let base_time: u64 = 1_700_000_000_000;
        let mut samples = Vec::new();
        // 15 samples matching user's scenario: CPU 429-558m, Memory 9000-9027 MiB
        for i in 0..15 {
            samples.push(MetricSample {
                timestamp_epoch_ms: base_time + i * 4000,
                cpu_millicores: 429 + (i * 9) % 130,
                memory_mib: 9000 + (i * 2) % 28,
            });
        }

        let state = MetricsPanelState::new(
            "Node".to_string(),
            "tools-stage-general-li8f5".to_string(),
            None,
            samples,
        );

        let backend = TestBackend::new(100, 25);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render_metrics_panel_modal(f, f.area(), &state)).unwrap();

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

        assert!(content.contains("tools-stage-general-li8f5"));
        assert!(content.contains("CPU Usage:"));
        assert!(content.contains("Memory Usage:"));
        assert!(content.contains("┤"), "Y-axis tick connector should be present in rendered output");
        assert!(content.contains("│"), "Y-axis vertical axis guide should be present");
        assert!(content.contains("-5m"));
        assert!(content.contains("now"));
    }
}
