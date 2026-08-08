//! Live subscriptions for the stdio transport. Owned by the `serve` loop, so a
//! watch cannot outlive the session that asked for it.

use std::collections::BTreeMap;
use std::sync::Mutex;

use tokio::task::AbortHandle;

/// Prompt bodies and agents both loop, so an unbounded subscription count is a
/// resource-exhaustion path. 32 concurrent watches is far more than a human
/// triage session needs.
pub const MAX_SUBSCRIPTIONS: usize = 32;

/// Live subscriptions, keyed by canonical URI.
///
/// Uses a std `Mutex` around a plain map: every operation is a short,
/// non-blocking map edit plus an `abort()` call, so nothing is held across an
/// await.
#[derive(Default)]
pub struct SubscriptionRegistry {
    live: Mutex<BTreeMap<String, AbortHandle>>,
}

impl SubscriptionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a watch for `uri`. Re-subscribing to the same URI aborts the
    /// previous watch and replaces it, so one URI never has two.
    pub fn insert(&self, uri: String, handle: AbortHandle) -> Result<(), String> {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        if !live.contains_key(&uri) && live.len() >= MAX_SUBSCRIPTIONS {
            return Err(format!(
                "too many subscriptions: the limit is {MAX_SUBSCRIPTIONS}; \
                 unsubscribe from something first"
            ));
        }
        if let Some(previous) = live.insert(uri, handle) {
            previous.abort();
        }
        Ok(())
    }

    /// Abort and forget `uri`. Returns whether it was subscribed.
    pub fn remove(&self, uri: &str) -> bool {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        match live.remove(uri) {
            Some(handle) => {
                handle.abort();
                true
            }
            None => false,
        }
    }

    pub fn len(&self) -> usize {
        self.live.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Abort every live watch. Called when the serve loop exits, so no watch
    /// outlives the session.
    pub fn abort_all(&self) {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        for handle in live.values() {
            handle.abort();
        }
        live.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spawn_forever() -> tokio::task::AbortHandle {
        tokio::spawn(async { std::future::pending::<()>().await }).abort_handle()
    }

    /// Spawns a task that never finishes on its own, returning both the
    /// `AbortHandle` (what `SubscriptionRegistry` stores) and the
    /// `JoinHandle`.
    ///
    /// Tests that need to observe an abort use the `JoinHandle` rather than
    /// polling `AbortHandle::is_finished()` synchronously: `abort()` only
    /// *requests* cancellation, and `is_finished()` only flips once the
    /// runtime has actually reaped the task. This crate's `#[tokio::test]`
    /// runs on a current-thread runtime (`crates/mcp` enables tokio's `rt`
    /// feature, not `rt-multi-thread`), so a freshly spawned task is never
    /// even polled before a synchronous post-`abort()` check runs — measured
    /// empirically at 0/50 passes for both `is_finished()`-based assertions
    /// this replaced. Awaiting the `JoinHandle` instead synchronizes on the
    /// cancellation actually completing, so it is deterministic rather than
    /// racing the executor.
    fn spawn_forever_joined() -> (tokio::task::AbortHandle, tokio::task::JoinHandle<()>) {
        let join = tokio::spawn(async { std::future::pending::<()>().await });
        let abort = join.abort_handle();
        (abort, join)
    }

    #[tokio::test]
    async fn insert_then_remove_tracks_length() {
        let r = SubscriptionRegistry::new();
        r.insert("k8s://c/ns/Pod/a".into(), spawn_forever()).unwrap();
        assert_eq!(r.len(), 1);
        assert!(r.remove("k8s://c/ns/Pod/a"));
        assert_eq!(r.len(), 0);
    }

    #[tokio::test]
    async fn removing_an_unknown_uri_reports_false() {
        let r = SubscriptionRegistry::new();
        assert!(!r.remove("k8s://c/ns/Pod/nope"));
    }

    /// Re-subscribing must not leave two watches running for one URI — the leak
    /// this type exists to prevent.
    #[tokio::test]
    async fn re_inserting_aborts_the_previous_watch() {
        let r = SubscriptionRegistry::new();
        let (first, first_join) = spawn_forever_joined();
        r.insert("k8s://c/ns/Pod/a".into(), first).unwrap();
        r.insert("k8s://c/ns/Pod/a".into(), spawn_forever()).unwrap();
        assert_eq!(r.len(), 1, "one URI, one subscription");
        let err = first_join.await.unwrap_err();
        assert!(err.is_cancelled(), "the replaced watch must have been aborted");
    }

    #[tokio::test]
    async fn the_cap_is_enforced_and_names_itself() {
        let r = SubscriptionRegistry::new();
        for i in 0..MAX_SUBSCRIPTIONS {
            r.insert(format!("k8s://c/ns/Pod/p{i}"), spawn_forever()).unwrap();
        }
        let e = r.insert("k8s://c/ns/Pod/one-too-many".into(), spawn_forever()).unwrap_err();
        assert!(e.contains(&MAX_SUBSCRIPTIONS.to_string()), "got: {e}");
        assert_eq!(r.len(), MAX_SUBSCRIPTIONS);
    }

    /// The `!contains_key` half of the cap guard (`if !live.contains_key(&uri)
    /// && live.len() >= MAX_SUBSCRIPTIONS`) exists precisely so that, once
    /// already at the cap, re-subscribing to a URI already held still
    /// succeeds instead of being rejected. Deleting that clause leaves every
    /// other test in this module green, so it needs its own coverage.
    #[tokio::test]
    async fn re_subscribing_at_the_cap_still_succeeds() {
        let r = SubscriptionRegistry::new();
        for i in 0..MAX_SUBSCRIPTIONS {
            r.insert(format!("k8s://c/ns/Pod/p{i}"), spawn_forever()).unwrap();
        }
        assert_eq!(r.len(), MAX_SUBSCRIPTIONS);

        r.insert("k8s://c/ns/Pod/p0".into(), spawn_forever())
            .expect("re-subscribing to an already-held URI must succeed even at the cap");
        assert_eq!(r.len(), MAX_SUBSCRIPTIONS, "replacing in place must not change the count");
    }

    #[tokio::test]
    async fn abort_all_stops_every_watch() {
        let r = SubscriptionRegistry::new();
        let (a, a_join) = spawn_forever_joined();
        let (b, b_join) = spawn_forever_joined();
        r.insert("a".into(), a).unwrap();
        r.insert("b".into(), b).unwrap();
        r.abort_all();
        assert_eq!(r.len(), 0);
        assert!(a_join.await.unwrap_err().is_cancelled());
        assert!(b_join.await.unwrap_err().is_cancelled());
    }
}
