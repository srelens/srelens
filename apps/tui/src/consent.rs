//! srectl's answer to a local agent's gated tool call: ask the person at the
//! keyboard. srelens's MCP server asks a [`ConfirmPolicy`] before any tool that
//! changes the cluster or reads a Secret; [`TuiConsent`] turns that question
//! into an [`AppEvent`] for the app to show, and waits for the reply.
//!
//! Every path that is not an explicit yes is a denial, and each says why, so
//! the agent can tell the user said no from nobody being there to ask.

use crate::event::AppEvent;
use crossterm::event::{KeyCode, KeyEvent};
use srelens_mcp::policy::{ConfirmPolicy, ConsentRequest, Decision};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;

/// How long a prompt waits for an answer before the call is refused. Nobody
/// at the keyboard is a "no", not a hang.
pub const CONSENT_TIMEOUT: Duration = Duration::from_secs(60);

/// What the app answers with when the user declines. Here, not in the app, so
/// the words the agent reads are fixed beside the other denials.
pub const DECLINED: &str = "declined in srectl";

/// One question for the user, with the way back to the waiting policy.
/// Dropping it unanswered is a denial ("closed").
#[derive(Debug)]
pub struct PendingConsent {
    pub request: ConsentRequest,
    /// The agent that raised the call, as srectl names it.
    pub agent: String,
    /// The context of the chat whose turn raised the call. The call may name
    /// another in its own arguments; that is the one it would act on.
    pub context: String,
    pub reply: oneshot::Sender<Decision>,
}

/// The prompts waiting on the user, oldest first; only the front one is on
/// screen. Each keeps when it arrived (its 60 s run from then), and the queue
/// keeps when the front one was first drawn (its keys arm from then).
#[derive(Debug, Default)]
pub struct ConsentQueue {
    waiting: VecDeque<(PendingConsent, Instant)>,
    front_shown: Option<Instant>,
}

impl ConsentQueue {
    pub fn push(&mut self, pending: PendingConsent) {
        self.waiting.push_back((pending, Instant::now()));
    }

    /// The front prompt is being drawn. Its keys arm from the first time, not
    /// from when it arrived: the screen is redrawn only between batches of
    /// events, and one event in a batch can take a while.
    pub fn shown(&mut self) {
        if !self.waiting.is_empty() {
            self.front_shown.get_or_insert_with(Instant::now);
        }
    }

    pub fn front(&self) -> Option<&PendingConsent> {
        self.waiting.front().map(|(pending, _)| pending)
    }

    pub fn is_empty(&self) -> bool {
        self.waiting.is_empty()
    }

    /// Answers the front prompt and shows the next.
    pub fn answer(&mut self, decision: Decision) {
        if let Some((pending, _)) = self.waiting.pop_front() {
            // The policy may have stopped waiting a moment ago; it denied then.
            let _ = pending.reply.send(decision);
        }
        self.front_changed();
    }

    /// Drops every prompt nobody waits on any more: the agent's wait timed
    /// out, or its turn was cancelled. Nothing is sent for them.
    pub fn prune_closed(&mut self) {
        self.drop_closed();
    }

    /// [`Self::prune_closed`], saying whether the front prompt was one of
    /// them: a key pressed for it was meant for a prompt that is gone.
    fn drop_closed(&mut self) -> bool {
        let front_closed = self
            .front()
            .is_some_and(|pending| pending.reply.is_closed());
        self.waiting
            .retain(|(pending, _)| !pending.reply.is_closed());
        if front_closed {
            self.front_changed();
        }
        front_closed
    }

    /// A key while a prompt is up: `y` approves, `n` or Esc declines, and
    /// every other key, Enter included, does nothing. Keys before the front
    /// prompt has been on screen for `arm_delay` are dropped, so one typed
    /// before the user could see it cannot answer it; so is a key for a
    /// prompt whose agent has stopped waiting, which leaves instead.
    pub fn press(&mut self, key: KeyEvent, arm_delay: Duration) {
        if self.drop_closed() || self.ready_in(arm_delay) > 0 {
            return;
        }
        let decision = match key.code {
            KeyCode::Char('y' | 'Y') => Decision::Approved,
            KeyCode::Char('n' | 'N') | KeyCode::Esc => Decision::Denied(DECLINED.into()),
            _ => return,
        };
        self.answer(decision);
    }

    /// Ctrl+C on a prompt: declines it at once, armed or not, and returns the
    /// context of the chat that asked, for the app to stop that turn. `None`
    /// when the front prompt had already gone.
    pub fn cancel(&mut self) -> Option<String> {
        if self.drop_closed() {
            return None;
        }
        let context = self.front()?.context.clone();
        self.answer(Decision::Denied(DECLINED.into()));
        Some(context)
    }

    /// Whole seconds, rounded up, before keys reach the front prompt; 0 once
    /// it has been on screen for `arm_delay`. A prompt not yet drawn has the
    /// whole delay to go.
    pub fn ready_in(&self, arm_delay: Duration) -> u64 {
        let on_screen = self.front_shown.map_or(Duration::ZERO, |s| s.elapsed());
        arm_delay
            .saturating_sub(on_screen)
            .as_millis()
            .div_ceil(1000) as u64
    }

    /// Whole seconds before the front prompt's agent stops waiting.
    pub fn seconds_left(&self) -> u64 {
        self.waiting.front().map_or(0, |(_, arrived)| {
            CONSENT_TIMEOUT.saturating_sub(arrived.elapsed()).as_secs()
        })
    }

    /// A new front prompt has not been drawn yet.
    fn front_changed(&mut self) {
        self.front_shown = None;
    }
}

pub struct TuiConsent {
    tx: UnboundedSender<AppEvent>,
    agent: String,
    context: String,
    timeout: Duration,
    waiting: Arc<AtomicUsize>,
}

impl TuiConsent {
    pub fn new(tx: UnboundedSender<AppEvent>, agent: &str, context: &str) -> Self {
        Self {
            tx,
            agent: agent.to_string(),
            context: context.to_string(),
            timeout: CONSENT_TIMEOUT,
            waiting: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Above zero while a prompt is open, for the app to know an agent is
    /// held up on the user.
    pub fn waiting(&self) -> Arc<AtomicUsize> {
        self.waiting.clone()
    }
}

fn no_answer(wait: Duration) -> String {
    format!("no answer in srectl within {} s", wait.as_secs())
}

/// Counts one open prompt for as long as it lives. A guard rather than a
/// decrement at the end of `confirm`, because the future is dropped, not
/// finished, when the agent's request is abandoned.
struct OpenPrompt(Arc<AtomicUsize>);

impl OpenPrompt {
    fn new(counter: &Arc<AtomicUsize>) -> Self {
        counter.fetch_add(1, Ordering::SeqCst);
        Self(counter.clone())
    }
}

impl Drop for OpenPrompt {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl ConfirmPolicy for TuiConsent {
    async fn confirm(&self, request: &ConsentRequest) -> Decision {
        // Counted before the send, so the app never sees a prompt the count
        // does not yet include.
        let _open = OpenPrompt::new(&self.waiting);
        let (reply, answer) = oneshot::channel();
        let pending = PendingConsent {
            request: request.clone(),
            agent: self.agent.clone(),
            context: self.context.clone(),
            reply,
        };
        if self.tx.send(AppEvent::AgentConsent(pending)).is_err() {
            return Decision::Denied("srectl could not show an approval prompt".into());
        }
        match tokio::time::timeout(self.timeout, answer).await {
            Ok(Ok(decision)) => decision,
            Ok(Err(_)) => Decision::Denied("the approval prompt in srectl was closed".into()),
            Err(_) => Decision::Denied(no_answer(self.timeout)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use srelens_capability::Impact;
    use srelens_mcp::policy::ConsentKind;
    use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

    fn request() -> ConsentRequest {
        ConsentRequest {
            tool: "k8s.deletePod".into(),
            args: json!({ "name": "api-0", "namespace": "prod" }),
            kind: ConsentKind::Destructive,
            impact: Impact::High,
            confirm_text: Some("Delete pod prod/api-0?".into()),
            caller: None,
        }
    }

    /// The prompt the app would put on screen.
    async fn next_prompt(rx: &mut UnboundedReceiver<AppEvent>) -> PendingConsent {
        match rx.recv().await.expect("a prompt was sent") {
            AppEvent::AgentConsent(pending) => pending,
            other => panic!("expected AgentConsent, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn the_users_answer_is_the_decision() {
        let (tx, mut rx) = unbounded_channel();
        let consent = Arc::new(TuiConsent::new(tx, "claude", "kind-dev"));
        let asked = tokio::spawn({
            let consent = consent.clone();
            async move { consent.confirm(&request()).await }
        });

        let pending = next_prompt(&mut rx).await;
        assert_eq!(pending.request, request());
        assert_eq!(pending.agent, "claude");
        assert_eq!(pending.context, "kind-dev");
        pending.reply.send(Decision::Approved).unwrap();

        assert_eq!(asked.await.unwrap(), Decision::Approved);
    }

    #[tokio::test]
    async fn the_users_refusal_reaches_the_agent_in_their_words() {
        let (tx, mut rx) = unbounded_channel();
        let consent = Arc::new(TuiConsent::new(tx, "claude", "kind-dev"));
        let asked = tokio::spawn({
            let consent = consent.clone();
            async move { consent.confirm(&request()).await }
        });

        next_prompt(&mut rx)
            .await
            .reply
            .send(Decision::Denied(DECLINED.into()))
            .unwrap();

        assert_eq!(
            asked.await.unwrap(),
            Decision::Denied("declined in srectl".into())
        );
    }

    #[tokio::test]
    async fn no_answer_in_time_is_a_denial() {
        let (tx, _rx) = unbounded_channel();
        let consent =
            TuiConsent::new(tx, "claude", "kind-dev").with_timeout(Duration::from_millis(10));

        match consent.confirm(&request()).await {
            Decision::Denied(why) => assert!(why.contains("no answer"), "got: {why}"),
            other => panic!("expected a denial, got {other:?}"),
        }
    }

    #[test]
    fn the_default_wait_is_sixty_seconds() {
        let (tx, _rx) = unbounded_channel();
        let consent = TuiConsent::new(tx, "claude", "kind-dev");

        assert_eq!(consent.timeout, Duration::from_secs(60));
        assert_eq!(
            no_answer(consent.timeout),
            "no answer in srectl within 60 s"
        );
    }

    #[tokio::test]
    async fn a_closed_prompt_is_a_denial() {
        let (tx, mut rx) = unbounded_channel();
        let consent = Arc::new(TuiConsent::new(tx, "claude", "kind-dev"));
        let asked = tokio::spawn({
            let consent = consent.clone();
            async move { consent.confirm(&request()).await }
        });

        drop(next_prompt(&mut rx).await);

        match asked.await.unwrap() {
            Decision::Denied(why) => assert!(why.contains("closed"), "got: {why}"),
            other => panic!("expected a denial, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn no_app_to_ask_is_a_denial() {
        let (tx, rx) = unbounded_channel();
        drop(rx);
        let consent = TuiConsent::new(tx, "claude", "kind-dev");

        match consent.confirm(&request()).await {
            Decision::Denied(why) => {
                assert_eq!(why, "srectl could not show an approval prompt")
            }
            other => panic!("expected a denial, got {other:?}"),
        }
        assert_eq!(consent.waiting().load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn waiting_counts_only_while_a_prompt_is_open() {
        let (tx, mut rx) = unbounded_channel();
        let consent = Arc::new(TuiConsent::new(tx, "claude", "kind-dev"));
        let waiting = consent.waiting();
        assert_eq!(waiting.load(Ordering::SeqCst), 0);
        let asked = tokio::spawn({
            let consent = consent.clone();
            async move { consent.confirm(&request()).await }
        });

        let pending = next_prompt(&mut rx).await;
        assert_eq!(waiting.load(Ordering::SeqCst), 1);
        pending.reply.send(Decision::Approved).unwrap();
        asked.await.unwrap();

        assert_eq!(waiting.load(Ordering::SeqCst), 0);
    }

    /// The agent's request can be abandoned (its connection drops, the MCP
    /// server gives up) and the future is dropped mid-wait; the prompt is no
    /// longer open then, so the count must not stay up.
    #[tokio::test]
    async fn a_cancelled_ask_stops_counting() {
        let (tx, mut rx) = unbounded_channel();
        let consent = TuiConsent::new(tx, "claude", "kind-dev");
        let waiting = consent.waiting();

        let abandoned =
            tokio::time::timeout(Duration::from_millis(10), consent.confirm(&request())).await;

        assert!(
            abandoned.is_err(),
            "nobody answered, so the wait is cut short"
        );
        let _ = next_prompt(&mut rx).await;
        assert_eq!(waiting.load(Ordering::SeqCst), 0);
    }
}
