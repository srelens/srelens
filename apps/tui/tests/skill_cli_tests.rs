use clap::Parser;
use srectl::ai_skills::{
    delete_skill, expand_slash_command_from, fallback_name_from_url, list_all_skills, list_skills,
    load_all_playbooks_from, load_skill_or_builtin, match_slash_commands_from, parse_skill,
    parse_skill_content, read_skill, resolve_fetch_url, to_markdown, validate_name,
    write_skill_atomic, Skill,
};
use srectl::cli::{Cli, CliCommand, SkillSubcommand};
use std::path::PathBuf;
use tempfile::tempdir;

#[test]
fn cli_parses_skill_subcommands_and_flags() {
    // 1. Top-level `srectl skill` (no subcommand)
    let cli = Cli::try_parse_from(["srectl", "skill"]).expect("parse skill");
    assert_eq!(cli.command, Some(CliCommand::Skill { subcommand: None }));

    // 2. `srectl skill list`
    let cli = Cli::try_parse_from(["srectl", "skill", "list"]).expect("parse skill list");
    assert_eq!(
        cli.command,
        Some(CliCommand::Skill {
            subcommand: Some(SkillSubcommand::List { json: false }),
        })
    );

    // 3. `srectl skill list --json`
    let cli = Cli::try_parse_from(["srectl", "skill", "list", "--json"])
        .expect("parse skill list --json");
    assert_eq!(
        cli.command,
        Some(CliCommand::Skill {
            subcommand: Some(SkillSubcommand::List { json: true }),
        })
    );

    // 4. `srectl skill show my-skill`
    let cli =
        Cli::try_parse_from(["srectl", "skill", "show", "my-skill"]).expect("parse skill show");
    assert_eq!(
        cli.command,
        Some(CliCommand::Skill {
            subcommand: Some(SkillSubcommand::Show {
                name: "my-skill".to_string(),
                raw: false,
            }),
        })
    );

    // 5. `srectl skill show my-skill --raw`
    let cli = Cli::try_parse_from(["srectl", "skill", "show", "my-skill", "--raw"])
        .expect("parse skill show --raw");
    assert_eq!(
        cli.command,
        Some(CliCommand::Skill {
            subcommand: Some(SkillSubcommand::Show {
                name: "my-skill".to_string(),
                raw: true,
            }),
        })
    );

    // 6. `srectl skill add --file runbook.md`
    let cli = Cli::try_parse_from(["srectl", "skill", "add", "--file", "runbook.md"])
        .expect("parse skill add --file");
    assert_eq!(
        cli.command,
        Some(CliCommand::Skill {
            subcommand: Some(SkillSubcommand::Add {
                name: None,
                file: Some(PathBuf::from("runbook.md")),
                url: None,
                desc: None,
            }),
        })
    );

    // 7. `srectl skill add my-name --file runbook.md --desc "My runbook"`
    let cli = Cli::try_parse_from([
        "srectl",
        "skill",
        "add",
        "my-name",
        "--file",
        "runbook.md",
        "--desc",
        "My runbook",
    ])
    .expect("parse skill add with override");
    assert_eq!(
        cli.command,
        Some(CliCommand::Skill {
            subcommand: Some(SkillSubcommand::Add {
                name: Some("my-name".to_string()),
                file: Some(PathBuf::from("runbook.md")),
                url: None,
                desc: Some("My runbook".to_string()),
            }),
        })
    );

    // 8. `srectl skill add --url https://github.com/owner/repo/blob/main/skills/triage.md`
    let cli = Cli::try_parse_from([
        "srectl",
        "skill",
        "add",
        "--url",
        "https://github.com/owner/repo/blob/main/skills/triage.md",
    ])
    .expect("parse skill add --url");
    assert_eq!(
        cli.command,
        Some(CliCommand::Skill {
            subcommand: Some(SkillSubcommand::Add {
                name: None,
                file: None,
                url: Some("https://github.com/owner/repo/blob/main/skills/triage.md".to_string()),
                desc: None,
            }),
        })
    );

    // 9. `srectl skill delete my-skill`
    let cli =
        Cli::try_parse_from(["srectl", "skill", "delete", "my-skill"]).expect("parse skill delete");
    assert_eq!(
        cli.command,
        Some(CliCommand::Skill {
            subcommand: Some(SkillSubcommand::Delete {
                name: "my-skill".to_string(),
            }),
        })
    );

    // 10. `srectl skill path`
    let cli = Cli::try_parse_from(["srectl", "skill", "path"]).expect("parse skill path");
    assert_eq!(
        cli.command,
        Some(CliCommand::Skill {
            subcommand: Some(SkillSubcommand::Path),
        })
    );
}

#[test]
fn skill_name_validation_rules() {
    assert!(validate_name("crashloop-triage").is_ok());
    assert!(validate_name("my_custom_skill").is_ok());
    assert!(validate_name("v1.2.3").is_ok());
    assert!(validate_name("Skill123").is_ok());

    assert!(validate_name("").is_err());
    assert!(validate_name("a/b").is_err());
    assert!(validate_name("a\\b").is_err());
    assert!(validate_name("../evil").is_err());
    assert!(validate_name("skill name with spaces").is_err());
    assert!(validate_name("foo;rm").is_err());
}

#[test]
fn frontmatter_parsing_and_fallbacks() {
    // 1. Full frontmatter
    let raw =
        "---\nname: db-triage\ndescription: Triage DB connections\n---\nCheck connection pool.";
    let skill = parse_skill_content(raw, None, None).unwrap();
    assert_eq!(skill.name, "db-triage");
    assert_eq!(skill.description, "Triage DB connections");
    assert_eq!(skill.body, "Check connection pool.");

    // Roundtrip markdown serialization
    let md = to_markdown(&skill);
    let parsed_back = parse_skill(&md).unwrap();
    assert_eq!(parsed_back, skill);

    // 2. Frontmatter with missing name, fallback provided
    let raw_no_name = "---\ndescription: Fallback desc\n---\nBody here.";
    let skill = parse_skill_content(raw_no_name, Some("derived-name"), None).unwrap();
    assert_eq!(skill.name, "derived-name");
    assert_eq!(skill.description, "Fallback desc");

    // 3. Plain markdown without frontmatter
    let plain = "# Guide\n1. Run kubectl get pods\n2. Inspect logs";
    let skill = parse_skill_content(plain, Some("plain-guide"), Some("CLI description")).unwrap();
    assert_eq!(skill.name, "plain-guide");
    assert_eq!(skill.description, "CLI description");
    assert_eq!(skill.body, plain);

    // 4. Missing name everywhere errors out
    let err = parse_skill_content(plain, None, None).unwrap_err();
    assert!(err.contains("skill name is required"));

    // 5. CRLF line endings are normalized
    let crlf = "---\r\nname: crlf-skill\r\ndescription: CRLF test\r\n---\r\nLine 1\r\nLine 2";
    let skill = parse_skill_content(crlf, None, None).unwrap();
    assert_eq!(skill.name, "crlf-skill");
    assert_eq!(skill.description, "CRLF test");
    assert_eq!(skill.body, "Line 1\nLine 2");
}

#[test]
fn url_resolution_and_fallback_names() {
    // GitHub blob URL -> raw.githubusercontent.com
    let blob = "https://github.com/srelens/srelens/blob/main/skills/triage.md";
    let raw = resolve_fetch_url(blob);
    assert_eq!(
        raw,
        "https://raw.githubusercontent.com/srelens/srelens/main/skills/triage.md"
    );

    // Query string preserved
    let blob_query = "https://github.com/srelens/srelens/blob/v1.0.0/skills/triage.md?raw=true";
    let raw_query = resolve_fetch_url(blob_query);
    assert_eq!(
        raw_query,
        "https://raw.githubusercontent.com/srelens/srelens/v1.0.0/skills/triage.md?raw=true"
    );

    // Already raw URL remains untouched
    let already_raw = "https://raw.githubusercontent.com/srelens/srelens/main/skills/triage.md";
    assert_eq!(resolve_fetch_url(already_raw), already_raw);

    // Other external URLs untouched
    let external = "https://gitlab.com/sre/skills/blob/main/triage.md";
    assert_eq!(resolve_fetch_url(external), external);

    // Fallback names from URLs
    assert_eq!(
        fallback_name_from_url("https://github.com/srelens/srelens/blob/main/skills/triage.md"),
        Some("triage".to_string())
    );
    assert_eq!(
        fallback_name_from_url(
            "https://raw.githubusercontent.com/srelens/srelens/main/skills/pod-health.md?token=123"
        ),
        Some("pod-health".to_string())
    );
    assert_eq!(
        fallback_name_from_url("https://example.com/custom_playbook"),
        Some("custom_playbook".to_string())
    );
}

#[test]
fn disk_storage_atomicity_and_lifecycle() {
    let temp = tempdir().unwrap();
    let dir = temp.path();

    let skill = Skill {
        name: "custom-worker".to_string(),
        description: "Diagnose worker nodes".to_string(),
        body: "Check kubelet logs and systemd journal.".to_string(),
    };

    // 1. Atomic write
    let file_path = write_skill_atomic(dir, &skill).unwrap();
    assert!(file_path.exists());
    assert_eq!(file_path.file_name().unwrap(), "custom-worker.md");

    // 2. Read back
    let read_back = read_skill(dir, "custom-worker").unwrap();
    assert_eq!(read_back, skill);

    // 3. List skills (user only)
    let user_skills = list_skills(dir).unwrap();
    assert_eq!(user_skills.len(), 1);
    assert_eq!(user_skills[0].name, "custom-worker");
    assert_eq!(user_skills[0].description, "Diagnose worker nodes");
    assert!(!user_skills[0].builtin);
    assert!(!user_skills[0].overrides_builtin);

    // 4. List all skills (built-ins + user skills)
    let all_skills = list_all_skills(dir).unwrap();
    assert!(all_skills.len() > 1);
    let found = all_skills
        .iter()
        .find(|s| s.name == "custom-worker")
        .unwrap();
    assert!(!found.builtin);
    assert!(!found.overrides_builtin);

    // 5. Delete skill
    let deleted = delete_skill(dir, "custom-worker").unwrap();
    assert!(deleted);
    assert!(!file_path.exists());

    // 6. Delete already removed skill is idempotent
    let second_delete = delete_skill(dir, "custom-worker").unwrap();
    assert!(!second_delete);
}

#[test]
fn builtin_override_and_restoration() {
    let temp = tempdir().unwrap();
    let dir = temp.path();

    // Built-in before override
    let initial = load_skill_or_builtin(dir, "crashloop-triage").unwrap();
    assert!(initial.body.contains("When a pod keeps restarting"));

    let all_initial = list_all_skills(dir).unwrap();
    let crash_meta = all_initial
        .iter()
        .find(|s| s.name == "crashloop-triage")
        .unwrap();
    assert!(crash_meta.builtin);
    assert!(!crash_meta.overrides_builtin);

    // User overrides crashloop-triage
    let override_skill = Skill {
        name: "crashloop-triage".to_string(),
        description: "Company customized crashloop triage".to_string(),
        body: "1. Run internal diagnostic curl.\n2. Page on-call team.".to_string(),
    };
    write_skill_atomic(dir, &override_skill).unwrap();

    // After override
    let loaded = load_skill_or_builtin(dir, "crashloop-triage").unwrap();
    assert_eq!(loaded.description, "Company customized crashloop triage");
    assert_eq!(loaded.body, override_skill.body);

    let all_overridden = list_all_skills(dir).unwrap();
    let crash_meta_overridden = all_overridden
        .iter()
        .find(|s| s.name == "crashloop-triage")
        .unwrap();
    assert!(!crash_meta_overridden.builtin);
    assert!(crash_meta_overridden.overrides_builtin);

    // Delete override restores built-in
    delete_skill(dir, "crashloop-triage").unwrap();

    let restored = load_skill_or_builtin(dir, "crashloop-triage").unwrap();
    assert!(restored.body.contains("When a pod keeps restarting"));

    let all_restored = list_all_skills(dir).unwrap();
    let crash_meta_restored = all_restored
        .iter()
        .find(|s| s.name == "crashloop-triage")
        .unwrap();
    assert!(crash_meta_restored.builtin);
    assert!(!crash_meta_restored.overrides_builtin);
}

#[test]
fn dynamic_tui_slash_commands_discovery() {
    let temp = tempdir().unwrap();
    let dir = temp.path();

    let skill = Skill {
        name: "custom-db-drain".to_string(),
        description: "Safely drain database replicas".to_string(),
        body: "Step 1: Check replication lag.\nStep 2: Cordon and step down leader.".to_string(),
    };
    write_skill_atomic(dir, &skill).unwrap();

    // 1. Playbooks loaded from dir include custom skill
    let playbooks = load_all_playbooks_from(dir);
    let found = playbooks
        .iter()
        .find(|p| p.name == "custom-db-drain")
        .unwrap();
    assert_eq!(found.command, "custom-db-drain");
    assert_eq!(found.description, "Safely drain database replicas");
    assert!(found.is_user_defined);

    // 2. Slash command prefix matching discovers custom skill
    let suggestions = match_slash_commands_from("/custom", dir);
    assert_eq!(suggestions.len(), 1);
    assert_eq!(suggestions[0].name, "custom-db-drain");
    assert_eq!(suggestions[0].command, "custom-db-drain");

    // 3. Prompt expansion for custom skill
    let prompt = expand_slash_command_from(
        "custom-db-drain",
        Some("postgres-cluster-1"),
        "production-us-east",
        "database",
        dir,
    )
    .unwrap();

    assert!(prompt.contains("Playbook: Safely drain database replicas"));
    assert!(prompt.contains("Context: Cluster 'production-us-east', namespace 'database'"));
    assert!(prompt.contains("Focus on resource 'postgres-cluster-1' in namespace 'database'."));
    assert!(prompt.contains("Step 1: Check replication lag."));
}

#[test]
fn fetch_skill_url_handles_success_and_errors() {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    let server = std::thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut req = [0u8; 1024];
            let _ = stream.read(&mut req);
            let req_str = String::from_utf8_lossy(&req);

            if req_str.contains("GET /notfound.md") {
                let resp =
                    "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                stream.write_all(resp.as_bytes()).unwrap();
            } else {
                let body =
                    "---\nname: remote-skill\ndescription: Remote desc\n---\nRemote content body";
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream.write_all(resp.as_bytes()).unwrap();
            }
        }
    });

    // 1. Successful fetch
    let url_ok = format!("http://127.0.0.1:{port}/skill.md");
    let fetched = srectl::ai_skills::fetch_skill_url(&url_ok).unwrap();
    assert!(fetched.contains("remote-skill"));

    // 2. 404 Not Found error
    let url_404 = format!("http://127.0.0.1:{port}/notfound.md");
    let err = srectl::ai_skills::fetch_skill_url(&url_404).unwrap_err();
    assert!(err.contains("HTTP 404"));

    let _ = server.join();
}
