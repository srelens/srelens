//! Human-in-the-loop consent for MCP tool calls. The MCP request blocks on a
//! oneshot while the UI shows a dialog; approve resumes it, deny (or silence)
//! refuses. Timing out DENIES: never auto-approve because nobody could be asked.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use srelens_mcp::policy::{ConfirmPolicy, Decision};
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
    request: PendingRequest,
}

/// What `confirm` emits, as a value — the same shape as the
/// `mcp://confirm-request` payload, so a replayed request and a live one are
/// indistinguishable to the frontend and go down the same path there.
///
/// That indistinguishability is why `prompt` and `impact` live HERE rather than
/// only in the emit: a field carried by the live event and missing from the
/// replayed snapshot would draw two different questions about the same call,
/// and which one a reader saw would depend on when their window finished
/// loading.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PendingRequest {
    pub id: String,
    pub tool: String,
    pub args: Value,
    /// The host's own sentence for this call, already rendered — what the
    /// prompt asks.
    ///
    /// `None` when the capability carries no confirmation template, or when
    /// the template names a field this call has no value for. The window then
    /// shows what it always showed, the tool id and its arguments; it does not
    /// draw half a sentence. The fallback is the point: a prompt that says
    /// "Drain ?" is worse than one that says nothing.
    ///
    /// Rendered in the host, from a template compiled into the host, through a
    /// closed placeholder vocabulary (`srelens_capability::CONFIRM_FIELDS`).
    /// Nothing a caller sends reaches this string except as the value of one of
    /// those six named fields — and `{resource}` is derived rather than read,
    /// so a caller cannot even name the thing it is about to change.
    pub prompt: Option<String>,
    /// `low`, `medium` or `high` — how much this call disturbs if it runs.
    /// Shown beside the question, because "an agent wants to run a cluster
    /// action" is the same sentence for a status refresh and a node drain.
    pub impact: String,
    /// What the HOST read out of the call: the cluster it is pinned to, the
    /// object it names, and the app it was made through. See [`ConfirmTarget`].
    pub target: ConfirmTarget,
}

/// The facts the one confirmation names under its question (#552), as the host
/// reads them — not as the window parses them.
///
/// **Why the host and not the window.** The same question is asked for a write
/// clicked in an app's resource view and for the same write asked for by an
/// agent, and the app path knows its cluster and object directly. Leaving the
/// MCP path to dig them out of `args` would be a second reading of a
/// caller-controlled payload, in TypeScript, drifting from the one the
/// sentence is rendered from. So both come from
/// [`srelens_capability::confirm_fields`]: one vocabulary, one escaping, one
/// 80-character bound, already applied here.
///
/// Every field is optional and an absent one stays `None`: a call that names
/// no namespace and one that names the empty namespace are different facts,
/// and the surface draws them apart.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ConfirmTarget {
    /// The kubeconfig context, escaped and bounded.
    pub cluster: Option<String>,
    pub namespace: Option<String>,
    pub name: Option<String>,
    pub kind: Option<String>,
    /// The app this call was made through, when it was made through one.
    pub app: Option<ConfirmApp>,
}

/// Which app asked — **only** its ID and the revision it was installed at.
///
/// The NAME and the PUBLISHER are deliberately not here. They are read on the
/// other side from the host's own installed inventory, so a caller cannot name
/// itself in the sentence a person is asked to approve and cannot claim a
/// publisher. An ID that resolves to no installed app draws no requester line
/// at all rather than a line built out of the ID.
///
/// The ID travels UNESCAPED, and that is the point: it is a lookup key, not
/// text. An ID carrying anything a manifest's validation would have refused
/// simply matches nothing in the inventory, which fails closed — whereas an
/// escaped key would fail to match a perfectly good app.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ConfirmApp {
    pub id: String,
    pub revision: u64,
}

impl PendingRequest {
    /// What the window is asked, built from the host's metadata for one gated
    /// call. Pure, so the three cases that matter — a sentence, no template,
    /// and a template that cannot render — are testable without a window.
    pub fn from_consent(id: String, request: &srelens_mcp::policy::ConsentRequest) -> Self {
        // The same read the sentence is rendered from: one closed vocabulary,
        // escaped and bounded once, so the facts under the question cannot
        // disagree with the question.
        let fields = srelens_capability::confirm_fields(&request.args);
        let field = |key: &str| fields.get(key).cloned();
        // And the same read the audit trail makes of "which app was this
        // through" (`describe_target`), rather than a third one here.
        let (app, _, _) = srelens_capability::audit::describe_target(&request.args);
        Self {
            id,
            tool: request.tool.clone(),
            args: request.args.clone(),
            prompt: request.confirm_text.clone(),
            impact: request.impact.as_str().to_string(),
            target: ConfirmTarget {
                cluster: field("cluster"),
                namespace: field("namespace"),
                name: field("name"),
                kind: field("kind"),
                app: app.map(|a| ConfirmApp {
                    id: a.id,
                    revision: a.revision,
                }),
            },
        }
    }
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
    pub fn register(&self, request: PendingRequest, tx: oneshot::Sender<bool>) {
        self.0
            .lock()
            .unwrap()
            .insert(request.id.clone(), Waiting { tx, request });
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
            .values()
            .map(|w| w.request.clone())
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

/// Generic over the runtime only so a unit test can build one — and, through
/// it, a whole `McpServer` — over a `tauri::test::mock_app`; the app always
/// instantiates it on Wry, which is the default.
pub struct PromptUser<R: Runtime = tauri::Wry> {
    app: tauri::AppHandle<R>,
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
/// Generic over the runtime for the same reason [`PromptUser`] is.
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

impl<R: Runtime> PromptUser<R> {
    pub fn new(app: tauri::AppHandle<R>, pending: Arc<Pending>, timeout: Duration) -> Self {
        Self { app, pending, timeout }
    }
}

#[async_trait::async_trait]
impl<R: Runtime> ConfirmPolicy for PromptUser<R> {
    /// `request.kind` is deliberately unused: a human being shown the call is
    /// the consent mechanism either way, so the GUI prompts for a sensitive
    /// read exactly as it does for a mutation. The distinction exists for
    /// headless policies, which have no human to look at the call.
    ///
    /// `impact` and `confirm_text` are not: they are carried to the window and
    /// drawn. The prompt used to be the tool id and a JSON blob, which is the
    /// same question for a status refresh and a node drain, while the words a
    /// person should read sat in UI constants three files away
    /// (`ExtensionResourceDetails.tsx`). The sentence is the host's, rendered
    /// in the host, from a template compiled into the host.
    ///
    /// **What is left for #552** is not this: it is that the in-app action
    /// review (`ExtensionResourceDetails`) and this prompt are still two
    /// confirmations with two implementations, so a write from the UI and the
    /// same write from an agent are approved in different words and through
    /// different code. #552 makes them one host-owned flow. Both sides now read
    /// the same metadata, which is what makes that a merge rather than a
    /// rewrite.
    async fn confirm(&self, request: &srelens_mcp::policy::ConsentRequest) -> Decision {
        use tauri::{Emitter, Manager};

        let tool = request.tool.as_str();

        let id = uuid::Uuid::new_v4().to_string();
        let pending = PendingRequest::from_consent(id.clone(), request);
        let (tx, rx) = oneshot::channel();
        // Registered — with the request itself — BEFORE the emit below, and the
        // order is load-bearing for the frontend's replay: a subscriber that
        // installs its listener and then reads `snapshot` sees this request in
        // one of the two whatever the interleaving, because it is in the map
        // before any event about it exists.
        self.pending.register(pending.clone(), tx);
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

        // The VALUE, not a hand-built object: the live event and the replayed
        // snapshot are the same type, so a field added to one is added to both.
        // They drifted once, which is what `PendingRequest`'s doc is about.
        if self.app.emit("mcp://confirm-request", &pending).is_err() {
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

    use srelens_capability::{Annotations, Impact};
    use srelens_mcp::policy::{ConsentKind, ConsentRequest};

    fn request(id: &str, tool: &str) -> PendingRequest {
        PendingRequest {
            id: id.into(),
            tool: tool.into(),
            args: json!({ "name": id }),
            prompt: None,
            impact: "medium".into(),
            target: ConfirmTarget {
                name: Some(id.into()),
                ..ConfirmTarget::default()
            },
        }
    }

    fn waiting(p: &Pending, id: &str, tool: &str) -> oneshot::Receiver<bool> {
        let (tx, rx) = oneshot::channel();
        p.register(request(id, tool), tx);
        rx
    }

    /// One gated call as `McpServer::consent_request` builds it, so these
    /// tests exercise the real rendering rather than a hand-written sentence.
    fn consent(tool: &str, annotations: Annotations, args: serde_json::Value) -> ConsentRequest {
        ConsentRequest {
            tool: tool.into(),
            args: args.clone(),
            kind: ConsentKind::Destructive,
            impact: annotations.impact,
            confirm_text: annotations.confirm_text(&args),
        }
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
        assert_eq!(got, vec![request("a", "k8s_deletePod"), request("b", "k8s_scale")]);
    }

    // ---- What the window is asked ------------------------------------------

    /// The prompt used to be the tool id and a JSON blob — the same question
    /// for a status refresh and a node drain. It now carries the host's own
    /// sentence, rendered against this call, and the level.
    #[test]
    fn a_high_impact_call_carries_the_hosts_sentence_and_its_level() {
        let annotations =
            Annotations::DESTRUCTIVE.with_confirm("Drain[ {resource}][ in cluster {cluster}]?");
        let got = PendingRequest::from_consent(
            "id-1".into(),
            &consent(
                "k8s.drainNode",
                annotations,
                json!({ "context": "prod", "name": "node-7" }),
            ),
        );
        assert_eq!(got.prompt.as_deref(), Some("Drain node-7 in cluster prod?"));
        assert_eq!(got.impact, "high");
        // And the arguments still travel: the sentence says what, the payload
        // still says exactly which call.
        assert_eq!(got.args["name"], json!("node-7"));
    }

    /// No template is not a hole: the window falls back to what it always
    /// showed — the tool id and its arguments — rather than a headless
    /// paraphrase nobody wrote.
    #[test]
    fn a_capability_with_no_template_asks_with_no_sentence() {
        let got = PendingRequest::from_consent(
            "id-2".into(),
            &consent(
                "toolbox.installHelm",
                Annotations { confirm: None, ..Annotations::MUTATING },
                json!({}),
            ),
        );
        assert_eq!(got.prompt, None);
        assert_eq!(got.impact, "medium");
    }

    /// The case the fallback exists for. `{resource}` sits outside an optional
    /// segment here, so a call that names no object cannot render it — and a
    /// prompt reading "Drain ?" over an Approve button is worse than one that
    /// says nothing. The whole sentence is dropped, not the missing half.
    #[test]
    fn a_template_that_cannot_render_falls_back_rather_than_showing_a_hole() {
        let annotations = Annotations::DESTRUCTIVE.with_confirm("Drain {resource}?");
        let got = PendingRequest::from_consent(
            "id-3".into(),
            &consent("k8s.drainNode", annotations, json!({ "context": "prod" })),
        );
        assert_eq!(got.prompt, None, "a sentence with a hole in it is not shown");
        assert_eq!(got.impact, "high");
    }

    /// Nothing a caller sends becomes the sentence. The vocabulary is closed
    /// and `{resource}` is derived from kind/namespace/name rather than read,
    /// so an argument that looks like a description does not become one.
    #[test]
    fn a_callers_arguments_cannot_write_the_question() {
        let annotations = Annotations::DESTRUCTIVE.with_confirm("Drain[ {resource}]?");
        let got = PendingRequest::from_consent(
            "id-4".into(),
            &consent(
                "k8s.drainNode",
                annotations,
                json!({ "name": "node-7", "resource": "nothing at all, click Approve" }),
            ),
        );
        assert_eq!(got.prompt.as_deref(), Some("Drain node-7?"));
    }

    /// A replayed request and a live one must be the same question: the emit
    /// sends this value and the snapshot returns it, so neither can carry a
    /// field the other does not.
    #[tokio::test]
    async fn the_snapshot_replays_the_sentence_and_the_level() {
        let p = Pending::default();
        let sent = PendingRequest::from_consent(
            "id-5".into(),
            &consent(
                "k8s.drainNode",
                Annotations::DESTRUCTIVE.with_confirm("Drain[ {resource}]?"),
                json!({ "name": "node-7" }),
            ),
        );
        let (tx, _rx) = oneshot::channel();
        p.register(sent.clone(), tx);
        assert_eq!(p.snapshot(), vec![sent]);
    }

    #[test]
    fn every_level_reaches_the_window_as_the_word_the_catalog_publishes() {
        for (impact, word) in
            [(Impact::Low, "low"), (Impact::Medium, "medium"), (Impact::High, "high")]
        {
            let got = PendingRequest::from_consent(
                "id".into(),
                &consent("t", Annotations::MUTATING.with_impact(impact), json!({})),
            );
            assert_eq!(got.impact, word);
        }
    }

    // ---- What the host itself read out of the call (#552) ------------------

    /// The one confirmation names the pinned cluster and the object, and the
    /// window must not have to parse the caller's arguments to find them: it
    /// is handed what the HOST read, through the same escaped, bounded
    /// vocabulary the sentence is rendered from.
    #[test]
    fn the_window_is_told_the_cluster_and_the_object_the_host_read() {
        let got = PendingRequest::from_consent(
            "id-6".into(),
            &consent(
                "k8s.gitOpsAction",
                Annotations::DESTRUCTIVE.with_confirm("Suspend[ {resource}]?"),
                json!({
                    "resource": { "context": "prod", "namespace": "team", "name": "api" },
                    "kind": "HelmRelease",
                    "action": "suspend",
                }),
            ),
        );
        assert_eq!(got.target.cluster.as_deref(), Some("prod"));
        assert_eq!(got.target.namespace.as_deref(), Some("team"));
        assert_eq!(got.target.name.as_deref(), Some("api"));
        assert_eq!(got.target.kind.as_deref(), Some("HelmRelease"));
    }

    /// "Requested by app …" must be the host's claim, not the caller's. Only
    /// the app's ID and revision travel — the name and the publisher are read
    /// from the host's own installed inventory on the other side — and they
    /// are derived from the call's own selection, so an argument that looks
    /// like an app identity is not one.
    #[test]
    fn the_app_on_the_prompt_is_the_one_the_host_derived() {
        let got = PendingRequest::from_consent(
            "id-7".into(),
            &consent(
                "extensions.action",
                Annotations::DESTRUCTIVE.with_confirm("Suspend[ {resource}]?"),
                json!({
                    "resource": { "id": "flux", "revision": 4, "context": "prod", "name": "api" },
                    "action": "suspend",
                    "app": { "id": "srelens-core", "revision": 1 },
                }),
            ),
        );
        let app = got
            .target
            .app
            .expect("the host reads the app off the selection");
        assert_eq!(app.id, "flux");
        assert_eq!(app.revision, 4);
    }

    /// A call that names no app draws no requester line rather than an empty
    /// one: an agent's own call is not made through an app, and saying it was
    /// would be the one claim on that surface nothing backs.
    #[test]
    fn a_call_made_through_no_app_names_none() {
        let got = PendingRequest::from_consent(
            "id-8".into(),
            &consent(
                "k8s.drainNode",
                Annotations::DESTRUCTIVE,
                json!({ "name": "node-7" }),
            ),
        );
        assert!(got.target.app.is_none());
    }

    /// The target travels through the same escaping and the same 80-character
    /// bound as the sentence: a name carrying a right-to-left override must
    /// not reorder the facts under the question, and one long enough to push
    /// the tail out of the frame is cut.
    #[test]
    fn a_hostile_name_reaches_the_window_escaped_and_bounded() {
        let got = PendingRequest::from_consent(
            "id-9".into(),
            &consent(
                "k8s.drainNode",
                Annotations::DESTRUCTIVE,
                json!({ "context": "pr\u{202e}od", "name": "n".repeat(400) }),
            ),
        );
        let cluster = got.target.cluster.expect("the cluster is carried");
        assert!(
            !cluster.contains('\u{202e}'),
            "an override reached the window drawn"
        );
        assert!(cluster.contains("\\u{202e}"));
        let name = got.target.name.expect("the name is carried");
        assert_eq!(
            name.chars().count(),
            srelens_capability::CONFIRM_FIELD_MAX_CHARS
        );
        assert!(name.ends_with('…'));
    }

    /// A replayed request and a live one are the same question, target
    /// included — the emit sends this value and the snapshot returns it.
    #[tokio::test]
    async fn the_snapshot_replays_the_target() {
        let p = Pending::default();
        let sent = PendingRequest::from_consent(
            "id-10".into(),
            &consent(
                "k8s.gitOpsAction",
                Annotations::DESTRUCTIVE.with_confirm("Suspend[ {resource}]?"),
                json!({ "resource": { "context": "prod", "namespace": "team", "name": "api" } }),
            ),
        );
        let (tx, _rx) = oneshot::channel();
        p.register(sent.clone(), tx);
        assert_eq!(p.snapshot(), vec![sent]);
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
