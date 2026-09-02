use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::ai_config::{
    default_base_url_for_provider, default_model_for_provider, env_var_for_provider,
    find_cursor_binary, provider_display_name, provider_slug, AiProvider, AiSettings,
    ALL_PROVIDERS,
};
use crate::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingField {
    ProviderToggle,
    ApiKey,
    Model,
    BaseUrl,
}

pub struct SettingsViewState {
    pub settings: AiSettings,
    pub selected_provider_idx: usize,
    pub selected_field: SettingField,
    pub is_editing: bool,
    pub edit_buffer: String,
    pub toast: Option<String>,
}

impl SettingsViewState {
    pub fn new() -> Self {
        let settings = AiSettings::load();
        let default_idx = ALL_PROVIDERS
            .iter()
            .position(|&p| p == settings.default_provider)
            .unwrap_or(0);

        Self {
            settings,
            selected_provider_idx: default_idx,
            selected_field: SettingField::ProviderToggle,
            is_editing: false,
            edit_buffer: String::new(),
            toast: None,
        }
    }

    pub fn current_provider(&self) -> AiProvider {
        ALL_PROVIDERS[self.selected_provider_idx % ALL_PROVIDERS.len()]
    }

    pub fn select_next_provider(&mut self) {
        if !self.is_editing {
            self.selected_provider_idx = (self.selected_provider_idx + 1) % ALL_PROVIDERS.len();
        }
    }

    pub fn select_prev_provider(&mut self) {
        if !self.is_editing {
            if self.selected_provider_idx > 0 {
                self.selected_provider_idx -= 1;
            } else {
                self.selected_provider_idx = ALL_PROVIDERS.len() - 1;
            }
        }
    }

    pub fn select_next_field(&mut self) {
        if self.is_editing {
            return;
        }
        let is_custom = self.current_provider() == AiProvider::OpenAiCompatible;
        self.selected_field = match self.selected_field {
            SettingField::ProviderToggle => SettingField::ApiKey,
            SettingField::ApiKey => SettingField::Model,
            SettingField::Model => {
                if is_custom {
                    SettingField::BaseUrl
                } else {
                    SettingField::ProviderToggle
                }
            }
            SettingField::BaseUrl => SettingField::ProviderToggle,
        };
    }

    pub fn select_prev_field(&mut self) {
        if self.is_editing {
            return;
        }
        let is_custom = self.current_provider() == AiProvider::OpenAiCompatible;
        self.selected_field = match self.selected_field {
            SettingField::ProviderToggle => {
                if is_custom {
                    SettingField::BaseUrl
                } else {
                    SettingField::Model
                }
            }
            SettingField::ApiKey => SettingField::ProviderToggle,
            SettingField::Model => SettingField::ApiKey,
            SettingField::BaseUrl => SettingField::Model,
        };
    }

    pub fn set_active_provider(&mut self) {
        self.settings.default_provider = self.current_provider();
    }

    pub fn start_editing(&mut self) {
        let provider = self.current_provider();
        let slug = provider_slug(provider);
        self.edit_buffer = match self.selected_field {
            SettingField::ProviderToggle => {
                self.set_active_provider();
                return;
            }
            SettingField::ApiKey => self.settings.api_keys.get(slug).cloned().unwrap_or_default(),
            SettingField::Model => self.settings.get_model(provider),
            SettingField::BaseUrl => self.settings.get_base_url(provider),
        };
        self.is_editing = true;
    }

    pub fn finish_editing(&mut self) {
        let provider = self.current_provider();
        let slug = provider_slug(provider).to_string();
        let val = self.edit_buffer.trim().to_string();

        match self.selected_field {
            SettingField::ProviderToggle => {}
            SettingField::ApiKey => {
                if val.is_empty() {
                    self.settings.api_keys.remove(&slug);
                } else {
                    self.settings.api_keys.insert(slug, val);
                }
            }
            SettingField::Model => {
                if val.is_empty() {
                    self.settings.models.remove(&slug);
                } else {
                    self.settings.models.insert(slug, val);
                }
            }
            SettingField::BaseUrl => {
                if val.is_empty() {
                    self.settings.base_urls.remove(&slug);
                } else {
                    self.settings.base_urls.insert(slug, val);
                }
            }
        }
        self.is_editing = false;
        self.edit_buffer.clear();
    }

    pub fn cancel_editing(&mut self) {
        self.is_editing = false;
        self.edit_buffer.clear();
    }

    pub fn save(&mut self) -> Result<std::path::PathBuf, String> {
        self.settings.save()
    }
}

pub fn render_settings_view(f: &mut Frame, area: Rect, state: &SettingsViewState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::ACCENT))
        .title(Span::styled(
            " SRElens AI & Assistant Settings ",
            Theme::title(),
        ));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Top description & instructions
            Constraint::Min(10),   // Provider list
            Constraint::Length(2), // Bottom key hints
        ])
        .split(inner);

    // 1. Top Header Description
    let active_prov_name = provider_display_name(state.settings.default_provider);
    let desc_lines = vec![
        Line::from(vec![
            Span::styled("Active AI Provider: ", Theme::header_label()),
            Span::styled(
                format!("● {} ", active_prov_name),
                Style::default().fg(Theme::GREEN).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("(model: {})", state.settings.get_model(state.settings.default_provider)),
                Style::default().fg(Theme::CYAN),
            ),
        ]),
        Line::from(vec![Span::styled(
            "Configure default provider, API keys, models, and local/CLI agents (including Cursor).",
            Style::default().fg(Theme::DIM),
        )]),
    ];
    f.render_widget(Paragraph::new(desc_lines), chunks[0]);

    // 2. Providers List (5 providers now!)
    let provider_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5), // Anthropic
            Constraint::Length(5), // OpenAI
            Constraint::Length(5), // Gemini
            Constraint::Length(5), // OpenAICompatible / Ollama
            Constraint::Length(5), // Cursor Agent
        ])
        .split(chunks[1]);

    for (idx, &provider) in ALL_PROVIDERS.iter().enumerate() {
        if idx >= provider_chunks.len() {
            break;
        }
        let is_selected_provider = idx == state.selected_provider_idx;
        let is_active_provider = provider == state.settings.default_provider;
        let provider_area = provider_chunks[idx];

        let border_color = if is_selected_provider {
            Theme::CYAN
        } else {
            Theme::BORDER
        };

        let card_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color));

        let card_inner = card_block.inner(provider_area);
        f.render_widget(card_block, provider_area);

        // Header line: Radio icon, Name, Active Badge
        let radio = if is_active_provider { "● " } else { "○ " };
        let radio_style = if is_active_provider {
            Style::default().fg(Theme::GREEN).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Theme::DIM)
        };

        let title_style = if is_selected_provider && state.selected_field == SettingField::ProviderToggle {
            Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)
        } else if is_active_provider {
            Style::default().fg(Theme::FG).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Theme::FG)
        };

        let mut header_spans = vec![
            Span::styled(radio, radio_style),
            Span::styled(format!("{}. {}", idx + 1, provider_display_name(provider)), title_style),
        ];

        if is_active_provider {
            header_spans.push(Span::raw("  "));
            header_spans.push(Span::styled(
                "[ACTIVE DEFAULT]",
                Style::default().fg(Theme::GREEN).add_modifier(Modifier::BOLD),
            ));
        }

        if provider == AiProvider::Cursor {
            if let Some(bin_path) = find_cursor_binary() {
                header_spans.push(Span::raw("  "));
                header_spans.push(Span::styled(
                    format!("[installed: {}]", bin_path),
                    Style::default().fg(Theme::GREEN),
                ));
            } else {
                header_spans.push(Span::raw("  "));
                header_spans.push(Span::styled(
                    "[not found on PATH]",
                    Style::default().fg(Theme::RED),
                ));
            }
        }

        // Field 1: API Key / Auth Display
        let key_focus = is_selected_provider && state.selected_field == SettingField::ApiKey;
        let key_label_style = if key_focus {
            Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Theme::DIM)
        };

        let raw_key = state.settings.api_keys.get(provider_slug(provider));
        let env_key = std::env::var(env_var_for_provider(provider)).ok();

        let key_value_display = if let Some(k) = raw_key.filter(|s| !s.trim().is_empty()) {
            let masked = if k.len() > 8 {
                format!("{}••••••••{}", &k[..4], &k[k.len() - 4..])
            } else {
                "••••••••".to_string()
            };
            Span::styled(format!("{} [stored in config]", masked), Style::default().fg(Theme::GREEN))
        } else if env_key.is_some() {
            Span::styled(
                format!("[env: {} set]", env_var_for_provider(provider)),
                Style::default().fg(Theme::CYAN),
            )
        } else if provider == AiProvider::Cursor {
            Span::styled("auto (uses logged-in cursor auth or CURSOR_API_KEY)", Style::default().fg(Theme::CYAN))
        } else if provider == AiProvider::OpenAiCompatible {
            Span::styled("optional (local Ollama)", Style::default().fg(Theme::DIM))
        } else {
            Span::styled(
                format!("no key set (press 'e' to set or export {})", env_var_for_provider(provider)),
                Style::default().fg(Theme::RED),
            )
        };

        // Field 2: Model Display
        let model_focus = is_selected_provider && state.selected_field == SettingField::Model;
        let model_label_style = if model_focus {
            Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Theme::DIM)
        };
        let current_model = state.settings.get_model(provider);

        let mut lines = vec![
            Line::from(header_spans),
            Line::from(vec![
                Span::styled(if provider == AiProvider::Cursor { "   Auth:    " } else { "   API Key: " }, key_label_style),
                key_value_display,
            ]),
            Line::from(vec![
                Span::styled("   Model:   ", model_label_style),
                Span::styled(current_model, if model_focus { Style::default().fg(Theme::YELLOW) } else { Style::default().fg(Theme::FG) }),
            ]),
        ];

        // Field 3: Base URL (for OpenAICompatible)
        if provider == AiProvider::OpenAiCompatible {
            let url_focus = is_selected_provider && state.selected_field == SettingField::BaseUrl;
            let url_label_style = if url_focus {
                Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Theme::DIM)
            };
            lines.push(Line::from(vec![
                Span::styled("   Base URL: ", url_label_style),
                Span::styled(state.settings.get_base_url(provider), if url_focus { Style::default().fg(Theme::YELLOW) } else { Style::default().fg(Theme::FG) }),
            ]));
        }

        f.render_widget(Paragraph::new(lines), card_inner);
    }

    // 3. Bottom Key Hints
    let hints_line = Line::from(vec![
        Span::styled("[↑/↓/j/k]", Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)),
        Span::styled(" Provider  ", Theme::header_label()),
        Span::styled("[Tab/←/→]", Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)),
        Span::styled(" Field  ", Theme::header_label()),
        Span::styled("[Space]", Style::default().fg(Theme::GREEN).add_modifier(Modifier::BOLD)),
        Span::styled(" Set Active  ", Theme::header_label()),
        Span::styled("[e/Enter]", Style::default().fg(Theme::CYAN).add_modifier(Modifier::BOLD)),
        Span::styled(" Edit  ", Theme::header_label()),
        Span::styled("[s]", Style::default().fg(Theme::ACCENT).add_modifier(Modifier::BOLD)),
        Span::styled(" Save to Disk  ", Theme::header_label()),
        Span::styled("[Esc/q]", Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)),
        Span::styled(" Back", Theme::header_label()),
    ]);
    f.render_widget(Paragraph::new(hints_line).alignment(Alignment::Center), chunks[2]);

    // 4. Modal Edit Input Dialog (when editing)
    if state.is_editing {
        let edit_area = crate::ui::help::centered_rect(60, 25, area);
        f.render_widget(Clear, edit_area);

        let field_name = match state.selected_field {
            SettingField::ApiKey => if state.current_provider() == AiProvider::Cursor { "API Key (or leave blank for cursor login)" } else { "API Key" },
            SettingField::Model => "Model ID",
            SettingField::BaseUrl => "Base URL (e.g. http://localhost:11434/v1)",
            SettingField::ProviderToggle => "Provider",
        };

        let edit_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Theme::YELLOW))
            .title(format!(" Edit {} for {} ", field_name, provider_display_name(state.current_provider())));

        let edit_inner = edit_block.inner(edit_area);
        f.render_widget(edit_block, edit_area);

        let edit_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(3),
                Constraint::Length(2),
            ])
            .split(edit_inner);

        let prompt = Paragraph::new(format!("Enter new value for {}:", field_name));
        f.render_widget(prompt, edit_chunks[0]);

        let input_line = Line::from(vec![
            Span::styled(&state.edit_buffer, Style::default().fg(Theme::FG).add_modifier(Modifier::BOLD)),
            Span::styled("█", Style::default().fg(Theme::CYAN)),
        ]);
        let input_box = Paragraph::new(input_line)
            .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Theme::CYAN)));
        f.render_widget(input_box, edit_chunks[1]);

        let help = Line::from(vec![
            Span::styled("<Enter> ", Style::default().fg(Theme::GREEN).add_modifier(Modifier::BOLD)),
            Span::styled("Confirm  |  ", Theme::header_label()),
            Span::styled("<Ctrl+w> ", Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled("Rubout  |  ", Theme::header_label()),
            Span::styled("<Esc> ", Style::default().fg(Theme::RED).add_modifier(Modifier::BOLD)),
            Span::styled("Cancel", Theme::header_label()),
        ]);
        f.render_widget(Paragraph::new(help).alignment(Alignment::Center), edit_chunks[2]);
    }
}
