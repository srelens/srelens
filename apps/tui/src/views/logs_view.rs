use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::theme::Theme;

pub struct LogsViewState {
    pub pod_name: String,
    pub namespace: String,
    pub container: Option<String>,
    pub channel: String,
    pub lines: Vec<String>,
    pub scroll_offset: usize,
    pub follow: bool,
    pub timestamps: bool,
    pub previous: bool,
    pub wrap: bool,
    pub search_query: String,
    pub search_matches: Vec<usize>,
    pub current_match_idx: Option<usize>,
}

impl LogsViewState {
    pub fn new(pod_name: String, namespace: String, container: Option<String>, channel: String) -> Self {
        Self {
            pod_name,
            namespace,
            container,
            channel,
            lines: Vec::new(),
            scroll_offset: 0,
            follow: true,
            timestamps: false,
            previous: false,
            wrap: false,
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
            self.follow = false;
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
        self.follow = false;
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
        self.follow = false;
    }

    pub fn clear_search(&mut self) {
        self.search_query.clear();
        self.search_matches.clear();
        self.current_match_idx = None;
    }

    pub fn push_line(&mut self, line: String) {
        self.lines.push(sanitize_log_line(&line));
        if self.follow {
            self.scroll_to_bottom();
        }
    }

    pub fn scroll_down(&mut self, n: usize) {
        if self.scroll_offset + n < self.lines.len() {
            self.scroll_offset += n;
            self.follow = false;
        }
    }

    pub fn scroll_up(&mut self, n: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(n);
        self.follow = false;
    }

    pub fn scroll_top(&mut self) {
        self.scroll_offset = 0;
        self.follow = false;
    }

    pub fn scroll_to_bottom(&mut self) {
        if !self.lines.is_empty() {
            self.scroll_offset = self.lines.len().saturating_sub(1);
        }
    }

    pub fn toggle_follow(&mut self) {
        self.follow = !self.follow;
        if self.follow {
            self.scroll_to_bottom();
        }
    }

    pub fn toggle_timestamps(&mut self) {
        self.timestamps = !self.timestamps;
    }

    pub fn toggle_previous(&mut self) {
        self.previous = !self.previous;
    }

    pub fn toggle_wrap(&mut self) {
        self.wrap = !self.wrap;
    }

    pub fn save_to_file(&self) -> Result<String, String> {
        let filename = format!("{}-{}-logs.txt", self.pod_name, chrono_timestamp());
        let path = std::env::temp_dir().join(&filename);
        std::fs::write(&path, self.lines.join("\n")).map_err(|e| e.to_string())?;
        Ok(path.to_string_lossy().into_owned())
    }
}

/// Shared sanitizer for cluster-controlled Span text; see
/// [`crate::views::sanitize_span_text`].
pub use super::sanitize_span_text as sanitize_log_line;

fn chrono_timestamp() -> String {
    use std::time::SystemTime;
    let dur = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    dur.as_secs().to_string()
}

pub fn render_logs_view(f: &mut Frame, area: Rect, state: &LogsViewState) {
    let container_str = state.container.as_deref().unwrap_or("all");
    let flags_str = format!(
        "[{}{}{}{}]",
        if state.follow { "F" } else { "f" },
        if state.timestamps { "T" } else { "t" },
        if state.previous { "P" } else { "p" },
        if state.wrap { "W" } else { "w" },
    );

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
        " Logs: {} ({}/{}) {} [{}/{} lines]{} (<f> Follow <t> Time <p> Prev <w> Wrap <s> Save <Esc> Back) ",
        state.pod_name,
        state.namespace,
        container_str,
        flags_str,
        state.scroll_offset + 1,
        state.lines.len(),
        search_badge,
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::BORDER_FOCUS))
        .title(Span::styled(title, Theme::title()));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if state.lines.is_empty() {
        let msg = Paragraph::new(Line::from(vec![
            Span::styled("Waiting for logs...", Style::default().fg(Theme::DIM)),
        ]));
        f.render_widget(msg, inner);
        return;
    }

    let visible_lines = inner.height as usize;
    let start_idx = if state.follow {
        state.lines.len().saturating_sub(visible_lines)
    } else {
        state.scroll_offset
    };
    let end_idx = (start_idx + visible_lines).min(state.lines.len());

    let match_style = Style::default()
        .bg(Theme::YELLOW)
        .fg(Color::Rgb(20, 20, 20))
        .add_modifier(Modifier::BOLD);

    let mut rendered_lines = Vec::new();

    for (i, line) in state.lines.iter().enumerate().take(end_idx).skip(start_idx) {
        let line_num = Span::styled(
            format!("{:5} │ ", i + 1),
            Style::default().fg(Theme::DIM),
        );

        let mut spans = vec![line_num];

        let has_match = !state.search_query.is_empty()
            && line.to_lowercase().contains(&state.search_query.to_lowercase());

        if has_match {
            let highlighted = super::highlight_text_matches(line, &state.search_query, Style::default().fg(Theme::FG), match_style);
            spans.extend(highlighted);
        } else {
            let lower = line.to_lowercase();
            let log_style = if lower.contains("error") || lower.contains("fatal") || lower.contains("exception") || lower.contains("panic") {
                Style::default().fg(Theme::RED)
            } else if lower.contains("warn") || lower.contains("warning") {
                Style::default().fg(Theme::YELLOW)
            } else if lower.contains("info") {
                Style::default().fg(Theme::FG)
            } else if lower.contains("debug") || lower.contains("trace") {
                Style::default().fg(Theme::DIM)
            } else {
                Style::default().fg(Theme::FG)
            };
            spans.push(Span::styled(line.clone(), log_style));
        }

        rendered_lines.push(Line::from(spans));
    }

    let mut paragraph = Paragraph::new(rendered_lines);
    if state.wrap {
        paragraph = paragraph.wrap(Wrap { trim: false });
    }
    f.render_widget(paragraph, inner);
}
