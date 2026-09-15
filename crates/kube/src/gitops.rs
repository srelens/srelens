//! Host-owned inspection and explicit GitOps actions for custom resources.
use crate::{client_cache::ClientCache, connect::request_timeout};
use kube::{
    api::{DynamicObject, Patch, PatchParams},
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
    fn api(&self, client: Client) -> Api<DynamicObject> {
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
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActionIn {
    pub resource: ResourceIn,
    pub action: String,
    pub uid: String,
    #[serde(rename = "resourceVersion")]
    pub resource_version: String,
}
#[derive(Serialize, JsonSchema)]
pub struct ResourceOut {
    pub resource: Value,
    pub actions: Vec<String>,
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

/// Actions are offered only for the API versions whose schema carries the fields they
/// write. Every listed Flux version has `spec.suspend` (per each controller's `api/`
/// package; OCIRepository has no v1beta1), HelmRelease `forceAt`/`resetAt` arrived with
/// v2beta2, and Argo CD's `operation` is a v1alpha1 field. An unlisted version, such as
/// a future one that moves a field, gets no actions.
pub fn supported_actions(r: &ResourceIn) -> Vec<String> {
    if !r.namespaced {
        return vec![];
    }
    const FLUX: &[&str] = &["v1", "v1beta2", "v1beta1"];
    const RECONCILE: &[&str] = &["suspend", "resume", "reconcile"];
    let (versions, actions): (&[&str], &[&str]) =
        match (r.group.as_str(), r.kind.as_str(), r.plural.as_str()) {
            ("kustomize.toolkit.fluxcd.io", "Kustomization", "kustomizations")
            | ("source.toolkit.fluxcd.io", "GitRepository", "gitrepositories")
            | ("source.toolkit.fluxcd.io", "HelmRepository", "helmrepositories")
            | ("source.toolkit.fluxcd.io", "HelmChart", "helmcharts")
            | ("source.toolkit.fluxcd.io", "Bucket", "buckets")
            | ("image.toolkit.fluxcd.io", "ImageRepository", "imagerepositories")
            | ("image.toolkit.fluxcd.io", "ImageUpdateAutomation", "imageupdateautomations") => {
                (FLUX, RECONCILE)
            }
            ("source.toolkit.fluxcd.io", "OCIRepository", "ocirepositories") => {
                (&["v1", "v1beta2"], RECONCILE)
            }
            ("helm.toolkit.fluxcd.io", "HelmRelease", "helmreleases") => match r.version.as_str() {
                "v2beta1" => (&["v2beta1"], RECONCILE),
                _ => (
                    &["v2", "v2beta2"],
                    &["suspend", "resume", "reconcile", "force", "reset"],
                ),
            },
            ("argoproj.io", "Application", "applications") => {
                (&["v1alpha1"], &["refresh", "hard-refresh", "sync"])
            }
            _ => return vec![],
        };
    if !versions.contains(&r.version.as_str()) {
        return vec![];
    }
    actions.iter().map(|action| (*action).to_owned()).collect()
}
fn action_patch(r: &ResourceIn, action: &str, token: &str) -> Result<Value, String> {
    if !supported_actions(r).iter().any(|a| a == action) {
        return Err("This action is not supported for this resource".into());
    }
    Ok(match action {
        "suspend" | "resume" => json!({"spec":{"suspend":action == "suspend"}}),
        "refresh" | "hard-refresh" => {
            json!({"metadata":{"annotations":{"argocd.argoproj.io/refresh":if action == "refresh" { "normal" } else { "hard" }}}})
        }
        "sync" => {
            json!({"operation":{"initiatedBy":{"username":"srelens"},"sync":{"prune":false,"syncStrategy":{"hook":{}}}}})
        }
        _ => {
            let mut patch =
                json!({"metadata":{"annotations":{"reconcile.fluxcd.io/requestedAt":token}}});
            if action == "force" || action == "reset" {
                patch["metadata"]["annotations"][format!("reconcile.fluxcd.io/{}At", action)] =
                    json!(token);
            }
            patch
        }
    })
}
fn guard_action(current: &Value, uid: &str, version: &str, action: &str) -> Result<(), String> {
    if uid.is_empty()
        || version.is_empty()
        || current["metadata"]["uid"] != uid
        || current["metadata"]["resourceVersion"] != version
    {
        return Err("Resource changed or was replaced; refresh and review the action again".into());
    }
    if action == "sync" && !current["operation"].is_null() {
        return Err("An Argo CD operation is already in progress".into());
    }
    // A request that would change nothing is refused rather than reported as accepted.
    if action == "suspend" && current["spec"]["suspend"] == true {
        return Err("This resource is already suspended".into());
    }
    if action == "resume" && current["spec"]["suspend"] != true {
        return Err("This resource is not suspended".into());
    }
    if matches!(action, "reconcile" | "force" | "reset") && current["spec"]["suspend"] == true {
        return Err("Resume this resource before requesting reconciliation".into());
    }
    if !current["metadata"]["deletionTimestamp"].is_null() {
        return Err("Resource is being deleted".into());
    }
    Ok(())
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
        actions: supported_actions(r),
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
async fn execute(client: Client, input: ActionIn) -> Result<Value, String> {
    input.resource.validate()?;
    let token = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos()
        .to_string();
    let mut patch = action_patch(&input.resource, &input.action, &token)?;
    let api = input.resource.api(client);
    let current = serde_json::to_value(
        api.get(&input.resource.name)
            .await
            .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    guard_action(&current, &input.uid, &input.resource_version, &input.action)?;
    if patch.get("metadata").is_none() {
        patch["metadata"] = json!({});
    }
    patch["metadata"]["uid"] = json!(input.uid);
    patch["metadata"]["resourceVersion"] = json!(input.resource_version);
    api.patch(
        &input.resource.name,
        &PatchParams::default(),
        &Patch::Merge(&patch),
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(json!({"requested":true}))
}
pub fn resource_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ResourceIn, ResourceOut, _, _>(
        "k8s.getCustomResource",
        "Inspect one custom resource and its supported host actions",
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
pub fn action_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ActionIn, Value, _, _>("k8s.gitOpsAction", "Request a supported Flux or Argo CD operation on the reviewed resource; requires confirmation", Annotations::MUTATING, move |input| {
        let cache = cache.clone(); async move {
            input.resource.validate().map_err(CapabilityError::InvalidInput)?;
            action_patch(&input.resource, &input.action, "validate").map_err(CapabilityError::InvalidInput)?;
            let client = cache.get(&input.resource.context).await.map_err(CapabilityError::Handler)?;
            tokio::time::timeout(request_timeout(), execute(client, input)).await.map_err(|_| CapabilityError::Handler("Action request timed out; refresh the resource to check whether it was accepted".into()))?.map_err(CapabilityError::Handler)
        }
    })
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
    fn actions_are_offered_only_for_api_versions_that_carry_their_fields() {
        let actions = |group: &str, version: &str, kind: &str, plural: &str| {
            supported_actions(&versioned(group, version, kind, plural))
        };
        for version in ["v1", "v1beta2", "v1beta1"] {
            assert_eq!(
                actions(
                    "kustomize.toolkit.fluxcd.io",
                    version,
                    "Kustomization",
                    "kustomizations"
                ),
                ["suspend", "resume", "reconcile"]
            );
        }
        assert!(actions(
            "kustomize.toolkit.fluxcd.io",
            "v2",
            "Kustomization",
            "kustomizations"
        )
        .is_empty());
        assert!(actions(
            "source.toolkit.fluxcd.io",
            "v1beta1",
            "OCIRepository",
            "ocirepositories"
        )
        .is_empty());
        assert_eq!(
            actions(
                "source.toolkit.fluxcd.io",
                "v1beta2",
                "OCIRepository",
                "ocirepositories"
            )
            .len(),
            3
        );
        assert_eq!(
            actions(
                "helm.toolkit.fluxcd.io",
                "v2beta2",
                "HelmRelease",
                "helmreleases"
            ),
            ["suspend", "resume", "reconcile", "force", "reset"]
        );
        // Force and reset arrived with v2beta2; a v2beta1-only controller ignores them.
        let old_helm = versioned(
            "helm.toolkit.fluxcd.io",
            "v2beta1",
            "HelmRelease",
            "helmreleases",
        );
        assert_eq!(
            supported_actions(&old_helm),
            ["suspend", "resume", "reconcile"]
        );
        assert!(action_patch(&old_helm, "force", "token").is_err());
        assert_eq!(
            actions("argoproj.io", "v1alpha1", "Application", "applications"),
            ["refresh", "hard-refresh", "sync"]
        );
        assert!(actions("argoproj.io", "v1beta1", "Application", "applications").is_empty());
    }
    #[test]
    fn suspend_and_resume_refuse_requests_that_would_change_nothing() {
        let current =
            |spec: Value| json!({"metadata":{"uid":"u","resourceVersion":"2"},"spec":spec});
        let refused = |spec: Value, action: &str| guard_action(&current(spec), "u", "2", action);
        assert!(refused(json!({"suspend":true}), "suspend")
            .unwrap_err()
            .contains("already suspended"));
        assert!(refused(json!({"suspend":false}), "resume")
            .unwrap_err()
            .contains("not suspended"));
        assert!(refused(json!({}), "resume")
            .unwrap_err()
            .contains("not suspended"));
        assert!(refused(json!({"suspend":false}), "suspend").is_ok());
        assert!(refused(json!({}), "suspend").is_ok());
        assert!(refused(json!({"suspend":true}), "resume").is_ok());
    }
    #[test]
    fn flux_actions_are_limited_to_supported_resources_and_fields() {
        let r = resource(
            "kustomize.toolkit.fluxcd.io",
            "Kustomization",
            "kustomizations",
        );
        assert_eq!(
            action_patch(&r, "suspend", "token").unwrap(),
            json!({"spec":{"suspend":true}})
        );
        assert_eq!(
            action_patch(&r, "resume", "token").unwrap(),
            json!({"spec":{"suspend":false}})
        );
        assert_eq!(
            action_patch(&r, "reconcile", "token").unwrap(),
            json!({"metadata":{"annotations":{"reconcile.fluxcd.io/requestedAt":"token"}}})
        );
        assert!(action_patch(&r, "force", "token").is_err());
        let helm = resource("helm.toolkit.fluxcd.io", "HelmRelease", "helmreleases");
        let patch = action_patch(&helm, "reset", "token").unwrap();
        assert_eq!(
            patch["metadata"]["annotations"]["reconcile.fluxcd.io/resetAt"],
            "token"
        );
        assert_eq!(
            patch["metadata"]["annotations"]["reconcile.fluxcd.io/requestedAt"],
            "token"
        );
        let wrong = resource("evil.toolkit.fluxcd.io", "Kustomization", "kustomizations");
        assert!(action_patch(&wrong, "suspend", "token").is_err());
        let wrong_plural = resource("kustomize.toolkit.fluxcd.io", "Kustomization", "secrets");
        assert!(action_patch(&wrong_plural, "suspend", "token").is_err());
    }
    #[test]
    fn argo_actions_do_not_enable_pruning_or_replace_existing_operations() {
        let r = resource("argoproj.io", "Application", "applications");
        assert_eq!(
            action_patch(&r, "refresh", "token").unwrap()["metadata"]["annotations"]
                ["argocd.argoproj.io/refresh"],
            "normal"
        );
        assert_eq!(
            action_patch(&r, "hard-refresh", "token").unwrap()["metadata"]["annotations"]
                ["argocd.argoproj.io/refresh"],
            "hard"
        );
        assert_eq!(
            action_patch(&r, "sync", "token").unwrap()["operation"]["sync"]["prune"],
            false
        );
        assert!(action_patch(&r, "suspend", "token").is_err());
        let current = json!({"metadata":{"uid":"u","resourceVersion":"2"},"operation":{"sync":{}}});
        assert!(guard_action(&current, "u", "2", "sync").is_err());
        assert!(guard_action(&current, "different", "2", "refresh").is_err());
        assert!(guard_action(&current, "u", "1", "refresh").is_err());
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
        let payload = json!({"resource":resource("argoproj.io", "Application", "applications"),"action":"sync","uid":"u","resourceVersion":"2"});
        assert!(serde_json::from_value::<ActionIn>(payload.clone()).is_ok());
        let mut wrong = payload;
        wrong["resource_version"] = wrong
            .as_object_mut()
            .unwrap()
            .remove("resourceVersion")
            .unwrap();
        assert!(serde_json::from_value::<ActionIn>(wrong).is_err());
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
    async fn writes_use_selected_namespace_and_resource_version_precondition() {
        let (client, requests) = mock_client(200, 200);
        let input = ActionIn {
            resource: resource("argoproj.io", "Application", "applications"),
            action: "sync".into(),
            uid: "u".into(),
            resource_version: "2".into(),
        };
        assert_eq!(
            execute(client, input).await.unwrap(),
            json!({"requested":true})
        );
        let requests = requests.lock().unwrap();
        assert_eq!(
            requests[0].0,
            "GET /apis/argoproj.io/v1alpha1/namespaces/team/applications/apps"
        );
        assert!(requests[1]
            .0
            .starts_with("PATCH /apis/argoproj.io/v1alpha1/namespaces/team/applications/apps"));
        assert_eq!(
            requests[1].1["metadata"],
            json!({"uid":"u","resourceVersion":"2"})
        );
        assert_eq!(requests[1].1["operation"]["sync"]["prune"], false);
    }
    #[tokio::test]
    async fn rejected_patch_is_not_reported_as_success_and_stale_review_does_not_write() {
        for version in ["2", "old"] {
            let (client, requests) = mock_client(409, 200);
            let input = ActionIn {
                resource: resource("argoproj.io", "Application", "applications"),
                action: "refresh".into(),
                uid: "u".into(),
                resource_version: version.into(),
            };
            assert!(execute(client, input).await.is_err());
            assert_eq!(
                requests.lock().unwrap().len(),
                if version == "2" { 2 } else { 1 }
            );
        }
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
        assert!(!result.actions.is_empty());
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
    #[test]
    fn supported_actions_and_patch_generation_all_kinds() {
        // Non-namespaced resource gets no actions
        let mut non_ns = resource("argoproj.io", "Application", "applications");
        non_ns.namespaced = false;
        assert!(supported_actions(&non_ns).is_empty());

        // Unrecognized group gets no actions
        let unrec = resource("unknown.io", "App", "apps");
        assert!(supported_actions(&unrec).is_empty());

        // Unsupported version gets no actions
        let mut bad_ver = resource("argoproj.io", "Application", "applications");
        bad_ver.version = "v99".into();
        assert!(supported_actions(&bad_ver).is_empty());

        // ArgoCD Application actions and patches
        let argo = resource("argoproj.io", "Application", "applications");
        let actions = supported_actions(&argo);
        assert_eq!(actions, vec!["refresh", "hard-refresh", "sync"]);
        assert_eq!(
            action_patch(&argo, "hard-refresh", "tok").unwrap(),
            json!({"metadata":{"annotations":{"argocd.argoproj.io/refresh":"hard"}}})
        );
        assert!(action_patch(&argo, "unsupported-action", "tok").is_err());

        // Flux HelmRelease actions and patches
        let helm = resource("helm.toolkit.fluxcd.io", "HelmRelease", "helmreleases");
        let helm_actions = supported_actions(&helm);
        assert_eq!(
            helm_actions,
            vec!["suspend", "resume", "reconcile", "force", "reset"]
        );
        assert_eq!(
            action_patch(&helm, "suspend", "tok").unwrap(),
            json!({"spec":{"suspend":true}})
        );
        assert_eq!(
            action_patch(&helm, "resume", "tok").unwrap(),
            json!({"spec":{"suspend":false}})
        );
        let force_patch = action_patch(&helm, "force", "2026-01-01T00:00:00Z").unwrap();
        assert_eq!(
            force_patch["metadata"]["annotations"]["reconcile.fluxcd.io/forceAt"],
            "2026-01-01T00:00:00Z"
        );
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

        // 2. Action capability validation
        let act_cap = action_capability(cache.clone());
        assert_eq!(act_cap.id, "k8s.gitOpsAction");

        // Invalid action (unsupported action for Application)
        let err = (act_cap.handler)(json!({
            "resource": {
                "context": "cluster-1",
                "group": "argoproj.io",
                "version": "v1alpha1",
                "plural": "applications",
                "kind": "Application",
                "namespaced": true,
                "namespace": "argocd",
                "name": "billing"
            },
            "action": "non-existent-action",
            "uid": "12345",
            "resourceVersion": "1"
        }))
        .await
        .unwrap_err();
        assert!(matches!(
            err,
            srelens_capability::CapabilityError::InvalidInput(_)
        ));

        // Valid action but non-existent context -> Handler error
        let err = (act_cap.handler)(json!({
            "resource": {
                "context": "non-existent-cluster",
                "group": "argoproj.io",
                "version": "v1alpha1",
                "plural": "applications",
                "kind": "Application",
                "namespaced": true,
                "namespace": "argocd",
                "name": "billing"
            },
            "action": "refresh",
            "uid": "12345",
            "resourceVersion": "1"
        }))
        .await
        .unwrap_err();
        assert!(matches!(
            err,
            srelens_capability::CapabilityError::Handler(_)
        ));
    }
}
