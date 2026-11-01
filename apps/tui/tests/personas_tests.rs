//! Prebuilt assistant personas: who they are, how they shape a turn, and what
//! a read-only one is refused.

mod common;

use srectl::ai_config::AiSettings;
use srectl::ai_personas::{self, Persona};

#[test]
fn the_three_requested_personas_exist_with_stable_ids() {
    let ids: Vec<&str> = ai_personas::all().iter().map(|p| p.id).collect();
    assert_eq!(ids, vec!["platform", "security", "network"]);
    let names: Vec<&str> = ai_personas::all().iter().map(|p| p.name).collect();
    assert_eq!(
        names,
        vec![
            "Platform Engineer",
            "Kubernetes Security Engineer",
            "K8s Network Engineer"
        ]
    );
}

#[test]
fn a_persona_is_found_by_id_name_or_a_loose_spelling() {
    assert_eq!(ai_personas::find("security").unwrap().id, "security");
    assert_eq!(ai_personas::find("SECURITY").unwrap().id, "security");
    assert_eq!(ai_personas::find("Platform Engineer").unwrap().id, "platform");
    assert_eq!(ai_personas::find("k8s-network-engineer").unwrap().id, "network");
    assert!(ai_personas::find("wizard").is_none());
    assert!(ai_personas::find("").is_none());
}

#[test]
fn every_persona_is_complete_and_names_no_machine_detail() {
    for p in ai_personas::all() {
        assert!(!p.name.is_empty() && !p.tagline.is_empty(), "{}", p.id);
        assert!(p.instructions.len() > 200, "{} is too thin", p.id);
        // Same rule as the base prompt: nothing about the local machine.
        for banned in ["/home", "/Users", "C:\\"] {
            assert!(!p.instructions.contains(banned), "{} mentions {banned}", p.id);
        }
        assert!(
            p.instructions.contains("live"),
            "{} must tell the model to read live state",
            p.id
        );
    }
}

#[test]
fn security_and_network_are_read_only_and_platform_is_not() {
    assert!(ai_personas::find("security").unwrap().read_only);
    assert!(ai_personas::find("network").unwrap().read_only);
    assert!(!ai_personas::find("platform").unwrap().read_only);
}

#[test]
fn a_read_only_persona_says_so_to_the_model() {
    let p = ai_personas::find("security").unwrap();
    assert!(p.instructions.to_lowercase().contains("read-only"));
}

#[test]
fn applying_a_persona_puts_its_role_before_the_question() {
    let p: &Persona = ai_personas::find("network").unwrap();
    let out = p.apply("why is DNS slow?");
    assert!(out.starts_with("[ROLE: K8s Network Engineer]"), "{out}");
    assert!(out.ends_with("why is DNS slow?"), "the question is last and untouched");
}

#[test]
fn no_persona_leaves_the_question_alone() {
    assert_eq!(ai_personas::apply_to(None, "hello"), "hello");
    assert!(ai_personas::apply_to(Some("security"), "hello").ends_with("hello"));
    assert_eq!(
        ai_personas::apply_to(Some("no-such-persona"), "hello"),
        "hello",
        "an unknown id in settings must not break the Assistant"
    );
}

#[test]
fn the_chosen_persona_is_saved_in_ai_settings_and_old_files_still_load() {
    let _settings = common::env::isolate_settings();
    let mut s = AiSettings::default();
    assert_eq!(s.persona, None);
    s.persona = Some("security".into());
    let json = serde_json::to_string(&s).unwrap();
    let back: AiSettings = serde_json::from_str(&json).unwrap();
    assert_eq!(back.persona.as_deref(), Some("security"));

    // A settings file written before personas existed has no such key.
    let mut v: serde_json::Value = serde_json::to_value(AiSettings::default()).unwrap();
    v.as_object_mut().unwrap().remove("persona");
    let old: AiSettings = serde_json::from_value(v).unwrap();
    assert_eq!(old.persona, None);
}

// ---------------------------------------------------------------------------
// Read-only enforcement
// ---------------------------------------------------------------------------

use serde_json::json;
use srectl::consent::ReadOnly;
use srelens_capability::Impact;
use srelens_mcp::policy::{ConfirmPolicy, ConsentKind, ConsentRequest, Decision};

struct ApproveAll;

#[async_trait::async_trait]
impl ConfirmPolicy for ApproveAll {
    async fn confirm(&self, _: &ConsentRequest) -> Decision {
        Decision::Approved
    }
}

fn request(kind: ConsentKind) -> ConsentRequest {
    ConsentRequest {
        tool: "k8s.deleteObject".into(),
        args: json!({}),
        kind,
        impact: Impact::default(),
        confirm_text: None,
        caller: None,
    }
}

#[tokio::test]
async fn a_read_only_persona_has_its_writes_refused_even_if_the_user_would_approve() {
    let policy = ReadOnly::new(ApproveAll);
    match policy.confirm(&request(ConsentKind::Destructive)).await {
        Decision::Denied(why) => {
            assert!(why.contains("read-only"), "{why}");
            assert!(why.contains("k8s.deleteObject"), "names the tool: {why}");
        }
        other => panic!("a write must be refused, got {other:?}"),
    }
}

#[tokio::test]
async fn a_read_only_persona_still_asks_about_sensitive_reads() {
    let policy = ReadOnly::new(ApproveAll);
    assert_eq!(
        policy.confirm(&request(ConsentKind::SensitiveRead)).await,
        Decision::Approved,
        "reading a Secret is not a change; the inner policy decides"
    );
}

// ---------------------------------------------------------------------------
// What the agent is sent
// ---------------------------------------------------------------------------

use srectl::ai_skills::CavemanLevel;

#[test]
fn the_turn_query_is_persona_then_question_then_style() {
    let q = ai_personas::compose_turn_query(Some("security"), Some(CavemanLevel::Lite), "audit rbac");
    let role = q.find("[ROLE: Kubernetes Security Engineer]").expect("role");
    let question = q.find("audit rbac").expect("question");
    let style = q.find("CAVEMAN MODE").expect("style");
    assert!(role < question && question < style, "{q}");
}

#[test]
fn the_turn_query_is_unchanged_without_a_persona_or_style() {
    assert_eq!(ai_personas::compose_turn_query(None, None, "hi"), "hi");
}

#[test]
fn a_persona_that_is_read_only_is_found_from_settings() {
    assert!(ai_personas::is_read_only(Some("security")));
    assert!(ai_personas::is_read_only(Some("Network")));
    assert!(!ai_personas::is_read_only(Some("platform")));
    assert!(!ai_personas::is_read_only(None));
    assert!(!ai_personas::is_read_only(Some("unknown")));
}

// ---------------------------------------------------------------------------
// :persona, the picker and the title
// ---------------------------------------------------------------------------

use crossterm::event::KeyCode;
use srectl::commands::{resolve_command_with_crds, CommandTarget};
use srectl::ui::Modal;

#[test]
fn persona_commands_resolve() {
    let r = |s: &str| resolve_command_with_crds(s, &[]);
    assert_eq!(r("persona"), Some(CommandTarget::Persona));
    assert_eq!(r("personas"), Some(CommandTarget::Persona));
    assert_eq!(r("persona security"), Some(CommandTarget::SetPersona("security".into())));
    assert_eq!(r("role network"), Some(CommandTarget::SetPersona("network".into())));
}

#[tokio::test]
async fn setting_a_persona_by_name_is_saved_and_announced() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    app.execute_command_target(CommandTarget::SetPersona("Kubernetes Security Engineer".into()))
        .await;
    assert_eq!(app.ai_settings.persona.as_deref(), Some("security"), "stored as the stable id");
    let (msg, _, _) = app.toast.clone().expect("a toast");
    assert!(msg.contains("Kubernetes Security Engineer") && msg.contains("read-only"), "{msg}");
}

#[tokio::test]
async fn off_none_and_clear_return_to_the_plain_assistant() {
    for word in ["off", "none", "clear", "default"] {
        let _settings = common::env::isolate_settings();
        let (mut app, _rx) = common::app().await;
        app.ai_settings.persona = Some("network".into());
        app.execute_command_target(CommandTarget::SetPersona(word.into())).await;
        assert_eq!(app.ai_settings.persona, None, "{word}");
    }
}

#[tokio::test]
async fn an_unknown_persona_changes_nothing_and_lists_the_choices() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    app.ai_settings.persona = Some("platform".into());
    app.execute_command_target(CommandTarget::SetPersona("wizard".into())).await;
    assert_eq!(app.ai_settings.persona.as_deref(), Some("platform"));
    let (msg, _, _) = app.toast.clone().unwrap();
    assert!(msg.contains("wizard") && msg.contains("security"), "{msg}");
}

#[tokio::test]
async fn the_picker_lists_every_persona_and_marks_the_active_one() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    app.ai_settings.persona = Some("network".into());
    app.execute_command_target(CommandTarget::Persona).await;
    assert!(matches!(app.modal, Some(Modal::PersonaPicker { .. })));
    let screen = common::render_app(&mut app, 140, 40);
    for name in ["Platform Engineer", "Kubernetes Security Engineer", "K8s Network Engineer"] {
        assert!(screen.contains(name), "{name} missing:\n{screen}");
    }
    assert!(screen.contains("read-only"), "read-only personas are labelled:\n{screen}");
    assert!(screen.contains("(active)"), "the current one is marked:\n{screen}");
}

#[tokio::test]
async fn choosing_in_the_picker_sets_the_persona() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    app.execute_command_target(CommandTarget::Persona).await;
    // Rows: 0 = plain assistant, then the personas in order.
    common::press(&mut app, common::key(KeyCode::Down)).await;
    common::press(&mut app, common::key(KeyCode::Down)).await;
    common::press(&mut app, common::key(KeyCode::Enter)).await;
    assert!(app.modal.is_none());
    assert_eq!(app.ai_settings.persona.as_deref(), Some("security"));
}

#[tokio::test]
async fn the_first_row_of_the_picker_clears_the_persona_and_esc_cancels() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    app.ai_settings.persona = Some("platform".into());

    app.execute_command_target(CommandTarget::Persona).await;
    common::press(&mut app, common::key(KeyCode::Esc)).await;
    assert_eq!(app.ai_settings.persona.as_deref(), Some("platform"), "Esc changes nothing");

    app.execute_command_target(CommandTarget::Persona).await;
    // The picker opens on the active persona (row 1); go up to the plain row.
    common::press(&mut app, common::key(KeyCode::Up)).await;
    common::press(&mut app, common::key(KeyCode::Enter)).await;
    assert_eq!(app.ai_settings.persona, None);
}

#[test]
fn the_assistant_title_names_the_active_persona() {
    use srectl::views::assistant_view::{render_assistant_view, AssistantViewState};
    let state = AssistantViewState::for_context("test-cluster");
    let mut settings = AiSettings::default();
    settings.persona = Some("security".into());
    let screen = common::render_text(180, 12, |f| {
        render_assistant_view(f, f.area(), &state, &settings)
    });
    assert!(screen.contains("Kubernetes Security Engineer"), "{screen}");

    settings.persona = None;
    let plain = common::render_text(180, 12, |f| {
        render_assistant_view(f, f.area(), &state, &settings)
    });
    assert!(!plain.contains("Security Engineer"), "{plain}");
}

// ---------------------------------------------------------------------------
// End to end: what a real turn hands the agent
// ---------------------------------------------------------------------------

#[cfg(unix)]
async fn args_a_fake_claude_receives(persona: Option<&str>, question: &str) -> String {
    use srelens_agent::adapter::AgentKind;
    use std::os::unix::fs::PermissionsExt;

    let _settings = common::env::isolate_settings();
    let dir = tempfile::tempdir().unwrap();
    let record = dir.path().join("argv.txt");
    let script = dir.path().join("claude");
    std::fs::write(
        &script,
        format!("#!/bin/sh\nfor a in \"$@\"; do printf '%s\\n' \"$a\"; done > '{}'\n", record.display()),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

    let (mut app, _rx) = common::app().await;
    app.agent_search = common::agents_only_in(dir.path());
    app.ai_settings.agent = AgentKind::Claude;
    app.ai_settings.persona = persona.map(String::from);
    app.assistant_state.caveman_level = None;
    app.submit_assistant_query(question.to_string(), question.to_string());

    for _ in 0..100 {
        if let Ok(text) = std::fs::read_to_string(&record) {
            if !text.is_empty() {
                return text;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("the fake agent was never run");
}

#[cfg(unix)]
#[tokio::test]
async fn a_turn_with_a_persona_hands_the_agent_its_role_and_the_question() {
    let argv = args_a_fake_claude_receives(Some("security"), "audit rbac in prod").await;
    assert!(argv.contains("[ROLE: Kubernetes Security Engineer]"), "{argv}");
    assert!(argv.contains("audit rbac in prod"), "{argv}");
    assert!(argv.contains("read-only"), "{argv}");
}

#[cfg(unix)]
#[tokio::test]
async fn a_turn_without_a_persona_is_not_given_a_role() {
    let argv = args_a_fake_claude_receives(None, "why is my pod pending").await;
    assert!(argv.contains("why is my pod pending"), "{argv}");
    assert!(!argv.contains("[ROLE:"), "{argv}");
}

// ---------------------------------------------------------------------------
// The security and network tools the personas know about
// ---------------------------------------------------------------------------

fn text(id: &str) -> String {
    ai_personas::find(id).unwrap().instructions.to_lowercase()
}

#[test]
fn the_security_persona_knows_kube_bench_and_kubearmor() {
    let t = text("security");
    for needle in ["kube-bench", "kubearmor", "cis"] {
        assert!(t.contains(needle), "security should mention {needle}");
    }
    assert!(!t.contains("kubeshark"), "network tooling belongs to the network persona");
}

#[test]
fn the_network_persona_knows_kubeshark() {
    let t = text("network");
    assert!(t.contains("kubeshark"));
    assert!(!t.contains("kube-bench") && !t.contains("kubearmor"));
}

#[test]
fn the_platform_persona_is_not_given_the_security_or_network_tools() {
    let t = text("platform");
    for needle in ["kube-bench", "kubearmor", "kubeshark"] {
        assert!(!t.contains(needle), "{needle}");
    }
}

#[test]
fn personas_are_told_to_check_a_tool_is_installed_before_relying_on_it() {
    for id in ["security", "network"] {
        let t = text(id);
        assert!(t.contains("installed"), "{id}: check for the tool first");
        assert!(
            t.contains("do not claim") || t.contains("never claim") || t.contains("do not assume"),
            "{id}: must not claim results from a tool that is not there"
        );
    }
}

#[test]
fn the_read_only_personas_hand_installs_to_the_user_rather_than_doing_them() {
    for id in ["security", "network"] {
        let t = text(id);
        assert!(
            t.contains("install") && t.contains("for the user to run"),
            "{id} should give the install command for the user to run"
        );
    }
}

#[test]
fn the_security_persona_knows_where_each_tool_keeps_its_data() {
    let t = text("security");
    // KubeArmor's policies are custom resources in its own API group.
    assert!(t.contains("security.kubearmor.com"));
    // kube-bench's report is a Job's pod log.
    assert!(t.contains("job") && t.contains("log"));
}

// ---------------------------------------------------------------------------
// Scan tools: the one kind of write a read-only persona may ask to make
// ---------------------------------------------------------------------------

fn request_for(tool: &str, kind: ConsentKind) -> ConsentRequest {
    ConsentRequest { tool: tool.into(), ..request(kind) }
}

#[tokio::test]
async fn a_named_scan_tool_gets_through_a_read_only_persona_to_the_users_own_prompt() {
    let policy = ReadOnly::new(ApproveAll).allowing(&["k8s.runKubeBench"]);
    assert_eq!(
        policy.confirm(&request_for("k8s.runKubeBench", ConsentKind::Destructive)).await,
        Decision::Approved,
        "the inner policy (the user's prompt) decides"
    );
}

#[tokio::test]
async fn the_exception_is_for_that_tool_only() {
    let policy = ReadOnly::new(ApproveAll).allowing(&["k8s.runKubeBench"]);
    for tool in ["k8s.deleteObject", "k8s.applyManifest", "k8s.runKubeBench2", "k8s.runKubeBenc"] {
        assert!(
            matches!(policy.confirm(&request_for(tool, ConsentKind::Destructive)).await, Decision::Denied(_)),
            "{tool} must stay refused"
        );
    }
}

#[tokio::test]
async fn an_exception_is_still_refused_when_the_inner_policy_refuses() {
    struct DenyAll;
    #[async_trait::async_trait]
    impl ConfirmPolicy for DenyAll {
        async fn confirm(&self, _: &ConsentRequest) -> Decision {
            Decision::Denied("user said no".into())
        }
    }
    let policy = ReadOnly::new(DenyAll).allowing(&["k8s.runKubeBench"]);
    assert_eq!(
        policy.confirm(&request_for("k8s.runKubeBench", ConsentKind::Destructive)).await,
        Decision::Denied("user said no".into())
    );
}

#[test]
fn only_the_security_persona_may_run_kube_bench() {
    assert_eq!(ai_personas::find("security").unwrap().scan_tools, &["k8s.runKubeBench"]);
    assert!(ai_personas::find("network").unwrap().scan_tools.is_empty());
    assert!(ai_personas::find("platform").unwrap().scan_tools.is_empty());
}

#[test]
fn what_a_read_only_turn_may_still_run_comes_from_the_persona() {
    assert_eq!(ai_personas::scan_tools_for(Some("security")), &["k8s.runKubeBench"]);
    assert!(ai_personas::scan_tools_for(Some("network")).is_empty());
    assert!(ai_personas::scan_tools_for(None).is_empty());
    assert!(ai_personas::scan_tools_for(Some("unknown")).is_empty());
}

#[test]
fn the_security_persona_is_told_to_use_the_tool_and_that_it_asks_first() {
    let t = text("security");
    assert!(t.contains("k8s.runkubebench"), "names the tool");
    assert!(t.contains("asks the user") || t.contains("user confirms") || t.contains("asks the user to confirm"));
}

#[test]
fn a_scan_tool_is_never_listed_for_a_persona_that_is_not_read_only() {
    for p in ai_personas::all() {
        if !p.read_only {
            assert!(p.scan_tools.is_empty(), "{}: the exception only means something for a read-only persona", p.id);
        }
    }
}
