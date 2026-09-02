use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::theme::Theme;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolCallStatus {
    Running,
    Success,
    Error(String),
}

#[derive(Debug, Clone)]
pub struct ToolCallRecord {
    pub id: String,
    pub tool: String,
    pub args_summary: String,
    pub status: ToolCallStatus,
}

#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub cached_tokens: usize,
    pub total_tokens: usize,
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: String, // "user", "assistant", "system"
    pub content: String,
    pub timestamp: String,
    pub tool_calls: Vec<ToolCallRecord>,
    pub token_usage: Option<TokenUsage>,
}

pub fn current_timestamp() -> String {
    chrono::Local::now().format("%H:%M:%S").to_string()
}

fn format_number(n: usize) -> String {
    let s = n.to_string();
    let mut result = String::new();
    let len = s.len();
    for (idx, ch) in s.chars().enumerate() {
        if idx > 0 && (len - idx) % 3 == 0 {
            result.push(',');
        }
        result.push(ch);
    }
    result
}

use std::cell::Cell;

pub struct AssistantViewState {
    pub messages: Vec<ChatMessage>,
    pub input: String,
    pub is_busy: bool,
    pub busy_status: String,
    pub busy_start: Option<std::time::Instant>,
    pub spinner_frame: usize,
    pub scroll_offset: usize,
    pub auto_scroll: bool,
    pub last_max_scroll: Cell<usize>,
    pub last_total_lines: Cell<usize>,
    pub native_history: std::sync::Arc<tokio::sync::Mutex<Vec<srelens_llm::types::Turn>>>,
}

impl AssistantViewState {
    pub fn new() -> Self {
        Self {
            messages: vec![ChatMessage {
                role: "assistant".to_string(),
                content: "Hello! I am your SRElens AI Assistant. I can analyze pod crashes, diagnose cluster events, inspect configurations, and suggest Kubernetes remediation actions. Type your prompt below:".to_string(),
                timestamp: current_timestamp(),
                tool_calls: Vec::new(),
                token_usage: None,
            }],
            input: String::new(),
            is_busy: false,
            busy_status: String::new(),
            busy_start: None,
            spinner_frame: 0,
            scroll_offset: 0,
            auto_scroll: true,
            last_max_scroll: Cell::new(0),
            last_total_lines: Cell::new(0),
            native_history: std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new())),
        }
    }

    pub fn start_turn(&mut self, text: String) {
        self.messages.push(ChatMessage {
            role: "user".to_string(),
            content: text,
            timestamp: current_timestamp(),
            tool_calls: Vec::new(),
            token_usage: None,
        });
        self.input.clear();
        self.is_busy = true;
        self.busy_status = "Consulting AI provider & cluster state...".to_string();
        self.busy_start = Some(std::time::Instant::now());
        self.spinner_frame = 0;
        self.auto_scroll = true;
        self.scroll_offset = self.last_max_scroll.get();
    }

    pub fn add_user_message(&mut self, text: String) {
        self.start_turn(text);
    }

    pub fn add_assistant_message(&mut self, text: String) {
        self.messages.push(ChatMessage {
            role: "assistant".to_string(),
            content: text,
            timestamp: current_timestamp(),
            tool_calls: Vec::new(),
            token_usage: None,
        });
        self.is_busy = false;
        self.busy_start = None;
        self.auto_scroll = true;
        self.scroll_offset = self.last_max_scroll.get();
    }

    pub fn append_stream_chunk(&mut self, chunk: &str) {
        if chunk.is_empty() {
            return;
        }
        if let Some(last) = self.messages.last_mut() {
            if last.role == "assistant" {
                if !last.content.is_empty() {
                    let last_char = last.content.chars().last().unwrap();
                    let first_char = chunk.chars().next().unwrap();
                    // If previous content ends with punctuation (. ! ? : ;) and chunk starts without whitespace
                    if (last_char == '.' || last_char == '!' || last_char == '?' || last_char == ':' || last_char == ';')
                        && !first_char.is_whitespace()
                    {
                        last.content.push(' ');
                    }
                }
                last.content.push_str(chunk);
                return;
            }
        }
        self.messages.push(ChatMessage {
            role: "assistant".to_string(),
            content: chunk.to_string(),
            timestamp: current_timestamp(),
            tool_calls: Vec::new(),
            token_usage: None,
        });
    }

    pub fn add_tool_call_start(&mut self, id: String, tool: String, args_summary: String) {
        if let Some(last) = self.messages.last_mut() {
            if last.role == "assistant" {
                last.tool_calls.push(ToolCallRecord {
                    id,
                    tool,
                    args_summary,
                    status: ToolCallStatus::Running,
                });
                return;
            }
        }
        self.messages.push(ChatMessage {
            role: "assistant".to_string(),
            content: String::new(),
            timestamp: current_timestamp(),
            tool_calls: vec![ToolCallRecord {
                id,
                tool,
                args_summary,
                status: ToolCallStatus::Running,
            }],
            token_usage: None,
        });
    }

    pub fn finish_tool_call(&mut self, id: &str, status: ToolCallStatus) {
        if let Some(last) = self.messages.last_mut() {
            if let Some(tc) = last.tool_calls.iter_mut().find(|tc| tc.id == id) {
                tc.status = status;
            }
        }
    }

    pub fn set_token_usage(&mut self, usage: TokenUsage) {
        if let Some(last) = self.messages.last_mut() {
            if last.role == "assistant" {
                last.token_usage = Some(usage);
            }
        }
    }

    pub fn set_status(&mut self, status: String) {
        self.busy_status = status;
    }

    pub fn finish_turn(&mut self) {
        self.is_busy = false;
        self.busy_start = None;
    }

    pub fn clear_conversation(&mut self) {
        self.messages = vec![ChatMessage {
            role: "assistant".to_string(),
            content: "Hello! I am your SRElens AI Assistant. I can analyze pod crashes, diagnose cluster events, inspect configurations, and suggest Kubernetes remediation actions. Type your prompt below:".to_string(),
            timestamp: current_timestamp(),
            tool_calls: Vec::new(),
            token_usage: None,
        }];
        self.input.clear();
        self.is_busy = false;
        self.busy_status.clear();
        self.busy_start = None;
        self.scroll_offset = 0;
        self.auto_scroll = true;
        if let Ok(mut hist) = self.native_history.try_lock() {
            hist.clear();
        }
    }

    pub fn export_to_markdown(&self, provider_name: &str, model_name: &str) -> String {
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let mut md = String::new();
        md.push_str("# SRElens AI Assistant Conversation Export\n\n");
        md.push_str(&format!("- **Exported At**: {}\n", now));
        md.push_str(&format!("- **Provider**: {}\n", provider_name));
        md.push_str(&format!("- **Model**: {}\n\n", model_name));
        md.push_str("---\n\n");

        for msg in &self.messages {
            let role_label = match msg.role.as_str() {
                "user" => "### 👤 You",
                "assistant" => "### 🤖 SRElens Assistant",
                _ => "### ℹ️ System",
            };

            let time_str = if !msg.timestamp.is_empty() {
                format!(" [{}]", msg.timestamp)
            } else {
                String::new()
            };

            md.push_str(&format!("{}{}\n\n", role_label, time_str));

            let visible_tools: Vec<_> = msg.tool_calls.iter().filter(|tc| {
                tc.tool != "hookAdditionalContexts" && !(tc.args_summary.is_empty() && tc.tool.starts_with("hook"))
            }).collect();

            if !visible_tools.is_empty() {
                md.push_str("#### Executed Tools:\n");
                for tc in visible_tools {
                    let status = match &tc.status {
                        ToolCallStatus::Running => "running",
                        ToolCallStatus::Success => "ok",
                        ToolCallStatus::Error(e) => e.as_str(),
                    };
                    md.push_str(&format!("- `{}`: `{}` [{}]\n", tc.tool, tc.args_summary, status));
                }
                md.push_str("\n");
            }

            if !msg.content.trim().is_empty() {
                md.push_str(&msg.content);
                md.push_str("\n\n");
            }

            if let Some(usage) = &msg.token_usage {
                let duration_str = usage.duration_ms.map(|ms| {
                    if ms >= 1000 {
                        format!("{:.1}s", ms as f64 / 1000.0)
                    } else {
                        format!("{}ms", ms)
                    }
                }).unwrap_or_default();

                let cached_str = if usage.cached_tokens > 0 {
                    format!(" • {} cached", format_number(usage.cached_tokens))
                } else {
                    String::new()
                };

                let duration_badge = if !duration_str.is_empty() {
                    format!(" • {}", duration_str)
                } else {
                    String::new()
                };

                md.push_str(&format!(
                    "*⚡ {} tokens ({} prompt, {} completion{}){}*\n\n",
                    format_number(usage.total_tokens),
                    format_number(usage.prompt_tokens),
                    format_number(usage.completion_tokens),
                    cached_str,
                    duration_badge
                ));
            }

            md.push_str("---\n\n");
        }

        md
    }

    pub fn save_conversation_to_file(&self, provider_name: &str, model_name: &str, custom_path: Option<&str>) -> Result<std::path::PathBuf, String> {
        let md_content = self.export_to_markdown(provider_name, model_name);

        let path = if let Some(p) = custom_path {
            std::path::PathBuf::from(p)
        } else {
            let base_dir = dirs::config_dir()
                .map(|d| d.join("srelens").join("conversations"))
                .unwrap_or_else(|| std::path::PathBuf::from("srelens_conversations"));
            std::fs::create_dir_all(&base_dir).map_err(|e| format!("Failed to create export directory: {}", e))?;
            let filename = format!("srelens_ai_{}.md", chrono::Local::now().format("%Y%m%d_%H%M%S"));
            base_dir.join(filename)
        };

        std::fs::write(&path, md_content).map_err(|e| format!("Failed to save conversation: {}", e))?;
        Ok(path)
    }

    pub fn tick(&mut self) {
        if self.is_busy {
            self.spinner_frame = (self.spinner_frame + 1) % 10;
        }
    }

    pub fn scroll_up(&mut self, n: usize) {
        let max_scroll = self.last_max_scroll.get();
        let current = if self.auto_scroll {
            max_scroll
        } else {
            self.scroll_offset.min(max_scroll)
        };
        self.scroll_offset = current.saturating_sub(n);
        self.auto_scroll = false;
    }

    pub fn scroll_down(&mut self, n: usize) {
        let max_scroll = self.last_max_scroll.get();
        if self.auto_scroll {
            return;
        }
        let next = self.scroll_offset.saturating_add(n);
        if next >= max_scroll {
            self.scroll_offset = max_scroll;
            self.auto_scroll = true;
        } else {
            self.scroll_offset = next;
        }
    }

    pub fn scroll_to_top(&mut self) {
        self.auto_scroll = false;
        self.scroll_offset = 0;
    }

    pub fn scroll_to_bottom(&mut self) {
        self.auto_scroll = true;
        self.scroll_offset = self.last_max_scroll.get();
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
        " SRElens AI Assistant [{} - {}] [<Ctrl+e> Save, <Ctrl+l> Clear, <Ctrl+s> Settings, <Esc> Back] ",
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
            Constraint::Length(1), // Blank separator between chat history and input window
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

        // 1a. Render tool calls (if any)
        for tc in &msg.tool_calls {
            // Filter out internal metadata hooks that are not user-facing cluster tools
            if tc.tool == "hookAdditionalContexts" || (tc.args_summary.is_empty() && tc.tool.starts_with("hook")) {
                continue;
            }

            let (status_badge, status_style) = match &tc.status {
                ToolCallStatus::Running => ("[⠋ running]", Theme::status_warn()),
                ToolCallStatus::Success => ("[✓ ok]", Theme::status_ok()),
                ToolCallStatus::Error(_err) => ("[✗ error]", Theme::status_error()),
            };

            let arg_preview = if tc.args_summary.len() > 70 {
                format!("{}...", &tc.args_summary[..67])
            } else {
                tc.args_summary.clone()
            };

            rendered_lines.push(Line::from(vec![
                Span::styled("  ┌─ ⚙ Tool: ", Style::default().fg(Theme::CYAN).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{} ", tc.tool), Style::default().fg(Theme::ACCENT).add_modifier(Modifier::BOLD)),
                Span::styled("─".repeat(25), Style::default().fg(Theme::DIM)),
            ]));

            let command_display = if arg_preview.is_empty() {
                format!("executing {} ", tc.tool)
            } else {
                format!("$ {} ", arg_preview)
            };

            rendered_lines.push(Line::from(vec![
                Span::styled("  │ ", Style::default().fg(Theme::CYAN)),
                Span::styled(command_display, Style::default().fg(Color::White)),
                Span::styled(format!(" {}", status_badge), status_style),
            ]));

            rendered_lines.push(Line::from(vec![
                Span::styled("  └", Style::default().fg(Theme::CYAN)),
                Span::styled("─".repeat(45), Style::default().fg(Theme::DIM)),
            ]));
        }

        // 1b. Format message body (rich rendered markdown)
        if !msg.content.trim().is_empty() {
            format_message_content(&mut rendered_lines, &msg.content);
        }

        // 1c. Render token usage footer (if any)
        if let Some(usage) = &msg.token_usage {
            let duration_str = usage.duration_ms.map(|ms| {
                if ms >= 1000 {
                    format!("{:.1}s", ms as f64 / 1000.0)
                } else {
                    format!("{}ms", ms)
                }
            }).unwrap_or_default();

            let cached_str = if usage.cached_tokens > 0 {
                format!(" • {} cached", format_number(usage.cached_tokens))
            } else {
                String::new()
            };

            let duration_badge = if !duration_str.is_empty() {
                format!(" • {}", duration_str)
            } else {
                String::new()
            };

            rendered_lines.push(Line::from(vec![
                Span::styled("  ⚡ ", Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)),
                Span::styled(
                    format!(
                        "{} tokens ({} prompt, {} completion{}){}",
                        format_number(usage.total_tokens),
                        format_number(usage.prompt_tokens),
                        format_number(usage.completion_tokens),
                        cached_str,
                        duration_badge
                    ),
                    Style::default().fg(Theme::DIM).add_modifier(Modifier::ITALIC),
                ),
            ]));
        }

        rendered_lines.push(Line::from(""));
    }

    if state.is_busy {
        let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let spinner = frames[state.spinner_frame % frames.len()];
        let elapsed_secs = state
            .busy_start
            .map(|t| t.elapsed().as_secs())
            .unwrap_or(0);

        let status_text = if state.busy_status.is_empty() {
            "Consulting AI provider & cluster state..."
        } else {
            &state.busy_status
        };

        rendered_lines.push(Line::from(vec![
            Span::styled(format!("  {} ", spinner), Style::default().fg(Theme::CYAN).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{} ", status_text), Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled(format!("({}s elapsed)", elapsed_secs), Style::default().fg(Theme::DIM)),
        ]));
    }

    // Add breathing room of two empty lines between generated text and the input window
    rendered_lines.push(Line::from(""));
    rendered_lines.push(Line::from(""));

    let total_lines = rendered_lines.len();
    let viewport_height = chunks[0].height as usize;
    let max_scroll = total_lines.saturating_sub(viewport_height);
    state.last_max_scroll.set(max_scroll);
    state.last_total_lines.set(total_lines);

    let effective_scroll = if state.auto_scroll {
        max_scroll
    } else {
        state.scroll_offset.min(max_scroll)
    };

    let history_widget = Paragraph::new(rendered_lines)
        .wrap(Wrap { trim: false })
        .scroll((effective_scroll as u16, 0));
    f.render_widget(history_widget, chunks[0]);

    // 2. Input box
    let input_title = if !state.auto_scroll && effective_scroll < max_scroll {
        format!(" Ask Assistant (<End> Follow bottom, ↑/↓ Scroll) [Line {}/{}] ", effective_scroll + 1, total_lines)
    } else {
        " Ask Assistant (<Option+⌫>/<Ctrl+⌫> Rubout, <Ctrl+e> Save, <Ctrl+l> Clear, <Ctrl+s> Settings) ".to_string()
    };
    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::CYAN))
        .title(input_title);
    let input_widget = Paragraph::new(format!("{}█", state.input))
        .block(input_block);
    f.render_widget(input_widget, chunks[2]);
}

/// Formats message text lines, detecting and rendering Markdown structures:
/// - Aligned box-drawing tables
/// - Framed code blocks (```lang ... ```)
/// - Bold colored headings (#, ##, ###, ####)
/// - Bullet and numbered lists (•, ◦, 1.)
/// - Blockquotes (▎)
/// - Horizontal dividers (───)
/// - Inline markdown: `code`, **bold**, *italic*, [link](url), ~~strikethrough~~
pub fn format_message_content(out: &mut Vec<Line<'static>>, content: &str) {
    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        // 1. Fenced Code Block: ```lang
        if trimmed.starts_with("```") {
            let lang = trimmed.trim_start_matches('`').trim();
            i += 1;
            let mut code_lines = Vec::new();
            while i < lines.len() && !lines[i].trim().starts_with("```") {
                code_lines.push(lines[i]);
                i += 1;
            }
            if i < lines.len() && lines[i].trim().starts_with("```") {
                i += 1; // skip closing ```
            }
            render_code_block(out, lang, &code_lines);
            continue;
        }

        // 2. Markdown Table
        if is_table_row(line) && i + 1 < lines.len() && is_table_separator(lines[i + 1]) {
            let header_line = lines[i];
            i += 2; // skip header and separator
            let mut row_lines = Vec::new();
            while i < lines.len() && is_table_row(lines[i]) && !is_table_separator(lines[i]) {
                row_lines.push(lines[i]);
                i += 1;
            }
            render_markdown_table(out, header_line, &row_lines);
            continue;
        }

        // 3. Horizontal Rule: --- or *** or ___
        if (trimmed.starts_with("---") || trimmed.starts_with("***") || trimmed.starts_with("___"))
            && trimmed.chars().all(|c| c == '-' || c == '*' || c == '_' || c.is_whitespace())
            && trimmed.len() >= 3
        {
            out.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    "────────────────────────────────────────────────────────────────────────",
                    Style::default().fg(Theme::BORDER),
                ),
            ]));
            i += 1;
            continue;
        }

        // 4. Headings: #, ##, ###, ####
        if trimmed.starts_with("# ") {
            let heading_text = trimmed[2..].trim();
            out.push(Line::from(""));
            let mut spans = vec![Span::styled(
                "  ▌ ",
                Style::default().fg(Theme::ACCENT).add_modifier(Modifier::BOLD),
            )];
            spans.extend(parse_inline_markdown_with_base_style(
                heading_text,
                Style::default().fg(Theme::ACCENT).add_modifier(Modifier::BOLD),
            ));
            out.push(Line::from(spans));
            i += 1;
            continue;
        } else if trimmed.starts_with("## ") {
            let heading_text = trimmed[3..].trim();
            out.push(Line::from(""));
            let mut spans = vec![Span::styled(
                "  ▌ ",
                Style::default().fg(Theme::CYAN).add_modifier(Modifier::BOLD),
            )];
            spans.extend(parse_inline_markdown_with_base_style(
                heading_text,
                Style::default().fg(Theme::CYAN).add_modifier(Modifier::BOLD),
            ));
            out.push(Line::from(spans));
            i += 1;
            continue;
        } else if trimmed.starts_with("### ") {
            let heading_text = trimmed[4..].trim();
            let mut spans = vec![Span::styled(
                "  ● ",
                Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD),
            )];
            spans.extend(parse_inline_markdown_with_base_style(
                heading_text,
                Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD),
            ));
            out.push(Line::from(spans));
            i += 1;
            continue;
        } else if trimmed.starts_with("#### ") {
            let heading_text = trimmed[5..].trim();
            let mut spans = vec![Span::styled(
                "    ",
                Style::default().fg(Theme::FG).add_modifier(Modifier::BOLD),
            )];
            spans.extend(parse_inline_markdown_with_base_style(
                heading_text,
                Style::default().fg(Theme::FG).add_modifier(Modifier::BOLD),
            ));
            out.push(Line::from(spans));
            i += 1;
            continue;
        }

        // 5. Blockquote: > text
        if trimmed.starts_with('>') {
            let quote_text = trimmed.trim_start_matches('>').trim();
            let mut spans = vec![
                Span::raw("  "),
                Span::styled("▎ ", Style::default().fg(Theme::YELLOW)),
            ];
            spans.extend(parse_inline_markdown_with_base_style(
                quote_text,
                Style::default().fg(Theme::DIM).add_modifier(Modifier::ITALIC),
            ));
            out.push(Line::from(spans));
            i += 1;
            continue;
        }

        // 6. Bullet lists: - , * , + 
        let leading_spaces = line.len() - line.trim_start().len();
        if trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ") {
            let item_text = &trimmed[2..];
            let bullet_symbol = if leading_spaces >= 2 { "◦ " } else { "• " };
            let bullet_color = if leading_spaces >= 2 { Theme::YELLOW } else { Theme::CYAN };
            let indent = " ".repeat(leading_spaces + 2);
            let mut spans = vec![
                Span::raw(indent),
                Span::styled(bullet_symbol, Style::default().fg(bullet_color).add_modifier(Modifier::BOLD)),
            ];
            let normalized_item = ensure_spacing_after_periods(item_text);
            spans.extend(parse_inline_markdown(&normalized_item));
            out.push(Line::from(spans));
            i += 1;
            continue;
        }

        // 7. Numbered lists: 1. , 2. 
        if let Some(dot_pos) = trimmed.find(". ") {
            let prefix = &trimmed[..dot_pos];
            if !prefix.is_empty() && prefix.chars().all(|c| c.is_numeric()) {
                let item_text = &trimmed[dot_pos + 2..];
                let indent = " ".repeat(leading_spaces + 2);
                let mut spans = vec![
                    Span::raw(indent),
                    Span::styled(format!("{}. ", prefix), Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)),
                ];
                let normalized_item = ensure_spacing_after_periods(item_text);
                spans.extend(parse_inline_markdown(&normalized_item));
                out.push(Line::from(spans));
                i += 1;
                continue;
            }
        }

        // 8. Normal text paragraph
        render_text_line(out, line);
        i += 1;
    }
}

fn render_code_block(out: &mut Vec<Line<'static>>, lang: &str, code_lines: &[&str]) {
    let border_style = Style::default().fg(Theme::BORDER);
    let lang_display = if lang.is_empty() { "code" } else { lang };

    // Header border: ┌── lang ────────────────────────────────────────
    let header_prefix = format!("── {} ", lang_display);
    let bar_len = 65usize.saturating_sub(header_prefix.len());
    let top_line = format!("┌{}{}", header_prefix, "─".repeat(bar_len));
    out.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(top_line, border_style),
    ]));

    // Code lines with prefix "│ "
    for code_line in code_lines {
        let mut spans = vec![
            Span::raw("  "),
            Span::styled("│ ", border_style),
        ];

        let trimmed_code = code_line.trim_start();
        if trimmed_code.starts_with('#') || trimmed_code.starts_with("//") {
            // Comment
            spans.push(Span::styled(
                code_line.to_string(),
                Style::default().fg(Theme::DIM).add_modifier(Modifier::ITALIC),
            ));
        } else if trimmed_code.contains(':') && !trimmed_code.starts_with("http") {
            // YAML / Key-value
            if let Some(colon_idx) = code_line.find(':') {
                let key = &code_line[..=colon_idx];
                let val = &code_line[colon_idx + 1..];
                spans.push(Span::styled(key.to_string(), Style::default().fg(Theme::CYAN)));
                spans.push(Span::styled(val.to_string(), Style::default().fg(Theme::FG)));
            } else {
                spans.push(Span::styled(code_line.to_string(), Style::default().fg(Theme::FG)));
            }
        } else {
            spans.push(Span::styled(code_line.to_string(), Style::default().fg(Theme::FG)));
        }
        out.push(Line::from(spans));
    }

    // Bottom border: └────────────────────────────────────────────────
    out.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(format!("└{}", "─".repeat(66)), border_style),
    ]));
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

/// Ensures that sentences separated by a period have a space after the period.
/// Handles cases where multi-part assistant outputs or separate stream blocks are concatenated without spaces (e.g. `deployed.HAMi`, `doing.ArgoCD`, `GPUs.I`).
/// Preserves code spans within backticks (`...`), URLs, decimal numbers, and abbreviations (e.g. U.S.A.).
pub fn ensure_spacing_after_periods(text: &str) -> String {
    let mut result = String::with_capacity(text.len() + 8);
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut in_code_span = false;

    let mut i = 0;
    while i < len {
        let c = chars[i];
        if c == '`' {
            in_code_span = !in_code_span;
            result.push(c);
            i += 1;
            continue;
        }

        result.push(c);

        if !in_code_span && c == '.' && i + 1 < len {
            let next_char = chars[i + 1];
            // If next char is an uppercase letter, and previous char is an alphabetic character
            if next_char.is_ascii_uppercase() && i > 0 && chars[i - 1].is_alphabetic() {
                // Check that it's not an abbreviation like U.S.A. or St.
                let prev_prev_is_dot = i >= 2 && chars[i - 2] == '.';
                let next_next_is_dot = i + 2 < len && chars[i + 2] == '.';
                if !prev_prev_is_dot && !next_next_is_dot {
                    result.push(' ');
                }
            }
        }
        i += 1;
    }

    result
}

fn render_text_line(out: &mut Vec<Line<'static>>, line: &str) {
    if line.trim().is_empty() {
        out.push(Line::from(""));
        return;
    }
    let normalized = ensure_spacing_after_periods(line);
    let mut spans = vec![Span::raw("  ")];
    spans.extend(parse_inline_markdown(&normalized));
    out.push(Line::from(spans));
}

pub fn parse_inline_markdown(line: &str) -> Vec<Span<'static>> {
    parse_inline_markdown_with_base_style(line, Style::default().fg(Theme::FG))
}

pub fn parse_inline_markdown_with_base_style(line: &str, base: Style) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    let mut text_buf = String::new();

    let flush_buf = |buf: &mut String, out: &mut Vec<Span<'static>>, style: Style| {
        if !buf.is_empty() {
            out.push(Span::styled(std::mem::take(buf), style));
        }
    };

    while i < chars.len() {
        // 1. Inline Code: `code`
        if chars[i] == '`' {
            flush_buf(&mut text_buf, &mut spans, base);
            i += 1;
            let mut code_buf = String::new();
            while i < chars.len() && chars[i] != '`' {
                code_buf.push(chars[i]);
                i += 1;
            }
            if i < chars.len() && chars[i] == '`' {
                i += 1; // skip closing `
            }
            spans.push(Span::styled(
                code_buf,
                Style::default().fg(Theme::CYAN).add_modifier(Modifier::BOLD),
            ));
            continue;
        }

        // 2. Bold + Italic: ***text***
        if i + 2 < chars.len() && chars[i] == '*' && chars[i + 1] == '*' && chars[i + 2] == '*' {
            flush_buf(&mut text_buf, &mut spans, base);
            i += 3;
            let mut inner = String::new();
            while i + 2 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '*' && chars[i + 2] == '*') {
                inner.push(chars[i]);
                i += 1;
            }
            if i + 2 < chars.len() {
                i += 3; // skip closing ***
            }
            spans.push(Span::styled(
                inner,
                base.add_modifier(Modifier::BOLD | Modifier::ITALIC).fg(Theme::YELLOW),
            ));
            continue;
        }

        // 3. Bold: **text**
        if i + 1 < chars.len() && chars[i] == '*' && chars[i + 1] == '*' {
            flush_buf(&mut text_buf, &mut spans, base);
            i += 2;
            let mut inner = String::new();
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '*') {
                inner.push(chars[i]);
                i += 1;
            }
            if i + 1 < chars.len() {
                i += 2; // skip closing **
            }
            spans.push(Span::styled(
                inner,
                base.add_modifier(Modifier::BOLD).fg(Theme::YELLOW),
            ));
            continue;
        }

        // 4. Italic: *text* (single asterisk)
        if chars[i] == '*' && (i == 0 || chars[i - 1] != '\\') {
            flush_buf(&mut text_buf, &mut spans, base);
            i += 1;
            let mut inner = String::new();
            while i < chars.len() && chars[i] != '*' {
                inner.push(chars[i]);
                i += 1;
            }
            if i < chars.len() && chars[i] == '*' {
                i += 1; // skip closing *
            }
            spans.push(Span::styled(
                inner,
                base.add_modifier(Modifier::ITALIC),
            ));
            continue;
        }

        // 5. Link: [label](url)
        if chars[i] == '[' {
            if let Some(close_bracket) = chars[i + 1..].iter().position(|&c| c == ']') {
                let bracket_end = i + 1 + close_bracket;
                if bracket_end + 1 < chars.len() && chars[bracket_end + 1] == '(' {
                    if let Some(close_paren) = chars[bracket_end + 2..].iter().position(|&c| c == ')') {
                        let paren_end = bracket_end + 2 + close_paren;
                        flush_buf(&mut text_buf, &mut spans, base);
                        let label: String = chars[i + 1..bracket_end].iter().collect();
                        let url: String = chars[bracket_end + 2..paren_end].iter().collect();
                        spans.push(Span::styled(
                            label,
                            Style::default().fg(Theme::CYAN).add_modifier(Modifier::UNDERLINED),
                        ));
                        spans.push(Span::styled(
                            format!(" ({})", url),
                            Style::default().fg(Theme::DIM),
                        ));
                        i = paren_end + 1;
                        continue;
                    }
                }
            }
        }

        // 6. Strikethrough: ~~text~~
        if i + 1 < chars.len() && chars[i] == '~' && chars[i + 1] == '~' {
            flush_buf(&mut text_buf, &mut spans, base);
            i += 2;
            let mut inner = String::new();
            while i + 1 < chars.len() && !(chars[i] == '~' && chars[i + 1] == '~') {
                inner.push(chars[i]);
                i += 1;
            }
            if i + 1 < chars.len() {
                i += 2; // skip closing ~~
            }
            spans.push(Span::styled(
                inner,
                base.add_modifier(Modifier::CROSSED_OUT).fg(Theme::DIM),
            ));
            continue;
        }

        text_buf.push(chars[i]);
        i += 1;
    }

    flush_buf(&mut text_buf, &mut spans, base);
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
    fn test_format_message_content_with_code_block() {
        let content = "\
Example config:
```yaml
apiVersion: v1
kind: Pod
metadata:
  name: gpu-test
```
Done.";

        let mut lines = Vec::new();
        format_message_content(&mut lines, content);

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

        assert!(text_dump.contains("┌── yaml"));
        assert!(text_dump.contains("│ apiVersion: v1"));
        assert!(text_dump.contains("│ kind: Pod"));
        assert!(text_dump.contains("└"));
        // Make sure no raw ``` remains
        assert!(!text_dump.contains("```"));
    }

    #[test]
    fn test_inline_markdown_rendering() {
        let line = "In the current context `data-processing-prod-eu-dus1`, **4 of 32 nodes** advertise a GPU.";
        let spans = parse_inline_markdown(line);
        let text_dump: String = spans.iter().map(|s| s.content.as_ref()).collect();

        // Ensure backticks and asterisks are stripped
        assert_eq!(text_dump, "In the current context data-processing-prod-eu-dus1, 4 of 32 nodes advertise a GPU.");
        assert!(!text_dump.contains('`'));
        assert!(!text_dump.contains('*'));
    }

    #[test]
    fn test_headings_and_lists() {
        let content = "\
### GPU Nodes Overview
- Node 1: T4
- Node 2: A100
> Important notice: check quotas.";

        let mut lines = Vec::new();
        format_message_content(&mut lines, content);

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

        assert!(text_dump.contains("● GPU Nodes Overview"));
        assert!(text_dump.contains("• Node 1: T4"));
        assert!(text_dump.contains("• Node 2: A100"));
        assert!(text_dump.contains("▎ Important notice: check quotas."));
        // Make sure raw ### and - are not in the rendered output
        assert!(!text_dump.contains("###"));
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

    #[test]
    fn test_ensure_spacing_after_periods() {
        let input = "deployed.HAMi is running. Next doing.ArgoCD owns it. GPUs.I have";
        let output = ensure_spacing_after_periods(input);
        assert_eq!(output, "deployed. HAMi is running. Next doing. ArgoCD owns it. GPUs. I have");

        // Code spans preserved
        let code_input = "Use `pod.Status` to inspect.";
        assert_eq!(ensure_spacing_after_periods(code_input), "Use `pod.Status` to inspect.");

        // Domain names and versions preserved
        let domain_input = "Visit https://srelens.io or v1.31.7 with 10.240.0.1";
        assert_eq!(ensure_spacing_after_periods(domain_input), "Visit https://srelens.io or v1.31.7 with 10.240.0.1");

        // Abbreviations preserved
        let abbrev_input = "Made in the U.S.A. today.";
        assert_eq!(ensure_spacing_after_periods(abbrev_input), "Made in the U.S.A. today.");
    }

    #[test]
    fn test_append_stream_chunk_spacing() {
        let mut state = AssistantViewState::new();
        state.add_user_message("check cluster".to_string());
        state.append_stream_chunk("I'll query the cluster and how it's deployed.");
        // Appending chunk starting with capital letter after a period adds space
        state.append_stream_chunk("HAMi is running in hami-system.");
        assert_eq!(
            state.messages.last().unwrap().content,
            "I'll query the cluster and how it's deployed. HAMi is running in hami-system."
        );
    }
}
