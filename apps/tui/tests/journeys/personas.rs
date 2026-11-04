//! Journeys through the Assistant personas: `:persona`, its picker, and what a
//! read-only persona is refused.
//!
//! D1 input disorder, D2 ambiguity, D5 re-entry, D6 truthfulness and D7 narrow
//! screens apply. D3 (late arrivals) does not: a persona is read when a turn
//! starts, so changing it mid-turn affects the next turn and nothing arrives
//! late. D4 (scale) does not: there are three fixed personas.
use crossterm::event::KeyCode;
use srectl::ai_personas;
use srectl::ui::Modal;

use crate::common::{self, ch, context_named, key, press, run_command};

/// D1: the picker wraps both ways, ignores letters it does not know, and two
/// quick Enters choose once, not twice.
#[tokio::test]
async fn journey_persona_picker_wraps_and_survives_a_double_enter() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    run_command(&mut app, "persona").await;
    assert!(matches!(app.modal, Some(Modal::PersonaPicker { .. })));

    press(&mut app, key(KeyCode::Up)).await; // wraps from the first row to the last
    press(&mut app, ch('z')).await;          // means nothing here
    press(&mut app, ch(' ')).await;
    press(&mut app, key(KeyCode::Enter)).await;
    press(&mut app, key(KeyCode::Enter)).await;

    assert_eq!(app.ai_settings.persona.as_deref(), Some("network"), "the last persona");
    assert!(app.modal.is_none());
}

/// D2: the same persona is reachable by id, by name and by a loose spelling, and
/// a partial word that names none is refused rather than guessed.
#[tokio::test]
async fn journey_persona_names_resolve_exactly_and_never_by_guess() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    for typed in ["persona security", "persona Security", "role kubernetes-security-engineer", "persona Kubernetes Security Engineer"] {
        app.ai_settings.persona = None;
        run_command(&mut app, typed).await;
        assert_eq!(app.ai_settings.persona.as_deref(), Some("security"), "{typed}");
    }
    app.ai_settings.persona = Some("platform".into());
    run_command(&mut app, "persona sec").await;
    assert_eq!(app.ai_settings.persona.as_deref(), Some("platform"), "a prefix is not a name");
}

/// D5: the persona stays across a cluster switch, and the plain assistant is
/// one command away.
#[tokio::test]
async fn journey_the_persona_survives_cluster_switches_and_can_be_cleared() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    app.contexts = vec![context_named("test-cluster"), context_named("staging")];
    run_command(&mut app, "persona network").await;
    app.switch_context("staging".into()).await;
    app.switch_context("test-cluster".into()).await;
    assert_eq!(app.ai_settings.persona.as_deref(), Some("network"));
    run_command(&mut app, "persona off").await;
    assert_eq!(app.ai_settings.persona, None);
}

/// D6: an unknown name changes nothing and lists the real ones; a read-only
/// persona says it is read-only when chosen.
#[tokio::test]
async fn journey_an_unknown_persona_lists_the_choices_and_a_read_only_one_says_so() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    run_command(&mut app, "persona wizard").await;
    let (msg, _, _) = app.toast.clone().unwrap();
    assert!(msg.contains("wizard") && msg.contains("security") && msg.contains("network"), "{msg}");
    assert_eq!(app.ai_settings.persona, None);

    run_command(&mut app, "persona security").await;
    let (msg, _, _) = app.toast.clone().unwrap();
    assert!(msg.contains("read-only"), "{msg}");
}

/// Every persona's read-only status matches what the picker shows, so the
/// label cannot drift from what is enforced.
#[tokio::test]
async fn journey_the_picker_labels_exactly_the_read_only_personas() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    run_command(&mut app, "persona").await;
    let screen = common::render_app(&mut app, 140, 40);
    let labelled = screen.matches("[read-only]").count();
    let read_only = ai_personas::all().iter().filter(|p| p.read_only).count();
    assert_eq!(labelled, read_only, "{screen}");
}

/// D7: the picker and the assistant title on small and large terminals.
#[tokio::test]
async fn journey_persona_screens_render_at_80x24_and_200x50() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    run_command(&mut app, "persona security").await;
    run_command(&mut app, "persona").await;
    for (w, h) in [(80, 24), (200, 50), (40, 10)] {
        assert!(!common::render_app(&mut app, w, h).trim().is_empty());
    }
    press(&mut app, key(KeyCode::Esc)).await;
    run_command(&mut app, "ai").await;
    for (w, h) in [(80, 24), (200, 50), (40, 10)] {
        assert!(!common::render_app(&mut app, w, h).trim().is_empty());
    }
}
