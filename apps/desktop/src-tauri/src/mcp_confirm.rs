//! Human-in-the-loop consent for MCP tool calls. The MCP request blocks on a
//! oneshot while the UI shows a dialog; approve resumes it, deny (or silence)
//! refuses. Timing out DENIES: never auto-approve because nobody could be asked.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use srelens_mcp::policy::{ConfirmPolicy, ConsentKind, Decision};
use tokio::sync::oneshot;

#[derive(Default)]
pub struct Pending(Mutex<HashMap<String, oneshot::Sender<bool>>>);

impl Pending {
    pub fn register(&self, id: String, tx: oneshot::Sender<bool>) {
        self.0.lock().unwrap().insert(id, tx);
    }

    /// Returns false when the id is unknown (already answered or timed out).
    pub fn resolve(&self, id: &str, approved: bool) -> bool {
        match self.0.lock().unwrap().remove(id) {
            Some(tx) => tx.send(approved).is_ok(),
            None => false,
        }
    }

    pub fn forget(&self, id: &str) {
        self.0.lock().unwrap().remove(id);
    }

    /// Deny everything still waiting — used when the server is toggled off so
    /// in-flight calls fail fast instead of hanging until timeout.
    pub fn deny_all(&self) {
        for (_, tx) in self.0.lock().unwrap().drain() {
            let _ = tx.send(false);
        }
    }
}

pub struct PromptUser {
    app: tauri::AppHandle,
    pending: Arc<Pending>,
    timeout: Duration,
}

/// Cleans up one confirmation however `confirm` ends — answered, timed out,
/// or the future DROPPED mid-await (Stop aborts the native turn's task, and a
/// killed CLI tears down the HTTP request task blocked here; neither reaches
/// any code after the `.await`). Dropping this forgets the `Pending` entry
/// and broadcasts `mcp://confirm-resolved`, so neither the app-wide modal nor
/// the transcript's inline card can outlive the request they prompt for.
struct ResolveOnDrop {
    app: tauri::AppHandle,
    pending: Arc<Pending>,
    id: String,
}

impl Drop for ResolveOnDrop {
    fn drop(&mut self) {
        use tauri::Emitter;
        self.pending.forget(&self.id);
        let _ = self.app.emit("mcp://confirm-resolved", serde_json::json!({ "id": self.id }));
    }
}

impl PromptUser {
    pub fn new(app: tauri::AppHandle, pending: Arc<Pending>, timeout: Duration) -> Self {
        Self { app, pending, timeout }
    }
}

#[async_trait::async_trait]
impl ConfirmPolicy for PromptUser {
    /// `kind` is deliberately unused: a human being shown the tool name and its
    /// arguments is the consent mechanism either way, so the GUI prompts for a
    /// sensitive read exactly as it does for a mutation. The distinction exists
    /// for headless policies, which have no human to look at the call.
    async fn confirm(&self, tool: &str, args: &Value, _kind: ConsentKind) -> Decision {
        use tauri::{Emitter, Manager};

        let id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        self.pending.register(id.clone(), tx);
        // The same request is rendered in TWO places — the app-wide modal and
        // the assistant transcript's inline card — and answering in one only
        // clears that one's own queue. This guard broadcasts the resolution on
        // EVERY exit from this function, including cancellation (see its doc),
        // so no stale prompt lingers anywhere and no `Pending` entry leaks.
        let _cleanup =
            ResolveOnDrop { app: self.app.clone(), pending: self.pending.clone(), id: id.clone() };

        // A dialog behind another window is indistinguishable from a hang.
        let Some(win) = self.app.get_webview_window("main") else {
            return Decision::Denied(format!(
                "no window available to confirm `{tool}`"
            ));
        };
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();

        if self
            .app
            .emit(
                "mcp://confirm-request",
                serde_json::json!({ "id": id, "tool": tool, "args": args }),
            )
            .is_err()
        {
            return Decision::Denied("srelens could not show a confirmation dialog".into());
        }

        match tokio::time::timeout(self.timeout, rx).await {
            Ok(Ok(true)) => Decision::Approved,
            Ok(Ok(false)) => Decision::Denied(format!("user declined `{tool}`")),
            Ok(Err(_)) => Decision::Denied("confirmation channel closed".into()),
            Err(_) => Decision::Denied(format!("no response to the confirmation for `{tool}`")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn resolve_delivers_the_answer() {
        let p = Pending::default();
        let (tx, rx) = oneshot::channel();
        p.register("abc".into(), tx);
        assert!(p.resolve("abc", true));
        assert_eq!(rx.await.unwrap(), true);
    }

    #[tokio::test]
    async fn resolving_an_unknown_id_is_reported() {
        let p = Pending::default();
        assert!(!p.resolve("nope", true));
    }

    #[tokio::test]
    async fn deny_all_releases_every_waiter() {
        let p = Pending::default();
        let (tx1, rx1) = oneshot::channel();
        let (tx2, rx2) = oneshot::channel();
        p.register("a".into(), tx1);
        p.register("b".into(), tx2);
        p.deny_all();
        assert_eq!(rx1.await.unwrap(), false);
        assert_eq!(rx2.await.unwrap(), false);
    }

    #[tokio::test]
    async fn an_id_can_only_be_answered_once() {
        let p = Pending::default();
        let (tx, _rx) = oneshot::channel();
        p.register("a".into(), tx);
        assert!(p.resolve("a", true));
        assert!(!p.resolve("a", false), "second answer must not be accepted");
    }
}
