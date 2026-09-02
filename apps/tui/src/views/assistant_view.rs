use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::theme::Theme;

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: String, // "user", "assistant", "system"
    pub content: String,
    pub timestamp: String,
}

pub fn current_timestamp() -> String {
    chrono::Local::now().format("%H:%M:%S").to_string()
}

pub struct AssistantViewState {
    pub messages: Vec<ChatMessage>,
    pub input: String,
    pub is_busy: bool,
    pub scroll_offset: usize,
}

impl AssistantViewState {
    pub fn new() -> Self {
        Self {
            messages: vec![ChatMessage {
                role: "assistant".to_string(),
                content: "Hello! I am your SRElens AI Assistant. I can analyze pod crashes, diagnose cluster events, inspect configurations, and suggest Kubernetes remediation actions. Type your prompt below:".to_string(),
                timestamp: current_timestamp(),
            }],
            input: String::new(),
            is_busy: false,
            scroll_offset: 0,
        }
    }

    pub fn add_user_message(&mut self, text: String) {
        self.messages.push(ChatMessage {
            role: "user".to_string(),
            content: text,
            timestamp: current_timestamp(),
        });
        self.input.clear();
        self.is_busy = true;
    }

    pub fn add_assistant_message(&mut self, text: String) {
        self.messages.push(ChatMessage {
            role: "assistant".to_string(),
            content: text,
            timestamp: current_timestamp(),
        });
        self.is_busy = false;
    }

    pub fn scroll_down(&mut self, n: usize) {
        self.scroll_offset = self.scroll_offset.saturating_add(n);
    }

    pub fn scroll_up(&mut self, n: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(n);
    }
}

pub fn render_assistant_view(
    f: &mut Frame,
    area: Rect,
    state: &AssistantViewState,
    settings: &crate::ai_config::AiSettings,
) {
    let prov = settings.default_provider;
    let prov_name = crate::ai_config::provider_display_name(prov);
    let model = settings.get_model(prov);
    let title = format!(
        " SRElens AI Assistant [{} - {}] [<s> Settings, <Enter> Send, <Esc> Back] ",
        prov_name, model
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::ACCENT))
        .title(Span::styled(title, Theme::title()));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(5),    // Messages history
            Constraint::Length(3), // Input prompt
        ])
        .split(inner);

    // 1. Message history
    let mut rendered_lines = Vec::new();
    for msg in &state.messages {
        let (role_label, role_style) = match msg.role.as_str() {
            "user" => ("You", Style::default().fg(Theme::CYAN).add_modifier(Modifier::BOLD)),
            "assistant" => ("SRElens", Style::default().fg(Theme::ACCENT).add_modifier(Modifier::BOLD)),
            _ => ("System", Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)),
        };

        let mut header_spans = vec![
            Span::styled(role_label, role_style),
        ];

        if !msg.timestamp.is_empty() {
            header_spans.push(Span::raw(" "));
            header_spans.push(Span::styled(
                format!("[{}]", msg.timestamp),
                Style::default().fg(Theme::DIM),
            ));
        }
        header_spans.push(Span::styled(":", role_style));

        rendered_lines.push(Line::from(header_spans));

        // Format message body (with markdown table detection)
        format_message_content(&mut rendered_lines, &msg.content);
        rendered_lines.push(Line::from(""));
    }

    if state.is_busy {
        rendered_lines.push(Line::from(vec![
            Span::styled(
                "⚡ SRElens agent is consulting cluster state and AI provider...",
                Style::default().fg(Theme::YELLOW).add_modifier(Modifier::ITALIC),
            ),
        ]));
    }

    let history_widget = Paragraph::new(rendered_lines)
        .wrap(Wrap { trim: false })
        .scroll((state.scroll_offset as u16, 0));
    f.render_widget(history_widget, chunks[0]);

    // 2. Input box
    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::CYAN))
        .title(" Ask Assistant (<Ctrl+w> Rubout, <s> Settings) ");
    let input_widget = Paragraph::new(format!("{}█", state.input))
        .block(input_block);
    f.render_widget(input_widget, chunks[1]);
}

/// Formats message text lines, detecting and rendering Markdown tables as aligned box-drawing tables.
pub fn format_message_content(out: &mut Vec<Line<'static>>, content: &str) {
    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];

        // Check if this line begins a Markdown table
        if is_table_row(line) && i + 1 < lines.len() && is_table_separator(lines[i + 1]) {
            let header_line = lines[i];
            i += 2; // skip header and separator line
            let mut row_lines = Vec::new();
            while i < lines.len() && is_table_row(lines[i]) && !is_table_separator(lines[i]) {
                row_lines.push(lines[i]);
                i += 1;
            }
            render_markdown_table(out, header_line, &row_lines);
        } else {
            render_text_line(out, line);
            i += 1;
        }
    }
}

fn is_table_row(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with('|') && trimmed.ends_with('|') && trimmed.matches('|').count() >= 2
}

fn is_table_separator(line: &str) -> bool {
    let trimmed = line.trim();
    if !trimmed.starts_with('|') || !trimmed.ends_with('|') {
        return false;
    }
    trimmed.chars().all(|c| c == '|' || c == '-' || c == ':' || c.is_whitespace())
}

fn parse_cells(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    let inner = if trimmed.starts_with('|') && trimmed.ends_with('|') && trimmed.len() >= 2 {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    };
    inner.split('|').map(|c| c.trim().to_string()).collect()
}

fn clean_cell_text(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.starts_with('`') && trimmed.ends_with('`') && trimmed.len() >= 2 {
        trimmed[1..trimmed.len() - 1].trim().to_string()
    } else {
        trimmed.to_string()
    }
}

pub fn render_markdown_table(out: &mut Vec<Line<'static>>, header_line: &str, row_lines: &[&str]) {
    let raw_headers = parse_cells(header_line);
    let headers: Vec<String> = raw_headers.into_iter().map(|h| clean_cell_text(&h)).collect();
    if headers.is_empty() {
        return;
    }

    let mut rows: Vec<Vec<String>> = Vec::new();
    for r in row_lines {
        let cells: Vec<String> = parse_cells(r).into_iter().map(|c| clean_cell_text(&c)).collect();
        if !cells.is_empty() {
            rows.push(cells);
        }
    }

    let _num_cols = headers.len();
    let mut col_widths: Vec<usize> = headers
        .iter()
        .map(|h| unicode_width::UnicodeWidthStr::width(h.as_str()).max(3))
        .collect();

    for r in &rows {
        for (c_idx, cell) in r.iter().enumerate() {
            if c_idx < col_widths.len() {
                col_widths[c_idx] = col_widths[c_idx].max(unicode_width::UnicodeWidthStr::width(cell.as_str()));
            }
        }
    }

    let border_style = Style::default().fg(Theme::BORDER);
    let header_style = Style::default().fg(Theme::CYAN).add_modifier(Modifier::BOLD);

    // 1. Top border: ┌────────┬────────┐
    let mut top_spans = vec![Span::raw("  "), Span::styled("┌", border_style)];
    for (i, w) in col_widths.iter().enumerate() {
        top_spans.push(Span::styled("─".repeat(*w + 2), border_style));
        if i + 1 < col_widths.len() {
            top_spans.push(Span::styled("┬", border_style));
        } else {
            top_spans.push(Span::styled("┐", border_style));
        }
    }
    out.push(Line::from(top_spans));

    // 2. Header row: │ Header 1 │ Header 2 │
    let mut hdr_spans = vec![Span::raw("  "), Span::styled("│", border_style)];
    for (i, w) in col_widths.iter().enumerate() {
        let title = headers.get(i).map(String::as_str).unwrap_or("");
        let title_len = unicode_width::UnicodeWidthStr::width(title);
        let pad = w.saturating_sub(title_len);
        hdr_spans.push(Span::raw(" "));
        hdr_spans.push(Span::styled(title.to_string(), header_style));
        hdr_spans.push(Span::raw(" ".repeat(pad + 1)));
        hdr_spans.push(Span::styled("│", border_style));
    }
    out.push(Line::from(hdr_spans));

    // 3. Header-Data divider: ├────────┼────────┤
    let mut mid_spans = vec![Span::raw("  "), Span::styled("├", border_style)];
    for (i, w) in col_widths.iter().enumerate() {
        mid_spans.push(Span::styled("─".repeat(*w + 2), border_style));
        if i + 1 < col_widths.len() {
            mid_spans.push(Span::styled("┼", border_style));
        } else {
            mid_spans.push(Span::styled("┤", border_style));
        }
    }
    out.push(Line::from(mid_spans));

    // 4. Data rows
    for row in rows {
        let mut row_spans = vec![Span::raw("  "), Span::styled("│", border_style)];
        for (i, w) in col_widths.iter().enumerate() {
            let val = row.get(i).map(String::as_str).unwrap_or("");
            let val_len = unicode_width::UnicodeWidthStr::width(val);
            let pad = w.saturating_sub(val_len);
            row_spans.push(Span::raw(" "));

            let cell_style = if val.chars().all(|c| c.is_numeric() || c == '.') {
                Style::default().fg(Theme::YELLOW)
            } else if val.ends_with("GiB") || val.ends_with("MiB") || val.ends_with("GB") || val.ends_with("MB") {
                Style::default().fg(Theme::GREEN)
            } else if val.contains('/') || val.starts_with("data-") || val.starts_with("gpu-") {
                Style::default().fg(Theme::CYAN)
            } else {
                Style::default().fg(Theme::FG)
            };

            row_spans.push(Span::styled(val.to_string(), cell_style));
            row_spans.push(Span::raw(" ".repeat(pad + 1)));
            row_spans.push(Span::styled("│", border_style));
        }
        out.push(Line::from(row_spans));
    }

    // 5. Bottom border: └────────┴────────┘
    let mut bot_spans = vec![Span::raw("  "), Span::styled("└", border_style)];
    for (i, w) in col_widths.iter().enumerate() {
        bot_spans.push(Span::styled("─".repeat(*w + 2), border_style));
        if i + 1 < col_widths.len() {
            bot_spans.push(Span::styled("┴", border_style));
        } else {
            bot_spans.push(Span::styled("┘", border_style));
        }
    }
    out.push(Line::from(bot_spans));
}

fn render_text_line(out: &mut Vec<Line<'static>>, line: &str) {
    if line.trim().is_empty() {
        out.push(Line::from(""));
        return;
    }
    let mut spans = vec![Span::raw("  ")];
    spans.extend(parse_inline_markdown(line));
    out.push(Line::from(spans));
}

fn parse_inline_markdown(line: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut rest = line;

    while !rest.is_empty() {
        if let Some(code_start) = rest.find('`') {
            if let Some(code_end) = rest[code_start + 1..].find('`') {
                let end_idx = code_start + 1 + code_end;
                let before = &rest[..code_start];
                if !before.is_empty() {
                    spans.extend(parse_bold(before));
                }
                let code_content = &rest[code_start + 1..end_idx];
                spans.push(Span::styled(
                    code_content.to_string(),
                    Style::default().fg(Theme::CYAN).add_modifier(Modifier::BOLD),
                ));
                rest = &rest[end_idx + 1..];
                continue;
            }
        }
        spans.extend(parse_bold(rest));
        break;
    }
    spans
}

fn parse_bold(text: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut rest = text;
    while let Some(b_start) = rest.find("**") {
        if let Some(b_end) = rest[b_start + 2..].find("**") {
            let end_idx = b_start + 2 + b_end;
            let before = &rest[..b_start];
            if !before.is_empty() {
                spans.push(Span::styled(before.to_string(), Style::default().fg(Theme::FG)));
            }
            let bold_content = &rest[b_start + 2..end_idx];
            spans.push(Span::styled(
                bold_content.to_string(),
                Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD),
            ));
            rest = &rest[end_idx + 2..];
            continue;
        }
        break;
    }
    if !rest.is_empty() {
        spans.push(Span::styled(rest.to_string(), Style::default().fg(Theme::FG)));
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_message_content_with_table() {
        let content = "\
Here are the nodes with GPUs:

| Node | Physical GPUs | Allocatable | Memory each |
|---|---|---|---|
| `gpu-node-1` | 1 | 10 | 15 GiB |
| `gpu-node-2` | 2 | 20 | 15 GiB |

All nodes nominal.";

        let mut lines = Vec::new();
        format_message_content(&mut lines, content);

        // Verify that box-drawing table rows were generated
        let text_dump: String = lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<Vec<_>>()
                    .join("")
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(text_dump.contains("┌"));
        assert!(text_dump.contains("┬"));
        assert!(text_dump.contains("┐"));
        assert!(text_dump.contains("│ Node"));
        assert!(text_dump.contains("│ gpu-node-1"));
        assert!(text_dump.contains("15 GiB"));
        assert!(text_dump.contains("└"));
    }

    #[test]
    fn test_timestamp_added_to_messages() {
        let mut state = AssistantViewState::new();
        assert!(!state.messages[0].timestamp.is_empty());

        state.add_user_message("test user question".to_string());
        assert_eq!(state.messages.len(), 2);
        assert!(!state.messages[1].timestamp.is_empty());

        state.add_assistant_message("test assistant reply".to_string());
        assert_eq!(state.messages.len(), 3);
        assert!(!state.messages[2].timestamp.is_empty());
    }
}
