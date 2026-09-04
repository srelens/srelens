//! The `k8s.queryPodEndpoint` capability — queries an HTTP endpoint (e.g. /metrics,
//! /healthz) on any pod via an on-demand Kubernetes API port-forward tunnel.

use std::sync::Arc;
use std::time::Duration;

use k8s_openapi::api::core::v1::Pod;
use kube::api::ListParams;
use kube::Api;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use srelens_capability::{Annotations, Capability, CapabilityError};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::client_cache::ClientCache;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(8);
const DEFAULT_MAX_LINES: usize = 200;

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct QueryPodEndpointIn {
    /// Kubernetes context name
    pub context: String,
    /// Kubernetes namespace
    pub namespace: String,
    /// Target pod name. If omitted, auto-discovers a matching running pod in `namespace`.
    #[serde(default)]
    pub pod: Option<String>,
    /// Optional label selector (e.g. "app=nvidia-dcgm-exporter" or "app.kubernetes.io/name=dcgm-exporter") to locate a pod.
    #[serde(default)]
    pub selector: Option<String>,
    /// Container port to query. If omitted, auto-detects from container ports (looking for ports named 'metrics', 'http', 9400, 9100, 8080, etc.).
    #[serde(default)]
    pub port: Option<u16>,
    /// HTTP path to GET (default "/metrics"). Can also be "/healthz", "/readyz", etc.
    #[serde(default)]
    pub path: Option<String>,
    /// Optional case-insensitive substring or metric name to filter response lines (e.g. "DCGM_FI_DEV_FB" or "memory").
    #[serde(default)]
    pub filter: Option<String>,
    /// Maximum number of lines to return (default 200).
    #[serde(default)]
    pub max_lines: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct QueryPodEndpointOut {
    pub pod: String,
    pub port: u16,
    pub path: String,
    pub status_code: u16,
    pub total_lines: usize,
    pub returned_lines: usize,
    pub metrics: Vec<String>,
    pub summary: String,
}

/// Auto-detects the most likely metrics/service port from a Pod's specification.
pub fn detect_target_port(pod: &Pod, requested_port: Option<u16>) -> Option<u16> {
    if let Some(p) = requested_port {
        return Some(p);
    }

    let containers = pod.spec.as_ref().map(|s| s.containers.as_slice()).unwrap_or_default();

    // 1. Look for ports with explicit names matching metrics/prom/http
    for c in containers {
        if let Some(ports) = &c.ports {
            for p in ports {
                if let Some(name) = &p.name {
                    let lname = name.to_lowercase();
                    if lname.contains("metric") || lname.contains("prom") || lname == "http-metrics" {
                        return Some(p.container_port as u16);
                    }
                }
            }
        }
    }

    // 2. Look for well-known metric and monitoring port numbers
    let standard_ports = [9400, 9100, 9090, 8080, 8000, 80, 443];
    for c in containers {
        if let Some(ports) = &c.ports {
            for p in ports {
                let cp = p.container_port as u16;
                if standard_ports.contains(&cp) {
                    return Some(cp);
                }
            }
        }
    }

    // 3. Fall back to the first container port declared
    for c in containers {
        if let Some(ports) = &c.ports {
            if let Some(first) = ports.first() {
                return Some(first.container_port as u16);
            }
        }
    }

    // 4. Heuristic based on pod name if no container ports were declared in manifest
    let pod_name = pod.metadata.name.as_deref().unwrap_or_default().to_lowercase();
    if pod_name.contains("dcgm") || pod_name.contains("gpu") {
        Some(9400)
    } else if pod_name.contains("node-exporter") {
        Some(9100)
    } else {
        Some(8080)
    }
}

/// Helper to parse a raw HTTP/1.1 response stream into status code and body string.
pub fn parse_raw_http_response(bytes: &[u8]) -> Result<(u16, String), String> {
    let header_sep = bytes
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|pos| (pos, 4))
        .or_else(|| bytes.windows(2).position(|w| w == b"\n\n").map(|pos| (pos, 2)))
        .ok_or_else(|| "Invalid HTTP response: header delimiter not found".to_string())?;

    let (header_bytes, body_bytes) = (&bytes[..header_sep.0], &bytes[header_sep.0 + header_sep.1..]);

    let header_str = String::from_utf8_lossy(header_bytes);
    let first_line = header_str.lines().next().unwrap_or("");
    let status_code = first_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| format!("Invalid HTTP status line: '{}'", first_line))?;

    // Decode body: if chunked, unchunk
    let is_chunked = header_str.to_lowercase().contains("transfer-encoding: chunked");
    let body_str = if is_chunked {
        decode_chunked_body(body_bytes)
    } else {
        String::from_utf8_lossy(body_bytes).to_string()
    };

    Ok((status_code, body_str))
}

/// Simple chunked transfer decoding
fn decode_chunked_body(mut bytes: &[u8]) -> String {
    let mut out = Vec::new();
    while !bytes.is_empty() {
        if let Some(pos) = bytes.windows(2).position(|w| w == b"\r\n") {
            let size_str = String::from_utf8_lossy(&bytes[..pos]).trim().to_string();
            let size = match usize::from_str_radix(&size_str, 16) {
                Ok(s) => s,
                Err(_) => break,
            };
            if size == 0 {
                break;
            }
            let data_start = pos + 2;
            let data_end = data_start + size;
            if bytes.len() >= data_end {
                out.extend_from_slice(&bytes[data_start..data_end]);
                bytes = &bytes[data_end.min(bytes.len())..];
                if bytes.starts_with(b"\r\n") {
                    bytes = &bytes[2..];
                }
            } else {
                out.extend_from_slice(&bytes[data_start..]);
                break;
            }
        } else {
            out.extend_from_slice(bytes);
            break;
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

/// Filter lines based on pattern and max limit
pub fn filter_response_lines(
    body: &str,
    filter: Option<&str>,
    max_lines: usize,
) -> (usize, usize, Vec<String>) {
    let all_lines: Vec<&str> = body
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();

    let filtered: Vec<&str> = if let Some(pat) = filter {
        let lpat = pat.to_lowercase();
        all_lines
            .into_iter()
            .filter(|l| l.to_lowercase().contains(&lpat))
            .collect()
    } else {
        all_lines
    };

    let total = filtered.len();
    let capped: Vec<String> = filtered
        .into_iter()
        .take(max_lines)
        .map(|s| s.to_string())
        .collect();

    let returned = capped.len();
    (total, returned, capped)
}

/// Builds the `k8s.queryPodEndpoint` capability.
pub fn query_pod_endpoint_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<QueryPodEndpointIn, QueryPodEndpointOut, _, _>(
        "k8s.queryPodEndpoint",
        "Query an HTTP endpoint (such as /metrics, /healthz, or custom app endpoints) inside any running pod via an on-demand API port-forward tunnel. Automatically resolves pod by selector or name, auto-detects metric/service ports, and supports line filtering.",
        Annotations::READ_ONLY,
        move |input: QueryPodEndpointIn| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let api: Api<Pod> = Api::namespaced(client, &input.namespace);

                // 1. Resolve Target Pod
                let target_pod_obj: Pod = if let Some(ref pod_name) = input.pod {
                    api.get(pod_name)
                        .await
                        .map_err(|e| CapabilityError::Handler(format!("Pod '{}' not found in namespace '{}': {}", pod_name, input.namespace, e)))?
                } else {
                    let lp = if let Some(ref sel) = input.selector {
                        ListParams::default().labels(sel)
                    } else {
                        ListParams::default()
                    };

                    let list = api
                        .list(&lp)
                        .await
                        .map_err(|e| CapabilityError::Handler(format!("Failed to list pods in '{}': {}", input.namespace, e)))?;

                    let running_pods: Vec<Pod> = list
                        .items
                        .into_iter()
                        .filter(|p| {
                            p.status.as_ref().and_then(|s| s.phase.as_deref()) == Some("Running")
                        })
                        .collect();

                    if running_pods.is_empty() {
                        return Err(CapabilityError::Handler(format!(
                            "No running pods found in namespace '{}'{}",
                            input.namespace,
                            input.selector.as_ref().map(|s| format!(" matching selector '{}'", s)).unwrap_or_default()
                        )));
                    }

                    // Prefer a pod whose name or container name mentions dcgm, exporter, or metric
                    let preferred = running_pods.iter().find(|p| {
                        let name = p.metadata.name.as_deref().unwrap_or_default().to_lowercase();
                        name.contains("dcgm") || name.contains("exporter") || name.contains("metric")
                    }).cloned().unwrap_or_else(|| running_pods[0].clone());

                    preferred
                };

                let pod_name = target_pod_obj.metadata.name.clone().unwrap_or_default();
                let port = detect_target_port(&target_pod_obj, input.port).ok_or_else(|| {
                    CapabilityError::Handler(format!("Unable to determine target port for pod '{}'", pod_name))
                })?;

                let path = input.path.as_deref().unwrap_or("/metrics");
                let clean_path = if path.starts_with('/') { path.to_string() } else { format!("/{}", path) };

                // 2. Open port-forward tunnel via Kubernetes API server
                let pf_future = async {
                    let mut pf = api
                        .portforward(&pod_name, &[port])
                        .await
                        .map_err(|e| format!("Failed to initiate port-forward to {}:{}: {}", pod_name, port, e))?;

                    let mut stream = pf
                        .take_stream(port)
                        .ok_or_else(|| format!("Failed to take port-forward stream for port {}", port))?;

                    // Format HTTP/1.1 request
                    let req = format!(
                        "GET {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nUser-Agent: srelens\r\nAccept: */*\r\nConnection: close\r\n\r\n",
                        clean_path, port
                    );

                    stream
                        .write_all(req.as_bytes())
                        .await
                        .map_err(|e| format!("Failed to send HTTP request over tunnel: {}", e))?;

                    let mut response_bytes = Vec::new();
                    stream
                        .read_to_end(&mut response_bytes)
                        .await
                        .map_err(|e| format!("Failed to read HTTP response from tunnel: {}", e))?;

                    parse_raw_http_response(&response_bytes)
                };

                let (status_code, raw_body) = tokio::time::timeout(DEFAULT_TIMEOUT, pf_future)
                    .await
                    .map_err(|_| CapabilityError::Handler(format!("Timeout querying {}:{}{} after {}s", pod_name, port, clean_path, DEFAULT_TIMEOUT.as_secs())))?
                    .map_err(CapabilityError::Handler)?;

                // 3. Filter lines and enforce cap
                let max_lines = input.max_lines.unwrap_or(DEFAULT_MAX_LINES);
                let (total_lines, returned_lines, metrics) = filter_response_lines(&raw_body, input.filter.as_deref(), max_lines);

                let summary = if let Some(ref pat) = input.filter {
                    format!("Queried {}:{}{} [HTTP {}] - filtered by '{}': {} matching lines (returned {})", pod_name, port, clean_path, status_code, pat, total_lines, returned_lines)
                } else {
                    format!("Queried {}:{}{} [HTTP {}] - total {} lines (returned {})", pod_name, port, clean_path, status_code, total_lines, returned_lines)
                };

                Ok(QueryPodEndpointOut {
                    pod: pod_name,
                    port,
                    path: clean_path,
                    status_code,
                    total_lines,
                    returned_lines,
                    metrics,
                    summary,
                })
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::{Container, ContainerPort, PodSpec};
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

    #[test]
    fn test_detect_target_port_priority() {
        // 1. Pod with named metrics port
        let pod = Pod {
            metadata: ObjectMeta { name: Some("my-app-pod".to_string()), ..Default::default() },
            spec: Some(PodSpec {
                containers: vec![Container {
                    name: "app".to_string(),
                    ports: Some(vec![
                        ContainerPort { container_port: 8080, name: Some("http".to_string()), ..Default::default() },
                        ContainerPort { container_port: 9090, name: Some("metrics".to_string()), ..Default::default() },
                    ]),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(detect_target_port(&pod, None), Some(9090));

        // 2. Pod with DCGM exporter well-known port 9400
        let dcgm_pod = Pod {
            metadata: ObjectMeta { name: Some("nvidia-dcgm-exporter-xyz".to_string()), ..Default::default() },
            spec: Some(PodSpec {
                containers: vec![Container {
                    name: "dcgm".to_string(),
                    ports: Some(vec![
                        ContainerPort { container_port: 9400, ..Default::default() },
                    ]),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(detect_target_port(&dcgm_pod, None), Some(9400));

        // 3. User requested port override
        assert_eq!(detect_target_port(&dcgm_pod, Some(9999)), Some(9999));
    }

    #[test]
    fn test_parse_raw_http_response_and_filter() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\n# HELP dcgm_fb_used Framebuffer used\nDCGM_FI_DEV_FB_USED{gpu=\"0\"} 1024\nDCGM_FI_DEV_FB_FREE{gpu=\"0\"} 7168\nnode_cpu_seconds_total 500\n";
        let (status, body) = parse_raw_http_response(raw).unwrap();
        assert_eq!(status, 200);

        let (total, returned, lines) = filter_response_lines(&body, Some("DCGM_FI_DEV_FB"), 50);
        assert_eq!(total, 2);
        assert_eq!(returned, 2);
        assert_eq!(lines[0], "DCGM_FI_DEV_FB_USED{gpu=\"0\"} 1024");
        assert_eq!(lines[1], "DCGM_FI_DEV_FB_FREE{gpu=\"0\"} 7168");
    }

    #[test]
    fn test_parse_raw_http_response_errors_on_malformed_input() {
        // Missing delimiter
        let malformed = b"Some raw output without headers";
        assert!(parse_raw_http_response(malformed).is_err());

        // Missing valid HTTP status code
        let bad_status = b"NOT_HTTP\r\nHeader: value\r\n\r\nBody";
        assert!(parse_raw_http_response(bad_status).is_err());
    }
}
