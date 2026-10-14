//! Host-owned inspection of custom resources.
use crate::{client_cache::ClientCache, connect::request_timeout};
use kube::{
    api::DynamicObject,
    core::{ApiResource, GroupVersionKind},
    Api, Client,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use srelens_capability::{Annotations, Capability, CapabilityError};
use std::sync::Arc;

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResourceIn {
    pub context: String,
    pub group: String,
    pub version: String,
    pub plural: String,
    pub kind: String,
    pub namespaced: bool,
    pub namespace: String,
    pub name: String,
}
impl ResourceIn {
    pub fn validate(&self) -> Result<(), String> {
        let segment = |s: &str| {
            !s.is_empty()
                && s.len() <= 253
                && s != "."
                && s != ".."
                && s.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b".-".contains(&c))
        };
        if self.context.trim().is_empty()
            || [
                &self.group,
                &self.version,
                &self.plural,
                &self.kind,
                &self.name,
            ]
            .iter()
            .any(|s| !segment(s))
        {
            return Err(
                "An explicit context and valid custom-resource identity are required".into(),
            );
        }
        if self.namespaced {
            if self.namespace.is_empty()
                || self.namespace.len() > 63
                || self.namespace.starts_with('-')
                || self.namespace.ends_with('-')
                || !self
                    .namespace
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
            {
                return Err("An explicit namespace is required for this resource".into());
            }
        } else if !self.namespace.is_empty() {
            return Err("Cluster-scoped resources cannot have a namespace".into());
        }
        Ok(())
    }
    pub(crate) fn api(&self, client: Client) -> Api<DynamicObject> {
        let mut ar = ApiResource::from_gvk(&GroupVersionKind::gvk(
            &self.group,
            &self.version,
            &self.kind,
        ));
        ar.plural = self.plural.clone();
        if self.namespaced {
            Api::namespaced_with(client, &self.namespace, &ar)
        } else {
            Api::all_with(client, &ar)
        }
    }
}
#[derive(Serialize, JsonSchema)]
pub struct ResourceOut {
    pub resource: Value,
    /// Newest first, at most `EVENTS_SHOWN`.
    pub events: Vec<Value>,
    #[serde(rename = "eventsTruncated")]
    pub events_truncated: bool,
    /// True when `EVENT_PAGES` pages were read and more remained, so `events` are the
    /// newest of those read rather than of all.
    #[serde(rename = "eventsPartial")]
    pub events_partial: bool,
    /// How many events were read before the newest were chosen. A page holds at most
    /// `EVENT_PAGE` events, so this is the real count, not pages times the page size.
    #[serde(rename = "eventsRead")]
    pub events_read: usize,
    #[serde(rename = "eventsError")]
    pub events_error: Option<String>,
}

async fn inspect(client: Client, r: &ResourceIn) -> Result<ResourceOut, String> {
    inspect_with_timeout(client, r, request_timeout()).await
}
async fn inspect_with_timeout(
    client: Client,
    r: &ResourceIn,
    timeout: std::time::Duration,
) -> Result<ResourceOut, String> {
    r.validate()?;
    let api = r.api(client.clone());
    let object = tokio::time::timeout(timeout, api.get(&r.name))
        .await
        .map_err(|_| "Resource request timed out".to_string())?
        .map_err(|e| e.to_string())?;
    let mut resource = serde_json::to_value(object).map_err(|e| e.to_string())?;
    if let Some(meta) = resource.get_mut("metadata").and_then(Value::as_object_mut) {
        meta.remove("managedFields");
    }
    let events_api: Api<k8s_openapi::api::core::v1::Event> = if r.namespaced {
        Api::namespaced(client, &r.namespace)
    } else {
        Api::all(client)
    };
    let uid = resource["metadata"]["uid"].as_str().unwrap_or("");
    let (events, events_truncated, events_partial, events_read, events_error) = if uid.is_empty() {
        (
            vec![],
            false,
            false,
            0,
            Some("Resource UID is unavailable".into()),
        )
    } else {
        match tokio::time::timeout(timeout, list_events(&events_api, uid)).await {
            Ok(Ok((items, partial))) => {
                let read = items.len();
                let (events, truncated) = newest_events(items, partial);
                (events, truncated, partial, read, None)
            }
            Ok(Err(e)) => (vec![], false, false, 0, Some(e.to_string())),
            Err(_) => (
                vec![],
                false,
                false,
                0,
                Some("Events request timed out".into()),
            ),
        }
    };
    Ok(ResourceOut {
        resource,
        events,
        events_truncated,
        events_partial,
        events_read,
        events_error,
    })
}
/// Events are listed in storage order, which says nothing about recency, so every page
/// is read before the newest are chosen, up to `EVENT_PAGES` pages.
const EVENT_PAGE: u32 = 500;
const EVENT_PAGES: usize = 10;
const EVENTS_SHOWN: usize = 100;
/// All of an object's events, and whether pages were left unread at the bound.
async fn list_events(
    api: &Api<k8s_openapi::api::core::v1::Event>,
    uid: &str,
) -> Result<(Vec<k8s_openapi::api::core::v1::Event>, bool), kube::Error> {
    let mut items = Vec::new();
    let mut token: Option<String> = None;
    for _ in 0..EVENT_PAGES {
        let mut params = kube::api::ListParams::default()
            .fields(&format!("involvedObject.uid={uid}"))
            .limit(EVENT_PAGE);
        if let Some(token) = &token {
            params = params.continue_token(token);
        }
        let page = api.list(&params).await?;
        items.extend(page.items);
        token = page.metadata.continue_.filter(|token| !token.is_empty());
        if token.is_none() {
            return Ok((items, false));
        }
    }
    Ok((items, true))
}
/// The newest `EVENTS_SHOWN` events by when each was last seen, and whether any were left
/// out (more than that were read, or pages remained unread).
fn newest_events(
    mut items: Vec<k8s_openapi::api::core::v1::Event>,
    partial: bool,
) -> (Vec<Value>, bool) {
    // The latest of every time an event carries. A recurring series records its newest
    // occurrence in `series.lastObservedTime` while `eventTime` stays its first. Times
    // are compared parsed: "…00.5Z" sorts before "…00Z" as text but is later.
    let seen = |e: &k8s_openapi::api::core::v1::Event| {
        [
            e.series
                .as_ref()
                .and_then(|s| s.last_observed_time.as_ref())
                .map(|t| t.0),
            e.last_timestamp.as_ref().map(|t| t.0),
            e.event_time.as_ref().map(|t| t.0),
            e.first_timestamp.as_ref().map(|t| t.0),
            e.metadata.creation_timestamp.as_ref().map(|t| t.0),
        ]
        .into_iter()
        .flatten()
        .max()
    };
    items.sort_by_key(|e| std::cmp::Reverse(seen(e)));
    let truncated = partial || items.len() > EVENTS_SHOWN;
    let events = items
        .into_iter()
        .take(EVENTS_SHOWN)
        .map(|e| {
            let time = seen(&e).map(|t| t.to_string());
            let count = e.count.or_else(|| e.series.as_ref().and_then(|s| s.count));
            json!({"type":e.type_,"reason":e.reason,"message":e.message,"count":count,"time":time})
        })
        .collect();
    (events, truncated)
}
/// Pins a merge patch to the object the operator reviewed. The API server enforces a
/// `metadata.resourceVersion` carried in a patch as an optimistic-concurrency precondition
/// (409 Conflict when the object has moved on), and rejects a `metadata.uid` that differs
/// from the stored object's, so the write cannot land on a replacement created under the
/// same name, nor on a newer version the operator never saw.
pub(crate) fn pin_to_reviewed(patch: &mut Value, uid: &str, resource_version: &str) {
    if patch.get("metadata").is_none() {
        patch["metadata"] = json!({});
    }
    patch["metadata"]["uid"] = json!(uid);
    patch["metadata"]["resourceVersion"] = json!(resource_version);
}
pub fn resource_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ResourceIn, ResourceOut, _, _>(
        "k8s.getCustomResource",
        "Inspect one custom resource and its events",
        Annotations::READ_ONLY,
        move |input| {
            let cache = cache.clone();
            async move {
                input.validate().map_err(CapabilityError::InvalidInput)?;
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                inspect(client, &input)
                    .await
                    .map_err(CapabilityError::Handler)
            }
        },
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn resource(group: &str, kind: &str, plural: &str) -> ResourceIn {
        let version = match group {
            "argoproj.io" => "v1alpha1",
            "helm.toolkit.fluxcd.io" => "v2",
            _ => "v1",
        };
        versioned(group, version, kind, plural)
    }
    fn versioned(group: &str, version: &str, kind: &str, plural: &str) -> ResourceIn {
        serde_json::from_value(json!({"context":"cluster/a","group":group,"version":version,"kind":kind,"plural":plural,"namespaced":true,"namespace":"team","name":"apps"})).unwrap()
    }
    #[test]
    fn scope_and_caller_wire_names_are_validated() {
        let mut r = resource("argoproj.io", "Application", "applications");
        assert!(r.validate().is_ok());
        r.namespace.clear();
        assert!(r.validate().is_err());
        r.namespace = "team".into();
        r.context.clear();
        assert!(r.validate().is_err());
        r.context = "test".into();
        r.name = "../other".into();
        assert!(r.validate().is_err());
    }
    type Requests = Arc<std::sync::Mutex<Vec<(String, Value)>>>;
    fn mock_client(patch_status: u16, events_status: u16) -> (Client, Requests) {
        mock_client_with_events(
            patch_status,
            events_status,
            vec![json!({"apiVersion":"v1","kind":"EventList","metadata":{},"items":[]})],
        )
    }
    fn mock_client_with_events(
        patch_status: u16,
        events_status: u16,
        pages: Vec<Value>,
    ) -> (Client, Requests) {
        // Event list pages are served in request order; the last one repeats.
        let pages = Arc::new(pages);
        let served = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let requests = Arc::new(std::sync::Mutex::new(vec![]));
        let captured = requests.clone();
        let service = tower::service_fn(move |request: http::Request<kube::client::Body>| {
            let captured = captured.clone();
            let pages = pages.clone();
            let served = served.clone();
            async move {
                let method = request.method().to_string();
                let uri = request.uri().to_string();
                let body = request.into_body().collect_bytes().await.unwrap();
                let value = if body.is_empty() {
                    Value::Null
                } else {
                    serde_json::from_slice(&body).unwrap()
                };
                captured
                    .lock()
                    .unwrap()
                    .push((format!("{method} {uri}"), value));
                if uri.contains("/events") && events_status == 0 {
                    std::future::pending::<()>().await;
                }
                let status = if method == "PATCH" {
                    patch_status
                } else if uri.contains("/events") {
                    events_status
                } else {
                    200
                };
                let body = if status >= 400 {
                    json!({"apiVersion":"v1","kind":"Status","status":"Failure","code":status,"reason":"Forbidden","message":"rejected by cluster"})
                } else if uri.contains("/events") {
                    let page = served.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    pages[page.min(pages.len() - 1)].clone()
                } else {
                    json!({"apiVersion":"argoproj.io/v1alpha1","kind":"Application","metadata":{"name":"apps","namespace":"team","uid":"u","resourceVersion":"2","managedFields":[]},"spec":{},"status":{}})
                };
                Ok::<_, std::convert::Infallible>(
                    http::Response::builder()
                        .status(status)
                        .header("content-type", "application/json")
                        .body(kube::client::Body::from(body.to_string().into_bytes()))
                        .unwrap(),
                )
            }
        });
        (Client::new(service, "default"), requests)
    }
    fn event(reason: &str, last: Option<&str>, event_time: Option<&str>) -> Value {
        json!({"metadata":{"name":reason,"namespace":"team"},"involvedObject":{},"reason":reason,"lastTimestamp":last,"eventTime":event_time})
    }
    fn event_list(items: Vec<Value>, continue_token: Option<&str>) -> Value {
        json!({"apiVersion":"v1","kind":"EventList","metadata":{"continue":continue_token},"items":items})
    }
    #[tokio::test]
    async fn events_are_newest_first_by_parsed_time_across_every_page() {
        let argo = resource("argoproj.io", "Application", "applications");
        // A series first observed long ago whose newest occurrence is the most recent event.
        let mut recurring = event("recurring", None, Some("2026-01-01T00:00:00.000000Z"));
        recurring["series"] =
            json!({"count": 7, "lastObservedTime": "2026-01-03T00:00:00.000000Z"});
        let (client, requests) = mock_client_with_events(
            200,
            200,
            vec![
                event_list(
                    vec![
                        event("oldest", Some("2026-01-01T00:00:00Z"), None),
                        event("second", Some("2026-01-02T00:00:00Z"), None),
                        recurring,
                    ],
                    Some("page-2"),
                ),
                event_list(
                    vec![
                        // Later than "second", although it sorts before it as text.
                        event("third", None, Some("2026-01-02T00:00:00.500000Z")),
                        event("undated", None, None),
                    ],
                    None,
                ),
            ],
        );
        let result = inspect(client, &argo).await.unwrap();
        let order: Vec<&str> = result
            .events
            .iter()
            .map(|e| e["reason"].as_str().unwrap())
            .collect();
        assert_eq!(order, ["recurring", "third", "second", "oldest", "undated"]);
        assert_eq!(result.events[0]["count"], 7);
        assert!(result.events[0]["time"]
            .as_str()
            .unwrap()
            .starts_with("2026-01-03T00:00:00"));
        assert!(!result.events_truncated && !result.events_partial);
        let value = serde_json::to_value(&result).unwrap();
        assert_eq!(value["eventsTruncated"], false);
        assert_eq!(value["eventsPartial"], false);
        assert_eq!(value["eventsRead"], 5);
        let requests = requests.lock().unwrap();
        let listed: Vec<&String> = requests
            .iter()
            .map(|(request, _)| request)
            .filter(|request| request.contains("/events"))
            .collect();
        assert_eq!(listed.len(), 2);
        assert!(listed[0].contains("limit=500"));
        assert!(listed[1].contains("continue=page-2"));
    }
    #[tokio::test]
    async fn events_stop_at_the_page_bound_and_do_not_claim_to_be_the_latest() {
        let argo = resource("argoproj.io", "Application", "applications");
        let endless = event_list(
            vec![event("again", Some("2026-01-01T00:00:00Z"), None)],
            Some("more"),
        );
        let (client, requests) = mock_client_with_events(200, 200, vec![endless]);
        let result = inspect(client, &argo).await.unwrap();
        assert!(result.events_partial && result.events_truncated);
        // Ten one-event pages: the count read is ten, not ten times the page size.
        assert_eq!(result.events_read, EVENT_PAGES);
        assert_eq!(
            serde_json::to_value(&result).unwrap()["eventsRead"],
            EVENT_PAGES
        );
        assert_eq!(
            requests
                .lock()
                .unwrap()
                .iter()
                .filter(|(request, _)| request.contains("/events"))
                .count(),
            EVENT_PAGES
        );

        let many = (0..150)
            .map(|i| {
                let time = format!("2026-01-01T00:{:02}:{:02}Z", i / 60, i % 60);
                event(&format!("e{i}"), Some(&time), None)
            })
            .collect();
        let (client, _) = mock_client_with_events(200, 200, vec![event_list(many, None)]);
        let result = inspect(client, &argo).await.unwrap();
        assert_eq!(result.events.len(), 100);
        assert!(result.events_truncated && !result.events_partial);
        assert_eq!(result.events_read, 150);
        assert_eq!(result.events[0]["reason"], "e149");
    }
    #[tokio::test]
    async fn event_timeouts_preserve_the_already_fetched_resource() {
        let (client, requests) = mock_client(200, 0);
        let result = inspect_with_timeout(
            client,
            &resource("argoproj.io", "Application", "applications"),
            std::time::Duration::from_millis(50),
        )
        .await
        .unwrap();
        assert_eq!(result.resource["metadata"]["name"], "apps");
        assert!(result.events.is_empty());
        assert_eq!(
            result.events_error.as_deref(),
            Some("Events request timed out")
        );
        assert_eq!(requests.lock().unwrap().len(), 2);
    }
    #[tokio::test]
    async fn event_permission_errors_preserve_resource_details_and_filter_events_by_uid() {
        let (client, requests) = mock_client(200, 403);
        let result = inspect(
            client,
            &resource("argoproj.io", "Application", "applications"),
        )
        .await
        .unwrap();
        assert_eq!(result.resource["metadata"]["name"], "apps");
        assert!(result.resource["metadata"].get("managedFields").is_none());
        assert!(result.events_error.unwrap().contains("rejected by cluster"));
        assert!(requests.lock().unwrap()[1].0.contains("involvedObject.uid"));
    }

    #[tokio::test]
    async fn gitops_capabilities_validation_and_resolution() {
        let cache = ClientCache::new(std::path::PathBuf::from("/dev/null"));

        // 1. Resource capability validation
        let res_cap = resource_capability(cache.clone());
        assert_eq!(res_cap.id, "k8s.getCustomResource");

        // Invalid JSON input (missing required fields)
        let err = (res_cap.handler)(json!({})).await.unwrap_err();
        assert!(matches!(
            err,
            srelens_capability::CapabilityError::InvalidInput(_)
        ));

        // Invalid resource input (empty context)
        let err = (res_cap.handler)(json!({
            "context": "",
            "group": "argoproj.io",
            "version": "v1alpha1",
            "kind": "Application",
            "resource": "applications",
            "name": "billing",
            "namespaced": true
        }))
        .await
        .unwrap_err();
        assert!(matches!(
            err,
            srelens_capability::CapabilityError::InvalidInput(_)
        ));

        // Missing context in cache -> Handler error
        let err = (res_cap.handler)(json!({
            "context": "non-existent-cluster",
            "group": "argoproj.io",
            "version": "v1alpha1",
            "plural": "applications",
            "kind": "Application",
            "namespaced": true,
            "namespace": "argocd",
            "name": "billing"
        }))
        .await
        .unwrap_err();
        assert!(matches!(
            err,
            srelens_capability::CapabilityError::Handler(_)
        ));
    }
}
