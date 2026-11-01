//! Real spend from Kubecost or OpenCost, when one runs in the cluster.
//!
//! Both expose an allocation API that prices every namespace from the cloud
//! bill, the instance types, spot discounts, storage and network. That is the
//! number the `:cost` estimate can only approximate, so when one is found its
//! figures are shown beside the estimate and the screen says where they came
//! from.
//!
//! Neither is reachable from outside the cluster (they are ClusterIP
//! Services), and srelens does not port-forward for a read. The API server
//! already proxies to Services, the same way [`crate::prometheus`] reaches a
//! Prometheus, so a request goes through the user's own kubeconfig and RBAC:
//!
//! - Kubecost: its frontend Service (`kubecost-cost-analyzer`, port 9090)
//!   serves `/model/allocation`.
//! - OpenCost: its Service (`opencost`, port 9003) serves `/allocation/compute`.
//!
//! The response is read defensively. These are third-party APIs whose field
//! sets grow between versions, so a missing field is zero, not an error, and
//! an unexpected shape is reported as what it was.

use std::collections::BTreeMap;

use http::Request;
use k8s_openapi::api::core::v1::Service;
use kube::api::ListParams;
use kube::Api;
use serde_json::Value;

use crate::connect::request_timeout;

/// The look-back window asked for.
pub const WINDOW: &str = "7d";
const WINDOW_DAYS: f64 = 7.0;
/// Days in the average month the monthly figures are scaled to.
const DAYS_PER_MONTH: f64 = 730.0 / 24.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Kubecost,
    OpenCost,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Kubecost => "Kubecost",
            Source::OpenCost => "OpenCost",
        }
    }

    /// The path, under the Service, that serves the allocation API.
    fn path(self) -> &'static str {
        match self {
            Source::Kubecost => "/model/allocation",
            Source::OpenCost => "/allocation/compute",
        }
    }
}

/// A Service that looks like it serves one of the two APIs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub namespace: String,
    pub service: String,
    pub port: i32,
    pub source: Source,
}

/// Parts of a Kubecost install that are not the cost API.
const NOT_THE_API: &[&str] = &[
    "prometheus",
    "grafana",
    "kube-state-metrics",
    "node-exporter",
    "alertmanager",
    "forecasting",
    "network-costs",
    "cloud-cost",
    "aggregator",
];

fn has_port(service: &Service, wanted: i32) -> bool {
    service
        .spec
        .as_ref()
        .and_then(|s| s.ports.as_ref())
        .is_some_and(|ports| ports.iter().any(|p| p.port == wanted))
}

/// Every Service that looks like Kubecost's or OpenCost's cost API, Kubecost
/// first (it is the one with the richer data).
pub fn candidates(services: &[Service]) -> Vec<Candidate> {
    let mut out = Vec::new();
    for service in services {
        let name = service.metadata.name.clone().unwrap_or_default();
        let namespace = service.metadata.namespace.clone().unwrap_or_default();
        let labels = service.metadata.labels.clone().unwrap_or_default();
        let component = labels
            .get("app.kubernetes.io/name")
            .or_else(|| labels.get("app"))
            .cloned()
            .unwrap_or_default();
        if NOT_THE_API.iter().any(|bad| name.contains(bad) || component.contains(bad)) {
            continue;
        }
        let is_kubecost = component == "cost-analyzer" || name.ends_with("cost-analyzer");
        let is_opencost = component == "opencost" || name == "opencost";
        if is_kubecost && has_port(service, 9090) {
            out.push(Candidate { namespace, service: name, port: 9090, source: Source::Kubecost });
        } else if (is_opencost || is_kubecost) && has_port(service, 9003) {
            out.push(Candidate { namespace, service: name, port: 9003, source: Source::OpenCost });
        }
    }
    out.sort_by(|a, b| {
        (a.source != Source::Kubecost, &a.namespace, &a.service)
            .cmp(&(b.source != Source::Kubecost, &b.namespace, &b.service))
    });
    out
}

/// The API server path that proxies the allocation query to a Service.
pub fn allocation_path(c: &Candidate) -> String {
    format!(
        "/api/v1/namespaces/{}/services/{}:{}/proxy{}?window={WINDOW}&aggregate=namespace&accumulate=true&includeIdle=true",
        c.namespace,
        c.service,
        c.port,
        c.source.path()
    )
}

/// One namespace's real cost over the window.
#[derive(Debug, Clone, PartialEq)]
pub struct NamespaceSpend {
    pub name: String,
    pub total: f64,
    pub cpu: f64,
    pub ram: f64,
    pub storage: f64,
    pub network: f64,
    /// Share of what was allocated that was actually used, 0-1, when reported.
    pub efficiency: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderCosts {
    pub source: Source,
    /// Where it was read from, for the screen.
    pub location: String,
    pub window_days: f64,
    /// Everything over the window, idle included.
    pub total: f64,
    pub idle: f64,
    /// Largest first. Idle is not in this list.
    pub namespaces: Vec<NamespaceSpend>,
}

impl ProviderCosts {
    /// Scale a window figure to a 30.4-day month.
    pub fn monthly(&self, window_cost: f64) -> f64 {
        if self.window_days <= 0.0 {
            return 0.0;
        }
        window_cost / self.window_days * DAYS_PER_MONTH
    }

    pub fn total_monthly(&self) -> f64 {
        self.monthly(self.total)
    }

    pub fn idle_monthly(&self) -> f64 {
        self.monthly(self.idle)
    }

    /// How much history the figures rest on, in a unit that reads sensibly
    /// ("5 min", not "0 days").
    pub fn window_label(&self) -> String {
        let minutes = self.window_days * 1440.0;
        if minutes < 60.0 {
            return format!("{} min", minutes.round().max(1.0));
        }
        if minutes < 1440.0 {
            return format!("{} h", trim(minutes / 60.0));
        }
        let days = self.window_days;
        format!("{} {}", trim(days), if (days - 1.0).abs() < 1e-9 { "day" } else { "days" })
    }

    /// Less than a day of history: scaling it to a month is a long
    /// extrapolation, and the figures will move as more is collected.
    pub fn is_short_sample(&self) -> bool {
        self.window_days < 1.0
    }

    pub fn for_namespace(&self, name: &str) -> Option<&NamespaceSpend> {
        self.namespaces.iter().find(|n| n.name == name)
    }
}

fn num(v: &Value, key: &str) -> f64 {
    v.get(key)
        .and_then(Value::as_f64)
        .filter(|x| x.is_finite() && *x >= 0.0)
        .unwrap_or(0.0)
}

/// Read an allocation response.
///
/// The body is `{ code, data: [ { "<name>": { totalCost, cpuCost, ... } } ] }`:
/// one map per time step, and a single step when `accumulate=true` was sent.
/// More than one is summed, so a server that ignored `accumulate` still reads
/// right. A body-level `code` other than 200 is a failure even though the HTTP
/// status was fine; Kubecost reports its own errors that way.
pub fn parse_allocation(
    body: &Value,
    source: Source,
    location: String,
) -> Result<ProviderCosts, String> {
    if let Some(code) = body.get("code").and_then(Value::as_i64) {
        if code != 200 {
            let msg = body
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("no message");
            return Err(format!("{} answered code {code}: {msg}", source.label()));
        }
    }
    let steps = body
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{} returned no allocation data", source.label()))?;

    let mut by_name: BTreeMap<String, NamespaceSpend> = BTreeMap::new();
    let mut eff_weight: BTreeMap<String, (f64, f64)> = BTreeMap::new();
    let mut minutes = 0.0f64;
    for step in steps {
        let Some(map) = step.as_object() else { continue };
        for (name, entry) in map {
            minutes = minutes.max(num(entry, "minutes"));
            let row = by_name.entry(name.clone()).or_insert_with(|| NamespaceSpend {
                name: name.clone(),
                total: 0.0,
                cpu: 0.0,
                ram: 0.0,
                storage: 0.0,
                network: 0.0,
                efficiency: None,
            });
            let total = num(entry, "totalCost");
            row.total += total;
            row.cpu += num(entry, "cpuCost");
            row.ram += num(entry, "ramCost");
            row.storage += num(entry, "pvCost");
            row.network += num(entry, "networkCost") + num(entry, "loadBalancerCost");
            if let Some(eff) = entry.get("totalEfficiency").and_then(Value::as_f64) {
                if eff.is_finite() {
                    let w = eff_weight.entry(name.clone()).or_default();
                    w.0 += eff * total;
                    w.1 += total;
                }
            }
        }
    }
    if by_name.is_empty() {
        // Answered, but with nothing in it: a tool installed minutes ago has
        // not collected anything yet. Not a failure of the tool or the call.
        return Err(format!(
            "{} has no cost data yet; after a fresh install it needs about 25 minutes to collect metrics",
            source.label()
        ));
    }
    for (name, (weighted, weight)) in eff_weight {
        if weight > 0.0 {
            if let Some(row) = by_name.get_mut(&name) {
                row.efficiency = Some((weighted / weight).clamp(0.0, 1.0));
            }
        }
    }

    let total: f64 = by_name.values().map(|r| r.total).sum();
    let idle = by_name.get("__idle__").map_or(0.0, |r| r.total);
    let mut namespaces: Vec<NamespaceSpend> =
        by_name.into_values().filter(|r| r.name != "__idle__").collect();
    namespaces.sort_by(|a, b| {
        b.total
            .partial_cmp(&a.total)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.name.cmp(&b.name))
    });

    // Prefer what the server says the window was; fall back to what was asked.
    let window_days = if minutes > 0.0 { minutes / 1440.0 } else { WINDOW_DAYS };
    Ok(ProviderCosts { source, location, window_days, total, idle, namespaces })
}

/// Ask the cluster for real costs. `Ok(None)` is "no Kubecost or OpenCost
/// here", which is not a failure; `Err` is one that is installed but did not
/// answer, and says which.
pub async fn fetch_costs(client: &kube::Client) -> Result<Option<ProviderCosts>, String> {
    let api: Api<Service> = Api::all(client.clone());
    let (services, _) = crate::list_cap::list_capped(&api, ListParams::default())
        .await
        .map_err(|e| format!("Failed to list services: {e}"))?;
    let found = candidates(&services);
    if found.is_empty() {
        return Ok(None);
    }

    let mut last = String::new();
    for c in &found {
        let path = allocation_path(c);
        let req = Request::get(&path).body(Vec::new()).map_err(|e| e.to_string())?;
        let place = format!("{}/{}", c.namespace, c.service);
        match tokio::time::timeout(request_timeout(), client.request::<Value>(req)).await {
            Err(_) => last = format!("{} at {place} timed out", c.source.label()),
            Ok(Err(e)) => last = format!("{} at {place} failed: {}", c.source.label(), short(&e.to_string())),
            Ok(Ok(body)) => match parse_allocation(&body, c.source, place.clone()) {
                Ok(costs) => return Ok(Some(costs)),
                Err(e) => last = format!("{e} (at {place})"),
            },
        }
    }
    Err(last)
}

/// `7.0` -> `7`, `1.5` -> `1.5`.
fn trim(value: f64) -> String {
    let one_place = format!("{value:.1}");
    one_place.strip_suffix(".0").unwrap_or(&one_place).to_string()
}

fn short(s: &str) -> String {
    s.lines().next().unwrap_or(s).chars().take(140).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn svc(ns: &str, name: &str, labels: Value, ports: &[i32]) -> Service {
        serde_json::from_value(json!({
            "apiVersion": "v1", "kind": "Service",
            "metadata": {"name": name, "namespace": ns, "labels": labels},
            "spec": {"ports": ports.iter().map(|p| json!({"port": p})).collect::<Vec<_>>()}
        }))
        .unwrap()
    }

    #[test]
    fn kubecosts_frontend_service_is_found_on_9090() {
        let s = [svc("kubecost", "kubecost-cost-analyzer", json!({"app": "cost-analyzer"}), &[9003, 9090])];
        let c = candidates(&s);
        assert_eq!(c.len(), 1);
        assert_eq!((c[0].source, c[0].port), (Source::Kubecost, 9090));
    }

    #[test]
    fn opencosts_api_is_found_on_9003() {
        let s = [svc("opencost", "opencost", json!({"app.kubernetes.io/name": "opencost"}), &[9003, 9090])];
        let c = candidates(&s);
        assert_eq!((c[0].source, c[0].port), (Source::OpenCost, 9003));
    }

    #[test]
    fn the_other_parts_of_a_kubecost_install_are_not_the_cost_api() {
        let s = [
            svc("kubecost", "kubecost-prometheus-server", json!({}), &[9090]),
            svc("kubecost", "kubecost-grafana", json!({}), &[80]),
            svc("kubecost", "kubecost-kube-state-metrics", json!({}), &[8080]),
            svc("kubecost", "kubecost-forecasting", json!({"app": "cost-analyzer"}), &[9090]),
            svc("default", "something-else", json!({}), &[9090]),
        ];
        assert!(candidates(&s).is_empty());
    }

    #[test]
    fn a_service_without_the_expected_port_is_skipped() {
        let s = [svc("kubecost", "kubecost-cost-analyzer", json!({"app": "cost-analyzer"}), &[8080])];
        assert!(candidates(&s).is_empty());
    }

    #[test]
    fn kubecost_is_tried_before_opencost() {
        let s = [
            svc("opencost", "opencost", json!({"app.kubernetes.io/name": "opencost"}), &[9003]),
            svc("kubecost", "kubecost-cost-analyzer", json!({"app": "cost-analyzer"}), &[9090]),
        ];
        let c = candidates(&s);
        assert_eq!(c[0].source, Source::Kubecost);
        assert_eq!(c[1].source, Source::OpenCost);
    }

    #[test]
    fn the_request_goes_through_the_api_servers_service_proxy() {
        let c = Candidate { namespace: "kubecost".into(), service: "kubecost-cost-analyzer".into(), port: 9090, source: Source::Kubecost };
        let p = allocation_path(&c);
        assert!(p.starts_with("/api/v1/namespaces/kubecost/services/kubecost-cost-analyzer:9090/proxy/model/allocation?"), "{p}");
        for q in ["window=7d", "aggregate=namespace", "accumulate=true", "includeIdle=true"] {
            assert!(p.contains(q), "{q} missing in {p}");
        }
        let o = Candidate { namespace: "opencost".into(), service: "opencost".into(), port: 9003, source: Source::OpenCost };
        assert!(allocation_path(&o).contains("/proxy/allocation/compute?"));
    }

    fn body() -> Value {
        json!({
            "code": 200, "status": "success",
            "data": [{
                "payments": {"name": "payments", "minutes": 10080.0, "cpuCost": 7.0, "ramCost": 3.0, "pvCost": 1.0,
                             "networkCost": 0.5, "loadBalancerCost": 0.5, "totalCost": 12.0, "totalEfficiency": 0.6},
                "kube-system": {"name": "kube-system", "minutes": 10080.0, "cpuCost": 2.0, "ramCost": 1.0, "totalCost": 3.0, "totalEfficiency": 0.25},
                "__idle__": {"name": "__idle__", "minutes": 10080.0, "cpuCost": 4.0, "ramCost": 1.0, "totalCost": 5.0}
            }]
        })
    }

    #[test]
    fn an_allocation_is_read_into_namespaces_idle_and_a_total() {
        let c = parse_allocation(&body(), Source::Kubecost, "kubecost/x".into()).unwrap();
        assert!((c.total - 20.0).abs() < 1e-9, "total includes idle");
        assert!((c.idle - 5.0).abs() < 1e-9);
        assert_eq!(c.namespaces.len(), 2, "idle is not a namespace row");
        assert_eq!(c.namespaces[0].name, "payments", "largest first");
        assert!((c.namespaces[0].cpu - 7.0).abs() < 1e-9);
        assert!((c.namespaces[0].network - 1.0).abs() < 1e-9, "network + load balancer");
        assert!((c.namespaces[0].efficiency.unwrap() - 0.6).abs() < 1e-9);
        assert_eq!(c.namespaces[1].name, "kube-system");
    }

    #[test]
    fn the_window_comes_from_the_response_and_costs_scale_to_a_month() {
        let c = parse_allocation(&body(), Source::Kubecost, "x".into()).unwrap();
        assert!((c.window_days - 7.0).abs() < 1e-9);
        // 20 over 7 days, to a 30.4167-day month.
        assert!((c.total_monthly() - 20.0 / 7.0 * (730.0 / 24.0)).abs() < 1e-6);
        assert!((c.idle_monthly() - 5.0 / 7.0 * (730.0 / 24.0)).abs() < 1e-6);
    }

    #[test]
    fn several_time_steps_are_summed() {
        let one = json!({"ns": {"totalCost": 1.0, "minutes": 1440.0}});
        let b = json!({"code": 200, "data": [one.clone(), one.clone(), one]});
        let c = parse_allocation(&b, Source::OpenCost, "x".into()).unwrap();
        assert!((c.total - 3.0).abs() < 1e-9);
    }

    #[test]
    fn a_missing_field_is_zero_and_nonsense_is_ignored() {
        let b = json!({"code": 200, "data": [{"ns": {"totalCost": 4.0, "cpuCost": "oops", "ramCost": -9.0}}]});
        let c = parse_allocation(&b, Source::Kubecost, "x".into()).unwrap();
        assert_eq!((c.namespaces[0].cpu, c.namespaces[0].ram, c.namespaces[0].efficiency), (0.0, 0.0, None));
        assert_eq!(c.window_days, WINDOW_DAYS, "no minutes reported: use the window asked for");
    }

    #[test]
    fn an_error_code_in_a_200_body_is_a_failure_with_its_message() {
        let b = json!({"code": 500, "message": "prometheus is not ready"});
        let e = parse_allocation(&b, Source::Kubecost, "x".into()).unwrap_err();
        assert!(e.contains("Kubecost") && e.contains("500") && e.contains("prometheus is not ready"), "{e}");
    }

    #[test]
    fn a_body_that_is_not_an_allocation_says_so() {
        for b in [json!({}), json!({"code": 200}), json!({"data": "nope"})] {
            assert!(parse_allocation(&b, Source::OpenCost, "x".into()).is_err(), "{b}");
        }
    }

    #[test]
    fn a_freshly_installed_cost_tool_that_has_no_data_yet_says_so() {
        // What a real Kubecost answered minutes after install: 200 with a null
        // (or empty) step. It answered; it just has nothing to report yet.
        for b in [
            json!({"code": 200, "data": [null]}),
            json!({"code": 200, "status": "success", "data": [{}]}),
            json!({"code": 200, "data": []}),
        ] {
            let e = parse_allocation(&b, Source::Kubecost, "kubecost/x".into()).unwrap_err();
            assert!(e.contains("no cost data yet"), "{b} -> {e}");
            assert!(e.contains("25 minutes"), "says how long to wait: {e}");
            assert!(!e.contains("did not answer"), "{e}");
        }
    }

    #[test]
    fn a_body_with_no_data_field_at_all_is_still_reported_as_not_an_allocation() {
        let e = parse_allocation(&json!({"code": 200}), Source::OpenCost, "x".into()).unwrap_err();
        assert!(e.contains("no allocation data") && !e.contains("yet"), "{e}");
    }

    fn costs_over(days: f64) -> ProviderCosts {
        ProviderCosts { source: Source::Kubecost, location: "x".into(), window_days: days, total: 1.0, idle: 0.0, namespaces: vec![] }
    }

    #[test]
    fn the_window_is_described_in_a_unit_that_does_not_round_to_nothing() {
        assert_eq!(costs_over(5.0 / 1440.0).window_label(), "5 min");
        assert_eq!(costs_over(90.0 / 1440.0).window_label(), "1.5 h");
        assert_eq!(costs_over(7.0).window_label(), "7 days");
        assert_eq!(costs_over(1.5).window_label(), "1.5 days");
        assert_eq!(costs_over(1.0).window_label(), "1 day");
    }

    #[test]
    fn under_a_day_of_data_is_a_short_sample() {
        assert!(costs_over(5.0 / 1440.0).is_short_sample());
        assert!(costs_over(0.99).is_short_sample());
        assert!(!costs_over(1.0).is_short_sample());
        assert!(!costs_over(7.0).is_short_sample());
    }

    #[test]
    fn a_zero_length_window_never_divides() {
        let c = ProviderCosts { source: Source::Kubecost, location: "x".into(), window_days: 0.0, total: 5.0, idle: 1.0, namespaces: vec![] };
        assert_eq!(c.total_monthly(), 0.0);
    }

    #[test]
    fn efficiency_is_weighted_by_cost_across_steps() {
        let b = json!({"code": 200, "data": [
            {"ns": {"totalCost": 1.0, "totalEfficiency": 1.0}},
            {"ns": {"totalCost": 3.0, "totalEfficiency": 0.2}}
        ]});
        let c = parse_allocation(&b, Source::Kubecost, "x".into()).unwrap();
        assert!((c.namespaces[0].efficiency.unwrap() - (1.0 * 1.0 + 0.2 * 3.0) / 4.0).abs() < 1e-9);
    }
}
