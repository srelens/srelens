//! Human-in-the-loop consent for MCP tool calls. The MCP request blocks on a
//! oneshot while the UI shows a dialog; approve resumes it, deny (or silence)
//! refuses. Timing out DENIES: never auto-approve because nobody could be asked.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use srelens_mcp::policy::{ConfirmPolicy, ConsentKind, Decision};
use tauri::Runtime;
use tokio::sync::oneshot;

/// One confirmation still waiting on an answer: the channel that answer goes
/// down, AND the request it is an answer to.
///
/// The request is kept for a subscriber that turns up late. `confirm` emits
/// `mcp://confirm-request` exactly once, and the frontend's listener is a React
/// effect that runs only once the new design's chunks have downloaded and the
/// tree has mounted — so a request raised while that was still happening used
/// to be denied on timeout with nothing ever drawn. Three rounds moved the
/// listener earlier and each left an earlier window; the fix is that whoever
/// subscribes is handed what is already waiting (`Pending::snapshot`, served by
/// `mcp_confirm_pending`), which needs the map to hold the question and not
/// only the answer channel.
struct Waiting {
    tx: oneshot::Sender<bool>,
    tool: String,
    args: Value,
}

/// What `confirm` emits, as a value — the same shape as the
/// `mcp://confirm-request` payload, so a replayed request and a live one are
/// indistinguishable to the frontend and go down the same path there.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PendingRequest {
    pub id: String,
    pub tool: String,
    pub args: Value,
}

/// Every confirmation waiting on an answer, by id.
///
/// **The map IS the live set** — an entry is present for exactly as long as a
/// `confirm` future is awaiting its answer. `ResolveOnDrop` guarantees the
/// second half: it forgets the entry on every exit from `confirm`, including a
/// dropped future. That invariant is what makes `snapshot` correct — a
/// replayed request is never one whose answer could no longer land — and it
/// must not be weakened.
#[derive(Default)]
pub struct Pending(Mutex<HashMap<String, Waiting>>);

impl Pending {
    pub fn register(&self, id: String, tool: String, args: Value, tx: oneshot::Sender<bool>) {
        self.0.lock().unwrap().insert(id, Waiting { tx, tool, args });
    }

    /// Returns false when the id is unknown (already answered or timed out).
    pub fn resolve(&self, id: &str, approved: bool) -> bool {
        match self.0.lock().unwrap().remove(id) {
            Some(w) => w.tx.send(approved).is_ok(),
            None => false,
        }
    }

    /// Everything still waiting, at this instant, with enough to draw each
    /// prompt. In no particular order: the frontend queues them by arrival and
    /// merges by id, and two requests raised while it was not yet listening
    /// have no arrival order it could honour anyway.
    pub fn snapshot(&self) -> Vec<PendingRequest> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .map(|(id, w)| PendingRequest { id: id.clone(), tool: w.tool.clone(), args: w.args.clone() })
            .collect()
    }

    pub fn forget(&self, id: &str) {
        self.0.lock().unwrap().remove(id);
    }

    /// Deny everything still waiting — used when the server is toggled off so
    /// in-flight calls fail fast instead of hanging until timeout.
    pub fn deny_all(&self) {
        for (_, w) in self.0.lock().unwrap().drain() {
            let _ = w.tx.send(false);
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
///
/// Generic over the runtime only so the unit test below can hold one over a
/// `tauri::test::mock_app`; `PromptUser` itself is Wry.
struct ResolveOnDrop<R: Runtime> {
    app: tauri::AppHandle<R>,
    pending: Arc<Pending>,
    id: String,
}

impl<R: Runtime> Drop for ResolveOnDrop<R> {
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
        // Registered — with the request itself — BEFORE the emit below, and the
        // order is load-bearing for the frontend's replay: a subscriber that
        // installs its listener and then reads `snapshot` sees this request in
        // one of the two whatever the interleaving, because it is in the map
        // before any event about it exists.
        self.pending.register(id.clone(), tool.to_string(), args.clone(), tx);
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
    use serde_json::json;

    fn waiting(p: &Pending, id: &str, tool: &str) -> oneshot::Receiver<bool> {
        let (tx, rx) = oneshot::channel();
        p.register(id.to_string(), tool.to_string(), json!({ "name": id }), tx);
        rx
    }

    #[tokio::test]
    async fn resolve_delivers_the_answer() {
        let p = Pending::default();
        let rx = waiting(&p, "abc", "k8s_scale");
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
        let rx1 = waiting(&p, "a", "toolA");
        let rx2 = waiting(&p, "b", "toolB");
        p.deny_all();
        assert_eq!(rx1.await.unwrap(), false);
        assert_eq!(rx2.await.unwrap(), false);
    }

    #[tokio::test]
    async fn an_id_can_only_be_answered_once() {
        let p = Pending::default();
        let _rx = waiting(&p, "a", "toolA");
        assert!(p.resolve("a", true));
        assert!(!p.resolve("a", false), "second answer must not be accepted");
    }

    // ---- The snapshot: what a subscriber who turned up late is handed -------

    /// The map holds the REQUEST, not only its answer channel, so a subscriber
    /// that mounted after the emit can be handed what is still waiting — with
    /// enough to draw the prompt: the tool and its arguments, not just an id.
    #[tokio::test]
    async fn snapshot_carries_every_waiting_request_with_its_tool_and_arguments() {
        let p = Pending::default();
        let _rx1 = waiting(&p, "a", "k8s_deletePod");
        let _rx2 = waiting(&p, "b", "k8s_scale");
        let mut got = p.snapshot();
        got.sort_by(|x, y| x.id.cmp(&y.id));
        assert_eq!(
            got,
            vec![
                PendingRequest { id: "a".into(), tool: "k8s_deletePod".into(), args: json!({ "name": "a" }) },
                PendingRequest { id: "b".into(), tool: "k8s_scale".into(), args: json!({ "name": "b" }) },
            ]
        );
    }

    #[test]
    fn snapshot_of_nothing_waiting_is_empty() {
        assert!(Pending::default().snapshot().is_empty());
    }

    /// The forget invariant, on every exit `Pending` has: the map IS the live
    /// set, so a replayed snapshot can never hand a late subscriber a request
    /// that has already been answered, denied wholesale, or forgotten. Weaken
    /// any of these and a replay would draw a prompt over a settled call —
    /// one whose answer can no longer land.
    #[tokio::test]
    async fn an_answered_request_leaves_the_snapshot() {
        let p = Pending::default();
        let _rx = waiting(&p, "a", "toolA");
        let _rx_b = waiting(&p, "b", "toolB");
        assert!(p.resolve("a", false));
        let ids: Vec<String> = p.snapshot().into_iter().map(|r| r.id).collect();
        assert_eq!(ids, vec!["b".to_string()]);
    }

    #[tokio::test]
    async fn a_forgotten_request_leaves_the_snapshot() {
        let p = Pending::default();
        let _rx = waiting(&p, "a", "toolA");
        p.forget("a");
        assert!(p.snapshot().is_empty());
    }

    #[tokio::test]
    async fn deny_all_empties_the_snapshot() {
        let p = Pending::default();
        let _rx1 = waiting(&p, "a", "toolA");
        let _rx2 = waiting(&p, "b", "toolB");
        p.deny_all();
        assert!(p.snapshot().is_empty());
    }

    /// And the guard that `confirm` holds is what ties the map to the future:
    /// dropping it — which every exit from `confirm` does, including a dropped
    /// future — takes the entry out. This is the invariant the replay stands on.
    #[tokio::test]
    async fn dropping_the_guard_forgets_the_request() {
        let app = tauri::test::mock_app();
        let pending = Arc::new(Pending::default());
        let _rx = waiting(&pending, "a", "toolA");
        assert_eq!(pending.snapshot().len(), 1);
        {
            let _cleanup =
                ResolveOnDrop { app: app.handle().clone(), pending: pending.clone(), id: "a".into() };
        }
        assert!(pending.snapshot().is_empty(), "the dropped guard must forget its entry");
    }
}
