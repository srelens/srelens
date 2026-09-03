use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::theme::Theme;

pub struct DescribeViewState {
    pub resource_name: String,
    pub resource_kind: String,
    pub namespace: Option<String>,
    pub content: String,
    pub lines: Vec<String>,
    pub scroll_offset: usize,
}

impl DescribeViewState {
    pub fn new(name: String, kind: String, namespace: Option<String>, content: String) -> Self {
        // Describe output embeds cluster data (event messages, annotations)
        // that can carry tabs/escapes which desync the terminal.
        let lines = content.lines().map(super::sanitize_span_text).collect();
        Self {
            resource_name: name,
            resource_kind: kind,
            namespace,
            content,
            lines,
            scroll_offset: 0,
        }
    }

    pub fn scroll_down(&mut self, n: usize) {
        if self.scroll_offset + n < self.lines.len() {
            self.scroll_offset += n;
        }
    }

    pub fn scroll_up(&mut self, n: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(n);
    }

    pub fn scroll_top(&mut self) {
        self.scroll_offset = 0;
    }

    pub fn scroll_bottom(&mut self) {
        if !self.lines.is_empty() {
            self.scroll_offset = self.lines.len().saturating_sub(1);
        }
    }
}

pub fn render_describe_view(f: &mut Frame, area: Rect, state: &DescribeViewState) {
    let title = format!(
        " Describe: {}/{} {} (Line {}/{}) [c: Copy all] [Esc: Back] ",
        state.resource_kind,
        state.resource_name,
        state.namespace.as_deref().map(|ns| format!("({})", ns)).unwrap_or_default(),
        state.scroll_offset + 1,
        state.lines.len()
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::BORDER_FOCUS))
        .title(Span::styled(title, Theme::title()));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let visible_lines = inner.height as usize;
    let end_idx = (state.scroll_offset + visible_lines).min(state.lines.len());

    let mut rendered_lines = Vec::new();

    for line in state.lines.iter().take(end_idx).skip(state.scroll_offset) {
        // Style headers, keys, values
        let mut spans = Vec::new();
        if let Some((k, v)) = line.split_once(':') {
            spans.push(Span::styled(format!("{}:", k), Style::default().fg(Theme::CYAN).add_modifier(Modifier::BOLD)));
            spans.push(Span::styled(v.to_string(), Style::default().fg(Theme::FG)));
        } else if line.starts_with("Events:") || line.starts_with("Conditions:") || line.starts_with("Containers:") {
            spans.push(Span::styled(line.clone(), Style::default().fg(Theme::ACCENT).add_modifier(Modifier::BOLD)));
        } else {
            spans.push(Span::styled(line.clone(), Style::default().fg(Theme::FG)));
        }
        rendered_lines.push(Line::from(spans));
    }

    let paragraph = Paragraph::new(rendered_lines);
    f.render_widget(paragraph, inner);
}
