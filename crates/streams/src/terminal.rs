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
use crate::terminal_scope::{banner, Scope};

/// Global counter for unique session directory names across all
/// TerminalManager instances in this process. Guarantees no collision even
/// when multiple managers (e.g., per-user in a web server) create them.
static NEXT_OVERLAY_ID: AtomicU64 = AtomicU64::new(1);

struct Session {
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    child: Mutex<Box<dyn Child + Send + Sync>>,
    /// The session's private directory — its one-context kubeconfig and the
    /// guards that keep it on that cluster — removed when the session closes.
    scope: Arc<Scope>,
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

/// The script that runs `command` and then becomes the user's own shell, for a
/// terminal opened to run one thing the reader has just confirmed — a
/// `kubectl drain` they want to watch (srelens/srelens#820).
///
/// The command is the PTY's first process rather than keystrokes sent to an
/// interactive shell, because nothing outside a shell can tell when it has
/// finished its rc files and is the one reading the terminal: typed too early,
/// a confirmed command is swallowed and the node is silently never drained.
///
/// `/bin/sh`, not `$SHELL`: the command is written in POSIX quoting, and the
/// environment is already the one the user's shell would give it (see the
/// `PATH` note in [`TerminalManager::start`]). The shell they chose is what
/// they are left in afterwards.
///
/// The `INT` trap is a handler, not `''`, so it is not inherited: Ctrl-C still
/// stops the command, and the script lives to hand over the shell instead of
/// dying with it and taking the terminal away mid-read.
///
/// One statement per line, so nothing the command ends with — a comment, a
/// trailing operator — can swallow the hand-over that follows it.
///
/// `shell_args` are the ones the shell is started with when there is no
/// command — the rc file that confines it (#846) — so the shell that follows a
/// command is confined the same way.
fn run_then_shell(command: &str, shell_args: &[String]) -> String {
    let args: String = shell_args
        .iter()
        .map(|arg| format!(" '{}'", arg.replace('\'', "'\\''")))
        .collect();
    format!("trap : INT\n{command}\ntrap - INT\nexec \"$SHELL\"{args}\n")
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
    ///
    /// `command`, when given, is run first and the shell follows it: see
    /// [`run_then_shell`].
    ///
    /// `namespace`, when given, is the one `kubectl` uses by default in this
    /// terminal — the one the tab it was opened from is looking at (#846).
    #[allow(clippy::too_many_arguments)]
    pub async fn start(
        &self,
        sink: Arc<dyn EventSink>,
        context: String,
        kubeconfig_paths: Vec<PathBuf>,
        channel: String,
        cols: Option<u16>,
        rows: Option<u16>,
        command: Option<String>,
        namespace: Option<String>,
    ) -> Result<u64, String> {
        // Refused before anything is allocated: `run_then_shell` is a POSIX
        // script for `/bin/sh`, and a platform without one would fail at the
        // spawn below with an error about a path the caller never named.
        #[cfg(not(unix))]
        if command.is_some() {
            return Err("Running a command in a new local terminal is not supported on this platform".into());
        }
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let overlay_id = NEXT_OVERLAY_ID.fetch_add(1, Ordering::SeqCst);
        let namespace = namespace.filter(|ns| !ns.is_empty());
        let ctx = context.clone();
        let ns = namespace.clone();
        let scope = tokio::task::spawn_blocking(move || {
            Scope::create(overlay_id, &ctx, ns.as_deref(), &kubeconfig_paths)
        })
        .await
        .map_err(|e| e.to_string())??;
        let scope = Arc::new(scope);
        // Everything below can fail, and a session that never started has
        // nobody to close it: the directory holds credentials, so it goes with
        // the error rather than staying in the temp directory.
        let started = self
            .spawn(id, sink, &scope, context, namespace, channel, cols, rows, command)
            .await;
        if started.is_err() {
            scope.remove();
        }
        started.map(|()| id)
    }

    #[allow(clippy::too_many_arguments)]
    async fn spawn(
        &self,
        id: u64,
        sink: Arc<dyn EventSink>,
        scope: &Arc<Scope>,
        context: String,
        namespace: Option<String>,
        channel: String,
        cols: Option<u16>,
        rows: Option<u16>,
        command: Option<String>,
    ) -> Result<(), String> {

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
        let shell_start = scope.shell_start(&shell);
        let mut cmd = match &command {
            Some(command) => {
                let mut cmd = CommandBuilder::new("/bin/sh");
                cmd.arg("-c");
                cmd.arg(run_then_shell(command, &shell_start.args));
                cmd
            }
            None => {
                let mut cmd = CommandBuilder::new(&shell);
                cmd.args(&shell_start.args);
                cmd
            }
        };
        // Propagate the parent environment (on desktop, PATH is already
        // resolved by fix-path-env at startup, so kubectl / helm / cloud CLIs
        // are found), then scope kubectl.
        for (key, value) in std::env::vars() {
            cmd.env(key, value);
        }
        // After the loop, so a `SHELL` the parent never had still names the
        // shell `run_then_shell` hands over to.
        cmd.env("SHELL", &shell);
        cmd.env("KUBECONFIG", &scope.kubeconfig);
        // The guards go first on PATH here, for every shell and for a command
        // run before one; bash and zsh put them back after their rc files.
        if let Some(path) = scope.path_with_guards(std::env::var_os("PATH")) {
            cmd.env("PATH", path);
        }
        for (key, value) in &shell_start.env {
            cmd.env(key, value);
        }
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
            scope: Arc::clone(scope),
        });
        self.sessions.lock().unwrap().insert(id, session);

        // Blocking read loop on a dedicated thread (portable-pty readers are sync).
        let out_channel = format!("term:out:{channel}");
        let exit_channel = format!("term:exit:{channel}");
        let sessions = Arc::clone(&self.sessions);
        let scope = Arc::clone(scope);
        // Before the reader thread exists, so it is the first thing on the
        // channel whatever the shell is quick enough to print.
        sink.emit(
            &out_channel,
            serde_json::Value::String(banner(&context, namespace.as_deref())),
        );
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
            scope.remove();
            sessions.lock().unwrap().remove(&id);
        });

        Ok(())
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
            s.scope.remove();
        }
    }

    /// Kill every running terminal's child shell and remove its session
    /// directory (used when a user's environment is dropped). Mirrors
    /// `close`, applied to every tracked session.
    pub fn shutdown_all(&self) {
        let mut sessions = self.sessions.lock().unwrap();
        for (_, s) in sessions.drain() {
            let _ = s.child.lock().unwrap().kill();
            s.scope.remove();
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
                None,
                None,
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

    /// Everything the terminal on `channel` has printed so far.
    fn output_on(sink: &TestSink, channel: &str) -> String {
        sink.payloads_for(&format!("term:out:{channel}"))
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect()
    }

    async fn wait_for_output(sink: &TestSink, channel: &str, needle: &str) -> bool {
        for _ in 0..100 {
            if output_on(sink, channel).contains(needle) {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        false
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_command_runs_first_without_being_typed_and_the_shell_follows_it() {
        let dir = tempfile::tempdir().unwrap();
        let kc = fixture_kubeconfig(dir.path());
        let sink = Arc::new(TestSink::default());
        let manager = TerminalManager::new();
        // Arithmetic, for the reason given above: only execution turns
        // `$((20+22))` into `42`. Nothing is sent on stdin for it.
        let id = manager
            .start(
                sink.clone(),
                "test".into(),
                vec![kc],
                "t3".into(),
                Some(80),
                Some(24),
                Some("printf 'srelens-first:%s\\n' $((20+22))".into()),
                None,
            )
            .await
            .expect("terminal starts");

        let ran = wait_for_output(&sink, "t3", "srelens-first:42").await;
        // And the session is still a shell afterwards: it takes the next line.
        manager.input(id, "printf 'srelens-after:%s\\n' $((1+2))\n");
        let followed = wait_for_output(&sink, "t3", "srelens-after:3").await;
        manager.close(id);

        assert!(ran, "the command ran as the terminal's first process");
        assert!(followed, "the user's shell took over after the command");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_failing_command_still_leaves_the_shell() {
        let dir = tempfile::tempdir().unwrap();
        let kc = fixture_kubeconfig(dir.path());
        let sink = Arc::new(TestSink::default());
        let manager = TerminalManager::new();
        let id = manager
            .start(
                sink.clone(),
                "test".into(),
                vec![kc],
                "t4".into(),
                Some(80),
                Some(24),
                Some("srelens-no-such-command-820".into()),
                None,
            )
            .await
            .expect("terminal starts");

        manager.input(id, "printf 'srelens-after:%s\\n' $((1+2))\n");
        let followed = wait_for_output(&sink, "t4", "srelens-after:3").await;
        manager.close(id);

        assert!(followed, "a refused or missing command does not take the terminal away");
    }

    #[test]
    fn the_hand_over_is_on_its_own_line_whatever_the_command_ends_with() {
        let script = run_then_shell("kubectl drain node-1 # a trailing comment", &[]);
        let lines: Vec<&str> = script.lines().collect();
        assert_eq!(
            lines,
            vec![
                "trap : INT",
                "kubectl drain node-1 # a trailing comment",
                "trap - INT",
                "exec \"$SHELL\"",
            ]
        );
    }

    #[test]
    fn the_shell_that_follows_a_command_is_started_with_the_same_arguments() {
        let script = run_then_shell("true", &["--rcfile".into(), "/tmp/it's here/bashrc".into()]);
        assert_eq!(
            script.lines().last(),
            Some(r#"exec "$SHELL" '--rcfile' '/tmp/it'\''s here/bashrc'"#)
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_terminal_says_what_it_is_bound_to_before_anything_else() {
        let dir = tempfile::tempdir().unwrap();
        let kc = fixture_kubeconfig(dir.path());
        let sink = Arc::new(TestSink::default());
        let manager = TerminalManager::new();
        let id = manager
            .start(
                sink.clone(),
                "test".into(),
                vec![kc],
                "t5".into(),
                Some(80),
                Some(24),
                None,
                Some("payments".into()),
            )
            .await
            .expect("terminal starts");

        let first = sink.payloads_for("term:out:t5").first().cloned();
        // And the namespace is the one `kubectl` will use, not only one named
        // in a sentence: it is in the kubeconfig the shell was given.
        manager.input(id, "grep -c 'namespace: payments' \"$KUBECONFIG\" | sed 's/^/srelens-ns:/'\n");
        let in_kubeconfig = wait_for_output(&sink, "t5", "srelens-ns:1").await;
        manager.close(id);

        let first = first.and_then(|v| v.as_str().map(String::from)).unwrap_or_default();
        assert!(first.contains("bound to test, namespace payments"), "{first}");
        assert!(in_kubeconfig, "the session kubeconfig names the namespace");
    }

    /// The guards, met the way the reader meets them: typed at the prompt of
    /// the real shell, after its rc files have had their say about `PATH`.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn kubectl_typed_in_the_terminal_cannot_be_aimed_at_another_cluster() {
        let dir = tempfile::tempdir().unwrap();
        let kc = fixture_kubeconfig(dir.path());
        let sink = Arc::new(TestSink::default());
        let manager = TerminalManager::new();
        let id = manager
            .start(
                sink.clone(),
                "test".into(),
                vec![kc],
                "t7".into(),
                Some(200),
                Some(24),
                None,
                None,
            )
            .await
            .expect("terminal starts");

        // Refused by the guard before any `kubectl` is looked for, so this
        // holds on a machine that has none installed.
        manager.input(id, "kubectl --context elsewhere get pods\n");
        let refused = wait_for_output(&sink, "t7", "This terminal is bound to test").await;
        manager.close(id);

        assert!(refused, "{}", output_on(&sink, "t7"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_terminal_for_a_context_that_is_not_there_is_refused_and_not_tracked() {
        let dir = tempfile::tempdir().unwrap();
        let kc = fixture_kubeconfig(dir.path());
        let manager = TerminalManager::new();
        let refused = manager
            .start(
                Arc::new(TestSink::default()),
                "no-such-context".into(),
                vec![kc],
                "t6".into(),
                Some(80),
                Some(24),
                None,
                None,
            )
            .await;
        assert!(refused.is_err());
        assert!(manager.sessions.lock().unwrap().is_empty());
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
                None,
                None,
            )
            .await
            .expect("terminal starts");

        let overlay = manager
            .sessions
            .lock()
            .unwrap()
            .get(&id)
            .expect("session tracked")
            .scope
            .dir
            .clone();
        assert!(overlay.join("kubeconfig").exists(), "session kubeconfig was written");

        manager.shutdown_all(); // no panic; subsequent close is a no-op

        assert!(manager.sessions.lock().unwrap().is_empty());
        assert!(!overlay.exists(), "session directory was removed");
        manager.close(id);
    }
}
