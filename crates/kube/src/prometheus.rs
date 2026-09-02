//! `k8s.prometheusDiscover` and `k8s.prometheusQuery` — reading a metrics
//! backend the cluster already runs, without installing anything.
//!
//! ## Why this exists
//!
//! [`crate::topology`] can say what a workload was BUILT to talk to, from its
//! configuration. It cannot say what actually happened: no request rate, no
//! error rate, no latency. Those live in a time series, and the only ones a
//! cluster is likely to already have are a Prometheus and whatever is feeding
//! it — Istio's `istio_requests_total`, Linkerd's `response_total`, Cilium
//! Hubble's flows, or an application's own RED metrics. One query path reaches
//! all of them, which is why this is a general capability and not a topology
//! detail: the Control Room's tiles need exactly the same thing.
//!
//! Nothing here installs a mesh or an agent. If the cluster has no Prometheus,
//! discovery returns nothing and every caller carries on without observed data.
//!
//! ## Reached through the API server, not a port-forward
//!
//! A Prometheus Service is a ClusterIP: srelens cannot dial it from the
//! machine. The obvious answer is a port-forward, and it is the wrong one — it
//! is a long-lived tunnel, a local port to allocate and free, and a lifetime to
//! manage for what should be one request.
//!
//! The API server already proxies to Services:
//!
//! ```text
//! /api/v1/namespaces/<ns>/services/<name>:<port>/proxy/api/v1/query?query=...
//! ```
//!
//! That runs over the connection and the credentials srelens already has, so it
//! works wherever `kubectl` works — including through a bastion, an OIDC login
//! or an exec plugin — and needs no new permission beyond `services/proxy`.
//! Nothing is left running afterwards.
//!
//! ## Discovery is deliberately suspicious
//!
//! Half the Services with `prometheus` in the name do not serve the query API:
//! `prometheus-operator` is a controller, `prometheus-node-exporter` and
//! `prometheus-kube-state-metrics` are exporters that are scraped BY
//! Prometheus, `alertmanager` and `pushgateway` are neither. Querying one of
//! those returns a 404 that looks like an outage rather than a mistake, so the
//! exclusions are as much of this module as the matches.

use std::collections::BTreeMap;
use std::sync::Arc;

use http::Request;
use k8s_openapi::api::core::v1::Service;
use kube::api::ListParams;
use kube::Api;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use srelens_capability::{Annotations, Capability, CapabilityError};

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;

/// What is answering, when it can be told apart.
///
/// Only ever a hint for the caller's choice of query — every one of these
/// serves the Prometheus query API, which is the whole reason one path reaches
/// them all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Flavour {
    Prometheus,
    Thanos,
    Mimir,
    VictoriaMetrics,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PrometheusCandidate {
    pub namespace: String,
    pub name: String,
    pub port: i32,
    pub flavour: Flavour,
}

/// Service names that contain a match word but do not serve the query API.
///
/// Every one of these is a real thing found beside a Prometheus: two exporters
/// it scrapes, the controller that deploys it, and two parts of the alerting
/// path. Pointing a query at any of them fails in a way that reads like the
/// cluster is broken rather than like the wrong address.
const NOT_A_QUERY_API: &[&str] = &[
    "operator",
    "node-exporter",
    "kube-state-metrics",
    "alertmanager",
    "pushgateway",
    "adapter",
    "config-reloader",
    "blackbox",
    "snmp",
];

/// Ports a query API is served on, best first.
const QUERY_PORTS: &[i32] = &[9090, 10902, 8481, 8428, 9009];

fn flavour_of(name: &str) -> Option<Flavour> {
    if name.contains("thanos") {
        // Only the query layer answers PromQL; a sidecar, compactor or store
        // gateway does not.
        return name.contains("query").then_some(Flavour::Thanos);
    }
    if name.contains("victoria") || name.starts_with("vmselect") || name.starts_with("vmsingle") {
        return Some(Flavour::VictoriaMetrics);
    }
    if name.contains("mimir") {
        return name.contains("query").then_some(Flavour::Mimir);
    }
    name.contains("prometheus").then_some(Flavour::Prometheus)
}

/// The port to ask on: a named one first, since a chart that renames its port
/// is likelier to have moved it than to have kept 9090.
fn query_port(service: &Service) -> Option<i32> {
    let ports = service.spec.as_ref()?.ports.as_ref()?;
    let named = ports.iter().find(|p| {
        p.name
            .as_deref()
            .is_some_and(|n| n == "web" || n == "http-web" || n == "http" || n == "query")
    });
    if let Some(port) = named {
        return Some(port.port);
    }
    ports
        .iter()
        .find(|p| QUERY_PORTS.contains(&p.port))
        .map(|p| p.port)
}

/// Every Service that looks like it serves the Prometheus query API.
///
/// Matched on the well-known label first — a chart that sets
/// `app.kubernetes.io/name` is telling us outright — and on the name only as a
/// fallback, always minus [`NOT_A_QUERY_API`].
pub fn candidates(services: &[Service]) -> Vec<PrometheusCandidate> {
    let mut out: Vec<PrometheusCandidate> = Vec::new();
    for service in services {
        let name = service.metadata.name.clone().unwrap_or_default();
        let labelled = service
            .metadata
            .labels
            .as_ref()
            .and_then(|l| l.get("app.kubernetes.io/name"))
            .cloned()
            .unwrap_or_default();
        // The label names the component; the Service name is the release plus
        // the component, so both are tested against the same exclusions.
        let subject = if labelled.is_empty() { name.clone() } else { labelled };
        if NOT_A_QUERY_API.iter().any(|bad| name.contains(bad) || subject.contains(bad)) {
            continue;
        }
        let Some(flavour) = flavour_of(&subject).or_else(|| flavour_of(&name)) else {
            continue;
        };
        let Some(port) = query_port(service) else {
            continue;
        };
        out.push(PrometheusCandidate {
            namespace: service.metadata.namespace.clone().unwrap_or_default(),
            name,
            port,
            flavour,
        });
    }
    out.sort_by(|a, b| a.namespace.cmp(&b.namespace).then(a.name.cmp(&b.name)));
    out
}

/// Percent-encode one query-string value.
///
/// Hand-rolled because this crate has no URL dependency and needs exactly this
/// much. Everything outside the unreserved set is escaped — notably `+`, which
/// a decoder reads as a space, and the `{}"=~` a PromQL selector is made of.
pub fn encode_query(value: &str) -> String {
    let mut out = String::with_capacity(value.len() * 2);
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// The API server path that proxies one instant query to a Service.
pub fn instant_query_path(namespace: &str, service: &str, port: i32, query: &str) -> String {
    format!(
        "/api/v1/namespaces/{namespace}/services/{service}:{port}/proxy/api/v1/query?query={}",
        encode_query(query)
    )
}

/// One row of a query result: the labels that identify it, and its value.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct Sample {
    pub labels: BTreeMap<String, String>,
    pub value: f64,
}

/// Read an instant-query response.
///
/// Prometheus reports its own failures in a 200 body (`status: "error"`), so
/// the status field is checked before the data — a caller that only looked at
/// the HTTP code would read a failed query as an empty result, which for a
/// topology means "no traffic" rather than "we could not ask".
///
/// A sample's value arrives as a STRING, and one that will not parse is
/// dropped rather than defaulted: `NaN`, `+Inf` and `-Inf` are all legal there
/// and none of them is a rate worth drawing.
pub fn parse_instant(body: &Value) -> Result<Vec<Sample>, String> {
    match body.get("status").and_then(Value::as_str) {
        Some("success") => {}
        Some("error") => {
            let kind = body.get("errorType").and_then(Value::as_str).unwrap_or("error");
            let detail = body.get("error").and_then(Value::as_str).unwrap_or("no detail");
            return Err(format!("{kind}: {detail}"));
        }
        _ => return Err("not a Prometheus query response".into()),
    }
    let result = body
        .pointer("/data/result")
        .and_then(Value::as_array)
        .ok_or("query response carried no result")?;
    let mut out = Vec::with_capacity(result.len());
    for entry in result {
        let labels = entry
            .get("metric")
            .and_then(Value::as_object)
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        // `[ <unix seconds>, "<value>" ]` — the timestamp is not kept: an
        // instant query is answered as of now, and a caller that wanted a
        // series would have asked for a range.
        let Some(raw) = entry.pointer("/value/1").and_then(Value::as_str) else {
            continue;
        };
        let Ok(value) = raw.parse::<f64>() else {
            continue;
        };
        if !value.is_finite() {
            continue;
        }
        out.push(Sample { labels, value });
    }
    Ok(out)
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DiscoverIn {
    pub context: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DiscoverOut {
    /// Every Service that looks like a query API. Empty is an ordinary answer:
    /// most clusters do not run one, and every caller works without it.
    pub candidates: Vec<PrometheusCandidate>,
}

/// `k8s.prometheusDiscover` — find a metrics backend the cluster already runs.
pub fn prometheus_discover_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<DiscoverIn, DiscoverOut, _, _>(
        "k8s.prometheusDiscover",
        "find a Prometheus-compatible query API running in the cluster",
        Annotations::READ_ONLY,
        move |input: DiscoverIn| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                // Every namespace: a monitoring stack is rarely in the one a
                // reader happens to be looking at.
                let api: Api<Service> = Api::all(client);
                let list = tokio::time::timeout(request_timeout(), api.list(&ListParams::default()))
                    .await
                    .map_err(|_| CapabilityError::Handler("list services timed out".into()))?
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                Ok(DiscoverOut {
                    candidates: candidates(&list.items),
                })
            }
        },
    )
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct QueryIn {
    pub context: String,
    /// Where the query API is, as [`prometheus_discover_capability`] reported
    /// it. Passed rather than rediscovered so a reader can point at one srelens
    /// did not find, and so a screen does not list every Service on every read.
    pub namespace: String,
    pub service: String,
    pub port: i32,
    /// PromQL, sent as an instant query.
    pub query: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct QueryOut {
    pub series: Vec<Sample>,
}

/// One instant query, for a caller that already holds a client.
///
/// The capability below is this plus argument plumbing; [`crate::topology`]
/// calls it directly rather than going back out through the registry, because
/// it is already inside a handler with a client in hand.
pub async fn instant_query(
    client: &kube::Client,
    namespace: &str,
    service: &str,
    port: i32,
    query: &str,
) -> Result<Vec<Sample>, String> {
    let path = instant_query_path(namespace, service, port, query);
    let req = Request::get(&path)
        .body(Vec::new())
        .map_err(|e| e.to_string())?;
    let body: Value = tokio::time::timeout(request_timeout(), client.request(req))
        .await
        .map_err(|_| "prometheus query timed out".to_string())?
        .map_err(|e| e.to_string())?;
    parse_instant(&body)
}

/// `k8s.prometheusQuery` — one PromQL instant query, proxied through the API
/// server.
pub fn prometheus_query_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<QueryIn, QueryOut, _, _>(
        "k8s.prometheusQuery",
        "run a PromQL instant query against an in-cluster Prometheus",
        Annotations::READ_ONLY,
        move |input: QueryIn| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let series = instant_query(
                    &client,
                    &input.namespace,
                    &input.service,
                    input.port,
                    &input.query,
                )
                .await
                .map_err(CapabilityError::Handler)?;
                Ok(QueryOut { series })
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::{ServicePort, ServiceSpec};
    use kube::core::ObjectMeta;
    use serde_json::json;
    use std::path::PathBuf;

    fn service(namespace: &str, name: &str, labels: &[(&str, &str)], ports: &[(&str, i32)]) -> Service {
        Service {
            metadata: ObjectMeta {
                name: Some(name.into()),
                namespace: Some(namespace.into()),
                labels: if labels.is_empty() {
                    None
                } else {
                    Some(labels.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect())
                },
                ..Default::default()
            },
            spec: Some(ServiceSpec {
                ports: Some(
                    ports
                        .iter()
                        .map(|(name, port)| ServicePort {
                            name: (!name.is_empty()).then(|| (*name).to_string()),
                            port: *port,
                            ..Default::default()
                        })
                        .collect(),
                ),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn capabilities_have_expected_ids() {
        let cache = ClientCache::new(PathBuf::from("/x"));
        let discover = prometheus_discover_capability(cache.clone());
        assert_eq!(discover.id, "k8s.prometheusDiscover");
        assert!(discover.annotations.read_only);
        let query = prometheus_query_capability(cache);
        assert_eq!(query.id, "k8s.prometheusQuery");
        assert!(query.annotations.read_only);
    }

    #[test]
    fn finds_a_prometheus_by_its_well_known_label() {
        let found = candidates(&[service(
            "monitoring",
            "kube-prometheus-stack-prometheus",
            &[("app.kubernetes.io/name", "prometheus")],
            &[("http-web", 9090)],
        )]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].namespace, "monitoring");
        assert_eq!(found[0].port, 9090);
        assert_eq!(found[0].flavour, Flavour::Prometheus);
    }

    #[test]
    fn refuses_the_things_that_sit_beside_a_prometheus() {
        // Every one of these is real and none serves the query API. Pointing a
        // query at one fails in a way that reads like an outage rather than
        // like the wrong address.
        let found = candidates(&[
            service("monitoring", "prometheus-operator", &[], &[("http", 8080)]),
            service("monitoring", "prometheus-node-exporter", &[], &[("metrics", 9100)]),
            service("monitoring", "prometheus-kube-state-metrics", &[], &[("http", 8080)]),
            service("monitoring", "alertmanager-operated", &[], &[("web", 9093)]),
            service("monitoring", "prometheus-pushgateway", &[], &[("http", 9091)]),
            service("monitoring", "prometheus-adapter", &[], &[("https", 443)]),
        ]);
        assert_eq!(found, vec![], "none of these answers PromQL");
    }

    #[test]
    fn only_the_query_layer_of_a_thanos_or_mimir_counts() {
        // A sidecar, a compactor and a store gateway all carry the name and
        // none of them answers a query.
        let found = candidates(&[
            service("monitoring", "thanos-query", &[], &[("http", 10902)]),
            service("monitoring", "thanos-store", &[], &[("http", 10902)]),
            service("monitoring", "thanos-compactor", &[], &[("http", 10902)]),
            service("monitoring", "mimir-query-frontend", &[], &[("http", 8080)]),
            service("monitoring", "mimir-ingester", &[], &[("http", 8080)]),
        ]);
        assert_eq!(
            found.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            vec!["mimir-query-frontend", "thanos-query"],
        );
    }

    #[test]
    fn prefers_a_named_port_over_a_well_known_number() {
        // A chart that renamed its port is likelier to have moved it than to
        // have kept 9090 beside it.
        let found = candidates(&[service(
            "monitoring",
            "prometheus",
            &[],
            &[("metrics", 9090), ("web", 9999)],
        )]);
        assert_eq!(found[0].port, 9999);
    }

    #[test]
    fn a_prometheus_on_no_recognisable_port_is_not_offered() {
        // Better nothing than a guess: a wrong port fails as a timeout, which
        // is the least legible failure there is.
        assert_eq!(candidates(&[service("m", "prometheus", &[], &[("", 1234)])]), vec![]);
    }

    #[test]
    fn a_query_is_encoded_so_a_selector_survives_the_round_trip() {
        // `+` is the one that bites: a decoder reads it as a space, so a
        // `[5m]` rate with a `+` anywhere in it comes back as a different
        // query. The braces and quotes of a selector matter just as much.
        let encoded = encode_query("sum(rate(istio_requests_total{code=~\"5..\"}[5m]))");
        assert!(!encoded.contains('+'));
        assert!(!encoded.contains('{'));
        assert!(!encoded.contains('"'));
        assert_eq!(encode_query("a b"), "a%20b");
        assert_eq!(encode_query("up"), "up");
    }

    #[test]
    fn the_path_proxies_through_the_api_server() {
        // Not a port-forward: this runs on the connection and credentials
        // srelens already has, and leaves nothing behind.
        let path = instant_query_path("monitoring", "prometheus", 9090, "up");
        assert_eq!(
            path,
            "/api/v1/namespaces/monitoring/services/prometheus:9090/proxy/api/v1/query?query=up"
        );
    }

    #[test]
    fn reads_labels_and_values_out_of_a_result() {
        let body = json!({
            "status": "success",
            "data": {
                "resultType": "vector",
                "result": [{
                    "metric": { "source_workload": "checkout", "destination_service_name": "payments" },
                    "value": [1710000000.0, "41.2"]
                }]
            }
        });
        let series = parse_instant(&body).unwrap();
        assert_eq!(series.len(), 1);
        assert_eq!(series[0].value, 41.2);
        assert_eq!(series[0].labels["source_workload"], "checkout");
    }

    #[test]
    fn a_failed_query_is_an_error_and_not_an_empty_result() {
        // Prometheus reports its own failures in a 200 body. Reading one as an
        // empty vector would tell a topology screen there is no traffic, which
        // is a confident false statement rather than a missing answer.
        let body = json!({ "status": "error", "errorType": "bad_data", "error": "parse error" });
        let err = parse_instant(&body).unwrap_err();
        assert!(err.contains("bad_data"), "{err}");
        assert!(err.contains("parse error"), "{err}");
        assert!(parse_instant(&json!({ "hello": "world" })).is_err());
    }

    #[test]
    fn values_that_are_not_finite_are_dropped_rather_than_defaulted() {
        // `NaN` and `+Inf` are legal in a result and neither is a rate worth
        // drawing; zero would be a number the cluster never reported.
        let body = json!({
            "status": "success",
            "data": { "result": [
                { "metric": {}, "value": [1.0, "NaN"] },
                { "metric": {}, "value": [1.0, "+Inf"] },
                { "metric": {}, "value": [1.0, "not-a-number"] },
                { "metric": {}, "value": [1.0, "7"] }
            ]}
        });
        let series = parse_instant(&body).unwrap();
        assert_eq!(series.len(), 1);
        assert_eq!(series[0].value, 7.0);
    }
}
