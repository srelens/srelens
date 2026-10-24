//! Per-turn setup shared by every front end that drives an agent CLI: the
//! private scratch files and directories a turn writes (MCP configs carrying a
//! bearer token, attached images, the agent's empty working directory) and the
//! guards that remove them when the turn ends.

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
