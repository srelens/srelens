use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use std::io::Write;
use std::process::Command;

use crate::theme::Theme;

pub struct YamlViewState {
    pub resource_name: String,
    pub resource_kind: String,
    pub namespace: Option<String>,
    pub yaml_content: String,
    pub lines: Vec<String>,
    pub scroll_offset: usize,
    pub search_query: String,
    pub is_diff: bool,
    pub selection: Option<(usize, usize)>,
    pub is_selecting: bool,
    pub last_viewport_rect: std::cell::Cell<Rect>,
}

impl YamlViewState {
    pub fn new(name: String, kind: String, namespace: Option<String>, yaml_content: String) -> Self {
        // Serialized manifests can embed cluster-authored strings with
        // tabs/escapes that desync the terminal when rendered raw.
        let lines = yaml_content.lines().map(super::sanitize_span_text).collect();
        Self {
            resource_name: name,
            resource_kind: kind,
            namespace,
            yaml_content,
            lines,
            scroll_offset: 0,
            search_query: String::new(),
            is_diff: false,
            selection: None,
            is_selecting: false,
            last_viewport_rect: std::cell::Cell::new(Rect::default()),
        }
    }

    pub fn start_selection(&mut self, line_idx: usize) {
        if self.lines.is_empty() { return; }
        let valid_line = line_idx.min(self.lines.len() - 1);
        self.selection = Some((valid_line, valid_line));
        self.is_selecting = true;
    }

    pub fn update_selection(&mut self, line_idx: usize) {
        if self.is_selecting && !self.lines.is_empty() {
            if let Some((start, _)) = self.selection {
                let valid_line = line_idx.min(self.lines.len() - 1);
                self.selection = Some((start, valid_line));
            }
        }
    }

    pub fn finish_selection(&mut self, line_idx: usize) -> Option<String> {
        self.is_selecting = false;
        if self.lines.is_empty() { return None; }
        if let Some((start, _)) = self.selection {
            let valid_line = line_idx.min(self.lines.len() - 1);
            self.selection = Some((start, valid_line));
            self.selected_text()
        } else {
            None
        }
    }

    pub fn clear_selection(&mut self) {
        self.selection = None;
        self.is_selecting = false;
    }

    pub fn selected_text(&self) -> Option<String> {
        let (start, end) = self.selection?;
        if self.lines.is_empty() { return None; }
        let min_l = start.min(end);
        let max_l = start.max(end).min(self.lines.len() - 1);
        let selected_lines = &self.lines[min_l..=max_l];
        Some(selected_lines.join("\n"))
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

    /// Spawns the user's $EDITOR on a temp file with the YAML content
    pub fn spawn_editor(&self) -> Result<Option<String>, String> {
        let editor = std::env::var("EDITOR")
            .or_else(|_| std::env::var("VISUAL"))
            .unwrap_or_else(|_| "vi".to_string());

        let mut temp_file = tempfile::Builder::new()
            .prefix(&format!("srelens-{}-", self.resource_name))
            .suffix(".yaml")
            .tempfile()
            .map_err(|e| e.to_string())?;

        temp_file
            .write_all(self.yaml_content.as_bytes())
            .map_err(|e| e.to_string())?;
        temp_file.flush().map_err(|e| e.to_string())?;

        let status = Command::new(&editor)
            .arg(temp_file.path())
            .status()
            .map_err(|e| format!("Failed to spawn editor '{}': {}", editor, e))?;

        if !status.success() {
            return Err(format!("Editor exited with status: {}", status));
        }

        let new_content = std::fs::read_to_string(temp_file.path())
            .map_err(|e| e.to_string())?;

        if new_content == self.yaml_content {
            Ok(None) // No changes made
        } else {
            Ok(Some(new_content))
        }
    }
}

pub fn render_yaml_view(f: &mut Frame, area: Rect, state: &YamlViewState) {
    let copy_hint = if state.selection.is_some() {
        "[c: Copy Selection, Esc: Clear Selection]"
    } else {
        "[e: Edit, c: Copy, Esc: Back]"
    };

    let title = format!(
        " YAML: {}/{} {} (Line {}/{}) {} ",
        state.resource_kind,
        state.resource_name,
        state.namespace.as_deref().map(|ns| format!("({})", ns)).unwrap_or_default(),
        state.scroll_offset + 1,
        state.lines.len(),
        copy_hint,
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::BORDER_FOCUS))
        .title(Span::styled(title, Theme::title()));

    let inner = block.inner(area);
    state.last_viewport_rect.set(inner);
    f.render_widget(block, area);

    let visible_lines = inner.height as usize;
    let end_idx = (state.scroll_offset + visible_lines).min(state.lines.len());

    let (sel_min, sel_max) = state.selection
        .map(|(s, e)| (s.min(e), s.max(e)))
        .unwrap_or((usize::MAX, usize::MAX));

    let mut rendered_lines = Vec::new();

    for (i, line) in state.lines.iter().enumerate().take(end_idx).skip(state.scroll_offset) {
        let is_selected = i >= sel_min && i <= sel_max;
        let line_num = Span::styled(
            format!("{:4} │ ", i + 1),
            Style::default().fg(if is_selected { Theme::CYAN } else { Theme::DIM }),
        );

        let mut spans = vec![line_num];

        // Highlight YAML tokens
        let trimmed = line.trim_start();
        let leading_spaces = &line[..(line.len() - trimmed.len())];
        if !leading_spaces.is_empty() {
            spans.push(Span::raw(leading_spaces.to_string()));
        }

        if trimmed.starts_with('#') {
            // Comment
            spans.push(Span::styled(trimmed.to_string(), Style::default().fg(Theme::DIM)));
        } else if trimmed.starts_with("---") {
            // Document separator
            spans.push(Span::styled(trimmed.to_string(), Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)));
        } else if let Some((key, val)) = trimmed.split_once(':') {
            // Key: Value
            let (clean_key, key_style) = if key.starts_with("- ") {
                let dash = &key[..2];
                let rest_key = &key[2..];
                spans.push(Span::styled(dash.to_string(), Style::default().fg(Theme::YELLOW)));
                (rest_key, Style::default().fg(Theme::CYAN).add_modifier(Modifier::BOLD))
            } else {
                (key, Style::default().fg(Theme::CYAN).add_modifier(Modifier::BOLD))
            };

            spans.push(Span::styled(format!("{}:", clean_key), key_style));

            if !val.is_empty() {
                let trimmed_val = val.trim();
                let leading_val_space = &val[..(val.len() - val.trim_start().len())];
                spans.push(Span::raw(leading_val_space.to_string()));

                let val_style = if trimmed_val == "true" || trimmed_val == "false" || trimmed_val.parse::<i64>().is_ok() {
                    Style::default().fg(Theme::YELLOW)
                } else if trimmed_val.starts_with('"') || trimmed_val.starts_with('\'') {
                    Style::default().fg(Theme::GREEN)
                } else {
                    Style::default().fg(Theme::FG)
                };
                spans.push(Span::styled(trimmed_val.to_string(), val_style));
            }
        } else {
            spans.push(Span::styled(trimmed.to_string(), Style::default().fg(Theme::FG)));
        }

        let line_style = if is_selected {
            Style::default().bg(Color::Rgb(35, 55, 95))
        } else {
            Style::default()
        };
        rendered_lines.push(Line::from(spans).style(line_style));
    }

    let paragraph = Paragraph::new(rendered_lines);
    f.render_widget(paragraph, inner);
}
