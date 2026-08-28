//! `k8s.podOverview` — the cluster overview's three pod facts, answered
//! without shipping the cluster's pod bodies.
//!
//! The overview asks three questions about pods at once: how many there are,
//! how many are on each node, and which ones are not well. It used to answer
//! all three from `k8s.listPods` with an empty namespace — every pod in the
//! cluster, with its containers, images, ages and phases. On the demo cluster
//! that is 33 pods and costs nothing. On the 113-node cluster this capability
//! was written for it is 5 416 pods and **114 MB uncompressed**, which takes
//! the API server 15-23 s to serve; the request budget is 8 s, so the screen
//! got a timeout and the Pods tile, every node's Pods column and the rail's
//! Pods count all read "No reading".
//!
//! ## What replaces it
//!
//! **The counts come from the Table representation** (`as=Table`,
//! `includeObject=None`) — the server-side printer `kubectl get` itself uses.
//! The server renders one row of cells per pod instead of an object, and its
//! wide columns include NODE, so the whole cluster's pods-per-node grouping
//! arrives in **1.6 MB and under a second**. That is 70× less than the list it
//! replaces.
//!
//! **`list_metadata` cannot do this job**, which is worth stating because it
//! is the obvious guess and it is wrong twice: `PartialObjectMetadata` carries
//! `metadata` and nothing else, so there is no `spec.nodeName` in it to group
//! by — and on this cluster a metadata-only pod list is still **55 MB**,
//! because `managedFields` and annotations travel with the metadata. It is the
//! right tool for [`crate::pod_count`], which only counts; it is not one here.
//!
//! **The unhealthy list still needs bodies**, and the interesting part is how
//! few. `podStatus` in `@srelens/core` flags a pod when its phase is not
//! `Running`/`Succeeded`, OR when a container is waiting with a reason — and
//! that second half is the one that matters most, because a `CrashLoopBackOff`
//! pod's phase is `Running`. No field selector can express it: pods support
//! selectors on `metadata.*`, `spec.nodeName`, `spec.restartPolicy`,
//! `spec.schedulerName`, `spec.serviceAccountName`, `status.phase`,
//! `status.podIP` and `status.nominatedNodeName`, and not one of them looks at
//! a container's state. So:
//!
//! 1. `status.phase!=Running` fetches every pod whose PHASE condemns it — 96
//!    bodies out of 5 416 on that cluster, 1.5 MB.
//! 2. A pod with a waiting container is never fully ready, so the Table's
//!    READY column narrows the rest: only rows short of ready can be the
//!    crash-loopers, and only the ones the first request did not already
//!    bring back are fetched, by name — "did not bring back" decided per POD
//!    and not per name, because two namespaces may run the same one. On that
//!    cluster that is **four**.
//!
//! Both halves are capped ([`UNSETTLED_CAP`]) and the cap is reported, because
//! an unbounded list is the failure this module exists to end.
//!
//! **Nothing here decides whether a pod is healthy.** The READY column and the
//! phase selector choose which pods are worth READING; `podStatus` in core
//! decides what each one means, from the same `phase` and `waitingReason` it
//! has always read. A pod this module fetched needlessly costs one small
//! request; a pod it judged would be a second opinion contradicting core's.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use http::Request;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, ListParams};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use srelens_capability::{Annotations, Capability, CapabilityError};

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;
use crate::workloads::{summarise_pod, PodSummary};

/// The most pod bodies this capability will return, however sick the cluster.
///
/// The `Not ready` band is a dashboard summary, not an inventory — the nodes
/// table above it caps at ten rows for the same reason. A cluster with three
/// thousand `Failed` pods has one problem, not three thousand, and fetching
/// every one of them to say so would rebuild the request this module deleted.
/// Whenever the cap bites, [`PodOverviewOut::truncated`] says so, so the
/// screen can report a short list as short rather than as the whole truth.
pub const UNSETTLED_CAP: usize = 200;

/// How many by-name pod fetches run at once.
///
/// These are the crash-loopers the phase selector could not reach — four on
/// the cluster this was measured against, and rarely many more. Eight at a
/// time keeps a sick cluster's tail from becoming a serial walk without
/// opening a connection per pod on the API server.
const CANDIDATE_CONCURRENCY: usize = 8;

/// Every pod whose PHASE is enough to condemn it, in one server-side filter.
///
/// `Succeeded` is deliberately still in: core leaves a finished pod alone
/// (`TERMINAL_POD_PHASES`), so it costs a row nobody flags — and excluding it
/// here would leave it out of the "already fetched" set, which is what stops
/// the second pass from fetching all 53 of them one at a time. That exclusion
/// holds per pod, not per name; see the note on it in [`gather`].
const NOT_RUNNING: &str = "status.phase!=Running";

/// The server-side printer's own media type — what `kubectl get` asks for.
const TABLE_ACCEPT: &str = "application/json;as=Table;v=v1;g=meta.k8s.io";

/// Every pod in the cluster as printed rows, with no object attached.
///
/// `includeObject=None` is the whole point: `includeObject=Metadata` would
/// staple a `PartialObjectMetadata` — `managedFields` and all — to every row
/// and put the 55 MB straight back.
const PODS_TABLE_PATH: &str = "/api/v1/pods?includeObject=None";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PodOverviewIn {
    pub context: String,
}

/// How many pods one node is running.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct NodePodCount {
    pub node: String,
    pub pods: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PodOverviewOut {
    /// Every pod in the cluster, whatever phase it is in.
    pub total: i64,
    /// One entry per node that is running at least one pod. A node absent
    /// from this list is running none — the list is complete, which is what
    /// lets the screen read a missing node as a genuine zero.
    ///
    /// Pods that have not been scheduled yet carry no node and are counted in
    /// [`Self::total`] only; there is no node whose count they belong to.
    pub by_node: Vec<NodePodCount>,
    /// Every pod that is not simply running, for core to judge.
    ///
    /// Deliberately NOT "the unhealthy pods": this is a superset, and the
    /// distinction is the point. `Succeeded` pods are in it and core flags
    /// none of them. Whether a pod needs attention is `podStatus`'s to say,
    /// from the `phase` and `waitingReason` on each summary here.
    pub unsettled: Vec<PodSummary>,
    /// Whether [`Self::unsettled`] is shorter than the truth — the cap bit, or
    /// a pod the READY column singled out could not be read. The screen says
    /// the list is short rather than presenting it as complete.
    pub truncated: bool,
}

/// One column of a server-printed table.
#[derive(Debug, Deserialize)]
struct TableColumn {
    name: String,
}

#[derive(Debug, Deserialize)]
struct TableRow {
    cells: Vec<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PodTable {
    column_definitions: Vec<TableColumn>,
    rows: Vec<TableRow>,
}

impl PodTable {
    /// Where a named column sits, or an error naming the column that is
    /// missing.
    ///
    /// Looked up by name on every request rather than assumed by position:
    /// the printer owns the column set, and reading NODE out of index 6
    /// because that is where it sits today would silently start counting
    /// IP addresses the day it moves.
    fn column(&self, name: &str) -> Result<usize, String> {
        self.column_definitions
            .iter()
            .position(|c| c.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| format!("the pod table carried no {name} column"))
    }

    fn cell(&self, row: &TableRow, at: usize) -> String {
        match row.cells.get(at) {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Null) | None => String::new(),
            Some(other) => other.to_string(),
        }
    }
}

/// The printer's word for "this pod is not on a node".
const NO_NODE: &str = "<none>";

/// Whether a READY cell (`1/1`, `0/2`) says a container is not ready.
///
/// A waiting container is never a ready one, so this is the necessary
/// condition for the pods `status.phase!=Running` cannot reach. A cell that
/// cannot be read counts as short of ready: fetching a pod needlessly costs
/// one small request, and skipping one costs a crash-looping pod nobody sees.
fn short_of_ready(cell: &str) -> bool {
    let Some((ready, total)) = cell.split_once('/') else {
        return true;
    };
    match (ready.trim().parse::<i64>(), total.trim().parse::<i64>()) {
        (Ok(ready), Ok(total)) => ready < total,
        _ => true,
    }
}

/// Fetch every pod as a printed row: one request, no pod bodies.
async fn pod_table(client: kube::Client) -> Result<PodTable, String> {
    let req = Request::get(PODS_TABLE_PATH)
        .header(http::header::ACCEPT, TABLE_ACCEPT)
        .body(Vec::new())
        .map_err(|e| e.to_string())?;
    client.request::<PodTable>(req).await.map_err(|e| e.to_string())
}

/// The overview's pod facts. See the module docs for why each one is fetched
/// the way it is.
async fn overview(client: kube::Client, timeout: Duration) -> Result<PodOverviewOut, String> {
    tokio::time::timeout(timeout, gather(client))
        .await
        .map_err(|_| "pod overview timed out".to_string())?
}

async fn gather(client: kube::Client) -> Result<PodOverviewOut, String> {
    let api: Api<Pod> = Api::all(client.clone());
    let phase_filtered = ListParams::default()
        .fields(NOT_RUNNING)
        .limit(UNSETTLED_CAP as u32);

    // Concurrent: neither answer depends on the other, and the by-name pass
    // below needs both before it can decide what is left to fetch.
    let (table, not_running) = futures::future::join(
        pod_table(client),
        async { api.list(&phase_filtered).await.map_err(|e| e.to_string()) },
    )
    .await;
    let table = table?;
    let not_running = not_running?;

    let name_at = table.column("Name")?;
    let ready_at = table.column("Ready")?;
    let node_at = table.column("Node")?;

    let total = table.rows.len() as i64;
    let mut counts: BTreeMap<String, i64> = BTreeMap::new();
    let mut short: Vec<String> = Vec::new();
    // How many pods in the whole cluster carry each name — every row, not just
    // the short ones. This is what makes the exclusion below sound; see there.
    let mut rows_by_name: HashMap<String, usize> = HashMap::new();
    for row in &table.rows {
        let node = table.cell(row, node_at);
        if !node.is_empty() && node != NO_NODE {
            *counts.entry(node).or_insert(0) += 1;
        }
        let name = table.cell(row, name_at);
        *rows_by_name.entry(name.clone()).or_insert(0) += 1;
        if short_of_ready(&table.cell(row, ready_at)) {
            short.push(name);
        }
    }
    let by_node = counts
        .into_iter()
        .map(|(node, pods)| NodePodCount { node, pods })
        .collect();

    // The phase selector's page is truncated when the server handed back a
    // continue token — with a field selector it does not send
    // `remainingItemCount`, so the token is the only thing that says there is
    // more, and a count of how much more is not available at any price.
    let mut truncated = not_running
        .metadata
        .continue_
        .as_deref()
        .is_some_and(|token| !token.is_empty());
    let mut unsettled: Vec<PodSummary> = not_running.items.into_iter().map(summarise_pod).collect();

    let fetched: HashSet<(String, String)> = unsettled
        .iter()
        .map(|p| (p.namespace.clone(), p.name.clone()))
        .collect();
    let fetched_names: HashSet<&str> = fetched.iter().map(|(_, name)| name.as_str()).collect();

    // **A name is only "already fetched" when the cluster has one pod by that
    // name**, and the table is what proves it: `rows_by_name` counted every
    // row, so a count of one means this row IS the pod the phase filter
    // returned. Two rows by that name and the exclusion says nothing — the pod
    // that came back may be either of them.
    //
    // The unguarded name check dropped an unhealthy pod in silence.
    // `payments/api-0` Pending is in the phase-filtered list, so the name
    // `api-0` looked accounted for and `checkout/api-0` in `CrashLoopBackOff`
    // — phase `Running`, invisible to that filter — was never fetched, never
    // in `unsettled`, with `truncated` left false. `fetched` is keyed by
    // `(namespace, name)` for exactly this reason; throwing the namespace away
    // to build the exclusion threw away the reason it is keyed that way.
    //
    // The guard is worth the two lines rather than dropping the exclusion
    // outright: a cluster with fifty finished Job pods has fifty short-of-ready
    // rows whose names are its own, and each would otherwise cost a request
    // for a body the one filtered list already brought back.
    let mut names: Vec<String> = short
        .into_iter()
        .filter(|n| {
            rows_by_name.get(n.as_str()).copied().unwrap_or(0) > 1
                || !fetched_names.contains(n.as_str())
        })
        .collect();
    names.sort();
    names.dedup();

    // A first pass at the cap, over NAMES, so a sick cluster does not issue
    // three thousand requests to fill a list of two hundred. It is a bound on
    // the asking, not on the answer — a name can come back as a pod per
    // namespace — so the append loop below caps the pods themselves.
    let budget = UNSETTLED_CAP.saturating_sub(unsettled.len());
    if names.len() > budget {
        truncated = true;
        names.truncate(budget);
    }

    // A name, not a namespace: `includeObject=None` keeps the rows free of
    // objects, so the printed row has no namespace on it. `metadata.name=`
    // across every namespace is what turns a name back into a pod, and it
    // brings back both of them when two namespaces run the same name.
    let results = futures::stream::iter(names.into_iter().map(|name| {
        let api = api.clone();
        async move {
            api.list(&ListParams::default().fields(&format!("metadata.name={name}")))
                .await
        }
    }))
    .buffer_unordered(CANDIDATE_CONCURRENCY)
    .collect::<Vec<_>>()
    .await;

    for result in results {
        match result {
            Ok(list) => {
                for pod in list.items {
                    let summary = summarise_pod(pod);
                    if fetched.contains(&(summary.namespace.clone(), summary.name.clone())) {
                        continue;
                    }
                    // [`UNSETTLED_CAP`] is a cap on the pods this returns, and
                    // the budget spent above could only bound the names asked
                    // for. `metadata.name=<name>` answers from every namespace
                    // at once, so two hundred reused names came back as more
                    // than two hundred pods — the large response this module
                    // exists to end, reported as `truncated: false`.
                    if unsettled.len() >= UNSETTLED_CAP {
                        truncated = true;
                        break;
                    }
                    unsettled.push(summary);
                }
            }
            // One pod we could not read is not a reason to lose the other
            // 199, but it IS a reason the list may be short — and saying so
            // is the whole difference between a summary and a claim.
            Err(_) => truncated = true,
        }
    }

    Ok(PodOverviewOut {
        total,
        by_node,
        unsettled,
        truncated,
    })
}

/// `k8s.podOverview` — pod totals, per-node counts and the pods that are not
/// simply running, for one context. See the module docs.
pub fn pod_overview_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<PodOverviewIn, PodOverviewOut, _, _>(
        "k8s.podOverview",
        "pod totals, per-node counts and the pods that are not running, without listing pod bodies",
        Annotations::READ_ONLY,
        move |input: PodOverviewIn| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                overview(client, request_timeout())
                    .await
                    .map_err(CapabilityError::Handler)
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn pod_overview_capability_has_id() {
        let cache = ClientCache::new(PathBuf::from("/x"));
        assert_eq!(pod_overview_capability(cache).id, "k8s.podOverview");
    }

    #[test]
    fn a_ready_cell_that_is_short_is_the_only_one_worth_fetching() {
        assert!(!short_of_ready("1/1"));
        assert!(!short_of_ready("3/3"));
        // No containers at all is not "one of them is waiting".
        assert!(!short_of_ready("0/0"));
        assert!(short_of_ready("0/1"));
        assert!(short_of_ready("1/2"));
        // A cell nobody can read is fetched rather than assumed healthy: one
        // needless request against one crash-looping pod nobody sees.
        assert!(short_of_ready(""));
        assert!(short_of_ready("unknown"));
    }
}

/// These tests run against a bare, hand-rolled HTTP server, for the reason
/// [`crate::pod_count`]'s do: `kube::Client::new` needs a `tower::Service` and
/// this crate carries no test double for one. The server routes on the request
/// line, so a test can assert *how* the capability asked — which is the whole
/// point here. Counting 5 416 pods correctly by listing all 5 416 of them
/// would pass every assertion about the numbers while being exactly the bug
/// this module was written to remove, so the requests themselves are pinned:
/// one Table request, one phase-filtered list, and a by-name fetch only for
/// the rows the READY column singled out.
#[cfg(test)]
mod pod_overview_tests {
    use super::*;
    use kube::{Client, Config};
    use std::sync::Mutex;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// One printed row: name, ready, status, restarts, age, ip, node.
    fn row(name: &str, ready: &str, status: &str, node: &str) -> serde_json::Value {
        serde_json::json!({ "cells": [name, ready, status, "0", "3d", "10.0.0.1", node, "<none>", "<none>"] })
    }

    fn table(rows: Vec<serde_json::Value>) -> String {
        serde_json::json!({
            "kind": "Table",
            "apiVersion": "meta.k8s.io/v1",
            "columnDefinitions": [
                { "name": "Name" }, { "name": "Ready" }, { "name": "Status" },
                { "name": "Restarts" }, { "name": "Age" }, { "name": "IP" },
                { "name": "Node" }, { "name": "Nominated Node" }, { "name": "Readiness Gates" },
            ],
            "rows": rows,
        })
        .to_string()
    }

    /// A pod body, as the API server would send it.
    fn pod(name: &str, namespace: &str, phase: &str, waiting: Option<&str>) -> serde_json::Value {
        let state = match waiting {
            Some(reason) => serde_json::json!({ "waiting": { "reason": reason } }),
            None => serde_json::json!({ "running": { "startedAt": "2024-01-01T00:00:00Z" } }),
        };
        serde_json::json!({
            "metadata": { "name": name, "namespace": namespace },
            "spec": { "nodeName": "n1" },
            "status": {
                "phase": phase,
                "containerStatuses": [
                    { "name": "c", "ready": waiting.is_none(), "restartCount": 0, "image": "acme/api:1", "imageID": "", "state": state }
                ],
            },
        })
    }

    fn pod_list(pods: Vec<serde_json::Value>, continue_token: Option<&str>) -> String {
        let mut meta = serde_json::Map::new();
        if let Some(token) = continue_token {
            meta.insert("continue".into(), serde_json::json!(token));
        }
        serde_json::json!({
            "apiVersion": "v1",
            "kind": "PodList",
            "metadata": serde_json::Value::Object(meta),
            "items": pods,
        })
        .to_string()
    }

    /// What each kind of request gets back, and a log of every request line
    /// and Accept header the capability actually sent.
    struct Server {
        client: Client,
        handle: tokio::task::JoinHandle<()>,
        seen: Arc<Mutex<Vec<(String, String)>>>,
    }

    impl Server {
        fn lines(&self) -> Vec<String> {
            self.seen.lock().unwrap().iter().map(|(line, _)| line.clone()).collect()
        }

        /// Request lines matching a substring — how a test says "and this is
        /// the only request of that shape that went out".
        fn asked(&self, needle: &str) -> Vec<String> {
            self.lines().into_iter().filter(|l| l.contains(needle)).collect()
        }

        fn accept_for(&self, needle: &str) -> String {
            self.seen
                .lock()
                .unwrap()
                .iter()
                .find(|(line, _)| line.contains(needle))
                .map(|(_, accept)| accept.clone())
                .unwrap_or_default()
        }
    }

    impl Drop for Server {
        fn drop(&mut self) {
            self.handle.abort();
        }
    }

    async fn serve(table_body: String, not_running: String, by_name: String) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen_task = seen.clone();
        let handle = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let table_body = table_body.clone();
                let not_running = not_running.clone();
                let by_name = by_name.clone();
                let seen = seen_task.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 16384];
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]).to_string();
                    let line = req.lines().next().unwrap_or("").to_string();
                    let accept = req
                        .lines()
                        .find(|l| l.to_ascii_lowercase().starts_with("accept:"))
                        .unwrap_or("")
                        .to_string();
                    seen.lock().unwrap().push((line.clone(), accept.clone()));
                    let body = if accept.contains("as=Table") {
                        table_body
                    } else if line.contains("metadata.name") {
                        by_name
                    } else {
                        not_running
                    };
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                });
            }
        });
        let config = Config::new(format!("http://{addr}").parse().unwrap());
        Server {
            client: Client::try_from(config).unwrap(),
            handle,
            seen,
        }
    }

    /// The property this module exists for: every pod is counted and grouped
    /// by node, and NOT ONE pod body is listed to do it.
    #[tokio::test(flavor = "multi_thread")]
    async fn counts_and_groups_every_pod_without_listing_one() {
        let server = serve(
            table(vec![
                row("api-1", "1/1", "Running", "n1"),
                row("api-2", "1/1", "Running", "n1"),
                row("web-1", "1/1", "Running", "n2"),
                // Not scheduled yet: counted in the total, on no node's row.
                row("queue-0", "0/1", "Pending", "<none>"),
            ]),
            pod_list(vec![pod("queue-0", "payments", "Pending", None)], None),
            pod_list(vec![], None),
        )
        .await;

        let out = overview(server.client.clone(), Duration::from_secs(5)).await.unwrap();

        assert_eq!(out.total, 4);
        assert_eq!(
            out.by_node,
            vec![
                NodePodCount { node: "n1".into(), pods: 2 },
                NodePodCount { node: "n2".into(), pods: 1 },
            ],
            "a pod with no node belongs to no node's count"
        );

        // Exactly one request asked for the printed table, and it asked for
        // rows with no objects stapled to them.
        let table_requests = server.asked("includeObject=None");
        assert_eq!(table_requests.len(), 1, "expected one table request, got {table_requests:?}");
        assert!(
            server.accept_for("includeObject=None").contains("as=Table"),
            "the counts must come from the printed table, asked: {}",
            server.accept_for("includeObject=None")
        );

        // And no request listed pods without narrowing them. `Api::list` with
        // no selector would produce the same counts from the same cluster
        // while shipping every pod's spec and status — this is the guard that
        // makes reaching for it a failing test.
        for line in server.lines() {
            if line.contains("includeObject=None") || !line.contains("/api/v1/pods") {
                continue;
            }
            assert!(
                line.contains("fieldSelector"),
                "a pod list went out with nothing narrowing it: {line}"
            );
        }
    }

    /// Every pod the PHASE condemns comes back with a body, so core can read
    /// its phase and its waiting reason — one server-side filter, not a scan.
    #[tokio::test(flavor = "multi_thread")]
    async fn carries_the_pods_whose_phase_is_not_running() {
        let server = serve(
            table(vec![
                row("ok-web-0", "1/1", "Running", "n1"),
                row("bb-queue-0", "0/1", "Pending", "n2"),
                row("done-backup-0", "0/1", "Completed", "n2"),
            ]),
            pod_list(
                vec![
                    pod("bb-queue-0", "payments", "Pending", None),
                    pod("done-backup-0", "ops", "Succeeded", None),
                ],
                None,
            ),
            pod_list(vec![], None),
        )
        .await;

        let out = overview(server.client.clone(), Duration::from_secs(5)).await.unwrap();

        let names: Vec<&str> = out.unsettled.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["bb-queue-0", "done-backup-0"]);
        // A superset, deliberately: `Succeeded` is here and core flags none of
        // them. What a pod MEANS is `podStatus`'s to say, not this module's.
        assert_eq!(out.unsettled[1].phase, "Succeeded");
        assert!(!out.truncated);

        let asked = server.asked("fieldSelector");
        assert_eq!(asked.len(), 1, "expected one filtered list, got {asked:?}");
        assert!(
            asked[0].contains("status.phase") && (asked[0].contains("%21%3D") || asked[0].contains("!=")),
            "the phase filter must be server-side and a not-equals: {}",
            asked[0]
        );
        // A healthy row is never fetched by name.
        assert!(server.asked("metadata.name").is_empty());
    }

    /// The case the whole second pass exists for: a `CrashLoopBackOff` pod's
    /// phase is `Running`, so `status.phase!=Running` cannot see it. The
    /// READY column can, and the pod is fetched by name so core gets the
    /// waiting reason it needs.
    #[tokio::test(flavor = "multi_thread")]
    async fn fetches_the_crash_looping_pod_the_phase_filter_cannot_reach() {
        let server = serve(
            table(vec![
                row("ok-web-0", "1/1", "Running", "n1"),
                row("aa-worker-0", "0/1", "CrashLoopBackOff", "n3"),
            ]),
            pod_list(vec![], None),
            pod_list(
                vec![pod("aa-worker-0", "checkout", "Running", Some("CrashLoopBackOff"))],
                None,
            ),
        )
        .await;

        let out = overview(server.client.clone(), Duration::from_secs(5)).await.unwrap();

        assert_eq!(out.unsettled.len(), 1);
        let pod = &out.unsettled[0];
        assert_eq!(pod.name, "aa-worker-0");
        assert_eq!(pod.namespace, "checkout");
        // The two fields `podStatus` reads. Without the second one core draws
        // this pod green.
        assert_eq!(pod.phase, "Running");
        assert_eq!(pod.waiting_reason, "CrashLoopBackOff");
        assert!(!out.truncated);

        let by_name = server.asked("metadata.name");
        assert_eq!(by_name.len(), 1, "only the short-of-ready row is fetched: {by_name:?}");
        assert!(by_name[0].contains("aa-worker-0"));
    }

    /// A pod the phase filter already brought back is not fetched a second
    /// time — which is what keeps a cluster with fifty finished Job pods from
    /// paying fifty extra requests for names it already has.
    #[tokio::test(flavor = "multi_thread")]
    async fn never_fetches_a_pod_it_already_has() {
        let server = serve(
            table(vec![
                row("done-1", "0/1", "Completed", "n1"),
                row("done-2", "0/1", "Completed", "n1"),
                row("done-3", "0/1", "Completed", "n2"),
            ]),
            pod_list(
                vec![
                    pod("done-1", "ops", "Succeeded", None),
                    pod("done-2", "ops", "Succeeded", None),
                    pod("done-3", "ops", "Succeeded", None),
                ],
                None,
            ),
            pod_list(vec![], None),
        )
        .await;

        let out = overview(server.client.clone(), Duration::from_secs(5)).await.unwrap();

        assert_eq!(out.unsettled.len(), 3);
        assert!(
            server.asked("metadata.name").is_empty(),
            "every short-of-ready row was already fetched: {:?}",
            server.asked("metadata.name")
        );
    }

    /// A name is not a pod. `payments/api-0` is Pending, so the phase filter
    /// already has it; `checkout/api-0` is in `CrashLoopBackOff` with phase
    /// `Running`, so the phase filter cannot see it and only a by-name fetch
    /// can. Treating the NAME `api-0` as already fetched dropped the
    /// crash-looper out of the overview with `truncated` left false — an
    /// unhealthy pod silently absent, which is the one thing the `Not ready`
    /// band exists to prevent.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_name_fetched_in_one_namespace_does_not_hide_the_same_name_in_another() {
        let server = serve(
            table(vec![
                row("api-0", "0/1", "Pending", "n1"),
                row("api-0", "0/1", "CrashLoopBackOff", "n2"),
            ]),
            pod_list(vec![pod("api-0", "payments", "Pending", None)], None),
            // What `metadata.name=api-0` answers: every namespace's api-0.
            pod_list(
                vec![
                    pod("api-0", "payments", "Pending", None),
                    pod("api-0", "checkout", "Running", Some("CrashLoopBackOff")),
                ],
                None,
            ),
        )
        .await;

        let out = overview(server.client.clone(), Duration::from_secs(5)).await.unwrap();

        let mut seen: Vec<(&str, &str)> = out
            .unsettled
            .iter()
            .map(|p| (p.namespace.as_str(), p.waiting_reason.as_str()))
            .collect();
        seen.sort();
        assert_eq!(
            seen,
            vec![("checkout", "CrashLoopBackOff"), ("payments", "")],
            "both namespaces' api-0 belong in the list"
        );
        // And the one the phase filter already had is in it once, not twice:
        // the namespace-qualified `fetched` check is what drops the duplicate.
        assert_eq!(out.unsettled.len(), 2);
        assert!(!out.truncated);

        let by_name = server.asked("metadata.name");
        assert_eq!(by_name.len(), 1, "one query answers the shared name: {by_name:?}");
        assert!(by_name[0].contains("api-0"));
    }

    /// The cap is over PODS, not over names. `metadata.name=<name>` asks every
    /// namespace at once — that is the whole reason the second pass can reach a
    /// pod whose printed row carried no namespace — so ONE name can come back
    /// as five pods. Spending the budget per name let those five past a cap of
    /// 200 while still reporting `truncated: false`: the large response this
    /// capability exists to prevent, presented as the whole truth.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_reused_name_cannot_push_the_list_past_the_cap() {
        // One short of the cap from the phase filter alone, so the by-name
        // pass has room for exactly one more pod — and the name it asks for is
        // run by five namespaces. The table carries only the row the READY
        // column singles out: `unsettled` takes the other 199 straight from
        // the phase-filtered list, which needs no rows here.
        let filler: Vec<serde_json::Value> = (1..UNSETTLED_CAP)
            .map(|i| pod(&format!("sick-{i}"), "ops", "Failed", None))
            .collect();
        assert_eq!(filler.len(), UNSETTLED_CAP - 1);
        let shared: Vec<serde_json::Value> = ["a", "b", "c", "d", "e"]
            .iter()
            .map(|ns| pod("api-0", ns, "Running", Some("CrashLoopBackOff")))
            .collect();
        let server = serve(
            table(vec![row("api-0", "0/1", "CrashLoopBackOff", "n1")]),
            pod_list(filler, None),
            pod_list(shared, None),
        )
        .await;

        let out = overview(server.client.clone(), Duration::from_secs(5)).await.unwrap();

        assert_eq!(
            out.unsettled.len(),
            UNSETTLED_CAP,
            "the cap holds over pods, not over the names that were asked for"
        );
        assert!(out.truncated, "a cap that bites is reported, never presented as the whole list");
    }

    /// A cap that bites is reported. A summary that is short and says nothing
    /// is read as the whole truth, which is the failure this module replaces.
    #[tokio::test(flavor = "multi_thread")]
    async fn says_so_when_there_is_more_than_it_will_return() {
        let server = serve(
            table(vec![row("sick-1", "0/1", "Error", "n1")]),
            pod_list(vec![pod("sick-1", "ops", "Failed", None)], Some("more-please")),
            pod_list(vec![], None),
        )
        .await;

        let out = overview(server.client.clone(), Duration::from_secs(5)).await.unwrap();
        assert!(out.truncated, "a continue token means the page was not the whole list");
        assert_eq!(out.unsettled.len(), 1);
    }

    /// A pod the READY column singled out and the server then refused leaves
    /// the list short — said out loud, not swallowed.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_pod_it_could_not_read_leaves_the_list_short_and_says_so() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let table_body = table(vec![row("aa-worker-0", "0/1", "CrashLoopBackOff", "n3")]);
        let empty = pod_list(vec![], None);
        let handle = tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let table_body = table_body.clone();
                let empty = empty.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 16384];
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]).to_string();
                    let line = req.lines().next().unwrap_or("").to_string();
                    let accept = req
                        .lines()
                        .find(|l| l.to_ascii_lowercase().starts_with("accept:"))
                        .unwrap_or("")
                        .to_string();
                    let response = if line.contains("metadata.name") {
                        "HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".to_string()
                    } else {
                        let body = if accept.contains("as=Table") { table_body } else { empty };
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        )
                    };
                    let _ = stream.write_all(response.as_bytes()).await;
                });
            }
        });
        let config = Config::new(format!("http://{addr}").parse().unwrap());
        let client = Client::try_from(config).unwrap();

        let out = overview(client, Duration::from_secs(5)).await.unwrap();
        // The counts still answered — one refused pod does not blank the
        // Pods tile or the per-node column.
        assert_eq!(out.total, 1);
        assert!(out.unsettled.is_empty());
        assert!(out.truncated, "a pod that could not be read makes the list short");
        handle.abort();
    }

    /// A table with no NODE column is an error, never 113 nodes reading zero.
    /// The printer owns the column set; a build that stopped sending NODE
    /// would otherwise have every node quietly claim it runs nothing.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_table_without_a_node_column_is_an_error_not_a_row_of_zeroes() {
        let narrow = serde_json::json!({
            "kind": "Table",
            "apiVersion": "meta.k8s.io/v1",
            "columnDefinitions": [{ "name": "Name" }, { "name": "Ready" }, { "name": "Status" }],
            "rows": [ { "cells": ["api-1", "1/1", "Running"] } ],
        })
        .to_string();
        let server = serve(narrow, pod_list(vec![], None), pod_list(vec![], None)).await;

        let err = overview(server.client.clone(), Duration::from_secs(5)).await.unwrap_err();
        assert!(err.contains("Node"), "expected the missing column named, got: {err}");
    }

    /// A cluster that did not answer has not told us it has no pods.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_server_that_never_answers_is_an_error_not_an_empty_cluster() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let _held = stream;
                    tokio::time::sleep(Duration::from_secs(30)).await;
                });
            }
        });
        let config = Config::new(format!("http://{addr}").parse().unwrap());
        let client = Client::try_from(config).unwrap();

        let err = overview(client, Duration::from_millis(50)).await.unwrap_err();
        assert!(err.contains("timed out"), "expected a timeout, got: {err}");
        handle.abort();
    }
}
