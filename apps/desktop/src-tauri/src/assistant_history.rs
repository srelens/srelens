//! Disk-backed chat-session store for the in-app AI assistant.
//!
//! Sessions live one-per-file under `<config>/assistant/sessions/<id>.json`,
//! with a small `index.json` alongside carrying just the picker metadata
//! (`SessionMeta`) so `chat_history_list` doesn't need to read and parse
//! every full session — `messages` can grow arbitrarily large — just to
//! render a list of titles.
//!
//! The `#[tauri::command]` wrappers at the bottom only resolve the app
//! config dir and delegate; every real decision lives in the pure `fn`s
//! above them, which take a `base`/`dir: &Path` so tests can drive them
//! against a throwaway temp directory without touching the real app config.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

/// A full chat session, including its message transcript.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default)]
    pub contexts: Vec<String>,
    /// Active skill names (Task 23). Defaulted so session files written
    /// before this field existed still deserialize.
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub cli_session_id: Option<String>,
    /// The agent CLI this conversation used (e.g. "claude"/"codex"), so
    /// reopening it restores the same agent in the picker. Defaulted for
    /// session files written before this field existed.
    #[serde(default)]
    pub agent_kind: Option<String>,
    /// Opaque to the backend — the frontend owns the message shape. Stored
    /// and returned verbatim.
    pub messages: Vec<serde_json::Value>,
}

/// Lightweight metadata for the session picker — everything but `messages`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionMeta {
    pub id: String,
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// `<base>/assistant/sessions` — where session files and `index.json` live.
fn sessions_dir(base: &Path) -> PathBuf {
    base.join("assistant").join("sessions")
}

fn index_path(dir: &Path) -> PathBuf {
    dir.join("index.json")
}

/// Reject a session id that isn't a bare `[A-Za-z0-9._-]+` filename component,
/// so an id arriving over IPC (or read back from a crafted `index.json` entry)
/// can never resolve outside the sessions directory via `/`, `\`, or an
/// absolute path — the same guard `assistant_skills::validate_name` gives
/// skill names. Real ids are `startChat`-minted uuids, which pass.
fn validate_id(id: &str) -> Result<(), String> {
    let ok = !id.is_empty()
        && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if ok {
        Ok(())
    } else {
        Err(format!(
            "invalid session id {id:?}: must match ^[A-Za-z0-9._-]+$ (no `/`, `\\`, or other separators)"
        ))
    }
}

fn session_path(dir: &Path, id: &str) -> Result<PathBuf, String> {
    validate_id(id)?;
    Ok(dir.join(format!("{id}.json")))
}

/// Read `index.json`, treating a missing file as an empty index — the state
/// before the very first session has ever been saved.
fn read_index(dir: &Path) -> Result<Vec<SessionMeta>, String> {
    let path = index_path(dir);
    match fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str(&raw)
            .map_err(|e| format!("corrupt session index {}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("could not read session index {}: {e}", path.display())),
    }
}

/// Persist `metas` as `index.json`, atomically (`.tmp` write + rename).
fn write_index(dir: &Path, metas: &[SessionMeta]) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let path = index_path(dir);
    let tmp = dir.join("index.json.tmp");
    let raw = serde_json::to_string_pretty(metas).map_err(|e| e.to_string())?;
    // Owner-only: the index holds session titles derived from the first user
    // prompt, which can name clusters/resources/incidents.
    write_private(&tmp, &raw).map_err(|e| format!("could not write {}: {e}", tmp.display()))?;
    fs::rename(&tmp, &path).map_err(|e| format!("could not finalize {}: {e}", path.display()))
}

/// Insert or update `meta` in `index.json` by id, then persist atomically.
/// A second upsert for the same id replaces its entry in place rather than
/// appending a duplicate.
fn upsert_index(dir: &Path, meta: &SessionMeta) -> Result<(), String> {
    let mut metas = read_index(dir)?;
    match metas.iter_mut().find(|m| m.id == meta.id) {
        Some(existing) => *existing = meta.clone(),
        None => metas.push(meta.clone()),
    }
    write_index(dir, &metas)
}

/// Write `contents` to `path` with owner-only permissions (`0600` on Unix).
/// A saved transcript can contain secrets returned by the consent-gated
/// `k8s.getSecret` tool (plus logs/manifests), so on a shared Unix host it must
/// not be left world-readable by the default `022`-umask `0644`. On non-Unix
/// the default per-user permissions apply.
fn write_private(path: &Path, contents: &str) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(contents.as_bytes())
    }
    #[cfg(not(unix))]
    {
        fs::write(path, contents)
    }
}

/// Write `<id>.json.tmp` then rename onto `<id>.json` — a crash mid-write (or
/// a concurrent read) never observes a half-written session file. The tmp file
/// is created `0600` and `rename` preserves that mode onto the final file.
fn write_session_atomic(dir: &Path, session: &Session) -> Result<(), String> {
    let path = session_path(dir, &session.id)?;
    fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let tmp = dir.join(format!("{}.json.tmp", session.id));
    let raw = serde_json::to_string_pretty(session).map_err(|e| e.to_string())?;
    write_private(&tmp, &raw).map_err(|e| format!("could not write {}: {e}", tmp.display()))?;
    fs::rename(&tmp, &path).map_err(|e| format!("could not finalize {}: {e}", path.display()))
}

/// Read one session's full file back off disk. A missing file (unknown id)
/// is reported as a clear, id-carrying `Err` rather than a raw IO message.
fn read_session(dir: &Path, id: &str) -> Result<Session, String> {
    let path = session_path(dir, id)?;
    let raw = fs::read_to_string(&path).map_err(|_| format!("no chat session found for id {id:?}"))?;
    serde_json::from_str(&raw).map_err(|e| format!("corrupt session {}: {e}", path.display()))
}

/// Remove a session's file and its `index.json` entry. Removing an id that
/// has no file is not an error — deleting is idempotent.
fn delete_session(dir: &Path, id: &str) -> Result<(), String> {
    let path = session_path(dir, id)?;
    if let Err(e) = fs::remove_file(&path) {
        if e.kind() != std::io::ErrorKind::NotFound {
            return Err(format!("could not delete {}: {e}", path.display()));
        }
    }
    let mut metas = read_index(dir)?;
    metas.retain(|m| m.id != id);
    write_index(dir, &metas)
}

/// Metas sorted newest-first by `updated_at`, for the session picker.
fn list_sessions(dir: &Path) -> Result<Vec<SessionMeta>, String> {
    let mut metas = read_index(dir)?;
    metas.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(metas)
}

/// Write the full session, then upsert its metadata into the index. Order
/// matters: if the process dies between the two, `index.json` at worst
/// lags a session file that's already safely on disk, rather than pointing
/// at an id with no backing file.
fn save_session(dir: &Path, session: &Session) -> Result<(), String> {
    write_session_atomic(dir, session)?;
    let meta = SessionMeta {
        id: session.id.clone(),
        title: session.title.clone(),
        created_at: session.created_at,
        updated_at: session.updated_at,
    };
    upsert_index(dir, &meta)
}

/// Resolve `<app config dir>/assistant/sessions`, creating it if needed —
/// the one bit of app-specific wiring the pure helpers above don't do.
fn resolve_sessions_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let base = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let dir = sessions_dir(&base);
    fs::create_dir_all(&dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    Ok(dir)
}

/// List saved chat sessions, newest first.
#[tauri::command]
pub fn chat_history_list(app: AppHandle) -> Result<Vec<SessionMeta>, String> {
    list_sessions(&resolve_sessions_dir(&app)?)
}

/// Load one full chat session (including its message transcript) by id.
#[tauri::command]
pub fn chat_history_load(app: AppHandle, id: String) -> Result<Session, String> {
    read_session(&resolve_sessions_dir(&app)?, &id)
}

/// Persist a chat session, creating or updating both its file and index
/// entry.
#[tauri::command]
pub fn chat_history_save(app: AppHandle, session: Session) -> Result<(), String> {
    save_session(&resolve_sessions_dir(&app)?, &session)
}

/// Delete a chat session's file and its index entry.
#[tauri::command]
pub fn chat_history_delete(app: AppHandle, id: String) -> Result<(), String> {
    delete_session(&resolve_sessions_dir(&app)?, &id)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh, empty directory under the OS temp dir, unique per test so
    /// parallel test runs never collide. Removed on drop so a test's fixture
    /// files don't linger.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("srelens-assistant-history-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn sample_session(id: &str, updated_at: i64) -> Session {
        Session {
            id: id.to_string(),
            title: format!("session {id}"),
            created_at: 1_000,
            updated_at,
            contexts: vec!["ctx-a".to_string()],
            skills: vec!["skill-a".to_string()],
            cli_session_id: Some("cli-123".to_string()),
            agent_kind: Some("codex".to_string()),
            messages: vec![serde_json::json!({"role": "user", "text": "hi"})],
        }
    }

    #[test]
    fn sessions_dir_joins_assistant_sessions_under_base() {
        let base = Path::new("/some/config/dir");
        assert_eq!(sessions_dir(base), base.join("assistant").join("sessions"));
    }

    #[test]
    fn session_round_trips_through_serde_with_camel_case_keys() {
        let session = sample_session("abc", 42);
        let raw = serde_json::to_string(&session).unwrap();

        // The wire format is camelCase, not the Rust snake_case field names.
        assert!(raw.contains("\"createdAt\""));
        assert!(raw.contains("\"updatedAt\""));
        assert!(raw.contains("\"cliSessionId\""));
        assert!(!raw.contains("created_at"));

        let round_tripped: Session = serde_json::from_str(&raw).unwrap();
        assert_eq!(round_tripped, session);
    }

    #[test]
    fn session_missing_optional_fields_defaults_them() {
        // An older session file written before `skills`/`contexts`/
        // `cliSessionId` existed must still deserialize.
        let raw = serde_json::json!({
            "id": "old",
            "title": "legacy",
            "createdAt": 1,
            "updatedAt": 2,
            "messages": [],
        })
        .to_string();

        let session: Session = serde_json::from_str(&raw).unwrap();
        assert_eq!(session.contexts, Vec::<String>::new());
        assert_eq!(session.skills, Vec::<String>::new());
        assert_eq!(session.cli_session_id, None);
        assert_eq!(session.agent_kind, None);
    }

    #[test]
    fn save_then_load_returns_an_equal_session() {
        let tmp = TempDir::new();
        let dir = sessions_dir(tmp.path());
        let session = sample_session("s1", 100);

        save_session(&dir, &session).unwrap();
        let loaded = read_session(&dir, "s1").unwrap();

        assert_eq!(loaded, session);
    }

    #[test]
    fn save_upserts_index_in_place_without_duplicating() {
        let tmp = TempDir::new();
        let dir = sessions_dir(tmp.path());

        let first = sample_session("dup", 100);
        save_session(&dir, &first).unwrap();

        let mut updated = sample_session("dup", 200);
        updated.title = "renamed".to_string();
        save_session(&dir, &updated).unwrap();

        let metas = read_index(&dir).unwrap();
        assert_eq!(metas.len(), 1, "expected the second save to update in place, not duplicate");
        assert_eq!(metas[0].title, "renamed");
        assert_eq!(metas[0].updated_at, 200);
    }

    #[test]
    fn list_returns_metas_sorted_by_updated_at_descending() {
        let tmp = TempDir::new();
        let dir = sessions_dir(tmp.path());

        // Inserted out of chronological order on purpose: if `list_sessions`
        // merely echoed insertion order, this would fail.
        save_session(&dir, &sample_session("mid", 50)).unwrap();
        save_session(&dir, &sample_session("newest", 300)).unwrap();
        save_session(&dir, &sample_session("oldest", 10)).unwrap();

        let ids: Vec<String> = list_sessions(&dir).unwrap().into_iter().map(|m| m.id).collect();
        assert_eq!(ids, vec!["newest", "mid", "oldest"]);
    }

    #[test]
    fn delete_removes_both_the_file_and_the_index_entry() {
        let tmp = TempDir::new();
        let dir = sessions_dir(tmp.path());
        save_session(&dir, &sample_session("gone", 1)).unwrap();
        save_session(&dir, &sample_session("stays", 2)).unwrap();

        delete_session(&dir, "gone").unwrap();

        assert!(!session_path(&dir, "gone").unwrap().exists(), "session file should be removed");
        let remaining_ids: Vec<String> = read_index(&dir).unwrap().into_iter().map(|m| m.id).collect();
        assert_eq!(remaining_ids, vec!["stays"]);
    }

    #[test]
    fn load_of_unknown_id_returns_a_clear_err() {
        let tmp = TempDir::new();
        let dir = sessions_dir(tmp.path());
        fs::create_dir_all(&dir).unwrap();

        let err = read_session(&dir, "does-not-exist").unwrap_err();
        assert!(err.contains("does-not-exist"), "error should name the missing id, got: {err}");
    }

    /// Security: a session id from IPC (or a crafted `index.json` entry) that
    /// isn't a bare filename component must be rejected everywhere it becomes a
    /// path, so it can never read/write/delete `.json` files outside the
    /// sessions directory.
    #[test]
    fn a_session_id_with_traversal_or_separators_is_rejected_everywhere() {
        let tmp = TempDir::new();
        let dir = sessions_dir(tmp.path());
        fs::create_dir_all(&dir).unwrap();

        for bad in ["../evil", "a/b", "a\\b", "/etc/passwd", "../../secret", ""] {
            assert!(read_session(&dir, bad).is_err(), "read({bad:?}) must be rejected");
            assert!(delete_session(&dir, bad).is_err(), "delete({bad:?}) must be rejected");
            let mut s = sample_session("valid", 1);
            s.id = bad.to_string();
            assert!(write_session_atomic(&dir, &s).is_err(), "save({bad:?}) must be rejected");
        }
        // A normal uuid-shaped id is accepted.
        assert!(session_path(&dir, "0b3e97f0-1234-4abc-9def-000000000000").is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn a_saved_session_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new();
        let dir = sessions_dir(tmp.path());
        let session = sample_session("0b3e97f0-1234-4abc-9def-000000000000", 1);
        write_session_atomic(&dir, &session).unwrap();
        let path = session_path(&dir, &session.id).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        // Transcripts can contain k8s.getSecret output — never world/group readable.
        assert_eq!(mode & 0o777, 0o600, "saved session must be 0600, got {:o}", mode & 0o777);
    }
}
