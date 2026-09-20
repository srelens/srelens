//! Caps for unbounded app-reader Kubernetes lists (#609).
//!
//! `k8s.listCustomResource` and `k8s.listEvents` used to issue one uncapped
//! list and hold the whole response. A large cluster or busy namespace could
//! stall or exhaust the desktop app. Both now page with `limit`/`continue` and
//! stop at [`APP_LIST_CAP`], telling the caller when they were cut off.

use crate::connect::request_timeout;
use kube::api::{Api, ListParams};
use kube::Resource;
use serde::de::DeserializeOwned;
use srelens_capability::CapabilityError;
use std::fmt::Debug;
use std::time::Duration;

/// Page size when walking the API server's continue token.
pub const APP_LIST_PAGE: u32 = 500;

/// Hard ceiling on rows returned by app-reader lists.
pub const APP_LIST_CAP: usize = 2_000;

/// Most pages one walk will request before giving up with what it has.
///
/// The row cap alone does not bound the walk: a selector-filtered list can
/// answer page after page with no matching item and a fresh continue token,
/// since the API server pages the unfiltered collection and filters each
/// chunk. Each such page spends a full per-page budget, so without this the
/// number of requests — and the wall clock — was unbounded. Twenty pages is
/// ten thousand objects scanned, five times what the cap can return; a walk
/// that hits it stops and reports itself truncated, which is true.
pub const APP_LIST_MAX_PAGES: usize = 20;

/// Why a capped list did not finish.
///
/// Its own type rather than `kube::Error` because a timeout is not one: the
/// budget belongs to a REQUEST, and a capped list makes up to four of them.
/// Callers used to wrap the whole walk in one `request_timeout()`, which gave
/// four pages a single page's time — a cluster slow enough to need paging was
/// the one most likely to be cut off by it, and the message still said the
/// list timed out rather than which part did.
#[derive(Debug)]
pub enum ListCappedError {
    /// The API server answered, with an error.
    Api(kube::Error),
    /// One page did not answer within its budget. Carries how many pages had
    /// already been read, so the message can say the walk was partway through
    /// rather than implying the first request hung, and the budget that was
    /// actually applied — not whatever the process-wide setting reads when the
    /// message is rendered, which may differ from it.
    Timeout { pages_read: usize, budget: Duration },
    /// A page answered with the same continue token it was asked for. The
    /// server says more remain but has not moved; following it would read the
    /// same page again until the page bound and hand back the duplicates as a
    /// list. Refused instead, naming the token.
    StuckContinuation { token: String },
}

/// `5s`, or `100ms` for a budget under a second — the way a reader would say it.
fn describe_budget(budget: Duration) -> String {
    if budget.subsec_millis() == 0 {
        format!("{}s", budget.as_secs())
    } else {
        format!("{}ms", budget.as_millis())
    }
}

impl std::fmt::Display for ListCappedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Api(error) => write!(f, "{error}"),
            // "Timed out" and not "the cluster did not answer": a timeout says
            // only that the answer did not arrive in time, not whose fault
            // that was.
            Self::Timeout {
                pages_read: 0,
                budget,
            } => write!(
                f,
                "Kubernetes list request timed out after {}",
                describe_budget(*budget)
            ),
            Self::Timeout { pages_read, budget } => write!(
                f,
                "Kubernetes list request timed out after {} ({pages_read} page(s) already read)",
                describe_budget(*budget)
            ),
            Self::StuckContinuation { token } => write!(
                f,
                "Kubernetes list pagination did not advance: the server repeated continue token {token:?}"
            ),
        }
    }
}

impl ListCappedError {
    /// The capability error a handler returns for this failure.
    ///
    /// An API error keeps its own words untouched — the frontend parses them
    /// for the cluster-login prompt — and only a timeout gets a sentence of
    /// ours, naming `what` was being listed. One place for both callers
    /// (`k8s.listCustomResource`, `k8s.listEvents`) so they cannot drift.
    pub fn into_capability_error(self, what: &str) -> CapabilityError {
        match self {
            Self::Api(error) => CapabilityError::Handler(error.to_string()),
            timeout @ Self::Timeout { .. } => {
                CapabilityError::Handler(format!("{what} timed out: {timeout}"))
            }
            stuck @ Self::StuckContinuation { .. } => {
                CapabilityError::Handler(format!("{what} failed: {stuck}"))
            }
        }
    }
}

impl std::error::Error for ListCappedError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Api(error) => Some(error),
            Self::Timeout { .. } | Self::StuckContinuation { .. } => None,
        }
    }
}

impl From<kube::Error> for ListCappedError {
    fn from(error: kube::Error) -> Self {
        Self::Api(error)
    }
}

/// List until the cap or the end of the collection.
///
/// `truncated` is true when more items remain unread (a continue token) or when
/// the last page had to be cut to fit the cap.
///
/// An empty page with a continue token is not the end: a selector-filtered
/// chunk may return zero items while more matching objects remain later
/// (Kubernetes list pagination). Stop only when the continue token is absent.
///
/// [`request_timeout`] is applied PER PAGE, here, because that is what it
/// measures: every other capability spends it on one `api.list`. A caller must
/// not wrap this call in a timeout of its own — that would divide one
/// request's budget across every page of the walk. The walk as a whole is
/// bounded by [`APP_LIST_MAX_PAGES`] instead.
pub async fn list_capped<K>(
    api: &Api<K>,
    base: ListParams,
) -> Result<(Vec<K>, bool), ListCappedError>
where
    K: Resource + Clone + DeserializeOwned + Debug,
{
    list_capped_within(api, base, request_timeout()).await
}

/// [`list_capped`] with the per-page budget given rather than read from the
/// process-wide setting.
///
/// Exists so the tests can assert the budget's BEHAVIOUR in milliseconds
/// instead of seconds. The alternative — moving the global with
/// `set_request_timeout_secs` — would reach every other test running beside
/// them in the same process, and its one-second floor would make a "slow page"
/// case cost seconds of wall clock.
pub async fn list_capped_within<K>(
    api: &Api<K>,
    base: ListParams,
    per_page: Duration,
) -> Result<(Vec<K>, bool), ListCappedError>
where
    K: Resource + Clone + DeserializeOwned + Debug,
{
    let mut items = Vec::new();
    let mut token: Option<String> = None;
    let mut pages_read = 0usize;
    loop {
        let mut params = base.clone().limit(APP_LIST_PAGE);
        if let Some(ref t) = token {
            params = params.continue_token(t);
        }
        let page = tokio::time::timeout(per_page, api.list(&params))
            .await
            .map_err(|_| ListCappedError::Timeout {
                pages_read,
                budget: per_page,
            })??;
        pages_read += 1;
        let next = page.metadata.continue_.filter(|t| !t.is_empty());
        // A token that did not advance is a server that will serve this page
        // forever; the items it carried are not appended, since they would be
        // the same rows a second time.
        if next.is_some() && next == token {
            return Err(ListCappedError::StuckContinuation {
                token: next.unwrap_or_default(),
            });
        }
        items.extend(page.items);
        token = next;
        if items.len() >= APP_LIST_CAP {
            let truncated = items.len() > APP_LIST_CAP || token.is_some();
            items.truncate(APP_LIST_CAP);
            return Ok((items, truncated));
        }
        if token.is_none() {
            return Ok((items, false));
        }
        // More remain, but this walk has spent its pages: what was read is
        // returned as a cut-off list rather than requesting on indefinitely.
        if pages_read >= APP_LIST_MAX_PAGES {
            return Ok((items, true));
        }
    }
}

/// Apply a row cap to an already-collected list (for tests and callers that
/// gather items another way).
pub fn apply_cap<T>(mut items: Vec<T>, more_remain: bool) -> (Vec<T>, bool) {
    let truncated = more_remain || items.len() > APP_LIST_CAP;
    if items.len() > APP_LIST_CAP {
        items.truncate(APP_LIST_CAP);
    }
    (items, truncated)
}

/// Shared by this module's tests and the capability-level tests in `crds`
/// and `events`, which drive the handlers end to end against the same fake
/// API server.
#[cfg(test)]
pub(crate) mod test_support {
    use kube::Client;
    use serde_json::Value;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, MutexGuard};
    use std::time::Duration;

    /// A server that answers each request with the next of `pages` (the last
    /// one repeated), after holding it for `per_page`, and records each URI.
    pub(crate) fn mock_slow_pages(
        pages: Vec<Value>,
        per_page: Duration,
    ) -> (Client, Arc<Mutex<Vec<String>>>) {
        let pages = Arc::new(pages);
        let served = Arc::new(AtomicUsize::new(0));
        let uris = Arc::new(Mutex::new(vec![]));
        let captured = uris.clone();
        let service = tower::service_fn(move |request: http::Request<kube::client::Body>| {
            let captured = captured.clone();
            let pages = pages.clone();
            let served = served.clone();
            async move {
                captured.lock().unwrap().push(request.uri().to_string());
                let page = served.fetch_add(1, Ordering::SeqCst);
                if !per_page.is_zero() {
                    tokio::time::sleep(per_page).await;
                }
                let body = pages[page.min(pages.len() - 1)].clone();
                Ok::<_, std::convert::Infallible>(
                    http::Response::builder()
                        .status(200)
                        .header("content-type", "application/json")
                        .body(kube::client::Body::from(body.to_string().into_bytes()))
                        .unwrap(),
                )
            }
        });
        (Client::new(service, "default"), uris)
    }

    static TIMEOUT_LOCK: Mutex<()> = Mutex::new(());

    /// Holds the process-wide request timeout at `secs` for the guard's
    /// lifetime and puts the previous value back on drop, a panic included.
    /// Tests that need the real setting take this so they run one at a time.
    pub(crate) struct RequestTimeoutGuard {
        previous: u64,
        _lock: MutexGuard<'static, ()>,
    }

    pub(crate) fn hold_request_timeout(secs: u64) -> RequestTimeoutGuard {
        let lock = TIMEOUT_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = crate::connect::request_timeout_secs();
        crate::connect::set_request_timeout_secs(secs);
        RequestTimeoutGuard {
            previous,
            _lock: lock,
        }
    }

    impl Drop for RequestTimeoutGuard {
        fn drop(&mut self) {
            crate::connect::set_request_timeout_secs(self.previous);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::mock_slow_pages;
    use super::*;
    use k8s_openapi::api::core::v1::Event;
    use kube::Client;
    use serde_json::{json, Value};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    #[test]
    fn apply_cap_leaves_a_short_list_alone() {
        let (items, truncated) = apply_cap(vec![1, 2, 3], false);
        assert_eq!(items, [1, 2, 3]);
        assert!(!truncated);
    }

    #[test]
    fn apply_cap_marks_unread_pages_even_when_under_the_ceiling() {
        let (items, truncated) = apply_cap(vec![1, 2, 3], true);
        assert_eq!(items, [1, 2, 3]);
        assert!(truncated);
    }

    #[test]
    fn apply_cap_cuts_an_overlong_list_and_says_so() {
        let items: Vec<_> = (0..APP_LIST_CAP + 50).collect();
        let (kept, truncated) = apply_cap(items, false);
        assert_eq!(kept.len(), APP_LIST_CAP);
        assert_eq!(kept[0], 0);
        assert_eq!(kept[APP_LIST_CAP - 1], APP_LIST_CAP - 1);
        assert!(truncated);
    }

    fn event(name: &str) -> Value {
        json!({"metadata":{"name":name,"namespace":"default"},"involvedObject":{}})
    }

    fn event_list(items: Vec<Value>, continue_token: Option<&str>) -> Value {
        json!({"apiVersion":"v1","kind":"EventList","metadata":{"continue":continue_token},"items":items})
    }

    fn mock_event_pages(pages: Vec<Value>) -> (Client, Arc<Mutex<Vec<String>>>) {
        let pages = Arc::new(pages);
        let served = Arc::new(AtomicUsize::new(0));
        let uris = Arc::new(Mutex::new(vec![]));
        let captured = uris.clone();
        let service = tower::service_fn(move |request: http::Request<kube::client::Body>| {
            let captured = captured.clone();
            let pages = pages.clone();
            let served = served.clone();
            async move {
                captured.lock().unwrap().push(request.uri().to_string());
                let page = served.fetch_add(1, Ordering::SeqCst);
                let body = pages[page.min(pages.len() - 1)].clone();
                Ok::<_, std::convert::Infallible>(
                    http::Response::builder()
                        .status(200)
                        .header("content-type", "application/json")
                        .body(kube::client::Body::from(body.to_string().into_bytes()))
                        .unwrap(),
                )
            }
        });
        (Client::new(service, "default"), uris)
    }

    #[tokio::test]
    async fn continues_past_an_empty_page_that_still_has_a_token() {
        let (client, uris) = mock_event_pages(vec![
            event_list(vec![], Some("page-2")),
            event_list(vec![event("kept")], None),
        ]);
        let api: Api<Event> = Api::all(client);
        let (items, truncated) = list_capped(&api, ListParams::default())
            .await
            .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].metadata.name.as_deref(), Some("kept"));
        assert!(!truncated);
        let asked = uris.lock().unwrap();
        assert_eq!(asked.len(), 2);
        assert!(asked[0].contains(&format!("limit={APP_LIST_PAGE}")));
        assert!(asked[1].contains("continue=page-2"));
    }

    #[tokio::test]
    async fn pages_until_exhausted_under_the_cap() {
        let (client, uris) = mock_event_pages(vec![
            event_list(vec![event("a"), event("b")], Some("next")),
            event_list(vec![event("c")], None),
        ]);
        let api: Api<Event> = Api::all(client);
        let (items, truncated) = list_capped(&api, ListParams::default())
            .await
            .unwrap();
        assert_eq!(
            items
                .iter()
                .map(|e| e.metadata.name.clone().unwrap())
                .collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
        assert!(!truncated);
        assert_eq!(uris.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn stops_at_the_cap_and_marks_truncated_when_more_remain() {
        // Four full pages of APP_LIST_PAGE fill the cap; a continue token on
        // the last page means more remain unread.
        let page: Vec<_> = (0..APP_LIST_PAGE as usize)
            .map(|i| event(&format!("e{i}")))
            .collect();
        let (client, uris) = mock_event_pages(vec![
            event_list(page.clone(), Some("p2")),
            event_list(page.clone(), Some("p3")),
            event_list(page.clone(), Some("p4")),
            event_list(page, Some("p5")),
        ]);
        let api: Api<Event> = Api::all(client);
        let (items, truncated) = list_capped(&api, ListParams::default())
            .await
            .unwrap();
        assert_eq!(items.len(), APP_LIST_CAP);
        assert!(truncated);
        assert_eq!(uris.lock().unwrap().len(), 4);
    }

    /// A server that answers each page slowly, so a per-page budget can be
    /// exceeded deliberately. `per_page` is how long each response is held.
    fn mock_slow_event_pages(
        pages: Vec<Value>,
        per_page: std::time::Duration,
    ) -> (Client, Arc<Mutex<Vec<String>>>) {
        mock_slow_pages(pages, per_page)
    }

    /// A server that keeps answering with the continue token it was asked
    /// for never advances; following it would return the same rows again and
    /// again. The walk refuses at the first repeat, before handing back data.
    #[tokio::test]
    async fn a_continue_token_that_does_not_advance_is_an_error() {
        let (client, uris) = mock_event_pages(vec![
            event_list(vec![event("a")], Some("p2")),
            event_list(vec![event("a")], Some("p2")),
        ]);
        let api: Api<Event> = Api::all(client);

        let error = list_capped(&api, ListParams::default())
            .await
            .expect_err("a repeated continue token must not be followed");

        assert!(
            matches!(&error, ListCappedError::StuckContinuation { token } if token == "p2"),
            "got {error:?}"
        );
        assert!(error.to_string().contains("did not advance"), "{error}");
        assert_eq!(uris.lock().unwrap().len(), 2, "one repeat is enough to know");
        match error.into_capability_error("list events") {
            CapabilityError::Handler(message) => {
                assert!(message.starts_with("list events failed: "), "{message}")
            }
            other => panic!("expected a handler error, got {other:?}"),
        }
    }

    /// A page that overruns the per-request budget fails the walk, and says it
    /// timed out rather than blaming the API.
    #[tokio::test]
    async fn a_page_that_overruns_the_request_budget_times_out() {
        let budget = std::time::Duration::from_millis(100);
        let (client, uris) = mock_slow_event_pages(
            vec![event_list(vec![event("never-arrives")], None)],
            budget * 8,
        );
        let api: Api<Event> = Api::all(client);

        let error = list_capped_within(&api, ListParams::default(), budget)
            .await
            .expect_err("a page slower than the budget must not be waited out");

        assert!(
            matches!(error, ListCappedError::Timeout { pages_read: 0, budget: applied } if applied == budget),
            "expected a timeout on the first page, got {error:?}"
        );
        // The message names the budget that was APPLIED, not the process-wide
        // setting the caller may have bypassed.
        assert_eq!(
            error.to_string(),
            "Kubernetes list request timed out after 100ms"
        );
        assert_eq!(uris.lock().unwrap().len(), 1);
    }

    /// A timeout partway through says how far the walk got.
    #[tokio::test]
    async fn a_second_page_that_times_out_reports_the_pages_already_read() {
        let budget = Duration::from_millis(150);
        // The first page answers at once; the second is held past the budget.
        let served = Arc::new(AtomicUsize::new(0));
        let uris = Arc::new(Mutex::new(vec![]));
        let captured = uris.clone();
        let service = tower::service_fn(move |request: http::Request<kube::client::Body>| {
            let captured = captured.clone();
            let served = served.clone();
            async move {
                captured.lock().unwrap().push(request.uri().to_string());
                let page = served.fetch_add(1, Ordering::SeqCst);
                let body = if page == 0 {
                    event_list(vec![event("a")], Some("p2"))
                } else {
                    tokio::time::sleep(budget * 8).await;
                    event_list(vec![event("b")], None)
                };
                Ok::<_, std::convert::Infallible>(
                    http::Response::builder()
                        .status(200)
                        .header("content-type", "application/json")
                        .body(kube::client::Body::from(body.to_string().into_bytes()))
                        .unwrap(),
                )
            }
        });
        let api: Api<Event> = Api::all(Client::new(service, "default"));

        let error = list_capped_within(&api, ListParams::default(), budget)
            .await
            .expect_err("the second page overran the budget");

        assert!(
            matches!(error, ListCappedError::Timeout { pages_read: 1, .. }),
            "got {error:?}"
        );
        assert_eq!(
            error.to_string(),
            "Kubernetes list request timed out after 150ms (1 page(s) already read)"
        );
        assert_eq!(uris.lock().unwrap().len(), 2);
    }

    /// A server that answers every page empty with a fresh continue token
    /// never fills the cap. The walk stops at `APP_LIST_MAX_PAGES` and says
    /// it was cut off, rather than requesting forever.
    #[tokio::test]
    async fn a_walk_of_empty_pages_with_fresh_tokens_is_bounded() {
        let uris = Arc::new(Mutex::new(vec![]));
        let captured = uris.clone();
        let served = Arc::new(AtomicUsize::new(0));
        let service = tower::service_fn(move |request: http::Request<kube::client::Body>| {
            let captured = captured.clone();
            let served = served.clone();
            async move {
                captured.lock().unwrap().push(request.uri().to_string());
                let n = served.fetch_add(1, Ordering::SeqCst);
                let body = event_list(vec![], Some(&format!("page-{}", n + 2)));
                Ok::<_, std::convert::Infallible>(
                    http::Response::builder()
                        .status(200)
                        .header("content-type", "application/json")
                        .body(kube::client::Body::from(body.to_string().into_bytes()))
                        .unwrap(),
                )
            }
        });
        let api: Api<Event> = Api::all(Client::new(service, "default"));

        let (items, truncated) = list_capped(&api, ListParams::default())
            .await
            .expect("every page answered");

        assert!(items.is_empty());
        assert!(truncated, "a walk cut off by the page bound has more unread");
        assert_eq!(uris.lock().unwrap().len(), APP_LIST_MAX_PAGES);
    }

    /// The handlers' mapping: an API error's own words pass through untouched
    /// (the frontend parses them for the cluster-login prompt), and only a
    /// timeout gets a sentence naming what was being listed.
    #[test]
    fn an_api_error_keeps_its_words_and_a_timeout_names_the_list() {
        let api_error = kube::Error::Api(Box::new(
            kube::core::Status::failure("Unauthorized", "Unauthorized").with_code(401),
        ));
        let expected = api_error.to_string();
        match ListCappedError::Api(api_error).into_capability_error("list events") {
            CapabilityError::Handler(message) => assert_eq!(message, expected),
            other => panic!("expected a handler error, got {other:?}"),
        }

        let timeout = ListCappedError::Timeout {
            pages_read: 2,
            budget: Duration::from_secs(5),
        };
        match timeout.into_capability_error("list custom resource") {
            CapabilityError::Handler(message) => assert_eq!(
                message,
                "list custom resource timed out: Kubernetes list request timed out after 5s (2 page(s) already read)"
            ),
            other => panic!("expected a handler error, got {other:?}"),
        }
    }

    /// The budget is per request, not per walk: three pages that each answer
    /// comfortably inside it complete, even though together they take longer
    /// than one budget. The outer whole-walk timeouts this replaces cut this
    /// exact case off — four pages had to share one request's time.
    #[tokio::test]
    async fn a_multi_page_walk_outlasts_one_request_budget() {
        let budget = std::time::Duration::from_millis(300);
        // Each page well inside the budget; three of them well outside it.
        let per_page = budget / 2;
        let (client, uris) = mock_slow_event_pages(
            vec![
                event_list(vec![event("a")], Some("p2")),
                event_list(vec![event("b")], Some("p3")),
                event_list(vec![event("c")], None),
            ],
            per_page,
        );
        let api: Api<Event> = Api::all(client);

        let started = std::time::Instant::now();
        let (items, truncated) = list_capped_within(&api, ListParams::default(), budget)
            .await
            .expect("every page answered inside the per-request budget");

        assert_eq!(
            items
                .iter()
                .map(|e| e.metadata.name.clone().unwrap())
                .collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
        assert!(!truncated);
        assert_eq!(uris.lock().unwrap().len(), 3);
        assert!(
            started.elapsed() > budget,
            "the walk should have taken longer than one request's budget"
        );
        // And the old whole-walk wrapper would have refused it.
        let (client, _) = mock_slow_event_pages(
            vec![
                event_list(vec![event("a")], Some("p2")),
                event_list(vec![event("b")], Some("p3")),
                event_list(vec![event("c")], None),
            ],
            per_page,
        );
        let api: Api<Event> = Api::all(client);
        let whole_walk = tokio::time::timeout(
            budget,
            list_capped_within(&api, ListParams::default(), budget),
        )
        .await;
        assert!(
            whole_walk.is_err(),
            "the whole-walk timeout this replaces would have cut the same walk off"
        );
    }
}
