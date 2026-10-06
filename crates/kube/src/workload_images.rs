//! Narrow workload image inventory: no environment, volume or Secret fields.
use crate::{client_cache::ClientCache, connect::request_timeout};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use kube::api::ListParams;
use kube::core::{ApiResource, DynamicObject, GroupVersionKind};
use kube::{Api, Client};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use srelens_capability::{Annotations, Capability, CapabilityError};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
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
    #[serde(default)]
    #[schemars(length(max = 8192))]
    pub cursor: Option<String>,
}
#[derive(Debug, Serialize, JsonSchema)]
pub struct WorkloadImages {
    pub items: Vec<WorkloadImage>,
    #[serde(rename = "nextCursor")]
    pub next_cursor: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ImageCursor {
    context: String,
    namespace: Option<String>,
    kind: ImageWorkloadKind,
    token: String,
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

async fn list_page(
    client: Client,
    context: &str,
    kind: ImageWorkloadKind,
    namespace: Option<String>,
    cursor: &str,
) -> Result<WorkloadImages, CapabilityError> {
    let namespace = namespace.filter(|value| !value.is_empty());
    let invalid_cursor = || {
        CapabilityError::InvalidInput("This image cursor belongs to another cluster, namespace or workload kind, or is invalid; refresh Images".into())
    };
    let token = if cursor.is_empty() {
        String::new()
    } else {
        if cursor.len() > 8192 {
            return Err(invalid_cursor());
        }
        let raw = URL_SAFE_NO_PAD
            .decode(cursor)
            .map_err(|_| invalid_cursor())?;
        let page: ImageCursor = serde_json::from_slice(&raw).map_err(|_| invalid_cursor())?;
        if page.context != context
            || page.namespace != namespace
            || page.kind != kind
            || page.token.is_empty()
        {
            return Err(invalid_cursor());
        }
        page.token
    };
    let resource = ApiResource::from_gvk(&GroupVersionKind::gvk("apps", "v1", kind.name()));
    let api: Api<DynamicObject> = match namespace
        .as_deref()
        .filter(|namespace| !namespace.is_empty())
    {
        Some(namespace) => Api::namespaced_with(client, namespace, &resource),
        None => Api::all_with(client, &resource),
    };
    let future = async move {
        let mut result = WorkloadImages {
            items: Vec::new(),
            next_cursor: String::new(),
        };
        let mut containers = 0usize;
        let page = api
            .list(&ListParams::default().limit(100).continue_token(&token))
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
        if let Some(token) = page.metadata.continue_.filter(|token| !token.is_empty()) {
            result.next_cursor = URL_SAFE_NO_PAD.encode(
                serde_json::to_vec(&ImageCursor {
                    context: context.into(),
                    namespace,
                    kind,
                    token,
                })
                .map_err(|error| CapabilityError::Handler(error.to_string()))?,
            );
            if result.next_cursor.len() > 8192 {
                return Err(invalid_cursor());
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

async fn list_complete(
    client: Client,
    context: &str,
    kind: ImageWorkloadKind,
    namespace: Option<String>,
    timeout: std::time::Duration,
) -> Result<WorkloadImages, CapabilityError> {
    let future = async {
        let mut result = WorkloadImages {
            items: Vec::new(),
            next_cursor: String::new(),
        };
        let mut cursor = String::new();
        let mut containers = 0;
        loop {
            let page = list_page(client.clone(), context, kind, namespace.clone(), &cursor).await?;
            containers += page
                .items
                .iter()
                .map(|row| row.containers.len())
                .sum::<usize>();
            if containers > 1000 || result.items.len() + page.items.len() > 1000 {
                return Err(CapabilityError::Handler("The image inventory exceeds 1,000 rows; narrow the namespace or use a paginated reader".into()));
            }
            result.items.extend(page.items);
            cursor = page.next_cursor;
            if cursor.is_empty() {
                break;
            }
        }
        if serde_json::to_vec(&result)
            .map_err(|error| CapabilityError::Handler(error.to_string()))?
            .len()
            > 4 * 1024 * 1024
        {
            return Err(CapabilityError::Handler("The image inventory exceeds its message limit; narrow the namespace or use a paginated reader".into()));
        }
        Ok(result)
    };
    tokio::time::timeout(timeout, future)
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
                match input.cursor {
                    Some(cursor) => {
                        list_page(client, &input.context, input.kind, input.namespace, &cursor)
                            .await
                    }
                    None => {
                        list_complete(
                            client,
                            &input.context,
                            input.kind,
                            input.namespace,
                            request_timeout(),
                        )
                        .await
                    }
                }
            }
        },
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn inventory_page(next: &str, count: usize) -> serde_json::Value {
        json!({"apiVersion":"apps/v1","kind":"DeploymentList","metadata":{"continue":next},"items":(0..count).map(|i| json!({
            "metadata":{"name":format!("web-{i}"),"namespace":"team","uid":format!("u-{i}"),"resourceVersion":"7"},
            "spec":{"template":{"spec":{"containers":[{"name":"app","image":"alpine:3.10"}]}}}
        })).collect::<Vec<_>>()})
    }

    #[tokio::test]
    async fn legacy_inventory_completes_all_pages_but_refuses_cumulative_overflow() {
        for count in [100, 600] {
            let (client, seen) = crate::test_support::fake_api(move |request| {
                inventory_page(
                    if request.query.contains("continue=next") {
                        ""
                    } else {
                        "next"
                    },
                    count,
                )
            });
            let result = list_complete(
                client,
                "cluster-a",
                ImageWorkloadKind::Deployment,
                Some("team".into()),
                request_timeout(),
            )
            .await;
            if count == 100 {
                let result = result.unwrap();
                assert_eq!(result.items.len(), 200);
                assert!(result.next_cursor.is_empty());
            } else {
                assert!(result.unwrap_err().to_string().contains("exceeds 1,000"));
            }
            assert_eq!(seen.lock().unwrap().len(), 2);
        }
    }

    #[tokio::test]
    async fn legacy_inventory_has_one_deadline_across_all_pages() {
        let service = tower::service_fn(|request: http::Request<kube::client::Body>| async move {
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            let next = if request
                .uri()
                .query()
                .unwrap_or_default()
                .contains("continue=next")
            {
                ""
            } else {
                "next"
            };
            Ok::<_, std::convert::Infallible>(
                http::Response::builder()
                    .header("content-type", "application/json")
                    .body(kube::client::Body::from(
                        inventory_page(next, 1).to_string().into_bytes(),
                    ))
                    .unwrap(),
            )
        });
        let result = list_complete(
            Client::new(service, "default"),
            "cluster-a",
            ImageWorkloadKind::Deployment,
            None,
            std::time::Duration::from_millis(45),
        )
        .await;
        assert!(result.unwrap_err().to_string().contains("timed out"));
    }

    #[tokio::test]
    async fn image_cursor_rejects_other_clusters_namespaces_and_kinds_before_a_read() {
        let (client, seen) = crate::test_support::fake_api(|_| inventory_page("next", 1));
        let first = list_page(
            client.clone(),
            "cluster-a",
            ImageWorkloadKind::Deployment,
            Some("team".into()),
            "",
        )
        .await
        .unwrap();
        for (context, kind, namespace) in [
            (
                "cluster-b",
                ImageWorkloadKind::Deployment,
                Some("team".into()),
            ),
            (
                "cluster-a",
                ImageWorkloadKind::StatefulSet,
                Some("team".into()),
            ),
            ("cluster-a", ImageWorkloadKind::Deployment, None),
        ] {
            let result =
                list_page(client.clone(), context, kind, namespace, &first.next_cursor).await;
            assert!(matches!(result, Err(CapabilityError::InvalidInput(_))));
        }
        assert_eq!(seen.lock().unwrap().len(), 1);
    }
    async fn list(
        client: Client,
        kind: ImageWorkloadKind,
        namespace: Option<String>,
    ) -> Result<WorkloadImages, CapabilityError> {
        list_page(client, "test-cluster", kind, namespace, "").await
    }

    #[tokio::test]
    async fn a_large_image_inventory_returns_a_bounded_page_instead_of_failing() {
        let (client, seen) = crate::test_support::fake_api(|_| {
            json!({"apiVersion":"apps/v1","kind":"DeploymentList","metadata":{"continue":"more"},"items":(0..100).map(|i| json!({
                "metadata":{"name":format!("web-{i}"),"namespace":"team","uid":format!("u-{i}"),"resourceVersion":"7"},
                "spec":{"template":{"spec":{"containers":[{"name":"app","image":"alpine:3.10"}]}}}
            })).collect::<Vec<_>>()})
        });
        let result = list(client, ImageWorkloadKind::Deployment, Some("team".into()))
            .await
            .unwrap();
        let result = serde_json::to_value(result).unwrap();
        assert_eq!(result["items"].as_array().unwrap().len(), 100);
        assert!(!result["nextCursor"].as_str().unwrap_or_default().is_empty());
        assert_eq!(seen.lock().unwrap().len(), 1);
    }

    #[test]
    fn paged_inventory_accepts_the_callers_cursor_payload() {
        let input = serde_json::from_value::<ListWorkloadImagesIn>(
            json!({"context":"cluster-a","namespace":"team","kind":"Deployment","cursor":"opaque-page"}),
        );
        assert!(input.is_ok());
    }

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
        let result = list(
            client.clone(),
            ImageWorkloadKind::Deployment,
            Some("team".into()),
        )
        .await
        .unwrap();
        assert_eq!(result.items.len(), 1);
        let second = list_page(
            client.clone(),
            "test-cluster",
            ImageWorkloadKind::Deployment,
            Some("team".into()),
            &result.next_cursor,
        )
        .await
        .unwrap();
        assert_eq!(second.items[0].name, "second");
        assert!(second.next_cursor.is_empty());
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
