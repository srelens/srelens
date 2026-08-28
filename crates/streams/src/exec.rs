//! Interactive in-pod exec core: spawns kube-rs exec sessions, streams stdout
//! to an EventSink, and forwards stdin/resizes from the host.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use srelens_kube::client_cache::ClientCache;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::sink::EventSink;

struct Session {
    handle: JoinHandle<()>,
    input: mpsc::Sender<String>,
    resize: mpsc::Sender<(u16, u16)>,
}

/// Options for opening an exec session.
#[derive(Default)]
pub struct ExecOpts {
    pub container: Option<String>,
    pub shell: Option<String>,
    pub command: Option<Vec<String>>,
    pub cols: Option<u16>,
    pub rows: Option<u16>,
}

/// Owns running exec sessions (keyed by numeric id).
pub struct ExecManager {
    cache: Arc<ClientCache>,
    next_id: AtomicU64,
    sessions: Mutex<HashMap<u64, Session>>,
}

impl ExecManager {
    pub fn new(cache: Arc<ClientCache>) -> Self {
        Self {
            cache,
            next_id: AtomicU64::new(1),
            sessions: Mutex::new(HashMap::new()),
        }
    }

    /// Open an interactive shell into a pod. Returns the session id; stdout
    /// streams on `exec:out:<channel>` and an `exec:exit:<channel>` event
    /// fires (with an optional error string) when the session ends.
    ///
    /// `channel` is the CALLER'S subscription token, not the session id this
    /// returns, and that is the whole point: the task spawned below can emit
    /// its exit event in the same tick — an unresolvable context, an RBAC
    /// refusal — so a frontend that could only subscribe once it had the id
    /// would lose the only exit event there will ever be, leaving the session's
    /// row attached and a node session's privileged debug pod on the node. Same
    /// shape as `TerminalManager::start`.
    pub async fn start(
        &self,
        sink: Arc<dyn EventSink>,
        context: String,
        namespace: String,
        pod: String,
        channel: String,
        opts: ExecOpts,
    ) -> Result<u64, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel::<String>(64);
        let (resize_tx, resize_rx) = mpsc::channel::<(u16, u16)>(8);
        let initial_size = opts.cols.zip(opts.rows);
        let cache = self.cache.clone();
        let out_channel = format!("exec:out:{channel}");
        let exit_channel = format!("exec:exit:{channel}");
        let out_sink = sink.clone();

        let handle = tokio::spawn(async move {
            let result = srelens_kube::exec::exec_shell(
                cache,
                context,
                namespace,
                pod,
                opts.container,
                opts.shell,
                opts.command,
                initial_size,
                resize_rx,
                move |chunk| {
                    if let Ok(v) = serde_json::to_value(&chunk) {
                        out_sink.emit(&out_channel, v);
                    }
                },
                rx,
            )
            .await;
            sink.emit(
                &exit_channel,
                serde_json::to_value(result.err()).unwrap_or(serde_json::Value::Null),
            );
        });

        self.sessions.lock().unwrap().insert(
            id,
            Session {
                handle,
                input: tx,
                resize: resize_tx,
            },
        );
        Ok(id)
    }

    /// Forward a keystroke / input string to an exec session's stdin.
    pub async fn input(&self, session: u64, data: String) {
        let sender = self
            .sessions
            .lock()
            .unwrap()
            .get(&session)
            .map(|s| s.input.clone());
        if let Some(tx) = sender {
            let _ = tx.send(data).await;
        }
    }

    /// Resize an exec session's remote PTY to `cols` x `rows`.
    pub async fn resize(&self, session: u64, cols: u16, rows: u16) {
        let sender = self
            .sessions
            .lock()
            .unwrap()
            .get(&session)
            .map(|s| s.resize.clone());
        if let Some(tx) = sender {
            let _ = tx.send((cols, rows)).await;
        }
    }

    /// Close an exec session and abort its task.
    pub fn close(&self, session: u64) {
        if let Some(s) = self.sessions.lock().unwrap().remove(&session) {
            s.handle.abort();
        }
    }

    /// Abort every running exec session (used when a user's environment is
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
    use crate::test_util::TestSink;

    #[tokio::test(flavor = "multi_thread")]
    async fn failed_session_emits_exit_with_error() {
        // Empty cache: exec_shell cannot resolve the context, so the session
        // task must end by emitting exec:exit:<channel> with an error string —
        // on the CALLER'S channel, which the frontend was already subscribed
        // to before this call, not on the session id it has yet to receive.
        let manager = ExecManager::new(ClientCache::new_many(vec![]));
        let sink = Arc::new(TestSink::default());
        let id = manager
            .start(
                sink.clone(),
                "nope".into(),
                "ns".into(),
                "pod-a".into(),
                "exec-0-abcd".into(),
                ExecOpts::default(),
            )
            .await
            .expect("start allocates a session id");

        let exit_channel = "exec:exit:exec-0-abcd".to_string();
        for _ in 0..100 {
            let exits = sink.payloads_for(&exit_channel);
            if let Some(payload) = exits.first() {
                assert!(payload.is_string(), "exit carries an error: {payload}");
                // And on no other channel: an id-derived one is a channel the
                // caller could not have been listening on yet.
                assert!(
                    sink.payloads_for(&format!("exec:exit:{id}")).is_empty(),
                    "channels: {:?}",
                    sink.channels()
                );
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("exec:exit event never arrived");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn shutdown_all_aborts_sessions() {
        let manager = ExecManager::new(ClientCache::new_many(vec![]));
        let sink = Arc::new(TestSink::default());
        let id = manager
            .start(
                sink,
                "nope".into(),
                "ns".into(),
                "pod-a".into(),
                "exec-1-abcd".into(),
                ExecOpts::default(),
            )
            .await
            .expect("start allocates a session id");

        manager.shutdown_all(); // no panic; subsequent close is a no-op
        assert!(manager.sessions.lock().unwrap().is_empty());
        manager.close(id);
    }
}
