//! Workload-listing capabilities backed by kube-rs: `k8s.listNamespaces` and
//! `k8s.listPods` for a connected context.

use std::sync::Arc;

use srelens_capability::{Annotations, Capability, CapabilityError};
use k8s_openapi::api::core::v1::{Namespace, Pod};
use kube::api::ListParams;
use kube::Api;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListNamespacesIn {
    pub context: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListNamespacesOut {
    pub namespaces: Vec<String>,
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
    pub age: String,
    /// RFC 3339 creation time, so frontend surfaces can render an age that
    /// continues to tick instead of freezing this summary's `age` string.
    #[serde(rename = "createdAt", skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
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
    /// The first waiting reason across the pod's containers, or `""` when none
    /// is waiting.
    #[serde(rename = "waitingReason")]
    pub waiting_reason: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListPodsOut {
    pub pods: Vec<PodSummary>,
}

fn handler_err(e: impl ToString) -> CapabilityError {
    CapabilityError::Handler(e.to_string())
}

/// `k8s.listNamespaces` — list namespace names in a connected context.
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
                let list = tokio::time::timeout(request_timeout(), api.list(&ListParams::default()))
                    .await
                    .map_err(|_| CapabilityError::Handler("list namespaces timed out".into()))?
                    .map_err(handler_err)?;
                let namespaces = list
                    .items
                    .into_iter()
                    .filter_map(|ns| ns.metadata.name)
                    .collect();
                Ok(ListNamespacesOut { namespaces })
            }
        },
    )
}

/// Summarise a pod's ready count, total restarts, and phase.
pub(crate) fn summarise_pod(pod: Pod) -> PodSummary {
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
    // same reason they are excluded there.
    let waiting_reason = statuses
        .and_then(|cs| {
            cs.iter()
                .find_map(|c| c.state.as_ref()?.waiting.as_ref()?.reason.clone())
        })
        .unwrap_or_default();

    let created_at = pod
        .metadata
        .creation_timestamp
        .as_ref()
        .map(|created| created.0.to_string());
    PodSummary {
        name,
        namespace,
        phase,
        ready: format!("{ready_count}/{total}"),
        restarts,
        node,
        age: crate::humanize_age(pod.metadata.creation_timestamp.as_ref()),
        created_at,
        image,
        waiting_reason,
    }
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
        ContainerState, ContainerStateRunning, ContainerStateWaiting, ContainerStatus, PodSpec,
        PodStatus,
    };

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
        assert_eq!(summary.created_at.as_deref(), Some("2026-08-20T00:00:00Z"));
    }

    #[test]
    fn pod_summary_omits_an_unknown_creation_timestamp() {
        let summary = summarise_pod(Pod::default());
        let json = serde_json::to_value(summary).unwrap();
        assert!(!json.as_object().unwrap().contains_key("createdAt"));
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
