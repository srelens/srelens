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
        }
    }

    pub fn push_line(&mut self, line: String) {
        self.lines.push(line);
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

    let title = format!(
        " Logs: {} ({}/{}) {} [{}/{} lines] (<f> Follow <t> Time <p> Prev <w> Wrap <s> Save <Esc> Back) ",
        state.pod_name,
        state.namespace,
        container_str,
        flags_str,
        state.scroll_offset + 1,
        state.lines.len()
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

    let mut rendered_lines = Vec::new();

    for (i, line) in state.lines.iter().enumerate().take(end_idx).skip(start_idx) {
        let line_num = Span::styled(
            format!("{:5} │ ", i + 1),
            Style::default().fg(Theme::DIM),
        );

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

        rendered_lines.push(Line::from(vec![line_num, Span::styled(line.clone(), log_style)]));
    }

    let mut paragraph = Paragraph::new(rendered_lines);
    if state.wrap {
        paragraph = paragraph.wrap(Wrap { trim: false });
    }
    f.render_widget(paragraph, inner);
}
