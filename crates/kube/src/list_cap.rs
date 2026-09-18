//! Caps for unbounded app-reader Kubernetes lists (#609).
//!
//! `k8s.listCustomResource` and `k8s.listEvents` used to issue one uncapped
//! list and hold the whole response. A large cluster or busy namespace could
//! stall or exhaust the desktop app. Both now page with `limit`/`continue` and
//! stop at [`APP_LIST_CAP`], telling the caller when they were cut off.

use kube::api::{Api, ListParams};
use kube::Resource;
use serde::de::DeserializeOwned;
use std::fmt::Debug;

/// Page size when walking the API server's continue token.
pub const APP_LIST_PAGE: u32 = 500;

/// Hard ceiling on rows returned by app-reader lists.
pub const APP_LIST_CAP: usize = 2_000;

/// List until the cap or the end of the collection.
///
/// `truncated` is true when more items remain unread (a continue token) or when
/// the last page had to be cut to fit the cap.
///
/// An empty page with a continue token is not the end: a selector-filtered
/// chunk may return zero items while more matching objects remain later
/// (Kubernetes list pagination). Stop only when the continue token is absent.
pub async fn list_capped<K>(
    api: &Api<K>,
    base: ListParams,
) -> Result<(Vec<K>, bool), kube::Error>
where
    K: Resource + Clone + DeserializeOwned + Debug,
{
    let mut items = Vec::new();
    let mut token: Option<String> = None;
    loop {
        let mut params = base.clone().limit(APP_LIST_PAGE);
        if let Some(ref t) = token {
            params = params.continue_token(t);
        }
        let page = api.list(&params).await?;
        items.extend(page.items);
        token = page.metadata.continue_.filter(|t| !t.is_empty());
        if items.len() >= APP_LIST_CAP {
            let truncated = items.len() > APP_LIST_CAP || token.is_some();
            items.truncate(APP_LIST_CAP);
            return Ok((items, truncated));
        }
        if token.is_none() {
            return Ok((items, false));
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

#[cfg(test)]
mod tests {
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
}
