use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use std::path::PathBuf;
use std::sync::LazyLock;

use srelens_agent::adapter::{valid_model_id, AgentKind, ModelChoice};

use crate::ai_config::{
    agent_install_url, agent_slug, default_base_url_for_provider, default_model_for_provider,
    env_var_for_provider, find_agent_in, provider_display_name, provider_slug, AiProvider,
    AiSettings, ALL_PROVIDERS,
};
use crate::theme::Theme;
use srelens_kube::toolbox::SearchPaths;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingField {
    /// The Agent card's agent row; its other field is [`SettingField::Model`].
    Agent,
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
    /// an agent's list starts with Default. An id equal to the query comes
    /// first, so Enter takes `llama3.2` over a `llama3.2:3b` listed above it.
    pub fn visible(&self) -> Vec<&ModelChoice> {
        let query = self.query.trim().to_lowercase();
        let default = matches!(self.target, PickerTarget::Agent(_)).then(|| &*DEFAULT_MODEL);
        let mut rows: Vec<&ModelChoice> = default
            .into_iter()
            .chain(&self.items)
            .filter(|m| {
                m.id.to_lowercase().contains(&query) || m.label.to_lowercase().contains(&query)
            })
            .collect();
        if let Some(exact) = rows.iter().position(|m| m.id.to_lowercase() == query) {
            rows[..=exact].rotate_right(1);
        }
        rows
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
    /// The cursor is on the Agent card above the provider cards.
    pub agent_card_focused: bool,
    /// Where each local agent CLI was found, looked up once when the view
    /// opens; an agent missing here is not installed.
    pub agent_paths: Vec<(AgentKind, PathBuf)>,
}

impl SettingsViewState {
    /// The view on the saved settings, with each local agent CLI looked up
    /// in `agent_search`.
    pub fn new(agent_search: &SearchPaths) -> Self {
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
            agent_card_focused: false,
            agent_paths: AGENTS
                .iter()
                .filter_map(|&kind| Some((kind, find_agent_in(kind, agent_search)?)))
                .collect(),
        }
    }

    /// Open the picker on `target`'s list under `key`: Ready on a `cached`
    /// list, else Loading until [`Self::models_loaded`].
    pub fn open_picker(
        &mut self,
        target: PickerTarget,
        key: String,
        cached: Option<Vec<ModelChoice>>,
    ) {
        let load = if cached.is_some() {
            PickerLoad::Ready
        } else {
            PickerLoad::Loading
        };
        self.picker = Some(ModelPicker {
            target,
            key,
            query: String::new(),
            items: cached.unwrap_or_default(),
            load,
            selected: 0,
        });
    }

    /// A fetched list (or why there is none) for `key`. A picker that is
    /// already Ready keeps its list and highlight.
    pub fn models_loaded(&mut self, key: &str, result: Result<Vec<ModelChoice>, String>) {
        let Some(picker) = self
            .picker
            .as_mut()
            .filter(|p| p.key == key && p.load != PickerLoad::Ready)
        else {
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

    /// Down: the next provider; past the last one, the Agent card; from the
    /// card, the first provider.
    pub fn select_next_provider(&mut self) {
        if self.is_editing {
            return;
        }
        if self.agent_card_focused {
            self.selected_provider_idx = 0;
            self.leave_agent_card();
        } else if self.selected_provider_idx % ALL_PROVIDERS.len() == ALL_PROVIDERS.len() - 1 {
            self.focus_agent_card();
        } else {
            self.selected_provider_idx += 1;
        }
    }

    /// Up: the previous provider; above the first one, the Agent card; from
    /// the card, the last provider.
    pub fn select_prev_provider(&mut self) {
        if self.is_editing {
            return;
        }
        if self.agent_card_focused {
            self.selected_provider_idx = ALL_PROVIDERS.len() - 1;
            self.leave_agent_card();
        } else if self.selected_provider_idx % ALL_PROVIDERS.len() == 0 {
            self.focus_agent_card();
        } else {
            self.selected_provider_idx -= 1;
        }
    }

    /// Model is the one field both kinds of card have, so it stays put as the
    /// cursor crosses; any other field lands on the card's Agent row.
    fn focus_agent_card(&mut self) {
        self.agent_card_focused = true;
        if self.selected_field != SettingField::Model {
            self.selected_field = SettingField::Agent;
        }
    }

    fn leave_agent_card(&mut self) {
        self.agent_card_focused = false;
        if self.selected_field == SettingField::Agent {
            self.selected_field = SettingField::ProviderToggle;
        }
    }

    /// Step the Assistant's agent: srelens, Claude Code, Codex, Cursor, round.
    pub fn cycle_agent(&mut self) -> AgentKind {
        let at = AGENTS.iter().position(|&a| a == self.settings.agent);
        let next = AGENTS[(at.unwrap_or(0) + 1) % AGENTS.len()];
        self.settings.agent = next;
        if !self.agent_card_fields().contains(&self.selected_field) {
            self.selected_field = SettingField::Agent;
        }
        next
    }

    /// The Agent card's fields: the agent and its model, and for Cursor its
    /// own API key.
    fn agent_card_fields(&self) -> &'static [SettingField] {
        if self.settings.agent == AgentKind::Cursor {
            &[
                SettingField::Agent,
                SettingField::Model,
                SettingField::ApiKey,
            ]
        } else {
            &[SettingField::Agent, SettingField::Model]
        }
    }

    /// Where an API key being edited is kept: the focused provider's, or on
    /// the Agent card the active agent's (only Cursor has one).
    fn key_slug(&self) -> &'static str {
        if self.agent_card_focused {
            agent_slug(self.settings.agent)
        } else {
            provider_slug(self.current_provider())
        }
    }

    /// What the field being edited belongs to, for the edit box's title.
    fn editing_for(&self) -> &'static str {
        if self.agent_card_focused {
            self.settings.agent.label()
        } else {
            provider_display_name(self.current_provider())
        }
    }

    /// What the Agent card's Model field picks from: the active local agent's
    /// own list, or the default provider's when srelens answers.
    pub fn agent_picker_target(&self) -> PickerTarget {
        match self.settings.agent {
            AgentKind::Srelens => PickerTarget::Provider(self.settings.default_provider),
            kind => PickerTarget::Agent(kind),
        }
    }

    /// The next of the card's fields (`forward`) or the previous, round.
    fn step_agent_card_field(&mut self, forward: bool) {
        let fields = self.agent_card_fields();
        let n = fields.len();
        let at = fields
            .iter()
            .position(|f| *f == self.selected_field)
            .unwrap_or(0);
        let next = if forward { at + 1 } else { at + n - 1 };
        self.selected_field = fields[next % n];
    }

    pub fn select_next_field(&mut self) {
        if self.is_editing {
            return;
        }
        if self.agent_card_focused {
            return self.step_agent_card_field(true);
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
            SettingField::Timeout | SettingField::Agent => SettingField::ProviderToggle,
        };
    }

    pub fn select_prev_field(&mut self) {
        if self.is_editing {
            return;
        }
        if self.agent_card_focused {
            return self.step_agent_card_field(false);
        }
        let is_custom = self.current_provider() == AiProvider::OpenAiCompatible;
        self.selected_field = match self.selected_field {
            SettingField::ProviderToggle | SettingField::Agent => SettingField::Timeout,
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
        self.edit_buffer = match self.selected_field {
            SettingField::ProviderToggle => {
                self.set_active_provider();
                return;
            }
            // The agent is stepped through, not typed.
            SettingField::Agent => return,
            SettingField::ApiKey => self
                .settings
                .api_keys
                .get(self.key_slug())
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
            SettingField::ProviderToggle | SettingField::Agent => {}
            SettingField::ApiKey => {
                let slug = self.key_slug().to_string();
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

/// The Assistant's agents, in the order Space steps through them.
const AGENTS: [AgentKind; 4] = [
    AgentKind::Srelens,
    AgentKind::Claude,
    AgentKind::Codex,
    AgentKind::Cursor,
];

/// How far a status that did not fit beside its agent is indented below it.
const STATUS_INDENT: &str = "  ";

fn provider_card_rows(provider: AiProvider) -> u16 {
    // The custom endpoint's card has a Base URL line.
    if provider == AiProvider::OpenAiCompatible {
        6
    } else {
        5
    }
}

/// The fewest rows a provider card can have and still say which provider it
/// is: its borders and its name line.
const MIN_PROVIDER_CARD_ROWS: u16 = 3;

/// Where the provider cards go in `area`: in order, each at its natural
/// height, the last one cut short; a card left with fewer than
/// [`MIN_PROVIDER_CARD_ROWS`] is not drawn rather than drawn as a bare frame.
/// When they do not all fit, the list starts at the first card that lets the
/// `selected` one show its name, so the cursor is never off screen.
fn provider_card_areas(area: Rect, selected: Option<usize>) -> Vec<(usize, Rect)> {
    let rows = |i: usize| provider_card_rows(ALL_PROVIDERS[i]);
    let mut first = selected.map_or(0, |s| s % ALL_PROVIDERS.len());
    let mut room = area.height.saturating_sub(MIN_PROVIDER_CARD_ROWS);
    while first > 0 && rows(first - 1) <= room {
        first -= 1;
        room -= rows(first);
    }
    let end = area.y + area.height;
    let mut y = area.y;
    let mut areas = Vec::new();
    for idx in first..ALL_PROVIDERS.len() {
        let height = rows(idx).min(end - y);
        if height < MIN_PROVIDER_CARD_ROWS {
            break;
        }
        areas.push((idx, Rect::new(area.x, y, area.width, height)));
        y += height;
    }
    areas
}

fn label_style(focused: bool) -> Style {
    if focused {
        Style::default()
            .fg(Theme::YELLOW)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Theme::DIM)
    }
}

fn value_style(focused: bool) -> Style {
    Style::default().fg(if focused { Theme::YELLOW } else { Theme::FG })
}

/// The active agent's model; while srelens answers, the default provider's.
fn agent_model(state: &SettingsViewState) -> String {
    match state.settings.agent {
        AgentKind::Srelens => {
            let provider = state.settings.default_provider;
            format!(
                "{} · {}",
                provider_display_name(provider),
                state.settings.get_model(provider)
            )
        }
        kind => state
            .settings
            .get_agent_model(kind)
            .unwrap_or_else(|| "Default".to_string()),
    }
}

/// Whether `kind`'s CLI is installed (`folded`: just the word, else the path)
/// or where to get it; an active agent that is missing is an error. srelens's
/// own agent is built in and has no CLI.
fn agent_status(state: &SettingsViewState, kind: AgentKind, folded: bool) -> Option<Span<'static>> {
    kind.binary()?;
    let found = state.agent_paths.iter().find(|(k, _)| *k == kind);
    Some(match found {
        Some(_) if folded => Span::styled("installed", Style::default().fg(Theme::DIM)),
        Some((_, path)) => Span::styled(
            format!("installed: {}", path.display()),
            Style::default().fg(Theme::GREEN),
        ),
        None => Span::styled(
            format!("not installed — {}", agent_install_url(kind)),
            if kind == state.settings.agent {
                Theme::status_error()
            } else {
                Style::default().fg(Theme::DIM)
            },
        ),
    })
}

/// `head` and `status` on one line (after `sep`) when `width` allows;
/// otherwise the status alone on the line below, never clipped beside it.
fn with_status(
    head: Vec<Span<'static>>,
    sep: &'static str,
    status: Option<Span<'static>>,
    width: usize,
) -> Vec<Line<'static>> {
    let mut line = Line::from(head);
    let Some(status) = status else {
        return vec![line];
    };
    let sep = Span::styled(sep, Style::default().fg(Theme::DIM));
    if line.width() + sep.width() + status.width() <= width {
        line.spans.push(sep);
        line.spans.push(status);
        vec![line]
    } else {
        vec![line, Line::from(vec![Span::raw(STATUS_INDENT), status])]
    }
}

/// An API key as a settings card shows it: masked when stored, the variable
/// it comes from when set in the environment, else `unset`.
fn key_display(stored: Option<&String>, env_var: &str, unset: Span<'static>) -> Span<'static> {
    if let Some(k) = stored.filter(|s| !s.trim().is_empty()) {
        let masked = if k.len() > 8 {
            format!("{}••••••••{}", &k[..4], &k[k.len() - 4..])
        } else {
            "••••••••".to_string()
        };
        Span::styled(
            format!("{} [stored in config]", masked),
            Style::default().fg(Theme::GREEN),
        )
    } else if std::env::var(env_var).is_ok() {
        Span::styled(
            format!("[env: {} set]", env_var),
            Style::default().fg(Theme::CYAN),
        )
    } else {
        unset
    }
}

/// The full card: a row per agent with whether its CLI is installed, then
/// the active agent's model, and Cursor's own API key.
fn full_agent_lines(state: &SettingsViewState, width: usize) -> Vec<Line<'static>> {
    let on_card = state.agent_card_focused;
    let agent_focus = on_card && state.selected_field == SettingField::Agent;
    let model_focus = on_card && state.selected_field == SettingField::Model;
    let key_focus = on_card && state.selected_field == SettingField::ApiKey;
    let mut lines: Vec<Line<'static>> = AGENTS
        .iter()
        .flat_map(|&kind| {
            let active = kind == state.settings.agent;
            let radio = if active {
                Span::styled(
                    "● ",
                    Style::default()
                        .fg(Theme::GREEN)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled("○ ", Style::default().fg(Theme::DIM))
            };
            let name_style = if active {
                value_style(agent_focus).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Theme::FG)
            };
            let head = vec![
                radio,
                Span::styled(format!("{:<13}", kind.label()), name_style),
            ];
            with_status(head, "", agent_status(state, kind, false), width)
        })
        .collect();
    lines.push(Line::from(vec![
        Span::styled("   Model:   ", label_style(model_focus)),
        Span::styled(agent_model(state), value_style(model_focus)),
    ]));
    if state.settings.agent == AgentKind::Cursor {
        let unset = Span::styled(
            "auto (uses logged-in cursor auth or CURSOR_API_KEY)",
            Style::default().fg(Theme::DIM),
        );
        lines.push(Line::from(vec![
            Span::styled("   API Key: ", label_style(key_focus)),
            key_display(
                state.settings.api_keys.get(agent_slug(AgentKind::Cursor)),
                "CURSOR_API_KEY",
                unset,
            ),
        ]));
    }
    lines
}

/// The card folded: the active agent and its model on one line, with whether
/// its CLI is installed.
fn folded_agent_lines(state: &SettingsViewState, width: usize) -> Vec<Line<'static>> {
    let agent = state.settings.agent;
    let head = vec![
        Span::styled("Agent: ", label_style(false)),
        Span::styled(
            format!("● {}", agent.label()),
            value_style(false).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" · ", Style::default().fg(Theme::DIM)),
        Span::styled(agent_model(state), value_style(false)),
    ];
    with_status(head, " · ", agent_status(state, agent, true), width)
}

/// The Agent card: which agent answers the Assistant, around `lines`.
fn render_agent_card(f: &mut Frame, area: Rect, lines: Vec<Line<'static>>, focused: bool) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if focused { Theme::CYAN } else { Theme::BORDER }))
        .title(Span::styled(" Assistant agent ", Theme::header_label()));
    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(Paragraph::new(lines), inner);
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

    // The Agent card is drawn in full while it has the cursor, or when every
    // provider card still fits below it. Otherwise it folds into one line (two
    // when its CLI's status does not fit beside it) and the providers get the
    // rows that are left.
    let card_width = usize::from(inner.width.saturating_sub(2));
    let full_lines = full_agent_lines(state, card_width);
    let providers_rows: u16 = ALL_PROVIDERS.iter().map(|&p| provider_card_rows(p)).sum();
    let full_rows = full_lines.len() as u16 + 2;
    let agent_lines =
        if state.agent_card_focused || inner.height >= 3 + full_rows + providers_rows + 2 {
            full_lines
        } else {
            folded_agent_lines(state, card_width)
        };
    let agent_rows = agent_lines.len() as u16 + 2;

    // The description, the Agent card and the hints keep their rows; the
    // provider list takes whatever is left, however little.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),          // Top description & instructions
            Constraint::Length(agent_rows), // Agent card
            Constraint::Min(0),             // Provider list
            Constraint::Length(2),          // Bottom key hints
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

    // 2. Agent card
    render_agent_card(f, chunks[1], agent_lines, state.agent_card_focused);

    // 3. Providers List
    let selected = (!state.agent_card_focused).then_some(state.selected_provider_idx);
    for (idx, provider_area) in provider_card_areas(chunks[2], selected) {
        let provider = ALL_PROVIDERS[idx];
        let is_selected_provider = selected == Some(idx);
        let is_active_provider = provider == state.settings.default_provider;

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

        let unset = if provider == AiProvider::OpenAiCompatible {
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
        let key_value_display = key_display(
            state.settings.api_keys.get(provider_slug(provider)),
            env_var_for_provider(provider),
            unset,
        );

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

    // 4. Bottom Key Hints
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
        chunks[3],
    );

    // 5. Modal Edit Input Dialog (when editing)
    if state.is_editing {
        let edit_area = crate::ui::help::centered_rect(60, 25, area);
        f.render_widget(Clear, edit_area);

        let field_name = match state.selected_field {
            SettingField::ApiKey => "API Key",
            SettingField::Model => "Model ID",
            SettingField::BaseUrl => "Base URL (e.g. http://localhost:11434/v1)",
            SettingField::Timeout => "Turn Timeout (seconds, e.g. 120)",
            SettingField::ProviderToggle => "Provider",
            SettingField::Agent => "Agent",
        };

        let edit_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Theme::YELLOW))
            .title(format!(" Edit {} for {} ", field_name, state.editing_for()));

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
            agent_card_focused: false,
            agent_paths: Vec::new(),
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
    fn a_typed_id_listed_exactly_comes_first_ahead_of_longer_matches() {
        for (items, query) in [
            (["llama3.2:3b", "llama3.2"], "llama3.2"),
            (["gpt-4o-audio-preview", "gpt-4o"], "GPT-4o"),
        ] {
            let mut p = picker(PickerTarget::Provider(AiProvider::OpenAiCompatible));
            p.items = items.iter().map(|id| choice(id, "")).collect();
            p.query = query.into();
            assert_eq!(
                p.choose(),
                Ok(Some(query.to_lowercase())),
                "{query} among {items:?}"
            );
            assert_eq!(ids(&p)[1], items[0], "the rest keep their order");
        }
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
    fn a_list_or_its_failure_lands_on_its_own_picker_only() {
        let mut s = state();
        let target = PickerTarget::Provider(AiProvider::OpenAi);
        s.open_picker(target, "k".into(), None);
        assert_eq!(s.picker.as_ref().unwrap().load, PickerLoad::Loading);

        s.models_loaded("other", Ok(vec![choice("m-0", "Zero")]));
        assert_eq!(s.picker.as_ref().unwrap().load, PickerLoad::Loading);

        s.models_loaded("k", Err("offline".into()));
        assert_eq!(
            s.picker.as_ref().unwrap().load,
            PickerLoad::Failed("offline".into())
        );

        s.models_loaded("k", Ok(vec![choice("m-1", "One")]));
        let p = s.picker.as_ref().unwrap();
        assert_eq!((&p.load, ids(p)), (&PickerLoad::Ready, vec!["m-1"]));

        s.open_picker(target, "k".into(), Some(vec![choice("m-1", "One")]));
        assert_eq!(s.picker.as_ref().unwrap().load, PickerLoad::Ready);
    }

    #[test]
    fn a_late_result_does_not_disturb_a_ready_list() {
        let mut s = state();
        s.open_picker(PickerTarget::Provider(AiProvider::OpenAi), "k".into(), None);
        s.models_loaded("k", Ok(vec![choice("m-1", "One"), choice("m-2", "Two")]));
        s.picker.as_mut().unwrap().move_down();

        s.models_loaded("k", Ok(vec![choice("m-3", "Three")]));
        let p = s.picker.as_ref().unwrap();
        assert_eq!(p.choose(), Ok(Some("m-2".to_string())), "highlight kept");

        s.models_loaded("k", Err("boom".into()));
        let p = s.picker.as_ref().unwrap();
        assert_eq!((&p.load, ids(p)), (&PickerLoad::Ready, vec!["m-1", "m-2"]));
    }

    #[test]
    fn picking_default_for_an_agent_clears_its_model() {
        let mut s = state();
        s.settings
            .set_agent_model(AgentKind::Codex, Some("gpt-5.6-sol".into()));
        s.open_picker(PickerTarget::Agent(AgentKind::Codex), "a".into(), None);
        assert_eq!(s.pick_model(), Ok(None));
        assert_eq!(s.settings.get_agent_model(AgentKind::Codex), None);
        assert!(s.picker.is_none());
    }

    #[test]
    fn a_refused_id_keeps_the_picker_open_and_the_model_unchanged() {
        let mut s = state();
        s.open_picker(PickerTarget::Provider(AiProvider::OpenAi), "p".into(), None);
        s.picker.as_mut().unwrap().query = "-x".into();
        assert!(s.pick_model().is_err());
        assert!(s.picker.is_some());
        assert_eq!(s.settings.get_model(AiProvider::OpenAi), "gpt-4o");

        s.picker.as_mut().unwrap().query = "gpt-x".into();
        assert_eq!(s.pick_model(), Ok(Some("gpt-x".to_string())));
        assert_eq!(s.settings.get_model(AiProvider::OpenAi), "gpt-x");
    }
}
