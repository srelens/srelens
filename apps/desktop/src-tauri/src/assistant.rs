//! The in-app AI assistant: detect agent CLIs, spawn the chosen one against our
//! own MCP server, and stream normalized events to the WebView.

use std::sync::Arc;

use srelens_agent::adapter::{AgentInfo, AgentKind};
use srelens_agent::event::AgentEvent;
use srelens_agent::turn::{create_private_dir, write_private_file, TempDir, TempFile};
use srelens_streams::sink::EventSink;

const CLAUDE_INSTALL: &str = "https://docs.anthropic.com/en/docs/claude-code/setup";
const CODEX_INSTALL: &str = "https://developers.openai.com/codex/cli/";
const CURSOR_INSTALL: &str = "https://docs.cursor.com/en/cli/overview";

fn install_url(kind: AgentKind) -> &'static str {
    match kind {
        AgentKind::Claude => CLAUDE_INSTALL,
        AgentKind::Codex => CODEX_INSTALL,
        AgentKind::Cursor => CURSOR_INSTALL,
        // The native agent installs nothing — it needs an API key, not a CLI.
        AgentKind::Srelens => "",
    }
}

/// No agent is gated: Claude, Codex, and Cursor are each boxed to srelens's
/// own MCP tools (see `chat_send`'s per-kind spawn arms) and all three connect
/// and are selectable. Kept as a function (rather than deleted) so a future
/// agent that isn't ready can gate from here again.
fn is_gated(_kind: AgentKind) -> bool {
    false
}

/// Build an `AgentInfo`, resolving availability through the injected `resolve`
/// (real code passes a PATH lookup; tests pass a stub).
fn detect(kind: AgentKind, resolve: impl Fn(&str) -> Option<String>) -> AgentInfo {
    // The native agent has no binary to resolve; CLI kinds look theirs up.
    let path = kind.binary().and_then(&resolve);
    AgentInfo {
        kind,
        label: kind.label().to_string(),
        available: path.is_some(),
        path,
        version: None,
        install_url: install_url(kind).to_string(),
        gated: is_gated(kind),
    }
}

/// Enumerate every agent with its availability, for the picker: the three CLIs
/// (resolved on PATH) plus srelens's native agent (always listed, "available"
/// once a key is configured for the default provider).
#[tauri::command]
pub async fn agent_list(app: tauri::AppHandle) -> Result<Vec<AgentInfo>, String> {
    let paths = srelens_kube::toolbox::SearchPaths::from_env();
    let resolve = |bin: &str| resolve_agent(bin, &paths);
    Ok(vec![
        native_agent_info(&app),
        detect(AgentKind::Claude, resolve),
        detect(AgentKind::Codex, resolve),
        detect(AgentKind::Cursor, resolve),
    ])
}

/// The native agent's picker entry. It resolves no binary; it's "available"
/// once the default provider has an API key, otherwise it's shown but not
/// selectable (the composer points the user to Settings → Assistant).
fn native_agent_info(app: &tauri::AppHandle) -> AgentInfo {
    use tauri::Manager;
    let vault = app.state::<std::sync::Arc<crate::vault::Vault>>();
    let available = crate::llm_agent::llm_dir(app)
        .map(|dir| {
            let settings = crate::llm_config::load_settings(&dir.join("settings.json"));
            crate::llm_config::has_key(&vault, settings.default_provider)
        })
        .unwrap_or(false);
    AgentInfo {
        kind: AgentKind::Srelens,
        label: AgentKind::Srelens.label().to_string(),
        available,
        path: None,
        version: None,
        install_url: String::new(),
        gated: false,
    }
}

/// Locate an agent binary across both search paths: the app's own PATH first
/// (post `fix-path-env`, plus srelens's managed dirs), then broader system
/// locations the app doesn't search. This closes the prod gap where a CLI
/// installed under `~/.local/bin` — in `system_path` but not `app_path` — was
/// invisible to a packaged build even though a dev shell's inherited PATH
/// happened to cover it.
fn resolve_agent(bin: &str, paths: &srelens_kube::toolbox::SearchPaths) -> Option<String> {
    which_on_path(bin, &paths.app_path).or_else(|| which_on_path(bin, &paths.system_path))
}

/// File names to probe for `bin` within a single PATH directory. On Unix a CLI
/// is its bare name, but Windows resolves `claude` by appending each `PATHEXT`
/// extension (`.EXE`, `.CMD`, …) — an installed `claude.cmd`/`claude.exe` is
/// invisible if we only probe the extensionless path — so we return the bare
/// name followed by one candidate per extension. Kept pure (PATHEXT passed in,
/// `None` off Windows) so the extension logic is testable on any host. When the
/// caller already named a concrete extension we don't append more.
fn executable_candidates(bin: &str, pathext: Option<&str>) -> Vec<String> {
    let mut names = vec![bin.to_string()];
    if let Some(pathext) = pathext {
        if std::path::Path::new(bin).extension().is_none() {
            for ext in pathext.split(';').map(str::trim).filter(|e| !e.is_empty()) {
                // Each PATHEXT entry carries its own leading dot (".EXE").
                names.push(format!("{bin}{ext}"));
            }
        }
    }
    names
}

/// First directory on `path` (a platform-delimited PATH string) that holds an
/// executable `bin`, as an absolute path. Uses `std::env::split_paths` rather
/// than splitting on `:` so a Windows `;`-delimited PATH with drive-letter
/// colons (`C:\...;D:\...`) resolves correctly instead of shredding into
/// invalid directories, and honors `PATHEXT` so a `.exe`/`.cmd` install is found.
fn which_on_path(bin: &str, path: &str) -> Option<String> {
    #[cfg(windows)]
    let pathext =
        Some(std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string()));
    #[cfg(not(windows))]
    let pathext: Option<String> = None;

    std::env::split_paths(path).filter(|d| !d.as_os_str().is_empty()).find_map(|dir| {
        executable_candidates(bin, pathext.as_deref()).into_iter().find_map(|name| {
            let candidate = dir.join(&name);
            is_executable(&candidate).then(|| candidate.to_string_lossy().into_owned())
        })
    })
}

/// Whether `path` is a file we can actually run. On Unix a PATH entry only
/// counts if it carries an execute bit — a plain `0644` file named `claude`
/// sitting earlier on PATH would otherwise be reported as the installed agent,
/// and every send would then fail with `Permission denied`. On Windows
/// executability comes from the `PATHEXT` extension (handled by
/// `executable_candidates`), so being a regular file is sufficient.
fn is_executable(path: &std::path::Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// Map a sequence of raw stream-json lines to channel emits, returning whether
/// a `TurnDone` event was among them. Pure over the sink, so the parse→emit
/// path is unit-tested without spawning a process; `chat_send` also drives its
/// streaming loop through this so the tested contract is the one actually run.
/// `parse` is the agent-specific line parser (`claude::parse_line` or
/// `codex::parse_line`) — kept as a parameter rather than hardcoded so this
/// stays agent-agnostic and testable against either shape.
fn emit_events(
    sink: Arc<dyn EventSink>,
    channel: &str,
    lines: impl IntoIterator<Item = String>,
    parse: fn(&str) -> Vec<AgentEvent>,
) -> bool {
    let mut saw_done = false;
    for line in lines {
        for event in parse(&line) {
            saw_done |= matches!(event, AgentEvent::TurnDone);
            sink.emit(channel, serde_json::to_value(&event).unwrap());
        }
    }
    saw_done
}

/// Last non-empty line of a byte buffer, trimmed — used to turn a raw stderr
/// tail into a one-line error message instead of dumping a whole traceback
/// into the transcript.
fn last_line(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .unwrap_or("")
        .to_string()
}

/// Close out a turn whose stdout stream ended without a `TurnDone`: if the
/// child also exited non-zero, that's a crash, not a clean finish, so an
/// `Error` (carrying the stderr tail) is emitted before the synthetic
/// `TurnDone` that always re-enables the drawer's input. A stream that DID
/// see `TurnDone` needs nothing further — the agent already closed its own
/// turn. Pure over the sink, so the crash-reporting decision is unit-tested
/// without spawning a process.
fn finish_turn(sink: &dyn EventSink, channel: &str, saw_done: bool, crashed: bool, stderr_tail: &[u8]) {
    if saw_done {
        return;
    }
    if crashed {
        let tail = last_line(stderr_tail);
        let message = if tail.is_empty() {
            "the agent process exited unexpectedly".to_string()
        } else {
            tail
        };
        sink.emit(
            channel,
            serde_json::to_value(srelens_agent::event::AgentEvent::Error { message }).unwrap(),
        );
    }
    sink.emit(
        channel,
        serde_json::to_value(srelens_agent::event::AgentEvent::TurnDone).unwrap(),
    );
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

/// Decode one attached image's base64 body (no `data:image/...;base64,`
/// prefix — the WebView strips that before sending) into raw bytes. A thin
/// wrapper so `chat_send` can decode without a fallible call inline, and so
/// both directions (valid → bytes, invalid → error) are unit-tested without
/// touching the filesystem.
fn decode_base64_image(data: &str) -> Result<Vec<u8>, base64::DecodeError> {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    STANDARD.decode(data)
}

/// Tauri-managed state owning the running chat turns, keyed by session id, so
/// `chat_cancel` can find and kill the right child process.
#[derive(Default)]
pub struct ChatManager {
    children: std::sync::Mutex<std::collections::HashMap<String, tokio::process::Child>>,
    /// The native (in-process) agent has no child process to kill; instead its
    /// turn runs as a task whose `AbortHandle` is parked here so `chat_cancel`
    /// can stop it the same way it kills a CLI child.
    natives: std::sync::Mutex<std::collections::HashMap<String, tokio::task::AbortHandle>>,
    /// Sessions whose `chat_cancel` arrived while nothing cancellable was
    /// registered — a Stop can reach the backend before `chat_send` even
    /// starts (the frontend awaits channel subscription in between), or during
    /// `chat_send`'s prep (spawning the child, writing config/image files).
    /// The value is the turn generation the Stop was aimed at (the frontend's
    /// turn nonce), so `chat_send` can consume a Stop meant for ITS turn while
    /// dropping a stale one left over from a previous turn.
    pending_cancels: std::sync::Mutex<std::collections::HashMap<String, u64>>,
    pub(crate) cursor_chats: CursorChatsInUse,
}

/// The Cursor chats a turn or a delete is using right now. A chat's directory
/// is one per conversation (#952), and two windows can hold one saved
/// conversation, each streaming on its own session and seeing only its own
/// turns as busy. Without this, the second turn would empty the first one's
/// workspace and either turn's guard would remove the other's files.
#[derive(Default, Clone)]
pub struct CursorChatsInUse(Arc<std::sync::Mutex<std::collections::HashSet<String>>>);

/// One holder's use of a Cursor chat, released when dropped.
pub(crate) struct CursorChatClaim {
    in_use: CursorChatsInUse,
    chat: String,
}

impl CursorChatsInUse {
    /// Claim `chat`, or `None` while something else holds it.
    pub(crate) fn claim(&self, chat: &str) -> Option<CursorChatClaim> {
        let fresh = self.0.lock().unwrap().insert(chat.to_string());
        fresh.then(|| CursorChatClaim { in_use: self.clone(), chat: chat.to_string() })
    }
}

impl Drop for CursorChatClaim {
    fn drop(&mut self) {
        self.in_use.0.lock().unwrap().remove(&self.chat);
    }
}

impl ChatManager {
    /// Record that a cancel for the given turn generation arrived before
    /// anything cancellable was registered.
    pub fn arm_pending_cancel(&self, session: String, turn: u64) {
        self.pending_cancels.lock().unwrap().insert(session, turn);
    }

    /// Take a session's pending cancel — true only if one was armed for this
    /// turn generation. A mismatched (stale) entry is dropped, not honored, so
    /// a Stop that landed just after a previous turn ended can't kill this one.
    pub fn take_pending_cancel(&self, session: &str, turn: u64) -> bool {
        self.pending_cancels.lock().unwrap().remove(session) == Some(turn)
    }
}

/// Kill a child that has been taken OUT of `ChatManager` and reap it in the
/// background. `chat_send`'s cleanup only `wait()`s children still in the map,
/// and tokio reaps a dropped `Child` on a best-effort basis only — so without
/// this, every Stop could leave a zombie in the process table.
fn kill_and_reap(mut child: tokio::process::Child) {
    let _ = child.start_kill();
    tokio::spawn(async move {
        let _ = child.wait().await;
    });
}

impl ChatManager {
    /// Park a native turn's task handle so `chat_cancel` can abort it.
    pub fn register_native(&self, session: String, handle: tokio::task::AbortHandle) {
        self.natives.lock().unwrap().insert(session, handle);
    }

    /// Drop a finished native turn's handle (a no-op if `chat_cancel` already
    /// took it).
    pub fn unregister_native(&self, session: &str) {
        self.natives.lock().unwrap().remove(session);
    }
}

/// Begin a conversation. Returns a fresh session id; the WebView subscribes to
/// `chat://<id>` before its first `chat_send`.
#[tauri::command]
pub async fn chat_start() -> Result<String, String> {
    Ok(uuid::Uuid::new_v4().to_string())
}

/// One Cursor turn, ready to spawn: its command, and the workspace to spawn it
/// in, which is removed (bearer token and all) when the turn ends.
struct CursorTurn {
    cmd: srelens_agent::adapter::AgentCommand,
    workspace: std::path::PathBuf,
    _workspace_guard: TempDir,
    // Declared after the guard, so it is dropped after it: the chat is
    // released only once its workspace is gone.
    _claim: CursorChatClaim,
}

/// Where Cursor's chats live (see `prepare_cursor_turn`): the per-user data
/// dir, never the shared temp dir, and not the cache dir either — a saved
/// conversation's chat must outlive the run, and goes only with the chat
/// (`chat_history_delete`, #952).
pub(crate) fn cursor_state_dir(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    use tauri::Manager;
    Ok(app.path().app_local_data_dir().map_err(|e| e.to_string())?.join("cursor-chats"))
}

/// At launch, before any turn runs: see `sweep_cursor_chats`.
pub fn sweep_cursor_state(app: &tauri::AppHandle) {
    let dirs = cursor_state_dir(app).and_then(|root| Ok((root, crate::assistant_history::resolve_sessions_dir(app)?)));
    match dirs {
        Ok((root, sessions)) => sweep_cursor_chats(&root, |id| crate::assistant_history::is_saved(&sessions, id)),
        Err(error) => log::warn!("could not find the Cursor chats to sweep them: {error}"),
    }
}

/// Remove the Cursor chats no saved conversation holds — a one-shot turn's,
/// or one whose save never landed — and, from those that stay, the workspace
/// a killed turn left behind (its guard never ran). They hold prompts and
/// tool results, so a failure is logged: it is why they are still on disk.
fn sweep_cursor_chats(root: &std::path::Path, saved: impl Fn(&str) -> bool) {
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
        if let Err(error) = remove_cursor_chats(&stale) {
            log::warn!("could not delete {}: {error}", stale.display());
        }
    }
}

pub(crate) use srelens_agent::{cursor_chat_dir, next_resume_id, remove_cursor_chats};

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
fn prepare_cursor_turn(
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
    let dir = cursor_chat_dir(root, chat)?;
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
    std::fs::write(config.join("cli-config.json"), srelens_agent::adapter::cursor_cli_config_json())
        .map_err(|e| e.to_string())?;
    // `mcp.json` carries the bearer token. The workspace guard removes it with
    // the workspace when the turn ends, and the model can't read it back —
    // the deny-list blocks `Read`.
    write_private_file(
        &cursor_dir.join("mcp.json"),
        srelens_agent::adapter::cursor_mcp_json(url, token).as_bytes(),
    )
    .map_err(|e| e.to_string())?;
    let cmd = srelens_agent::adapter::cursor_command(
        agent_path,
        prompt,
        &config.to_string_lossy(),
        &workspace.to_string_lossy(),
        resume,
    );
    Ok(CursorTurn { cmd, workspace, _workspace_guard: workspace_guard, _claim: claim })
}

/// Parse the frontend's serialized `AgentKind` tag — the exact camelCase
/// string serde emits for the enum ("claude"/"codex"/"cursor") — back into
/// the enum. Errors on anything else rather than silently defaulting: an
/// unrecognized value is a frontend/backend skew bug, not something to paper
/// over by falling back to some default agent.
fn parse_agent_kind(kind: &str) -> Result<AgentKind, String> {
    match kind {
        "claude" => Ok(AgentKind::Claude),
        "codex" => Ok(AgentKind::Codex),
        "cursor" => Ok(AgentKind::Cursor),
        "srelens" => Ok(AgentKind::Srelens),
        _ => Err("unknown agent".to_string()),
    }
}

/// Send one user turn. Spawns the agent against our running MCP server and
/// streams `AgentEvent`s on `chat://<session>`.
///
/// `resume` is the agent CLI's OWN session id from a previous turn of this
/// conversation (what this function returned last time), passed to the CLI's
/// `--resume` so follow-up turns keep their context. Returns what the caller
/// should now store as the conversation's id (see `next_resume_id`): the id
/// captured from this turn's stream, the echoed `resume` when the turn ran
/// clean or was cancelled without yielding one, or `None` — meaning CLEAR —
/// for a crashed resume, so a dead session id can't wedge the conversation
/// in a retry loop. The native agent's id names its own kept history (see
/// `llm_agent::prior_history`).
///
/// `conversation` is the saved conversation's chat-history id, when the caller
/// has one. Cursor keeps its chat under it rather than under `session`, which
/// is new each time a conversation is reopened (#952).
#[tauri::command]
pub async fn chat_send(
    session: String,
    prompt: String,
    images: Vec<String>,
    agent_path: String,
    agent_kind: String,
    turn: Option<u64>,
    resume: Option<String>,
    conversation: Option<String>,
    app: tauri::AppHandle,
    mcp: tauri::State<'_, crate::mcp::McpHttpManager>,
    chats: tauri::State<'_, ChatManager>,
) -> Result<Option<String>, String> {
    // The turn generation the frontend stamped this send with; a Stop for the
    // same turn arms `pending_cancels` under this value. Optional so callers
    // that never cancel (e.g. one-shot generation turns) can omit it.
    let turn = turn.unwrap_or(0);
    use tauri::Manager;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};

    let kind = parse_agent_kind(&agent_kind)?;

    // The session id (from IPC) becomes a `chat://` channel AND temp-path
    // components (`srelens-agent-cwd-<id>`, config/image files). A crafted id
    // with `/`, `\`, or `..` would let `create_dir_all` — and `_cwd_guard`'s
    // later `remove_dir_all` — resolve OUTSIDE the temp dir, so validate it to
    // a bare component before it touches any path. Real ids are startChat uuids.
    if session.is_empty()
        || !session.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err(format!("invalid session id {session:?}"));
    }
    // The saved conversation's id names a directory too (Cursor's chat, see
    // `prepare_cursor_turn`). It is a chat-history id, so it is checked as one.
    if let Some(id) = &conversation {
        crate::assistant_history::validate_id(id)?;
    }

    // The resume id lands in the agent's argv (`--resume <id>`). Real ids are
    // CLI-minted uuids, but this one round-trips through the persisted
    // session file on disk — validate it to the same bare charset as the
    // session id above so a tampered store can't smuggle a flag-shaped or
    // whitespace-carrying token into the command line. Invalid → dropped
    // (fresh session), not an error: continuity is best-effort by design.
    let resume = resume.filter(|id| {
        !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    });

    // The native agent takes an entirely different path from the CLI spawns
    // below: no process, no PATH binary, no loopback HTTP. It runs the loop
    // in-process against a provider API and drives MCP tools directly through
    // the same consent/audit server the CLIs reach over HTTP. Handle it here so
    // the CLI machinery below only ever sees the three CLI kinds.
    if matches!(kind, AgentKind::Srelens) {
        // The native agent keeps its own conversation history, under the
        // saved conversation, and its id is the resume id it returns (see
        // `llm_agent::prior_history`).
        return crate::llm_agent::run_native_agent(app, &chats, session, conversation, resume, prompt, !images.is_empty(), turn)
            .await;
    }

    let channel = format!("chat://{session}");
    let sink: Arc<dyn EventSink> = Arc::new(crate::sink::TauriSink(app.clone()));

    // A Stop aimed at THIS turn can beat us here: the frontend awaits channel
    // subscription between its own cancel check and invoking `chat_send`, and
    // a `chat_cancel` in that window finds nothing registered and arms a
    // pending cancel. Honor it — don't launch — but still close the turn with
    // a `TurnDone`: the frontend only settles (and persists) the turn on a
    // terminal event, so a bare return would strand its empty placeholder. A
    // stale entry from a previous turn has a different generation and is
    // dropped by the same take.
    if chats.take_pending_cancel(&session, turn) {
        sink.emit(&channel, serde_json::to_value(AgentEvent::TurnDone).unwrap());
        // Echo the resume id back: nothing ran, the stored session is intact,
        // and the caller stores exactly what's returned — a bare `None` here
        // would read as "clear it" (the failed-resume signal from
        // `next_resume_id`) and needlessly drop continuity on a cancel.
        return Ok(resume);
    }

    // A token of this turn's own (#393), held to the end of this function so
    // it dies with the turn: a gate the CLI raises is recorded in THIS chat,
    // and an external client presenting the Settings token is never mistaken
    // for it.
    let turn_token = mcp.mint_turn_token(&session).await?;
    let token = turn_token.token().to_string();
    let url = mcp.status_url().ok_or("MCP server URL unavailable")?;

    let dir = app.path().temp_dir().map_err(|e| e.to_string())?;

    // A fresh, empty scratch directory for the agent's CWD — never the
    // srelens process's own working directory (the user's repo or home) —
    // so a tool that tried to enumerate its surroundings would find nothing.
    // Removed on every exit from this function. This is also Codex's `-C`
    // workspace (see `codex_command`): genuinely empty, per-turn, and torn
    // down here — that emptiness is the whole box, so this guard must never
    // be pointed at anything else. Cursor's is its workspace, laid out with
    // the rest of its turn by `prepare_cursor_turn`, which guards it.
    let cursor_turn = match kind {
        AgentKind::Cursor => Some(prepare_cursor_turn(
            &cursor_state_dir(&app)?,
            &chats.cursor_chats,
            &session,
            conversation.as_deref(),
            &agent_path,
            &prompt,
            &url,
            &token,
            resume.as_deref(),
        )?),
        _ => None,
    };
    let cwd_path = match &cursor_turn {
        Some(turn) => turn.workspace.clone(),
        None => create_private_dir(&dir, &format!("srelens-agent-cwd-{session}"))?,
    };
    let _cwd_guard = cursor_turn.is_none().then(|| TempDir(cwd_path.clone()));

    // Decode each attached image to its own temp file under the same dir,
    // guarded by a `TempFile` per image so every one is cleaned up on any
    // exit from this function. A decode or write failure is reported as an
    // `Error` event and that image is skipped rather than aborting the whole
    // turn. Only CODEX can actually use an attached image — it takes image
    // files natively via `-i`. Claude and Cursor CANNOT: both have their
    // file-read tool denied by the sandbox (Claude's `Read` is in
    // `DISALLOWED_TOOLS`, Cursor's by `cursor_cli_config_json`), so a path
    // folded into the prompt is unreadable dead text. A Claude/Cursor turn with
    // attachments emits one note and proceeds text-only instead.
    let mut image_guards: Vec<TempFile> = Vec::new();
    let mut image_paths: Vec<String> = Vec::new();
    if !matches!(kind, AgentKind::Codex) {
        if !images.is_empty() {
            sink.emit(
                &channel,
                serde_json::to_value(AgentEvent::Error {
                    message: "image attachments are only supported with the Codex agent".to_string(),
                })
                .unwrap(),
            );
        }
    } else {
        for (i, data) in images.iter().enumerate() {
            let bytes = match decode_base64_image(data) {
                Ok(bytes) => bytes,
                Err(e) => {
                    sink.emit(
                        &channel,
                        serde_json::to_value(AgentEvent::Error {
                            message: format!("could not decode attached image {i}: {e}"),
                        })
                        .unwrap(),
                    );
                    continue;
                }
            };
            let image_path = dir.join(format!("srelens-img-{session}-{i}.png"));
            // Guard the path before writing to it, not after: a `write` that
            // fails partway through still leaves a (possibly partial) file on
            // disk, and only a guard pushed before the call runs its `Drop` to
            // clean that up. Pushing after `write` would only ever guard a fully
            // successful write, leaking a partial file on the error path below.
            image_guards.push(TempFile(image_path.clone()));
            // Owner-only: the path is exposed in Codex's `-i` argv, so another
            // local account could otherwise read the attachment from the shared
            // temp dir.
            if let Err(e) = write_private_file(&image_path, &bytes) {
                sink.emit(
                    &channel,
                    serde_json::to_value(AgentEvent::Error {
                        message: format!("could not save attached image {i}: {e}"),
                    })
                    .unwrap(),
                );
                continue;
            }
            image_paths.push(image_path.to_string_lossy().into_owned());
        }
    }

    // Only Claude reads its MCP server config from a file (`--mcp-config`);
    // Codex gets the same server via `-c` TOML overrides plus its bearer
    // token in `env` (see `codex_command`), so no config file is written for
    // it at all. `_cfg_guard` is declared here, outside the match, so its
    // `Drop` (removing the cleartext-token file) still runs at the end of
    // this function on every exit, exactly like `_cwd_guard` above — a guard
    // built and dropped inside the match arm itself would be gone before the
    // agent ever read the file. Cursor's files are `cursor_turn`'s, above.
    let mut _cfg_guard: Option<TempFile> = None;
    let cmd = match kind {
        AgentKind::Claude => {
            let cfg = srelens_agent::adapter::McpConfig::http(&url, &token);
            let cfg_path = dir.join(format!("srelens-mcp-{session}.json"));
            write_private_file(&cfg_path, &serde_json::to_vec(&cfg).unwrap()).map_err(|e| e.to_string())?;
            _cfg_guard = Some(TempFile(cfg_path.clone()));
            let effective_prompt = prompt_with_images(&prompt, &image_paths);
            srelens_agent::adapter::claude_command(
                &agent_path,
                &effective_prompt,
                &cfg_path.to_string_lossy(),
                resume.as_deref(),
            )
        }
        // A follow-up resumes the conversation's Codex thread with the whole
        // box re-asserted in front of `resume` (see `codex_command`), in this
        // turn's own fresh empty `-C` workspace: the thread is found by id in
        // `CODEX_HOME`, whatever directory it was started in (#215).
        AgentKind::Codex => srelens_agent::adapter::codex_command(
            &agent_path,
            &prompt,
            &url,
            &token,
            &cwd_path.to_string_lossy(),
            &image_paths,
            resume.as_deref(),
        ),
        AgentKind::Cursor => cursor_turn
            .as_ref()
            .map(|turn| turn.cmd.clone())
            .expect("a Cursor turn is prepared before its command is needed"),
        // Handled by the early return above, before any CLI machinery.
        AgentKind::Srelens => unreachable!("native agent dispatched before the CLI path"),
    };

    let parse_line: fn(&str) -> Vec<AgentEvent> = match kind {
        AgentKind::Claude => srelens_agent::claude::parse_line,
        AgentKind::Codex => srelens_agent::codex::parse_line,
        AgentKind::Cursor => srelens_agent::cursor::parse_line,
        AgentKind::Srelens => unreachable!("native agent dispatched before the CLI path"),
    };

    let mut child = tokio::process::Command::new(&cmd.program)
        .args(&cmd.args)
        // Required so Codex's MCP bearer token (carried in `cmd.env`, never
        // argv) actually reaches the child; harmless for Claude, whose `env`
        // is always empty (its token travels via the `--mcp-config` file).
        .envs(cmd.env.iter().map(|(k, v)| (k.clone(), v.clone())))
        .current_dir(&cwd_path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start the agent: {e}"))?;

    // `Stdio::piped()` above guarantees this is `Some`; still handled rather
    // than unwrapped so a future refactor that drops the `piped()` call
    // degrades to a clean turn-ending error instead of a silently hung UI —
    // every post-spawn exit re-enables input via a `TurnDone`.
    let Some(stdout) = child.stdout.take() else {
        let _ = child.start_kill();
        let message = "the agent process had no stdout".to_string();
        sink.emit(
            &channel,
            serde_json::to_value(AgentEvent::Error { message: message.clone() }).unwrap(),
        );
        sink.emit(&channel, serde_json::to_value(AgentEvent::TurnDone).unwrap());
        return Err(message);
    };

    // Drain stderr concurrently with the stdout loop below: it's piped but
    // otherwise unread, so once the agent writes past the OS pipe buffer it
    // would block on write() and wedge stdout along with it. Keep only a
    // bounded tail, surfaced as an `error` event below if the child exits
    // non-zero.
    let stderr_task = child.stderr.take().map(|mut stderr| {
        tokio::spawn(async move {
            const TAIL_CAP: usize = 8 * 1024;
            let mut tail: Vec<u8> = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                match stderr.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        tail.extend_from_slice(&buf[..n]);
                        if tail.len() > TAIL_CAP {
                            let excess = tail.len() - TAIL_CAP;
                            tail.drain(0..excess);
                        }
                    }
                }
            }
            tail
        })
    });

    chats.children.lock().unwrap().insert(session.clone(), child);

    // Honor a Stop that landed while we were preparing (before the child was
    // registered, so `chat_cancel` armed a pending flag instead of killing
    // anything): now that the child exists, kill it. The stream loop below then
    // reads EOF and `finish_turn` emits the closing `TurnDone`.
    if chats.take_pending_cancel(&session, turn) {
        if let Some(child) = chats.children.lock().unwrap().remove(&session) {
            kill_and_reap(child);
        }
    }

    let mut lines = BufReader::new(stdout).lines();
    let mut saw_done = false;
    // The CLI's own session id for this turn, captured from the stream so the
    // NEXT turn can resume it. Claude and Cursor stamp it on every line;
    // Codex names its thread on the first (`thread.started`). Captured fresh
    // each turn rather than reusing `resume`: Claude forks a resumed session
    // under a NEW id, so the stored id must follow the stream, not the input.
    let mut agent_session: Option<String> = None;
    while let Ok(Some(line)) = lines.next_line().await {
        if agent_session.is_none() {
            agent_session = srelens_agent::stream_session_id(&line);
        }
        saw_done |= emit_events(sink.clone(), &channel, std::iter::once(line), parse_line);
    }

    // Taken out of the map either here, or already by `chat_cancel` (a
    // user-initiated kill, which is not a crash worth reporting below).
    let removed_child = chats.children.lock().unwrap().remove(&session);

    let stderr_tail = match stderr_task {
        Some(task) => task.await.unwrap_or_default(),
        None => Vec::new(),
    };

    // Wait for the actual exit and reap the process — a well-behaved agent
    // that closes stdout just before exiting would otherwise leave a zombie —
    // and use the status to tell a crash from a clean finish.
    let crashed = match removed_child {
        Some(mut child) => !child.wait().await.map(|s| s.success()).unwrap_or(false),
        None => false,
    };

    finish_turn(sink.as_ref(), &channel, saw_done, crashed, &stderr_tail);

    Ok(next_resume_id(agent_session, resume, crashed))
}

/// Kill a running turn's agent process, if any. `turn` is the frontend's turn
/// generation for the send being stopped, so a cancel that lands before that
/// send reaches the backend is honored by it — and only by it.
#[tauri::command]
pub async fn chat_cancel(
    session: String,
    turn: Option<u64>,
    chats: tauri::State<'_, ChatManager>,
) -> Result<(), String> {
    let mut stopped = false;
    if let Some(child) = chats.children.lock().unwrap().remove(&session) {
        kill_and_reap(child);
        stopped = true;
    }
    // The native agent's turn is a task, not a child process.
    if let Some(handle) = chats.natives.lock().unwrap().remove(&session) {
        handle.abort();
        stopped = true;
    }
    // Nothing running yet — the turn hasn't reached the backend or is mid-
    // preparation. Arm a pending cancel under this turn's generation so
    // `chat_send` for the same turn stops at entry or the moment the
    // child/task is registered, instead of this Stop being lost.
    if !stopped {
        chats.arm_pending_cancel(session, turn.unwrap_or(0));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_captured_stream_id_always_wins() {
        assert_eq!(
            next_resume_id(Some("new".into()), Some("old".into()), true),
            Some("new".into())
        );
        assert_eq!(next_resume_id(Some("new".into()), None, false), Some("new".into()));
    }

    #[test]
    fn a_clean_turn_without_an_id_echoes_the_resume_back() {
        // Shape drift or a cancel (never `crashed` — chat_cancel reaps the
        // child): the stored session is intact, keep it.
        assert_eq!(next_resume_id(None, Some("old".into()), false), Some("old".into()));
        assert_eq!(next_resume_id(None, None, false), None);
    }

    #[test]
    fn a_crash_without_an_id_clears_the_resume() {
        // The signature of `--resume` itself failing (session GC'd/unknown):
        // echoing "old" back would retry the same failing resume forever.
        assert_eq!(next_resume_id(None, Some("old".into()), true), None);
        assert_eq!(next_resume_id(None, None, true), None);
    }

    #[test]
    fn a_found_binary_is_available_with_its_path() {
        let info = detect(AgentKind::Claude, |_| Some("/usr/bin/claude".into()));
        assert!(info.available);
        assert_eq!(info.path.as_deref(), Some("/usr/bin/claude"));
    }

    #[test]
    fn a_missing_binary_is_unavailable_with_an_install_link() {
        let info = detect(AgentKind::Codex, |_| None);
        assert!(!info.available);
        assert!(info.path.is_none());
        assert!(info.install_url.contains("codex"));
    }

    #[test]
    fn no_agent_is_gated_claude_codex_and_cursor_are_all_selectable() {
        let info = detect(AgentKind::Cursor, |_| Some("/usr/bin/cursor-agent".into()));
        assert!(info.available);
        assert!(!info.gated);

        let info = detect(AgentKind::Claude, |_| Some("/usr/bin/claude".into()));
        assert!(info.available);
        assert!(!info.gated);

        let info = detect(AgentKind::Codex, |_| Some("/usr/bin/codex".into()));
        assert!(info.available);
        assert!(!info.gated);
    }

    #[test]
    fn parse_agent_kind_maps_each_serialized_tag_to_its_enum_variant() {
        assert_eq!(parse_agent_kind("claude"), Ok(AgentKind::Claude));
        assert_eq!(parse_agent_kind("codex"), Ok(AgentKind::Codex));
        assert_eq!(parse_agent_kind("cursor"), Ok(AgentKind::Cursor));
        assert_eq!(parse_agent_kind("srelens"), Ok(AgentKind::Srelens));
    }

    #[test]
    fn off_windows_a_binary_is_probed_by_its_bare_name_only() {
        assert_eq!(executable_candidates("claude", None), vec!["claude".to_string()]);
    }

    #[test]
    fn on_windows_pathext_extensions_are_appended_to_the_bare_name() {
        assert_eq!(
            executable_candidates("cursor-agent", Some(".COM;.EXE;.CMD")),
            vec![
                "cursor-agent".to_string(),
                "cursor-agent.COM".to_string(),
                "cursor-agent.EXE".to_string(),
                "cursor-agent.CMD".to_string(),
            ]
        );
    }

    #[test]
    fn an_explicit_extension_is_not_doubled_up() {
        // A bin that already names its extension resolves as-is.
        assert_eq!(executable_candidates("claude.exe", Some(".EXE;.CMD")), vec![
            "claude.exe".to_string()
        ]);
    }

    #[cfg(unix)]
    #[test]
    fn a_non_executable_file_on_path_is_not_treated_as_an_agent() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("srelens-exec-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // A plain 0644 file named `claude` must NOT be reported as the agent —
        // spawning it would fail with EACCES.
        let plain = dir.join("claude");
        std::fs::write(&plain, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(which_on_path("claude", dir.to_str().unwrap()), None);
        // Once it carries an execute bit, it resolves.
        std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(which_on_path("claude", dir.to_str().unwrap()).as_deref(), plain.to_str());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_pending_cancel_is_consumed_only_by_its_own_turn_generation() {
        let chats = ChatManager::default();
        assert!(!chats.take_pending_cancel("s1", 1), "nothing armed yet");
        // A Stop during turn 1's prep arms it; turn 1 consumes it exactly once.
        chats.arm_pending_cancel("s1".into(), 1);
        assert!(chats.take_pending_cancel("s1", 1));
        assert!(!chats.take_pending_cancel("s1", 1), "consumed on first take");
        // A stale Stop aimed at turn 1 (it landed after that turn had already
        // finished) must not kill turn 2 — and taking it also drops it.
        chats.arm_pending_cancel("s2".into(), 1);
        assert!(!chats.take_pending_cancel("s2", 2));
        assert!(!chats.take_pending_cancel("s2", 1), "mismatch drops the entry");
    }

    #[test]
    fn parse_agent_kind_rejects_anything_else() {
        assert_eq!(parse_agent_kind("gpt5"), Err("unknown agent".to_string()));
        assert_eq!(parse_agent_kind(""), Err("unknown agent".to_string()));
        assert_eq!(parse_agent_kind("Claude"), Err("unknown agent".to_string()));
    }

    /// Builds a temp dir under `std::env::temp_dir()` with a single executable
    /// file `bin`, cleaned up on drop — the fixture `resolve_agent` tests use
    /// to prove the search actually walks `system_path`, not just `app_path`.
    struct BinDir {
        dir: std::path::PathBuf,
    }

    impl BinDir {
        fn new(bin: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("srelens-resolve-agent-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            let file = dir.join(bin);
            std::fs::write(&file, b"#!/bin/sh\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = std::fs::metadata(&file).unwrap().permissions();
                perms.set_mode(0o755);
                std::fs::set_permissions(&file, perms).unwrap();
            }
            BinDir { dir }
        }

        fn path(&self) -> String {
            self.dir.to_string_lossy().into_owned()
        }
    }

    impl Drop for BinDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn resolve_agent_finds_a_binary_only_present_on_the_system_path() {
        let bin_dir = BinDir::new("x");
        let paths = srelens_kube::toolbox::SearchPaths {
            app_path: "/nope".to_string(),
            system_path: bin_dir.path(),
        };
        let resolved = resolve_agent("x", &paths).expect("should find x via system_path");
        assert_eq!(resolved, bin_dir.dir.join("x").to_string_lossy());
    }

    #[test]
    fn resolve_agent_returns_none_when_absent_from_both_paths() {
        let paths = srelens_kube::toolbox::SearchPaths {
            app_path: "/nope".to_string(),
            system_path: "/also/nope".to_string(),
        };
        assert!(resolve_agent("x", &paths).is_none());
    }

    #[test]
    fn resolve_agent_prefers_app_path_when_present_in_both() {
        let app_dir = BinDir::new("x");
        let system_dir = BinDir::new("x");
        let paths = srelens_kube::toolbox::SearchPaths {
            app_path: app_dir.path(),
            system_path: system_dir.path(),
        };
        let resolved = resolve_agent("x", &paths).unwrap();
        assert_eq!(resolved, app_dir.dir.join("x").to_string_lossy());
    }

    struct RecordingSink(std::sync::Mutex<Vec<(String, serde_json::Value)>>);
    impl srelens_streams::sink::EventSink for RecordingSink {
        fn emit(&self, channel: &str, payload: serde_json::Value) {
            self.0.lock().unwrap().push((channel.to_string(), payload));
        }
    }

    #[test]
    fn each_parsed_event_is_emitted_on_the_session_channel() {
        let sink = std::sync::Arc::new(RecordingSink(Default::default()));
        let lines = [
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hi"}]}}"#,
            r#"{"type":"result","subtype":"success"}"#,
        ];
        emit_events(
            sink.clone(),
            "chat://s1",
            lines.iter().map(|s| s.to_string()),
            srelens_agent::claude::parse_line,
        );
        let got = sink.0.lock().unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].0, "chat://s1");
        assert_eq!(got[0].1["type"], "textDelta");
        assert_eq!(got[1].1["type"], "turnDone");
    }

    /// Same contract as `each_parsed_event_is_emitted_on_the_session_channel`
    /// above, but driven through Codex's own JSONL shape and parser — proves
    /// `emit_events` is genuinely agent-agnostic, not accidentally coupled to
    /// Claude's line format via a hardcoded parser.
    #[test]
    fn emit_events_with_the_codex_parser_emits_a_text_delta_then_turn_done() {
        let sink = std::sync::Arc::new(RecordingSink(Default::default()));
        let lines = [
            r#"{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"hi"}}"#,
            r#"{"type":"turn.completed","usage":{}}"#,
        ];
        emit_events(
            sink.clone(),
            "chat://s1",
            lines.iter().map(|s| s.to_string()),
            srelens_agent::codex::parse_line,
        );
        let got = sink.0.lock().unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].0, "chat://s1");
        assert_eq!(got[0].1["type"], "textDelta");
        assert_eq!(got[0].1["text"], "hi");
        assert_eq!(got[1].1["type"], "turnDone");
    }

    /// Same contract again, driven through Cursor's stream-json shape and
    /// parser — proves `emit_events` handles all three agent line formats,
    /// not just Claude's and Codex's.
    #[test]
    fn emit_events_with_the_cursor_parser_emits_a_text_delta_then_turn_done() {
        let sink = std::sync::Arc::new(RecordingSink(Default::default()));
        let lines = [
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"hi"}]}}"#,
            r#"{"type":"result","subtype":"success","is_error":false}"#,
        ];
        emit_events(
            sink.clone(),
            "chat://s1",
            lines.iter().map(|s| s.to_string()),
            srelens_agent::cursor::parse_line,
        );
        let got = sink.0.lock().unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].0, "chat://s1");
        assert_eq!(got[0].1["type"], "textDelta");
        assert_eq!(got[0].1["text"], "hi");
        assert_eq!(got[1].1["type"], "turnDone");
    }

    #[test]
    fn a_clean_turn_that_already_saw_turn_done_emits_nothing_further() {
        let sink = std::sync::Arc::new(RecordingSink(Default::default()));
        finish_turn(sink.as_ref(), "chat://s1", true, true, b"ignored, moot: saw_done wins");
        assert!(sink.0.lock().unwrap().is_empty());
    }

    #[test]
    fn a_stream_that_ends_cleanly_without_turn_done_gets_only_a_synthetic_one() {
        let sink = std::sync::Arc::new(RecordingSink(Default::default()));
        finish_turn(sink.as_ref(), "chat://s1", false, false, b"");
        let got = sink.0.lock().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].1["type"], "turnDone");
    }

    #[test]
    fn a_crash_emits_an_error_with_the_stderr_tail_before_the_synthetic_turn_done() {
        let sink = std::sync::Arc::new(RecordingSink(Default::default()));
        finish_turn(
            sink.as_ref(),
            "chat://s1",
            false,
            true,
            b"Traceback (most recent call last)\nRuntimeError: boom\n",
        );
        let got = sink.0.lock().unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].1["type"], "error");
        assert_eq!(got[0].1["message"], "RuntimeError: boom");
        assert_eq!(got[1].1["type"], "turnDone");
    }

    #[test]
    fn a_crash_with_no_stderr_still_gets_a_generic_error_message() {
        let sink = std::sync::Arc::new(RecordingSink(Default::default()));
        finish_turn(sink.as_ref(), "chat://s1", false, true, b"");
        let got = sink.0.lock().unwrap();
        assert_eq!(got[0].1["type"], "error");
        assert_eq!(got[0].1["message"], "the agent process exited unexpectedly");
    }

    #[test]
    fn last_line_trims_and_skips_trailing_blank_lines() {
        assert_eq!(last_line(b"first\nsecond\n\n"), "second");
        assert_eq!(last_line(b"  only  \n"), "only");
        assert_eq!(last_line(b""), "");
    }

    /// Where cursor-agent keeps a chat: `$CURSOR_CONFIG_DIR/chats/<md5 of the
    /// absolute --workspace>/<chatId>/store.db`, per its bundled `paths.js`.
    /// Read from the command a turn would run. Any function of the workspace
    /// path stands in for the md5: a workspace that moves moves the store.
    fn cursor_chat_store(cmd: &srelens_agent::adapter::AgentCommand, chat: &str) -> std::path::PathBuf {
        let at = cmd.args.iter().position(|a| a == "--workspace").expect("a Cursor turn passes --workspace");
        let hash: String =
            cmd.args[at + 1].chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
        cursor_config_dir(cmd).join("chats").join(hash).join(chat).join("store.db")
    }

    fn cursor_config_dir(cmd: &srelens_agent::adapter::AgentCommand) -> std::path::PathBuf {
        let (_, dir) = cmd
            .env
            .iter()
            .find(|(k, _)| k == srelens_agent::adapter::CURSOR_CONFIG_DIR_ENV)
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

    #[test]
    fn decode_base64_image_recovers_the_original_bytes() {
        // "hello" base64-encoded, hand-written rather than produced by encoding
        // "hello" ourselves in the test.
        let bytes = decode_base64_image("aGVsbG8=").expect("valid base64");
        assert_eq!(bytes, b"hello");
    }

    #[test]
    fn decode_base64_image_rejects_invalid_base64() {
        assert!(decode_base64_image("not-valid-base64!!!").is_err());
    }

    #[test]
    fn a_decoded_image_written_to_a_temp_file_reads_back_byte_identical() {
        let bytes = decode_base64_image("aGVsbG8=").expect("valid base64");
        let path = std::env::temp_dir().join(format!("srelens-assistant-image-test-{}", uuid::Uuid::new_v4()));
        std::fs::write(&path, &bytes).unwrap();
        let read_back = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(read_back, bytes);
        assert_eq!(read_back, b"hello");
    }

    /// The assistant must not stand up its own MCP server or policy — that would
    /// be a consent bypass. It only borrows the running server's token/url, so a
    /// gated call still hits the GUI's PromptUser. This guards against a refactor
    /// that gives the assistant its own server.
    #[test]
    fn the_assistant_module_wires_no_confirm_policy_of_its_own() {
        let src = include_str!("assistant.rs");
        let needle1 = ["McpServer", "::", "new"].join("");
        let needle2 = ["Flag", "Gated"].join("");
        let needle3 = ["Always", "Allow"].join("");
        assert!(!src.contains(&needle1), "assistant must reuse the running server");
        assert!(!src.contains(&needle2), "assistant must not introduce a headless policy");
        assert!(!src.contains(&needle3), "assistant must never bypass consent");
    }
}
