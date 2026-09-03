//! `k8s.topologyGraph` — the Topology screen's graph, joined where the objects
//! are.
//!
//! The screen draws how traffic reaches a workload: an Ingress routes to a
//! Service, the Service selects a workload's pods, and the workload owns the
//! ReplicaSets that are its revisions. Every one of those three joins needs a
//! field the list capabilities do not carry.
//!
//! ## Why this is a capability and not three calls from the screen
//!
//! The list capabilities return flattened rows built for tables:
//! `ServiceSummary` has `clusterIP` and `ports` but no `spec.selector`,
//! `IngressSummary` has `hosts` but not the backend Service each rule names,
//! and `ReplicaSetSummary` arrives already scoped to one owner. `listResource`
//! is no help either — it flattens every kind to name, namespace and age.
//!
//! So a screen doing this itself would have to fetch every Service and every
//! workload individually through `getObject` just to read a selector: one IPC
//! round trip per object, on every render of a namespace. That is the shape
//! [`crate::pod_overview`] exists to refuse, and the answer here is the same
//! one — join server-side, where the objects already are, and return a
//! structure built for the one screen that asks.
//!
//! ## Traffic flow, and where it comes from
//!
//! A topology screen exists to show how traffic moves, so ownership and
//! selectors are only half of it. The other half — `checkout-api` calls
//! `payments-api`, and both open a pool against a Postgres — is **not in the
//! Kubernetes API at all**. Nothing in a Deployment, a Service or an
//! EndpointSlice records one workload calling another.
//!
//! There are only three places that answer can come from, and this module is
//! built so all three land in the same graph:
//!
//! 1. **Configuration and the API's own external markers** — what a workload
//!    was TOLD to talk to: hosts named in its environment, its arguments and
//!    the ConfigMaps it reads ([`references_in`]), an `ExternalName` Service,
//!    and a Service whose EndpointSlices carry addresses that are not pods
//!    ([`external_backings`]) — the usual way a managed database is named from
//!    inside a cluster. Available everywhere with nothing installed, and the
//!    only source for the external systems the design shows
//!    (`postgres-primary`, `kafka`), since the API names neither directly.
//! 2. **NetworkPolicy** — what is PERMITTED. Intent rather than traffic.
//!    [`Provenance::Allowed`] is declared for it; nothing produces it yet.
//! 3. **Telemetry** — what actually HAPPENED: Istio's `istio_requests_total`,
//!    Linkerd's `response_total`, Cilium Hubble's flows, or any Prometheus
//!    scraping them. [`Provenance::Observed`] is declared for it; nothing
//!    produces it yet, because it needs a metrics source srelens does not have.
//!
//! **Every edge says which of those it came from.** That is the whole point of
//! [`Provenance`]: a diagram that draws a measured call and a string found in
//! an environment variable identically is worse than one that draws fewer
//! edges, because a reader trusts both the same. Naming the source lets all
//! three coexist honestly, and lets telemetry be an addition later rather than
//! a rewrite of everything here.
//!
//! What is still missing is the RATE — the design's `41.2k rpm` and
//! `12.4% 5xx`. Those need source 3; `k8s.podMetrics` is CPU millicores and
//! memory MiB, and there is no time series anywhere in srelens.
//!
//! ## The joins
//!
//! **Ingress -> Service** is read off the Ingress: every
//! `spec.rules[].http.paths[].backend.service.name`, plus `spec.defaultBackend`
//! when it names one. A rule pointing at a Service that does not exist produces
//! no edge rather than a node for the missing Service.
//!
//! **Service -> workload** is the selector, and it is a SUBSET test rather than
//! an equality one. A Service selects pods; a workload's pods carry its
//! `spec.template.metadata.labels` and usually more (`pod-template-hash`, and
//! whatever the team adds). So the Service fronts the workload when every pair
//! in its selector appears in the workload's template labels. A Service with
//! no selector at all — `ExternalName`, or one whose Endpoints are managed by
//! hand — gets no edge, because it genuinely selects no workload here.
//!
//! **Workload -> ReplicaSet** is `metadata.ownerReferences`, which is how the
//! Deployment controller marks the revisions it made.
//!
//! **Workload -> dependency** is the configuration scan. Hosts are classified
//! against the namespace's own Services: `name.namespace.svc[.cluster.local]`
//! is unambiguous and resolves wherever it points, including across
//! namespaces; a bare word is a Service only when one by that name exists,
//! which is what keeps `kafka:9092` from inventing an in-cluster Service on a
//! cluster that has no such thing; anything else is external. A ConfigMap's
//! contents reach only the workloads that actually mount or reference it —
//! attributing a namespace's whole configuration to every workload in it would
//! wire them all together and draw a diagram of nothing.
//!
//! Secrets are never opened. A topology picture is not worth reading secret
//! material for, and a host worth drawing is essentially always in plain
//! config.
//!
//! ## What is left out, and why
//!
//! **ReplicaSets scaled to zero.** A Deployment keeps ten old revisions by
//! default, all at zero replicas. Drawing them gives every workload a tail of
//! dead nodes that say nothing about how traffic flows today. One that is
//! scaled to zero AND has no pods left is dropped; a revision still winding
//! down is kept, because that is exactly the state a reader is looking for
//! during a rollout.
//!
//! **Health beyond ready-over-desired.** A node is `ok`, `degraded` or
//! `failing` purely from those two numbers. There is no error rate here and no
//! latency, because there is no source for either.

use std::collections::BTreeMap;
use std::sync::Arc;

use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, ReplicaSet, StatefulSet};
use k8s_openapi::api::core::v1::{ConfigMap, Service};
use k8s_openapi::api::discovery::v1::EndpointSlice;
use k8s_openapi::api::networking::v1::{Ingress, NetworkPolicy};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;
use kube::api::ListParams;
use kube::core::NamespaceResourceScope;
use kube::{Api, Resource};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use srelens_capability::{Annotations, Capability, CapabilityError};

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;

/// The annotation the Deployment controller stamps each ReplicaSet with.
const REVISION_ANNOTATION: &str = "deployment.kubernetes.io/revision";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TopologyGraphIn {
    pub context: String,
    /// The namespaces to draw, listed explicitly.
    ///
    /// Several, because a dependency rarely respects a namespace boundary: the
    /// design's own header reads `CHECKOUT · PAYMENTS · IDENTITY`, and a
    /// `checkout` that calls `payments-api.payments.svc` is only half a picture
    /// with `payments` left out.
    ///
    /// Still a list rather than "all", and deliberately: every namespace on a
    /// real cluster is thousands of nodes, which is not a picture of anything.
    /// An empty list draws nothing rather than everything — the safe reading of
    /// an unset field, and the opposite of what `listResource` does with one.
    pub namespaces: Vec<String>,
    /// Where to read measured traffic from, when the cluster has a metrics
    /// backend and the reader has pointed at it. Absent is the ordinary case:
    /// most clusters run none, and the graph is built from the API either way
    /// — telemetry only ever ADDS observed edges and rates on top.
    #[serde(default)]
    pub prometheus: Option<PrometheusSource>,
    /// Read each pod's own socket table over `pods/exec`.
    ///
    /// Off by default and deliberately: it is one exec per pod, it shows up
    /// in the audit log of every pod it touches, and it needs `pods/exec`
    /// wherever it runs. A reader asks for it; nothing turns it on for them.
    #[serde(default)]
    pub connections: bool,
}

/// Which column a node stands in. The screen lays these out left to right;
/// the order is fixed here so the two sides cannot disagree about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Lane {
    Route,
    Service,
    Workload,
    ReplicaSet,
    /// Things outside this cluster that something here was configured to talk
    /// to — a database host, a broker, an ExternalName target. Last, because
    /// that is where a dependency sits in the direction traffic travels.
    External,
}

/// How well a node is serving, from ready-over-desired and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Health {
    /// Every replica ready.
    Ok,
    /// Some ready, some not.
    Degraded,
    /// Replicas wanted, none of them ready.
    Failing,
    /// Nothing to count — an Ingress and a Service have no replicas of their
    /// own, and a workload scaled to zero has none to judge.
    Unknown,
}

/// Worst-first, so a Service can take the health of the worst thing behind it.
fn severity(health: Health) -> u8 {
    match health {
        Health::Failing => 0,
        Health::Degraded => 1,
        Health::Ok => 2,
        Health::Unknown => 3,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct TopologyNode {
    /// `Kind/namespace/name` — stable, and what edges refer to.
    pub id: String,
    pub kind: String,
    /// What to draw as the node's name. A ReplicaSet shows its revision
    /// (`rev 119`) rather than its generated name, which is the hash nobody
    /// reads; [`Self::id`] keeps the real one.
    pub name: String,
    pub namespace: String,
    pub lane: Lane,
    /// The one line under the name: `9/12`, `:80`, `3/3 ready`.
    pub detail: String,
    /// Ready and desired where the kind has them, so the screen renders its
    /// own ratio rather than parsing [`Self::detail`] back apart.
    pub ready: Option<i32>,
    pub desired: Option<i32>,
    pub health: Health,
}

/// What one edge means. `Routes` is traffic reaching something, `Owns` is a
/// controller having made it, and `Calls` is one thing depending on another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum EdgeKind {
    Routes,
    Owns,
    Calls,
}

/// HOW an edge is known, which matters at least as much as what it says.
///
/// A topology screen that draws a measured edge and a guessed one the same way
/// is worse than one that draws fewer edges, because a reader cannot tell them
/// apart and will trust both equally. So every edge says where it came from and
/// the screen renders the difference.
///
/// `Observed` has no producer yet — it is what a Prometheus, Hubble or mesh
/// source will set, and it is declared here so that source is an addition
/// rather than a reshaping of everything below it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Provenance {
    /// The API server said so outright: an ownerReference, a Service selector,
    /// an Ingress rule. Not an inference at all.
    Topology,
    /// Something names it in configuration — an env var, an argument, a
    /// ConfigMap value, an ExternalName. It says this workload was BUILT to
    /// talk to that, which is not the same as it having done so.
    Declared,
    /// A NetworkPolicy permits it. Intent, not traffic. No producer yet.
    Allowed,
    /// Telemetry measured it. No producer yet.
    Observed,
}

/// What a measurement counted.
///
/// Carried beside the number because the two sources do not measure the same
/// thing and must never be put on one scale: a metrics backend reports a RATE,
/// and a socket table reports how many connections are OPEN. Five idle pooled
/// connections and five requests a second are not comparable quantities, and a
/// screen scaling both against one maximum would draw that lie as a thickness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Unit {
    /// Requests per second, as PromQL returned it.
    Rps,
    /// Established TCP connections, counted in `/proc/net/tcp`.
    Connections,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct TopologyEdge {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
    pub provenance: Provenance,
    /// What to write along the edge — a measured rate, and nothing at all for
    /// an edge nobody measured. The design shows `41.2k rpm` here; only
    /// [`Provenance::Observed`] can fill it, which is why it is empty until a
    /// metrics source is configured.
    pub detail: String,
    /// The raw number behind {@link detail}, so the screen can draw volume
    /// rather than only write it.
    ///
    /// Sent separately rather than left for the frontend to read back out of
    /// `"41.2k rpm"`: a label is rounded, abbreviated and written for a human,
    /// and parsing one to recover a quantity we already had is a bug waiting
    /// for the day the wording changes.
    pub weight: Option<f64>,
    /// What {@link weight} counts. `None` exactly when `weight` is.
    pub unit: Option<Unit>,
    /// The health of the node this edge points AT, copied here so the screen
    /// can colour a path without walking back to the node table.
    pub health: Health,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TopologyGraphOut {
    pub nodes: Vec<TopologyNode>,
    pub edges: Vec<TopologyEdge>,
}

/// Ready and desired, reduced to a verdict.
///
/// `desired == 0` is [`Health::Unknown`] rather than `Ok`: a workload scaled to
/// zero is not healthy, it is absent, and painting a deliberately-stopped thing
/// the same green as a serving one is the kind of false reassurance a topology
/// screen exists to avoid.
pub fn health_of(ready: i32, desired: i32) -> Health {
    if desired <= 0 {
        Health::Unknown
    } else if ready >= desired {
        Health::Ok
    } else if ready <= 0 {
        Health::Failing
    } else {
        Health::Degraded
    }
}

/// One node per external host, shared across every namespace in view.
///
/// Not namespaced, deliberately: when two namespaces both name the same
/// database, that is ONE database with two callers, and drawing it twice would
/// hide exactly the fan-in a reader is looking for.
pub fn external_id(host: &str) -> String {
    format!("External//{host}")
}

/// `Kind/namespace/name`, the id every edge is written in terms of.
pub fn node_id(kind: &str, namespace: &str, name: &str) -> String {
    format!("{kind}/{namespace}/{name}")
}

/// Whether a Service's selector picks out a workload's pods.
///
/// Subset, not equality: the pods a workload makes carry its template labels
/// plus whatever the controller adds — a Deployment's carry `pod-template-hash`
/// — so a Service naming one label of the set still fronts that workload.
///
/// An empty selector matches nothing here. Kubernetes reads an empty selector
/// on a Service as "no selector", meaning the Endpoints are someone else's to
/// manage; reading it as "matches everything" would wire every Service in the
/// namespace to every workload in it.
pub fn selector_matches(
    selector: &BTreeMap<String, String>,
    labels: &BTreeMap<String, String>,
) -> bool {
    if selector.is_empty() {
        return false;
    }
    selector.iter().all(|(k, v)| labels.get(k) == Some(v))
}

/// The Services an Ingress sends traffic to, in the order the spec names them
/// and without repeats.
pub fn ingress_backends(ingress: &Ingress) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let Some(spec) = ingress.spec.as_ref() else {
        return out;
    };
    let mut push = |name: Option<&String>| {
        if let Some(n) = name {
            if !n.is_empty() && !out.contains(n) {
                out.push(n.clone());
            }
        }
    };
    push(
        spec.default_backend
            .as_ref()
            .and_then(|b| b.service.as_ref())
            .map(|s| &s.name),
    );
    for rule in spec.rules.iter().flatten() {
        let Some(http) = rule.http.as_ref() else {
            continue;
        };
        for path in &http.paths {
            push(path.backend.service.as_ref().map(|s| &s.name));
        }
    }
    out
}

/// A ReplicaSet's revision, as the Deployment controller recorded it.
pub fn revision_of(rs: &ReplicaSet) -> Option<String> {
    rs.metadata
        .annotations
        .as_ref()
        .and_then(|a| a.get(REVISION_ANNOTATION))
        .cloned()
}

/// The Deployment a ReplicaSet belongs to, if one made it.
pub fn owning_deployment(rs: &ReplicaSet) -> Option<String> {
    rs.metadata
        .owner_references
        .as_ref()?
        .iter()
        .find(|o| o.kind == "Deployment")
        .map(|o| o.name.clone())
}

/// Add an edge unless the same pair is already joined the same way.
///
/// Configuration repeats itself — the same host appears in an argument and in
/// the ConfigMap behind it, and two containers of one pod name it separately.
/// Each of those is the same dependency, and drawing it three times would say
/// something about strength that this data cannot support.
fn push_edge(
    edges: &mut Vec<TopologyEdge>,
    from: String,
    to: String,
    kind: EdgeKind,
    provenance: Provenance,
) {
    push_edge_labelled(edges, from, to, kind, provenance, String::new(), None)
}

/// As [`push_edge`], with something to write along the line and the quantity
/// behind it.
///
/// A measurement UPGRADES an edge rather than joining it: when telemetry has
/// seen the same call that configuration declared, that is one dependency now
/// known better, and drawing two lines between the same pair would say the
/// opposite. The observed provenance, its rate and its weight replace the
/// declared ones together — a stale weight left beside a fresh label would be
/// drawn at the wrong thickness.
fn push_edge_labelled(
    edges: &mut Vec<TopologyEdge>,
    from: String,
    to: String,
    kind: EdgeKind,
    provenance: Provenance,
    detail: String,
    measure: Option<(f64, Unit)>,
) {
    if let Some(existing) = edges
        .iter_mut()
        .find(|e| e.from == from && e.to == to && e.kind == kind)
    {
        if provenance == Provenance::Observed {
            existing.provenance = provenance;
            existing.detail = detail;
            existing.weight = measure.map(|(w, _)| w);
            existing.unit = measure.map(|(_, u)| u);
        }
        return;
    }
    edges.push(TopologyEdge {
        from,
        to,
        kind,
        provenance,
        detail,
        weight: measure.map(|(w, _)| w),
        unit: measure.map(|(_, u)| u),
        // A declared dependency says nothing about how the target is doing —
        // the target's own node carries that, and copying an unrelated health
        // onto the edge would colour a line with a fact it does not hold.
        health: Health::Unknown,
    });
}

/// A host something was configured to reach.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reference {
    /// A Service in this cluster. `namespace` is `None` when the reference was
    /// a bare name, which Kubernetes resolves in the pod's own namespace.
    Service { name: String, namespace: Option<String> },
    /// A host that is not a Service here.
    External { host: String },
}

/// Whether a string could be a DNS host at all.
///
/// Deliberately strict. This runs over every environment variable and
/// ConfigMap value in a namespace, most of which are not hosts, and a loose
/// rule turns log formats and feature flags into nodes on a diagram.
fn plausible_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && !host.starts_with(['-', '.'])
        && !host.ends_with(['-', '.'])
        && !host.contains("..")
        && host.chars().any(|c| c.is_ascii_alphabetic())
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
}

/// Whether a host is cluster DNS, which is self-qualifying: nothing but a
/// Kubernetes name is spelled `something.something.svc`.
fn is_cluster_dns(host: &str) -> bool {
    host.split('.').any(|label| label == "svc")
}

/// Pull the host out of one token, and say whether it was qualified enough to
/// trust on its own.
///
/// Returns `(host, qualified)`. A token is qualified when it carried a scheme
/// (`postgres://db`), a port (`kafka:9092`), or is cluster DNS
/// (`svc.cluster.local`).
///
/// **A dot is deliberately NOT enough**, which was learned from a real cluster:
/// `storefront`'s command mentions `nginx.conf`, and reading a dot as a
/// hostname put a `nginx.conf` node in the dependency lane. Filenames,
/// versions and class names are all dotted, and they outnumber bare hostnames
/// in configuration by a wide margin. A connection string worth drawing almost
/// always carries a scheme or a port, so requiring one costs little and stops
/// the diagram asserting dependencies that do not exist — which is the worse
/// failure of the two, because a missing edge is invisible while a false one is
/// believed.
fn host_of(token: &str) -> Option<(String, bool)> {
    let token = token.trim_matches(|c: char| {
        c.is_whitespace() || matches!(c, '"' | '\'' | ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}')
    });
    if token.is_empty() {
        return None;
    }
    // A scheme is the strongest signal there is, so it is taken first and
    // remembered even after it is stripped.
    let (rest, had_scheme) = match token.split_once("://") {
        Some((scheme, rest)) if !scheme.is_empty() && scheme.chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-') => (rest, true),
        _ => (token, false),
    };
    // Credentials, then path/query, then the fragment — everything after the
    // authority is not the host.
    let rest = rest.rsplit_once('@').map(|(_, after)| after).unwrap_or(rest);
    let rest = rest.split(['/', '?', '#']).next().unwrap_or("");
    if rest.is_empty() {
        return None;
    }
    let (host, had_port) = match rest.rsplit_once(':') {
        // Only a numeric port counts. `key:value` is not a host and a port.
        Some((h, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => (h, true),
        _ => (rest, false),
    };
    let host = host.trim_end_matches('.');
    if !plausible_host(host) {
        return None;
    }
    Some((host.to_string(), had_scheme || had_port || is_cluster_dns(host)))
}

/// Classify one host against the cluster's own naming.
///
/// `<name>.<namespace>.svc[.cluster.local]` is unambiguous and is read as a
/// Service wherever it points. A dotted host that is NOT `.svc` is external —
/// `db.example.com` is somebody else's machine. A bare word is a Service only
/// if the namespace actually has one by that name, which is what stops
/// `kafka:9092` inventing an in-cluster Service that does not exist.
pub fn classify(host: &str, services_here: &[String]) -> Reference {
    let labels: Vec<&str> = host.split('.').collect();
    if let Some(svc) = labels.iter().position(|l| *l == "svc") {
        // `name.namespace.svc...` — the two labels before `svc`.
        if svc >= 2 {
            return Reference::Service {
                name: labels[svc - 2].to_string(),
                namespace: Some(labels[svc - 1].to_string()),
            };
        }
    }
    if !host.contains('.') && services_here.iter().any(|s| s == host) {
        return Reference::Service { name: host.to_string(), namespace: None };
    }
    Reference::External { host: host.to_string() }
}

/// Every host named in one blob of configuration, in the order they appear and
/// without repeats.
///
/// The text is split on whitespace and on `=` so that `DB=postgres://host:5432`
/// yields its value, and every token is tested. Anything that does not look
/// like a host is dropped silently: this reads configuration written for other
/// programs, so most of what it sees is not addressed to it.
pub fn references_in(text: &str, services_here: &[String]) -> Vec<Reference> {
    let mut out: Vec<Reference> = Vec::new();
    for token in text.split(|c: char| c.is_whitespace() || c == '=') {
        let Some((host, qualified)) = host_of(token) else {
            continue;
        };
        // A bare unqualified word is only a host if the namespace names a
        // Service exactly that. Without this every value in every ConfigMap
        // becomes a candidate.
        if !qualified && !services_here.iter().any(|s| *s == host) {
            continue;
        }
        let reference = classify(&host, services_here);
        if !out.contains(&reference) {
            out.push(reference);
        }
    }
    out
}

/// One external address a Service is actually backed by.
pub struct ExternalBacking {
    pub namespace: String,
    pub host: String,
}

/// The Services whose endpoints are NOT pods, with the addresses behind them.
///
/// A Service with hand-managed Endpoints — the usual way to name a managed
/// database or a VM inside the cluster — looks identical to one whose selector
/// simply matches nothing: both have no workload behind them. The difference is
/// in the EndpointSlice, where a pod-backed endpoint carries a `targetRef` of
/// kind `Pod` and an external one carries none. That absence is the only signal
/// the API gives, and it is a real one: those addresses are a dependency the
/// cluster genuinely has.
///
/// Endpoints WITH a pod `targetRef` are ignored entirely — the Service already
/// reaches those through its selector, and adding their pod IPs would draw the
/// cluster's own internals as external systems.
pub fn external_backings(
    slices: &[EndpointSlice],
) -> Vec<(String, ExternalBacking)> {
    let mut out: Vec<(String, ExternalBacking)> = Vec::new();
    for slice in slices {
        let Some(service) = slice
            .metadata
            .labels
            .as_ref()
            .and_then(|l| l.get("kubernetes.io/service-name"))
            .cloned()
        else {
            continue;
        };
        let namespace = slice.metadata.namespace.clone().unwrap_or_default();
        for endpoint in &slice.endpoints {
            if endpoint
                .target_ref
                .as_ref()
                .and_then(|r| r.kind.as_deref())
                .is_some_and(|kind| kind == "Pod")
            {
                continue;
            }
            for address in &endpoint.addresses {
                let backing = ExternalBacking {
                    namespace: namespace.clone(),
                    host: address.clone(),
                };
                if !out
                    .iter()
                    .any(|(s, b)| s == &service && b.host == backing.host)
                {
                    out.push((service.clone(), backing));
                }
            }
        }
    }
    out
}

/// Whether a ReplicaSet is worth drawing.
///
/// A Deployment keeps its old revisions at zero replicas — ten of them by
/// default. One that wants nothing and is running nothing is history, not
/// topology. One still winding down has pods serving traffic, which is exactly
/// what a reader mid-rollout is looking for, so it stays.
pub fn replicaset_is_live(desired: i32, current: i32) -> bool {
    desired > 0 || current > 0
}

async fn list_of<K>(
    client: kube::Client,
    namespace: &str,
    what: &'static str,
) -> Result<Vec<K>, CapabilityError>
where
    K: Resource<Scope = NamespaceResourceScope>
        + Clone
        + std::fmt::Debug
        + serde::de::DeserializeOwned,
    <K as Resource>::DynamicType: Default,
{
    let api: Api<K> = crate::scoped_api(client, namespace);
    let list = tokio::time::timeout(request_timeout(), api.list(&ListParams::default()))
        .await
        .map_err(|_| CapabilityError::Handler(format!("list {what} timed out")))?
        .map_err(|e| CapabilityError::Handler(e.to_string()))?;
    Ok(list.items)
}

fn template_labels(labels: Option<&BTreeMap<String, String>>) -> BTreeMap<String, String> {
    labels.cloned().unwrap_or_default()
}

/// One workload reduced to what the graph needs of it: the labels its pods
/// carry, how many of them are ready, and what its configuration names.
struct Workload {
    kind: &'static str,
    name: String,
    namespace: String,
    labels: BTreeMap<String, String>,
    ready: i32,
    desired: i32,
    /// Every string in the pod spec that might name a host — env values,
    /// arguments, commands.
    text: Vec<String>,
    /// ConfigMaps this workload actually reads, by name.
    ///
    /// Collected rather than scanning every ConfigMap in the namespace,
    /// because a ConfigMap nothing mounts is not this workload's dependency —
    /// attributing the namespace's whole config to every workload in it would
    /// wire them all to the same hosts.
    config_maps: Vec<String>,
}

/// What a pod spec says, for the reference scan: the literal strings, and the
/// ConfigMaps it pulls in.
///
/// `valueFrom` env vars contribute their ConfigMap NAME but no text — the
/// value lives in the ConfigMap, and that is where it gets read. A Secret is
/// never opened: a topology diagram is not worth reading secret material for,
/// and a host worth drawing is essentially always in plain config.
fn pod_spec_sources(template: Option<&k8s_openapi::api::core::v1::PodTemplateSpec>) -> (Vec<String>, Vec<String>) {
    let mut text: Vec<String> = Vec::new();
    let mut config_maps: Vec<String> = Vec::new();
    let Some(spec) = template.and_then(|t| t.spec.as_ref()) else {
        return (text, config_maps);
    };
    for volume in spec.volumes.iter().flatten() {
        if let Some(cm) = volume.config_map.as_ref().map(|c| c.name.clone()) {
            config_maps.push(cm);
        }
    }
    let containers = spec
        .containers
        .iter()
        .chain(spec.init_containers.iter().flatten());
    for container in containers {
        for arg in container.args.iter().flatten() {
            text.push(arg.clone());
        }
        for cmd in container.command.iter().flatten() {
            text.push(cmd.clone());
        }
        for env in container.env.iter().flatten() {
            if let Some(value) = env.value.as_ref() {
                text.push(value.clone());
            }
            if let Some(cm) = env
                .value_from
                .as_ref()
                .and_then(|f| f.config_map_key_ref.as_ref())
                .map(|r| r.name.clone())
            {
                config_maps.push(cm);
            }
        }
        for from in container.env_from.iter().flatten() {
            if let Some(cm) = from.config_map_ref.as_ref().map(|r| r.name.clone()) {
                config_maps.push(cm);
            }
        }
    }
    config_maps.sort();
    config_maps.dedup();
    (text, config_maps)
}

impl Workload {
    fn node(&self) -> TopologyNode {
        TopologyNode {
            id: node_id(self.kind, &self.namespace, &self.name),
            kind: self.kind.to_string(),
            name: self.name.clone(),
            namespace: self.namespace.clone(),
            lane: Lane::Workload,
            detail: format!("{}/{}", self.ready, self.desired),
            ready: Some(self.ready),
            desired: Some(self.desired),
            health: health_of(self.ready, self.desired),
        }
    }
}

fn deployment_workload(d: &Deployment) -> Workload {
    let spec = d.spec.as_ref();
    let (text, config_maps) = pod_spec_sources(spec.map(|s| &s.template));
    Workload {
        kind: "Deployment",
        name: d.metadata.name.clone().unwrap_or_default(),
        namespace: d.metadata.namespace.clone().unwrap_or_default(),
        labels: template_labels(
            spec.and_then(|s| s.template.metadata.as_ref())
                .and_then(|m| m.labels.as_ref()),
        ),
        ready: d.status.as_ref().and_then(|s| s.ready_replicas).unwrap_or(0),
        desired: spec.and_then(|s| s.replicas).unwrap_or(0),
        text,
        config_maps,
    }
}

fn statefulset_workload(s: &StatefulSet) -> Workload {
    let spec = s.spec.as_ref();
    let (text, config_maps) = pod_spec_sources(spec.map(|s| &s.template));
    Workload {
        kind: "StatefulSet",
        name: s.metadata.name.clone().unwrap_or_default(),
        namespace: s.metadata.namespace.clone().unwrap_or_default(),
        labels: template_labels(
            spec.and_then(|s| s.template.metadata.as_ref())
                .and_then(|m| m.labels.as_ref()),
        ),
        ready: s.status.as_ref().and_then(|s| s.ready_replicas).unwrap_or(0),
        desired: spec.and_then(|s| s.replicas).unwrap_or(0),
        text,
        config_maps,
    }
}

/// A DaemonSet's "desired" is how many nodes it should run on, which the
/// scheduler decides — so both numbers come from status, not from a replica
/// count its spec does not have.
fn daemonset_workload(d: &DaemonSet) -> Workload {
    let status = d.status.as_ref();
    let (text, config_maps) = pod_spec_sources(d.spec.as_ref().map(|s| &s.template));
    Workload {
        kind: "DaemonSet",
        name: d.metadata.name.clone().unwrap_or_default(),
        namespace: d.metadata.namespace.clone().unwrap_or_default(),
        labels: template_labels(
            d.spec
                .as_ref()
                .and_then(|s| s.template.metadata.as_ref())
                .and_then(|m| m.labels.as_ref()),
        ),
        ready: status.map(|s| s.number_ready).unwrap_or(0),
        desired: status.map(|s| s.desired_number_scheduled).unwrap_or(0),
        text,
        config_maps,
    }
}

/// Build the graph from objects already fetched.
///
/// Separated from the capability so every join above is testable without a
/// cluster — the joins are the part that can be wrong, and the six list calls
/// are not.
pub fn build_graph(
    ingresses: Vec<Ingress>,
    services: Vec<Service>,
    deployments: Vec<Deployment>,
    statefulsets: Vec<StatefulSet>,
    daemonsets: Vec<DaemonSet>,
    replicasets: Vec<ReplicaSet>,
    config_maps: Vec<ConfigMap>,
    endpointslices: Vec<EndpointSlice>,
) -> TopologyGraphOut {
    let mut nodes: Vec<TopologyNode> = Vec::new();
    let mut edges: Vec<TopologyEdge> = Vec::new();

    let workloads: Vec<Workload> = deployments
        .iter()
        .map(deployment_workload)
        .chain(statefulsets.iter().map(statefulset_workload))
        .chain(daemonsets.iter().map(daemonset_workload))
        .collect();

    // Services are built first: an Ingress edge needs its Service node to
    // exist, and a Service's own health is the health of what stands behind it.
    // Keyed by (namespace, name): drawing several namespaces at once means two
    // of them may each have a Service called `checkout`, and a name-only key
    // silently merged them into one node with both sets of edges.
    let mut service_nodes: BTreeMap<(String, String), TopologyNode> = BTreeMap::new();
    for svc in &services {
        let name = svc.metadata.name.clone().unwrap_or_default();
        let namespace = svc.metadata.namespace.clone().unwrap_or_default();
        let spec = svc.spec.as_ref();
        let ports = spec
            .and_then(|s| s.ports.as_ref())
            .map(|ports| {
                ports
                    .iter()
                    .map(|p| format!(":{}", p.port))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();
        let selector: BTreeMap<String, String> =
            spec.and_then(|s| s.selector.clone()).unwrap_or_default();

        let backing: Vec<&Workload> = workloads
            .iter()
            .filter(|w| selector_matches(&selector, &w.labels))
            .collect();
        // As healthy as the worst thing behind it. With nothing behind it there
        // is no reading to take rather than a failure — an ExternalName Service
        // is meant to have no workload here.
        let health = backing
            .iter()
            .map(|w| health_of(w.ready, w.desired))
            .min_by_key(|h| severity(*h))
            .unwrap_or(Health::Unknown);

        let id = node_id("Service", &namespace, &name);
        service_nodes.insert(
            (namespace.clone(), name.clone()),
            TopologyNode {
                id: id.clone(),
                kind: "Service".into(),
                name: name.clone(),
                namespace: namespace.clone(),
                lane: Lane::Service,
                detail: ports,
                ready: None,
                desired: None,
                health,
            },
        );

        for w in backing {
            edges.push(TopologyEdge {
                from: id.clone(),
                to: node_id(w.kind, &w.namespace, &w.name),
                kind: EdgeKind::Routes,
                provenance: Provenance::Topology,
                detail: String::new(),
                weight: None,
                unit: None,
                health: health_of(w.ready, w.desired),
            });
        }
    }

    for ing in &ingresses {
        let name = ing.metadata.name.clone().unwrap_or_default();
        let namespace = ing.metadata.namespace.clone().unwrap_or_default();
        let hosts: Vec<String> = ing
            .spec
            .as_ref()
            .and_then(|s| s.rules.as_ref())
            .map(|rules| rules.iter().filter_map(|r| r.host.clone()).collect())
            .unwrap_or_default();
        let id = node_id("Ingress", &namespace, &name);
        nodes.push(TopologyNode {
            id: id.clone(),
            kind: "Ingress".into(),
            name,
            namespace: namespace.clone(),
            lane: Lane::Route,
            detail: hosts.join(" "),
            ready: None,
            desired: None,
            health: Health::Unknown,
        });
        for backend in ingress_backends(ing) {
            // Only for a Service that exists: a rule naming one that does not
            // is a broken Ingress, and inventing the node would draw traffic
            // arriving somewhere it cannot.
            let Some(target) = service_nodes.get(&(namespace.clone(), backend)) else {
                continue;
            };
            edges.push(TopologyEdge {
                from: id.clone(),
                to: target.id.clone(),
                kind: EdgeKind::Routes,
                provenance: Provenance::Topology,
                detail: String::new(),
                weight: None,
                unit: None,
                health: target.health,
            });
        }
    }

    nodes.extend(service_nodes.into_values());
    nodes.extend(workloads.iter().map(|w| w.node()));

    for rs in &replicasets {
        let desired = rs.spec.as_ref().and_then(|s| s.replicas).unwrap_or(0);
        let current = rs.status.as_ref().map(|s| s.replicas).unwrap_or(0);
        if !replicaset_is_live(desired, current) {
            continue;
        }
        let name = rs.metadata.name.clone().unwrap_or_default();
        let namespace = rs.metadata.namespace.clone().unwrap_or_default();
        let ready = rs
            .status
            .as_ref()
            .and_then(|s| s.ready_replicas)
            .unwrap_or(0);
        let id = node_id("ReplicaSet", &namespace, &name);
        nodes.push(TopologyNode {
            id: id.clone(),
            kind: "ReplicaSet".into(),
            name: revision_of(rs)
                .map(|r| format!("rev {r}"))
                .unwrap_or_else(|| name.clone()),
            namespace: namespace.clone(),
            lane: Lane::ReplicaSet,
            detail: format!("{ready}/{desired} ready"),
            ready: Some(ready),
            desired: Some(desired),
            health: health_of(ready, desired),
        });
        if let Some(owner) = owning_deployment(rs) {
            edges.push(TopologyEdge {
                from: node_id("Deployment", &namespace, &owner),
                to: id,
                kind: EdgeKind::Owns,
                provenance: Provenance::Topology,
                detail: String::new(),
                weight: None,
                unit: None,
                health: health_of(ready, desired),
            });
        }
    }

    // --- What each workload was configured to talk to ------------------------
    //
    // The lanes above are what the cluster BUILT. This is what it was TOLD:
    // every host named in a workload's environment, arguments and the
    // ConfigMaps it reads. It is the only source of service-to-service and
    // external dependencies available without a mesh, Prometheus or an eBPF
    // agent — and it is where a `postgres-primary` or a `kafka` comes from,
    // since nothing in the Kubernetes API mentions either.
    //
    // Marked `Declared` throughout, and that word is doing real work: this says
    // a workload was built to reach a host, NOT that it ever has.
    let service_names: Vec<String> = services
        .iter()
        .filter_map(|s| s.metadata.name.clone())
        .collect();
    let config_text: BTreeMap<String, Vec<String>> = config_maps
        .iter()
        .filter_map(|cm| {
            let name = cm.metadata.name.clone()?;
            let values = cm
                .data
                .iter()
                .flatten()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            Some((name, values))
        })
        .collect();

    let mut externals: BTreeMap<String, TopologyNode> = BTreeMap::new();
    for w in &workloads {
        let from = node_id(w.kind, &w.namespace, &w.name);
        let mut blobs: Vec<&String> = w.text.iter().collect();
        for cm in &w.config_maps {
            if let Some(values) = config_text.get(cm) {
                blobs.extend(values.iter());
            }
        }
        for blob in blobs {
            for reference in references_in(blob, &service_names) {
                match reference {
                    Reference::Service { name, namespace: ns } => {
                        let ns = ns.unwrap_or_else(|| w.namespace.clone());
                        let to = node_id("Service", &ns, &name);
                        // A Service in another namespace is real and named, but
                        // was never listed here — so it gets a node of its own,
                        // rather than an edge into nothing.
                        if !nodes.iter().any(|n| n.id == to) {
                            nodes.push(TopologyNode {
                                id: to.clone(),
                                kind: "Service".into(),
                                name: name.clone(),
                                namespace: ns.clone(),
                                lane: Lane::Service,
                                detail: if ns == w.namespace { String::new() } else { ns.clone() },
                                ready: None,
                                desired: None,
                                health: Health::Unknown,
                            });
                        }
                        push_edge(&mut edges, from.clone(), to, EdgeKind::Calls, Provenance::Declared);
                    }
                    Reference::External { host } => {
                        let to = external_id(&host);
                        externals.entry(host.clone()).or_insert(TopologyNode {
                            id: to.clone(),
                            kind: "External".into(),
                            name: host,
                            namespace: String::new(),
                            lane: Lane::External,
                            detail: String::new(),
                            ready: None,
                            desired: None,
                            health: Health::Unknown,
                        });
                        push_edge(&mut edges, from.clone(), to, EdgeKind::Calls, Provenance::Declared);
                    }
                }
            }
        }
    }

    // A Service whose endpoints are not pods is backed by something outside the
    // cluster: a managed database, a VM, an appliance. The addresses are in the
    // EndpointSlice and nothing else in the API says so — a Service with
    // hand-managed Endpoints looks exactly like one whose selector matches
    // nothing, and only the absence of a pod `targetRef` tells them apart.
    for (service, address) in external_backings(&endpointslices) {
        let ns = address.namespace.clone();
        let to = external_id(&address.host);
        externals.entry(address.host.clone()).or_insert(TopologyNode {
            id: to.clone(),
            kind: "External".into(),
            name: address.host,
            namespace: String::new(),
            lane: Lane::External,
            detail: String::new(),
            ready: None,
            desired: None,
            health: Health::Unknown,
        });
        push_edge(
            &mut edges,
            node_id("Service", &ns, &service),
            to,
            EdgeKind::Calls,
            Provenance::Declared,
        );
    }

    // An ExternalName Service IS a declared external dependency, spelled in the
    // API rather than in someone's environment.
    for svc in &services {
        let Some(target) = svc.spec.as_ref().and_then(|s| s.external_name.clone()) else {
            continue;
        };
        let name = svc.metadata.name.clone().unwrap_or_default();
        let ns = svc.metadata.namespace.clone().unwrap_or_default();
        let to = external_id(&target);
        externals.entry(target.clone()).or_insert(TopologyNode {
            id: to.clone(),
            kind: "External".into(),
            name: target,
            namespace: ns.clone(),
            lane: Lane::External,
            detail: String::new(),
            ready: None,
            desired: None,
            health: Health::Unknown,
        });
        push_edge(
            &mut edges,
            node_id("Service", &ns, &name),
            to,
            EdgeKind::Calls,
            Provenance::Declared,
        );
    }

    nodes.extend(externals.into_values());

    TopologyGraphOut { nodes, edges }
}

/// The PromQL that turns a mesh's counters into a call graph.
///
/// Istio first because it is the most widely deployed and the best documented,
/// and because `istio_requests_total` already carries exactly the four labels
/// an edge needs — who called, from where, what they called, and where that
/// lives. `rate(...[5m])` over five minutes rather than one: a minute of a
/// low-traffic service is mostly zero, and an edge that blinks in and out is
/// worse than one that lags a rollout by a few minutes.
///
/// Linkerd's `response_total` and Hubble's `hubble_flows_processed_total` are
/// the same shape with different label names, and belong here beside this one
/// when they are added. Nothing about the plumbing changes for them — which is
/// the point of routing all of it through one query capability.
/// How many pods one connection read will exec into.
///
/// Each is a round trip and an audit-log entry, so this is a budget rather
/// than a limit of the data: a namespace with more pods than this gives a
/// partial picture, which is the honest trade for not opening five hundred
/// exec sessions to draw one diagram.
pub const CONNECTION_POD_LIMIT: usize = 40;

pub const ISTIO_CALL_GRAPH: &str = "sum by (source_workload, source_workload_namespace, destination_service_name, destination_service_namespace) (rate(istio_requests_total[5m]))";

/// Cilium's Hubble, where its HTTP visibility is on.
///
/// `hubble_http_requests_total` and not `hubble_flows_processed_total`,
/// deliberately. The flow counter is what Hubble always exports, and it counts
/// FLOWS — packet-level events, several per request and a steady trickle per
/// idle connection — so a rate of it written as `rpm` would be a number nobody
/// measured. The HTTP counter is a request rate, the same quantity Istio
/// reports and the same unit on the screen, and it exists only where an
/// operator turned L7 visibility on. A cluster without that gets no observed
/// edges from Hubble, which is the honest answer.
///
/// The per-workload labels are the `labelsContext` set Cilium's own Helm
/// example configures for `httpV2`. A Hubble configured without them answers
/// this with no series at all, and the graph simply stays declared. Where the
/// `traffic_direction` label exists the ingress side is taken, so one request
/// is not counted once leaving its caller and once arriving.
pub const HUBBLE_HTTP_CALL_GRAPH: &str = "sum by (source_workload, source_namespace, destination_workload, destination_namespace) (rate(hubble_http_requests_total{traffic_direction=~\"ingress|\"}[5m]))";

/// Which telemetry a batch of samples came from, and so which labels name the
/// two ends of a call and what kind of thing each end is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallSchema {
    /// `istio_requests_total`: the source is a workload, the destination a
    /// Service — Istio reports the address that was called.
    Istio,
    /// `hubble_http_requests_total`: both ends are workloads. Hubble watches
    /// pod to pod, below the Service abstraction, so the address is worked
    /// back out from the graph; see [`fronting_service`].
    Hubble,
}

/// Every call-graph query, best evidence first.
///
/// One mesh per cluster is the working assumption: the first query that
/// returns any series is taken and the rest are not asked, so a cluster that
/// somehow runs both is not drawn with each edge's rate overwritten by the
/// other's.
pub const CALL_GRAPH_QUERIES: &[(CallSchema, &str)] = &[
    (CallSchema::Istio, ISTIO_CALL_GRAPH),
    (CallSchema::Hubble, HUBBLE_HTTP_CALL_GRAPH),
];

/// Where a metrics backend lives, as `k8s.prometheusDiscover` reported it.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct PrometheusSource {
    pub namespace: String,
    pub service: String,
    pub port: i32,
}

/// Per-minute, because per-second is what PromQL returns and not what anybody
/// reads. Below a tenth of a request per minute there is nothing worth a label
/// — the edge still draws, it just does not claim a number.
pub fn rate_label(per_second: f64) -> String {
    let per_minute = per_second * 60.0;
    if per_minute < 0.1 {
        String::new()
    } else if per_minute < 10.0 {
        format!("{per_minute:.1} rpm")
    } else if per_minute < 1000.0 {
        format!("{per_minute:.0} rpm")
    } else {
        format!("{:.1}k rpm", per_minute / 1000.0)
    }
}

/// Turn one measured call into an edge between nodes that are actually drawn.
///
/// Both ends must already be in the graph. Telemetry sees the whole mesh, and
/// most of what it reports is between namespaces nobody asked to look at;
/// adding nodes for those would grow the picture behind the reader's back every
/// time a service they do not care about took traffic.
fn observed_edge(
    sample: &crate::prometheus::Sample,
    graph: &TopologyGraphOut,
    schema: CallSchema,
) -> Option<(String, String, String, f64)> {
    let label = |key: &str| sample.labels.get(key).cloned().unwrap_or_default();
    let (source_ns, source, dest_ns, dest) = match schema {
        CallSchema::Istio => (
            label("source_workload_namespace"),
            label("source_workload"),
            label("destination_service_namespace"),
            label("destination_service_name"),
        ),
        CallSchema::Hubble => (
            label("source_namespace"),
            label("source_workload"),
            label("destination_namespace"),
            label("destination_workload"),
        ),
    };
    if source.is_empty() || dest.is_empty() {
        return None;
    }
    let nodes = &graph.nodes;
    // The workload's kind is not in the metric, so it is found rather than
    // assumed: Istio reports `source_workload` for a Deployment, a StatefulSet
    // and a DaemonSet alike.
    let from = nodes
        .iter()
        .find(|n| n.lane == Lane::Workload && n.name == source && n.namespace == source_ns)?;
    let to = match schema {
        CallSchema::Istio => nodes
            .iter()
            .find(|n| n.lane == Lane::Service && n.name == dest && n.namespace == dest_ns)?
            .id
            .clone(),
        CallSchema::Hubble => {
            let workload = nodes
                .iter()
                .find(|n| n.lane == Lane::Workload && n.name == dest && n.namespace == dest_ns)?;
            fronting_service(graph, &workload.id).unwrap_or_else(|| workload.id.clone())
        }
    };
    Some((from.id.clone(), to, rate_label(sample.value), sample.value))
}

/// The one Service that routes to a workload, if there is exactly one.
///
/// Hubble and a NetworkPolicy both speak of pods, not Services, and the graph
/// draws a call to the address its caller used. Where one Service fronts the
/// workload that address is not in doubt. Where none does the edge lands on
/// the workload itself, and where several do it does too — picking one would
/// be a guess drawn as a fact.
pub fn fronting_service(graph: &TopologyGraphOut, workload_id: &str) -> Option<String> {
    let mut found = graph
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Routes && e.to == workload_id && e.from.starts_with("Service/"))
        .map(|e| e.from.clone());
    let first = found.next()?;
    if found.next().is_some() {
        return None;
    }
    Some(first)
}

/// What a pod's socket table says, ready to be mapped onto the graph.
pub struct PodPeers {
    /// The node the pod belongs to — its workload, already in the graph.
    pub from: String,
    pub peers: Vec<crate::connections::Peer>,
}

/// Where every address in the cluster points, for turning a peer IP into a node.
///
/// A Service is preferred over the pod behind it, deliberately: `checkout`
/// talking to the `payments` Service is what a reader is looking for, and
/// `checkout` talking to `payments-7d9f4b8c6-x2mzp` is the same fact with the
/// useful half removed. Pod IPs are still mapped, for a connection to something
/// no Service fronts.
pub struct AddressBook {
    by_address: BTreeMap<String, String>,
}

impl AddressBook {
    /// Build from the objects the graph was built from.
    ///
    /// Services first, then pods — so a pod IP never wins over the Service IP
    /// of the thing in front of it.
    pub fn new(
        services: &[Service],
        pods: &[k8s_openapi::api::core::v1::Pod],
        nodes: &[TopologyNode],
    ) -> Self {
        let mut by_address = BTreeMap::new();
        for pod in pods {
            let Some(ip) = pod.status.as_ref().and_then(|s| s.pod_ip.clone()) else {
                continue;
            };
            let namespace = pod.metadata.namespace.clone().unwrap_or_default();
            // A pod is not drawn; the workload that made it is. The owner chain
            // is Pod -> ReplicaSet -> Deployment, so the pod's own owner name is
            // matched against every workload node rather than guessed at.
            let owner = pod
                .metadata
                .owner_references
                .as_ref()
                .and_then(|refs| refs.first())
                .map(|o| o.name.clone())
                .unwrap_or_default();
            if let Some(node) = nodes.iter().find(|n| {
                n.lane == Lane::Workload && n.namespace == namespace && owner.starts_with(&n.name)
            }) {
                by_address.insert(ip, node.id.clone());
            }
        }
        for service in services {
            let Some(ip) = service.spec.as_ref().and_then(|s| s.cluster_ip.clone()) else {
                continue;
            };
            if ip.is_empty() || ip == "None" {
                continue;
            }
            let name = service.metadata.name.clone().unwrap_or_default();
            let namespace = service.metadata.namespace.clone().unwrap_or_default();
            let id = node_id("Service", &namespace, &name);
            if nodes.iter().any(|n| n.id == id) {
                by_address.insert(ip, id);
            }
        }
        Self { by_address }
    }

    pub fn node_for(&self, address: &str) -> Option<&String> {
        self.by_address.get(address)
    }
}

/// Sockets held open, said the way a reader would.
///
/// Never a rate: a socket table is what is open right now, and a pool of five
/// idle connections looks exactly like five busy ones. Calling this `rpm` would
/// be a number the kernel never reported.
pub fn connection_label(count: u32) -> String {
    if count == 1 {
        "1 conn".to_string()
    } else {
        format!("{count} conns")
    }
}

/// Fold observed connections into a graph already built from the API.
///
/// A connection to something not drawn is dropped — a pod talks to the API
/// server, to CoreDNS and to whatever else the cluster runs, and putting a node
/// on the diagram for each would bury the namespace a reader asked about.
pub fn apply_connections(graph: &mut TopologyGraphOut, book: &AddressBook, pods: &[PodPeers]) {
    for pod in pods {
        for peer in &pod.peers {
            let Some(to) = book.node_for(&peer.address) else {
                continue;
            };
            // A pod's connections to its own workload are the replicas talking
            // among themselves, which is not a dependency.
            if to == &pod.from {
                continue;
            }
            push_edge_labelled(
                &mut graph.edges,
                pod.from.clone(),
                to.clone(),
                EdgeKind::Calls,
                Provenance::Observed,
                connection_label(peer.count),
                Some((f64::from(peer.count), Unit::Connections)),
            );
        }
    }
}

/// A workload reduced to what a selector can be judged against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkloadLabels {
    pub kind: &'static str,
    pub namespace: String,
    pub name: String,
    pub labels: BTreeMap<String, String>,
}

/// The pod-template labels of every workload, for the policy pass.
///
/// Taken BEFORE [`build_graph`] consumes the objects, and from the same
/// mappers it uses, so a policy is judged against exactly the labels the
/// Service selectors were.
pub fn workload_labels(
    deployments: &[Deployment],
    statefulsets: &[StatefulSet],
    daemonsets: &[DaemonSet],
) -> Vec<WorkloadLabels> {
    deployments
        .iter()
        .map(deployment_workload)
        .chain(statefulsets.iter().map(statefulset_workload))
        .chain(daemonsets.iter().map(daemonset_workload))
        .map(|w| WorkloadLabels {
            kind: w.kind,
            namespace: w.namespace,
            name: w.name,
            labels: w.labels,
        })
        .collect()
}

/// Whether a `LabelSelector` picks out a set of labels.
///
/// An EMPTY selector matches everything here — the opposite of a Service
/// selector, and exactly what Kubernetes means by `podSelector: {}` on a
/// policy. Callers that need a peer to be a specific thing check
/// [`names_something`] first rather than relying on this.
pub fn label_selector_matches(selector: &LabelSelector, labels: &BTreeMap<String, String>) -> bool {
    if let Some(wanted) = selector.match_labels.as_ref() {
        if !wanted.iter().all(|(k, v)| labels.get(k) == Some(v)) {
            return false;
        }
    }
    for expr in selector.match_expressions.iter().flatten() {
        let have = labels.get(&expr.key);
        let values: &[String] = expr.values.as_deref().unwrap_or(&[]);
        let ok = match expr.operator.as_str() {
            "In" => have.is_some_and(|v| values.contains(v)),
            "NotIn" => !have.is_some_and(|v| values.contains(v)),
            "Exists" => have.is_some(),
            "DoesNotExist" => have.is_none(),
            // An operator this code does not know cannot be said to match.
            _ => false,
        };
        if !ok {
            return false;
        }
    }
    true
}

/// Whether a selector singles anything out, as opposed to matching everything.
pub fn names_something(selector: &LabelSelector) -> bool {
    selector.match_labels.as_ref().is_some_and(|m| !m.is_empty())
        || selector.match_expressions.as_ref().is_some_and(|e| !e.is_empty())
}

/// The namespaces a peer's `namespaceSelector` reaches, out of those in view.
///
/// Judged against the one label every namespace is guaranteed to carry,
/// `kubernetes.io/metadata.name`, which is what a cross-namespace policy
/// almost always selects on — and which means no Namespace list is needed to
/// draw one. A selector on any other namespace label matches nothing here,
/// which drops an edge rather than inventing one.
fn peer_namespaces<'a>(
    selector: Option<&LabelSelector>,
    own: &'a str,
    in_view: &'a [String],
) -> Vec<&'a str> {
    let Some(selector) = selector else {
        return vec![own];
    };
    in_view
        .iter()
        .filter(|ns| {
            let labels =
                BTreeMap::from([("kubernetes.io/metadata.name".to_string(), (*ns).clone())]);
            label_selector_matches(selector, &labels)
        })
        .map(String::as_str)
        .collect()
}

/// Fold what NetworkPolicies permit into a graph already built from the API.
///
/// Intent, not traffic, and drawn as such — [`Provenance::Allowed`]. A rule
/// that names BOTH ends, a `podSelector` on the policy and a `podSelector` on
/// the peer, is a statement that these two things are meant to talk, and that
/// is worth an edge. A rule that names only one end is not: `from` left out,
/// an empty peer selector or a bare `namespaceSelector` all mean "anything in
/// there", and drawing that as a dependency would wire every pair in the
/// namespace together. An `ipBlock` is a CIDR, not a workload, and is skipped.
///
/// The policy's own selector MAY be empty — `podSelector: {}` is how a
/// namespace-wide allow-from is written, and its peers are still specific.
///
/// Where a config reference already declared the same pair, the declared edge
/// stands: a host in an environment variable is better evidence than a policy
/// that would let the call through. Telemetry later upgrades either.
pub fn apply_allowed(
    graph: &mut TopologyGraphOut,
    policies: &[NetworkPolicy],
    workloads: &[WorkloadLabels],
) {
    let in_view: Vec<String> = {
        let mut seen: Vec<String> = workloads.iter().map(|w| w.namespace.clone()).collect();
        seen.sort();
        seen.dedup();
        seen
    };
    let matching = |selector: &LabelSelector, namespaces: &[&str]| -> Vec<String> {
        workloads
            .iter()
            .filter(|w| {
                namespaces.contains(&w.namespace.as_str())
                    && label_selector_matches(selector, &w.labels)
            })
            .map(|w| node_id(w.kind, &w.namespace, &w.name))
            .collect()
    };

    let mut pairs: Vec<(String, String)> = Vec::new();
    for policy in policies {
        let Some(spec) = policy.spec.as_ref() else {
            continue;
        };
        let namespace = policy.metadata.namespace.clone().unwrap_or_default();
        // Absent and `{}` both mean every pod in the namespace.
        let subject = spec.pod_selector.clone().unwrap_or_default();
        let subjects = matching(&subject, &[namespace.as_str()]);
        if subjects.is_empty() {
            continue;
        }
        for rule in spec.ingress.iter().flatten() {
            for peer in rule.from.iter().flatten() {
                let Some(selector) = peer.pod_selector.as_ref().filter(|s| names_something(s)) else {
                    continue;
                };
                let namespaces = peer_namespaces(peer.namespace_selector.as_ref(), &namespace, &in_view);
                for from in matching(selector, &namespaces) {
                    for to in &subjects {
                        pairs.push((from.clone(), to.clone()));
                    }
                }
            }
        }
        for rule in spec.egress.iter().flatten() {
            for peer in rule.to.iter().flatten() {
                let Some(selector) = peer.pod_selector.as_ref().filter(|s| names_something(s)) else {
                    continue;
                };
                let namespaces = peer_namespaces(peer.namespace_selector.as_ref(), &namespace, &in_view);
                for to in matching(selector, &namespaces) {
                    for from in &subjects {
                        pairs.push((from.clone(), to.clone()));
                    }
                }
            }
        }
    }

    for (from, to) in pairs {
        // Pods of one workload talking among themselves is not a dependency.
        if from == to {
            continue;
        }
        let to = fronting_service(graph, &to).unwrap_or(to);
        push_edge(&mut graph.edges, from, to, EdgeKind::Calls, Provenance::Allowed);
    }
}

/// Fold measured traffic into a graph already built from the API.
///
/// Separated and public so the merge is testable without a cluster or a
/// Prometheus — it is the part that can be wrong, and the query is not.
pub fn apply_observed(
    graph: &mut TopologyGraphOut,
    samples: &[crate::prometheus::Sample],
    schema: CallSchema,
) {
    for sample in samples {
        let Some((from, to, detail, rps)) = observed_edge(sample, graph, schema) else {
            continue;
        };
        push_edge_labelled(
            &mut graph.edges,
            from,
            to,
            EdgeKind::Calls,
            Provenance::Observed,
            detail,
            Some((rps, Unit::Rps)),
        );
    }
}

/// Everything the graph is built from, accumulated across the namespaces asked
/// for.
#[derive(Default)]
struct Fetched {
    ingresses: Vec<Ingress>,
    services: Vec<Service>,
    deployments: Vec<Deployment>,
    statefulsets: Vec<StatefulSet>,
    daemonsets: Vec<DaemonSet>,
    replicasets: Vec<ReplicaSet>,
    config_maps: Vec<ConfigMap>,
    endpointslices: Vec<EndpointSlice>,
    policies: Vec<NetworkPolicy>,
}

/// `k8s.topologyGraph` — the route/service/workload/revision/dependency graph
/// of one or more namespaces.
pub fn topology_graph_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<TopologyGraphIn, TopologyGraphOut, _, _>(
        "k8s.topologyGraph",
        "graph the ingresses, services, workloads and dependencies of one or more namespaces",
        Annotations::READ_ONLY,
        move |input: TopologyGraphIn| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let mut all = Fetched::default();
                for ns in &input.namespaces {
                    // Concurrent within a namespace: seven independent lists, so
                    // the screen waits for the slowest rather than for the sum.
                    // Namespaces are walked in order because a reader picks a
                    // handful, not fifty, and a burst of 7n concurrent requests
                    // against one API server is a worse trade than the wait.
                    let (ingresses, services, deployments, statefulsets, daemonsets, replicasets, config_maps, endpointslices, policies) = tokio::try_join!(
                        list_of::<Ingress>(client.clone(), ns, "ingresses"),
                        list_of::<Service>(client.clone(), ns, "services"),
                        list_of::<Deployment>(client.clone(), ns, "deployments"),
                        list_of::<StatefulSet>(client.clone(), ns, "statefulsets"),
                        list_of::<DaemonSet>(client.clone(), ns, "daemonsets"),
                        list_of::<ReplicaSet>(client.clone(), ns, "replicasets"),
                        list_of::<ConfigMap>(client.clone(), ns, "configmaps"),
                        list_of::<EndpointSlice>(client.clone(), ns, "endpointslices"),
                        list_of::<NetworkPolicy>(client.clone(), ns, "networkpolicies"),
                    )?;
                    all.ingresses.extend(ingresses);
                    all.services.extend(services);
                    all.deployments.extend(deployments);
                    all.statefulsets.extend(statefulsets);
                    all.daemonsets.extend(daemonsets);
                    all.replicasets.extend(replicasets);
                    all.config_maps.extend(config_maps);
                    all.endpointslices.extend(endpointslices);
                    all.policies.extend(policies);
                }
                // Built once over everything, not once per namespace and merged:
                // a `checkout` that calls `payments-api.payments.svc` only
                // resolves to the real Service when both namespaces are in the
                // same pass.
                let all_services = all.services.clone();
                // Read off before the objects are handed over, from the same
                // mappers the graph uses.
                let labels = workload_labels(&all.deployments, &all.statefulsets, &all.daemonsets);
                let mut graph = build_graph(
                    all.ingresses,
                    all.services,
                    all.deployments,
                    all.statefulsets,
                    all.daemonsets,
                    all.replicasets,
                    all.config_maps,
                    all.endpointslices,
                );
                // Measured traffic, when there is somewhere to read it from.
                // A metrics backend that cannot be reached must NOT take the
                // graph down with it: the structural half is still true and
                // still worth drawing, so a failure here leaves the edges
                // declared rather than observed.
                // Before telemetry, so a measurement upgrades a permitted edge
                // the same way it upgrades a declared one.
                apply_allowed(&mut graph, &all.policies, &labels);
                if let Some(source) = input.prometheus.as_ref() {
                    for (schema, query) in CALL_GRAPH_QUERIES {
                        let Ok(samples) = crate::prometheus::instant_query(
                            &client,
                            &source.namespace,
                            &source.service,
                            source.port,
                            query,
                        )
                        .await
                        else {
                            continue;
                        };
                        // The first mesh that answers is the mesh this cluster
                        // runs; a query that returns no series is asked past.
                        if samples.is_empty() {
                            continue;
                        }
                        apply_observed(&mut graph, &samples, *schema);
                        break;
                    }
                }
                // Observed connections, when the reader asked for them.
                // Capped: this is an exec each, and a namespace of five
                // hundred pods is not a picture anyone wants at that price.
                if input.connections {
                    let pods = tokio::time::timeout(
                        request_timeout(),
                        list_of::<k8s_openapi::api::core::v1::Pod>(
                            client.clone(),
                            input.namespaces.first().map(String::as_str).unwrap_or(""),
                            "pods",
                        ),
                    )
                    .await
                    .unwrap_or_else(|_| Ok(Vec::new()))
                    .unwrap_or_default();
                    let book = AddressBook::new(&all_services, &pods, &graph.nodes);
                    let mut read = Vec::new();
                    for pod in pods.iter().take(CONNECTION_POD_LIMIT) {
                        let (Some(name), Some(namespace)) =
                            (pod.metadata.name.clone(), pod.metadata.namespace.clone())
                        else {
                            continue;
                        };
                        let owner = pod
                            .metadata
                            .owner_references
                            .as_ref()
                            .and_then(|r| r.first())
                            .map(|o| o.name.clone())
                            .unwrap_or_default();
                        let Some(from) = graph.nodes.iter().find(|n| {
                            n.lane == Lane::Workload
                                && n.namespace == namespace
                                && owner.starts_with(&n.name)
                        }) else {
                            continue;
                        };
                        // One pod refusing must not cost the rest: a single
                        // distroless sidecar would otherwise take the whole
                        // read with it.
                        if let Ok(peers) = crate::connections::read_pod_connections(
                            client.clone(),
                            &namespace,
                            &name,
                        )
                        .await
                        {
                            read.push(PodPeers { from: from.id.clone(), peers });
                        }
                    }
                    apply_connections(&mut graph, &book, &read);
                }
                Ok(graph)
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::apps::v1::{
        DaemonSetSpec, DaemonSetStatus, DeploymentSpec, DeploymentStatus, ReplicaSetSpec,
        ReplicaSetStatus, StatefulSetSpec, StatefulSetStatus,
    };
    use k8s_openapi::api::core::v1::{PodTemplateSpec, ServicePort, ServiceSpec};
    use k8s_openapi::api::networking::v1::{
        HTTPIngressPath, HTTPIngressRuleValue, IngressBackend, IngressRule, IngressServiceBackend,
        IngressSpec,
    };
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference;
    use kube::core::ObjectMeta;
    use std::path::PathBuf;

    fn labels(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn meta(name: &str) -> ObjectMeta {
        ObjectMeta {
            name: Some(name.into()),
            namespace: Some("checkout".into()),
            ..Default::default()
        }
    }

    fn template(pairs: &[(&str, &str)]) -> PodTemplateSpec {
        PodTemplateSpec {
            metadata: Some(ObjectMeta {
                labels: Some(labels(pairs)),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn deployment(name: &str, ready: i32, desired: i32, pairs: &[(&str, &str)]) -> Deployment {
        Deployment {
            metadata: meta(name),
            spec: Some(DeploymentSpec {
                replicas: Some(desired),
                template: template(pairs),
                ..Default::default()
            }),
            status: Some(DeploymentStatus {
                ready_replicas: Some(ready),
                ..Default::default()
            }),
        }
    }

    fn service(name: &str, selector: &[(&str, &str)], port: i32) -> Service {
        Service {
            metadata: meta(name),
            spec: Some(ServiceSpec {
                selector: if selector.is_empty() {
                    None
                } else {
                    Some(labels(selector))
                },
                ports: Some(vec![ServicePort {
                    port,
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn ingress(name: &str, host: &str, backends: &[&str]) -> Ingress {
        Ingress {
            metadata: meta(name),
            spec: Some(IngressSpec {
                rules: Some(vec![IngressRule {
                    host: Some(host.into()),
                    http: Some(HTTPIngressRuleValue {
                        paths: backends
                            .iter()
                            .map(|b| HTTPIngressPath {
                                backend: IngressBackend {
                                    service: Some(IngressServiceBackend {
                                        name: (*b).into(),
                                        ..Default::default()
                                    }),
                                    ..Default::default()
                                },
                                ..Default::default()
                            })
                            .collect(),
                    }),
                }]),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn replicaset(name: &str, owner: &str, revision: &str, ready: i32, desired: i32, current: i32) -> ReplicaSet {
        let mut metadata = meta(name);
        metadata.annotations = Some(labels(&[(REVISION_ANNOTATION, revision)]));
        metadata.owner_references = Some(vec![OwnerReference {
            kind: "Deployment".into(),
            name: owner.into(),
            ..Default::default()
        }]);
        ReplicaSet {
            metadata,
            spec: Some(ReplicaSetSpec {
                replicas: Some(desired),
                ..Default::default()
            }),
            status: Some(ReplicaSetStatus {
                replicas: current,
                ready_replicas: Some(ready),
                ..Default::default()
            }),
        }
    }

    fn find<'a>(g: &'a TopologyGraphOut, id: &str) -> &'a TopologyNode {
        g.nodes
            .iter()
            .find(|n| n.id == id)
            .unwrap_or_else(|| panic!("no node {id} in {:?}", g.nodes.iter().map(|n| &n.id).collect::<Vec<_>>()))
    }

    fn has_edge(g: &TopologyGraphOut, from: &str, to: &str, kind: EdgeKind) -> bool {
        g.edges
            .iter()
            .any(|e| e.from == from && e.to == to && e.kind == kind)
    }

    // ---- declared dependencies: what configuration names ----

    fn svc_names(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_cluster_dns_name_is_read_as_a_service_in_its_own_namespace() {
        // The unambiguous form, and the only one that can name another
        // namespace. Both the long and short spellings resolve the same.
        assert_eq!(
            references_in("http://payments-api.payments.svc.cluster.local:8080/authorize", &[]),
            vec![Reference::Service {
                name: "payments-api".into(),
                namespace: Some("payments".into())
            }],
        );
        assert_eq!(
            references_in("payments-api.payments.svc:8080", &[]),
            vec![Reference::Service {
                name: "payments-api".into(),
                namespace: Some("payments".into())
            }],
        );
    }

    #[test]
    fn a_bare_name_is_a_service_only_when_the_namespace_has_one() {
        // `kafka:9092` is a Service reference on a cluster that runs a Service
        // called kafka, and an external broker on one that does not. Guessing
        // either way without looking would be wrong half the time.
        assert_eq!(
            references_in("kafka:9092", &svc_names(&["kafka"])),
            vec![Reference::Service { name: "kafka".into(), namespace: None }],
        );
        assert_eq!(
            references_in("kafka:9092", &[]),
            vec![Reference::External { host: "kafka".into() }],
        );
    }

    #[test]
    fn a_dotted_host_that_is_not_cluster_dns_is_external() {
        assert_eq!(
            references_in("postgres://app:secret@db.prod.example.com:5432/orders", &[]),
            vec![Reference::External { host: "db.prod.example.com".into() }],
        );
        // Credentials are stripped with the rest of the authority — a password
        // must not become half a hostname.
        assert!(!references_in("postgres://app:secret@db.example.com/x", &[])
            .iter()
            .any(|r| matches!(r, Reference::External { host } if host.contains("secret"))));
    }

    #[test]
    fn a_dotted_word_with_no_scheme_or_port_is_not_a_host() {
        // Found on a real cluster: `storefront`'s command mentions
        // `nginx.conf`, and treating a dot as proof of a hostname put a
        // `nginx.conf` node in the dependency lane. Filenames, versions and
        // class names are all dotted and vastly outnumber bare hostnames in
        // configuration.
        for value in ["nginx.conf", "application.yaml", "1.2.3", "com.acme.Handler", "index.html"] {
            assert_eq!(references_in(value, &[]), vec![], "{value} should name no host");
        }
        // The same names ARE hosts once something says so.
        assert_eq!(
            references_in("https://nginx.conf", &[]),
            vec![Reference::External { host: "nginx.conf".into() }],
        );
    }

    #[test]
    fn ordinary_configuration_values_are_not_hosts() {
        // This runs over every env var and ConfigMap value in a namespace, most
        // of which are not addresses. A loose rule turns log levels and feature
        // flags into nodes on a diagram, which is worse than missing an edge.
        let services = svc_names(&["checkout"]);
        for value in [
            "production",
            "debug",
            "true",
            "LOG_LEVEL=info",
            "0.0.0.0",
            "timeout:30s",
            "key:value",
            "",
            "-",
            "...",
        ] {
            assert_eq!(references_in(value, &services), vec![], "{value} should name no host");
        }
    }

    #[test]
    fn the_same_host_named_twice_is_one_dependency() {
        // Config repeats itself — an argument and the ConfigMap behind it name
        // the same thing, and two containers name it separately.
        assert_eq!(
            references_in(
                "--upstream=http://payments.payments.svc.cluster.local --fallback=payments.payments.svc",
                &[]
            )
            .len(),
            1,
        );
    }

    #[test]
    fn a_workload_is_wired_to_what_its_environment_names() {
        let mut d = deployment("checkout-api", 1, 1, &[("app", "checkout-api")]);
        let container = k8s_openapi::api::core::v1::Container {
            name: "app".into(),
            env: Some(vec![
                k8s_openapi::api::core::v1::EnvVar {
                    name: "PAYMENTS_URL".into(),
                    value: Some("http://payments.checkout.svc.cluster.local:8080".into()),
                    ..Default::default()
                },
                k8s_openapi::api::core::v1::EnvVar {
                    name: "DATABASE_URL".into(),
                    value: Some("postgres://db.example.com:5432/orders".into()),
                    ..Default::default()
                },
            ]),
            ..Default::default()
        };
        d.spec.as_mut().unwrap().template.spec =
            Some(k8s_openapi::api::core::v1::PodSpec { containers: vec![container], ..Default::default() });

        let g = build_graph(
            vec![],
            vec![service("payments", &[("app", "payments")], 8080)],
            vec![d],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
        );

        // In-cluster: an edge to the Service that already exists.
        assert!(has_edge(
            &g,
            "Deployment/checkout/checkout-api",
            "Service/checkout/payments",
            EdgeKind::Calls
        ));
        // Out of cluster: a node of its own, in the external lane. This is the
        // `postgres-primary` the design shows, and nothing in the Kubernetes
        // API mentions it — only the configuration does.
        let db = find(&g, "External//db.example.com");
        assert_eq!(db.lane, Lane::External);
        assert!(has_edge(
            &g,
            "Deployment/checkout/checkout-api",
            "External//db.example.com",
            EdgeKind::Calls
        ));
        // And every one of them says how it is known.
        for e in g.edges.iter().filter(|e| e.kind == EdgeKind::Calls) {
            assert_eq!(e.provenance, Provenance::Declared, "a config reference is not observed traffic");
        }
    }

    #[test]
    fn a_configmap_reaches_only_the_workloads_that_read_it() {
        // Attributing a namespace's whole configuration to every workload in it
        // would wire them all to the same hosts, which is a diagram of nothing.
        let mut reader = deployment("reader", 1, 1, &[("app", "reader")]);
        reader.spec.as_mut().unwrap().template.spec = Some(k8s_openapi::api::core::v1::PodSpec {
            containers: vec![k8s_openapi::api::core::v1::Container {
                name: "app".into(),
                env_from: Some(vec![k8s_openapi::api::core::v1::EnvFromSource {
                    config_map_ref: Some(k8s_openapi::api::core::v1::ConfigMapEnvSource {
                        name: "shared".into(),
                        ..Default::default()
                    }),
                    ..Default::default()
                }]),
                ..Default::default()
            }],
            ..Default::default()
        });
        let bystander = deployment("bystander", 1, 1, &[("app", "bystander")]);

        let cm = ConfigMap {
            metadata: meta("shared"),
            data: Some(labels(&[("BROKER", "kafka.example.com:9092")])),
            ..Default::default()
        };
        let g = build_graph(vec![], vec![], vec![reader, bystander], vec![], vec![], vec![], vec![cm], vec![]);

        assert!(has_edge(
            &g,
            "Deployment/checkout/reader",
            "External//kafka.example.com",
            EdgeKind::Calls
        ));
        assert!(!g
            .edges
            .iter()
            .any(|e| e.from == "Deployment/checkout/bystander"));
    }

    fn slice(service: &str, endpoints: &[(&str, Option<&str>)]) -> EndpointSlice {
        use k8s_openapi::api::core::v1::ObjectReference;
        use k8s_openapi::api::discovery::v1::Endpoint;
        let mut metadata = meta(&format!("{service}-abc"));
        metadata.labels = Some(labels(&[("kubernetes.io/service-name", service)]));
        EndpointSlice {
            metadata,
            address_type: "IPv4".into(),
            endpoints: endpoints
                .iter()
                .map(|(address, target_kind)| Endpoint {
                    addresses: vec![(*address).to_string()],
                    target_ref: target_kind.map(|kind| ObjectReference {
                        kind: Some(kind.to_string()),
                        ..Default::default()
                    }),
                    ..Default::default()
                })
                .collect(),
            ports: None,
        }
    }

    #[test]
    fn a_service_backed_by_something_other_than_pods_names_an_external_dependency() {
        // The usual way to point at a managed database from inside the cluster:
        // a Service with hand-managed Endpoints. It looks exactly like a Service
        // whose selector matches nothing — both have no workload behind them —
        // and the only thing that tells them apart is the missing pod
        // `targetRef` on the endpoint.
        let g = build_graph(
            vec![],
            vec![service("orders-db", &[], 5432)],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![slice("orders-db", &[("10.0.9.4", None)])],
        );
        assert!(has_edge(
            &g,
            "Service/checkout/orders-db",
            "External//10.0.9.4",
            EdgeKind::Calls
        ));
        assert_eq!(find(&g, "External//10.0.9.4").lane, Lane::External);
    }

    #[test]
    fn pod_backed_endpoints_are_not_external() {
        // The Service already reaches its pods through the selector. Drawing
        // their IPs as well would put the cluster's own internals in the
        // dependency lane.
        let g = build_graph(
            vec![],
            vec![service("checkout", &[("app", "checkout")], 80)],
            vec![deployment("checkout", 1, 1, &[("app", "checkout")])],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![slice("checkout", &[("10.1.2.3", Some("Pod"))])],
        );
        assert!(!g.nodes.iter().any(|n| n.lane == Lane::External), "{:?}", g.nodes);
    }

    #[test]
    fn an_external_name_service_is_a_declared_dependency() {
        let mut svc = service("orders-db", &[], 5432);
        svc.spec.as_mut().unwrap().external_name = Some("db.example.com".into());
        let g = build_graph(vec![], vec![svc], vec![], vec![], vec![], vec![], vec![], vec![]);
        assert!(has_edge(
            &g,
            "Service/checkout/orders-db",
            "External//db.example.com",
            EdgeKind::Calls
        ));
    }

    #[test]
    fn ownership_and_routing_are_topology_not_inference() {
        // The provenance split is the point of the field: a selector join and a
        // string found in an environment variable must never read alike.
        let g = build_graph(
            vec![ingress("web", "checkout.acme.io", &["checkout-api"])],
            vec![service("checkout-api", &[("app", "checkout-api")], 80)],
            vec![deployment("checkout-api", 1, 1, &[("app", "checkout-api")])],
            vec![],
            vec![],
            vec![replicaset("checkout-api-1", "checkout-api", "1", 1, 1, 1)],
            vec![],
            vec![],
        );
        for e in &g.edges {
            assert_eq!(e.provenance, Provenance::Topology, "{e:?}");
        }
    }

    // ---- observed traffic: what telemetry measured ----

    fn sample(source_ns: &str, source: &str, dest_ns: &str, dest: &str, per_second: f64) -> crate::prometheus::Sample {
        crate::prometheus::Sample {
            labels: labels(&[
                ("source_workload_namespace", source_ns),
                ("source_workload", source),
                ("destination_service_namespace", dest_ns),
                ("destination_service_name", dest),
            ]),
            value: per_second,
        }
    }

    fn demo_graph() -> TopologyGraphOut {
        build_graph(
            vec![],
            vec![service("payments", &[("app", "payments")], 8080)],
            vec![
                deployment("checkout", 1, 1, &[("app", "checkout")]),
                deployment("payments", 1, 1, &[("app", "payments")]),
            ],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
        )
    }

    // ---- allowed: what a NetworkPolicy would let through ----

    fn selector(pairs: &[(&str, &str)]) -> LabelSelector {
        LabelSelector {
            match_labels: Some(labels(pairs)),
            ..Default::default()
        }
    }

    fn policy_peer(pods: &[(&str, &str)], namespace: Option<&str>) -> k8s_openapi::api::networking::v1::NetworkPolicyPeer {
        k8s_openapi::api::networking::v1::NetworkPolicyPeer {
            pod_selector: Some(selector(pods)),
            namespace_selector: namespace.map(|ns| selector(&[("kubernetes.io/metadata.name", ns)])),
            ..Default::default()
        }
    }

    /// A policy on `subject`'s pods in `namespace`, allowing ingress from and
    /// egress to the given peers.
    fn policy(
        namespace: &str,
        subject: &[(&str, &str)],
        from: Vec<k8s_openapi::api::networking::v1::NetworkPolicyPeer>,
        to: Vec<k8s_openapi::api::networking::v1::NetworkPolicyPeer>,
    ) -> NetworkPolicy {
        use k8s_openapi::api::networking::v1 as net;
        let mut metadata = meta("policy");
        metadata.namespace = Some(namespace.into());
        NetworkPolicy {
            metadata,
            spec: Some(net::NetworkPolicySpec {
                pod_selector: Some(selector(subject)),
                ingress: (!from.is_empty()).then(|| {
                    vec![net::NetworkPolicyIngressRule {
                        from: Some(from),
                        ..Default::default()
                    }]
                }),
                egress: (!to.is_empty()).then(|| {
                    vec![net::NetworkPolicyEgressRule {
                        to: Some(to),
                        ..Default::default()
                    }]
                }),
                ..Default::default()
            }),
        }
    }

    fn demo_labels() -> Vec<WorkloadLabels> {
        workload_labels(
            &[
                deployment("checkout", 1, 1, &[("app", "checkout")]),
                deployment("payments", 1, 1, &[("app", "payments")]),
            ],
            &[],
            &[],
        )
    }

    #[test]
    fn an_ingress_rule_naming_both_ends_is_an_allowed_call_to_the_fronting_service() {
        let mut g = demo_graph();
        apply_allowed(
            &mut g,
            &[policy("checkout", &[("app", "payments")], vec![policy_peer(&[("app", "checkout")], None)], vec![])],
            &demo_labels(),
        );
        // Drawn to the Service, which is the address the caller would use,
        // not to the pods the policy is literally about.
        let edge = g.edges.iter().find(|e| e.kind == EdgeKind::Calls).expect("an allowed call");
        assert_eq!(edge.from, "Deployment/checkout/checkout");
        assert_eq!(edge.to, "Service/checkout/payments");
        assert_eq!(edge.provenance, Provenance::Allowed);
        assert_eq!(edge.detail, "");
        assert_eq!(edge.weight, None);
    }

    #[test]
    fn an_egress_rule_points_the_other_way() {
        let mut g = demo_graph();
        apply_allowed(
            &mut g,
            &[policy("checkout", &[("app", "checkout")], vec![], vec![policy_peer(&[("app", "payments")], None)])],
            &demo_labels(),
        );
        assert!(has_edge(&g, "Deployment/checkout/checkout", "Service/checkout/payments", EdgeKind::Calls));
    }

    #[test]
    fn a_peer_that_names_nothing_specific_draws_nothing() {
        // `podSelector: {}` on a peer is every pod in the namespace. Wiring
        // that up would join every pair in the namespace.
        let mut g = demo_graph();
        let mut anyone = policy_peer(&[], None);
        anyone.pod_selector = Some(LabelSelector::default());
        let mut by_cidr = policy_peer(&[], None);
        by_cidr.pod_selector = None;
        by_cidr.ip_block = Some(k8s_openapi::api::networking::v1::IPBlock {
            cidr: "10.0.0.0/8".into(),
            except: None,
        });
        apply_allowed(
            &mut g,
            &[policy("checkout", &[("app", "payments")], vec![anyone, by_cidr], vec![])],
            &demo_labels(),
        );
        assert!(g.edges.iter().all(|e| e.kind != EdgeKind::Calls), "{:?}", g.edges);
    }

    #[test]
    fn a_cross_namespace_peer_is_found_by_the_namespace_name_label() {
        // No Namespace list is fetched; the selector is judged against the
        // one label every namespace is guaranteed to carry.
        let mut ledger = deployment("ledger", 1, 1, &[("app", "ledger")]);
        ledger.metadata.namespace = Some("payments".into());
        let checkout = deployment("checkout", 1, 1, &[("app", "checkout")]);
        let labels = workload_labels(&[checkout.clone(), ledger.clone()], &[], &[]);
        let mut g = build_graph(vec![], vec![], vec![checkout, ledger], vec![], vec![], vec![], vec![], vec![]);
        apply_allowed(
            &mut g,
            &[policy("payments", &[("app", "ledger")], vec![policy_peer(&[("app", "checkout")], Some("checkout"))], vec![])],
            &labels,
        );
        // Nothing fronts `ledger`, so the edge lands on the workload itself
        // rather than on a Service that was guessed.
        assert!(has_edge(&g, "Deployment/checkout/checkout", "Deployment/payments/ledger", EdgeKind::Calls));

        // And a selector on any OTHER namespace label matches nothing here.
        let mut g = build_graph(
            vec![],
            vec![],
            vec![deployment("checkout", 1, 1, &[("app", "checkout")]), {
                let mut l = deployment("ledger", 1, 1, &[("app", "ledger")]);
                l.metadata.namespace = Some("payments".into());
                l
            }],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
        );
        let mut by_team = policy_peer(&[("app", "checkout")], None);
        by_team.namespace_selector = Some(selector(&[("team", "shop")]));
        apply_allowed(&mut g, &[policy("payments", &[("app", "ledger")], vec![by_team], vec![])], &labels);
        assert!(g.edges.iter().all(|e| e.kind != EdgeKind::Calls));
    }

    #[test]
    fn a_declared_edge_is_not_downgraded_to_allowed() {
        // A host in an environment variable says the workload was built to
        // call this; a policy says only that it could. The better evidence
        // stands, and there is still one edge.
        let mut g = demo_graph();
        push_edge(
            &mut g.edges,
            "Deployment/checkout/checkout".into(),
            "Service/checkout/payments".into(),
            EdgeKind::Calls,
            Provenance::Declared,
        );
        apply_allowed(
            &mut g,
            &[policy("checkout", &[("app", "payments")], vec![policy_peer(&[("app", "checkout")], None)], vec![])],
            &demo_labels(),
        );
        let calls: Vec<&TopologyEdge> = g.edges.iter().filter(|e| e.kind == EdgeKind::Calls).collect();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].provenance, Provenance::Declared);
    }

    #[test]
    fn a_selector_expression_is_honoured() {
        let ok = labels(&[("app", "checkout"), ("tier", "web")]);
        let sel = LabelSelector {
            match_expressions: Some(vec![
                k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelectorRequirement {
                    key: "tier".into(),
                    operator: "In".into(),
                    values: Some(vec!["web".into(), "api".into()]),
                },
                k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelectorRequirement {
                    key: "canary".into(),
                    operator: "DoesNotExist".into(),
                    values: None,
                },
            ]),
            ..Default::default()
        };
        assert!(label_selector_matches(&sel, &ok));
        assert!(!label_selector_matches(&sel, &labels(&[("app", "checkout"), ("tier", "db")])));
        assert!(!label_selector_matches(&sel, &labels(&[("tier", "web"), ("canary", "yes")])));
        // Empty matches everything — the opposite of a Service selector.
        assert!(label_selector_matches(&LabelSelector::default(), &ok));
        assert!(!names_something(&LabelSelector::default()));
    }

    // ---- observed, from Hubble: pod to pod, below the Service ----

    fn hubble_sample(source_ns: &str, source: &str, dest_ns: &str, dest: &str, per_second: f64) -> crate::prometheus::Sample {
        crate::prometheus::Sample {
            labels: labels(&[
                ("source_namespace", source_ns),
                ("source_workload", source),
                ("destination_namespace", dest_ns),
                ("destination_workload", dest),
            ]),
            value: per_second,
        }
    }

    #[test]
    fn a_hubble_request_lands_on_the_service_fronting_the_destination_workload() {
        // Hubble reports the pod that answered; the graph draws the address
        // that was called. With one Service fronting the workload there is no
        // doubt which that was.
        let mut g = demo_graph();
        apply_observed(&mut g, &[hubble_sample("checkout", "checkout", "checkout", "payments", 2.0)], CallSchema::Hubble);
        let edge = g.edges.iter().find(|e| e.kind == EdgeKind::Calls).expect("an observed call");
        assert_eq!(edge.from, "Deployment/checkout/checkout");
        assert_eq!(edge.to, "Service/checkout/payments");
        assert_eq!(edge.provenance, Provenance::Observed);
        assert_eq!(edge.detail, "120 rpm");
        assert_eq!(edge.unit, Some(Unit::Rps));
    }

    #[test]
    fn a_hubble_request_to_an_unfronted_workload_lands_on_the_workload() {
        let mut g = build_graph(
            vec![],
            vec![],
            vec![
                deployment("checkout", 1, 1, &[("app", "checkout")]),
                deployment("payments", 1, 1, &[("app", "payments")]),
            ],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
        );
        apply_observed(&mut g, &[hubble_sample("checkout", "checkout", "checkout", "payments", 2.0)], CallSchema::Hubble);
        assert!(has_edge(&g, "Deployment/checkout/checkout", "Deployment/checkout/payments", EdgeKind::Calls));
    }

    #[test]
    fn istio_labels_mean_nothing_to_the_hubble_schema_and_vice_versa() {
        // Each query's labels are read under its own schema only; a sample
        // from the wrong one is dropped rather than half-matched.
        let mut g = demo_graph();
        apply_observed(&mut g, &[sample("checkout", "checkout", "checkout", "payments", 1.0)], CallSchema::Hubble);
        assert!(g.edges.iter().all(|e| e.kind != EdgeKind::Calls));
        apply_observed(&mut g, &[hubble_sample("checkout", "checkout", "checkout", "payments", 1.0)], CallSchema::Istio);
        assert!(g.edges.iter().all(|e| e.kind != EdgeKind::Calls));
    }

    // ---- observed connections: what the kernel has open ----

    fn pod(name: &str, ip: &str, owner: &str) -> k8s_openapi::api::core::v1::Pod {
        use k8s_openapi::api::core::v1::PodStatus;
        use k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference;
        let mut metadata = meta(name);
        metadata.owner_references = Some(vec![OwnerReference {
            kind: "ReplicaSet".into(),
            name: owner.into(),
            ..Default::default()
        }]);
        k8s_openapi::api::core::v1::Pod {
            metadata,
            status: Some(PodStatus {
                pod_ip: Some(ip.into()),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn peer(address: &str, port: u16, count: u32) -> crate::connections::Peer {
        crate::connections::Peer {
            address: address.into(),
            port,
            count,
        }
    }

    #[test]
    fn a_connection_count_is_never_called_a_rate() {
        // A socket table says what is open now. Five idle pooled connections
        // look exactly like five busy ones, so calling this rpm would be a
        // number the kernel never reported.
        assert_eq!(connection_label(1), "1 conn");
        assert_eq!(connection_label(5), "5 conns");
    }

    #[test]
    fn a_connection_to_a_service_ip_becomes_an_edge_to_that_service() {
        let mut svc = service("payments", &[("app", "payments")], 8080);
        svc.spec.as_mut().unwrap().cluster_ip = Some("10.96.1.5".into());
        let g_services = vec![svc];
        let pods = vec![pod("checkout-abc", "10.1.0.9", "checkout-7d9f")];
        let mut g = build_graph(
            vec![],
            g_services.clone(),
            vec![
                deployment("checkout", 1, 1, &[("app", "checkout")]),
                deployment("payments", 1, 1, &[("app", "payments")]),
            ],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
        );
        let book = AddressBook::new(&g_services, &pods, &g.nodes);
        apply_connections(
            &mut g,
            &book,
            &[PodPeers {
                from: "Deployment/checkout/checkout".into(),
                peers: vec![peer("10.96.1.5", 8080, 5)],
            }],
        );
        let edge = g.edges.iter().find(|e| e.kind == EdgeKind::Calls).expect("a call");
        assert_eq!(edge.to, "Service/checkout/payments");
        assert_eq!(edge.provenance, Provenance::Observed);
        assert_eq!(edge.detail, "5 conns");
        // The label is for a reader; the weight is for the screen, which draws
        // this line as thick as the number is large. Never `Rps`: a socket
        // table says what is OPEN, and five idle pooled connections look
        // exactly like five busy ones.
        assert_eq!(edge.weight, Some(5.0));
        assert_eq!(edge.unit, Some(Unit::Connections));
    }

    #[test]
    fn a_service_wins_over_the_pod_behind_it() {
        // `checkout talks to the payments Service` is what a reader wants;
        // `checkout talks to payments-7d9f4b8c6-x2mzp` is the same fact with
        // the useful half removed.
        let mut svc = service("payments", &[("app", "payments")], 8080);
        svc.spec.as_mut().unwrap().cluster_ip = Some("10.96.1.5".into());
        let services = vec![svc];
        let pods = vec![pod("payments-abc", "10.96.1.5", "payments-1")];
        let g = build_graph(
            vec![],
            services.clone(),
            vec![deployment("payments", 1, 1, &[("app", "payments")])],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
        );
        let book = AddressBook::new(&services, &pods, &g.nodes);
        assert_eq!(
            book.node_for("10.96.1.5").map(String::as_str),
            Some("Service/checkout/payments"),
        );
    }

    #[test]
    fn replicas_talking_among_themselves_are_not_a_dependency() {
        let pods = vec![pod("checkout-abc", "10.1.0.9", "checkout-7d9f")];
        let mut g = build_graph(
            vec![],
            vec![],
            vec![deployment("checkout", 2, 2, &[("app", "checkout")])],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
        );
        let book = AddressBook::new(&[], &pods, &g.nodes);
        apply_connections(
            &mut g,
            &book,
            &[PodPeers {
                from: "Deployment/checkout/checkout".into(),
                peers: vec![peer("10.1.0.9", 8080, 2)],
            }],
        );
        assert!(g.edges.iter().all(|e| e.kind != EdgeKind::Calls), "{:?}", g.edges);
    }

    #[test]
    fn a_connection_to_something_not_drawn_is_dropped() {
        // Every pod talks to the API server and to CoreDNS. Putting a node on
        // the diagram for each would bury the namespace a reader asked about.
        let mut g = demo_graph();
        let book = AddressBook::new(&[], &[], &g.nodes);
        apply_connections(
            &mut g,
            &book,
            &[PodPeers {
                from: "Deployment/checkout/checkout".into(),
                peers: vec![peer("10.96.0.1", 443, 3)],
            }],
        );
        assert!(g.edges.iter().all(|e| e.kind != EdgeKind::Calls));
    }

    #[test]
    fn a_rate_reads_per_minute_because_per_second_is_not_what_anyone_reads() {
        assert_eq!(rate_label(0.6867), "41 rpm");
        assert_eq!(rate_label(0.05), "3.0 rpm");
        assert_eq!(rate_label(686.7), "41.2k rpm");
        // Below a tenth of a request a minute there is no number worth
        // claiming — the edge still draws, it just says nothing.
        assert_eq!(rate_label(0.0), "");
        assert_eq!(rate_label(0.001), "");
    }

    #[test]
    fn measured_traffic_becomes_an_observed_edge_with_its_rate() {
        let mut g = demo_graph();
        apply_observed(&mut g, &[sample("checkout", "checkout", "checkout", "payments", 0.6867)], CallSchema::Istio);
        let edge = g
            .edges
            .iter()
            .find(|e| e.kind == EdgeKind::Calls)
            .expect("an observed call");
        assert_eq!(edge.from, "Deployment/checkout/checkout");
        assert_eq!(edge.to, "Service/checkout/payments");
        assert_eq!(edge.provenance, Provenance::Observed);
        assert_eq!(edge.detail, "41 rpm");
        // Per SECOND, as PromQL returned it, rather than the per-minute figure
        // the label is rounded to. The screen scales thicknesses off this, and
        // recovering `0.6867` from the string `41 rpm` is not possible.
        assert_eq!(edge.weight, Some(0.6867));
        assert_eq!(edge.unit, Some(Unit::Rps));
    }

    #[test]
    fn a_measurement_upgrades_the_declared_edge_rather_than_joining_it() {
        // Telemetry seeing the call configuration already declared is ONE
        // dependency now known better. Two lines between the same pair would
        // say the opposite.
        let mut d = deployment("checkout", 1, 1, &[("app", "checkout")]);
        d.spec.as_mut().unwrap().template.spec = Some(k8s_openapi::api::core::v1::PodSpec {
            containers: vec![k8s_openapi::api::core::v1::Container {
                name: "app".into(),
                env: Some(vec![k8s_openapi::api::core::v1::EnvVar {
                    name: "PAYMENTS_URL".into(),
                    value: Some("http://payments.checkout.svc.cluster.local:8080".into()),
                    ..Default::default()
                }]),
                ..Default::default()
            }],
            ..Default::default()
        });
        let mut g = build_graph(
            vec![],
            vec![service("payments", &[("app", "payments")], 8080)],
            vec![d, deployment("payments", 1, 1, &[("app", "payments")])],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
        );
        let declared = g.edges.iter().filter(|e| e.kind == EdgeKind::Calls).count();
        assert_eq!(declared, 1, "the config reference alone");

        apply_observed(&mut g, &[sample("checkout", "checkout", "checkout", "payments", 1.0)], CallSchema::Istio);
        let calls: Vec<&TopologyEdge> = g.edges.iter().filter(|e| e.kind == EdgeKind::Calls).collect();
        assert_eq!(calls.len(), 1, "still one dependency: {calls:?}");
        assert_eq!(calls[0].provenance, Provenance::Observed);
        assert_eq!(calls[0].detail, "60 rpm");
    }

    #[test]
    fn traffic_between_things_not_drawn_is_left_out() {
        // Telemetry sees the whole mesh, and most of it is between namespaces
        // nobody asked to look at. Adding nodes for those would grow the
        // picture behind the reader every time some unrelated service took
        // traffic.
        let mut g = demo_graph();
        apply_observed(
            &mut g,
            &[
                sample("other", "stranger", "checkout", "payments", 5.0),
                sample("checkout", "checkout", "elsewhere", "unknown-svc", 5.0),
            ],
            CallSchema::Istio,
        );
        assert!(g.edges.iter().all(|e| e.kind != EdgeKind::Calls), "{:?}", g.edges);
    }

    #[test]
    fn a_sample_missing_either_end_is_ignored() {
        let mut g = demo_graph();
        let mut half = sample("checkout", "checkout", "checkout", "payments", 5.0);
        half.labels.remove("destination_service_name");
        apply_observed(&mut g, &[half], CallSchema::Istio);
        assert!(g.edges.iter().all(|e| e.kind != EdgeKind::Calls));
    }

    #[test]
    fn capability_has_expected_id() {
        let cap = topology_graph_capability(ClientCache::new(PathBuf::from("/x")));
        assert_eq!(cap.id, "k8s.topologyGraph");
        assert!(cap.annotations.read_only);
    }

    #[test]
    fn health_reads_ready_over_desired() {
        assert_eq!(health_of(3, 3), Health::Ok);
        assert_eq!(health_of(9, 12), Health::Degraded);
        assert_eq!(health_of(0, 3), Health::Failing);
        // Scaled to zero is absent, not well. Calling it Ok would paint a
        // deliberately-stopped workload the same green as a serving one.
        assert_eq!(health_of(0, 0), Health::Unknown);
        // More ready than desired happens mid-scale-down, and is not a fault.
        assert_eq!(health_of(4, 3), Health::Ok);
    }

    #[test]
    fn a_selector_matches_a_superset_of_labels() {
        let pods = labels(&[("app", "checkout"), ("pod-template-hash", "7d9f4b8c6")]);
        // The Service names one label; the pods carry it plus the hash the
        // Deployment controller adds. Equality would miss this, which is every
        // Deployment there is.
        assert!(selector_matches(&labels(&[("app", "checkout")]), &pods));
        assert!(!selector_matches(&labels(&[("app", "payments")]), &pods));
        // Both halves must match, not either.
        assert!(!selector_matches(
            &labels(&[("app", "checkout"), ("tier", "web")]),
            &pods
        ));
    }

    #[test]
    fn an_empty_selector_matches_nothing() {
        // Kubernetes reads this as "no selector — someone else manages the
        // Endpoints". Reading it as "everything" would wire every Service in
        // the namespace to every workload in it.
        assert!(!selector_matches(
            &BTreeMap::new(),
            &labels(&[("app", "checkout")])
        ));
    }

    #[test]
    fn ingress_backends_take_every_rule_and_the_default_once() {
        let mut ing = ingress("web", "checkout.acme.io", &["checkout-web", "checkout-api"]);
        ing.spec.as_mut().unwrap().default_backend = Some(IngressBackend {
            service: Some(IngressServiceBackend {
                name: "checkout-web".into(),
                ..Default::default()
            }),
            ..Default::default()
        });
        // The default is named first and the rule repeating it does not add a
        // second edge.
        assert_eq!(ingress_backends(&ing), vec!["checkout-web", "checkout-api"]);
        assert!(ingress_backends(&Ingress::default()).is_empty());
    }

    #[test]
    fn a_replicaset_is_live_while_it_still_holds_pods() {
        assert!(replicaset_is_live(3, 3));
        // Winding down: wants nothing, still serving. This is the state a
        // reader mid-rollout is looking for.
        assert!(replicaset_is_live(0, 2));
        // History: a Deployment keeps ten of these.
        assert!(!replicaset_is_live(0, 0));
    }

    #[test]
    fn the_chain_from_ingress_to_revision_is_joined() {
        let g = build_graph(
            vec![ingress("web", "checkout.acme.io", &["checkout-api"])],
            vec![service("checkout-api", &[("app", "checkout-api")], 80)],
            vec![deployment("checkout-api", 9, 12, &[("app", "checkout-api")])],
            vec![],
            vec![],
            vec![replicaset("checkout-api-7d9f", "checkout-api", "119", 0, 3, 3)],
            vec![],
            vec![],
        );

        assert!(has_edge(
            &g,
            "Ingress/checkout/web",
            "Service/checkout/checkout-api",
            EdgeKind::Routes
        ));
        assert!(has_edge(
            &g,
            "Service/checkout/checkout-api",
            "Deployment/checkout/checkout-api",
            EdgeKind::Routes
        ));
        assert!(has_edge(
            &g,
            "Deployment/checkout/checkout-api",
            "ReplicaSet/checkout/checkout-api-7d9f",
            EdgeKind::Owns
        ));

        let deploy = find(&g, "Deployment/checkout/checkout-api");
        assert_eq!(deploy.lane, Lane::Workload);
        assert_eq!(deploy.detail, "9/12");
        assert_eq!(deploy.health, Health::Degraded);

        // A ReplicaSet is drawn as its revision: the generated name is a hash
        // nobody reads, and the id keeps it anyway.
        let rs = find(&g, "ReplicaSet/checkout/checkout-api-7d9f");
        assert_eq!(rs.name, "rev 119");
        assert_eq!(rs.health, Health::Failing);
    }

    #[test]
    fn a_service_takes_the_health_of_the_worst_thing_behind_it() {
        let g = build_graph(
            vec![],
            vec![service("both", &[("app", "checkout")], 80)],
            vec![
                deployment("healthy", 3, 3, &[("app", "checkout"), ("role", "a")]),
                deployment("broken", 0, 2, &[("app", "checkout"), ("role", "b")]),
            ],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
        );
        assert_eq!(find(&g, "Service/checkout/both").health, Health::Failing);
    }

    #[test]
    fn a_service_with_no_selector_gets_no_workload_edge() {
        // An ExternalName Service, or one whose Endpoints are managed by hand.
        // It genuinely fronts no workload here, so an edge would be invented.
        let g = build_graph(
            vec![],
            vec![service("external", &[], 443)],
            vec![deployment("checkout-api", 3, 3, &[("app", "checkout-api")])],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
        );
        assert!(g.edges.is_empty());
        assert_eq!(find(&g, "Service/checkout/external").health, Health::Unknown);
    }

    #[test]
    fn an_ingress_rule_naming_a_missing_service_draws_no_edge() {
        // A broken Ingress. Inventing the node would draw traffic arriving
        // somewhere it cannot.
        let g = build_graph(
            vec![ingress("web", "checkout.acme.io", &["gone"])],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
        );
        assert!(g.edges.is_empty());
        assert_eq!(g.nodes.len(), 1);
    }

    #[test]
    fn dead_revisions_are_left_out() {
        let g = build_graph(
            vec![],
            vec![],
            vec![deployment("checkout-api", 3, 3, &[("app", "checkout-api")])],
            vec![],
            vec![],
            vec![
                replicaset("live", "checkout-api", "119", 3, 3, 3),
                replicaset("dead", "checkout-api", "109", 0, 0, 0),
            ],
            vec![],
            vec![],
        );
        assert!(g.nodes.iter().any(|n| n.id == "ReplicaSet/checkout/live"));
        assert!(!g.nodes.iter().any(|n| n.id == "ReplicaSet/checkout/dead"));
        // And no owns-edge to a node that is not drawn.
        assert!(!has_edge(
            &g,
            "Deployment/checkout/checkout-api",
            "ReplicaSet/checkout/dead",
            EdgeKind::Owns
        ));
    }

    #[test]
    fn statefulsets_and_daemonsets_stand_in_the_workload_lane_too() {
        let sts = StatefulSet {
            metadata: meta("cart-session-store"),
            spec: Some(StatefulSetSpec {
                replicas: Some(3),
                template: template(&[("app", "cart")]),
                ..Default::default()
            }),
            status: Some(StatefulSetStatus {
                ready_replicas: Some(3),
                ..Default::default()
            }),
        };
        // A DaemonSet has no spec.replicas: how many it wants is how many nodes
        // the scheduler picked, which only status knows.
        let ds = DaemonSet {
            metadata: meta("ingress-nginx"),
            spec: Some(DaemonSetSpec {
                template: template(&[("app", "ingress-nginx")]),
                ..Default::default()
            }),
            status: Some(DaemonSetStatus {
                desired_number_scheduled: 42,
                number_ready: 42,
                ..Default::default()
            }),
        };
        let g = build_graph(vec![], vec![], vec![], vec![sts], vec![ds], vec![], vec![], vec![]);

        let sts_node = find(&g, "StatefulSet/checkout/cart-session-store");
        assert_eq!(sts_node.lane, Lane::Workload);
        assert_eq!(sts_node.detail, "3/3");

        let ds_node = find(&g, "DaemonSet/checkout/ingress-nginx");
        assert_eq!(ds_node.detail, "42/42");
        assert_eq!(ds_node.health, Health::Ok);
    }
}
