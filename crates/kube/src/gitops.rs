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
    pub events: Vec<Value>,
    #[serde(rename = "eventsError")]
    pub events_error: Option<String>,
}

pub fn supported_actions(r: &ResourceIn) -> Vec<String> {
    if !r.namespaced {
        return vec![];
    }
    let mut actions: Vec<&str> = match (r.group.as_str(), r.kind.as_str(), r.plural.as_str()) {
        ("kustomize.toolkit.fluxcd.io", "Kustomization", "kustomizations")
        | ("source.toolkit.fluxcd.io", "GitRepository", "gitrepositories")
        | ("source.toolkit.fluxcd.io", "HelmRepository", "helmrepositories")
        | ("source.toolkit.fluxcd.io", "HelmChart", "helmcharts")
        | ("source.toolkit.fluxcd.io", "Bucket", "buckets")
        | ("source.toolkit.fluxcd.io", "OCIRepository", "ocirepositories")
        | ("image.toolkit.fluxcd.io", "ImageRepository", "imagerepositories")
        | ("image.toolkit.fluxcd.io", "ImageUpdateAutomation", "imageupdateautomations") => {
            vec!["suspend", "resume", "reconcile"]
        }
        ("helm.toolkit.fluxcd.io", "HelmRelease", "helmreleases") => {
            vec!["suspend", "resume", "reconcile", "force", "reset"]
        }
        ("argoproj.io", "Application", "applications") => vec!["refresh", "hard-refresh", "sync"],
        _ => vec![],
    };
    actions.drain(..).map(str::to_owned).collect()
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
    if matches!(action, "reconcile" | "force" | "reset") && current["spec"]["suspend"] == true {
        return Err("Resume this resource before requesting reconciliation".into());
    }
    if !current["metadata"]["deletionTimestamp"].is_null() {
        return Err("Resource is being deleted".into());
    }
    Ok(())
}
async fn inspect(client: Client, r: &ResourceIn) -> Result<ResourceOut, String> {
    r.validate()?;
    let object = r
        .api(client.clone())
        .get(&r.name)
        .await
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
    let (events, events_error) = if uid.is_empty() {
        (vec![], Some("Resource UID is unavailable".into()))
    } else {
        match events_api.list(&kube::api::ListParams::default().fields(&format!("involvedObject.uid={uid}")).limit(100)).await {
            Ok(list) => (list.items.into_iter().map(|e| json!({"type":e.type_,"reason":e.reason,"message":e.message,"count":e.count})).collect(),None),
            Err(e) => (vec![],Some(e.to_string())),
        }
    };
    Ok(ResourceOut {
        resource,
        actions: supported_actions(r),
        events,
        events_error,
    })
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
                tokio::time::timeout(request_timeout(), inspect(client, &input))
                    .await
                    .map_err(|_| CapabilityError::Handler("Resource request timed out".into()))?
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
        serde_json::from_value(json!({"context":"cluster/a","group":group,"version":"v1","kind":kind,"plural":plural,"namespaced":true,"namespace":"team","name":"apps"})).unwrap()
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
    fn mock_client(
        patch_status: u16,
        events_status: u16,
    ) -> (Client, Arc<std::sync::Mutex<Vec<(String, Value)>>>) {
        let requests = Arc::new(std::sync::Mutex::new(vec![]));
        let captured = requests.clone();
        let service = tower::service_fn(move |request: http::Request<kube::client::Body>| {
            let captured = captured.clone();
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
                    json!({"apiVersion":"v1","kind":"EventList","metadata":{},"items":[]})
                } else {
                    json!({"apiVersion":"argoproj.io/v1","kind":"Application","metadata":{"name":"apps","namespace":"team","uid":"u","resourceVersion":"2","managedFields":[]},"spec":{},"status":{}})
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
            "GET /apis/argoproj.io/v1/namespaces/team/applications/apps"
        );
        assert!(requests[1]
            .0
            .starts_with("PATCH /apis/argoproj.io/v1/namespaces/team/applications/apps"));
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
}
