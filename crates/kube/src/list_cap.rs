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
        let page_len = page.items.len();
        items.extend(page.items);
        token = page.metadata.continue_.filter(|t| !t.is_empty());
        if items.len() >= APP_LIST_CAP {
            let truncated = items.len() > APP_LIST_CAP || token.is_some();
            items.truncate(APP_LIST_CAP);
            return Ok((items, truncated));
        }
        if token.is_none() || page_len == 0 {
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
}
