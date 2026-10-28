//! The models a picker offers: an HTTP provider's own list, or a local agent
//! CLI's (Claude Code has no list command, so its aliases).

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use srelens_agent::adapter::{
    model_list_args, valid_model_id, AgentKind, ModelChoice, CLAUDE_MODEL_ALIASES,
};
use srelens_llm::{HttpProvider, Provider};
use tokio::io::AsyncReadExt;

use crate::ai_config::{agent_slug, provider_display_name, provider_slug, AiProvider, AiSettings};

/// How long a listing may take before the picker stops waiting for it.
const LIST_TIMEOUT: Duration = Duration::from_secs(15);

/// The cache key for `p`'s list: another base URL, or another key (another
/// account may list other models), is another list.
pub fn provider_cache_key(settings: &AiSettings, p: AiProvider) -> String {
    format!(
        "{}|{}|{:?}",
        provider_slug(p),
        settings.get_base_url(p),
        fingerprint(settings.get_api_key(p))
    )
}

/// The cache key for `kind`'s list; Cursor's depends on its key too.
pub fn agent_cache_key(settings: &AiSettings, kind: AgentKind) -> String {
    let key = (kind == AgentKind::Cursor)
        .then(|| settings.get_cursor_key())
        .flatten();
    format!("agent|{}|{:?}", agent_slug(kind), fingerprint(key))
}

/// Tells keys apart without holding one: hashed with a salt picked once per
/// run, so the value means nothing outside this process.
fn fingerprint(secret: Option<String>) -> Option<u64> {
    use std::hash::BuildHasher;
    static SALT: std::sync::OnceLock<std::collections::hash_map::RandomState> =
        std::sync::OnceLock::new();
    secret.map(|s| SALT.get_or_init(Default::default).hash_one(s))
}

/// `p`'s models from its own models endpoint, with the configured key and
/// base URL. No key (where one is needed) fails before any request.
pub async fn provider_models(
    settings: &AiSettings,
    p: AiProvider,
) -> Result<Vec<ModelChoice>, String> {
    let name = provider_display_name(p);
    let config = settings
        .resolve_provider_config(p)
        .ok_or_else(|| format!("Add an API key for {name} first"))?;
    let listed = tokio::time::timeout(LIST_TIMEOUT, HttpProvider::new(config).list_models())
        .await
        .map_err(|_| format!("{name} did not answer within {} s", LIST_TIMEOUT.as_secs()))?
        .map_err(|e| e.to_string())?;
    let models: Vec<ModelChoice> = listed
        .into_iter()
        .filter(|m| valid_model_id(&m.id))
        .map(|m| ModelChoice {
            id: m.id,
            label: m.display_name,
        })
        .collect();
    if models.is_empty() {
        return Err(format!("{name} returned no models"));
    }
    Ok(models)
}

/// `kind`'s models: Claude Code's aliases, or what the CLI at `binary` lists
/// (stdout only, at most [`LIST_TIMEOUT`]). Cursor gets `cursor_key` as
/// `CURSOR_API_KEY` in its env, as a Cursor turn does, never in its argv.
pub async fn agent_models(
    kind: AgentKind,
    binary: Option<&Path>,
    cursor_key: Option<String>,
) -> Result<Vec<ModelChoice>, String> {
    let label = kind.label();
    let Some(args) = model_list_args(kind) else {
        return match kind {
            AgentKind::Claude => Ok(CLAUDE_MODEL_ALIASES
                .iter()
                .map(|a| ModelChoice {
                    id: a.to_string(),
                    label: a.to_string(),
                })
                .collect()),
            _ => Err(format!("{label} uses the provider's models")),
        };
    };
    let binary = binary.ok_or_else(|| format!("{label} is not installed"))?;
    let mut command = tokio::process::Command::new(binary);
    command
        .args(args)
        .envs(
            cursor_key
                .filter(|_| kind == AgentKind::Cursor)
                .map(|key| ("CURSOR_API_KEY", key)),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    // A CLI just written can be busy for a moment (`ETXTBSY`): an installer
    // still closing it, or a fork in another thread holding a copy of its
    // write fd until that fork execs (rust-lang/rust#114554).
    let mut busy = 0;
    let mut child = loop {
        match command.spawn() {
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && busy < 10 => {
                busy += 1;
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            spawned => break spawned.map_err(|e| format!("Could not run {label}: {e}"))?,
        }
    };
    let mut stdout = child.stdout.take().expect("stdout is piped");
    let mut out = Vec::new();
    // A `let`, so the future's borrow of `child` ends before the kill below.
    let run = tokio::time::timeout(LIST_TIMEOUT, async {
        stdout.read_to_end(&mut out).await?;
        child.wait().await
    })
    .await;
    let status = match run {
        Ok(Ok(status)) => status,
        Ok(Err(e)) => return Err(format!("Could not read {label}'s models: {e}")),
        Err(_) => {
            // A `.cmd` shim's real CLI outlives killing `cmd.exe` alone.
            srelens_agent::turn::kill_tree(&mut child);
            return Err(format!(
                "{label} did not list its models within {} s",
                LIST_TIMEOUT.as_secs()
            ));
        }
    };
    let text = String::from_utf8_lossy(&out);
    if !status.success() {
        let said: Option<String> = text
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .map(|l| l.chars().filter(|c| !c.is_control()).take(200).collect());
        return Err(match said {
            Some(said) => format!("{label} could not list its models ({status}): {said}"),
            None => format!("{label} could not list its models ({status})"),
        });
    }
    let models = match kind {
        AgentKind::Codex => srelens_agent::codex::parse_models(&text),
        _ => srelens_agent::cursor::parse_models(&text),
    };
    if models.is_empty() {
        return Err(format!("{label} listed no models"));
    }
    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A stand-in CLI that prints `out` (one line; the shell expands its
    /// variables) and exits `code`.
    fn fake_cli(dir: &Path, out: &str, code: i32) -> PathBuf {
        #[cfg(windows)]
        {
            let path = dir.join("fake.cmd");
            std::fs::write(
                &path,
                format!("@echo off\r\necho {out}\r\nexit /b {code}\r\n"),
            )
            .unwrap();
            path
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let path = dir.join("fake.sh");
            let out = out.replace('"', "\\\"");
            std::fs::write(&path, format!("#!/bin/sh\necho \"{out}\"\nexit {code}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path
        }
    }

    #[tokio::test]
    async fn claude_offers_its_aliases() {
        let models = agent_models(AgentKind::Claude, None, None).await.unwrap();
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, CLAUDE_MODEL_ALIASES);
        assert!(models.iter().all(|m| m.label == m.id));
    }

    #[tokio::test]
    async fn an_agent_that_is_not_installed_says_so() {
        let err = agent_models(AgentKind::Codex, None, None)
            .await
            .unwrap_err();
        assert_eq!(err, "Codex is not installed");
    }

    #[tokio::test]
    async fn a_cli_listing_becomes_the_choices() {
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_cli(dir.path(), "m-1 - Model One", 0);
        let models = agent_models(AgentKind::Cursor, Some(&cli), None)
            .await
            .unwrap();
        assert_eq!(
            models,
            [ModelChoice {
                id: "m-1".into(),
                label: "Model One".into()
            }]
        );
    }

    #[tokio::test]
    async fn cursor_lists_with_its_key_in_its_env_never_in_its_argv() {
        // The id echoes the key from the env; the label echoes the argv.
        #[cfg(windows)]
        const LINE: &str = "m-%CURSOR_API_KEY% - %*";
        #[cfg(unix)]
        const LINE: &str = "m-$CURSOR_API_KEY - $*";
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_cli(dir.path(), LINE, 0);
        let models = agent_models(AgentKind::Cursor, Some(&cli), Some("cur-test-1".into()))
            .await
            .unwrap();
        assert_eq!(
            models,
            [ModelChoice {
                id: "m-cur-test-1".into(),
                label: "models".into()
            }]
        );
    }

    #[tokio::test]
    async fn a_listing_with_no_models_is_an_error_not_an_empty_list() {
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_cli(dir.path(), r#"{"models":[]}"#, 0);
        let err = agent_models(AgentKind::Codex, Some(&cli), None)
            .await
            .unwrap_err();
        assert_eq!(err, "Codex listed no models");
    }

    #[tokio::test]
    async fn a_listing_that_fails_says_what_the_cli_printed() {
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_cli(dir.path(), "Not logged in", 3);
        let err = agent_models(AgentKind::Cursor, Some(&cli), None)
            .await
            .unwrap_err();
        assert!(err.starts_with("Cursor could not list its models"), "{err}");
        assert!(err.contains("Not logged in"), "{err}");
    }

    /// Linux will not exec a file that is open for writing (`ETXTBSY`). A CLI
    /// an installer has just written can be busy like that for a moment, and
    /// so can every fake here: a fork in another test thread keeps a copy of
    /// the script's write fd until it execs (rust-lang/rust#114554). Holding
    /// a write handle open makes the spawn hit it on purpose.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn a_cli_busy_for_a_moment_still_lists_its_models() {
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_cli(dir.path(), "m-1 - Model One", 0);
        let writer = std::fs::OpenOptions::new().write(true).open(&cli).unwrap();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            drop(writer);
        });
        let models = agent_models(AgentKind::Cursor, Some(&cli), None)
            .await
            .unwrap();
        assert_eq!(
            models,
            [ModelChoice {
                id: "m-1".into(),
                label: "Model One".into()
            }]
        );
    }

    /// ...but one that stays busy is reported, not waited on forever.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn a_cli_that_stays_busy_is_an_error_not_a_hang() {
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_cli(dir.path(), "m-1 - Model One", 0);
        let _writer = std::fs::OpenOptions::new().write(true).open(&cli).unwrap();
        let listing = agent_models(AgentKind::Cursor, Some(&cli), None);
        let err = tokio::time::timeout(Duration::from_secs(10), listing)
            .await
            .expect("a busy CLI is retried a few times, not forever")
            .unwrap_err();
        assert!(err.starts_with("Could not run Cursor: "), "{err}");
    }
}
