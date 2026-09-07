use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::theme::Theme;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub source: Option<String>,
    pub line: String,
}

pub struct LogsViewState {
    pub pod_name: String,
    pub namespace: String,
    pub container: Option<String>,
    pub channel: String,
    pub lines: Vec<String>,
    pub entries: Vec<LogEntry>,
    pub is_multi_pod: bool,
    pub known_sources: Vec<String>,
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
            entries: Vec::new(),
            is_multi_pod: false,
            known_sources: Vec::new(),
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

    pub fn new_multi_pod(target_name: String, namespace: String, pod_names: Vec<String>, channel: String) -> Self {
        Self {
            pod_name: target_name,
            namespace,
            container: None,
            channel,
            lines: Vec::new(),
            entries: Vec::new(),
            is_multi_pod: true,
            known_sources: pod_names,
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
        self.push_entry(None, line);
    }

    pub fn push_entry(&mut self, source: Option<String>, line: String) {
        let clean = sanitize_log_line(&line);
        if let Some(src) = &source {
            if !src.is_empty() && !self.known_sources.contains(src) {
                self.known_sources.push(src.clone());
            }
        }
        self.lines.push(clean.clone());
        self.entries.push(LogEntry { source, line: clean });
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
        let content = if self.is_multi_pod {
            self.entries
                .iter()
                .map(|e| {
                    if let Some(src) = &e.source {
                        format!("[{}] {}", sanitize_log_line(src), e.line)
                    } else {
                        e.line.clone()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            self.lines.join("\n")
        };
        std::fs::write(&path, content).map_err(|e| e.to_string())?;
        Ok(path.to_string_lossy().into_owned())
    }
}

const SOURCE_COLORS: &[Color] = &[
    Color::Rgb(6, 182, 212),    // Cyan
    Color::Rgb(217, 70, 239),   // Magenta
    Color::Rgb(34, 197, 94),    // Green
    Color::Rgb(234, 179, 8),    // Yellow
    Color::Rgb(59, 130, 246),   // Blue
    Color::Rgb(249, 115, 22),   // Orange
    Color::Rgb(168, 85, 247),   // Purple
    Color::Rgb(20, 184, 166),   // Teal
    Color::Rgb(244, 63, 94),    // Rose
];

pub fn source_color(source: &str) -> Color {
    let mut hash: usize = 0;
    for b in source.bytes() {
        hash = hash.wrapping_mul(31).wrapping_add(b as usize);
    }
    SOURCE_COLORS[hash % SOURCE_COLORS.len()]
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

    let total_lines = if state.is_multi_pod {
        state.entries.len()
    } else {
        state.lines.len()
    };

    let title = if state.is_multi_pod {
        let pod_count = state.known_sources.len();
        format!(
            " Logs: {} ({} pod{} in {}) {} [{}/{} lines]{} (<f> Follow <t> Time <p> Prev <w> Wrap <s> Save <Esc> Back) ",
            state.pod_name,
            pod_count,
            if pod_count == 1 { "" } else { "s" },
            state.namespace,
            flags_str,
            state.scroll_offset + 1,
            total_lines,
            search_badge,
        )
    } else {
        format!(
            " Logs: {} ({}/{}) {} [{}/{} lines]{} (<f> Follow <t> Time <p> Prev <w> Wrap <s> Save <Esc> Back) ",
            state.pod_name,
            state.namespace,
            container_str,
            flags_str,
            state.scroll_offset + 1,
            total_lines,
            search_badge,
        )
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::BORDER_FOCUS))
        .title(Span::styled(title, Theme::title()));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if total_lines == 0 {
        let msg = Paragraph::new(Line::from(vec![
            Span::styled("Waiting for logs...", Style::default().fg(Theme::DIM)),
        ]));
        f.render_widget(msg, inner);
        return;
    }

    let visible_lines = inner.height as usize;
    let start_idx = if state.follow {
        total_lines.saturating_sub(visible_lines)
    } else {
        state.scroll_offset
    };
    let end_idx = (start_idx + visible_lines).min(total_lines);

    let match_style = Style::default()
        .bg(Theme::YELLOW)
        .fg(Color::Rgb(20, 20, 20))
        .add_modifier(Modifier::BOLD);

    let mut rendered_lines = Vec::new();

    if state.is_multi_pod {
        for (i, entry) in state.entries.iter().enumerate().take(end_idx).skip(start_idx) {
            let line_num = Span::styled(
                format!("{:5} │ ", i + 1),
                Style::default().fg(Theme::DIM),
            );

            let mut spans = vec![line_num];

            if let Some(src) = &entry.source {
                let color = source_color(src);
                let clean_src = sanitize_log_line(src);
                spans.push(Span::styled(
                    format!("[{}] ", clean_src),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ));
            }

            let line = &entry.line;
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
    } else {
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
    }

    let mut paragraph = Paragraph::new(rendered_lines);
    if state.wrap {
        paragraph = paragraph.wrap(Wrap { trim: false });
    }
    f.render_widget(paragraph, inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn test_multi_pod_logs_state() {
        let mut state = LogsViewState::new_multi_pod(
            "frontend".to_string(),
            "default".to_string(),
            vec!["frontend-1".to_string(), "frontend-2".to_string()],
            "chan-123".to_string(),
        );

        assert!(state.is_multi_pod);
        assert_eq!(state.pod_name, "frontend");
        assert_eq!(state.known_sources.len(), 2);

        state.push_entry(Some("frontend-1".to_string()), "server started".to_string());
        state.push_entry(Some("frontend-2".to_string()), "connected to db".to_string());
        state.push_entry(Some("frontend-3".to_string()), "cache ready".to_string());

        assert_eq!(state.entries.len(), 3);
        assert_eq!(state.lines.len(), 3);
        assert_eq!(state.known_sources.len(), 3);

        let color1 = source_color("frontend-1");
        let color2 = source_color("frontend-2");
        let color1_again = source_color("frontend-1");
        assert_eq!(color1, color1_again);
        // Different pods likely have distinct colors
        assert_ne!(color1, color2);
    }

    #[test]
    fn test_multi_pod_logs_render() {
        let mut state = LogsViewState::new_multi_pod(
            "api-service".to_string(),
            "prod".to_string(),
            vec!["api-1".to_string(), "api-2".to_string()],
            "chan-456".to_string(),
        );
        state.push_entry(Some("api-1".to_string()), "GET /health 200".to_string());
        state.push_entry(Some("api-2".to_string()), "POST /login 200".to_string());

        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render_logs_view(f, area, &state);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let rendered: String = (0..buffer.area.height)
            .flat_map(|y| {
                let mut line = String::new();
                for x in 0..buffer.area.width {
                    line.push_str(buffer[(x, y)].symbol());
                }
                line.push('\n');
                line.into_bytes()
            })
            .map(|b| b as char)
            .collect();

        assert!(rendered.contains("api-service"));
        assert!(rendered.contains("2 pods in prod"));
        assert!(rendered.contains("[api-1]"));
        assert!(rendered.contains("[api-2]"));
        assert!(rendered.contains("GET /health 200"));
        assert!(rendered.contains("POST /login 200"));
    }
}

