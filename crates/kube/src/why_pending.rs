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

/// Parse a Kubernetes resource quantity into base unit (f64).
pub fn parse_k8s_quantity(s: &str) -> f64 {
    let s = s.trim();
    const SUFFIXES: [(&str, f64); 16] = [
        ("Ki", 1024.0),
        ("Mi", 1024.0 * 1024.0),
        ("Gi", 1024.0 * 1024.0 * 1024.0),
        ("Ti", 1024.0 * 1024.0 * 1024.0 * 1024.0),
        ("Pi", 1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0),
        ("Ei", 1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0),
        ("k", 1e3),
        ("M", 1e6),
        ("G", 1e9),
        ("T", 1e12),
        ("P", 1e15),
        ("E", 1e18),
        ("m", 1e-3),
        ("u", 1e-6),
        ("n", 1e-9),
        ("", 1.0),
    ];
    for (suffix, scale) in &SUFFIXES {
        if !suffix.is_empty() && s.ends_with(suffix) {
            let num_str = s[..s.len() - suffix.len()].trim();
            if let Ok(num) = num_str.parse::<f64>() {
                return num * scale;
            }
        }
    }
    s.parse::<f64>().unwrap_or(0.0)
}

/// Formats a base unit value back to a concise Kubernetes quantity string.
pub fn format_k8s_quantity(val: f64) -> String {
    if val <= 0.0 {
        return "0".to_string();
    }
    const GI: f64 = 1024.0 * 1024.0 * 1024.0;
    const MI: f64 = 1024.0 * 1024.0;
    const KI: f64 = 1024.0;

    if val >= GI && (val % GI).abs() < 1e-6 {
        format!("{:.0}Gi", val / GI)
    } else if val >= MI && (val % MI).abs() < 1e-6 {
        format!("{:.0}Mi", val / MI)
    } else if val >= KI && (val % KI).abs() < 1e-6 {
        format!("{:.0}Ki", val / KI)
    } else if val >= 1e9 && (val % 1e9).abs() < 1e-6 {
        format!("{:.0}G", val / 1e9)
    } else if val >= 1e6 && (val % 1e6).abs() < 1e-6 {
        format!("{:.0}M", val / 1e6)
    } else if val >= 1e3 && (val % 1e3).abs() < 1e-6 {
        format!("{:.0}k", val / 1e3)
    } else if val.fract().abs() < 1e-6 {
        format!("{:.0}", val)
    } else {
        format!("{:.2}", val)
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string()
    }
}

#[derive(Default, Clone, Debug)]
struct ContainerDemands {
    cpu_millicores: i64,
    memory_mib: i64,
    gpu_requests: i64,
    other: BTreeMap<String, f64>,
}

impl ContainerDemands {
    fn add(&mut self, other: &ContainerDemands) {
        self.cpu_millicores += other.cpu_millicores;
        self.memory_mib += other.memory_mib;
        self.gpu_requests += other.gpu_requests;
        for (k, v) in &other.other {
            *self.other.entry(k.clone()).or_default() += v;
        }
    }

    fn max_with(&mut self, other: &ContainerDemands) {
        self.cpu_millicores = self.cpu_millicores.max(other.cpu_millicores);
        self.memory_mib = self.memory_mib.max(other.memory_mib);
        self.gpu_requests = self.gpu_requests.max(other.gpu_requests);
        for (k, v) in &other.other {
            let entry = self.other.entry(k.clone()).or_default();
            *entry = entry.max(*v);
        }
    }
}

fn extract_container_demands(c: &k8s_openapi::api::core::v1::Container) -> ContainerDemands {
    let mut d = ContainerDemands::default();
    let discrete_gpu_keys = [
        "nvidia.com/gpu",
        "amd.com/gpu",
        "google.com/tpu",
        "nvidia.com/vgpu",
    ];

    if let Some(reqs) = c.resources.as_ref().and_then(|r| r.requests.as_ref()) {
        for (k, v) in reqs {
            if k == "cpu" {
                d.cpu_millicores += crate::metrics::cpu_millicores(&v.0);
            } else if k == "memory" {
                d.memory_mib += crate::metrics::mem_mib(&v.0);
            } else if discrete_gpu_keys.contains(&k.as_str()) || k.starts_with("nvidia.com/mig-") {
                if let Ok(count) = v.0.trim().parse::<i64>() {
                    d.gpu_requests += count;
                }
            } else {
                let parsed = parse_k8s_quantity(&v.0);
                *d.other.entry(k.clone()).or_default() += parsed;
            }
        }
    }
    d
}

fn extract_overhead_demands(
    overhead: &BTreeMap<String, k8s_openapi::apimachinery::pkg::api::resource::Quantity>,
) -> ContainerDemands {
    let mut d = ContainerDemands::default();
    let discrete_gpu_keys = [
        "nvidia.com/gpu",
        "amd.com/gpu",
        "google.com/tpu",
        "nvidia.com/vgpu",
    ];

    for (k, v) in overhead {
        if k == "cpu" {
            d.cpu_millicores += crate::metrics::cpu_millicores(&v.0);
        } else if k == "memory" {
            d.memory_mib += crate::metrics::mem_mib(&v.0);
        } else if discrete_gpu_keys.contains(&k.as_str()) || k.starts_with("nvidia.com/mig-") {
            if let Ok(count) = v.0.trim().parse::<i64>() {
                d.gpu_requests += count;
            }
        } else {
            let parsed = parse_k8s_quantity(&v.0);
            *d.other.entry(k.clone()).or_default() += parsed;
        }
    }
    d
}

/// Calculate effective pod resource requests across init containers, native sidecars,
/// app containers, and overhead according to Kubernetes scheduling rules:
/// effective = max(peak_init_demand, sum(app_containers) + sum(sidecars)) + overhead.
pub fn calculate_pod_resource_requests(spec: Option<&PodSpec>) -> ResourceRequestsSummary {
    let Some(spec) = spec else {
        return ResourceRequestsSummary {
            cpu_millicores: 0,
            memory_mib: 0,
            gpu_requests: 0,
            other_requests: Vec::new(),
        };
    };

    // Track running sidecars and peak init phase demand
    let mut running_sidecars = ContainerDemands::default();
    let mut max_init_demand = ContainerDemands::default();

    if let Some(init_containers) = &spec.init_containers {
        for c in init_containers {
            let c_demands = extract_container_demands(c);
            let is_sidecar = c.restart_policy.as_deref() == Some("Always");
            if is_sidecar {
                running_sidecars.add(&c_demands);
                max_init_demand.max_with(&running_sidecars);
            } else {
                let mut step_demand = running_sidecars.clone();
                step_demand.add(&c_demands);
                max_init_demand.max_with(&step_demand);
            }
        }
    }

    // App phase: app containers + all sidecars concurrently running
    let mut app_phase = running_sidecars;
    for c in &spec.containers {
        app_phase.add(&extract_container_demands(c));
    }

    // Effective demand before overhead: max(init_peak, app_phase)
    let mut effective = app_phase;
    effective.max_with(&max_init_demand);

    // Add overhead if specified
    if let Some(overhead) = &spec.overhead {
        effective.add(&extract_overhead_demands(overhead));
    }

    let other_requests = effective
        .other
        .into_iter()
        .map(|(name, val)| ResourceItem {
            name,
            quantity: format_k8s_quantity(val),
        })
        .collect();

    ResourceRequestsSummary {
        cpu_millicores: effective.cpu_millicores,
        memory_mib: effective.memory_mib,
        gpu_requests: effective.gpu_requests,
        other_requests,
    }
}

/// Pure analysis function for why a Pod is pending.
pub fn analyze_why_pending(
    pod: &Pod,
    events: &[Event],
    pvcs: &[Result<PersistentVolumeClaim, (String, String)>],
) -> WhyPendingOut {
    let name = pod.metadata.name.clone().unwrap_or_default();
    let namespace = pod.metadata.namespace.clone().unwrap_or_default();

    // 1. Gather scheduler events
    let mut scheduler_events = Vec::new();

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

    // Sort scheduler events by last_seen descending so newest events come first
    scheduler_events.sort_by(|a, b| b.last_seen.cmp(&a.last_seen));

    // Find the latest FailedScheduling event message AFTER sorting by last_seen descending
    let latest_failed_scheduling_msg = scheduler_events
        .iter()
        .find(|ev| ev.reason == "FailedScheduling")
        .map(|ev| ev.message.clone());

    // 2. Parse scheduler node rejection counts from the latest FailedScheduling event
    let (total_nodes_evaluated, node_rejection_counts) =
        if let Some(msg) = &latest_failed_scheduling_msg {
            parse_scheduler_rejections(msg)
        } else {
            (0, Vec::new())
        };

    // 3. Compute effective pod resource requests
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
            Err((missing_name, failure_phase)) => {
                unbound_pvcs.push(UnboundPvcEvidence {
                    pvc_name: missing_name.clone(),
                    phase: failure_phase.clone(),
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
                    .map_err(|e| match e {
                        kube::Error::Api(ref ae) if ae.code == 404 => CapabilityError::Handler(format!(
                            "Pod {}/{} not found",
                            input.namespace, input.pod
                        )),
                        other => CapabilityError::Handler(format!(
                            "Failed to fetch Pod {}/{}: {}",
                            input.namespace, input.pod, other
                        )),
                    })?;

                // Fetch events for this pod, bounded and UID-filtered
                let event_api: Api<Event> = Api::namespaced(client.clone(), &input.namespace);
                let pod_uid = pod.metadata.uid.as_deref().unwrap_or_default();
                let event_params = ListParams::default().fields(&format!(
                    "involvedObject.name={},involvedObject.kind=Pod",
                    input.pod
                ));
                let (raw_events, _) = crate::list_cap::list_capped(&event_api, event_params)
                    .await
                    .map_err(|e| e.into_capability_error("list events"))?;

                let events: Vec<_> = raw_events
                    .into_iter()
                    .filter(|ev| {
                        if !pod_uid.is_empty() {
                            if let Some(ev_uid) = &ev.involved_object.uid {
                                return ev_uid == pod_uid;
                            }
                        }
                        true
                    })
                    .collect();

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
                                    Ok(Err(kube::Error::Api(ae))) if ae.code == 404 => {
                                        pvcs.push(Err((claim_name.clone(), "NotFound".to_string())))
                                    }
                                    Ok(Err(_other)) => {
                                        pvcs.push(Err((claim_name.clone(), "FetchFailed".to_string())))
                                    }
                                    Err(_) => {
                                        pvcs.push(Err((claim_name.clone(), "FetchTimeout".to_string())))
                                    }
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

        let pvcs = vec![
            Ok(existing_pvc),
            Err(("ghost-pvc".to_string(), "NotFound".to_string())),
        ];

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

    #[test]
    fn calculates_effective_demand_with_init_containers_sidecars_and_overhead() {
        let pod = Pod {
            metadata: ObjectMeta {
                name: Some("heavy-pod".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            },
            spec: Some(PodSpec {
                init_containers: Some(vec![
                    // Native sidecar: runs alongside init containers and app containers
                    Container {
                        name: "vault-agent".to_string(),
                        restart_policy: Some("Always".to_string()),
                        resources: Some(ResourceRequirements {
                            requests: Some({
                                let mut req = BTreeMap::new();
                                req.insert("cpu".to_string(), Quantity("500m".to_string()));
                                req.insert("memory".to_string(), Quantity("512Mi".to_string()));
                                req.insert(
                                    "ephemeral-storage".to_string(),
                                    Quantity("2Gi".to_string()),
                                );
                                req
                            }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    // Standard init: runs while vault-agent is running
                    Container {
                        name: "db-migrate".to_string(),
                        resources: Some(ResourceRequirements {
                            requests: Some({
                                let mut req = BTreeMap::new();
                                req.insert("cpu".to_string(), Quantity("2000m".to_string()));
                                req.insert("memory".to_string(), Quantity("2Gi".to_string()));
                                req
                            }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                ]),
                containers: vec![Container {
                    name: "app".to_string(),
                    resources: Some(ResourceRequirements {
                        requests: Some({
                            let mut req = BTreeMap::new();
                            req.insert("cpu".to_string(), Quantity("1000m".to_string()));
                            req.insert("memory".to_string(), Quantity("1Gi".to_string()));
                            req.insert("nvidia.com/gpu".to_string(), Quantity("1".to_string()));
                            req
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                }],
                overhead: Some({
                    let mut oh = BTreeMap::new();
                    oh.insert("cpu".to_string(), Quantity("100m".to_string()));
                    oh.insert("memory".to_string(), Quantity("128Mi".to_string()));
                    oh
                }),
                ..Default::default()
            }),
            ..Default::default()
        };

        let summary = calculate_pod_resource_requests(pod.spec.as_ref());

        // Effective CPU:
        // Init peak: db-migrate (2000m) + vault-agent sidecar (500m) = 2500m
        // App phase: app (1000m) + vault-agent sidecar (500m) = 1500m
        // Max(2500, 1500) = 2500m + 100m overhead = 2600m
        assert_eq!(summary.cpu_millicores, 2600);

        // Effective Memory:
        // Init peak: db-migrate (2048 MiB) + vault-agent sidecar (512 MiB) = 2560 MiB
        // App phase: app (1024 MiB) + vault-agent sidecar (512 MiB) = 1536 MiB
        // Max(2560, 1536) = 2560 MiB + 128 MiB overhead = 2688 MiB
        assert_eq!(summary.memory_mib, 2688);

        // GPU:
        assert_eq!(summary.gpu_requests, 1);

        // Ephemeral storage preserved without falling back to "1":
        let eph = summary
            .other_requests
            .iter()
            .find(|r| r.name == "ephemeral-storage");
        assert!(eph.is_some());
        assert_eq!(eph.unwrap().quantity, "2Gi");
    }

    #[test]
    fn picks_latest_failed_scheduling_event_by_timestamp() {
        let pod = Pod {
            metadata: ObjectMeta {
                name: Some("test-pod".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            },
            spec: Some(PodSpec {
                containers: vec![Container {
                    name: "app".to_string(),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };

        let t1 = Timestamp::now();
        let t2 = Timestamp::from_second(t1.as_second() + 10).unwrap();

        // Older event is placed FIRST in the array
        let older_event = Event {
            metadata: ObjectMeta {
                name: Some("older-ev".to_string()),
                ..Default::default()
            },
            reason: Some("FailedScheduling".to_string()),
            message: Some("0/10 nodes are available: 10 Insufficient cpu.".to_string()),
            last_timestamp: Some(Time(t1)),
            ..Default::default()
        };

        // Newer event is placed SECOND in the array
        let newer_event = Event {
            metadata: ObjectMeta {
                name: Some("newer-ev".to_string()),
                ..Default::default()
            },
            reason: Some("FailedScheduling".to_string()),
            message: Some("0/5 nodes are available: 5 node(s) had untolerated taint.".to_string()),
            last_timestamp: Some(Time(t2)),
            ..Default::default()
        };

        let events = vec![older_event, newer_event];
        let out = analyze_why_pending(&pod, &events, &[]);

        // Must pick the newer event's facts (5 nodes, taint) instead of older (10 nodes, cpu)
        assert_eq!(out.total_nodes_evaluated, 5);
        assert_eq!(out.node_rejection_counts.len(), 1);
        assert_eq!(
            out.node_rejection_counts[0].reason,
            "node(s) had untolerated taint"
        );
        assert_eq!(out.node_rejection_counts[0].count, 5);
    }

    #[test]
    fn truthfully_differentiates_pvc_errors() {
        let pod = Pod {
            metadata: ObjectMeta {
                name: Some("test-pod".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };

        let pvcs = vec![
            Err(("pvc-404".to_string(), "NotFound".to_string())),
            Err(("pvc-timeout".to_string(), "FetchTimeout".to_string())),
            Err(("pvc-forbidden".to_string(), "FetchFailed".to_string())),
        ];

        let out = analyze_why_pending(&pod, &[], &pvcs);

        assert_eq!(out.unbound_pvcs.len(), 3);
        assert_eq!(out.unbound_pvcs[0].pvc_name, "pvc-404");
        assert_eq!(out.unbound_pvcs[0].phase, "NotFound");

        assert_eq!(out.unbound_pvcs[1].pvc_name, "pvc-forbidden");
        assert_eq!(out.unbound_pvcs[1].phase, "FetchFailed");

        assert_eq!(out.unbound_pvcs[2].pvc_name, "pvc-timeout");
        assert_eq!(out.unbound_pvcs[2].phase, "FetchTimeout");
    }
}
