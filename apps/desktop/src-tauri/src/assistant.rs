//! The in-app AI assistant: detect agent CLIs, spawn the chosen one against our
//! own MCP server, and stream normalized events to the WebView.

use std::sync::Arc;

use srelens_agent::adapter::{AgentInfo, AgentKind};
use srelens_agent::event::AgentEvent;
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
    let available = crate::llm_agent::llm_dir(app)
        .map(|dir| {
            let settings = crate::llm_config::load_settings(&dir.join("settings.json"));
            crate::llm_config::has_key(&dir, settings.default_provider)
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

/// Removes the wrapped path when dropped. The MCP config file `chat_send`
/// writes for a turn carries the bearer token in cleartext, so it must be
/// removed on every exit — including an early `?` after a failed spawn — not
/// just the happy path a bare `remove_file` at the end of the function would
/// cover.
struct TempFile(std::path::PathBuf);

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
struct TempDir(std::path::PathBuf);

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
fn create_private_dir(base: &std::path::Path, prefix: &str) -> Result<std::path::PathBuf, String> {
    for _ in 0..8 {
        let path = base.join(format!("{prefix}-{}", uuid::Uuid::new_v4()));
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
fn write_private_file(path: &std::path::Path, contents: &[u8]) -> std::io::Result<()> {
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
#[tauri::command]
pub async fn chat_send(
    session: String,
    prompt: String,
    images: Vec<String>,
    agent_path: String,
    agent_kind: String,
    turn: Option<u64>,
    app: tauri::AppHandle,
    mcp: tauri::State<'_, crate::mcp::McpHttpManager>,
    chats: tauri::State<'_, ChatManager>,
) -> Result<(), String> {
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

    // The native agent takes an entirely different path from the CLI spawns
    // below: no process, no PATH binary, no loopback HTTP. It runs the loop
    // in-process against a provider API and drives MCP tools directly through
    // the same consent/audit server the CLIs reach over HTTP. Handle it here so
    // the CLI machinery below only ever sees the three CLI kinds.
    if matches!(kind, AgentKind::Srelens) {
        return crate::llm_agent::run_native_agent(app, &chats, session, prompt, !images.is_empty(), turn).await;
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
        return Ok(());
    }

    let token = mcp
        .session_token()
        .ok_or("Start the MCP server in Settings → MCP before using the assistant.")?;
    let url = mcp.status_url().ok_or("MCP server URL unavailable")?;

    let dir = app.path().temp_dir().map_err(|e| e.to_string())?;

    // A fresh, empty scratch directory for the agent's CWD — never the
    // srelens process's own working directory (the user's repo or home) —
    // so a tool that tried to enumerate its surroundings would find nothing.
    // Removed on every exit from this function. This is also Codex's `-C`
    // workspace (see `codex_command`): genuinely empty, per-turn, and torn
    // down here — that emptiness is the whole box, so this guard must never
    // be pointed at anything else.
    let cwd_path = create_private_dir(&dir, &format!("srelens-agent-cwd-{session}"))?;
    let _cwd_guard = TempDir(cwd_path.clone());

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
    // agent ever read the file. `_cursor_cfg_dir_guard` is Cursor's
    // equivalent: unlike Claude's single file, Cursor reads a whole config
    // *directory* (`cli-config.json` + `mcp.json`, the latter carrying the
    // bearer token) pointed to by `CURSOR_CONFIG_DIR`, so it needs a `TempDir`
    // guard rather than a `TempFile` one — same reasoning, same lifecycle.
    let mut _cfg_guard: Option<TempFile> = None;
    let mut _cursor_cfg_dir_guard: Option<TempDir> = None;
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
                None,
            )
        }
        AgentKind::Codex => srelens_agent::adapter::codex_command(
            &agent_path,
            &prompt,
            &url,
            &token,
            &cwd_path.to_string_lossy(),
            &image_paths,
        ),
        AgentKind::Cursor => {
            // Cursor loads permissions and MCP servers from DIFFERENT places:
            // the deny-list is read from `CURSOR_CONFIG_DIR/cli-config.json`,
            // but MCP servers are read from the WORKSPACE's `.cursor/mcp.json`
            // (project config), NOT from `CURSOR_CONFIG_DIR` (verified live:
            // an mcp.json in CURSOR_CONFIG_DIR is silently ignored, so the
            // server never connects). So the deny-list goes in the config dir
            // and the MCP server goes in `<workspace>/.cursor/mcp.json`.
            let cursor_cfg_dir = create_private_dir(&dir, &format!("srelens-cursor-cfg-{session}"))?;
            _cursor_cfg_dir_guard = Some(TempDir(cursor_cfg_dir.clone()));
            std::fs::write(
                cursor_cfg_dir.join("cli-config.json"),
                srelens_agent::adapter::cursor_cli_config_json(),
            )
            .map_err(|e| e.to_string())?;
            // `mcp.json` (carrying the bearer token) goes under the per-turn
            // empty workspace's `.cursor/`; the `_cwd_guard` TempDir removes
            // the whole workspace (and this token file) on every exit. The
            // model can't read it back — the deny-list blocks `Read`.
            let cursor_dir = cwd_path.join(".cursor");
            std::fs::create_dir_all(&cursor_dir).map_err(|e| e.to_string())?;
            write_private_file(
                &cursor_dir.join("mcp.json"),
                srelens_agent::adapter::cursor_mcp_json(&url, &token).as_bytes(),
            )
            .map_err(|e| e.to_string())?;
            srelens_agent::adapter::cursor_command(
                &agent_path,
                &prompt,
                &cursor_cfg_dir.to_string_lossy(),
                &cwd_path.to_string_lossy(),
                None,
            )
        }
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
    while let Ok(Some(line)) = lines.next_line().await {
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

    Ok(())
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
