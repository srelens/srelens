//! `k8s.countResources` — how many objects of each kind there are, for the
//! sidebar's per-kind counts (#402). A count, never a list.
//!
//! **One object per request.** Each kind (per namespace) is one
//! metadata-only list asking for `limit=1`. The API server answers a page of
//! at most one item and, when there is more, `remainingItemCount`: the count
//! is the two added. `k8s.podCount` lists every pod's metadata to count them,
//! which on a 5 416-pod cluster is a real request; this is one small object
//! whatever the size, so the sidebar can afford dozens of kinds every 30 s.
//!
//! **One budget for the whole call**, not one per request. The fan-out runs a
//! few requests at a time, and a timeout per request would add up: 35 requests
//! to a cluster that never answers, 8 at a time, is about 40 s of waiting —
//! longer than the sidebar's refresh.
//!
//! **A cluster that answered nothing fails the call.** `ClientCache::get`
//! builds a client without contacting the cluster, so an unreachable cluster
//! only shows here. A call whose every request went unanswered is reported as
//! the call's failure, which the sidebar draws as stale numbers — not as a
//! column of kinds that "could not be counted", which would read as RBAC.
//! One API answer anywhere, a 403 included, means the cluster is up, and
//! everything else is that kind's own outcome.
//!
//! A kind that could not be counted is `count: null` WITH the reason, never
//! `0`: a refused list and an empty cluster are the same picture and opposite
//! facts, and zero is the one a reader believes.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use kube::api::{Api, ApiResource, DynamicObject, ListParams};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use srelens_capability::{Annotations, Capability, CapabilityError};
use tokio::time::Instant;

use crate::client_cache::ClientCache;

/// The most namespaces one call counts over.
pub const MAX_NAMESPACES: usize = 128;
/// The most requests one call may make, reckoned as `kinds × max(1,
/// namespaces)` — every kind counted as namespaced, so a caller can check it
/// without knowing which kinds are. Bounds what an MCP caller can fan out.
pub const MAX_REQUESTS: usize = 1024;
/// Requests in flight at once, per call.
const IN_FLIGHT: usize = 8;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct CountKind {
    /// Kubernetes Kind, e.g. "Pod", "ConfigMap".
    pub kind: String,
    /// API group hint, as `k8s.listResource` takes it.
    #[serde(default)]
    pub group: Option<String>,
    /// API version hint.
    #[serde(default)]
    pub version: Option<String>,
    /// Plural name hint.
    #[serde(default)]
    pub plural: Option<String>,
    /// Whether the kind is namespaced. With `group`, `version` and `plural`
    /// it addresses a custom resource without asking the cluster's discovery.
    #[serde(default)]
    pub namespaced: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CountResourcesIn {
    pub context: String,
    /// The kinds to count, answered in this order.
    pub kinds: Vec<CountKind>,
    /// Namespaces to count namespaced kinds in, summed. Empty or absent means
    /// all namespaces. Cluster-scoped kinds ignore it.
    #[serde(default)]
    pub namespaces: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct KindCount {
    pub kind: String,
    /// The group this kind was counted in: `apps` for a Deployment asked for
    /// without one, `""` for the core group. A kind that could not be resolved
    /// keeps the `group` asked for, `""` when none was given.
    pub group: String,
    /// `null` when this kind could not be counted — never `0` for a failure.
    pub count: Option<i64>,
    /// Why `count` is `null`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CountResourcesOut {
    pub counts: Vec<KindCount>,
}

/// A custom resource addressed by its hints alone, or `None` when any of the
/// four is missing. Without `namespaced`, `resolve_resource`'s first tier
/// still consults discovery — a whole-cluster CRD list when its index is cold
/// — and, if that cannot say, takes it as namespaced only when `namespaces`
/// are given.
fn hinted(k: &CountKind) -> Option<(ApiResource, bool)> {
    let (g, v, p, namespaced) = (
        k.group.as_deref()?,
        k.version.as_deref()?,
        k.plural.as_deref()?,
        k.namespaced?,
    );
    Some((
        crate::crds::custom_api_resource(g, v, &k.kind, p),
        namespaced,
    ))
}

fn check_bounds(kinds: usize, namespaces: usize) -> Result<(), CapabilityError> {
    if namespaces > MAX_NAMESPACES {
        return Err(CapabilityError::InvalidInput(format!(
            "at most {MAX_NAMESPACES} namespaces can be counted at once, {namespaces} were given"
        )));
    }
    let requests = kinds.saturating_mul(namespaces.max(1));
    if requests > MAX_REQUESTS {
        return Err(CapabilityError::InvalidInput(format!(
            "{kinds} kinds over {} namespace scope(s) is {requests} requests; at most {MAX_REQUESTS} per call",
            namespaces.max(1)
        )));
    }
    Ok(())
}

/// What one request came to. `NoAnswer` is the cluster saying nothing at all
/// — a connection failure or the budget running out — and is what tells an
/// unreachable cluster from a refused kind.
enum Outcome {
    Answered(Result<i64, String>),
    NoAnswer(String),
}

async fn count_one(api: Api<DynamicObject>, deadline: Instant) -> Outcome {
    let one = ListParams::default().limit(1);
    match tokio::time::timeout_at(deadline, api.list_metadata(&one)).await {
        Err(_) => Outcome::NoAnswer("timed out".into()),
        Ok(Ok(list)) => {
            let shown = list.items.len() as i64;
            let more = list
                .metadata
                .continue_
                .as_deref()
                .is_some_and(|c| !c.is_empty());
            match list.metadata.remaining_item_count {
                Some(rest) => Outcome::Answered(Ok(shown + rest)),
                None if !more => Outcome::Answered(Ok(shown)),
                // ponytail: no remainingItemCount = unknown; page list_metadata(limit=500) if a real server needs it
                None => Outcome::Answered(Err(
                    "the API server did not report how many there are".into()
                )),
            }
        }
        Ok(Err(kube::Error::Api(status))) => Outcome::Answered(Err(if status.message.is_empty() {
            status.reason.clone()
        } else {
            status.message.clone()
        })),
        Ok(Err(e)) => Outcome::NoAnswer(e.to_string()),
    }
}

/// A kind as asked, with what it resolved to (`namespaced` alongside) or why
/// it could not be.
type Target = (CountKind, Result<(ApiResource, bool), String>);

/// Count every resolved target against one client, under one budget. A
/// namespaced kind is summed over `namespaces` (deduplicated; none means all
/// namespaces), and one namespace's failure leaves the whole kind unknown —
/// a sum missing a namespace looks exact and is not.
async fn count_kinds(
    client: kube::Client,
    targets: Vec<Target>,
    namespaces: &[String],
    budget: Duration,
) -> Result<Vec<KindCount>, String> {
    let namespaces: BTreeSet<&str> = namespaces
        .iter()
        .map(String::as_str)
        .filter(|n| !n.is_empty())
        .collect();
    let deadline = Instant::now() + budget;

    let mut jobs = Vec::new();
    for (i, (_, resolved)) in targets.iter().enumerate() {
        let Ok((ar, namespaced)) = resolved else {
            continue;
        };
        if *namespaced && !namespaces.is_empty() {
            for ns in &namespaces {
                jobs.push((
                    i,
                    Api::<DynamicObject>::namespaced_with(client.clone(), ns, ar),
                ));
            }
        } else {
            jobs.push((i, Api::<DynamicObject>::all_with(client.clone(), ar)));
        }
    }
    let asked = jobs.len();
    let outcomes: Vec<(usize, Outcome)> = futures::stream::iter(jobs)
        .map(|(i, api)| async move { (i, count_one(api, deadline).await) })
        .buffer_unordered(IN_FLIGHT)
        .collect()
        .await;

    if asked > 0
        && outcomes
            .iter()
            .all(|(_, o)| matches!(o, Outcome::NoAnswer(_)))
    {
        let why = outcomes.iter().find_map(|(_, o)| match o {
            Outcome::NoAnswer(e) => Some(e.clone()),
            Outcome::Answered(_) => None,
        });
        return Err(format!(
            "the cluster did not answer: {}",
            why.unwrap_or_default()
        ));
    }

    let mut sums: Vec<Result<i64, String>> = targets
        .iter()
        .map(|(_, resolved)| resolved.as_ref().map(|_| 0).map_err(Clone::clone))
        .collect();
    for (i, outcome) in outcomes {
        let got = match outcome {
            Outcome::Answered(r) => r,
            Outcome::NoAnswer(e) => Err(e),
        };
        sums[i] = match (std::mem::replace(&mut sums[i], Ok(0)), got) {
            (Ok(sum), Ok(n)) => Ok(sum + n),
            (Err(e), _) | (Ok(_), Err(e)) => Err(e),
        };
    }

    Ok(targets
        .into_iter()
        .zip(sums)
        .map(|((k, resolved), sum)| {
            let group = match &resolved {
                Ok((ar, _)) => ar.group.clone(),
                Err(_) => k.group.unwrap_or_default(),
            };
            match sum {
                Ok(n) => KindCount {
                    kind: k.kind,
                    group,
                    count: Some(n),
                    error: None,
                },
                Err(e) => KindCount {
                    kind: k.kind,
                    group,
                    count: None,
                    error: Some(e),
                },
            }
        })
        .collect())
}

/// `k8s.countResources` — how many objects of each kind, counted with one
/// small request per kind and namespace. See the module docs.
pub fn count_resources_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<CountResourcesIn, CountResourcesOut, _, _>(
        "k8s.countResources",
        "how many objects of each kind there are, counted without listing them: one metadata-only \
         request per kind (and per namespace) asking for a single object, plus the API server's \
         remainingItemCount. Omit `namespaces` to count all namespaces; cluster-scoped kinds ignore \
         it. For a custom resource pass `group`, `version`, `plural` and `namespaced` as well as \
         `kind`. A kind that could not be counted (refused, timed out, or the server did not say) \
         has `count: null` and an `error`, never 0.",
        Annotations::READ_ONLY,
        move |input: CountResourcesIn| {
            let cache = cache.clone();
            async move {
                let namespaces: Vec<String> = input
                    .namespaces
                    .iter()
                    .filter(|n| !n.is_empty())
                    .cloned()
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();
                check_bounds(input.kinds.len(), namespaces.len())?;
                let mut targets = Vec::with_capacity(input.kinds.len());
                for k in input.kinds {
                    let resolved = match hinted(&k) {
                        Some(found) => Ok(found),
                        None => {
                            cache
                                .resolve_resource(
                                    &input.context,
                                    &k.kind,
                                    k.group.as_deref(),
                                    k.version.as_deref(),
                                    k.plural.as_deref(),
                                    !namespaces.is_empty(),
                                )
                                .await
                        }
                    };
                    targets.push((k, resolved));
                }
                let client = cache.get(&input.context).await.map_err(CapabilityError::Handler)?;
                let counts = count_kinds(client, targets, &namespaces, crate::connect::request_timeout())
                    .await
                    .map_err(CapabilityError::Handler)?;
                Ok(CountResourcesOut { counts })
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn kind(kind: &str) -> CountKind {
        CountKind {
            kind: kind.into(),
            group: None,
            version: None,
            plural: None,
            namespaced: None,
        }
    }

    #[test]
    fn count_resources_capability_has_id() {
        let cache = ClientCache::new(PathBuf::from("/x"));
        assert_eq!(count_resources_capability(cache).id, "k8s.countResources");
    }

    /// With all four hints a custom resource is addressed without asking the
    /// cluster anything: `resolve_resource`'s first tier would still consult
    /// discovery to learn whether it is namespaced.
    #[test]
    fn full_hints_skip_discovery() {
        let full = CountKind {
            kind: "Certificate".into(),
            group: Some("cert-manager.io".into()),
            version: Some("v1".into()),
            plural: Some("certificates".into()),
            namespaced: Some(false),
        };
        let (ar, namespaced) = hinted(&full).expect("all four hints given");
        assert_eq!(ar.plural, "certificates");
        assert_eq!(ar.group, "cert-manager.io");
        assert!(!namespaced);

        let partial = CountKind {
            namespaced: None,
            ..full
        };
        assert!(hinted(&partial).is_none());
        assert!(hinted(&kind("Pod")).is_none());
    }

    #[test]
    fn bounds() {
        assert!(matches!(
            check_bounds(1, MAX_NAMESPACES + 1),
            Err(CapabilityError::InvalidInput(_))
        ));
        assert!(matches!(
            check_bounds(9, 128),
            Err(CapabilityError::InvalidInput(_))
        ));
        assert!(check_bounds(8, 128).is_ok());
        assert!(check_bounds(MAX_REQUESTS, 0).is_ok());
        assert!(matches!(
            check_bounds(MAX_REQUESTS + 1, 0),
            Err(CapabilityError::InvalidInput(_))
        ));
    }
}

/// Counting runs against a bare, hand-rolled HTTP server, the way
/// `pod_count.rs`'s tests do: `kube::Client` needs a real `tower::Service`,
/// and a listener that routes by path and writes a canned body is enough to
/// prove what was asked (path, `limit=1`, the metadata-only Accept header)
/// as well as what came back.
#[cfg(test)]
mod count_tests {
    use super::*;
    use k8s_openapi::api::core::v1::{ConfigMap, Node, Pod};
    use kube::{Client, Config};
    use std::sync::Mutex;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// `status: 0` accepts the request and never answers.
    struct Route {
        path: &'static str,
        status: u16,
        body: String,
        delay_ms: u64,
    }

    fn ok(path: &'static str, body: String) -> Route {
        Route {
            path,
            status: 200,
            body,
            delay_ms: 0,
        }
    }

    fn hang(path: &'static str) -> Route {
        Route {
            path,
            status: 0,
            body: String::new(),
            delay_ms: 0,
        }
    }

    fn forbidden(path: &'static str, what: &str) -> Route {
        let body = serde_json::json!({
            "kind": "Status", "apiVersion": "v1", "metadata": {}, "status": "Failure",
            "message": format!("{what} is forbidden: User \"dana\" cannot list resource \"{what}\""),
            "reason": "Forbidden", "code": 403
        })
        .to_string();
        Route {
            path,
            status: 403,
            body,
            delay_ms: 0,
        }
    }

    /// One page of a metadata-only list: `n` items, and the `continue` /
    /// `remainingItemCount` a `limit=1` answer carries when there is more.
    fn page(n: usize, cont: Option<&str>, remaining: Option<i64>) -> String {
        let items: Vec<_> = (0..n)
            .map(|i| serde_json::json!({ "apiVersion": "meta.k8s.io/v1", "kind": "PartialObjectMetadata", "metadata": { "name": format!("o{i}") } }))
            .collect();
        let mut metadata = serde_json::json!({ "resourceVersion": "1" });
        if let Some(c) = cont {
            metadata["continue"] = c.into();
        }
        if let Some(r) = remaining {
            metadata["remainingItemCount"] = r.into();
        }
        serde_json::json!({ "apiVersion": "meta.k8s.io/v1", "kind": "PartialObjectMetadataList", "metadata": metadata, "items": items }).to_string()
    }

    /// `n` objects in all, answered the way a real server answers `limit=1`.
    fn holding(n: i64) -> String {
        match n {
            0 => page(0, None, None),
            1 => page(1, None, None),
            _ => page(1, Some("next"), Some(n - 1)),
        }
    }

    type Seen = Arc<Mutex<Vec<(String, String)>>>;

    async fn fake_api(routes: Vec<Route>) -> (Client, tokio::task::JoinHandle<()>, Seen) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen: Seen = Arc::new(Mutex::new(Vec::new()));
        let seen_task = seen.clone();
        let routes = Arc::new(routes);
        let handle = tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let seen = seen_task.clone();
                let routes = routes.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 8192];
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]).to_string();
                    let line = req.lines().next().unwrap_or("").to_string();
                    let accept = req
                        .lines()
                        .find(|l| l.to_ascii_lowercase().starts_with("accept:"))
                        .unwrap_or("")
                        .to_string();
                    let path = line
                        .split_whitespace()
                        .nth(1)
                        .unwrap_or("")
                        .split('?')
                        .next()
                        .unwrap_or("")
                        .to_string();
                    seen.lock().unwrap().push((line, accept));
                    let (status, body, delay) = match routes.iter().find(|r| r.path == path) {
                        Some(r) => (r.status, r.body.clone(), r.delay_ms),
                        None => (404, r#"{"kind":"Status","apiVersion":"v1","metadata":{},"status":"Failure","message":"not found","reason":"NotFound","code":404}"#.to_string(), 0),
                    };
                    if status == 0 {
                        let _held = stream;
                        tokio::time::sleep(Duration::from_secs(30)).await;
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                    let response = format!(
                        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                });
            }
        });
        let config = Config::new(format!("http://{addr}").parse().unwrap());
        (Client::try_from(config).unwrap(), handle, seen)
    }

    fn pods() -> (CountKind, Result<(ApiResource, bool), String>) {
        (
            CountKind {
                kind: "Pod".into(),
                group: None,
                version: None,
                plural: None,
                namespaced: None,
            },
            Ok((ApiResource::erase::<Pod>(&()), true)),
        )
    }
    fn configmaps() -> (CountKind, Result<(ApiResource, bool), String>) {
        (
            CountKind {
                kind: "ConfigMap".into(),
                group: None,
                version: None,
                plural: None,
                namespaced: None,
            },
            Ok((ApiResource::erase::<ConfigMap>(&()), true)),
        )
    }
    fn nodes() -> (CountKind, Result<(ApiResource, bool), String>) {
        (
            CountKind {
                kind: "Node".into(),
                group: None,
                version: None,
                plural: None,
                namespaced: None,
            },
            Ok((ApiResource::erase::<Node>(&()), false)),
        )
    }

    const BUDGET: Duration = Duration::from_secs(5);

    fn ns(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn asks_for_one_metadata_only_object() {
        let (client, handle, seen) = fake_api(vec![ok("/api/v1/pods", holding(3))]).await;
        count_kinds(client, vec![pods()], &[], BUDGET)
            .await
            .unwrap();
        let seen = seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 1, "{seen:?}");
        assert!(seen[0].0.contains("limit=1"), "asked: {}", seen[0].0);
        assert!(
            seen[0].1.contains("PartialObjectMetadataList"),
            "accept: {}",
            seen[0].1
        );
        handle.abort();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn count_is_the_page_plus_the_remaining_item_count() {
        let (client, handle, _) =
            fake_api(vec![ok("/api/v1/pods", page(1, Some("x"), Some(37)))]).await;
        let out = count_kinds(client, vec![pods()], &[], BUDGET)
            .await
            .unwrap();
        assert_eq!(out[0].count, Some(38), "{out:?}");
        assert_eq!(out[0].error, None);
        handle.abort();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_empty_list_counts_zero() {
        let (client, handle, _) = fake_api(vec![ok("/api/v1/pods", page(0, None, None))]).await;
        let out = count_kinds(client, vec![pods()], &[], BUDGET)
            .await
            .unwrap();
        assert_eq!(out[0].count, Some(0), "{out:?}");
        handle.abort();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_continue_token_without_a_remaining_count_is_unknown() {
        let (client, handle, _) =
            fake_api(vec![ok("/api/v1/pods", page(1, Some("x"), None))]).await;
        let out = count_kinds(client, vec![pods()], &[], BUDGET)
            .await
            .unwrap();
        assert_eq!(
            out[0].count, None,
            "a page of one with more to come is not a count of one: {out:?}"
        );
        assert!(out[0].error.is_some());
        handle.abort();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_refused_kind_is_null_and_the_next_kind_is_still_counted() {
        let (client, handle, _) = fake_api(vec![
            forbidden("/api/v1/pods", "pods"),
            ok("/api/v1/configmaps", holding(5)),
        ])
        .await;
        let out = count_kinds(client, vec![pods(), configmaps()], &[], BUDGET)
            .await
            .unwrap();
        assert_eq!(out[0].count, None, "{out:?}");
        assert!(
            out[0]
                .error
                .as_deref()
                .unwrap_or("")
                .to_lowercase()
                .contains("forbidden"),
            "{out:?}"
        );
        assert_eq!(out[1].count, Some(5), "{out:?}");
        handle.abort();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn namespaces_are_summed() {
        let (client, handle, seen) = fake_api(vec![
            ok("/api/v1/namespaces/a/pods", holding(3)),
            ok("/api/v1/namespaces/b/pods", holding(4)),
        ])
        .await;
        let out = count_kinds(client, vec![pods()], &ns(&["a", "b"]), BUDGET)
            .await
            .unwrap();
        assert_eq!(out[0].count, Some(7), "{out:?}");
        let seen = seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 2, "{seen:?}");
        assert!(seen.iter().any(|(l, _)| l.contains("/namespaces/a/")));
        assert!(seen.iter().any(|(l, _)| l.contains("/namespaces/b/")));
        handle.abort();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn duplicate_namespaces_are_counted_once() {
        let (client, handle, seen) =
            fake_api(vec![ok("/api/v1/namespaces/a/pods", holding(3))]).await;
        let out = count_kinds(client, vec![pods()], &ns(&["a", "a"]), BUDGET)
            .await
            .unwrap();
        assert_eq!(out[0].count, Some(3), "{out:?}");
        assert_eq!(seen.lock().unwrap().len(), 1);
        handle.abort();
    }

    /// A sum missing one namespace looks exact and is not.
    #[tokio::test(flavor = "multi_thread")]
    async fn one_refused_namespace_leaves_the_kind_unknown() {
        let (client, handle, _) = fake_api(vec![
            ok("/api/v1/namespaces/a/pods", holding(3)),
            forbidden("/api/v1/namespaces/b/pods", "pods"),
        ])
        .await;
        let out = count_kinds(client, vec![pods()], &ns(&["a", "b"]), BUDGET)
            .await
            .unwrap();
        assert_eq!(
            out[0].count, None,
            "never the answered namespace's 3 alone: {out:?}"
        );
        assert!(out[0].error.is_some());
        handle.abort();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_cluster_scoped_kind_ignores_namespaces() {
        let (client, handle, seen) = fake_api(vec![ok("/api/v1/nodes", holding(2))]).await;
        let out = count_kinds(client, vec![nodes()], &ns(&["a", "b"]), BUDGET)
            .await
            .unwrap();
        assert_eq!(out[0].count, Some(2), "{out:?}");
        let seen = seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 1, "{seen:?}");
        assert!(seen[0].0.contains("/api/v1/nodes"), "{seen:?}");
        handle.abort();
    }

    /// A built-in asked for without its group is answered with the group it was
    /// counted in, so a caller can tell `apps/Deployment` from `acme.io/Deployment`.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_built_in_is_answered_with_the_group_it_was_counted_in() {
        let deployments = (
            CountKind {
                kind: "Deployment".into(),
                group: None,
                version: None,
                plural: None,
                namespaced: None,
            },
            Ok((ApiResource::erase::<k8s_openapi::api::apps::v1::Deployment>(&()), true)),
        );
        let (client, handle, _) = fake_api(vec![
            ok("/apis/apps/v1/deployments", holding(2)),
            ok("/api/v1/pods", holding(1)),
        ])
        .await;
        let out = count_kinds(client, vec![deployments, pods()], &[], BUDGET)
            .await
            .unwrap();
        let got: Vec<(&str, &str, Option<i64>)> =
            out.iter().map(|c| (c.kind.as_str(), c.group.as_str(), c.count)).collect();
        assert_eq!(got, vec![("Deployment", "apps", Some(2)), ("Pod", "", Some(1))]);
        handle.abort();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_output_keeps_the_input_order() {
        let mut slow = ok("/api/v1/pods", holding(1));
        slow.delay_ms = 300;
        let (client, handle, _) = fake_api(vec![
            slow,
            ok("/api/v1/configmaps", holding(2)),
            ok("/api/v1/nodes", holding(3)),
        ])
        .await;
        let out = count_kinds(client, vec![pods(), configmaps(), nodes()], &[], BUDGET)
            .await
            .unwrap();
        let got: Vec<(&str, Option<i64>)> =
            out.iter().map(|c| (c.kind.as_str(), c.count)).collect();
        assert_eq!(
            got,
            vec![("Pod", Some(1)), ("ConfigMap", Some(2)), ("Node", Some(3))]
        );
        handle.abort();
    }

    /// `ClientCache::get` never contacts the cluster, so an unreachable one
    /// only shows here: nothing answered, and that is the call's failure —
    /// what the sidebar draws as stale — not a column of unknown rows.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_cluster_that_answers_nothing_fails_the_call() {
        let (client, handle, _) =
            fake_api(vec![hang("/api/v1/pods"), hang("/api/v1/configmaps")]).await;
        let started = std::time::Instant::now();
        let err = count_kinds(
            client,
            vec![pods(), configmaps()],
            &[],
            Duration::from_millis(100),
        )
        .await
        .unwrap_err();
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "the budget bounds the whole call: {:?}",
            started.elapsed()
        );
        assert!(!err.is_empty());
        handle.abort();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_hanging_kind_times_out_while_an_answered_one_counts() {
        let (client, handle, _) = fake_api(vec![
            forbidden("/api/v1/pods", "pods"),
            hang("/api/v1/configmaps"),
        ])
        .await;
        let out = count_kinds(
            client,
            vec![pods(), configmaps()],
            &[],
            Duration::from_millis(100),
        )
        .await
        .unwrap();
        assert_eq!(out[0].count, None);
        assert!(
            out[0]
                .error
                .as_deref()
                .unwrap_or("")
                .to_lowercase()
                .contains("forbidden"),
            "{out:?}"
        );
        assert_eq!(out[1].count, None);
        assert!(
            out[1].error.as_deref().unwrap_or("").contains("timed out"),
            "{out:?}"
        );
        handle.abort();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_unresolved_kind_is_that_rows_error() {
        let (client, handle, _) = fake_api(vec![ok("/api/v1/configmaps", holding(5))]).await;
        let missing = (
            CountKind {
                kind: "NoSuchKind".into(),
                group: None,
                version: None,
                plural: None,
                namespaced: None,
            },
            Err("no such kind".to_string()),
        );
        let out = count_kinds(client, vec![missing, configmaps()], &[], BUDGET)
            .await
            .unwrap();
        assert_eq!(out[0].count, None);
        assert_eq!(out[0].error.as_deref(), Some("no such kind"));
        assert_eq!(out[1].count, Some(5));
        handle.abort();
    }
}
