//! Local pseudo-terminal core: spawns the host's login shell in a PTY, scoped
//! to a kube context, and streams stdout to an EventSink (stdin/resize come
//! back through the manager). This is the in-app `kubectl` terminal — a LOCAL
//! process on the host machine, distinct from the in-pod exec core. Never
//! exposed through the MCP capability registry.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

use crate::sink::EventSink;

/// Global counter for unique overlay kubeconfig filenames across all
/// TerminalManager instances in this process. Guarantees no collision even
/// when multiple managers (e.g., per-user in a web server) create overlays.
static NEXT_OVERLAY_ID: AtomicU64 = AtomicU64::new(1);

struct Session {
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    child: Mutex<Box<dyn Child + Send + Sync>>,
    /// The temp overlay kubeconfig to clean up when the session closes.
    overlay: PathBuf,
}

/// Owns running local terminals (keyed by numeric id).
pub struct TerminalManager {
    next_id: AtomicU64,
    sessions: Arc<Mutex<HashMap<u64, Arc<Session>>>>,
}

impl Default for TerminalManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Write a **standalone** kubeconfig that contains only `context` (and the one
/// cluster/user it references) to a private temp file. Pointing `KUBECONFIG`
/// at just this file locks the terminal to that cluster — no other contexts
/// are listed or switchable. Returns the temp file path.
fn write_locked_kubeconfig(overlay_id: u64, context: &str, paths: &[PathBuf]) -> Result<PathBuf, String> {
    let yaml = srelens_kube::connect::single_context_kubeconfig_yaml(paths, context)?;

    // Unique per-process name (globally unique across all TerminalManager
    // instances), created atomically with O_EXCL so a pre-existing path/symlink
    // can't be followed or overwritten, and mode 0600 at creation so there's no
    // world-readable window.
    let path = std::env::temp_dir().join(format!(
        "srelens-term-{}-{}.kubeconfig",
        std::process::id(),
        overlay_id
    ));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(&path).map_err(|e| e.to_string())?;
    file.write_all(yaml.as_bytes()).map_err(|e| e.to_string())?;
    Ok(path)
}

impl TerminalManager {
    pub fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Start a local shell scoped to `context` (resolved from
    /// `kubeconfig_paths`, pre-merged by the caller). Returns the session id;
    /// output streams on `term:out:<channel>` and a `term:exit:<channel>`
    /// event fires when it ends.
    pub async fn start(
        &self,
        sink: Arc<dyn EventSink>,
        context: String,
        kubeconfig_paths: Vec<PathBuf>,
        channel: String,
        cols: Option<u16>,
        rows: Option<u16>,
    ) -> Result<u64, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let overlay_id = NEXT_OVERLAY_ID.fetch_add(1, Ordering::SeqCst);
        let ctx = context.clone();
        let overlay = tokio::task::spawn_blocking(move || {
            write_locked_kubeconfig(overlay_id, &ctx, &kubeconfig_paths)
        })
        .await
        .map_err(|e| e.to_string())??;
        let kubeconfig = overlay.clone().into_os_string();

        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize {
                rows: rows.unwrap_or(24),
                cols: cols.unwrap_or(80),
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| e.to_string())?;

        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        let mut cmd = CommandBuilder::new(&shell);
        // Propagate the parent environment (on desktop, PATH is already
        // resolved by fix-path-env at startup, so kubectl / helm / cloud CLIs
        // are found), then scope kubectl.
        for (key, value) in std::env::vars() {
            cmd.env(key, value);
        }
        cmd.env("KUBECONFIG", &kubeconfig);
        cmd.env("TERM", "xterm-256color");
        cmd.env("SRELENS_CONTEXT", &context);
        if let Ok(home) = std::env::var("HOME") {
            cmd.cwd(home);
        }

        let child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
        // Drop the slave so the reader sees EOF once the child exits.
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        let writer = pair.master.take_writer().map_err(|e| e.to_string())?;

        let session = Arc::new(Session {
            master: Mutex::new(pair.master),
            writer: Mutex::new(writer),
            child: Mutex::new(child),
            overlay: overlay.clone(),
        });
        self.sessions.lock().unwrap().insert(id, session);

        // Blocking read loop on a dedicated thread (portable-pty readers are sync).
        let out_channel = format!("term:out:{channel}");
        let exit_channel = format!("term:exit:{channel}");
        let sessions = Arc::clone(&self.sessions);
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            let mut carry: Vec<u8> = Vec::new();
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        carry.extend_from_slice(&buf[..n]);
                        // Emit the valid UTF-8 prefix; keep an incomplete
                        // trailing multibyte sequence (<=3 bytes) for the next
                        // read. If the leftover exceeds 3 bytes it contains an
                        // actually-invalid byte, so flush it lossily rather
                        // than stalling.
                        let valid = std::str::from_utf8(&carry)
                            .map(|s| s.len())
                            .unwrap_or_else(|e| e.valid_up_to());
                        let cut = if carry.len() - valid > 3 {
                            carry.len()
                        } else {
                            valid
                        };
                        if cut > 0 {
                            let text = String::from_utf8_lossy(&carry[..cut]).into_owned();
                            sink.emit(&out_channel, serde_json::Value::String(text));
                            carry.drain(..cut);
                        }
                    }
                    Err(_) => break,
                }
            }
            if !carry.is_empty() {
                sink.emit(
                    &out_channel,
                    serde_json::Value::String(String::from_utf8_lossy(&carry).into_owned()),
                );
            }
            sink.emit(&exit_channel, serde_json::Value::Null);
            let _ = std::fs::remove_file(&overlay);
            sessions.lock().unwrap().remove(&id);
        });

        Ok(id)
    }

    /// Forward keystrokes / pasted input to a terminal's stdin.
    pub fn input(&self, session: u64, data: &str) {
        let s = self.sessions.lock().unwrap().get(&session).cloned();
        if let Some(s) = s {
            let mut writer = s.writer.lock().unwrap();
            let _ = writer.write_all(data.as_bytes());
            let _ = writer.flush();
        }
    }

    /// Resize a terminal's PTY (columns/rows) to match the xterm viewport.
    pub fn resize(&self, session: u64, cols: u16, rows: u16) {
        let s = self.sessions.lock().unwrap().get(&session).cloned();
        if let Some(s) = s {
            let _ = s.master.lock().unwrap().resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
    }

    /// Close a terminal: kill the shell and drop the session.
    pub fn close(&self, session: u64) {
        if let Some(s) = self.sessions.lock().unwrap().remove(&session) {
            let _ = s.child.lock().unwrap().kill();
            let _ = std::fs::remove_file(&s.overlay);
        }
    }

    /// Kill every running terminal's child shell and remove its overlay
    /// kubeconfig (used when a user's environment is dropped). Mirrors
    /// `close`, applied to every tracked session.
    pub fn shutdown_all(&self) {
        let mut sessions = self.sessions.lock().unwrap();
        for (_, s) in sessions.drain() {
            let _ = s.child.lock().unwrap().kill();
            let _ = std::fs::remove_file(&s.overlay);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::TestSink;


    fn fixture_kubeconfig(dir: &std::path::Path) -> PathBuf {
        let path = dir.join("config");
        let yaml = r#"apiVersion: v1
kind: Config
current-context: test
clusters:
- name: test-cluster
  cluster:
    server: https://127.0.0.1:1
users:
- name: test-user
  user: {}
contexts:
- name: test
  context:
    cluster: test-cluster
    user: test-user
"#;
        std::fs::File::create(&path)
            .unwrap()
            .write_all(yaml.as_bytes())
            .unwrap();
        path
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pty_round_trips_output_through_sink() {
        let dir = tempfile::tempdir().unwrap();
        let kc = fixture_kubeconfig(dir.path());
        let sink = Arc::new(TestSink::default());
        let manager = TerminalManager::new();
        let id = manager
            .start(
                sink.clone(),
                "test".into(),
                vec![kc],
                "t1".into(),
                Some(80),
                Some(24),
            )
            .await
            .expect("terminal starts");

        // Send an arithmetic expression rather than a literal string: the
        // shell's own echoing of typed input (and, on some shells, title-bar
        // escape sequences / line redraws from prompt themes) reproduces the
        // literal keystrokes verbatim, but only genuine command *execution*
        // can turn `$((20+22))` into `42`. That decouples the assertion from
        // shell/theme-specific framing around the output (e.g. oh-my-zsh
        // emits an OSC window-title escape, not a bare `\n`/`\r`, right
        // before the real output line).
        manager.input(id, "printf 'srelens-pty-result:%s\\n' $((20+22))\n");
        let mut seen = false;
        for _ in 0..100 {
            let out: String = sink
                .payloads_for("term:out:t1")
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            if out.contains("srelens-pty-result:42") {
                seen = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        manager.close(id);
        assert!(seen, "PTY output arrived on the sink");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn shutdown_all_kills_children_and_removes_overlay() {
        let dir = tempfile::tempdir().unwrap();
        let kc = fixture_kubeconfig(dir.path());
        let sink = Arc::new(TestSink::default());
        let manager = TerminalManager::new();
        let id = manager
            .start(
                sink,
                "test".into(),
                vec![kc],
                "t2".into(),
                Some(80),
                Some(24),
            )
            .await
            .expect("terminal starts");

        let overlay = manager
            .sessions
            .lock()
            .unwrap()
            .get(&id)
            .expect("session tracked")
            .overlay
            .clone();
        assert!(overlay.exists(), "overlay kubeconfig was written");

        manager.shutdown_all(); // no panic; subsequent close is a no-op

        assert!(manager.sessions.lock().unwrap().is_empty());
        assert!(!overlay.exists(), "overlay kubeconfig was removed");
        manager.close(id);
    }
}
