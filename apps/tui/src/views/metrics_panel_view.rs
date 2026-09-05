use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Sparkline},
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
}

#[derive(Debug, Clone)]
pub struct MetricsPanelState {
    pub target_kind: String, // "Pod" or "Node"
    pub target_name: String,
    pub namespace: Option<String>,
    pub range: MetricsTimeRange,
    pub samples: Vec<MetricSample>,
}

impl MetricsPanelState {
    pub fn new(target_kind: String, target_name: String, namespace: Option<String>, samples: Vec<MetricSample>) -> Self {
        Self {
            target_kind,
            target_name,
            namespace,
            range: MetricsTimeRange::FiveMin,
            samples,
        }
    }

    pub fn cycle_time_range(&mut self) {
        self.range = self.range.next();
    }

    pub fn update_samples(&mut self, samples: &[MetricSample]) {
        self.samples = samples.to_vec();
    }
}

/// Renders the interactive Metrics Panel modal overlay with real-time Sparklines.
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
        range_spans.push(Span::styled(format!(" [{}: {}] ", idx + 1, r.label()), style));
        range_spans.push(Span::raw(" "));
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

    // Filter samples within window
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

    let cpu_data: Vec<u64> = active_samples.iter().map(|s| s.cpu_millicores).collect();
    let mem_data: Vec<u64> = active_samples.iter().map(|s| s.memory_mib).collect();

    let cur_cpu = cpu_data.last().copied().unwrap_or(0);
    let min_cpu = cpu_data.iter().copied().min().unwrap_or(0);
    let max_cpu = cpu_data.iter().copied().max().unwrap_or(0);
    let avg_cpu = if !cpu_data.is_empty() {
        cpu_data.iter().sum::<u64>() / cpu_data.len() as u64
    } else {
        0
    };

    let cur_mem = mem_data.last().copied().unwrap_or(0);
    let min_mem = mem_data.iter().copied().min().unwrap_or(0);
    let max_mem = mem_data.iter().copied().max().unwrap_or(0);
    let avg_mem = if !mem_data.is_empty() {
        mem_data.iter().sum::<u64>() / mem_data.len() as u64
    } else {
        0
    };

    // 1. CPU Sparkline Box
    let cpu_title = format!(
        " CPU Usage: {}m  (min: {}m, avg: {}m, peak: {}m) ",
        cur_cpu, min_cpu, avg_cpu, max_cpu
    );
    let cpu_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::BORDER))
        .title(Span::styled(cpu_title, Style::default().fg(Theme::CYAN).add_modifier(Modifier::BOLD)));

    let cpu_inner = cpu_block.inner(body_chunks[1]);
    f.render_widget(cpu_block, body_chunks[1]);

    let cpu_sparkline = Sparkline::default()
        .data(&cpu_data)
        .style(Style::default().fg(Theme::CYAN))
        .max(max_cpu.max(10));
    f.render_widget(cpu_sparkline, cpu_inner);

    // 2. Memory Sparkline Box
    let mem_title = format!(
        " Memory Usage: {} MiB  (min: {} MiB, avg: {} MiB, peak: {} MiB) ",
        cur_mem, min_mem, avg_mem, max_mem
    );
    let mem_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::BORDER))
        .title(Span::styled(mem_title, Style::default().fg(Color::Rgb(168, 85, 247)).add_modifier(Modifier::BOLD)));

    let mem_inner = mem_block.inner(body_chunks[2]);
    f.render_widget(mem_block, body_chunks[2]);

    let mem_sparkline = Sparkline::default()
        .data(&mem_data)
        .style(Style::default().fg(Color::Rgb(168, 85, 247)))
        .max(max_mem.max(10));
    f.render_widget(mem_sparkline, mem_inner);

    // 3. Footer Key Hints
    let footer_hints = Line::from(vec![
        Span::styled("<Tab/1-4>", Theme::header_label()),
        Span::raw(" Switch Range  "),
        Span::styled("<r>", Theme::header_label()),
        Span::raw(" Refresh  "),
        Span::styled("<Esc>", Theme::header_label()),
        Span::raw(" Close"),
    ]);
    f.render_widget(Paragraph::new(footer_hints).alignment(Alignment::Center), body_chunks[3]);
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
