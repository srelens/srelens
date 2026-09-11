use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

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
    pub horizontal_scroll: usize,
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
            horizontal_scroll: 0,
            follow: true,
            timestamps: false,
            previous: false,
            // Off by default. A pod that logs one long JSON entry gets a
            // paragraph-shaped block that pushes the short lines around it
            // out of view -- and the wrapped renderer scrolls by entry, so
            // an entry taller than the viewport has middle rows nothing can
            // reach. Machine text runs on one row and scrolls sideways;
            // `w` turns wrapping on for anyone reading prose.
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
            horizontal_scroll: 0,
            follow: true,
            timestamps: false,
            previous: false,
            // Off by default. A pod that logs one long JSON entry gets a
            // paragraph-shaped block that pushes the short lines around it
            // out of view -- and the wrapped renderer scrolls by entry, so
            // an entry taller than the viewport has middle rows nothing can
            // reach. Machine text runs on one row and scrolls sideways;
            // `w` turns wrapping on for anyone reading prose.
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

    pub fn scroll_left(&mut self, n: usize) {
        self.horizontal_scroll = self.horizontal_scroll.saturating_sub(n);
    }

    pub fn scroll_right(&mut self, n: usize) {
        self.horizontal_scroll = self.horizontal_scroll.saturating_add(n);
    }

    pub fn toggle_wrap(&mut self) {
        self.wrap = !self.wrap;
        if self.wrap {
            self.horizontal_scroll = 0;
        }
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

fn style_log_content<'a>(line: &'a str, search_query: &str, match_style: Style) -> Vec<Span<'a>> {
    let has_match = !search_query.is_empty()
        && line.to_lowercase().contains(&search_query.to_lowercase());

    if has_match {
        super::highlight_text_matches(line, search_query, Style::default().fg(Theme::FG), match_style)
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
        vec![Span::styled(line, log_style)]
    }
}

fn horizontal_slice_spans<'a>(spans: Vec<Span<'a>>, mut skip: usize) -> Vec<Span<'a>> {
    if skip == 0 {
        return spans;
    }
    let mut result = Vec::new();
    for span in spans {
        if skip == 0 {
            result.push(span);
            continue;
        }
        let w = unicode_width::UnicodeWidthStr::width(span.content.as_ref());
        if w <= skip {
            skip -= w;
            continue;
        }
        let mut cur_skip = skip;
        let mut start_byte = span.content.len();
        for (b_idx, ch) in span.content.char_indices() {
            if cur_skip == 0 {
                start_byte = b_idx;
                break;
            }
            let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1);
            cur_skip = cur_skip.saturating_sub(cw);
        }
        let remaining_text = span.content[start_byte..].to_string();
        if !remaining_text.is_empty() {
            result.push(Span::styled(remaining_text, span.style));
        }
        skip = 0;
    }
    result
}

fn wrap_spans_to_visual_lines<'a>(
    spans: Vec<Span<'a>>,
    first_max_w: usize,
    cont_max_w: usize,
) -> Vec<Vec<Span<'static>>> {
    if spans.is_empty() {
        return vec![Vec::new()];
    }

    let first_max_w = first_max_w.max(5);
    let cont_max_w = cont_max_w.max(5);

    let mut result: Vec<Vec<Span<'static>>> = Vec::new();
    let mut current_line: Vec<Span<'static>> = Vec::new();
    let mut current_line_width: usize = 0;

    let target_width = |line_idx: usize| -> usize {
        if line_idx == 0 {
            first_max_w
        } else {
            cont_max_w
        }
    };

    for span in spans {
        let style = span.style;
        let content = span.content;

        let mut chunks: Vec<String> = Vec::new();
        let mut cur_chunk = String::new();
        let mut is_space = false;

        for ch in content.chars() {
            let ch_is_space = ch == ' ';
            if cur_chunk.is_empty() {
                cur_chunk.push(ch);
                is_space = ch_is_space;
            } else if ch_is_space == is_space {
                cur_chunk.push(ch);
            } else {
                chunks.push(cur_chunk);
                cur_chunk = String::new();
                cur_chunk.push(ch);
                is_space = ch_is_space;
            }
        }
        if !cur_chunk.is_empty() {
            chunks.push(cur_chunk);
        }

        for chunk in chunks {
            let chunk_width = unicode_width::UnicodeWidthStr::width(chunk.as_str());
            let is_whitespace = chunk.chars().all(|c| c == ' ');

            let max_w = target_width(result.len());

            if current_line_width + chunk_width <= max_w {
                current_line.push(Span::styled(chunk, style));
                current_line_width += chunk_width;
            } else if is_whitespace {
                if !current_line.is_empty() {
                    result.push(std::mem::take(&mut current_line));
                    current_line_width = 0;
                }
            } else {
                if current_line_width > 0 {
                    result.push(std::mem::take(&mut current_line));
                    current_line_width = 0;
                }

                let max_w = target_width(result.len());
                if chunk_width > max_w {
                    let mut sub = String::new();
                    let mut sub_w = 0;
                    for ch in chunk.chars() {
                        let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1);
                        let cur_max = target_width(result.len());
                        if sub_w + cw > cur_max && sub_w > 0 {
                            result.push(vec![Span::styled(sub, style)]);
                            sub = String::new();
                            sub_w = 0;
                        }
                        sub.push(ch);
                        sub_w += cw;
                    }
                    if !sub.is_empty() {
                        current_line.push(Span::styled(sub, style));
                        current_line_width = sub_w;
                    }
                } else {
                    current_line.push(Span::styled(chunk, style));
                    current_line_width = chunk_width;
                }
            }
        }
    }

    if !current_line.is_empty() || result.is_empty() {
        result.push(current_line);
    }

    result
}

fn format_entry_wrapped(
    i: usize,
    state: &LogsViewState,
    cont_max_w: usize,
    match_style: Style,
) -> Vec<Line<'static>> {
    let line_num = Span::styled(
        format!("{:5} │ ", i + 1),
        Style::default().fg(Theme::DIM),
    );
    let cont_num = Span::styled(
        "      │ ".to_string(),
        Style::default().fg(Theme::DIM),
    );

    let (source_span, source_w, content_spans) = if state.is_multi_pod {
        if let Some(entry) = state.entries.get(i) {
            let (src_span, src_w) = if let Some(src) = &entry.source {
                let color = source_color(src);
                let clean_src = sanitize_log_line(src);
                let text = format!("[{}] ", clean_src);
                let w = unicode_width::UnicodeWidthStr::width(text.as_str());
                (Some(Span::styled(text, Style::default().fg(color).add_modifier(Modifier::BOLD))), w)
            } else {
                (None, 0)
            };
            (src_span, src_w, style_log_content(&entry.line, &state.search_query, match_style))
        } else {
            (None, 0, Vec::new())
        }
    } else if let Some(line) = state.lines.get(i) {
        (None, 0, style_log_content(line, &state.search_query, match_style))
    } else {
        (None, 0, Vec::new())
    };

    let first_max_w = cont_max_w.saturating_sub(source_w).max(5);
    let chunks = wrap_spans_to_visual_lines(content_spans, first_max_w, cont_max_w);

    let mut visual_rows = Vec::with_capacity(chunks.len());
    for (c_idx, chunk_spans) in chunks.into_iter().enumerate() {
        let mut row_spans = Vec::new();
        if c_idx == 0 {
            row_spans.push(line_num.clone());
            if let Some(ref s) = source_span {
                row_spans.push(s.clone());
            }
        } else {
            row_spans.push(cont_num.clone());
        }
        row_spans.extend(chunk_spans);
        visual_rows.push(Line::from(row_spans));
    }
    if visual_rows.is_empty() {
        let mut row_spans = vec![line_num];
        if let Some(ref s) = source_span {
            row_spans.push(s.clone());
        }
        visual_rows.push(Line::from(row_spans));
    }
    visual_rows
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

    let h_scroll_badge = if !state.wrap && state.horizontal_scroll > 0 {
        format!(" [H+{}col]", state.horizontal_scroll)
    } else {
        String::new()
    };

    let title = if state.is_multi_pod {
        let pod_count = state.known_sources.len();
        format!(
            " Logs: {} ({} pod{} in {}) {} [{}/{} lines]{}{} (<f> Follow <t> Time <p> Prev <w> Wrap <s> Save <Esc> Back) ",
            state.pod_name,
            pod_count,
            if pod_count == 1 { "" } else { "s" },
            state.namespace,
            flags_str,
            state.scroll_offset + 1,
            total_lines,
            search_badge,
            h_scroll_badge,
        )
    } else {
        format!(
            " Logs: {} ({}/{}) {} [{}/{} lines]{}{} (<f> Follow <t> Time <p> Prev <w> Wrap <s> Save <Esc> Back) ",
            state.pod_name,
            state.namespace,
            container_str,
            flags_str,
            state.scroll_offset + 1,
            total_lines,
            search_badge,
            h_scroll_badge,
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
    if visible_lines == 0 {
        return;
    }

    let match_style = Style::default()
        .bg(Theme::YELLOW)
        .fg(Color::Rgb(20, 20, 20))
        .add_modifier(Modifier::BOLD);

    let rendered_lines = if !state.wrap {
        // Unwrapped mode: 1 visual line per entry, supports horizontal scroll with pinned line numbers.
        let start_idx = if state.follow {
            total_lines.saturating_sub(visible_lines)
        } else {
            state.scroll_offset.min(total_lines.saturating_sub(1))
        };
        let end_idx = (start_idx + visible_lines).min(total_lines);

        let mut lines = Vec::with_capacity(end_idx.saturating_sub(start_idx));
        for i in start_idx..end_idx {
            let line_num = Span::styled(
                format!("{:5} │ ", i + 1),
                Style::default().fg(Theme::DIM),
            );

            let mut content_spans = Vec::new();
            if state.is_multi_pod {
                if let Some(entry) = state.entries.get(i) {
                    if let Some(src) = &entry.source {
                        let color = source_color(src);
                        let clean_src = sanitize_log_line(src);
                        content_spans.push(Span::styled(
                            format!("[{}] ", clean_src),
                            Style::default().fg(color).add_modifier(Modifier::BOLD),
                        ));
                    }
                    content_spans.extend(style_log_content(&entry.line, &state.search_query, match_style));
                }
            } else if let Some(line) = state.lines.get(i) {
                content_spans.extend(style_log_content(line, &state.search_query, match_style));
            }

            let scrolled_content = horizontal_slice_spans(content_spans, state.horizontal_scroll);
            let mut spans = Vec::with_capacity(scrolled_content.len() + 1);
            spans.push(line_num);
            spans.extend(scrolled_content);
            lines.push(Line::from(spans));
        }
        lines
    } else {
        // Wrapped mode: entries wrap into visual lines with continuation indent "      │ "
        let avail_w = (inner.width as usize).saturating_sub(8); // line number gutter is 8 columns
        let cont_max_w = avail_w.max(5);

        if state.follow {
            let mut rev_lines: Vec<Line<'static>> = Vec::new();
            for i in (0..total_lines).rev() {
                if rev_lines.len() >= visible_lines {
                    break;
                }
                let visual_rows = format_entry_wrapped(i, state, cont_max_w, match_style);
                let needed = visible_lines - rev_lines.len();
                if visual_rows.len() <= needed {
                    for row in visual_rows.into_iter().rev() {
                        rev_lines.push(row);
                    }
                } else {
                    let skip_top = visual_rows.len() - needed;
                    for row in visual_rows.into_iter().skip(skip_top).rev() {
                        rev_lines.push(row);
                    }
                }
            }
            rev_lines.reverse();
            rev_lines
        } else {
            let start_entry = state.scroll_offset.min(total_lines.saturating_sub(1));
            let mut lines = Vec::new();
            for i in start_entry..total_lines {
                if lines.len() >= visible_lines {
                    break;
                }
                let visual_rows = format_entry_wrapped(i, state, cont_max_w, match_style);
                for row in visual_rows {
                    if lines.len() >= visible_lines {
                        break;
                    }
                    lines.push(row);
                }
            }
            lines
        }
    };

    let paragraph = Paragraph::new(rendered_lines);
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

