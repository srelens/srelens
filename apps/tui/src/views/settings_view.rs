use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use std::ops::Range;
use std::path::PathBuf;
use std::sync::LazyLock;

use srelens_agent::adapter::{valid_model_id, AgentKind, ModelChoice};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

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
    /// The cursor is on the Agent CLIs list below the providers, on
    /// [`SettingsViewState::focused_agent`]'s row.
    pub agents_focused: bool,
    /// Index into the agent rows: srelens, Claude Code, Codex, Cursor.
    pub selected_agent_idx: usize,
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
            agents_focused: false,
            selected_agent_idx: 0,
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

    /// The agent whose row is (or was last) under the cursor.
    pub fn focused_agent(&self) -> AgentKind {
        AGENTS[self.selected_agent_idx % AGENTS.len()]
    }

    /// Down: the next row, the providers then the agents, round.
    pub fn select_next_row(&mut self) {
        self.step_row(true);
    }

    /// Up: the previous row, round.
    pub fn select_prev_row(&mut self) {
        self.step_row(false);
    }

    /// One row down (`forward`) or up. A field the new row also has stays
    /// focused; one it lacks gives way to the row itself.
    fn step_row(&mut self, forward: bool) {
        if self.is_editing {
            return;
        }
        let providers = ALL_PROVIDERS.len();
        let rows = providers + AGENTS.len();
        let at = if self.agents_focused {
            providers + self.selected_agent_idx % AGENTS.len()
        } else {
            self.selected_provider_idx % providers
        };
        let next = (if forward { at + 1 } else { at + rows - 1 }) % rows;
        self.agents_focused = next >= providers;
        if self.agents_focused {
            self.selected_agent_idx = next - providers;
        } else {
            self.selected_provider_idx = next;
        }
        let fields = self.row_fields();
        if !fields.contains(&self.selected_field) {
            self.selected_field = fields[0];
        }
    }

    /// The focused row's fields, the row itself first.
    fn row_fields(&self) -> &'static [SettingField] {
        if self.agents_focused {
            return agent_fields(self.focused_agent());
        }
        if self.current_provider() == AiProvider::OpenAiCompatible {
            &[
                SettingField::ProviderToggle,
                SettingField::ApiKey,
                SettingField::Model,
                SettingField::BaseUrl,
                SettingField::Timeout,
            ]
        } else {
            &[
                SettingField::ProviderToggle,
                SettingField::ApiKey,
                SettingField::Model,
                SettingField::Timeout,
            ]
        }
    }

    /// Make the agent under the cursor the one the Assistant runs.
    pub fn use_focused_agent(&mut self) -> AgentKind {
        self.settings.agent = self.focused_agent();
        self.settings.agent
    }

    /// Where an API key being edited is kept: the focused provider's, or on
    /// an agent's row that agent's (only Cursor has one).
    fn key_slug(&self) -> &'static str {
        if self.agents_focused {
            agent_slug(self.focused_agent())
        } else {
            provider_slug(self.current_provider())
        }
    }

    /// What the field being edited belongs to, for the edit box's title.
    fn editing_for(&self) -> &'static str {
        if self.agents_focused {
            self.focused_agent().label()
        } else {
            provider_display_name(self.current_provider())
        }
    }

    /// What an agent row's Model field picks from: that local agent's own
    /// list, or on srelens's row the default provider's.
    pub fn agent_picker_target(&self) -> PickerTarget {
        match self.focused_agent() {
            AgentKind::Srelens => PickerTarget::Provider(self.settings.default_provider),
            kind => PickerTarget::Agent(kind),
        }
    }

    /// The next of the row's fields (`forward`) or the previous, round.
    fn step_field(&mut self, forward: bool) {
        if self.is_editing {
            return;
        }
        let fields = self.row_fields();
        let n = fields.len();
        let at = fields
            .iter()
            .position(|f| *f == self.selected_field)
            .unwrap_or(0);
        let next = if forward { at + 1 } else { at + n - 1 };
        self.selected_field = fields[next % n];
    }

    pub fn select_next_field(&mut self) {
        self.step_field(true);
    }

    pub fn select_prev_field(&mut self) {
        self.step_field(false);
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
            // The agent is picked, not typed.
            SettingField::Agent => return,
            SettingField::ApiKey => self
                .settings
                .api_keys
                .get(self.key_slug())
                .cloned()
                .unwrap_or_default(),
            SettingField::Model => self.settings.get_model(provider),
            SettingField::BaseUrl => self.settings.get_base_url(provider),
            SettingField::Timeout if self.agents_focused => self
                .settings
                .get_agent_timeout_seconds(self.focused_agent())
                .to_string(),
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
                    let secs = secs.clamp(5, 3600);
                    if self.agents_focused {
                        self.settings
                            .set_agent_timeout_seconds(self.focused_agent(), secs);
                    } else {
                        self.settings.set_timeout_seconds(provider, secs);
                    }
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

/// The Assistant's agents, in the order their rows are listed.
const AGENTS: [AgentKind; 4] = [
    AgentKind::Srelens,
    AgentKind::Claude,
    AgentKind::Codex,
    AgentKind::Cursor,
];

/// An agent row's fields: the agent and its model, and for a local agent its
/// turn timeout (after Cursor's own API key). srelens's own agent runs under
/// its provider's timeout.
fn agent_fields(kind: AgentKind) -> &'static [SettingField] {
    match kind {
        AgentKind::Srelens => &[SettingField::Agent, SettingField::Model],
        AgentKind::Cursor => &[
            SettingField::Agent,
            SettingField::Model,
            SettingField::ApiKey,
            SettingField::Timeout,
        ],
        AgentKind::Claude | AgentKind::Codex => &[
            SettingField::Agent,
            SettingField::Model,
            SettingField::Timeout,
        ],
    }
}

const PROVIDERS_NOTE: &str = "srelens's own agent talks directly to a provider with your API key.";
const AGENTS_NOTE: &str =
    "External coding-agent CLIs the Assistant can run instead of srelens's own agent.";

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

/// `kind`'s model; for srelens, the default provider's.
fn agent_model(state: &SettingsViewState, kind: AgentKind) -> String {
    match kind {
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

/// Where a key comes from; a blank stored key counts as none.
enum KeySource<'a> {
    Stored(&'a str),
    Env,
    Unset,
}

fn key_source<'a>(stored: Option<&'a String>, env_var: &str) -> KeySource<'a> {
    match stored.filter(|s| !s.trim().is_empty()) {
        Some(k) => KeySource::Stored(k),
        None if std::env::var(env_var).is_ok() => KeySource::Env,
        None => KeySource::Unset,
    }
}

/// An API key as a field shows it: masked when stored, the variable it comes
/// from when set in the environment, else `unset`.
fn key_display(stored: Option<&String>, env_var: &str, unset: Span<'static>) -> Span<'static> {
    match key_source(stored, env_var) {
        KeySource::Stored(k) => {
            let masked = if k.len() > 8 {
                format!("{}••••••••{}", &k[..4], &k[k.len() - 4..])
            } else {
                "••••••••".to_string()
            };
            Span::styled(
                format!("{} [stored in config]", masked),
                Style::default().fg(Theme::GREEN),
            )
        }
        KeySource::Env => Span::styled(
            format!("[env: {} set]", env_var),
            Style::default().fg(Theme::CYAN),
        ),
        KeySource::Unset => unset,
    }
}

/// What a provider's row says at its right edge: where its key comes from
/// and the model it uses, or that it has none.
fn provider_status(state: &SettingsViewState, provider: AiProvider) -> Span<'static> {
    let model = state.settings.get_model(provider);
    let env_var = env_var_for_provider(provider);
    let stored = state.settings.api_keys.get(provider_slug(provider));
    let (text, color) = match key_source(stored, env_var) {
        KeySource::Stored(_) => (format!("key set · {model}"), Theme::GREEN),
        KeySource::Env => (format!("env {env_var} · {model}"), Theme::CYAN),
        KeySource::Unset if provider == AiProvider::OpenAiCompatible => {
            (format!("key optional · {model}"), Theme::DIM)
        }
        KeySource::Unset => ("no key".to_string(), Theme::DIM),
    };
    Span::styled(text, Style::default().fg(color))
}

/// What an agent's row says at its right edge: for srelens the provider it
/// answers through; for a CLI whether it is installed, with its model, or
/// where to get it (an error for the active agent).
fn agent_status(state: &SettingsViewState, kind: AgentKind) -> Span<'static> {
    let dim = Style::default().fg(Theme::DIM);
    if kind == AgentKind::Srelens {
        return Span::styled(agent_model(state, kind), dim);
    }
    if state.agent_paths.iter().any(|(k, _)| *k == kind) {
        return Span::styled(
            format!("installed · {}", agent_model(state, kind)),
            Style::default().fg(Theme::GREEN),
        );
    }
    let style = if kind == state.settings.agent {
        Theme::status_error()
    } else {
        dim
    };
    Span::styled(
        format!("not installed — {}", agent_install_url(kind)),
        style,
    )
}

/// A row's radio and name: filled when `chosen`; the name is yellow while
/// the cursor is on the row itself rather than one of its fields.
fn row_head(name: &str, chosen: bool, on_row: bool) -> Vec<Span<'static>> {
    let radio = if chosen {
        Span::styled(
            "● ",
            Style::default()
                .fg(Theme::GREEN)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled("○ ", Style::default().fg(Theme::DIM))
    };
    let name_style = if on_row {
        Style::default()
            .fg(Theme::YELLOW)
            .add_modifier(Modifier::BOLD)
    } else if chosen {
        Style::default().fg(Theme::FG).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Theme::FG)
    };
    vec![radio, Span::styled(name.to_string(), name_style)]
}

/// The longest start of `s` that fits in `max` columns.
fn take(s: &str, max: usize) -> String {
    let mut used = 0;
    s.chars()
        .take_while(|c| {
            used += c.width().unwrap_or(0);
            used <= max
        })
        .collect()
}

/// `s` in `max` columns, ending in "…" when it had to be cut.
fn cut(s: &str, max: usize) -> String {
    if s.width() <= max {
        s.to_string()
    } else if max == 0 {
        String::new()
    } else {
        format!("{}…", take(s, max - 1))
    }
}

/// `head` at the left of a `width`-wide row and `status` at its right edge,
/// two columns apart at least; the status is what gets cut short.
fn list_row(mut head: Vec<Span<'static>>, status: Span<'static>, width: usize) -> Line<'static> {
    let head_width = Line::from(head.clone()).width();
    let text = cut(&status.content, width.saturating_sub(head_width + 2));
    head.push(Span::raw(
        " ".repeat(width.saturating_sub(head_width + text.width())),
    ));
    head.push(Span::styled(text, status.style));
    Line::from(head)
}

/// `line` padded to `width` columns, or cut to them with "…".
fn fit(mut line: Line<'static>, width: usize) -> Line<'static> {
    if line.width() > width {
        let mut left = width.saturating_sub(1);
        let mut spans = Vec::new();
        for span in line.spans {
            if span.width() <= left {
                left -= span.width();
                spans.push(span);
            } else {
                spans.push(Span::styled(
                    format!("{}…", take(&span.content, left)),
                    span.style,
                ));
                break;
            }
        }
        line = Line::from(spans);
    }
    let pad = width.saturating_sub(line.width());
    line.spans.push(Span::raw(" ".repeat(pad)));
    line
}

/// A `width`-wide frame with `title` in its top border round `lines`, each
/// with a column of space either side; cyan while the cursor is in it.
fn framed(
    title: &str,
    lines: Vec<Line<'static>>,
    width: usize,
    focused: bool,
) -> Vec<Line<'static>> {
    let border = Style::default().fg(if focused { Theme::CYAN } else { Theme::BORDER });
    let rule = |n: usize| "─".repeat(n);
    let mut out = vec![Line::from(vec![
        Span::styled("┌ ", border),
        Span::styled(title.to_string(), Theme::header_label()),
        Span::styled(
            format!(" {}┐", rule(width.saturating_sub(title.width() + 4))),
            border,
        ),
    ])];
    out.extend(lines.into_iter().map(|line| {
        let mut spans = vec![Span::styled("│ ", border)];
        spans.extend(fit(line, width.saturating_sub(4)).spans);
        spans.push(Span::styled(" │", border));
        Line::from(spans)
    }));
    out.push(Line::from(Span::styled(
        format!("└{}┘", rule(width.saturating_sub(2))),
        border,
    )));
    out
}

/// The fields under the provider row the cursor is on, a line each in Tab's
/// order, so a value cut short never hides the field after it.
fn provider_field_lines(state: &SettingsViewState, provider: AiProvider) -> Vec<Line<'static>> {
    let focus = |field| state.selected_field == field;
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
    let mut lines = vec![
        Line::from(vec![
            Span::styled("   API Key: ", label_style(focus(SettingField::ApiKey))),
            key_display(
                state.settings.api_keys.get(provider_slug(provider)),
                env_var_for_provider(provider),
                unset,
            ),
        ]),
        Line::from(vec![
            Span::styled("   Model:   ", label_style(focus(SettingField::Model))),
            Span::styled(
                state.settings.get_model(provider),
                value_style(focus(SettingField::Model)),
            ),
        ]),
    ];
    if provider == AiProvider::OpenAiCompatible {
        lines.push(Line::from(vec![
            Span::styled("   Base URL: ", label_style(focus(SettingField::BaseUrl))),
            Span::styled(
                state.settings.get_base_url(provider),
                value_style(focus(SettingField::BaseUrl)),
            ),
        ]));
    }
    lines.push(Line::from(vec![
        Span::styled("   Timeout: ", label_style(focus(SettingField::Timeout))),
        Span::styled(
            format!("{}s", state.settings.get_timeout_seconds(provider)),
            value_style(focus(SettingField::Timeout)),
        ),
    ]));
    lines
}

/// The fields under the agent row the cursor is on, then where its CLI was
/// found or where to get it.
fn agent_field_lines(state: &SettingsViewState, kind: AgentKind) -> Vec<Line<'static>> {
    let focus = |field| state.selected_field == field;
    let dim = Style::default().fg(Theme::DIM);
    let mut lines = vec![Line::from(vec![
        Span::styled("   Model:   ", label_style(focus(SettingField::Model))),
        Span::styled(
            agent_model(state, kind),
            value_style(focus(SettingField::Model)),
        ),
    ])];
    if kind == AgentKind::Cursor {
        let unset = Span::styled("auto (uses logged-in cursor auth or CURSOR_API_KEY)", dim);
        lines.push(Line::from(vec![
            Span::styled("   API Key: ", label_style(focus(SettingField::ApiKey))),
            key_display(
                state.settings.api_keys.get(agent_slug(kind)),
                "CURSOR_API_KEY",
                unset,
            ),
        ]));
    }
    if kind == AgentKind::Srelens {
        return lines;
    }
    lines.push(Line::from(vec![
        Span::styled("   Timeout: ", label_style(focus(SettingField::Timeout))),
        Span::styled(
            format!("{}s", state.settings.get_agent_timeout_seconds(kind)),
            value_style(focus(SettingField::Timeout)),
        ),
    ]));
    lines.push(match state.agent_paths.iter().find(|(k, _)| *k == kind) {
        Some((_, path)) => Line::from(vec![
            Span::styled("   Path:    ", label_style(false)),
            Span::styled(path.display().to_string(), dim),
        ]),
        None => Line::from(vec![
            Span::styled("   Install: ", label_style(false)),
            Span::styled(agent_install_url(kind), dim),
        ]),
    });
    lines
}

/// Both lists, `width` wide: a row per provider, then a row per agent, and
/// under the row the cursor is on its fields. Also says which lines those
/// are, for the view to keep in sight.
fn list_lines(state: &SettingsViewState, width: usize) -> (Vec<Line<'static>>, Range<usize>) {
    let row_width = width.saturating_sub(4);
    let dim = Style::default().fg(Theme::DIM);
    let mut focus = 0..0;

    let mut providers = vec![Line::from(Span::styled(PROVIDERS_NOTE, dim))];
    for (idx, &provider) in ALL_PROVIDERS.iter().enumerate() {
        let on_row =
            !state.agents_focused && state.selected_provider_idx % ALL_PROVIDERS.len() == idx;
        let is_default = provider == state.settings.default_provider;
        let mut head = row_head(
            provider_display_name(provider),
            is_default,
            on_row && state.selected_field == SettingField::ProviderToggle,
        );
        if is_default {
            head.push(Span::raw("  "));
            head.push(Span::styled(
                "[DEFAULT]",
                Style::default()
                    .fg(Theme::GREEN)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        providers.push(list_row(head, provider_status(state, provider), row_width));
        if on_row {
            // + 1 for the frame's top border.
            let row = providers.len();
            providers.extend(provider_field_lines(state, provider));
            focus = row..providers.len() + 1;
        }
    }
    let mut lines = framed("Providers", providers, width, !state.agents_focused);

    let mut agents = vec![Line::from(Span::styled(AGENTS_NOTE, dim))];
    for (idx, &kind) in AGENTS.iter().enumerate() {
        let on_row = state.agents_focused && state.selected_agent_idx % AGENTS.len() == idx;
        let name = if kind == AgentKind::Srelens {
            "srelens (built-in)"
        } else {
            kind.label()
        };
        let head = row_head(
            name,
            kind == state.settings.agent,
            on_row && state.selected_field == SettingField::Agent,
        );
        agents.push(list_row(head, agent_status(state, kind), row_width));
        if on_row {
            let row = lines.len() + agents.len();
            agents.extend(agent_field_lines(state, kind));
            focus = row..lines.len() + agents.len() + 1;
        }
    }
    lines.extend(framed("Agent CLIs", agents, width, state.agents_focused));
    (lines, focus)
}

/// Key hints, each its keys in their colour and what they do.
fn hint_line(hints: &[(&'static str, Color, &'static str)]) -> Line<'static> {
    let mut spans = Vec::new();
    for (i, &(keys, color, does)) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(
            keys,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(format!(" {does}"), Theme::header_label()));
    }
    Line::from(spans)
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
            Constraint::Min(0),    // The two lists
            Constraint::Length(2), // Bottom key hints
        ])
        .split(inner);

    // The lists scroll, when they do not fit, just far enough to show the
    // row under the cursor and its fields.
    let (lines, focus) = list_lines(state, usize::from(chunks[0].width));
    let scroll = focus
        .end
        .saturating_sub(usize::from(chunks[0].height))
        .min(focus.start);
    f.render_widget(
        Paragraph::new(lines).scroll((u16::try_from(scroll).unwrap_or(u16::MAX), 0)),
        chunks[0],
    );

    // The key hints on one line, or split in two between hints, never inside
    // one, when the view is too narrow for them.
    let hints = [
        ("[↑/↓/j/k]", Theme::YELLOW, "Row"),
        ("[Tab/←/→]", Theme::YELLOW, "Field"),
        ("[Space]", Theme::GREEN, "Use"),
        ("[e/Enter]", Theme::CYAN, "Edit"),
        ("[s]", Theme::ACCENT, "Save"),
        ("[Esc/q]", Theme::YELLOW, "Back"),
    ];
    let one_line = hint_line(&hints);
    let hint_lines = if one_line.width() <= usize::from(chunks[1].width) {
        vec![one_line]
    } else {
        vec![hint_line(&hints[..3]), hint_line(&hints[3..])]
    };
    f.render_widget(
        Paragraph::new(hint_lines).alignment(Alignment::Center),
        chunks[1],
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
            agents_focused: false,
            selected_agent_idx: 0,
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
