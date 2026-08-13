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
    live: Mutex<BTreeMap<String, Entry>>,
    /// Monotonic id stamped onto each stored entry, so a watch's own
    /// `on_dead` can prove the entry it is about to evict is still ITS watch
    /// (see `remove_if`) — a re-subscribe may have replaced it in the
    /// meantime, and evicting the replacement would silently unsubscribe a
    /// live watch the client believes in.
    next_generation: std::sync::atomic::AtomicU64,
}

struct Entry {
    generation: u64,
    handle: AbortHandle,
}

impl SubscriptionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a watch for `uri`. Re-subscribing to the same URI aborts the
    /// previous watch and replaces it, so one URI never has two.
    ///
    /// Takes ownership of `handle`, and disposes of it either way: on
    /// success it is stored (and a displaced previous watch, if any, is
    /// aborted); on `Err` — the cap was hit — `handle` itself is aborted
    /// before returning, since a rejected watch is useless by definition and
    /// the caller has no reference left to clean it up. Callers never need to
    /// abort on `Err`.
    ///
    /// Returns the stored entry's generation — the token `remove_if` needs.
    pub fn insert(&self, uri: String, handle: AbortHandle) -> Result<u64, String> {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        if !live.contains_key(&uri) && live.len() >= MAX_SUBSCRIPTIONS {
            // Before refusing, reap entries whose watch task already ended
            // (#195): the honest eviction path is the watcher's `on_dead`
            // callback, but a dead entry that slipped past it must not hold
            // a slot against a legitimate subscription. `is_finished` is
            // false for live watches, so this never evicts a working one.
            live.retain(|_, e| !e.handle.is_finished());
        }
        if !live.contains_key(&uri) && live.len() >= MAX_SUBSCRIPTIONS {
            handle.abort();
            return Err(format!(
                "too many subscriptions: the limit is {MAX_SUBSCRIPTIONS}; \
                 unsubscribe from something first"
            ));
        }
        let generation = self
            .next_generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if let Some(previous) = live.insert(uri, Entry { generation, handle }) {
            previous.handle.abort();
        }
        Ok(generation)
    }

    /// Abort and forget `uri`. Returns whether it was subscribed.
    pub fn remove(&self, uri: &str) -> bool {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        match live.remove(uri) {
            Some(entry) => {
                entry.handle.abort();
                true
            }
            None => false,
        }
    }

    /// Abort and forget `uri` ONLY if the stored entry still carries
    /// `generation` — i.e. it is still the watch that obtained that token
    /// from `insert`. The eviction path for a dead watch's `on_dead`
    /// callback: a client may have re-subscribed the same URI while the
    /// callback was in flight, and unconditional removal would abort the
    /// replacement — leaving the client subscribed to nothing while
    /// believing otherwise. Returns whether an entry was removed.
    pub fn remove_if(&self, uri: &str, generation: u64) -> bool {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        match live.get(uri) {
            Some(entry) if entry.generation == generation => {
                let entry = live.remove(uri).expect("checked present under the same lock");
                entry.handle.abort();
                true
            }
            _ => false,
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
        for entry in live.values() {
            entry.handle.abort();
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

    /// Awaits `join` under a short bound and asserts the task was actually
    /// cancelled.
    ///
    /// A bare `join.await` here would hang the whole test binary forever
    /// if a regression (e.g. deleting an `abort()` call) stops the task from
    /// ever being cancelled — verified: doing exactly that during review made
    /// the affected test hang past 20s instead of failing. Wrapping in
    /// `tokio::time::timeout` turns that into a fast, explicit, named
    /// failure instead of an indefinite stall that looks like an
    /// infrastructure problem rather than a test failure.
    async fn assert_aborted(join: tokio::task::JoinHandle<()>) {
        match tokio::time::timeout(std::time::Duration::from_secs(2), join).await {
            Ok(Ok(())) => panic!("watch task ran to completion instead of being aborted"),
            Ok(Err(err)) => assert!(
                err.is_cancelled(),
                "watch task ended for a reason other than cancellation: {err}"
            ),
            Err(_) => panic!("timed out after 2s waiting for the watch to be aborted — it was never cancelled"),
        }
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
        assert_aborted(first_join).await;
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

    /// A dead watch's late `on_dead` must not evict the watch that REPLACED
    /// it: `remove_if` only removes when the stored generation still matches
    /// the caller's token.
    #[tokio::test]
    async fn remove_if_spares_a_replacement_watch() {
        let r = SubscriptionRegistry::new();
        let stale = r.insert("k8s://c/ns/Pod/a".into(), spawn_forever()).unwrap();
        let (fresh_handle, fresh_join) = spawn_forever_joined();
        let fresh = r.insert("k8s://c/ns/Pod/a".into(), fresh_handle).unwrap();

        // The first watch died and its callback arrives late, carrying the
        // stale token — the replacement must survive.
        assert!(!r.remove_if("k8s://c/ns/Pod/a", stale));
        assert_eq!(r.len(), 1, "the replacement watch stays subscribed");

        // The replacement's own token still evicts it.
        assert!(r.remove_if("k8s://c/ns/Pod/a", fresh));
        assert_aborted(fresh_join).await;
    }

    /// The cheap half of #195: entries whose watch task already ended must
    /// not hold slots against a legitimate subscription — the cap check
    /// reaps them before refusing. (The honest path is the watcher's
    /// `on_dead` eviction; this is the backstop for a watcher that never
    /// reports.)
    #[tokio::test]
    async fn dead_entries_are_reaped_before_the_cap_refuses() {
        let r = SubscriptionRegistry::new();
        for i in 0..MAX_SUBSCRIPTIONS {
            // A task that completes instantly, awaited to completion so
            // `is_finished` is deterministically true by insert time.
            let join = tokio::spawn(async {});
            let handle = join.abort_handle();
            join.await.unwrap();
            r.insert(format!("k8s://c/ns/Pod/dead{i}"), handle).unwrap();
        }
        assert_eq!(r.len(), MAX_SUBSCRIPTIONS, "dead entries fill the registry");
        // At the cap, but every occupant is dead — the new subscription must
        // get a slot, not `too many subscriptions`.
        r.insert("k8s://c/ns/Pod/alive".into(), spawn_forever())
            .expect("reaping the dead entries makes room");
        assert_eq!(r.len(), 1, "only the live subscription remains");
    }

    /// `insert` takes ownership of `handle`; on the cap-rejection branch it
    /// must dispose of it by aborting it, not merely drop it —
    /// `AbortHandle`'s `Drop` does not cancel the underlying task, so a
    /// dropped-but-not-aborted handle would leak a detached, permanently
    /// running watch. The caller has already spawned the task and, once
    /// ownership of the handle moved into `insert`, has no reference left to
    /// clean it up — a client retrying against the cap would leak one watch
    /// per attempt, exactly the resource exhaustion `MAX_SUBSCRIPTIONS`
    /// exists to prevent.
    #[tokio::test]
    async fn cap_rejection_aborts_the_rejected_watch() {
        let r = SubscriptionRegistry::new();
        for i in 0..MAX_SUBSCRIPTIONS {
            r.insert(format!("k8s://c/ns/Pod/p{i}"), spawn_forever()).unwrap();
        }
        let (rejected, rejected_join) = spawn_forever_joined();
        r.insert("k8s://c/ns/Pod/one-too-many".into(), rejected)
            .expect_err("the cap should reject the extra subscription");
        assert_aborted(rejected_join).await;
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
        assert_aborted(a_join).await;
        assert_aborted(b_join).await;
    }
}
