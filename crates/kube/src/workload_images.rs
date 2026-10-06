//! Narrow workload image inventory: no environment, volume or Secret fields.
use crate::{client_cache::ClientCache, connect::request_timeout};
use kube::api::ListParams;
use kube::core::{ApiResource, DynamicObject, GroupVersionKind};
use kube::{Api, Client};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use srelens_capability::{Annotations, Capability, CapabilityError};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema)]
pub enum ImageWorkloadKind {
    Deployment,
    StatefulSet,
    DaemonSet,
}
impl ImageWorkloadKind {
    fn name(self) -> &'static str {
        match self {
            Self::Deployment => "Deployment",
            Self::StatefulSet => "StatefulSet",
            Self::DaemonSet => "DaemonSet",
        }
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListWorkloadImagesIn {
    pub context: String,
    #[serde(default)]
    pub namespace: Option<String>,
    pub kind: ImageWorkloadKind,
}
#[derive(Debug, Serialize, JsonSchema)]
pub struct WorkloadImages {
    pub items: Vec<WorkloadImage>,
}
#[derive(Debug, Serialize, JsonSchema)]
pub struct WorkloadImage {
    pub kind: ImageWorkloadKind,
    pub namespace: String,
    pub name: String,
    pub uid: String,
    #[serde(rename = "resourceVersion")]
    pub resource_version: String,
    pub containers: Vec<ImageContainer>,
}
#[derive(Debug, Serialize, JsonSchema)]
pub struct ImageContainer {
    pub name: String,
    #[serde(rename = "type")]
    pub container_type: String,
    pub image: String,
}

fn summarize(
    workload: &DynamicObject,
    kind: ImageWorkloadKind,
) -> Result<WorkloadImage, CapabilityError> {
    let spec = &workload.data["spec"]["template"]["spec"];
    let regular = spec["containers"].as_array().ok_or_else(|| {
        CapabilityError::Handler("The workload has no readable container inventory".into())
    })?;
    let mut containers = Vec::new();
    for (values, container_type) in [
        (Some(regular), "regular"),
        (spec["initContainers"].as_array(), "init"),
    ] {
        for container in values.into_iter().flatten() {
            let name = container["name"]
                .as_str()
                .ok_or_else(|| CapabilityError::Handler("A container has no name".into()))?;
            let image = container["image"].as_str().unwrap_or_default();
            containers.push(ImageContainer {
                name: name.into(),
                container_type: container_type.into(),
                image: image.into(),
            });
        }
    }
    Ok(WorkloadImage {
        kind,
        namespace: workload.metadata.namespace.clone().unwrap_or_default(),
        name: workload.metadata.name.clone().unwrap_or_default(),
        uid: workload.metadata.uid.clone().unwrap_or_default(),
        resource_version: workload
            .metadata
            .resource_version
            .clone()
            .unwrap_or_default(),
        containers,
    })
}

async fn list(
    client: Client,
    kind: ImageWorkloadKind,
    namespace: Option<String>,
) -> Result<WorkloadImages, CapabilityError> {
    let resource = ApiResource::from_gvk(&GroupVersionKind::gvk("apps", "v1", kind.name()));
    let api: Api<DynamicObject> = match namespace
        .as_deref()
        .filter(|namespace| !namespace.is_empty())
    {
        Some(namespace) => Api::namespaced_with(client, namespace, &resource),
        None => Api::all_with(client, &resource),
    };
    let future = async move {
        let mut result = WorkloadImages { items: Vec::new() };
        let mut containers = 0usize;
        let mut cursor = String::new();
        loop {
            let page = api
                .list(&ListParams::default().limit(100).continue_token(&cursor))
                .await
                .map_err(|error| CapabilityError::Handler(error.to_string()))?;
            for item in page.items {
                let row = summarize(&item, kind)?;
                containers += row.containers.len();
                if containers > 1000 || result.items.len() >= 1000 {
                    return Err(CapabilityError::Handler(
                        "The image inventory exceeds 1,000 rows; narrow the namespace".into(),
                    ));
                }
                result.items.push(row);
            }
            cursor = page.metadata.continue_.unwrap_or_default();
            if cursor.is_empty() {
                break;
            }
        }
        if serde_json::to_vec(&result)
            .map_err(|error| CapabilityError::Handler(error.to_string()))?
            .len()
            > 4 * 1024 * 1024
        {
            return Err(CapabilityError::Handler(
                "The image inventory exceeds its message limit; narrow the namespace".into(),
            ));
        }
        Ok(result)
    };
    tokio::time::timeout(request_timeout(), future)
        .await
        .map_err(|_| CapabilityError::Handler("Workload image discovery timed out".into()))?
}

pub fn list_workload_images_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ListWorkloadImagesIn, WorkloadImages, _, _>(
        "k8s.listWorkloadImages",
        "List regular and init-container images from a fixed workload kind",
        Annotations::READ_ONLY,
        move |input| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                list(client, input.kind, input.namespace).await
            }
        },
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn images_include_regular_and_init_containers_without_secret_details() {
        let workload: kube::core::DynamicObject = serde_json::from_value(json!({
            "apiVersion":"apps/v1", "kind":"Deployment",
            "metadata":{"name":"web","namespace":"team","uid":"uid-7","resourceVersion":"42","annotations":{"private":"do-not-return"}},
            "spec":{"template":{"spec":{
                "containers":[{"name":"app","image":"alpine:3.9","env":[{"name":"TOKEN","value":"do-not-return"}]}],
                "initContainers":[{"name":"prepare","image":"busybox:1.36","envFrom":[{"secretRef":{"name":"do-not-return"}}]}],
                "volumes":[{"secret":{"secretName":"do-not-return"}}]
            }}}
        })).unwrap();
        let row = summarize(&workload, ImageWorkloadKind::Deployment).unwrap();
        let value = serde_json::to_value(row).unwrap();
        assert_eq!(
            value,
            json!({"kind":"Deployment","namespace":"team","name":"web","uid":"uid-7","resourceVersion":"42","containers":[
                {"name":"app","type":"regular","image":"alpine:3.9"},
                {"name":"prepare","type":"init","image":"busybox:1.36"}
            ]})
        );
        assert!(!value.to_string().contains("do-not-return"));
    }

    #[tokio::test]
    async fn image_discovery_pages_the_pinned_namespace_and_refuses_partial_overflow() {
        let (client, seen) = crate::test_support::fake_api(|request| {
            let next = if request.query.contains("continue=next") {
                ""
            } else {
                "next"
            };
            json!({"apiVersion":"apps/v1","kind":"DeploymentList","metadata":{"continue":next},"items":[{
                "metadata":{"name":if next.is_empty(){"second"}else{"first"},"namespace":"team","uid":"u","resourceVersion":"7"},
                "spec":{"template":{"spec":{"containers":[{"name":"app","image":"alpine:3.9"}]}}}
            }]})
        });
        let result = list(client, ImageWorkloadKind::Deployment, Some("team".into()))
            .await
            .unwrap();
        assert_eq!(result.items.len(), 2);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].path, "/apis/apps/v1/namespaces/team/deployments");
        assert!(seen
            .iter()
            .all(|request| request.query.contains("limit=100")));
        drop(seen);
        let (client, _) = crate::test_support::fake_api(|_| {
            json!({"apiVersion":"apps/v1","kind":"DeploymentList","metadata":{},"items":[{
                "metadata":{"name":"large","namespace":"team"}, "spec":{"template":{"spec":{"containers":vec![json!({"name":"app","image":"alpine"});1001]}}}
            }]})
        });
        let error = list(client, ImageWorkloadKind::Deployment, None)
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("narrow the namespace"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn a_denied_image_list_is_not_an_empty_cluster() {
        let service = tower::service_fn(|_: http::Request<kube::client::Body>| async {
            Ok::<_, std::convert::Infallible>(http::Response::builder().status(403).header("content-type","application/json")
                .body(kube::client::Body::from(json!({"kind":"Status","apiVersion":"v1","status":"Failure","reason":"Forbidden","message":"workloads forbidden","code":403}).to_string().into_bytes())).unwrap())
        });
        let error = list(
            kube::Client::new(service, "default"),
            ImageWorkloadKind::DaemonSet,
            None,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("forbidden"), "{error}");
    }
}
