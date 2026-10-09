//! Workload-listing capabilities backed by kube-rs: `k8s.listNamespaces` and
//! `k8s.listPods` for a connected context.

use std::collections::BTreeMap;
use std::sync::Arc;

use k8s_openapi::api::core::v1::{ContainerStateTerminated, Namespace, Pod};
use kube::api::ListParams;
use kube::Api;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use srelens_capability::{Annotations, Capability, CapabilityError};

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListNamespacesIn {
    pub context: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct NamespaceSummary {
    pub name: String,
    pub phase: String,
    pub labels: BTreeMap<String, String>,
    pub age: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListNamespacesOut {
    /// Kept for the namespace selector and for compatibility with existing
    /// consumers of this capability.
    pub namespaces: Vec<String>,
    /// Rich rows for the Namespaces resource list.
    pub summaries: Vec<NamespaceSummary>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListPodsIn {
    pub context: String,
    pub namespace: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PodSummary {
    pub name: String,
    pub namespace: String,
    pub phase: String,
    pub ready: String,
    pub restarts: i32,
    pub node: String,
    /// `creationTimestamp` (RFC 3339), so the frontend can derive a LIVE age.
    /// `age` below is rendered once, when this summary is built, and a summary
    /// is only rebuilt when a watch event arrives for the object — so it goes
    /// stale (#405). Prefer this; `age` stays for callers that have no clock.
    pub created: Option<String>,
    pub age: String,
    /// Raw ISO 8601 timestamp `age` derives from, so UIs can recompute the
    /// age live at render time. Empty when the resource carries none.
    #[serde(rename = "createdAt")]
    pub created_at: String,
    /// Container image(s) the pod runs, e.g. `acme/checkout-api:118a7e`.
    /// A pod with several containers joins them as `"img-a, img-b"`; a pod
    /// with no containers (or no status yet) is `""`.
    pub image: String,
    /// Why a container is waiting, when one is — `CrashLoopBackOff`,
    /// `ImagePullBackOff`, `CreateContainerConfigError`, `ContainerCreating`.
    ///
    /// `phase` alone cannot tell a healthy pod from a crash-looping one: a pod
    /// whose only container is restarting in a back-off loop still reports
    /// `Running`, so a list that reads nothing but the phase draws it green.
    /// This carries the fact the phase omits; what it *means* — which reasons
    /// are a failure and which are a pod on its way up — is decided once, in
    /// `podStatus` in `@srelens/core`, not here and not twice.
    ///
    /// The first non-empty waiting reason across the pod's containers, or `""`
    /// when none is waiting.
    #[serde(rename = "waitingReason")]
    pub waiting_reason: String,
    /// The pod's STATUS as `kubectl get pods` prints it: `CrashLoopBackOff`,
    /// `Init:0/2`, `OOMKilled`, `Terminating`, `Completed`, and the phase only
    /// when nothing more specific applies.
    ///
    /// Unlike `waitingReason` this reads init containers, terminated
    /// containers and the deletion timestamp, the same way kubectl does (see
    /// `kubectl_status`). It is kubectl's word, not a verdict: which words
    /// are failures is still the reader's call.
    pub status: String,
    /// Pod IP address from `status.podIP`.
    #[serde(rename = "podIp", default)]
    pub pod_ip: String,
    /// CPU requested in millicores across all containers
    #[serde(rename = "cpuReqMillicores", default)]
    pub cpu_req_millicores: i64,
    /// CPU limit in millicores across all containers
    #[serde(rename = "cpuLimMillicores", default)]
    pub cpu_lim_millicores: i64,
    /// Memory requested in MiB across all containers
    #[serde(rename = "memReqMiB", default)]
    pub mem_req_mib: i64,
    /// Memory limit in MiB across all containers
    #[serde(rename = "memLimMiB", default)]
    pub mem_lim_mib: i64,
    /// Whether EVERY container sets a CPU limit, so `cpuLimMillicores` is the
    /// pod's whole ceiling and not the sum of the containers that have one.
    ///
    /// A pod with one limited container and one unlimited one has no CPU
    /// ceiling at all, and its partial sum is a number it can exceed without
    /// anything being wrong. A reader measuring usage against a limit needs to
    /// know which of the two it was handed (srelens/srelens#864). False for a
    /// pod with no containers.
    #[serde(rename = "cpuLimAll", default)]
    pub cpu_lim_all: bool,
    /// As `cpuLimAll`, for memory — where the ceiling is an OOM kill.
    #[serde(rename = "memLimAll", default)]
    pub mem_lim_all: bool,
    /// Each container's own state, in the order a reader meets them: app
    /// containers as the spec lists them, then init containers.
    ///
    /// `ready` above says how many are ready and not which one is not, or why.
    /// A list that draws one mark per container needs each one's state
    /// (srelens/srelens#878).
    #[serde(default)]
    pub containers: Vec<PodContainer>,
}

/// One container of a pod, as far as a list row needs it.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PodContainer {
    pub name: String,
    /// `app`, `init`, or `sidecar` — an init container that restarts always,
    /// and so runs for the pod's whole life beside the app containers.
    pub kind: String,
    /// `running`, `waiting`, `terminated`, or `unknown` when the kubelet has
    /// reported nothing for it yet. What each *means* is `containerVerdict`'s
    /// to say, in `@srelens/core`, not this struct's.
    pub state: String,
    /// The waiting or terminated reason (`CrashLoopBackOff`, `OOMKilled`,
    /// `Completed`), or `""`.
    pub reason: String,
    /// The exit code of a terminated container.
    #[serde(rename = "exitCode")]
    pub exit_code: Option<i32>,
    pub ready: bool,
    pub restarts: i32,
    /// The image the spec asks for.
    pub image: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListPodsOut {
    pub pods: Vec<PodSummary>,
}

fn handler_err(e: impl ToString) -> CapabilityError {
    CapabilityError::Handler(e.to_string())
}

pub(crate) fn summarise_namespace(namespace: Namespace) -> NamespaceSummary {
    let phase = namespace
        .status
        .as_ref()
        .and_then(|status| status.phase.clone())
        .unwrap_or_else(|| "Unknown".into());
    NamespaceSummary {
        name: namespace.metadata.name.clone().unwrap_or_default(),
        phase,
        labels: namespace.metadata.labels.clone().unwrap_or_default(),
        age: crate::humanize_age(namespace.metadata.creation_timestamp.as_ref()),
    }
}

/// `k8s.listNamespaces` — list namespace names and summaries in a connected context.
pub fn list_namespaces_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ListNamespacesIn, ListNamespacesOut, _, _>(
        "k8s.listNamespaces",
        "list namespaces in a connected kube context",
        Annotations::READ_ONLY,
        move |input: ListNamespacesIn| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let api: Api<Namespace> = Api::all(client);
                let list =
                    tokio::time::timeout(request_timeout(), api.list(&ListParams::default()))
                        .await
                        .map_err(|_| CapabilityError::Handler("list namespaces timed out".into()))?
                        .map_err(handler_err)?;
                let summaries: Vec<_> = list
                    .items
                    .into_iter()
                    .map(summarise_namespace)
                    // Preserve the old selector contract: Kubernetes objects
                    // without a name never became namespace options.
                    .filter(|summary| !summary.name.is_empty())
                    .collect();
                let namespaces = summaries
                    .iter()
                    .map(|summary| summary.name.clone())
                    .collect();
                Ok(ListNamespacesOut {
                    namespaces,
                    summaries,
                })
            }
        },
    )
}

/// The STATUS `kubectl get pods` prints for `pod`: kubectl's `printPod`
/// (`pkg/printers/internalversion/printers.go`), rule for rule, so a row says
/// what a terminal beside it says.
///
/// In kubectl's order, each later rule overriding an earlier one:
///
/// 1. The pod's own `status.reason` (`Evicted`), else `phase`. A pod held by a
///    scheduling gate is `SchedulingGated`.
/// 2. The first init container that has not finished: `Init:<reason>` when it
///    failed or is stuck, `Init:<finished>/<total>` while it works. A started
///    native sidecar (`restartPolicy: Always`) counts as finished here, since it
///    runs for the pod's whole life.
/// 3. Unless an init container spoke and the pod is not yet `Initialized`, the
///    first regular container that is waiting or terminated, by its reason
///    (`ExitCode:<n>`/`Signal:<n>` when it gives none). A `Completed` word does
///    not always stand: beside a running container it is `Running` when the
///    pod is Ready; otherwise it is the first container's failure that exited
///    non-zero, if any, else `NotReady` when a container still runs.
/// 4. A pod being deleted is `Terminating`, or `Unknown` when its node was
///    lost, unless it had already finished.
fn kubectl_status(pod: &Pod, phase: &str) -> String {
    let status = pod.status.as_ref();
    let conditions = status
        .and_then(|s| s.conditions.as_deref())
        .unwrap_or_default();
    let condition_true = |type_: &str| {
        conditions
            .iter()
            .any(|c| c.type_ == type_ && c.status == "True")
    };
    let pod_reason = status.and_then(|s| s.reason.as_deref()).unwrap_or_default();

    let mut reason = if pod_reason.is_empty() {
        phase.to_string()
    } else {
        pod_reason.to_string()
    };
    if conditions
        .iter()
        .any(|c| c.type_ == "PodScheduled" && c.reason.as_deref() == Some("SchedulingGated"))
    {
        reason = "SchedulingGated".to_string();
    }

    let init_specs = pod
        .spec
        .as_ref()
        .and_then(|s| s.init_containers.as_deref())
        .unwrap_or_default();
    let is_sidecar = |name: &str| {
        init_specs
            .iter()
            .any(|c| c.name == name && c.restart_policy.as_deref() == Some("Always"))
    };
    let init_statuses = status
        .and_then(|s| s.init_container_statuses.as_deref())
        .unwrap_or_default();
    let mut initializing = false;
    for (i, c) in init_statuses.iter().enumerate() {
        let state = c.state.as_ref();
        let terminated = state.and_then(|s| s.terminated.as_ref());
        if terminated.is_some_and(|t| t.exit_code == 0)
            || (is_sidecar(&c.name) && c.started == Some(true))
        {
            continue;
        }
        let waiting = state
            .and_then(|s| s.waiting.as_ref())
            .and_then(|w| w.reason.as_deref())
            .filter(|r| !r.is_empty() && *r != "PodInitializing");
        reason = match (terminated, waiting) {
            (Some(t), _) => format!("Init:{}", terminated_word(t)),
            (None, Some(w)) => format!("Init:{w}"),
            (None, None) => format!("Init:{i}/{}", init_specs.len()),
        };
        initializing = true;
        break;
    }

    if !initializing || condition_true("Initialized") {
        let container_statuses = status
            .and_then(|s| s.container_statuses.as_deref())
            .unwrap_or_default();
        let mut has_running = false;
        let mut error_reason = None;
        // Last to first, overwriting, so the first container with something
        // to say has the last word.
        for c in container_statuses.iter().rev() {
            let state = c.state.as_ref();
            let waiting = state
                .and_then(|s| s.waiting.as_ref())
                .and_then(|w| w.reason.as_deref())
                .filter(|r| !r.is_empty());
            if let Some(w) = waiting {
                reason = w.to_string();
            } else if let Some(t) = state.and_then(|s| s.terminated.as_ref()) {
                reason = terminated_word(t);
                if t.exit_code != 0 {
                    error_reason = Some(reason.clone());
                }
            } else if c.ready && state.is_some_and(|s| s.running.is_some()) {
                has_running = true;
            }
        }
        if reason == "Completed" {
            if has_running && condition_true("Ready") {
                reason = "Running".to_string();
            } else if let Some(e) = error_reason {
                reason = e;
            } else if has_running {
                reason = "NotReady".to_string();
            }
        }
    }

    if pod.metadata.deletion_timestamp.is_some() {
        if pod_reason == "NodeLost" {
            reason = "Unknown".to_string();
        } else if phase != "Succeeded" && phase != "Failed" {
            reason = "Terminating".to_string();
        }
    }
    reason
}

/// A terminated container in kubectl's words: its reason, else the signal
/// that killed it, else its exit code.
fn terminated_word(t: &ContainerStateTerminated) -> String {
    match t.reason.as_deref().filter(|r| !r.is_empty()) {
        Some(r) => r.to_string(),
        None => match t.signal.filter(|s| *s != 0) {
            Some(s) => format!("Signal:{s}"),
            None => format!("ExitCode:{}", t.exit_code),
        },
    }
}

/// Summarise a pod's ready count, total restarts, and phase.
///
/// Public so tests outside this crate can build the exact row the pods watch
/// emits from a pod as the API server returns it.
pub fn summarise_pod(pod: Pod) -> PodSummary {
    let name = pod.metadata.name.clone().unwrap_or_default();
    let namespace = pod.metadata.namespace.clone().unwrap_or_default();
    let node = pod
        .spec
        .as_ref()
        .and_then(|s| s.node_name.clone())
        .unwrap_or_default();
    let phase = pod
        .status
        .as_ref()
        .and_then(|s| s.phase.clone())
        .unwrap_or_else(|| "Unknown".into());

    let statuses = pod
        .status
        .as_ref()
        .and_then(|s| s.container_statuses.as_ref());
    let (ready_count, restarts) = match statuses {
        Some(cs) => (
            cs.iter().filter(|c| c.ready).count(),
            cs.iter().map(|c| c.restart_count).sum(),
        ),
        None => (0, 0),
    };
    let total = statuses.map(|cs| cs.len()).unwrap_or(0);
    // One row shows one pod, so several containers are joined into a single
    // string — same shape as the multi-value `ports` summaries elsewhere in
    // this crate (e.g. ingresses' "80, 443"). Init containers are excluded:
    // they run to completion before the pod is "running" the images that
    // matter for this column.
    let image = statuses
        .map(|cs| {
            cs.iter()
                .map(|c| c.image.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();

    // Reported raw, in container order, exactly as `image` is: the first
    // container with something to say. Init containers are excluded for the
    // same reason they are excluded there. An empty reason says nothing, and
    // kubectl skips it too, so `status` and this name the same container.
    let waiting_reason = statuses
        .and_then(|cs| {
            cs.iter().find_map(|c| {
                c.state
                    .as_ref()?
                    .waiting
                    .as_ref()?
                    .reason
                    .clone()
                    .filter(|r| !r.is_empty())
            })
        })
        .unwrap_or_default();

    let status = kubectl_status(&pod, &phase);

    let pod_ip = pod
        .status
        .as_ref()
        .and_then(|s| s.pod_ip.clone())
        .unwrap_or_default();

    let (mut req_cpu, mut lim_cpu, mut req_mem, mut lim_mem) = (0i64, 0i64, 0i64, 0i64);
    // The containers that run for as long as the pod does, which are the ones
    // metrics-server's figure for the pod adds up: the app containers, and the
    // init containers that restart always — native sidecars. An ordinary init
    // container has finished before the pod is running and uses nothing.
    //
    // A sidecar left out of this would be usage with no bound to set it
    // against: 200m of app under a 500m limit plus 400m of an unlimited
    // sidecar reads as 120% of a ceiling the pod does not have.
    let running: Vec<&k8s_openapi::api::core::v1::Container> = pod
        .spec
        .as_ref()
        .map(|spec| {
            spec.containers
                .iter()
                .chain(
                    spec.init_containers
                        .iter()
                        .flatten()
                        .filter(|c| c.restart_policy.as_deref() == Some("Always")),
                )
                .collect()
        })
        .unwrap_or_default();
    let limited = |resource: &str| {
        // `all` over nothing is true; a pod with nothing in it has no ceiling.
        !running.is_empty()
            && running.iter().all(|c| {
                c.resources
                    .as_ref()
                    .and_then(|r| r.limits.as_ref())
                    .is_some_and(|limits| limits.contains_key(resource))
            })
    };
    let (cpu_lim_all, mem_lim_all) = (limited("cpu"), limited("memory"));
    for c in &running {
        if let Some(resources) = &c.resources {
            if let Some(reqs) = &resources.requests {
                if let Some(q) = reqs.get("cpu") {
                    req_cpu += crate::metrics::cpu_millicores(&q.0);
                }
                if let Some(q) = reqs.get("memory") {
                    req_mem += crate::metrics::mem_mib(&q.0);
                }
            }
            if let Some(lims) = &resources.limits {
                if let Some(q) = lims.get("cpu") {
                    lim_cpu += crate::metrics::cpu_millicores(&q.0);
                }
                if let Some(q) = lims.get("memory") {
                    lim_mem += crate::metrics::mem_mib(&q.0);
                }
            }
        }
    }

    let containers = pod_containers(&pod);

    PodSummary {
        name,
        namespace,
        phase,
        ready: format!("{ready_count}/{total}"),
        restarts,
        node,
        created: crate::creation_rfc3339(pod.metadata.creation_timestamp.as_ref()),
        age: crate::humanize_age(pod.metadata.creation_timestamp.as_ref()),
        created_at: crate::creation_timestamp_iso(pod.metadata.creation_timestamp.as_ref()),
        image,
        waiting_reason,
        status,
        pod_ip,
        cpu_req_millicores: req_cpu,
        cpu_lim_millicores: lim_cpu,
        mem_req_mib: req_mem,
        mem_lim_mib: lim_mem,
        cpu_lim_all,
        mem_lim_all,
        containers,
    }
}

/// Every container the spec names, each with what the kubelet last said of
/// it. From the spec rather than the statuses, so a container the kubelet has
/// not reported on yet is still listed — as `unknown` — instead of missing.
fn pod_containers(pod: &Pod) -> Vec<PodContainer> {
    let Some(spec) = pod.spec.as_ref() else {
        return Vec::new();
    };
    let status = pod.status.as_ref();
    let app_statuses = status
        .and_then(|s| s.container_statuses.as_deref())
        .unwrap_or_default();
    let init_statuses = status
        .and_then(|s| s.init_container_statuses.as_deref())
        .unwrap_or_default();
    let one = |c: &k8s_openapi::api::core::v1::Container,
               kind: &str,
               statuses: &[k8s_openapi::api::core::v1::ContainerStatus]| {
        let reported = statuses.iter().find(|s| s.name == c.name);
        let state = reported.and_then(|s| s.state.as_ref());
        // Running first, then terminated, then waiting: the kubelet sets one
        // of the three, and a state with none of them set says nothing.
        let running = state.is_some_and(|s| s.running.is_some());
        let terminated = state.and_then(|s| s.terminated.as_ref());
        let waiting = state.and_then(|s| s.waiting.as_ref());
        let (word, reason, exit_code) = match (running, terminated, waiting) {
            (true, _, _) => ("running", String::new(), None),
            (false, Some(t), _) => ("terminated", t.reason.clone().unwrap_or_default(), Some(t.exit_code)),
            (false, None, Some(w)) => ("waiting", w.reason.clone().unwrap_or_default(), None),
            (false, None, None) => ("unknown", String::new(), None),
        };
        PodContainer {
            name: c.name.clone(),
            kind: kind.to_string(),
            state: word.to_string(),
            reason,
            exit_code,
            ready: reported.is_some_and(|s| s.ready),
            restarts: reported.map(|s| s.restart_count).unwrap_or(0),
            image: c.image.clone().unwrap_or_default(),
        }
    };
    let apps = spec.containers.iter().map(|c| one(c, "app", app_statuses));
    let inits = spec.init_containers.iter().flatten().map(|c| {
        let kind = if c.restart_policy.as_deref() == Some("Always") { "sidecar" } else { "init" };
        one(c, kind, init_statuses)
    });
    apps.chain(inits).collect()
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PodsOnNodeIn {
    pub context: String,
    pub node: String,
}

fn pods_on_node_params(node: &str) -> Result<ListParams, CapabilityError> {
    if node.trim().is_empty() {
        return Err(CapabilityError::InvalidInput(
            "node must not be empty".into(),
        ));
    }
    Ok(ListParams::default().fields(&format!("spec.nodeName={node}")))
}

/// `k8s.podsOnNode` — list pods scheduled on one node, across namespaces.
pub fn pods_on_node_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<PodsOnNodeIn, ListPodsOut, _, _>(
        "k8s.podsOnNode",
        "list pods scheduled on a node across all namespaces",
        Annotations::READ_ONLY,
        move |input: PodsOnNodeIn| {
            let cache = cache.clone();
            async move {
                let params = pods_on_node_params(&input.node)?;
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                // A node is cluster-scoped and can host pods from every
                // namespace, so this query must use the all-namespaces API.
                let api: Api<Pod> = Api::all(client);
                let list = tokio::time::timeout(request_timeout(), api.list(&params))
                    .await
                    .map_err(|_| CapabilityError::Handler("list pods on node timed out".into()))?
                    .map_err(handler_err)?;
                let pods = list.items.into_iter().map(summarise_pod).collect();
                Ok(ListPodsOut { pods })
            }
        },
    )
}

/// `k8s.listPods` — list pods in a namespace of a connected context.
pub fn list_pods_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ListPodsIn, ListPodsOut, _, _>(
        "k8s.listPods",
        "list pods in a namespace of a connected kube context",
        Annotations::READ_ONLY,
        move |input: ListPodsIn| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let api: Api<Pod> = crate::scoped_api(client, &input.namespace);
                let list = tokio::time::timeout(request_timeout(), api.list(&ListParams::default()))
                    .await
                    .map_err(|_| CapabilityError::Handler("list pods timed out".into()))?
                    .map_err(handler_err)?;
                let pods = list.items.into_iter().map(summarise_pod).collect();
                Ok(ListPodsOut { pods })
            }
        },
    )
}

/// One `matchExpressions` entry of a Kubernetes `LabelSelector`.
///
/// Spelled as the API spells it — `operator` is one of `In`, `NotIn`,
/// `Exists`, `DoesNotExist`, **case-sensitively**, so a workload's
/// `spec.selector.matchExpressions` can be handed over untouched. Anything
/// else is refused rather than guessed at: a selector rendered wrongly returns
/// the wrong pods and says nothing about it.
#[derive(Debug, Clone, PartialEq, Deserialize, JsonSchema)]
pub struct LabelSelectorRequirement {
    /// The label key the requirement is about, e.g. `app.kubernetes.io/name`.
    pub key: String,
    /// `In`, `NotIn`, `Exists`, or `DoesNotExist`.
    pub operator: String,
    /// The set for `In`/`NotIn`; empty (or absent) for the existence
    /// operators, which refuse a set.
    #[serde(default)]
    pub values: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PodsForSelectorIn {
    pub context: String,
    pub namespace: String,
    /// Equality label selector as a map, e.g. `{ "app": "web" }` — a
    /// `LabelSelector`'s `matchLabels` half.
    pub selector: std::collections::BTreeMap<String, String>,
    /// The set-based half, a `LabelSelector`'s `matchExpressions`. Optional:
    /// a caller with only equality labels omits it and nothing changes.
    ///
    /// The two halves are a **conjunction** — a pod matches when it satisfies
    /// every entry of `selector` *and* every requirement here — which is what
    /// makes sending only `matchLabels` for a workload that has both a bug
    /// rather than an approximation: it queries a strictly wider set than the
    /// workload owns.
    #[serde(default, rename = "matchExpressions")]
    pub match_expressions: Vec<LabelSelectorRequirement>,
}

/// Build a kube equality label selector string ("k1=v1,k2=v2") from a map.
pub(crate) fn label_selector(selector: &std::collections::BTreeMap<String, String>) -> String {
    selector
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(",")
}

/// Refuse a key or value that carries the selector grammar's own punctuation.
///
/// The query goes to the API server as a *string*, so a comma or a paren in a
/// value is read as syntax and silently widens what comes back. Kubernetes'
/// own label syntax allows none of these characters in a key or a value, so
/// nothing legitimate is turned away — `/`, `.`, `-` and `_` all pass.
fn ensure_selector_safe(part: &str, what: &str) -> Result<(), CapabilityError> {
    if part.is_empty() {
        return Err(CapabilityError::InvalidInput(format!(
            "label selector {what} must not be empty"
        )));
    }
    ensure_no_selector_syntax(part, what)
}

/// The same refusal for a `matchLabels` VALUE, which may legitimately be empty.
///
/// `app=` selects pods carrying `app` with an empty value, so the value half
/// cannot inherit [`ensure_selector_safe`]'s non-empty rule — only its
/// punctuation rule. Split out rather than parameterised so neither caller can
/// pass the wrong flag.
fn ensure_selector_value_safe(part: &str, what: &str) -> Result<(), CapabilityError> {
    ensure_no_selector_syntax(part, what)
}

/// Refuse the selector grammar's own punctuation, wherever it appears.
fn ensure_no_selector_syntax(part: &str, what: &str) -> Result<(), CapabilityError> {
    if part
        .chars()
        .any(|c| c.is_whitespace() || matches!(c, ',' | '(' | ')' | '!' | '=' | '<' | '>'))
    {
        return Err(CapabilityError::InvalidInput(format!(
            "label selector {what} {part:?} contains selector syntax"
        )));
    }
    Ok(())
}

/// The label-selector string to ask the API server for, or `None` when the
/// request must answer with no pods without asking at all.
///
/// `None` covers the two ways a selector has nothing to ask:
///
/// - **It constrains nothing.** An empty selector matches *every* pod in the
///   namespace, which is never what a caller asking for a workload's pods
///   wants; returning nothing is the deliberate answer. A `NotIn ()` term
///   constrains nothing either — every pod is outside the empty set — so it
///   drops out, and a selector left with no terms lands here.
/// - **It can never match.** `In ()` is membership of the empty set, false for
///   every pod, so the whole conjunction is false and no query is needed.
///
/// A selector made only of `matchExpressions` is *not* one of those cases: it
/// constrains plenty, and answering it with no pods would report a workload as
/// having none when it has every one of them.
pub(crate) fn selector_query(
    labels: &std::collections::BTreeMap<String, String>,
    expressions: &[LabelSelectorRequirement],
) -> Result<Option<String>, CapabilityError> {
    let mut terms: Vec<String> = Vec::new();
    if !labels.is_empty() {
        // The equality half is gated too, not only the expressions below. It
        // used to go straight into `format!("{k}={v}")`, so `{"app":
        // "web,tier=cache"}` rendered as two conjoined terms and the caller
        // silently got a narrower set than it asked for. Label selectors are
        // conjunction-only, so an injected term can only NARROW — which on a
        // "which pods does this workload own" answer is an under-report, and
        // the same class of wrong answer the expression gate exists to stop.
        for (k, v) in labels {
            ensure_selector_safe(k, "key")?;
            ensure_selector_value_safe(v, "value")?;
        }
        terms.push(label_selector(labels));
    }

    for req in expressions {
        ensure_selector_safe(&req.key, "key")?;
        let key = &req.key;
        match req.operator.as_str() {
            "In" | "NotIn" => {
                if req.values.is_empty() {
                    // `In ()` is unsatisfiable, so the conjunction is; `NotIn ()`
                    // is satisfied by everything, so the term simply goes.
                    if req.operator == "In" {
                        return Ok(None);
                    }
                    continue;
                }
                for v in &req.values {
                    ensure_selector_safe(v, "value")?;
                }
                let set = req.values.join(",");
                let op = if req.operator == "In" { "in" } else { "notin" };
                terms.push(format!("{key} {op} ({set})"));
            }
            "Exists" | "DoesNotExist" => {
                if !req.values.is_empty() {
                    return Err(CapabilityError::InvalidInput(format!(
                        "label selector operator {} takes no values",
                        req.operator
                    )));
                }
                terms.push(if req.operator == "Exists" {
                    key.to_string()
                } else {
                    format!("!{key}")
                });
            }
            other => {
                return Err(CapabilityError::InvalidInput(format!(
                    "unknown label selector operator {other:?}; expected In, NotIn, Exists, or DoesNotExist"
                )))
            }
        }
    }

    Ok(if terms.is_empty() {
        None
    } else {
        Some(terms.join(","))
    })
}

/// `k8s.podsForSelector` — pods in a namespace matching a label selector, used
/// to show the pods a workload (Deployment/StatefulSet) manages.
pub fn pods_for_selector_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<PodsForSelectorIn, ListPodsOut, _, _>(
        "k8s.podsForSelector",
        "list pods matching a label selector (a workload's managed pods)",
        Annotations::READ_ONLY,
        move |input: PodsForSelectorIn| {
            let cache = cache.clone();
            async move {
                // Decided before a client is touched: a selector with nothing to
                // ask (see `selector_query`) returns nothing rather than every pod
                // in the namespace, and a malformed one is refused rather than
                // rendered into a query that quietly matches the wrong pods.
                let Some(query) = selector_query(&input.selector, &input.match_expressions)? else {
                    return Ok(ListPodsOut { pods: vec![] });
                };
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let api: Api<Pod> = crate::scoped_api(client, &input.namespace);
                let params = ListParams::default().labels(&query);
                let list = tokio::time::timeout(request_timeout(), api.list(&params))
                    .await
                    .map_err(|_| CapabilityError::Handler("list pods timed out".into()))?
                    .map_err(handler_err)?;
                let pods = list.items.into_iter().map(summarise_pod).collect();
                Ok(ListPodsOut { pods })
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::{
        ContainerState, ContainerStateRunning, ContainerStateWaiting, ContainerStatus,
        NamespaceStatus, PodSpec, PodStatus,
    };

    #[test]
    fn summarises_namespace_phase_labels_and_age_without_a_timestamp() {
        let labels = BTreeMap::from([
            ("env".to_string(), "prod".to_string()),
            ("team".to_string(), "sre".to_string()),
        ]);
        let namespace = Namespace {
            metadata: kube::core::ObjectMeta {
                name: Some("monitoring".into()),
                labels: Some(labels.clone()),
                ..Default::default()
            },
            status: Some(NamespaceStatus {
                phase: Some("Active".into()),
                ..Default::default()
            }),
            ..Default::default()
        };

        assert_eq!(
            summarise_namespace(namespace),
            NamespaceSummary {
                name: "monitoring".into(),
                phase: "Active".into(),
                labels,
                age: "-".into(),
            }
        );
    }

    #[test]
    fn summarises_an_unsettled_namespace_without_inventing_metadata() {
        let summary = summarise_namespace(Namespace::default());
        assert_eq!(summary.name, "");
        assert_eq!(summary.phase, "Unknown");
        assert!(summary.labels.is_empty());
    }

    #[test]
    fn capabilities_have_expected_ids() {
        use std::path::PathBuf;
        let cache = ClientCache::new(PathBuf::from("/x"));
        assert_eq!(
            list_namespaces_capability(cache.clone()).id,
            "k8s.listNamespaces"
        );
        assert_eq!(list_pods_capability(cache.clone()).id, "k8s.listPods");
        assert_eq!(
            pods_for_selector_capability(cache.clone()).id,
            "k8s.podsForSelector"
        );
        assert_eq!(pods_on_node_capability(cache).id, "k8s.podsOnNode");
    }

    #[test]
    fn pods_on_node_uses_the_supported_node_field_selector() {
        let params = pods_on_node_params("worker-2").unwrap();
        assert_eq!(
            params.field_selector.as_deref(),
            Some("spec.nodeName=worker-2")
        );
        assert!(pods_on_node_params("").is_err());
        assert!(pods_on_node_params("   ").is_err());
    }

    /// A pod whose containers set the given `(cpu, memory)` limits, each
    /// `None` for a container that sets none. Built from JSON, as the API
    /// server would send it.
    fn pod_with_limits(containers: &[(Option<&str>, Option<&str>)]) -> Pod {
        let containers: Vec<serde_json::Value> = containers
            .iter()
            .enumerate()
            .map(|(i, (cpu, memory))| {
                let mut limits = serde_json::Map::new();
                if let Some(cpu) = cpu {
                    limits.insert("cpu".into(), serde_json::json!(cpu));
                }
                if let Some(memory) = memory {
                    limits.insert("memory".into(), serde_json::json!(memory));
                }
                serde_json::json!({ "name": format!("c{i}"), "image": "img", "resources": { "limits": limits } })
            })
            .collect();
        serde_json::from_value(serde_json::json!({
            "metadata": { "name": "p" },
            "spec": { "containers": containers }
        }))
        .unwrap()
    }

    #[test]
    fn a_limit_is_the_pods_ceiling_only_when_every_container_sets_one() {
        let all = summarise_pod(pod_with_limits(&[
            (Some("500m"), Some("256Mi")),
            (Some("250m"), Some("128Mi")),
        ]));
        assert_eq!((all.cpu_lim_millicores, all.mem_lim_mib), (750, 384));
        assert!(all.cpu_lim_all && all.mem_lim_all);

        // One container with no CPU limit: the pod has no CPU ceiling, and
        // the 500m the other one set is not it. Memory is still whole.
        let partial = summarise_pod(pod_with_limits(&[
            (Some("500m"), Some("256Mi")),
            (None, Some("128Mi")),
        ]));
        assert_eq!(partial.cpu_lim_millicores, 500);
        assert!(!partial.cpu_lim_all);
        assert!(partial.mem_lim_all);
    }

    #[test]
    fn a_pod_with_no_limits_or_no_containers_has_no_ceiling() {
        let none = summarise_pod(pod_with_limits(&[(None, None)]));
        assert!(!none.cpu_lim_all && !none.mem_lim_all);
        // `all` over nothing is true; a pod with nothing in it has no ceiling.
        let empty = summarise_pod(pod_with_limits(&[]));
        assert!(!empty.cpu_lim_all && !empty.mem_lim_all);
        let no_spec = summarise_pod(Pod::default());
        assert!(!no_spec.cpu_lim_all && !no_spec.mem_lim_all);
    }

    /// A pod from JSON, as the API server would send it.
    fn pod_json(value: serde_json::Value) -> Pod {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn lists_each_container_with_its_own_state() {
        let pod = pod_json(serde_json::json!({
            "metadata": { "name": "p" },
            "spec": {
                "containers": [
                    { "name": "api", "image": "acme/api:1" },
                    { "name": "worker", "image": "acme/worker:1" },
                    { "name": "report", "image": "acme/report:1" },
                    { "name": "late", "image": "acme/late:1" }
                ]
            },
            "status": {
                "containerStatuses": [
                    // Reported out of spec order, as the kubelet may.
                    { "name": "worker", "image": "x", "imageID": "", "ready": false, "restartCount": 14,
                      "state": { "waiting": { "reason": "CrashLoopBackOff" } } },
                    { "name": "api", "image": "x", "imageID": "", "ready": true, "restartCount": 0,
                      "state": { "running": {} } },
                    { "name": "report", "image": "x", "imageID": "", "ready": false, "restartCount": 1,
                      "state": { "terminated": { "exitCode": 137, "reason": "OOMKilled" } } }
                ]
            }
        }));
        let containers = summarise_pod(pod).containers;
        let row = |c: &PodContainer| {
            (c.name.clone(), c.state.clone(), c.reason.clone(), c.exit_code, c.ready, c.restarts)
        };
        assert_eq!(
            containers.iter().map(row).collect::<Vec<_>>(),
            vec![
                ("api".into(), "running".into(), "".into(), None, true, 0),
                ("worker".into(), "waiting".into(), "CrashLoopBackOff".into(), None, false, 14),
                ("report".into(), "terminated".into(), "OOMKilled".into(), Some(137), false, 1),
                // Named in the spec, not yet reported on: listed, not missing.
                ("late".into(), "unknown".into(), "".into(), None, false, 0),
            ]
        );
        assert!(containers.iter().all(|c| c.kind == "app"));
        assert_eq!(containers[0].image, "acme/api:1");
    }

    #[test]
    fn lists_init_containers_after_the_app_ones_and_tells_a_sidecar_from_an_init() {
        let pod = pod_json(serde_json::json!({
            "metadata": { "name": "p" },
            "spec": {
                "initContainers": [
                    { "name": "migrate", "image": "acme/migrate:1" },
                    { "name": "proxy", "image": "acme/proxy:1", "restartPolicy": "Always" }
                ],
                "containers": [{ "name": "api", "image": "acme/api:1" }]
            },
            "status": {
                "initContainerStatuses": [
                    { "name": "migrate", "image": "x", "imageID": "", "ready": true, "restartCount": 0,
                      "state": { "terminated": { "exitCode": 0, "reason": "Completed" } } },
                    { "name": "proxy", "image": "x", "imageID": "", "ready": true, "restartCount": 0,
                      "state": { "running": {} } }
                ],
                "containerStatuses": [
                    { "name": "api", "image": "x", "imageID": "", "ready": true, "restartCount": 0,
                      "state": { "running": {} } }
                ]
            }
        }));
        let containers = summarise_pod(pod).containers;
        assert_eq!(
            containers.iter().map(|c| (c.name.as_str(), c.kind.as_str(), c.state.as_str())).collect::<Vec<_>>(),
            vec![("api", "app", "running"), ("migrate", "init", "terminated"), ("proxy", "sidecar", "running")]
        );
        assert_eq!(containers[1].exit_code, Some(0));
    }

    #[test]
    fn a_pod_with_no_spec_lists_no_containers_and_the_field_is_sent_by_name() {
        assert!(summarise_pod(Pod::default()).containers.is_empty());
        let json = serde_json::to_value(summarise_pod(pod_with_limits(&[(None, None)]))).unwrap();
        assert_eq!(json["containers"][0]["name"], "c0");
        assert_eq!(json["containers"][0]["state"], "unknown");
        assert!(json["containers"][0]["exitCode"].is_null());
    }

    /// A pod with one app container and one init container, each given
    /// `(cpu, memory)` limits; `sidecar` makes the init container restart
    /// always, which is what a native sidecar is.
    fn pod_with_init(
        app: (Option<&str>, Option<&str>),
        init: (Option<&str>, Option<&str>),
        sidecar: bool,
    ) -> Pod {
        let limits = |(cpu, memory): (Option<&str>, Option<&str>)| {
            let mut limits = serde_json::Map::new();
            if let Some(cpu) = cpu {
                limits.insert("cpu".into(), serde_json::json!(cpu));
            }
            if let Some(memory) = memory {
                limits.insert("memory".into(), serde_json::json!(memory));
            }
            serde_json::json!({ "limits": limits.clone(), "requests": limits })
        };
        let mut init_container = serde_json::json!({ "name": "init", "image": "img", "resources": limits(init) });
        if sidecar {
            init_container["restartPolicy"] = serde_json::json!("Always");
        }
        serde_json::from_value(serde_json::json!({
            "metadata": { "name": "p" },
            "spec": {
                "containers": [{ "name": "app", "image": "img", "resources": limits(app) }],
                "initContainers": [init_container]
            }
        }))
        .unwrap()
    }

    #[test]
    fn a_sidecar_counts_toward_the_pods_bounds_because_its_usage_counts_toward_the_pods_usage() {
        // An unlimited sidecar beside a limited app: the pod has no CPU
        // ceiling, whatever the app's own limit says.
        let unlimited = summarise_pod(pod_with_init((Some("500m"), Some("256Mi")), (None, None), true));
        assert!(!unlimited.cpu_lim_all && !unlimited.mem_lim_all);

        // A limited sidecar: its limit and request are part of the pod's.
        let limited = summarise_pod(pod_with_init(
            (Some("500m"), Some("256Mi")),
            (Some("100m"), Some("64Mi")),
            true,
        ));
        assert!(limited.cpu_lim_all && limited.mem_lim_all);
        assert_eq!((limited.cpu_lim_millicores, limited.mem_lim_mib), (600, 320));
        assert_eq!((limited.cpu_req_millicores, limited.mem_req_mib), (600, 320));
    }

    #[test]
    fn an_ordinary_init_container_does_not_count_having_finished_before_the_pod_ran() {
        // No limit on it, and it does not take the pod's ceiling away.
        let pod = summarise_pod(pod_with_init((Some("500m"), Some("256Mi")), (None, None), false));
        assert!(pod.cpu_lim_all && pod.mem_lim_all);
        // A large limit on it, and it does not raise the pod's ceiling.
        let pod = summarise_pod(pod_with_init((Some("500m"), Some("256Mi")), (Some("4"), Some("8Gi")), false));
        assert_eq!((pod.cpu_lim_millicores, pod.mem_lim_mib), (500, 256));
    }

    #[test]
    fn a_limit_written_in_decimal_units_is_still_a_limit() {
        // `500M`, not `500Mi`: read as nothing, it was a pod with no memory
        // limit, and the list said "no request or limit set" of a pod that
        // had one.
        let pod = summarise_pod(pod_with_limits(&[(Some("1"), Some("500M"))]));
        assert_eq!(pod.mem_lim_mib, 476);
        assert!(pod.mem_lim_all);
        let pod = summarise_pod(pod_with_limits(&[(Some("1"), Some("2G"))]));
        assert_eq!(pod.mem_lim_mib, 1907);
    }

    #[test]
    fn the_ceiling_flags_are_sent_under_the_names_the_frontend_reads() {
        let json = serde_json::to_value(summarise_pod(pod_with_limits(&[(Some("1"), Some("1Gi"))]))).unwrap();
        assert_eq!(json["cpuLimAll"], true);
        assert_eq!(json["memLimAll"], true);
        assert_eq!(json["cpuLimMillicores"], 1000);
        assert_eq!(json["memLimMiB"], 1024);
    }

    #[test]
    fn pod_summary_carries_the_creation_timestamp_for_live_ages() {
        let pod = Pod {
            metadata: kube::core::ObjectMeta {
                name: Some("web-1".into()),
                namespace: Some("default".into()),
                creation_timestamp: Some(k8s_openapi::apimachinery::pkg::apis::meta::v1::Time(
                    "2026-08-20T00:00:00Z".parse().unwrap(),
                )),
                ..Default::default()
            },
            ..Default::default()
        };
        let summary = summarise_pod(pod);
        assert_eq!(summary.created.as_deref(), Some("2026-08-20T00:00:00Z"));
    }

    #[test]
    fn pod_summary_marks_an_unknown_creation_timestamp_as_absent() {
        let summary = summarise_pod(Pod::default());
        assert_eq!(summary.created, None);
    }

    #[test]
    fn builds_label_selector_string() {
        let mut m = std::collections::BTreeMap::new();
        m.insert("app".to_string(), "web".to_string());
        m.insert("tier".to_string(), "frontend".to_string());
        assert_eq!(label_selector(&m), "app=web,tier=frontend");
    }

    /// `matchLabels` for a selector fixture, so an expression test can state
    /// the equality half in one line.
    fn labels(pairs: &[(&str, &str)]) -> std::collections::BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn req(key: &str, operator: &str, values: &[&str]) -> LabelSelectorRequirement {
        LabelSelectorRequirement {
            key: key.to_string(),
            operator: operator.to_string(),
            values: values.iter().map(|v| v.to_string()).collect(),
        }
    }

    #[test]
    fn renders_each_set_based_operator_in_kubernetes_syntax() {
        // Deliberately no `matchLabels`: an expression rendered by accident
        // through the equality half would show up here as a missing term.
        let none = labels(&[]);
        assert_eq!(
            selector_query(&none, &[req("track", "In", &["canary", "stable"])]).unwrap(),
            Some("track in (canary,stable)".to_string())
        );
        assert_eq!(
            selector_query(&none, &[req("track", "NotIn", &["canary"])]).unwrap(),
            Some("track notin (canary)".to_string())
        );
        assert_eq!(
            selector_query(&none, &[req("track", "Exists", &[])]).unwrap(),
            Some("track".to_string())
        );
        assert_eq!(
            selector_query(&none, &[req("track", "DoesNotExist", &[])]).unwrap(),
            Some("!track".to_string())
        );
    }

    #[test]
    fn conjoins_both_halves_of_the_selector() {
        // `app=web` alone would select the canary pods too, so a query that
        // drops the expression is visibly different from this one.
        let q = selector_query(
            &labels(&[("app", "web")]),
            &[req("track", "NotIn", &["canary"])],
        )
        .unwrap();
        assert_eq!(q, Some("app=web,track notin (canary)".to_string()));
    }

    #[test]
    fn an_expression_only_selector_still_queries() {
        // The empty-`matchLabels` guard must not swallow a workload whose
        // selector is expressed entirely in `matchExpressions`.
        let q = selector_query(&labels(&[]), &[req("app", "In", &["web"])]).unwrap();
        assert_eq!(q, Some("app in (web)".to_string()));
    }

    #[test]
    fn a_selector_with_neither_half_asks_for_nothing() {
        assert_eq!(selector_query(&labels(&[]), &[]).unwrap(), None);
    }

    #[test]
    fn in_with_no_values_can_never_match() {
        // Membership of the empty set is false for every pod — including the
        // ones `app=web` would otherwise have selected.
        assert_eq!(
            selector_query(&labels(&[("app", "web")]), &[req("track", "In", &[])]).unwrap(),
            None
        );
    }

    #[test]
    fn notin_with_no_values_constrains_nothing() {
        // Every pod is outside the empty set, so the term drops out — but the
        // rest of the selector stands.
        assert_eq!(
            selector_query(&labels(&[("app", "web")]), &[req("track", "NotIn", &[])]).unwrap(),
            Some("app=web".to_string())
        );
        // ...and on its own it leaves a selector that matches the whole
        // namespace, which asks for nothing.
        assert_eq!(
            selector_query(&labels(&[]), &[req("track", "NotIn", &[])]).unwrap(),
            None
        );
    }

    #[test]
    fn refuses_an_operator_the_api_does_not_spell_that_way() {
        // Case-sensitive: "in" is not `In`, and a selector we render wrongly
        // returns the wrong pods silently.
        for operator in ["in", "IN", "NOTIN", "exists", "Contains", ""] {
            let out = selector_query(&labels(&[]), &[req("track", operator, &["canary"])]);
            assert!(
                matches!(out, Err(CapabilityError::InvalidInput(_))),
                "expected {operator:?} to be refused, got {out:?}"
            );
        }
    }

    #[test]
    fn refuses_values_on_an_existence_operator() {
        for operator in ["Exists", "DoesNotExist"] {
            let out = selector_query(&labels(&[]), &[req("track", operator, &["canary"])]);
            assert!(
                matches!(out, Err(CapabilityError::InvalidInput(_))),
                "expected {operator} with values to be refused, got {out:?}"
            );
        }
    }

    #[test]
    fn refuses_a_key_or_value_that_would_break_the_selector_grammar() {
        // A comma or paren in a key or value would be read as syntax by the
        // API server, quietly widening the query.
        let broken = [
            req("track,app", "In", &["canary"]),
            req("track", "In", &["canary),app in (web"]),
            req("", "Exists", &[]),
            req("track", "In", &[""]),
            req("tr ack", "Exists", &[]),
        ];
        for r in broken {
            let out = selector_query(&labels(&[]), std::slice::from_ref(&r));
            assert!(
                matches!(out, Err(CapabilityError::InvalidInput(_))),
                "expected {r:?} to be refused, got {out:?}"
            );
        }
    }

    #[test]
    fn refuses_match_labels_that_would_break_the_selector_grammar() {
        // The equality half went straight into `format!("{k}={v}")` while only
        // the expression half was gated, and the capability's own comment
        // claimed a malformed selector was refused. Label selectors are
        // conjunction-only, so an injected term can only NARROW the answer —
        // which on "which pods does this workload own" is a silent under-report
        // rather than a leak, and is exactly the wrong answer the expression
        // gate exists to prevent.
        let broken = [
            labels(&[("app", "web,tier=cache")]),
            labels(&[("app", "web,!tier")]),
            labels(&[("app=web,tier", "x")]),
            labels(&[("app", "web)")]),
            labels(&[("ap p", "web")]),
            labels(&[("", "web")]),
        ];
        for m in broken {
            let out = selector_query(&m, &[]);
            assert!(
                matches!(out, Err(CapabilityError::InvalidInput(_))),
                "expected {m:?} to be refused, got {out:?}"
            );
        }
    }

    #[test]
    fn keeps_an_empty_match_label_value_which_is_a_real_selector() {
        // `app=` asks for pods carrying `app` with an empty value, which is a
        // legitimate thing to select on — so the value rule must not inherit
        // the key rule's non-empty requirement.
        assert_eq!(
            selector_query(&labels(&[("app", "")]), &[]).unwrap(),
            Some("app=".to_string()),
        );
    }

    #[test]
    fn keeps_a_qualified_label_key_intact() {
        // Real selectors use `/` and `.` in keys; refusing those would refuse
        // most of the cluster.
        assert_eq!(
            selector_query(
                &labels(&[]),
                &[req("app.kubernetes.io/name", "In", &["web"])]
            )
            .unwrap(),
            Some("app.kubernetes.io/name in (web)".to_string())
        );
    }

    #[test]
    fn summarises_ready_and_restarts() {
        let pod = Pod {
            metadata: kube::core::ObjectMeta {
                name: Some("web-1".into()),
                namespace: Some("default".into()),
                ..Default::default()
            },
            spec: Some(PodSpec {
                node_name: Some("node-a".into()),
                ..Default::default()
            }),
            status: Some(PodStatus {
                phase: Some("Running".into()),
                container_statuses: Some(vec![
                    ContainerStatus {
                        ready: true,
                        restart_count: 1,
                        ..Default::default()
                    },
                    ContainerStatus {
                        ready: false,
                        restart_count: 2,
                        ..Default::default()
                    },
                ]),
                ..Default::default()
            }),
        };
        let s = summarise_pod(pod);
        assert_eq!(s.name, "web-1");
        assert_eq!(s.phase, "Running");
        assert_eq!(s.ready, "1/2");
        assert_eq!(s.restarts, 3);
        assert_eq!(s.node, "node-a");
    }

    #[test]
    fn summarises_pod_with_no_status() {
        let pod = Pod {
            metadata: kube::core::ObjectMeta {
                name: Some("pending".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        let s = summarise_pod(pod);
        assert_eq!(s.phase, "Unknown");
        assert_eq!(s.ready, "0/0");
        assert_eq!(s.restarts, 0);
        assert_eq!(s.image, "");
    }

    #[test]
    fn reports_the_waiting_reason_a_running_phase_hides() {
        // The defect this field exists for: a pod whose only container is in
        // CrashLoopBackOff still reports phase "Running", so a row that reads
        // nothing but the phase draws it green and healthy.
        let pod = Pod {
            metadata: kube::core::ObjectMeta {
                name: Some("checkout-api".into()),
                ..Default::default()
            },
            status: Some(PodStatus {
                phase: Some("Running".into()),
                container_statuses: Some(vec![ContainerStatus {
                    name: "api".into(),
                    ready: false,
                    restart_count: 7,
                    state: Some(ContainerState {
                        waiting: Some(ContainerStateWaiting {
                            reason: Some("CrashLoopBackOff".into()),
                            message: Some("back-off 5m0s restarting failed container".into()),
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let s = summarise_pod(pod);
        assert_eq!(s.phase, "Running");
        assert_eq!(s.waiting_reason, "CrashLoopBackOff");
    }

    #[test]
    fn leaves_the_waiting_reason_empty_when_nothing_is_waiting() {
        let pod = Pod {
            metadata: kube::core::ObjectMeta {
                name: Some("web-1".into()),
                ..Default::default()
            },
            status: Some(PodStatus {
                phase: Some("Running".into()),
                container_statuses: Some(vec![ContainerStatus {
                    name: "web".into(),
                    ready: true,
                    restart_count: 0,
                    state: Some(ContainerState {
                        running: Some(ContainerStateRunning { started_at: None }),
                        ..Default::default()
                    }),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(summarise_pod(pod).waiting_reason, "");
    }

    #[test]
    fn takes_the_first_waiting_container_of_several() {
        // Container order, exactly as `image` joins in container order: one
        // row shows one reason, and it is the first one the pod reports.
        let waiting = |reason: &str| ContainerStatus {
            name: reason.into(),
            ready: false,
            restart_count: 0,
            state: Some(ContainerState {
                waiting: Some(ContainerStateWaiting {
                    reason: Some(reason.into()),
                    message: None,
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let pod = Pod {
            metadata: kube::core::ObjectMeta {
                name: Some("web-1".into()),
                ..Default::default()
            },
            status: Some(PodStatus {
                phase: Some("Pending".into()),
                container_statuses: Some(vec![
                    ContainerStatus {
                        name: "sidecar".into(),
                        ready: true,
                        restart_count: 0,
                        ..Default::default()
                    },
                    waiting("ImagePullBackOff"),
                    waiting("CreateContainerConfigError"),
                ]),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(summarise_pod(pod).waiting_reason, "ImagePullBackOff");
    }

    #[test]
    fn skips_a_waiting_container_with_an_empty_reason() {
        // kubectl skips it too, so `status` names the later container's reason;
        // a `waitingReason` that stopped at the empty one disagreed with it, and
        // the desktop's row toned the same pod differently from its header.
        let waiting = |name: &str, reason: &str| ContainerStatus {
            name: name.into(),
            ready: false,
            restart_count: 0,
            state: Some(ContainerState {
                waiting: Some(ContainerStateWaiting {
                    reason: Some(reason.into()),
                    message: None,
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let pod = Pod {
            metadata: kube::core::ObjectMeta {
                name: Some("web-1".into()),
                ..Default::default()
            },
            status: Some(PodStatus {
                phase: Some("Running".into()),
                container_statuses: Some(vec![
                    waiting("init-shim", ""),
                    waiting("api", "CrashLoopBackOff"),
                ]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let s = summarise_pod(pod);
        assert_eq!(s.waiting_reason, "CrashLoopBackOff");
        assert_eq!(s.status, "CrashLoopBackOff");
    }

    #[test]
    fn summarises_single_container_image() {
        let pod = Pod {
            metadata: kube::core::ObjectMeta {
                name: Some("web-1".into()),
                namespace: Some("default".into()),
                ..Default::default()
            },
            status: Some(PodStatus {
                phase: Some("Running".into()),
                container_statuses: Some(vec![ContainerStatus {
                    name: "web".into(),
                    image: "redis:7.4-alpine".into(),
                    ready: true,
                    restart_count: 0,
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let s = summarise_pod(pod);
        assert_eq!(s.image, "redis:7.4-alpine");
    }

    #[test]
    fn summarises_multi_container_image_as_joined_list() {
        let pod = Pod {
            metadata: kube::core::ObjectMeta {
                name: Some("web-1".into()),
                namespace: Some("default".into()),
                ..Default::default()
            },
            status: Some(PodStatus {
                phase: Some("Running".into()),
                container_statuses: Some(vec![
                    ContainerStatus {
                        name: "app".into(),
                        image: "acme/checkout-api:118a7e".into(),
                        ready: true,
                        restart_count: 0,
                        ..Default::default()
                    },
                    ContainerStatus {
                        name: "sidecar".into(),
                        image: "envoyproxy/envoy:v1.30".into(),
                        ready: true,
                        restart_count: 0,
                        ..Default::default()
                    },
                ]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let s = summarise_pod(pod);
        assert_eq!(s.image, "acme/checkout-api:118a7e, envoyproxy/envoy:v1.30");
    }

    #[test]
    fn summarises_pod_with_no_containers_has_empty_image() {
        let pod = Pod {
            metadata: kube::core::ObjectMeta {
                name: Some("empty".into()),
                namespace: Some("default".into()),
                ..Default::default()
            },
            status: Some(PodStatus {
                phase: Some("Pending".into()),
                container_statuses: Some(vec![]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let s = summarise_pod(pod);
        assert_eq!(s.image, "");
    }
}
