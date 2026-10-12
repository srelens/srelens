//! The `k8s.whyPending` capability.
//!
//! Assembles structured root-cause evidence for pending pods:
//! scheduler node rejection reasons and counts, pod resource requests, unbound PVCs, and scheduler events.

use std::collections::BTreeMap;
use std::sync::Arc;

use k8s_openapi::api::core::v1::{Event, PersistentVolumeClaim, Pod, PodSpec};
use kube::api::ListParams;
use kube::Api;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use srelens_capability::{Annotations, Capability, CapabilityError};

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;
use crate::events::{event_first_timestamp, event_last_timestamp};
use crate::why_not_ready::CompactEventSummary;

/// Input arguments for `k8s.whyPending`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WhyPendingIn {
    pub context: String,
    pub namespace: String,
    pub pod: String,
}

/// Output evidence for `k8s.whyPending`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WhyPendingOut {
    pub pod: String,
    pub namespace: String,
    pub total_nodes_evaluated: usize,
    pub node_rejection_counts: Vec<NodeRejectionReasonCount>,
    pub pod_resource_requests: ResourceRequestsSummary,
    pub unbound_pvcs: Vec<UnboundPvcEvidence>,
    pub scheduler_events: Vec<CompactEventSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NodeRejectionReasonCount {
    pub reason: String,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourceRequestsSummary {
    pub cpu_millicores: i64,
    pub memory_mib: i64,
    pub gpu_requests: i64,
    pub other_requests: Vec<ResourceItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourceItem {
    pub name: String,
    pub quantity: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UnboundPvcEvidence {
    pub pvc_name: String,
    pub phase: String,
    pub storage_class: Option<String>,
    pub requested_storage: Option<String>,
}

/// Parses Kubernetes scheduler FailedScheduling message into evaluated node count and structured rejection counts.
pub fn parse_scheduler_rejections(message: &str) -> (usize, Vec<NodeRejectionReasonCount>) {
    let mut total_nodes = 0;
    let mut reason_map: BTreeMap<String, usize> = BTreeMap::new();

    // 1. Extract total nodes evaluated (e.g., "0/10 nodes are available:")
    if let Some(start_idx) = message.find("0/") {
        let after_slash = &message[start_idx + 2..];
        if let Some(space_idx) = after_slash.find(' ') {
            if let Ok(n) = after_slash[..space_idx].parse::<usize>() {
                total_nodes = n;
            }
        }
    }

    // 2. Extract reasons substring between "nodes are available:" and any preemption or trailing block
    if let Some(colon_idx) = message.find("nodes are available:") {
        let after_colon = &message[colon_idx + "nodes are available:".len()..];
        let main_section = if let Some(preempt_idx) = after_colon.find("preemption:") {
            &after_colon[..preempt_idx]
        } else {
            after_colon
        };

        for chunk in main_section.split(',') {
            let trimmed = chunk.trim().trim_end_matches('.');
            if trimmed.is_empty() {
                continue;
            }

            if let Some(space_idx) = trimmed.find(' ') {
                let first_word = &trimmed[..space_idx];
                let rest = trimmed[space_idx + 1..].trim();
                if let Ok(count) = first_word.parse::<usize>() {
                    *reason_map.entry(rest.to_string()).or_default() += count;
                } else {
                    *reason_map.entry(trimmed.to_string()).or_default() += 1;
                }
            } else {
                *reason_map.entry(trimmed.to_string()).or_default() += 1;
            }
        }
    }

    let mut result: Vec<NodeRejectionReasonCount> = reason_map
        .into_iter()
        .map(|(reason, count)| NodeRejectionReasonCount { reason, count })
        .collect();

    // Deterministic sort: count descending, then reason alphabetically ascending
    result.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.reason.cmp(&b.reason)));

    (total_nodes, result)
}

/// Calculate resource requests summary for a Pod across its containers.
pub fn calculate_pod_resource_requests(spec: Option<&PodSpec>) -> ResourceRequestsSummary {
    let mut cpu_millicores: i64 = 0;
    let mut memory_mib: i64 = 0;
    let mut gpu_requests: i64 = 0;
    let mut other_map: BTreeMap<String, i64> = BTreeMap::new();

    let discrete_gpu_keys = [
        "nvidia.com/gpu",
        "amd.com/gpu",
        "google.com/tpu",
        "nvidia.com/vgpu",
    ];

    if let Some(spec) = spec {
        for c in &spec.containers {
            if let Some(res) = &c.resources {
                if let Some(reqs) = &res.requests {
                    for (k, v) in reqs {
                        if k == "cpu" {
                            cpu_millicores += crate::metrics::cpu_millicores(&v.0);
                        } else if k == "memory" {
                            memory_mib += crate::metrics::mem_mib(&v.0);
                        } else if discrete_gpu_keys.contains(&k.as_str())
                            || k.starts_with("nvidia.com/mig-")
                        {
                            if let Ok(count) = v.0.trim().parse::<i64>() {
                                gpu_requests += count;
                            }
                        } else if let Ok(num) = v.0.trim().parse::<i64>() {
                            *other_map.entry(k.clone()).or_default() += num;
                        } else {
                            *other_map.entry(k.clone()).or_default() += 1;
                        }
                    }
                }
            }
        }
    }

    let other_requests = other_map
        .into_iter()
        .map(|(name, val)| ResourceItem {
            name,
            quantity: val.to_string(),
        })
        .collect();

    ResourceRequestsSummary {
        cpu_millicores,
        memory_mib,
        gpu_requests,
        other_requests,
    }
}

/// Pure analysis function for why a Pod is pending.
pub fn analyze_why_pending(
    pod: &Pod,
    events: &[Event],
    pvcs: &[Result<PersistentVolumeClaim, String>],
) -> WhyPendingOut {
    let name = pod.metadata.name.clone().unwrap_or_default();
    let namespace = pod.metadata.namespace.clone().unwrap_or_default();

    // 1. Gather scheduler events
    let mut scheduler_events = Vec::new();
    let mut latest_failed_scheduling_msg: Option<String> = None;

    for ev in events {
        let is_scheduler = ev
            .source
            .as_ref()
            .and_then(|s| s.component.as_deref())
            .map(|c| c.contains("scheduler"))
            .unwrap_or(false)
            || ev
                .reporting_component
                .as_deref()
                .map(|c| c.contains("scheduler"))
                .unwrap_or(false)
            || ev.reason.as_deref() == Some("FailedScheduling")
            || ev.reason.as_deref() == Some("Scheduled");

        if is_scheduler {
            let first_seen = event_first_timestamp(ev).map(|t| t.to_string());
            let last_seen = event_last_timestamp(ev).map(|t| t.to_string());

            if ev.reason.as_deref() == Some("FailedScheduling")
                && latest_failed_scheduling_msg.is_none()
            {
                latest_failed_scheduling_msg = ev.message.clone();
            }

            scheduler_events.push(CompactEventSummary {
                type_: ev.type_.clone().unwrap_or_else(|| "Warning".to_string()),
                reason: ev.reason.clone().unwrap_or_default(),
                message: ev.message.clone().unwrap_or_default(),
                count: ev.count.unwrap_or(1),
                first_seen,
                last_seen,
            });
        }
    }

    // Sort scheduler events by last_seen descending
    scheduler_events.sort_by(|a, b| b.last_seen.cmp(&a.last_seen));

    // 2. Parse scheduler node rejection counts from the latest FailedScheduling event
    let (total_nodes_evaluated, node_rejection_counts) =
        if let Some(msg) = &latest_failed_scheduling_msg {
            parse_scheduler_rejections(msg)
        } else {
            (0, Vec::new())
        };

    // 3. Compute pod resource requests
    let pod_resource_requests = calculate_pod_resource_requests(pod.spec.as_ref());

    // 4. Examine unbound PVCs
    let mut unbound_pvcs = Vec::new();
    for pvc_result in pvcs {
        match pvc_result {
            Ok(pvc) => {
                let pvc_name = pvc.metadata.name.clone().unwrap_or_default();
                let phase = pvc
                    .status
                    .as_ref()
                    .and_then(|s| s.phase.clone())
                    .unwrap_or_else(|| "Unknown".to_string());

                if phase != "Bound" {
                    let storage_class =
                        pvc.spec.as_ref().and_then(|s| s.storage_class_name.clone());
                    let requested_storage = pvc
                        .spec
                        .as_ref()
                        .and_then(|s| s.resources.as_ref())
                        .and_then(|r| r.requests.as_ref())
                        .and_then(|req| req.get("storage"))
                        .map(|q| q.0.clone());

                    unbound_pvcs.push(UnboundPvcEvidence {
                        pvc_name,
                        phase,
                        storage_class,
                        requested_storage,
                    });
                }
            }
            Err(missing_name) => {
                unbound_pvcs.push(UnboundPvcEvidence {
                    pvc_name: missing_name.clone(),
                    phase: "NotFound".to_string(),
                    storage_class: None,
                    requested_storage: None,
                });
            }
        }
    }

    // Sort unbound PVCs by name
    unbound_pvcs.sort_by(|a, b| a.pvc_name.cmp(&b.pvc_name));

    WhyPendingOut {
        pod: name,
        namespace,
        total_nodes_evaluated,
        node_rejection_counts,
        pod_resource_requests,
        unbound_pvcs,
        scheduler_events,
    }
}

/// `k8s.whyPending` capability definition.
pub fn why_pending_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<WhyPendingIn, WhyPendingOut, _, _>(
        "k8s.whyPending",
        "diagnoses why a pod is stuck pending by evaluating scheduler node rejection counts, pod resource requests, unbound PVCs, and scheduling events",
        Annotations::READ_ONLY,
        move |input: WhyPendingIn| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;

                let pod_api: Api<Pod> = Api::namespaced(client.clone(), &input.namespace);
                let pod = tokio::time::timeout(request_timeout(), pod_api.get(&input.pod))
                    .await
                    .map_err(|_| CapabilityError::Handler("fetch pod timed out".into()))?
                    .map_err(|e| {
                        CapabilityError::Handler(format!(
                            "Pod {}/{} not found: {}",
                            input.namespace, input.pod, e
                        ))
                    })?;

                // Fetch events for this pod
                let event_api: Api<Event> = Api::namespaced(client.clone(), &input.namespace);
                let event_params = ListParams::default().fields(&format!(
                    "involvedObject.name={},involvedObject.kind=Pod",
                    input.pod
                ));
                let events = tokio::time::timeout(request_timeout(), event_api.list(&event_params))
                    .await
                    .ok()
                    .and_then(|res| res.ok())
                    .map(|l| l.items)
                    .unwrap_or_default();

                // Fetch referenced PVCs if any
                let mut pvcs = Vec::new();
                if let Some(spec) = &pod.spec {
                    if let Some(volumes) = &spec.volumes {
                        let pvc_api: Api<PersistentVolumeClaim> =
                            Api::namespaced(client.clone(), &input.namespace);

                        for v in volumes {
                            if let Some(claim) = &v.persistent_volume_claim {
                                let claim_name = &claim.claim_name;
                                match tokio::time::timeout(
                                    request_timeout(),
                                    pvc_api.get(claim_name),
                                )
                                .await
                                {
                                    Ok(Ok(pvc)) => pvcs.push(Ok(pvc)),
                                    _ => pvcs.push(Err(claim_name.clone())),
                                }
                            }
                        }
                    }
                }

                Ok(analyze_why_pending(&pod, &events, &pvcs))
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::{
        Container, ObjectReference, PersistentVolumeClaimSpec, PersistentVolumeClaimStatus,
        PodSpec, ResourceRequirements, Volume,
    };
    use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, Time};
    use k8s_openapi::jiff::Timestamp;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    #[test]
    fn capability_has_expected_id() {
        let cap = why_pending_capability(ClientCache::new(PathBuf::from("/x")));
        assert_eq!(cap.id, "k8s.whyPending");
    }

    #[test]
    fn deserializes_caller_payload_and_denies_unknown_fields() {
        let valid = serde_json::json!({
            "context": "kind-srelens",
            "namespace": "default",
            "pod": "pending-worker-0"
        });
        let parsed: Result<WhyPendingIn, _> = serde_json::from_value(valid);
        assert!(parsed.is_ok());
        let input = parsed.unwrap();
        assert_eq!(input.context, "kind-srelens");
        assert_eq!(input.namespace, "default");
        assert_eq!(input.pod, "pending-worker-0");

        // Unknown field rejected
        let invalid = serde_json::json!({
            "context": "kind-srelens",
            "namespace": "default",
            "pod": "pending-worker-0",
            "extra": true
        });
        assert!(serde_json::from_value::<WhyPendingIn>(invalid).is_err());
    }

    #[test]
    fn parses_scheduler_rejections_deterministically() {
        let msg = "0/10 nodes are available: 3 Insufficient cpu, 3 Insufficient memory, 4 node(s) had untolerated taint {node-role.kubernetes.io/control-plane: }. preemption: 0/10 nodes are available: 10 Preemption is not helpful for scheduling.";
        let (total, rejections) = parse_scheduler_rejections(msg);

        assert_eq!(total, 10);
        assert_eq!(rejections.len(), 3);

        // Highest count first: 4 taints
        assert_eq!(
            rejections[0].reason,
            "node(s) had untolerated taint {node-role.kubernetes.io/control-plane: }"
        );
        assert_eq!(rejections[0].count, 4);

        // Equal counts (3) sorted alphabetically: Insufficient cpu before Insufficient memory
        assert_eq!(rejections[1].reason, "Insufficient cpu");
        assert_eq!(rejections[1].count, 3);
        assert_eq!(rejections[2].reason, "Insufficient memory");
        assert_eq!(rejections[2].count, 3);
    }

    #[test]
    fn detects_unbound_and_missing_pvcs() {
        let pod = Pod {
            metadata: ObjectMeta {
                name: Some("data-processor".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            },
            spec: Some(PodSpec {
                volumes: Some(vec![
                    Volume {
                        name: "cache".to_string(),
                        persistent_volume_claim: Some(
                            k8s_openapi::api::core::v1::PersistentVolumeClaimVolumeSource {
                                claim_name: "cache-pvc".to_string(),
                                read_only: Some(false),
                            },
                        ),
                        ..Default::default()
                    },
                    Volume {
                        name: "missing".to_string(),
                        persistent_volume_claim: Some(
                            k8s_openapi::api::core::v1::PersistentVolumeClaimVolumeSource {
                                claim_name: "ghost-pvc".to_string(),
                                read_only: Some(false),
                            },
                        ),
                        ..Default::default()
                    },
                ]),
                containers: vec![Container {
                    name: "worker".to_string(),
                    resources: Some(ResourceRequirements {
                        requests: Some({
                            let mut req = BTreeMap::new();
                            req.insert("cpu".to_string(), Quantity("500m".to_string()));
                            req.insert("memory".to_string(), Quantity("1Gi".to_string()));
                            req.insert("nvidia.com/gpu".to_string(), Quantity("2".to_string()));
                            req
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };

        let now = Timestamp::now();
        let events = vec![Event {
            metadata: ObjectMeta {
                name: Some("sched-ev".to_string()),
                ..Default::default()
            },
            type_: Some("Warning".to_string()),
            reason: Some("FailedScheduling".to_string()),
            message: Some(
                "0/2 nodes are available: 2 pod has unbound immediate PersistentVolumeClaims."
                    .to_string(),
            ),
            count: Some(3),
            involved_object: ObjectReference {
                kind: Some("Pod".to_string()),
                name: Some("data-processor".to_string()),
                ..Default::default()
            },
            last_timestamp: Some(Time(now)),
            ..Default::default()
        }];

        let existing_pvc = PersistentVolumeClaim {
            metadata: ObjectMeta {
                name: Some("cache-pvc".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            },
            spec: Some(PersistentVolumeClaimSpec {
                storage_class_name: Some("standard".to_string()),
                resources: Some(k8s_openapi::api::core::v1::VolumeResourceRequirements {
                    requests: Some({
                        let mut req = BTreeMap::new();
                        req.insert("storage".to_string(), Quantity("50Gi".to_string()));
                        req
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            status: Some(PersistentVolumeClaimStatus {
                phase: Some("Pending".to_string()),
                ..Default::default()
            }),
        };

        let pvcs = vec![Ok(existing_pvc), Err("ghost-pvc".to_string())];

        let out = analyze_why_pending(&pod, &events, &pvcs);

        assert_eq!(out.pod, "data-processor");
        assert_eq!(out.total_nodes_evaluated, 2);
        assert_eq!(out.node_rejection_counts.len(), 1);
        assert_eq!(
            out.node_rejection_counts[0].reason,
            "pod has unbound immediate PersistentVolumeClaims"
        );

        // Resource requests
        assert_eq!(out.pod_resource_requests.cpu_millicores, 500);
        assert_eq!(out.pod_resource_requests.memory_mib, 1024);
        assert_eq!(out.pod_resource_requests.gpu_requests, 2);

        // Unbound PVCs
        assert_eq!(out.unbound_pvcs.len(), 2);
        assert_eq!(out.unbound_pvcs[0].pvc_name, "cache-pvc");
        assert_eq!(out.unbound_pvcs[0].phase, "Pending");
        assert_eq!(
            out.unbound_pvcs[0].storage_class.as_deref(),
            Some("standard")
        );
        assert_eq!(
            out.unbound_pvcs[0].requested_storage.as_deref(),
            Some("50Gi")
        );

        assert_eq!(out.unbound_pvcs[1].pvc_name, "ghost-pvc");
        assert_eq!(out.unbound_pvcs[1].phase, "NotFound");
    }
}
