//! Streaming core for helm write operations: runs `helm` against a
//! context-scoped kubeconfig and streams stdout+stderr to an EventSink.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::task::JoinHandle;

use crate::sink::EventSink;

/// Read `reader` line-by-line, calling `emit` for each line (newline stripped).
pub async fn stream_lines<R: tokio::io::AsyncRead + Unpin>(
    reader: R,
    mut emit: impl FnMut(String),
) {
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        emit(line);
    }
}

struct Session {
    handle: JoinHandle<()>,
}

/// Owns running helm operations (keyed by numeric id).
pub struct HelmManager {
    next_id: AtomicU64,
    sessions: Mutex<HashMap<u64, Session>>,
}

impl Default for HelmManager {
    fn default() -> Self {
        Self::new()
    }
}

impl HelmManager {
    pub fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            sessions: Mutex::new(HashMap::new()),
        }
    }

    /// Run `helm <args>` scoped to `context` (resolved from `kubeconfig_paths`),
    /// streaming stdout+stderr on `helm:out:<channel>`; `helm:exit:<channel>`
    /// fires with None on success or an error string on failure. Returns the
    /// session id.
    ///
    /// `helm_home`, when set, isolates helm's on-disk state (repository config,
    /// cache, plugins) under that directory via the `HELM_*_HOME` env vars. The
    /// multi-user web server passes each user's private dir so one user's helm
    /// state can never leak into or be read from another's; desktop passes
    /// `None` and uses helm's default single-user home unchanged.
    pub async fn start(
        &self,
        sink: Arc<dyn EventSink>,
        context: String,
        kubeconfig_paths: Vec<PathBuf>,
        args: Vec<String>,
        values: String,
        channel: String,
        helm_home: Option<PathBuf>,
    ) -> Result<u64, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let bin = srelens_kube::helm_cli::helm_binary()?;

        let ctx = context.clone();
        let kubeconfig_path = tokio::task::spawn_blocking(move || {
            srelens_kube::connect::write_single_context_kubeconfig(&kubeconfig_paths, &ctx)
        })
        .await
        .map_err(|e| e.to_string())??;
        // Guard wraps the kubeconfig so it's removed on every exit path
        // (success, error, or an aborted/cancelled session) — see `close`.
        let kubeconfig = srelens_kube::helm_cli::TempFile(kubeconfig_path);

        let values_file = srelens_kube::helm_cli::write_values_file(&values)?
            .map(srelens_kube::helm_cli::TempFile);
        let mut full_args = args;
        if let Some(ref vf) = values_file {
            full_args.push("--values".to_string());
            full_args.push(vf.path().display().to_string());
        }

        let out_channel = format!("helm:out:{channel}");
        let exit_channel = format!("helm:exit:{channel}");

        let handle = tokio::spawn(async move {
            // `kubeconfig` and `values_file` are moved into this task so they
            // live for the whole run and are removed via `Drop` when the task
            // finishes (any exit path) or is aborted (`close`).
            //
            // `values_file` MUST be bound here: an `async move` block only
            // captures the variables it MENTIONS, and the values path is
            // otherwise only read above (when building `full_args`). Without
            // this binding the guard stays a local of `start` and drops the
            // moment it returns — deleting the values file while helm is still
            // starting. `kubeconfig` is captured implicitly by `.env(...)`.
            let _values_file = values_file;
            let mut cmd = tokio::process::Command::new(&bin);
            cmd.args(&full_args)
                .env("KUBECONFIG", kubeconfig.path())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true);
            if let Some(home) = &helm_home {
                // Point every helm state location at this user's private dir.
                // repositories.yaml, the download cache, and installed plugins
                // then live under `home` alone — a `repo`/`plugin`/cache write
                // by one user can't reach another's environment on the shared
                // server. helm creates these subdirs on demand.
                cmd.env("HELM_CONFIG_HOME", home.join("config"))
                    .env("HELM_CACHE_HOME", home.join("cache"))
                    .env("HELM_DATA_HOME", home.join("data"))
                    .env(
                        "HELM_REPOSITORY_CONFIG",
                        home.join("config").join("repositories.yaml"),
                    )
                    .env(
                        "HELM_REPOSITORY_CACHE",
                        home.join("cache").join("repository"),
                    );
            }
            let spawn = cmd.spawn();

            let result = match spawn {
                Ok(mut child) => {
                    let stdout = child.stdout.take();
                    let stderr = child.stderr.take();
                    let out_ch = out_channel.clone();
                    let err_ch = out_channel.clone();
                    let s1 = sink.clone();
                    let s2 = sink.clone();
                    let t_out = tokio::spawn(async move {
                        if let Some(s) = stdout {
                            stream_lines(s, |l| {
                                s1.emit(&out_ch, serde_json::Value::String(l));
                            })
                            .await;
                        }
                    });
                    let t_err = tokio::spawn(async move {
                        if let Some(s) = stderr {
                            stream_lines(s, |l| {
                                s2.emit(&err_ch, serde_json::Value::String(l));
                            })
                            .await;
                        }
                    });
                    let _ = t_out.await;
                    let _ = t_err.await;
                    match child.wait().await {
                        Ok(status) if status.success() => None,
                        Ok(status) => Some(format!(
                            "helm exited with code {}",
                            status.code().unwrap_or(-1)
                        )),
                        Err(e) => Some(e.to_string()),
                    }
                }
                Err(e) => Some(e.to_string()),
            };
            sink.emit(
                &exit_channel,
                serde_json::to_value(result).unwrap_or(serde_json::Value::Null),
            );
        });

        self.sessions.lock().unwrap().insert(id, Session { handle });
        Ok(id)
    }

    /// Abort a running helm operation (best-effort) and drop its session. The
    /// aborted task's `TempFile` guards are dropped as part of cancelling the
    /// in-flight future, which removes their temp files.
    pub fn close(&self, session: u64) {
        if let Some(s) = self.sessions.lock().unwrap().remove(&session) {
            s.handle.abort();
        }
    }

    /// Abort every running helm operation (used when a user's environment is
    /// dropped).
    pub fn shutdown_all(&self) {
        let mut sessions = self.sessions.lock().unwrap();
        for (_, session) in sessions.drain() {
            session.handle.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stream_lines_emits_each_line() {
        let data = &b"first\nsecond\nthird\n"[..];
        let mut got = Vec::new();
        stream_lines(data, |l| got.push(l)).await;
        assert_eq!(got, vec!["first", "second", "third"]);
    }

    // Constructs a `Session` directly (rather than going through `start`) so
    // this test doesn't depend on a `helm` binary being on PATH.
    #[tokio::test(flavor = "multi_thread")]
    async fn shutdown_all_aborts_sessions() {
        let manager = HelmManager::new();
        let handle = tokio::spawn(async {
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        });
        manager
            .sessions
            .lock()
            .unwrap()
            .insert(1, Session { handle });

        manager.shutdown_all(); // no panic; subsequent close is a no-op
        assert!(manager.sessions.lock().unwrap().is_empty());
        manager.close(1);
    }
}
