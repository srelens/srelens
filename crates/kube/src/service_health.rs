//! The `k8s.serviceHealthProbe` capability.
//!
//! Evaluates the health, targetPort alignment, and endpoint readiness of a Kubernetes Service
//! in a single atomic call without requiring multiple investigative queries.

use std::collections::BTreeSet;
use std::sync::Arc;

use k8s_openapi::api::core::v1::{Pod, Service};
use k8s_openapi::api::discovery::v1::EndpointSlice;
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use kube::api::ListParams;
use kube::Api;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use srelens_capability::{Annotations, Capability, CapabilityError};

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;

/// Input arguments for `k8s.serviceHealthProbe`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ServiceHealthProbeIn {
    pub context: String,
    pub namespace: String,
    pub service: String,
}

/// Output evidence for `k8s.serviceHealthProbe`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ServiceHealthProbeOut {
    pub service: String,
    pub namespace: String,
    pub service_type: String,
    pub cluster_ip: Option<String>,
    pub selector: Vec<LabelPair>,
    pub port_mappings: Vec<ServicePortMapping>,
    pub endpoints_summary: EndpointReadinessSummary,
    pub port_mismatches: Vec<PortMismatchEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LabelPair {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ServicePortMapping {
    pub name: Option<String>,
    pub port: i32,
    pub target_port: String,
    pub protocol: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EndpointReadinessSummary {
    pub total_endpoints: usize,
    pub ready_endpoints: usize,
    pub not_ready_endpoints: usize,
    pub matching_pods_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PortMismatchEvidence {
    pub port_name: Option<String>,
    pub service_port: i32,
    pub target_port: String,
    pub container_ports_found: Vec<i32>,
    pub mismatch_kind: String, // "TargetPortNotFound" | "NamedPortNotFound" | "NoMatchingPods" | "ZeroReadyEndpoints"
}

/// Pure evaluation function comparing Service ports and selector against EndpointSlices and candidate Pods.
pub fn analyze_service_health(
    namespace: &str,
    service_name: &str,
    svc: &Service,
    slices: &[EndpointSlice],
    pods: &[Pod],
) -> ServiceHealthProbeOut {
    let spec = svc.spec.as_ref();
    let service_type = spec
        .and_then(|s| s.type_.clone())
        .unwrap_or_else(|| "ClusterIP".to_string());
    let cluster_ip = spec.and_then(|s| s.cluster_ip.clone());

    let mut selector = Vec::new();
    if let Some(sel) = spec.and_then(|s| s.selector.as_ref()) {
        for (k, v) in sel {
            selector.push(LabelPair {
                key: k.clone(),
                value: v.clone(),
            });
        }
    }

    let mut port_mappings = Vec::new();
    if let Some(ports) = spec.and_then(|s| s.ports.as_ref()) {
        for p in ports {
            let target_port_str = match &p.target_port {
                Some(IntOrString::Int(val)) => val.to_string(),
                Some(IntOrString::String(val)) => val.clone(),
                None => p.port.to_string(),
            };
            port_mappings.push(ServicePortMapping {
                name: p.name.clone(),
                port: p.port,
                target_port: target_port_str,
                protocol: p.protocol.clone().unwrap_or_else(|| "TCP".to_string()),
            });
        }
    }

    // Tally EndpointSlice endpoints
    let mut total_endpoints = 0;
    let mut ready_endpoints = 0;
    let mut not_ready_endpoints = 0;

    for slice in slices {
        for ep in &slice.endpoints {
            total_endpoints += 1;
            let is_ready = ep.conditions.as_ref().and_then(|c| c.ready).unwrap_or(true); // Default in Kubernetes EndpointSlice is ready if unset
            if is_ready {
                ready_endpoints += 1;
            } else {
                not_ready_endpoints += 1;
            }
        }
    }

    // Inspect Pods: extract declared container ports and names (including native sidecars)
    let mut container_ports_set = BTreeSet::new();
    let mut container_port_names = BTreeSet::new();

    for pod in pods {
        if let Some(pspec) = &pod.spec {
            let all_containers = pspec.containers.iter().chain(
                pspec
                    .init_containers
                    .iter()
                    .flatten()
                    .filter(|c| c.restart_policy.as_deref() == Some("Always")),
            );
            for c in all_containers {
                if let Some(ports) = &c.ports {
                    for cp in ports {
                        container_ports_set.insert(cp.container_port);
                        if let Some(pname) = &cp.name {
                            container_port_names.insert(pname.clone());
                        }
                    }
                }
            }
        }
    }

    let container_ports_found: Vec<i32> = container_ports_set.into_iter().collect();

    // Check for targetPort misalignments
    let mut port_mismatches = Vec::new();

    if let Some(ports) = spec.and_then(|s| s.ports.as_ref()) {
        for p in ports {
            let target = match &p.target_port {
                Some(IntOrString::Int(val)) => IntOrString::Int(*val),
                Some(IntOrString::String(val)) => IntOrString::String(val.clone()),
                None => IntOrString::Int(p.port),
            };

            match target {
                IntOrString::Int(num) => {
                    if !pods.is_empty()
                        && !container_ports_found.is_empty()
                        && !container_ports_found.contains(&num)
                    {
                        port_mismatches.push(PortMismatchEvidence {
                            port_name: p.name.clone(),
                            service_port: p.port,
                            target_port: num.to_string(),
                            container_ports_found: container_ports_found.clone(),
                            mismatch_kind: "TargetPortNotFound".to_string(),
                        });
                    }
                }
                IntOrString::String(name) => {
                    if !pods.is_empty() && !container_port_names.contains(&name) {
                        port_mismatches.push(PortMismatchEvidence {
                            port_name: p.name.clone(),
                            service_port: p.port,
                            target_port: name,
                            container_ports_found: container_ports_found.clone(),
                            mismatch_kind: "NamedPortNotFound".to_string(),
                        });
                    }
                }
            }
        }
    }

    if !selector.is_empty() && pods.is_empty() {
        port_mismatches.push(PortMismatchEvidence {
            port_name: None,
            service_port: 0,
            target_port: String::new(),
            container_ports_found: Vec::new(),
            mismatch_kind: "NoMatchingPods".to_string(),
        });
    } else if !pods.is_empty() && ready_endpoints == 0 {
        port_mismatches.push(PortMismatchEvidence {
            port_name: None,
            service_port: 0,
            target_port: String::new(),
            container_ports_found: container_ports_found.clone(),
            mismatch_kind: "ZeroReadyEndpoints".to_string(),
        });
    }

    ServiceHealthProbeOut {
        service: service_name.to_string(),
        namespace: namespace.to_string(),
        service_type,
        cluster_ip,
        selector,
        port_mappings,
        endpoints_summary: EndpointReadinessSummary {
            total_endpoints,
            ready_endpoints,
            not_ready_endpoints,
            matching_pods_count: pods.len(),
        },
        port_mismatches,
    }
}

/// `k8s.serviceHealthProbe` capability definition.
pub fn service_health_probe_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ServiceHealthProbeIn, ServiceHealthProbeOut, _, _>(
        "k8s.serviceHealthProbe",
        "evaluates service port mapping, targetPort alignment with containerPorts, and EndpointSlice readiness",
        Annotations::READ_ONLY,
        move |input: ServiceHealthProbeIn| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;

                let svc_api: Api<Service> = Api::namespaced(client.clone(), &input.namespace);
                let svc = tokio::time::timeout(request_timeout(), svc_api.get(&input.service))
                    .await
                    .map_err(|_| CapabilityError::Handler("fetch service timed out".into()))?
                    .map_err(|e| match e {
                        kube::Error::Api(ref ae) if ae.code == 404 => CapabilityError::Handler(format!(
                            "Service {}/{} not found",
                            input.namespace, input.service
                        )),
                        other => CapabilityError::Handler(format!(
                            "Failed to fetch Service {}/{}: {}",
                            input.namespace, input.service, other
                        )),
                    })?;

                // Fetch EndpointSlices for this service using bounded list
                let slice_api: Api<EndpointSlice> = Api::namespaced(client.clone(), &input.namespace);
                let slice_params = ListParams::default()
                    .labels(&format!("kubernetes.io/service-name={}", input.service));
                let (slices, slices_trunc) = crate::list_cap::list_capped(&slice_api, slice_params)
                    .await
                    .map_err(|e| e.into_capability_error("list endpointslices"))?;
                if slices_trunc {
                    return Err(CapabilityError::Handler(format!(
                        "EndpointSlices for service '{}/{}' were truncated by list limit",
                        input.namespace, input.service
                    )));
                }

                // If service defines a selector, query matching pods using bounded list
                let pods = if let Some(sel) = svc.spec.as_ref().and_then(|s| s.selector.as_ref()) {
                    if sel.is_empty() {
                        Vec::new()
                    } else {
                        let selector_str = sel
                            .iter()
                            .map(|(k, v)| format!("{}={}", k, v))
                            .collect::<Vec<_>>()
                            .join(",");
                        let pod_api: Api<Pod> = Api::namespaced(client.clone(), &input.namespace);
                        let pod_params = ListParams::default().labels(&selector_str);
                        let (pod_items, pod_trunc) = crate::list_cap::list_capped(&pod_api, pod_params)
                            .await
                            .map_err(|e| e.into_capability_error("list matching pods"))?;
                        if pod_trunc {
                            return Err(CapabilityError::Handler(format!(
                                "Matching pods for service '{}/{}' were truncated by list limit",
                                input.namespace, input.service
                            )));
                        }
                        pod_items
                    }
                } else {
                    Vec::new()
                };

                Ok(analyze_service_health(
                    &input.namespace,
                    &input.service,
                    &svc,
                    &slices,
                    &pods,
                ))
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::{Container, ContainerPort, PodSpec, ServicePort, ServiceSpec};
    use k8s_openapi::api::discovery::v1::{Endpoint, EndpointConditions, EndpointPort};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    #[test]
    fn capability_has_expected_id() {
        let cap = service_health_probe_capability(ClientCache::new(PathBuf::from("/x")));
        assert_eq!(cap.id, "k8s.serviceHealthProbe");
    }

    #[test]
    fn deserializes_caller_payload_and_denies_unknown_fields() {
        let valid = serde_json::json!({
            "context": "kind-srelens",
            "namespace": "default",
            "service": "api-gateway"
        });
        let parsed: Result<ServiceHealthProbeIn, _> = serde_json::from_value(valid);
        assert!(parsed.is_ok());
        let input = parsed.unwrap();
        assert_eq!(input.context, "kind-srelens");
        assert_eq!(input.namespace, "default");
        assert_eq!(input.service, "api-gateway");

        // Missing service name
        let missing = serde_json::json!({
            "context": "kind-srelens",
            "namespace": "default"
        });
        assert!(serde_json::from_value::<ServiceHealthProbeIn>(missing).is_err());

        // Extra unexpected field rejected by deny_unknown_fields
        let extra = serde_json::json!({
            "context": "kind-srelens",
            "namespace": "default",
            "service": "api-gateway",
            "extra": 123
        });
        assert!(serde_json::from_value::<ServiceHealthProbeIn>(extra).is_err());
    }

    #[test]
    fn detects_target_port_mismatch_against_pod_containers() {
        let mut selector = BTreeMap::new();
        selector.insert("app".to_string(), "api".to_string());

        let svc = Service {
            spec: Some(ServiceSpec {
                type_: Some("ClusterIP".to_string()),
                cluster_ip: Some("10.96.0.100".to_string()),
                selector: Some(selector),
                ports: Some(vec![ServicePort {
                    name: Some("http".to_string()),
                    port: 80,
                    target_port: Some(IntOrString::Int(8080)), // Service sends to 8080
                    protocol: Some("TCP".to_string()),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        };

        // Pod only listens on containerPort 3000!
        let pod = Pod {
            spec: Some(PodSpec {
                containers: vec![Container {
                    name: "web".to_string(),
                    ports: Some(vec![ContainerPort {
                        container_port: 3000,
                        name: Some("http-alt".to_string()),
                        ..Default::default()
                    }]),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };

        let slices = vec![EndpointSlice {
            endpoints: vec![Endpoint {
                conditions: Some(EndpointConditions {
                    ready: Some(true),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ports: Some(vec![EndpointPort {
                port: Some(8080),
                ..Default::default()
            }]),
            ..Default::default()
        }];

        let report = analyze_service_health("default", "api", &svc, &slices, &[pod]);

        assert_eq!(report.service, "api");
        assert_eq!(report.endpoints_summary.ready_endpoints, 1);
        assert_eq!(report.port_mismatches.len(), 1);
        assert_eq!(
            report.port_mismatches[0].mismatch_kind,
            "TargetPortNotFound"
        );
        assert_eq!(report.port_mismatches[0].target_port, "8080");
        assert_eq!(report.port_mismatches[0].container_ports_found, vec![3000]);
    }

    #[test]
    fn reports_zero_endpoints_and_missing_pods() {
        let mut selector = BTreeMap::new();
        selector.insert("app".to_string(), "ghost".to_string());

        let svc = Service {
            spec: Some(ServiceSpec {
                selector: Some(selector),
                ports: Some(vec![ServicePort {
                    port: 80,
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        };

        let report = analyze_service_health("default", "ghost-service", &svc, &[], &[]);
        assert_eq!(report.endpoints_summary.total_endpoints, 0);
        assert_eq!(report.endpoints_summary.matching_pods_count, 0);
        assert!(report
            .port_mismatches
            .iter()
            .any(|m| m.mismatch_kind == "NoMatchingPods"));
    }

    #[test]
    fn detects_named_port_not_found_when_no_ports_are_named() {
        let mut selector = BTreeMap::new();
        selector.insert("app".to_string(), "api".to_string());

        let svc = Service {
            spec: Some(ServiceSpec {
                selector: Some(selector),
                ports: Some(vec![ServicePort {
                    port: 80,
                    target_port: Some(IntOrString::String("http".to_string())),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        };

        // Pod has containerPort 8080, but it is NOT named "http"
        let pod = Pod {
            spec: Some(PodSpec {
                containers: vec![Container {
                    name: "web".to_string(),
                    ports: Some(vec![ContainerPort {
                        container_port: 8080,
                        name: None,
                        ..Default::default()
                    }]),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };

        let report = analyze_service_health("default", "api", &svc, &[], &[pod]);
        assert!(report
            .port_mismatches
            .iter()
            .any(|m| m.mismatch_kind == "NamedPortNotFound" && m.target_port == "http"));
    }

    #[test]
    fn recognizes_native_sidecar_ports() {
        let mut selector = BTreeMap::new();
        selector.insert("app".to_string(), "mesh".to_string());

        let svc = Service {
            spec: Some(ServiceSpec {
                selector: Some(selector),
                ports: Some(vec![ServicePort {
                    port: 15021,
                    target_port: Some(IntOrString::Int(15021)),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        };

        // 15021 is declared on a restartable init sidecar (restartPolicy: Always)
        let pod = Pod {
            spec: Some(PodSpec {
                init_containers: Some(vec![Container {
                    name: "istio-proxy".to_string(),
                    restart_policy: Some("Always".to_string()),
                    ports: Some(vec![ContainerPort {
                        container_port: 15021,
                        name: Some("status-port".to_string()),
                        ..Default::default()
                    }]),
                    ..Default::default()
                }]),
                containers: vec![Container {
                    name: "app".to_string(),
                    ports: Some(vec![ContainerPort {
                        container_port: 8080,
                        ..Default::default()
                    }]),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };

        let report = analyze_service_health("default", "mesh", &svc, &[], &[pod]);
        // TargetPortNotFound should NOT be reported because 15021 is on the sidecar!
        assert!(!report
            .port_mismatches
            .iter()
            .any(|m| m.mismatch_kind == "TargetPortNotFound"));
    }
}
