//! Per-turn setup shared by every front end that drives an agent CLI: the
//! private scratch files and directories a turn writes (MCP configs carrying a
//! bearer token, attached images, the agent's empty working directory) and the
//! guards that remove them when the turn ends.

use crate::adapter::AgentKind;

/// Whether `id` may follow `--resume` (or `exec resume`) in an agent's argv.
/// Real ids are CLI-minted uuids, but the one a turn resumes round-trips
/// through a session file on disk, so a tampered store must not be able to
/// smuggle a flag-shaped or whitespace-carrying token into the command line:
/// a bare `[A-Za-z0-9._-]+` that does not start with `-`. Callers drop an
/// invalid id (a fresh session) rather than failing the turn — continuity is
/// best-effort by design.
pub fn valid_resume_id(id: &str) -> bool {
    !id.is_empty()
        && !id.starts_with('-')
        && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Removes the wrapped path when dropped. The MCP config file `chat_send`
/// writes for a turn carries the bearer token in cleartext, so it must be
/// removed on every exit — including an early `?` after a failed spawn — not
/// just the happy path a bare `remove_file` at the end of the function would
/// cover.
pub struct TempFile(pub std::path::PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Removes the wrapped directory (and everything under it) when dropped. The
/// agent is spawned with this as its working directory so it has no code or
/// files of ours to enumerate even if a tool tried; the directory is created
/// empty per turn and torn down when the turn ends, same lifecycle as
/// `TempFile` above.
pub struct TempDir(pub std::path::PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Create a fresh private directory under `base` — the directory analogue of
/// `write_private_file`. The shared system temp dir is world-writable on a
/// multi-user Unix host, and a predictable name could be pre-created by
/// another account (as a symlink or an attacker-writable directory) to read
/// or replace what a turn writes inside — e.g. swapping Cursor's
/// `cli-config.json` deny-list, or seeding Codex's supposedly-empty `-C`
/// workspace. So: a randomized suffix (unguessable), `DirBuilder::create`
/// rather than `create_dir_all` (anything pre-existing — including a planted
/// symlink — is an error, never accepted), and `0700` at creation on Unix.
pub fn create_private_dir(
    base: &std::path::Path,
    prefix: &str,
) -> Result<std::path::PathBuf, String> {
    for _ in 0..8 {
        let path = base.join(format!("{prefix}-{}", uuid::Uuid::new_v4()));
        // Only the Unix branch below mutates it; on Windows the `mut` is
        // unused and rustc says so on every build.
        #[cfg_attr(not(unix), allow(unused_mut))]
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        match builder.create(&path) {
            Ok(()) => return Ok(path),
            // A uuid collision is practically impossible; treat it as the
            // race it would be and try a fresh name.
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    Err("could not create a private temp directory".into())
}

/// Write a per-turn scratch file with owner-only permissions (`0600` on Unix),
/// so another local account can't read it out of the world-readable shared temp
/// dir while a turn runs. Used for everything sensitive we drop there: the MCP
/// bearer-token configs (Claude `--mcp-config`, Cursor `mcp.json`) and attached
/// image files (their paths are visible in Codex's `-i` argv). On non-Unix the
/// default per-user temp permissions apply.
pub fn write_private_file(path: &std::path::Path, contents: &[u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        // Remove any leftover of OUR own from a prior turn, then `create_new`
        // (O_CREAT|O_EXCL): the open refuses if anything exists at `path` —
        // including a symlink another local account pre-created at a
        // predictable/reused scratch path to redirect the write or read the
        // token. Removing a symlink drops the link, not its target; if the
        // attacker re-creates it in the race window the exclusive open fails,
        // so we never follow it. `.mode(0o600)` only governs a fresh file.
        let _ = std::fs::remove_file(path);
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(contents)
    }
    #[cfg(not(unix))]
    {
        // Match the fail-closed semantics on other platforms too.
        use std::io::Write;
        let _ = std::fs::remove_file(path);
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
        f.write_all(contents)
    }
}

/// The Cursor chats a turn or a delete is using right now. A chat's directory
/// is one per conversation (#952), and two windows can hold one saved
/// conversation, each streaming on its own session and seeing only its own
/// turns as busy. Without this, the second turn would empty the first one's
/// workspace and either turn's guard would remove the other's files.
#[derive(Default, Clone)]
pub struct CursorChatsInUse(std::sync::Arc<std::sync::Mutex<std::collections::HashSet<String>>>);

/// One holder's use of a Cursor chat, released when dropped.
pub struct CursorChatClaim {
    in_use: CursorChatsInUse,
    chat: String,
}

impl CursorChatsInUse {
    /// Claim `chat`, or `None` while something else holds it.
    pub fn claim(&self, chat: &str) -> Option<CursorChatClaim> {
        let fresh = self.0.lock().unwrap().insert(chat.to_string());
        fresh.then(|| CursorChatClaim { in_use: self.clone(), chat: chat.to_string() })
    }
}

impl Drop for CursorChatClaim {
    fn drop(&mut self) {
        self.in_use.0.lock().unwrap().remove(&self.chat);
    }
}

/// One Cursor turn, ready to spawn: its command, and the workspace to spawn it
/// in, which is removed (bearer token and all) when the turn ends.
pub struct CursorTurn {
    pub cmd: crate::adapter::AgentCommand,
    pub workspace: std::path::PathBuf,
    _workspace_guard: TempDir,
    // Declared after the guard, so it is dropped after it: the chat is
    // released only once its workspace is gone.
    _claim: CursorChatClaim,
}

/// Remove the Cursor chats no saved conversation holds — a one-shot turn's,
/// or one whose save never landed — and, from those that stay, the workspace
/// a killed turn left behind (its guard never ran). They hold prompts and
/// tool results, so a failure is logged: it is why they are still on disk.
pub fn sweep_cursor_chats(root: &std::path::Path, saved: impl Fn(&str) -> bool) {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => {
            log::warn!("could not list the Cursor chats in {}: {error}", root.display());
            return;
        }
    };
    for entry in entries.flatten() {
        let chat = entry.path();
        let stale = if entry.file_name().to_str().is_some_and(&saved) { chat.join("workspace") } else { chat };
        if let Err(error) = crate::remove_cursor_chats(&stale) {
            log::warn!("could not delete {}: {error}", stale.display());
        }
    }
}

/// Lay out a Cursor turn under `root` and build its command.
///
/// cursor-agent keeps a chat at `CURSOR_CONFIG_DIR/chats/<md5 of the absolute
/// --workspace>/<chatId>` (its bundled `paths.js`), so `--resume` finds the
/// previous turn's chat only when both paths are the same on every turn and
/// the config dir outlives the turn (#952). Both are fixed per conversation
/// under `root`, and the config dir is kept between turns and across runs.
///
/// What it keeps must not loosen the box. Unlike `codex exec resume` (#215),
/// `--resume` takes every flag a fresh turn does, so `cursor_command` is
/// unchanged; and nothing a turn leaves carries into the next: the deny-list
/// is rewritten every turn, and the workspace is emptied before the turn and
/// removed after it, taking the bearer token with it.
///
/// Cursor loads permissions and MCP servers from DIFFERENT places: the
/// deny-list is read from `CURSOR_CONFIG_DIR/cli-config.json`, but MCP servers
/// are read from the WORKSPACE's `.cursor/mcp.json` (project config), NOT from
/// `CURSOR_CONFIG_DIR` (verified live: an mcp.json in CURSOR_CONFIG_DIR is
/// silently ignored, so the server never connects). So the deny-list goes in
/// the config dir and the MCP server goes in `<workspace>/.cursor/mcp.json`.
#[allow(clippy::too_many_arguments)]
pub fn prepare_cursor_turn(
    root: &std::path::Path,
    in_use: &CursorChatsInUse,
    session: &str,
    conversation: Option<&str>,
    agent_path: &str,
    prompt: &str,
    url: &str,
    token: &str,
    resume: Option<&str>,
) -> Result<CursorTurn, String> {
    // The saved conversation names the chat, so a reopened one — on a new
    // session — finds it again. A turn with no conversation (a one-shot
    // generation) has only its session.
    let chat = conversation.unwrap_or(session);
    let dir = crate::cursor_chat_dir(root, chat)?;
    // Held for the whole turn, before the workspace below is emptied.
    let claim = in_use
        .claim(chat)
        .ok_or("This conversation is answering in another window. Wait for it to finish, then ask again.")?;
    // Fixed names are safe here, unlike in the shared temp dir (see
    // `create_private_dir`): `root` is in the user's own cache dir, and
    // owner-only so no other account can read the chats kept under it.
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    }
    let config = dir.join("config");
    let workspace = dir.join("workspace");
    std::fs::create_dir_all(&config).map_err(|e| e.to_string())?;
    // Emptied first: a turn whose guard never ran (srelens was killed) left
    // its files here, and a project `.cursor/cli.json` among them would widen
    // the box. Refuse the turn rather than run it in a workspace that can't be.
    match std::fs::remove_dir_all(&workspace) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.to_string()),
        _ => {}
    }
    let cursor_dir = workspace.join(".cursor");
    std::fs::create_dir_all(&cursor_dir).map_err(|e| e.to_string())?;
    let workspace_guard = TempDir(workspace.clone());
    std::fs::write(config.join("cli-config.json"), crate::adapter::cursor_cli_config_json())
        .map_err(|e| e.to_string())?;
    // `mcp.json` carries the bearer token. The workspace guard removes it with
    // the workspace when the turn ends, and the model can't read it back —
    // the deny-list blocks `Read`.
    write_private_file(&cursor_dir.join("mcp.json"), crate::adapter::cursor_mcp_json(url, token).as_bytes())
        .map_err(|e| e.to_string())?;
    let cmd = crate::adapter::cursor_command(
        agent_path,
        prompt,
        &config.to_string_lossy(),
        &workspace.to_string_lossy(),
        resume,
    );
    Ok(CursorTurn { cmd, workspace, _workspace_guard: workspace_guard, _claim: claim })
}

/// Everything a front end knows about one agent-CLI turn before it is laid
/// out on disk.
pub struct TurnSpec<'a> {
    pub kind: AgentKind,
    pub binary: &'a str,
    pub prompt: &'a str,
    pub mcp_url: &'a str,
    pub token: &'a str,
    /// The CLI's own session id from the conversation's previous turn,
    /// already checked with `valid_resume_id`.
    pub resume: Option<&'a str>,
    /// The model to run the turn on (`AgentCommand::with_model`); `None`, or
    /// an id it refuses, runs the CLI's own default.
    pub model: Option<&'a str>,
    /// Names the turn's scratch files, so it must be one bare path component.
    pub session: &'a str,
    /// The saved conversation, which names a Cursor chat (see
    /// `prepare_cursor_turn`).
    pub conversation: Option<&'a str>,
    pub cursor_root: &'a std::path::Path,
    pub cursor_chats: &'a CursorChatsInUse,
    pub temp_dir: &'a std::path::Path,
    /// Attached images already written to disk; only Codex can read them.
    pub image_paths: &'a [String],
}

/// One agent-CLI turn, ready to spawn: its command and the directory to spawn
/// it in. Everything the turn wrote — its working directory, and the files
/// carrying the bearer token — goes when this is dropped.
pub struct PreparedTurn {
    pub cmd: crate::adapter::AgentCommand,
    pub cwd: std::path::PathBuf,
    _config_guard: Option<TempFile>,
    /// A batch shim's Claude system prompt (see `is_batch_shim`).
    _sysprompt_guard: Option<TempFile>,
    _cwd_guard: Option<TempDir>,
    _cursor_turn: Option<CursorTurn>,
}

/// Lay out one agent-CLI turn and build its command. The native agent
/// (`Srelens`) runs in-process and has no CLI turn, so it is an error here.
pub fn prepare_turn(spec: &TurnSpec) -> Result<PreparedTurn, String> {
    match spec.kind {
        AgentKind::Srelens => Err("srelens's own agent runs in-process; it has no CLI turn to prepare".into()),
        // Cursor's working directory is its workspace, laid out with the rest
        // of its turn by `prepare_cursor_turn`, which guards it.
        AgentKind::Cursor => {
            let turn = prepare_cursor_turn(
                spec.cursor_root,
                spec.cursor_chats,
                spec.session,
                spec.conversation,
                spec.binary,
                spec.prompt,
                spec.mcp_url,
                spec.token,
                spec.resume,
            )?;
            Ok(PreparedTurn {
                cmd: turn.cmd.clone().with_model(spec.kind, spec.model),
                cwd: turn.workspace.clone(),
                _config_guard: None,
                _sysprompt_guard: None,
                _cwd_guard: None,
                _cursor_turn: Some(turn),
            })
        }
        AgentKind::Claude | AgentKind::Codex => {
            // A fresh, empty scratch directory for the agent's CWD — never the
            // srelens process's own working directory (the user's repo or home) —
            // so a tool that tried to enumerate its surroundings would find nothing.
            // Removed with the turn. This is also Codex's `-C`
            // workspace (see `codex_command`): genuinely empty, per-turn, and torn
            // down with the turn — that emptiness is the whole box, so this guard
            // must never be pointed at anything else.
            let cwd = create_private_dir(spec.temp_dir, &format!("srelens-agent-cwd-{}", spec.session))?;
            let cwd_guard = TempDir(cwd.clone());

            // Only Claude reads its MCP server config from a file (`--mcp-config`);
            // Codex gets the same server via `-c` TOML overrides plus its bearer
            // token in `env` (see `codex_command`), so no config file is written for
            // it at all. The guard is kept on the turn, so its `Drop` (removing the
            // cleartext-token file) runs when the turn ends, exactly like the cwd's
            // — a guard built and dropped inside the match arm itself would be gone
            // before the agent ever read the file.
            let mut config_guard: Option<TempFile> = None;
            let mut sysprompt_guard: Option<TempFile> = None;
            // npm installs `claude` and `codex` as `.cmd` shims on Windows, which
            // can't take a newline in argv and expand `%VAR%` in it: their
            // prompt goes on stdin instead (see `is_batch_shim`).
            let shim = crate::adapter::is_batch_shim(spec.binary);
            let cmd = if spec.kind == AgentKind::Claude {
                let cfg = crate::adapter::McpConfig::http(spec.mcp_url, spec.token);
                let cfg_path = spec.temp_dir.join(format!("srelens-mcp-{}.json", spec.session));
                write_private_file(&cfg_path, &serde_json::to_vec(&cfg).unwrap()).map_err(|e| e.to_string())?;
                config_guard = Some(TempFile(cfg_path.clone()));
                let effective_prompt = prompt_with_images(spec.prompt, spec.image_paths);
                if shim {
                    // The system prompt has newlines too, so it goes by file.
                    let sys_path = spec.temp_dir.join(format!("srelens-sysprompt-{}.txt", spec.session));
                    sysprompt_guard = Some(TempFile(sys_path.clone()));
                    write_private_file(&sys_path, crate::adapter::BASE_SYSTEM_PROMPT.as_bytes())
                        .map_err(|e| e.to_string())?;
                    let mut cmd = crate::adapter::claude_command_via_stdin(
                        spec.binary,
                        &cfg_path.to_string_lossy(),
                        &sys_path.to_string_lossy(),
                        spec.resume,
                    );
                    cmd.stdin = Some(effective_prompt);
                    cmd
                } else {
                    crate::adapter::claude_command(spec.binary, &effective_prompt, &cfg_path.to_string_lossy(), spec.resume)
                }
            } else if shim {
                // Whether `exec … resume <thread> -` takes a stdin prompt is
                // unverified, so a shim's follow-up starts a fresh thread.
                let mut cmd = crate::adapter::codex_command_via_stdin(
                    spec.binary,
                    spec.mcp_url,
                    spec.token,
                    &cwd.to_string_lossy(),
                    spec.image_paths,
                    None,
                );
                cmd.stdin = Some(spec.prompt.to_string());
                cmd
            } else {
                // A follow-up resumes the conversation's Codex thread with the whole
                // box re-asserted in front of `resume` (see `codex_command`), in this
                // turn's own fresh empty `-C` workspace: the thread is found by id in
                // `CODEX_HOME`, whatever directory it was started in (#215).
                crate::adapter::codex_command(
                    spec.binary,
                    spec.prompt,
                    spec.mcp_url,
                    spec.token,
                    &cwd.to_string_lossy(),
                    spec.image_paths,
                    spec.resume,
                )
            };
            Ok(PreparedTurn {
                cmd: cmd.with_model(spec.kind, spec.model),
                cwd,
                _config_guard: config_guard,
                _sysprompt_guard: sysprompt_guard,
                _cwd_guard: Some(cwd_guard),
                _cursor_turn: None,
            })
        }
    }
}

/// Begin ending `child` and everything it started; the caller reaps it.
///
/// On Windows an npm-installed agent is a `.cmd` shim, so `child` is only
/// `cmd.exe`: ending that alone leaves the real agent running, spending tokens
/// and holding the stdout pipe open. `taskkill /T` ends the whole tree. Its
/// status is ignored (the tree may already be gone) and it gets no stdio and no
/// console window, which the GUI desktop would otherwise flash.
pub fn kill_tree(child: &mut tokio::process::Child) {
    #[cfg(windows)]
    if let Some(pid) = child.id() {
        use std::os::windows::process::CommandExt;
        use std::process::Stdio;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let _ = std::process::Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .status();
    }
    let _ = child.start_kill();
}

/// Prepend one `Attached image: <path>` line per path, then a blank line,
/// then the original prompt — so the agent sees where each decoded image
/// landed on disk before reading the user's actual question. `image_paths`
/// empty (the common case: no attachments) returns `prompt` unchanged,
/// byte-identical, so a turn without images pays no cost here.
fn prompt_with_images(prompt: &str, image_paths: &[String]) -> String {
    if image_paths.is_empty() {
        return prompt.to_string();
    }
    let mut out = String::new();
    for path in image_paths {
        out.push_str("Attached image: ");
        out.push_str(path);
        out.push('\n');
    }
    out.push('\n');
    out.push_str(prompt);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remove_cursor_chats;

    #[test]
    fn a_resume_id_is_a_bare_token_that_cannot_pass_for_a_flag_or_a_path() {
        assert!(valid_resume_id("ee637368-ca14-4420-b431-e346fc3877b5"));
        for bad in ["", "a b", "--resume", "../x", "a/b"] {
            assert!(!valid_resume_id(bad), "{bad:?} must be refused");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_private_file_is_written_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("srelens-private-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mcp.json");
        write_private_file(&path, b"secret-bearer-token").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        // Only the owner may read/write; group and other bits must be clear.
        assert_eq!(mode & 0o777, 0o600, "private file must be 0600, got {:o}", mode & 0o777);
        assert_eq!(std::fs::read(&path).unwrap(), b"secret-bearer-token");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn private_dirs_are_freshly_created_owner_only_and_never_reused() {
        let base = std::env::temp_dir().join(format!("srelens-test-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let a = create_private_dir(&base, "cfg-s1").unwrap();
        let b = create_private_dir(&base, "cfg-s1").unwrap();
        // Same prefix, distinct directories — the randomized suffix is what
        // makes pre-creation by another local account impossible.
        assert_ne!(a, b);
        assert!(a.is_dir() && b.is_dir());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&a).unwrap().permissions().mode() & 0o777, 0o700);
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn temp_file_guard_removes_the_file_on_drop() {
        let path = std::env::temp_dir()
            .join(format!("srelens-assistant-tempfile-test-{}", std::process::id()));
        std::fs::write(&path, b"secret").unwrap();
        {
            let _guard = TempFile(path.clone());
            assert!(path.exists());
        }
        assert!(!path.exists());
    }

    /// Pins the ordering `chat_send`'s image loop relies on: the guard must
    /// be constructed *before* `std::fs::write` is attempted, not after a
    /// successful write. This doesn't inject a real fs failure (that would be
    /// a lot of mocking for one ordering guarantee) — it just proves a guard
    /// built ahead of the write still cleans up whatever lands on disk at
    /// that path, including a write that only partially completes, which a
    /// guard pushed *after* `write` returns would never get the chance to do.
    #[test]
    fn temp_file_guard_created_before_the_write_still_cleans_up_a_partial_write() {
        let path = std::env::temp_dir().join(format!(
            "srelens-assistant-tempfile-partial-write-test-{}",
            std::process::id()
        ));
        assert!(!path.exists());
        {
            let _guard = TempFile(path.clone()); // guard first, as in chat_send's loop
            std::fs::write(&path, b"partial").unwrap(); // stands in for a write that landed bytes before failing
            assert!(path.exists());
        }
        assert!(!path.exists());
    }

    #[test]
    fn temp_dir_guard_removes_the_directory_and_its_contents_on_drop() {
        let path = std::env::temp_dir()
            .join(format!("srelens-assistant-tempdir-test-{}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("nested.txt"), b"nothing to see here").unwrap();
        {
            let _guard = TempDir(path.clone());
            assert!(path.exists());
        }
        assert!(!path.exists());
    }

    /// Where cursor-agent keeps a chat: `$CURSOR_CONFIG_DIR/chats/<md5 of the
    /// absolute --workspace>/<chatId>/store.db`, per its bundled `paths.js`.
    /// Read from the command a turn would run. Any function of the workspace
    /// path stands in for the md5: a workspace that moves moves the store.
    fn cursor_chat_store(cmd: &crate::adapter::AgentCommand, chat: &str) -> std::path::PathBuf {
        let at = cmd.args.iter().position(|a| a == "--workspace").expect("a Cursor turn passes --workspace");
        let hash: String =
            cmd.args[at + 1].chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
        cursor_config_dir(cmd).join("chats").join(hash).join(chat).join("store.db")
    }

    fn cursor_config_dir(cmd: &crate::adapter::AgentCommand) -> std::path::PathBuf {
        let (_, dir) = cmd
            .env
            .iter()
            .find(|(k, _)| k == crate::adapter::CURSOR_CONFIG_DIR_ENV)
            .expect("a Cursor turn sets CURSOR_CONFIG_DIR");
        dir.into()
    }

    /// A root of this test's own for its Cursor turns, removed when it ends.
    fn cursor_test_root() -> TempDir {
        let root = std::env::temp_dir().join(format!("srelens-cursor-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        TempDir(root)
    }

    fn cursor_turn(root: &TempDir, session: &str, resume: Option<&str>) -> CursorTurn {
        prepare_cursor_turn(
            &root.0,
            &CursorChatsInUse::default(),
            session,
            None,
            "cursor-agent",
            "in which node that pod is running?",
            "http://127.0.0.1:1/mcp",
            "turn-token",
            resume,
        )
        .unwrap()
    }

    #[test]
    fn a_second_cursor_turn_finds_the_chat_its_first_turn_stored() {
        let root = cursor_test_root();
        let first = cursor_turn(&root, "s1", None);
        // What cursor-agent does during the turn: it keeps the chat.
        let store = cursor_chat_store(&first.cmd, "chat-1");
        std::fs::create_dir_all(store.parent().unwrap()).unwrap();
        std::fs::write(&store, "cpu and ram of pod nic-ring-audit-sz2vb").unwrap();
        drop(first); // the turn ends

        let second = cursor_turn(&root, "s1", Some("chat-1"));
        assert_eq!(
            std::fs::read_to_string(cursor_chat_store(&second.cmd, "chat-1")).ok().as_deref(),
            Some("cpu and ram of pod nic-ring-audit-sz2vb"),
            "turn 2's --resume must reach the chat turn 1 kept (#952)"
        );
    }

    #[test]
    fn a_cursor_turn_for_a_reopened_conversation_finds_the_chat_it_kept_before() {
        let root = cursor_test_root();
        let first =
            prepare_cursor_turn(&root.0, &CursorChatsInUse::default(), "run-1", Some("conv-1"), "cursor-agent", "hi", "http://127.0.0.1:1/mcp", "t", None)
                .unwrap();
        let store = cursor_chat_store(&first.cmd, "chat-1");
        std::fs::create_dir_all(store.parent().unwrap()).unwrap();
        std::fs::write(&store, "cpu and ram of pod nic-ring-audit-sz2vb").unwrap();
        drop(first);

        // Reopened after a restart: the conversation is the same, the session
        // `chat_send` streams on is new.
        let reopened = prepare_cursor_turn(
            &root.0,
            &CursorChatsInUse::default(),
            "run-2",
            Some("conv-1"),
            "cursor-agent",
            "and its node?",
            "http://127.0.0.1:1/mcp",
            "t",
            Some("chat-1"),
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(cursor_chat_store(&reopened.cmd, "chat-1")).ok().as_deref(),
            Some("cpu and ram of pod nic-ring-audit-sz2vb"),
            "a reopened conversation's --resume must reach the chat it kept (#952)"
        );
    }

    #[test]
    fn a_conversation_answering_in_one_window_refuses_a_turn_from_another() {
        let root = cursor_test_root();
        let in_use = CursorChatsInUse::default();
        // Each window streams on its own session; the conversation is one.
        let turn = |session: &str| {
            prepare_cursor_turn(&root.0, &in_use, session, Some("conv-1"), "cursor-agent", "hi", "http://127.0.0.1:1/mcp", "t", None)
        };
        let first = turn("window-1").unwrap();

        assert!(turn("window-2").is_err(), "a second turn would empty the first one's workspace");
        assert!(first.workspace.join(".cursor").join("mcp.json").exists());
        drop(first);
        assert!(turn("window-2").is_ok(), "once the turn has ended, the conversation is free again");
    }

    #[test]
    fn a_cursor_chat_outlives_a_restart_only_while_a_saved_conversation_holds_it() {
        let root = cursor_test_root();
        let saved =
            prepare_cursor_turn(&root.0, &CursorChatsInUse::default(), "run-1", Some("saved"), "cursor-agent", "hi", "http://127.0.0.1:1/mcp", "t", None)
                .unwrap();
        let store = cursor_chat_store(&saved.cmd, "chat-1");
        std::fs::create_dir_all(store.parent().unwrap()).unwrap();
        std::fs::write(&store, "kept").unwrap();
        drop(saved);
        // A one-shot turn has no conversation, so nothing saved holds its chat.
        drop(cursor_turn(&root, "run-1-generation", None));

        sweep_cursor_chats(&root.0, |id| id == "saved");

        assert!(store.exists(), "a saved conversation keeps its chat across a restart (#952)");
        assert!(!root.0.join("run-1-generation").exists());
    }

    #[test]
    fn a_restart_removes_the_workspace_a_killed_cursor_turn_left() {
        let root = cursor_test_root();
        let turn =
            prepare_cursor_turn(&root.0, &CursorChatsInUse::default(), "run-1", Some("saved"), "cursor-agent", "hi", "http://127.0.0.1:1/mcp", "t", None)
                .unwrap();
        let workspace = turn.workspace.clone();
        std::mem::forget(turn); // srelens was killed: the guard never ran

        sweep_cursor_chats(&root.0, |id| id == "saved");

        assert!(!workspace.exists(), "a turn's workspace, bearer token and all, never outlives the run");
    }

    #[test]
    fn a_cursor_turn_takes_its_bearer_token_with_it() {
        let root = cursor_test_root();
        let turn = cursor_turn(&root, "s1", None);
        let token_file = turn.workspace.join(".cursor").join("mcp.json");
        assert!(std::fs::read_to_string(&token_file).unwrap().contains("Bearer turn-token"));
        drop(turn);
        assert!(!token_file.exists(), "the turn's bearer token must not outlive it");
    }

    #[test]
    fn every_cursor_turn_starts_from_srelens_deny_list() {
        let root = cursor_test_root();
        let first = cursor_turn(&root, "s1", None);
        // Whatever rewrote it during the turn — Cursor itself, say — must not
        // carry into the next one: this file is the only thing boxing Cursor.
        std::fs::write(
            cursor_config_dir(&first.cmd).join("cli-config.json"),
            r#"{"version":1,"permissions":{"allow":["Shell(**)"],"deny":[]}}"#,
        )
        .unwrap();
        drop(first);

        let second = cursor_turn(&root, "s1", Some("chat-1"));
        let config: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(cursor_config_dir(&second.cmd).join("cli-config.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(config["permissions"]["allow"], serde_json::json!([]));
        let deny = config["permissions"]["deny"].as_array().unwrap();
        for tool in ["Read(**)", "Write(**)", "Shell(**)", "Fetch(**)"] {
            assert!(deny.contains(&serde_json::json!(tool)), "{tool} must be denied again");
        }
    }

    #[test]
    fn every_cursor_turn_starts_in_an_empty_workspace() {
        let root = cursor_test_root();
        let first = cursor_turn(&root, "s1", None);
        // Left behind by a turn whose cleanup never ran (srelens was killed):
        // project permissions in the workspace would widen the box.
        std::fs::write(first.workspace.join(".cursor").join("cli.json"), r#"{"permissions":{"allow":["Shell(**)"]}}"#)
            .unwrap();
        std::fs::write(first.workspace.join("notes.txt"), "left over").unwrap();
        std::mem::forget(first);

        let second = cursor_turn(&root, "s1", Some("chat-1"));
        assert!(!second.workspace.join(".cursor").join("cli.json").exists());
        assert!(!second.workspace.join("notes.txt").exists());
        assert!(second.workspace.join(".cursor").join("mcp.json").exists());
    }

    #[test]
    fn two_chats_never_share_cursor_state() {
        let root = cursor_test_root();
        let a = cursor_turn(&root, "s1", None);
        let b = cursor_turn(&root, "s2", None);
        // Each turn's workspace carries its own bearer token, so a shared one
        // would hand one chat's gates to the other (#393).
        assert_ne!(a.workspace, b.workspace);
        assert_ne!(cursor_config_dir(&a.cmd), cursor_config_dir(&b.cmd));
    }

    #[test]
    fn a_session_id_cannot_take_a_cursor_turn_out_of_its_root() {
        let base = cursor_test_root();
        let root = base.0.join("cursor-chats");
        // `chat_send` lets `.` through its session-id charset, so `..` is a
        // well-formed id there — and the workspace under it gets emptied.
        for session in ["..", ".", "a/../.."] {
            let turn =
                prepare_cursor_turn(&root, &CursorChatsInUse::default(), session, None, "cursor-agent", "hi", "http://127.0.0.1:1/mcp", "turn-token", None);
            assert!(turn.is_err(), "{session:?} must be refused");
        }
        assert!(!base.0.join("config").exists());
        assert!(!base.0.join("workspace").exists());
    }

    #[test]
    fn cursor_chats_already_gone_are_not_a_failure_to_remove_them() {
        let base = cursor_test_root();
        assert!(remove_cursor_chats(&base.0.join("cursor-chats")).is_ok());
    }

    #[test]
    fn cursor_chats_that_could_not_be_removed_are_reported() {
        let base = cursor_test_root();
        // Not a directory, so it can't be removed as one.
        let file = base.0.join("cursor-chats");
        std::fs::write(&file, "kept").unwrap();
        assert!(remove_cursor_chats(&file).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn the_root_cursor_chats_are_kept_in_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let base = cursor_test_root();
        // Not there yet, as on the first Cursor turn after launch.
        let root = base.0.join("cursor-chats");
        let _turn =
            prepare_cursor_turn(&root, &CursorChatsInUse::default(), "s1", None, "cursor-agent", "hi", "http://127.0.0.1:1/mcp", "turn-token", None).unwrap();
        assert_eq!(std::fs::metadata(&root).unwrap().permissions().mode() & 0o777, 0o700);
    }

    /// A root of the test's own for every file a turn writes, and the Cursor
    /// chats in use there.
    struct TurnFixture {
        root: TempDir,
        chats: CursorChatsInUse,
    }

    impl TurnFixture {
        fn new() -> Self {
            TurnFixture { root: cursor_test_root(), chats: CursorChatsInUse::default() }
        }

        fn spec(&self, kind: AgentKind) -> TurnSpec<'_> {
            TurnSpec {
                kind,
                binary: "agent",
                prompt: "in which node is that pod running?",
                mcp_url: "http://127.0.0.1:1/mcp",
                token: "turn-token",
                resume: None,
                model: None,
                session: "s1",
                conversation: None,
                cursor_root: &self.root.0,
                cursor_chats: &self.chats,
                temp_dir: &self.root.0,
                image_paths: &[],
            }
        }
    }

    #[test]
    fn a_claude_turn_points_at_a_private_config_that_goes_with_the_turn() {
        let f = TurnFixture::new();
        let turn = prepare_turn(&f.spec(AgentKind::Claude)).unwrap();
        let at = turn.cmd.args.iter().position(|a| a == "--mcp-config").expect("a Claude turn passes --mcp-config");
        let config = std::path::PathBuf::from(&turn.cmd.args[at + 1]);
        assert!(std::fs::read_to_string(&config).unwrap().contains("Bearer turn-token"));
        assert!(!turn.cmd.args.iter().any(|a| a.contains("turn-token")), "the token never rides in argv");
        let cwd = turn.cwd.clone();
        assert!(cwd.is_dir());
        drop(turn);
        assert!(!config.exists(), "the turn's bearer token must not outlive it");
        assert!(!cwd.exists());
    }

    #[test]
    fn a_codex_turn_runs_in_an_empty_private_dir_removed_with_the_turn() {
        let f = TurnFixture::new();
        let images = ["/tmp/a.png".to_string()];
        let turn = prepare_turn(&TurnSpec { image_paths: &images, ..f.spec(AgentKind::Codex) }).unwrap();
        assert!(turn.cmd.args.windows(2).any(|w| w[0] == "-C" && std::path::Path::new(&w[1]) == turn.cwd));
        assert_eq!(std::fs::read_dir(&turn.cwd).unwrap().count(), 0);
        assert!(turn.cmd.args.windows(2).any(|w| w == ["-i", "/tmp/a.png"]));
        assert!(turn.cmd.env.contains(&(crate::adapter::CODEX_TOKEN_ENV.to_string(), "turn-token".to_string())));
        let cwd = turn.cwd.clone();
        drop(turn);
        assert!(!cwd.exists());
    }

    #[test]
    fn a_cursor_turn_is_prepare_cursor_turn() {
        let f = TurnFixture::new();
        let turn = prepare_turn(&TurnSpec { conversation: Some("conv-1"), ..f.spec(AgentKind::Cursor) }).unwrap();
        assert_eq!(turn.cwd, f.root.0.join("conv-1").join("workspace"));
        assert!(turn.cmd.env.iter().any(|(k, _)| k == crate::adapter::CURSOR_CONFIG_DIR_ENV));
        // The turn holds the workspace and the chat until it is dropped.
        let token_file = turn.cwd.join(".cursor").join("mcp.json");
        assert!(token_file.exists());
        assert!(prepare_turn(&TurnSpec { conversation: Some("conv-1"), ..f.spec(AgentKind::Cursor) }).is_err());
        drop(turn);
        assert!(!token_file.exists(), "the turn's bearer token must not outlive it");
    }

    #[test]
    fn every_cli_turn_resumes_the_session_it_is_given() {
        for kind in [AgentKind::Claude, AgentKind::Codex, AgentKind::Cursor] {
            let f = TurnFixture::new();
            let turn = prepare_turn(&TurnSpec { resume: Some("prev-1"), ..f.spec(kind) }).unwrap();
            assert!(turn.cmd.args.iter().any(|a| a == "prev-1"), "{kind:?} must resume prev-1");
        }
    }

    /// Rust refuses to spawn a `.cmd`/`.bat` with a newline in argv, and cmd.exe
    /// would expand `%VAR%` in what it is given, so a batch shim's prompt
    /// travels on stdin; Claude's system prompt (it has newlines) by file.
    #[test]
    fn a_shim_turn_carries_a_hostile_prompt_only_on_stdin() {
        let p = "100% \"quoted\" & %PATH%\nsecond line";
        for (kind, shim) in [(AgentKind::Claude, r"x\claude.cmd"), (AgentKind::Codex, r"x\codex.cmd")] {
            let f = TurnFixture::new();
            let turn = prepare_turn(&TurnSpec { binary: shim, prompt: p, ..f.spec(kind) }).unwrap();
            assert_eq!(turn.cmd.stdin.as_deref(), Some(p), "{kind:?}");
            assert!(
                !turn.cmd.args.iter().any(|a| a.contains("PATH%") || a.contains('\n')),
                "{kind:?}: {:?}",
                turn.cmd.args
            );
            if kind == AgentKind::Claude {
                let sysprompt = f.root.0.join("srelens-sysprompt-s1.txt");
                assert!(turn.cmd.args.windows(2).any(|w| w[0] == "--append-system-prompt-file"
                    && std::path::Path::new(&w[1]) == sysprompt));
                assert_eq!(std::fs::read_to_string(&sysprompt).unwrap(), crate::adapter::BASE_SYSTEM_PROMPT);
                drop(turn);
                assert!(!sysprompt.exists(), "the system-prompt file goes with the turn");
            }

            // Not a shim: the prompt stays the trailing positional, stdin closed.
            let plain = prepare_turn(&TurnSpec { binary: "agent", prompt: p, ..f.spec(kind) }).unwrap();
            assert_eq!(plain.cmd.stdin, None, "{kind:?}");
            assert_eq!(plain.cmd.args.last().map(String::as_str), Some(p), "{kind:?}");
        }
    }

    /// Whether `codex exec … resume <thread> -` accepts a stdin prompt is
    /// unverified, so a Codex shim's follow-up starts a fresh thread. Claude's
    /// `--resume <id>` stays in argv (ids are bare tokens, `valid_resume_id`).
    #[test]
    fn a_codex_shim_turn_starts_fresh_while_a_claude_shim_turn_resumes() {
        let f = TurnFixture::new();
        let codex =
            prepare_turn(&TurnSpec { binary: r"x\codex.cmd", resume: Some("prev-1"), ..f.spec(AgentKind::Codex) })
                .unwrap();
        assert!(!codex.cmd.args.iter().any(|a| a == "resume" || a == "prev-1"), "{:?}", codex.cmd.args);
        let claude =
            prepare_turn(&TurnSpec { binary: r"x\claude.cmd", resume: Some("prev-1"), ..f.spec(AgentKind::Claude) })
                .unwrap();
        assert!(claude.cmd.args.windows(2).any(|w| w == ["--resume", "prev-1"]));
    }

    /// Codex's `resume` is a subcommand with no `-m`, so the model must sit
    /// ahead of it; an id that could pass for a flag is dropped, not passed on.
    #[test]
    fn a_turn_runs_on_its_model_ahead_of_resume_and_drops_one_that_could_be_a_flag() {
        for (kind, flag) in [(AgentKind::Claude, "--model"), (AgentKind::Cursor, "--model"), (AgentKind::Codex, "-m")] {
            let f = TurnFixture::new();
            let turn = prepare_turn(&TurnSpec { model: Some("m-1"), resume: Some("prev-1"), ..f.spec(kind) }).unwrap();
            assert_eq!(turn.cmd.args[1..3], [flag, "m-1"], "{kind:?}");
            if kind == AgentKind::Codex {
                let resume = turn.cmd.args.iter().position(|a| a == "resume").unwrap();
                assert!(2 < resume, "{:?}", turn.cmd.args);
            }
            drop(turn); // a Cursor turn holds its chat until it ends

            let bad = prepare_turn(&TurnSpec { model: Some("--not-a-model"), ..f.spec(kind) }).unwrap();
            assert!(!bad.cmd.args.iter().any(|a| a == flag || a == "--not-a-model"), "{kind:?}: {:?}", bad.cmd.args);
        }
    }

    #[test]
    fn the_native_agent_has_no_turn_to_prepare() {
        let f = TurnFixture::new();
        assert!(prepare_turn(&f.spec(AgentKind::Srelens)).is_err());
    }

    /// An npm-installed agent is a `.cmd` shim on Windows. Ending only the shim
    /// leaves the program it started running, spending tokens and holding the
    /// stdout pipe open, so a reader never sees the end of the stream (the
    /// desktop's Stop hung on it).
    #[cfg(windows)]
    #[tokio::test]
    async fn ending_a_shim_also_ends_the_program_it_started() {
        use std::time::Duration;
        use tokio::io::AsyncReadExt;
        let mut child = tokio::process::Command::new("cmd")
            .args(["/c", "ping", "-n", "30", "127.0.0.1"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let mut stdout = child.stdout.take().unwrap();
        // Its first line says ping is running: ending the shim before the
        // program exists would prove nothing.
        let mut first = [0u8; 64];
        let read = tokio::time::timeout(Duration::from_secs(5), stdout.read(&mut first)).await;
        assert!(read.expect("the shim started nothing").unwrap() > 0);

        kill_tree(&mut child);

        let mut rest = Vec::new();
        tokio::time::timeout(Duration::from_secs(5), stdout.read_to_end(&mut rest))
            .await
            .expect("the program the shim started still holds its stdout open")
            .unwrap();
        let _ = child.wait().await;
    }

    #[test]
    fn prompt_with_images_lists_each_path_then_a_blank_line_then_the_prompt() {
        let images = vec!["/tmp/a.png".to_string(), "/tmp/b.png".to_string()];
        let got = prompt_with_images("why?", &images);
        assert_eq!(got, "Attached image: /tmp/a.png\nAttached image: /tmp/b.png\n\nwhy?");
    }

    #[test]
    fn prompt_with_images_returns_the_prompt_unchanged_when_there_are_no_images() {
        let got = prompt_with_images("why?", &[]);
        assert_eq!(got, "why?");
    }
}
