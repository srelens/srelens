//! Journeys through the AI settings screen (`:settings`, #1000): a list of
//! providers, then a list of agent CLIs, a row each, with the row under the
//! cursor showing its fields.
use crossterm::event::KeyCode;
use srectl::ai_config::AiProvider;
use srectl::app::{ActiveView, App};
use srectl::views::settings_view::{PickerLoad, PickerTarget};
use srectl::views::{SettingField, SettingsViewState};
use srelens_agent::adapter::{AgentKind, ModelChoice};

use crate::common::{self, ch, key, press, type_str};

fn settings(app: &App) -> &SettingsViewState {
    match &app.active_view {
        ActiveView::Settings(s) => s,
        _ => panic!("expected the settings view"),
    }
}

/// `:settings` typed into the command bar, as a person opens it.
async fn open_settings(app: &mut App) {
    press(app, ch(':')).await;
    type_str(app, "settings").await;
    press(app, key(KeyCode::Enter)).await;
    assert!(matches!(app.active_view, ActiveView::Settings(_)));
}

fn saved_agent() -> AgentKind {
    let saved = std::fs::read_to_string(srectl::AiSettings::config_path()).expect("saved");
    srectl::AiSettings::from_json(&saved).unwrap().agent
}

/// D1: Input Disorder & Interleaving.
/// A person arrows into the agents, tabs to a field, overshoots back up into
/// the providers, comes down again and presses Space twice. A second Space
/// on the same row keeps that agent; it does not step on to the next one, as
/// the old agent card did.
#[tokio::test]
async fn journey_settings_messy_arrows_and_double_space_keep_the_agent_under_the_cursor() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    open_settings(&mut app).await;

    for _ in 0..4 {
        press(&mut app, ch('j')).await;
    }
    assert_eq!(settings(&app).focused_agent(), AgentKind::Srelens);
    press(&mut app, key(KeyCode::Tab)).await;
    assert_eq!(settings(&app).selected_field, SettingField::Model);

    // Overshoot up into the last provider: it has a Model field too.
    press(&mut app, key(KeyCode::Up)).await;
    assert!(!settings(&app).agents_focused);
    assert_eq!(settings(&app).selected_field, SettingField::Model);
    press(&mut app, key(KeyCode::Down)).await;
    press(&mut app, key(KeyCode::Down)).await;
    assert_eq!(settings(&app).focused_agent(), AgentKind::Claude);

    press(&mut app, ch(' ')).await;
    press(&mut app, ch(' ')).await;
    assert_eq!(app.ai_settings.agent, AgentKind::Claude);
    assert_eq!(saved_agent(), AgentKind::Claude);

    // Tab on, back twice, and Enter twice on the row itself: still Claude.
    press(&mut app, key(KeyCode::Tab)).await;
    assert_eq!(settings(&app).selected_field, SettingField::Timeout);
    press(&mut app, key(KeyCode::BackTab)).await;
    press(&mut app, key(KeyCode::BackTab)).await;
    assert_eq!(settings(&app).selected_field, SettingField::Agent);
    press(&mut app, key(KeyCode::Enter)).await;
    press(&mut app, key(KeyCode::Enter)).await;
    assert_eq!(app.ai_settings.agent, AgentKind::Claude);
    assert!(!settings(&app).is_editing, "no edit box for an agent");
}

/// D2: Ambiguity & Collisions.
/// "Anthropic (Claude)" is a provider and "Claude Code" an agent, and Space
/// means "use this" on both lists. Each Space changes its own list only.
#[tokio::test]
async fn journey_settings_space_on_a_provider_and_on_an_agent_each_change_their_own_list() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    open_settings(&mut app).await;

    press(&mut app, ch('j')).await;
    press(&mut app, ch(' ')).await;
    assert_eq!(app.ai_settings.default_provider, AiProvider::OpenAi);
    assert_eq!(app.ai_settings.agent, AgentKind::Srelens);

    for _ in 0..4 {
        press(&mut app, ch('j')).await;
    }
    assert_eq!(settings(&app).focused_agent(), AgentKind::Claude);
    press(&mut app, ch(' ')).await;
    assert_eq!(app.ai_settings.agent, AgentKind::Claude);
    assert_eq!(app.ai_settings.default_provider, AiProvider::OpenAi);

    let screen = common::render_app(&mut app, 160, 40);
    assert!(screen.contains("● OpenAI (GPT-4o)  [DEFAULT]"), "{screen}");
    assert!(screen.contains("○ Anthropic (Claude)"), "{screen}");
    assert!(screen.contains("● Claude Code"), "{screen}");
    assert!(screen.contains("○ srelens (built-in)"), "{screen}");
    assert_eq!(screen.matches("[DEFAULT]").count(), 1, "{screen}");
}

/// D3: Temporal & Asynchrony.
/// Claude Code's model list is slow. The person gives up on it, moves down
/// to Codex and opens Codex's list; Claude's then arrives. It must not land
/// in Codex's picker, which waits for its own.
#[tokio::test]
async fn journey_settings_a_late_model_list_does_not_land_on_the_next_agents_picker() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    open_settings(&mut app).await;

    // Down to Claude Code's Model field and open its list.
    for _ in 0..5 {
        press(&mut app, ch('j')).await;
    }
    press(&mut app, key(KeyCode::Tab)).await;
    press(&mut app, key(KeyCode::Enter)).await;
    let claude_key = settings(&app)
        .picker
        .as_ref()
        .expect("a picker")
        .key
        .clone();
    press(&mut app, key(KeyCode::Esc)).await;

    press(&mut app, ch('j')).await;
    press(&mut app, key(KeyCode::Enter)).await;
    let picker = settings(&app).picker.as_ref().expect("Codex's picker");
    assert_eq!(picker.target, PickerTarget::Agent(AgentKind::Codex));
    let codex_key = picker.key.clone();

    let model = |id: &str| ModelChoice {
        id: id.to_string(),
        label: id.to_string(),
    };
    app.handle_models_loaded(&claude_key, Ok(vec![model("claude-opus")]));
    let picker = settings(&app)
        .picker
        .as_ref()
        .expect("still Codex's picker");
    assert_eq!(picker.target, PickerTarget::Agent(AgentKind::Codex));
    assert!(picker.items.is_empty(), "{:?}", picker.items);

    app.handle_models_loaded(&codex_key, Ok(vec![model("gpt-5.6-sol")]));
    let picker = settings(&app).picker.as_ref().expect("Codex's picker");
    assert_eq!(picker.load, PickerLoad::Ready);
    assert_eq!(picker.items, vec![model("gpt-5.6-sol")]);
}

/// D4: Boundary & Scale Extremes.
/// A model id as long as one may be (128 characters) on an 80x24 terminal:
/// the row stays one line, its status is cut with "…", and the expanded
/// fields still start on the next line.
#[tokio::test]
async fn journey_settings_a_128_character_model_id_keeps_its_row_on_one_line_at_80x24() {
    let mut env = common::env::isolate_settings();
    env.remove_env("OPENAI_COMPATIBLE_API_KEY");
    let (mut app, _rx) = common::app().await;
    open_settings(&mut app).await;

    let long_id = format!("acme/{}", "m".repeat(123));
    assert_eq!(long_id.len(), 128);
    for _ in 0..3 {
        press(&mut app, ch('j')).await;
    }
    press(&mut app, key(KeyCode::Tab)).await;
    press(&mut app, key(KeyCode::Tab)).await;
    press(&mut app, key(KeyCode::Enter)).await;
    type_str(&mut app, &long_id).await;
    press(&mut app, key(KeyCode::Enter)).await;
    assert_eq!(
        app.ai_settings.get_model(AiProvider::OpenAiCompatible),
        long_id
    );

    let screen = common::render_app(&mut app, 80, 24);
    let lines: Vec<&str> = screen.lines().collect();
    let row = lines
        .iter()
        .position(|l| l.contains("OpenAI-Compatible / Ollama (Local)"))
        .unwrap_or_else(|| panic!("{screen}"));
    assert!(lines[row].contains("key optional · acme/mmm"), "{screen}");
    assert!(lines[row].contains('…'), "{screen}");
    assert!(lines[row + 1].contains("API Key:"), "{screen}");
    // The field showing the id in full is cut short too, not wrapped.
    assert!(lines[row + 2].contains("Model:   acme/mmm"), "{screen}");
    assert!(lines[row + 2].contains('…'), "{screen}");
    assert!(lines[row + 3].contains("Base URL:"), "{screen}");
    assert!(
        lines.iter().all(|l| l.chars().count() <= 80),
        "nothing spills past the terminal: {screen}"
    );
}

/// D5: State Transitions & Re-Entry.
/// The person picks Codex, leaves the screen and comes back. The choice was
/// saved and still shows; the cursor starts afresh on the default provider,
/// not on the agent row it was left on.
#[tokio::test]
async fn journey_settings_leaving_and_reopening_keeps_the_agent_and_resets_the_cursor() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    open_settings(&mut app).await;

    for _ in 0..6 {
        press(&mut app, ch('j')).await;
    }
    assert_eq!(settings(&app).focused_agent(), AgentKind::Codex);
    press(&mut app, ch(' ')).await;
    press(&mut app, key(KeyCode::Esc)).await;
    assert!(!matches!(app.active_view, ActiveView::Settings(_)));

    open_settings(&mut app).await;
    let s = settings(&app);
    assert_eq!(s.settings.agent, AgentKind::Codex);
    assert!(!s.agents_focused);
    assert_eq!(s.current_provider(), s.settings.default_provider);
    assert_eq!(saved_agent(), AgentKind::Codex);
    let screen = common::render_app(&mut app, 160, 40);
    assert!(screen.contains("● Codex"), "{screen}");
    assert!(screen.contains("○ srelens (built-in)"), "{screen}");
}

/// D6: Feedback & Truthfulness.
/// When the settings file cannot be written, choosing an agent says so; it
/// never shows the "✓ set" toast as if the choice had been kept.
#[tokio::test]
async fn journey_settings_an_unwritable_settings_file_is_reported_not_hidden() {
    let mut env = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    open_settings(&mut app).await;
    // A directory where the file should be: every write fails.
    let dir = tempfile::tempdir().unwrap();
    env.set_env("SRELENS_AI_SETTINGS_PATH", dir.path());

    press(&mut app, ch('k')).await;
    assert_eq!(settings(&app).focused_agent(), AgentKind::Cursor);
    press(&mut app, ch(' ')).await;
    let screen = common::render_app(&mut app, 160, 40);
    assert!(screen.contains("Failed to save settings"), "{screen}");
    assert!(!screen.contains("Assistant agent set to"), "{screen}");
}

/// D7: Visual & Perceptual Reality.
/// On a wide 200x50 screen both lists show whole, with the header and the
/// hints intact; on 80x24 the last agent's row and fields stay in sight.
#[tokio::test]
async fn journey_settings_both_lists_read_whole_at_200x50_and_scroll_at_80x24() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    open_settings(&mut app).await;

    let wide = common::render_app(&mut app, 200, 50);
    for row in [
        "● Anthropic (Claude)  [DEFAULT]",
        "○ OpenAI (GPT-4o)",
        "○ Google Gemini",
        "○ OpenAI-Compatible / Ollama (Local)",
        "● srelens (built-in)",
        "○ Claude Code",
        "○ Codex",
        "○ Cursor",
        "SRElens AI & Assistant Settings",
        "[Esc/q] Back",
    ] {
        assert!(wide.contains(row), "{row}: {wide}");
    }

    press(&mut app, ch('k')).await;
    let narrow = common::render_app(&mut app, 80, 24);
    assert!(narrow.contains("○ Cursor"), "{narrow}");
    assert!(
        narrow.contains("Install: https://docs.cursor.com"),
        "{narrow}"
    );
    assert!(narrow.contains("[Esc/q] Back"), "{narrow}");
}
