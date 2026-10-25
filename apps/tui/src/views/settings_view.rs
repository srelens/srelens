use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use std::collections::HashMap;
use std::sync::LazyLock;

use srelens_agent::adapter::{valid_model_id, AgentKind, ModelChoice};

use crate::ai_config::{
    default_base_url_for_provider, default_model_for_provider, env_var_for_provider,
    provider_display_name, provider_slug, AiProvider, AiSettings, ALL_PROVIDERS,
};
use crate::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingField {
    ProviderToggle,
    ApiKey,
    Model,
    BaseUrl,
    Timeout,
}

/// What a model picker picks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerTarget {
    Provider(AiProvider),
    Agent(AgentKind),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerLoad {
    Loading,
    Ready,
    Failed(String),
}

/// The model picker drawn over the settings view: the target's list (from
/// the cache or a fetch under `key`), filtered by what the user types.
#[derive(Debug, Clone)]
pub struct ModelPicker {
    pub target: PickerTarget,
    pub key: String,
    pub query: String,
    pub items: Vec<ModelChoice>,
    pub load: PickerLoad,
    /// Index into [`ModelPicker::visible`].
    pub selected: usize,
}

/// An agent list's first entry: no model flag, the CLI decides. Its empty id
/// is no model id, which is how [`ModelPicker::choose`] tells it apart.
static DEFAULT_MODEL: LazyLock<ModelChoice> = LazyLock::new(|| ModelChoice {
    id: String::new(),
    label: "Default".to_string(),
});

impl ModelPicker {
    /// The rows the query matches (case-insensitive, in the id or the label);
    /// an agent's list starts with Default.
    pub fn visible(&self) -> Vec<&ModelChoice> {
        let query = self.query.trim().to_lowercase();
        let default = matches!(self.target, PickerTarget::Agent(_)).then(|| &*DEFAULT_MODEL);
        default
            .into_iter()
            .chain(&self.items)
            .filter(|m| {
                m.id.to_lowercase().contains(&query) || m.label.to_lowercase().contains(&query)
            })
            .collect()
    }

    /// The highlighted row's id (`None` for Default), or the typed id when no
    /// row matches; a typed id that could reach argv as a flag is refused.
    pub fn choose(&self) -> Result<Option<String>, String> {
        if let Some(m) = self.visible().get(self.selected) {
            return Ok((!m.id.is_empty()).then(|| m.id.clone()));
        }
        let typed = self.query.trim();
        if valid_model_id(typed) {
            Ok(Some(typed.to_string()))
        } else {
            Err(format!("not a model id: {typed}"))
        }
    }

    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn move_down(&mut self) {
        if self.selected + 1 < self.visible().len() {
            self.selected += 1;
        }
    }
}

pub struct SettingsViewState {
    pub settings: AiSettings,
    pub selected_provider_idx: usize,
    pub selected_field: SettingField,
    pub is_editing: bool,
    pub edit_buffer: String,
    pub toast: Option<String>,
    pub picker: Option<ModelPicker>,
    /// Model lists fetched this visit, by cache key; failures are not kept.
    pub model_cache: HashMap<String, Vec<ModelChoice>>,
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
            picker: None,
            model_cache: HashMap::new(),
        }
    }

    /// Open the picker on `target`'s list under `key`; true when that list
    /// is not cached and must be fetched.
    pub fn open_picker(&mut self, target: PickerTarget, key: String) -> bool {
        let cached = self.model_cache.get(&key).cloned();
        let fetch = cached.is_none();
        self.picker = Some(ModelPicker {
            target,
            key,
            query: String::new(),
            items: cached.unwrap_or_default(),
            load: if fetch {
                PickerLoad::Loading
            } else {
                PickerLoad::Ready
            },
            selected: 0,
        });
        fetch
    }

    /// A fetched list (or why there is none) for `key`.
    pub fn models_loaded(&mut self, key: &str, result: Result<Vec<ModelChoice>, String>) {
        if let Ok(models) = &result {
            self.model_cache.insert(key.to_string(), models.clone());
        }
        let Some(picker) = self.picker.as_mut().filter(|p| p.key == key) else {
            return;
        };
        match result {
            Ok(models) => {
                picker.items = models;
                picker.load = PickerLoad::Ready;
            }
            Err(e) => picker.load = PickerLoad::Failed(e),
        }
        picker.selected = 0;
    }

    /// Store the picker's choice where its target reads it and close the
    /// picker; a refused typed id leaves it open.
    pub fn pick_model(&mut self) -> Result<Option<String>, String> {
        let Some(picker) = &self.picker else {
            return Err("no model picker is open".to_string());
        };
        let model = picker.choose()?;
        match picker.target {
            PickerTarget::Provider(p) => {
                let slug = provider_slug(p).to_string();
                match model.clone() {
                    Some(id) => self.settings.models.insert(slug, id),
                    None => self.settings.models.remove(&slug),
                };
            }
            PickerTarget::Agent(kind) => self.settings.set_agent_model(kind, model.clone()),
        }
        self.picker = None;
        Ok(model)
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
                    SettingField::Timeout
                }
            }
            SettingField::BaseUrl => SettingField::Timeout,
            SettingField::Timeout => SettingField::ProviderToggle,
        };
    }

    pub fn select_prev_field(&mut self) {
        if self.is_editing {
            return;
        }
        let is_custom = self.current_provider() == AiProvider::OpenAiCompatible;
        self.selected_field = match self.selected_field {
            SettingField::ProviderToggle => SettingField::Timeout,
            SettingField::Timeout => {
                if is_custom {
                    SettingField::BaseUrl
                } else {
                    SettingField::Model
                }
            }
            SettingField::BaseUrl => SettingField::Model,
            SettingField::Model => SettingField::ApiKey,
            SettingField::ApiKey => SettingField::ProviderToggle,
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
            SettingField::ApiKey => self
                .settings
                .api_keys
                .get(slug)
                .cloned()
                .unwrap_or_default(),
            SettingField::Model => self.settings.get_model(provider),
            SettingField::BaseUrl => self.settings.get_base_url(provider),
            SettingField::Timeout => self.settings.get_timeout_seconds(provider).to_string(),
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
            SettingField::Timeout => {
                if let Ok(secs) = val.parse::<u32>() {
                    self.settings
                        .set_timeout_seconds(provider, secs.clamp(5, 3600));
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
            "Configure default provider, API keys, models, and timeouts.",
            Style::default().fg(Theme::DIM),
        )]),
    ];
    f.render_widget(
        Paragraph::new(desc_lines).wrap(Wrap { trim: true }),
        chunks[0],
    );

    // 2. Providers List
    let provider_constraints: Vec<Constraint> = ALL_PROVIDERS
        .iter()
        .map(|&p| {
            if p == AiProvider::OpenAiCompatible {
                Constraint::Length(6)
            } else {
                Constraint::Length(5)
            }
        })
        .collect();

    let provider_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(provider_constraints)
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
            Style::default()
                .fg(Theme::GREEN)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Theme::DIM)
        };

        let title_style =
            if is_selected_provider && state.selected_field == SettingField::ProviderToggle {
                Style::default()
                    .fg(Theme::YELLOW)
                    .add_modifier(Modifier::BOLD)
            } else if is_active_provider {
                Style::default().fg(Theme::FG).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Theme::FG)
            };

        let mut header_spans = vec![
            Span::styled(radio, radio_style),
            Span::styled(
                format!("{}. {}", idx + 1, provider_display_name(provider)),
                title_style,
            ),
        ];

        if is_active_provider {
            header_spans.push(Span::raw("  "));
            header_spans.push(Span::styled(
                "[ACTIVE DEFAULT]",
                Style::default()
                    .fg(Theme::GREEN)
                    .add_modifier(Modifier::BOLD),
            ));
        }

        // Field 1: API Key / Auth Display
        let key_focus = is_selected_provider && state.selected_field == SettingField::ApiKey;
        let key_label_style = if key_focus {
            Style::default()
                .fg(Theme::YELLOW)
                .add_modifier(Modifier::BOLD)
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
            Span::styled(
                format!("{} [stored in config]", masked),
                Style::default().fg(Theme::GREEN),
            )
        } else if env_key.is_some() {
            Span::styled(
                format!("[env: {} set]", env_var_for_provider(provider)),
                Style::default().fg(Theme::CYAN),
            )
        } else if provider == AiProvider::OpenAiCompatible {
            Span::styled("optional (local Ollama)", Style::default().fg(Theme::DIM))
        } else {
            Span::styled(
                format!(
                    "no key set (press 'e' to set or export {})",
                    env_var_for_provider(provider)
                ),
                Style::default().fg(Theme::RED),
            )
        };

        // Field 2: Model Display
        let model_focus = is_selected_provider && state.selected_field == SettingField::Model;
        let model_label_style = if model_focus {
            Style::default()
                .fg(Theme::YELLOW)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Theme::DIM)
        };
        let current_model = state.settings.get_model(provider);

        // Field: Timeout Display
        let timeout_focus = is_selected_provider && state.selected_field == SettingField::Timeout;
        let timeout_label_style = if timeout_focus {
            Style::default()
                .fg(Theme::YELLOW)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Theme::DIM)
        };
        let timeout_display = format!("{}s", state.settings.get_timeout_seconds(provider));

        let mut lines = vec![
            Line::from(header_spans),
            Line::from(vec![
                Span::styled("   API Key: ", key_label_style),
                key_value_display,
            ]),
            Line::from(vec![
                Span::styled("   Model:   ", model_label_style),
                Span::styled(
                    current_model,
                    if model_focus {
                        Style::default().fg(Theme::YELLOW)
                    } else {
                        Style::default().fg(Theme::FG)
                    },
                ),
                Span::styled("   │   Timeout: ", timeout_label_style),
                Span::styled(
                    timeout_display,
                    if timeout_focus {
                        Style::default().fg(Theme::YELLOW)
                    } else {
                        Style::default().fg(Theme::FG)
                    },
                ),
            ]),
        ];

        // Field 3: Base URL (for OpenAICompatible)
        if provider == AiProvider::OpenAiCompatible {
            let url_focus = is_selected_provider && state.selected_field == SettingField::BaseUrl;
            let url_label_style = if url_focus {
                Style::default()
                    .fg(Theme::YELLOW)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Theme::DIM)
            };
            lines.push(Line::from(vec![
                Span::styled("   Base URL: ", url_label_style),
                Span::styled(
                    state.settings.get_base_url(provider),
                    if url_focus {
                        Style::default().fg(Theme::YELLOW)
                    } else {
                        Style::default().fg(Theme::FG)
                    },
                ),
            ]));
        }

        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), card_inner);
    }

    // 3. Bottom Key Hints
    let hints_line = Line::from(vec![
        Span::styled(
            "[↑/↓/j/k]",
            Style::default()
                .fg(Theme::YELLOW)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Provider  ", Theme::header_label()),
        Span::styled(
            "[Tab/←/→]",
            Style::default()
                .fg(Theme::YELLOW)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Field  ", Theme::header_label()),
        Span::styled(
            "[Space]",
            Style::default()
                .fg(Theme::GREEN)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Set Active  ", Theme::header_label()),
        Span::styled(
            "[e/Enter]",
            Style::default()
                .fg(Theme::CYAN)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Edit  ", Theme::header_label()),
        Span::styled(
            "[s]",
            Style::default()
                .fg(Theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Save to Disk  ", Theme::header_label()),
        Span::styled(
            "[Esc/q]",
            Style::default()
                .fg(Theme::YELLOW)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Back", Theme::header_label()),
    ]);
    f.render_widget(
        Paragraph::new(hints_line)
            .wrap(Wrap { trim: true })
            .alignment(Alignment::Center),
        chunks[2],
    );

    // 4. Modal Edit Input Dialog (when editing)
    if state.is_editing {
        let edit_area = crate::ui::help::centered_rect(60, 25, area);
        f.render_widget(Clear, edit_area);

        let field_name = match state.selected_field {
            SettingField::ApiKey => "API Key",
            SettingField::Model => "Model ID",
            SettingField::BaseUrl => "Base URL (e.g. http://localhost:11434/v1)",
            SettingField::Timeout => "Turn Timeout (seconds, e.g. 120)",
            SettingField::ProviderToggle => "Provider",
        };

        let edit_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Theme::YELLOW))
            .title(format!(
                " Edit {} for {} ",
                field_name,
                provider_display_name(state.current_provider())
            ));

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

        let prompt = Paragraph::new(format!("Enter new value for {}:", field_name))
            .wrap(Wrap { trim: true });
        f.render_widget(prompt, edit_chunks[0]);

        let input_line = Line::from(vec![
            Span::styled(
                &state.edit_buffer,
                Style::default().fg(Theme::FG).add_modifier(Modifier::BOLD),
            ),
            Span::styled("█", Style::default().fg(Theme::CYAN)),
        ]);
        let input_box = Paragraph::new(input_line).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Theme::CYAN)),
        );
        f.render_widget(input_box, edit_chunks[1]);

        let help = Line::from(vec![
            Span::styled(
                "<Enter> ",
                Style::default()
                    .fg(Theme::GREEN)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("Confirm  |  ", Theme::header_label()),
            Span::styled(
                "<Ctrl+v> ",
                Style::default()
                    .fg(Theme::CYAN)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("Paste  |  ", Theme::header_label()),
            Span::styled(
                "<Ctrl+w> ",
                Style::default()
                    .fg(Theme::YELLOW)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("Rubout  |  ", Theme::header_label()),
            Span::styled(
                "<Esc> ",
                Style::default().fg(Theme::RED).add_modifier(Modifier::BOLD),
            ),
            Span::styled("Cancel", Theme::header_label()),
        ]);
        f.render_widget(
            Paragraph::new(help)
                .wrap(Wrap { trim: true })
                .alignment(Alignment::Center),
            edit_chunks[2],
        );
    }

    if let Some(picker) = &state.picker {
        render_model_picker(f, area, picker);
    }
}

/// The picker overlay: the target, the filter, the list scrolled to keep
/// the highlighted row in view, and one status line.
fn render_model_picker(f: &mut Frame, area: Rect, picker: &ModelPicker) {
    let rect = crate::ui::help::centered_rect(70, 70, area);
    f.render_widget(Clear, rect);
    let target = match picker.target {
        PickerTarget::Provider(p) => provider_display_name(p),
        PickerTarget::Agent(kind) => kind.label(),
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::YELLOW))
        .title(format!(" Model for {target} "))
        .title_bottom(" ↑/↓ Move  Enter Pick (or the typed id)  Esc Cancel ");
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(inner);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Filter: ", Theme::header_label()),
            Span::styled(
                picker.query.as_str(),
                Style::default().fg(Theme::FG).add_modifier(Modifier::BOLD),
            ),
            Span::styled("█", Style::default().fg(Theme::CYAN)),
        ])),
        rows[0],
    );

    let visible = picker.visible();
    let height = rows[1].height as usize;
    let first = (picker.selected + 1).saturating_sub(height);
    let lines: Vec<Line> = visible
        .iter()
        .enumerate()
        .skip(first)
        .take(height)
        .map(|(i, m)| {
            let text = match (m.id.as_str(), m.label.as_str()) {
                (id, "") => id.to_string(),
                ("", label) => label.to_string(),
                (id, label) if id == label => id.to_string(),
                (id, label) => format!("{label}  ({id})"),
            };
            if i == picker.selected {
                Line::from(Span::styled(
                    format!("> {text}"),
                    Style::default()
                        .fg(Theme::YELLOW)
                        .add_modifier(Modifier::BOLD),
                ))
            } else {
                Line::from(format!("  {text}"))
            }
        })
        .collect();
    f.render_widget(Paragraph::new(lines), rows[1]);

    let (status, style) = match &picker.load {
        PickerLoad::Loading => (
            "Loading models…".to_string(),
            Style::default().fg(Theme::DIM),
        ),
        PickerLoad::Failed(e) => (e.clone(), Style::default().fg(Theme::RED)),
        PickerLoad::Ready if visible.is_empty() => (
            "type a model id and press Enter".to_string(),
            Style::default().fg(Theme::DIM),
        ),
        PickerLoad::Ready => (String::new(), Style::default()),
    };
    f.render_widget(Paragraph::new(status).style(style), rows[2]);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(id: &str, label: &str) -> ModelChoice {
        ModelChoice {
            id: id.into(),
            label: label.into(),
        }
    }

    fn picker(target: PickerTarget) -> ModelPicker {
        ModelPicker {
            target,
            key: "k".into(),
            query: String::new(),
            items: vec![
                choice("gpt-5.6-sol", "GPT-5.6 Sol"),
                choice("gpt-5.6-terra", "GPT-5.6 Terra"),
            ],
            load: PickerLoad::Ready,
            selected: 0,
        }
    }

    fn state() -> SettingsViewState {
        SettingsViewState {
            settings: AiSettings::default(),
            selected_provider_idx: 0,
            selected_field: SettingField::Model,
            is_editing: false,
            edit_buffer: String::new(),
            toast: None,
            picker: None,
            model_cache: HashMap::new(),
        }
    }

    fn ids(p: &ModelPicker) -> Vec<&str> {
        p.visible().iter().map(|m| m.id.as_str()).collect()
    }

    #[test]
    fn typing_filters_ids_and_labels_ignoring_case() {
        let mut p = picker(PickerTarget::Provider(AiProvider::OpenAi));
        p.query = "SOL".into();
        assert_eq!(ids(&p), ["gpt-5.6-sol"]);
        // Only the label has the space.
        p.query = "6 TERRA".into();
        assert_eq!(ids(&p), ["gpt-5.6-terra"]);
    }

    #[test]
    fn enter_takes_the_highlighted_model() {
        let mut p = picker(PickerTarget::Provider(AiProvider::OpenAi));
        p.move_down();
        p.move_down();
        assert_eq!(p.selected, 1, "stops at the last row");
        assert_eq!(p.choose(), Ok(Some("gpt-5.6-terra".to_string())));
        p.move_up();
        p.move_up();
        assert_eq!(p.choose(), Ok(Some("gpt-5.6-sol".to_string())));
    }

    #[test]
    fn enter_with_no_match_takes_the_typed_id() {
        let mut p = picker(PickerTarget::Provider(AiProvider::OpenAi));
        p.query = "my-model".into();
        assert!(p.visible().is_empty());
        assert_eq!(p.choose(), Ok(Some("my-model".to_string())));
    }

    #[test]
    fn a_typed_id_that_could_be_a_flag_is_refused() {
        let mut p = picker(PickerTarget::Agent(AgentKind::Codex));
        p.query = "--x".into();
        assert_eq!(p.choose(), Err("not a model id: --x".to_string()));
    }

    #[test]
    fn agent_lists_offer_default_first_and_it_means_no_model() {
        let mut p = picker(PickerTarget::Agent(AgentKind::Codex));
        assert_eq!(p.visible()[0].label, "Default");
        assert_eq!(p.visible().len(), 3);
        assert_eq!(p.choose(), Ok(None));
        p.move_down();
        assert_eq!(p.choose(), Ok(Some("gpt-5.6-sol".to_string())));

        let p = picker(PickerTarget::Provider(AiProvider::OpenAi));
        assert_eq!(ids(&p), ["gpt-5.6-sol", "gpt-5.6-terra"], "no Default");
    }

    #[test]
    fn a_failed_list_is_fetched_again_next_time() {
        let mut s = state();
        let target = PickerTarget::Provider(AiProvider::OpenAi);
        assert!(s.open_picker(target, "k".into()), "nothing cached yet");
        assert_eq!(s.picker.as_ref().unwrap().load, PickerLoad::Loading);

        s.models_loaded("k", Err("offline".into()));
        assert_eq!(
            s.picker.as_ref().unwrap().load,
            PickerLoad::Failed("offline".into())
        );
        assert!(s.open_picker(target, "k".into()), "a failure is not kept");

        s.models_loaded("k", Ok(vec![choice("m-1", "One")]));
        assert!(!s.open_picker(target, "k".into()), "a list is kept");
        let p = s.picker.as_ref().unwrap();
        assert_eq!((&p.load, ids(p)), (&PickerLoad::Ready, vec!["m-1"]));
    }

    #[test]
    fn picking_default_for_an_agent_clears_its_model() {
        let mut s = state();
        s.settings
            .set_agent_model(AgentKind::Codex, Some("gpt-5.6-sol".into()));
        s.open_picker(PickerTarget::Agent(AgentKind::Codex), "a".into());
        assert_eq!(s.pick_model(), Ok(None));
        assert_eq!(s.settings.get_agent_model(AgentKind::Codex), None);
        assert!(s.picker.is_none());
    }

    #[test]
    fn a_refused_id_keeps_the_picker_open_and_the_model_unchanged() {
        let mut s = state();
        s.open_picker(PickerTarget::Provider(AiProvider::OpenAi), "p".into());
        s.picker.as_mut().unwrap().query = "-x".into();
        assert!(s.pick_model().is_err());
        assert!(s.picker.is_some());
        assert_eq!(s.settings.get_model(AiProvider::OpenAi), "gpt-4o");

        s.picker.as_mut().unwrap().query = "gpt-x".into();
        assert_eq!(s.pick_model(), Ok(Some("gpt-x".to_string())));
        assert_eq!(s.settings.get_model(AiProvider::OpenAi), "gpt-x");
    }
}
