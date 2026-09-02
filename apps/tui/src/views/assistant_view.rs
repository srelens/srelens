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
            messages: vec![
                ChatMessage {
                    role: "assistant".to_string(),
                    content: "Hello! I am your SRElens AI Assistant. I can analyze pod crashes, diagnose cluster events, inspect configurations, and suggest Kubernetes remediation actions. Type your prompt below:".to_string(),
                }
            ],
            input: String::new(),
            is_busy: false,
            scroll_offset: 0,
        }
    }

    pub fn add_user_message(&mut self, text: String) {
        self.messages.push(ChatMessage {
            role: "user".to_string(),
            content: text,
        });
        self.input.clear();
        self.is_busy = true;
    }

    pub fn add_assistant_message(&mut self, text: String) {
        self.messages.push(ChatMessage {
            role: "assistant".to_string(),
            content: text,
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
        let (prefix, style) = match msg.role.as_str() {
            "user" => ("You: ", Style::default().fg(Theme::CYAN).add_modifier(Modifier::BOLD)),
            "assistant" => ("SRElens: ", Style::default().fg(Theme::ACCENT).add_modifier(Modifier::BOLD)),
            _ => ("System: ", Style::default().fg(Theme::YELLOW)),
        };

        rendered_lines.push(Line::from(vec![Span::styled(prefix, style)]));
        for line in msg.content.lines() {
            rendered_lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(line.to_string(), Style::default().fg(Theme::FG)),
            ]));
        }
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
