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
    pub search_query: String,
    pub search_matches: Vec<usize>,
    pub current_match_idx: Option<usize>,
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
            search_query: String::new(),
            search_matches: Vec::new(),
            current_match_idx: None,
        }
    }

    pub fn set_search_query(&mut self, query: &str) {
        self.search_query = query.to_string();
        if query.is_empty() {
            self.search_matches.clear();
            self.current_match_idx = None;
            return;
        }
        let q = query.to_lowercase();
        self.search_matches = self
            .lines
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
        self.search_matches.clear();
        self.current_match_idx = None;
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

    let title = format!(
        " Describe: {}/{} {} (Line {}/{}) [c: Copy]{} [/: Search] [Esc: Back] ",
        state.resource_kind,
        state.resource_name,
        state.namespace.as_deref().map(|ns| format!("({})", ns)).unwrap_or_default(),
        state.scroll_offset + 1,
        state.lines.len(),
        search_badge
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::BORDER_FOCUS))
        .title(Span::styled(title, Theme::title()));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let visible_lines = inner.height as usize;
    let end_idx = (state.scroll_offset + visible_lines).min(state.lines.len());

    let match_style = Style::default()
        .bg(Theme::YELLOW)
        .fg(Color::Rgb(20, 20, 20))
        .add_modifier(Modifier::BOLD);

    let mut rendered_lines = Vec::new();

    for line in state.lines.iter().take(end_idx).skip(state.scroll_offset) {
        let has_match = !state.search_query.is_empty()
            && line.to_lowercase().contains(&state.search_query.to_lowercase());

        if has_match {
            let spans = super::highlight_text_matches(line, &state.search_query, Style::default().fg(Theme::FG), match_style);
            rendered_lines.push(Line::from(spans));
        } else {
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
    }

    let paragraph = Paragraph::new(rendered_lines);
    f.render_widget(paragraph, inner);
}
