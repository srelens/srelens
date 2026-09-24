//! Performance budgets for the extension platform (#581).
//!
//! The roadmap sets three targets: the host loads the installed apps' manifests
//! in under 50 ms, the navigation apps contribute builds in under 10 ms (the
//! client's half, in `packages/ui-next/src/extensions/extensionBudgets.test.tsx`),
//! and a typical host call's own overhead stays under 5 ms. It also requires
//! batching, deduplication and cancellation, which are counts rather than times.
//!
//! So there are two kinds of assertion here:
//!
//! - **Counts are exact.** Resolving a column over 1,000 rows lists each joined
//!   reader once; closing a view releases every watch and stream it opened. A
//!   count does not depend on the machine, so it is held to the number.
//! - **Times are held to a ceiling, not to the target.** CI runs this suite as a
//!   debug build under coverage instrumentation, on a shared runner — many times
//!   slower than the release build the targets describe. A test that failed at
//!   the target there would fail on every run, and one that flakes teaches people
//!   to rerun it. The ceiling catches a change that makes a path an order of
//!   magnitude slower; the target is what every report compares against.
//!
//! With `SRELENS_PERF_REPORT_DIR` set, every measurement is written there as one
//! JSON file (`rust-<name>.json`): the median, the spread, the target, the
//! ceiling and the build it was measured in. The targets describe a release
//! build, so only a release build says whether one was met; a debug build's
//! report leaves `withinTarget` empty. CI's `extension budgets (release)` job
//! runs `cargo test --release -p srelens-registry --lib budget_tests` and
//! uploads the directory, so a slow drift shows up across runs long before it
//! reaches a ceiling.
use super::*;
use srelens_capability::audit::{NoopAudit, Source};
use srelens_kube::watch::KindSignal;
use srelens_streams::test_util::TestSink;
use std::future::Future;
use std::io::{BufRead, BufReader, Read as _, Write as _};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Where the reports go, when anywhere.
const REPORT_DIR: &str = "SRELENS_PERF_REPORT_DIR";

/// One measured path: what the roadmap asks of it, and what fails the test.
struct Budget {
    /// Stable across runs: the report's file name, and what a tracker keys on.
    name: &'static str,
    what: &'static str,
    /// The roadmap's number, for a release build. `None` for a path it names no
    /// number for, which is tracked all the same.
    target: Option<Duration>,
    /// What fails the test, in any build on any runner.
    ceiling: Duration,
}

/// The durations of a path's timed runs, after its warm-up runs.
struct Timing(Vec<Duration>);

impl Timing {
    fn sorted(&self) -> Vec<Duration> {
        let mut runs = self.0.clone();
        runs.sort();
        runs
    }
    fn median(&self) -> Duration {
        let runs = self.sorted();
        runs[runs.len() / 2]
    }
}

fn ms(duration: Duration) -> f64 {
    (duration.as_secs_f64() * 1_000_000.0).round() / 1_000.0
}

fn build() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

/// Write one report, when a directory was asked for.
fn report(name: &str, mut entry: Value) {
    let Ok(dir) = std::env::var(REPORT_DIR) else {
        return;
    };
    entry["suite"] = json!("srelens-registry");
    entry["name"] = json!(name);
    entry["build"] = json!(build());
    if let Ok(commit) = std::env::var("GITHUB_SHA") {
        entry["commit"] = json!(commit);
    }
    fs::create_dir_all(&dir).unwrap();
    let file = Path::new(&dir).join(format!("rust-{name}.json"));
    fs::write(file, serde_json::to_string_pretty(&entry).unwrap() + "\n").unwrap();
}

/// Held while a path is timed, so two budgets never measure each other.
static TIMING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Run `run` `warmup` times untimed, then `runs` times timed.
async fn time<F, Fut, T>(warmup: usize, runs: usize, mut run: F) -> Timing
where
    F: FnMut() -> Fut,
    Fut: Future<Output = T>,
{
    let _alone = TIMING.lock().await;
    for _ in 0..warmup {
        std::hint::black_box(run().await);
    }
    let mut taken = Vec::with_capacity(runs);
    for _ in 0..runs {
        let started = Instant::now();
        std::hint::black_box(run().await);
        taken.push(started.elapsed());
    }
    Timing(taken)
}

/// Report `timing` against `budget`, then hold its median to the ceiling.
fn hold(budget: &Budget, timing: &Timing, detail: Value) {
    let runs = timing.sorted();
    let median = timing.median();
    report(
        budget.name,
        json!({
            "kind": "timing",
            "what": budget.what,
            "unit": "ms",
            "runs": runs.len(),
            "medianMs": ms(median),
            "minMs": ms(runs[0]),
            "maxMs": ms(runs[runs.len() - 1]),
            "targetMs": budget.target.map(ms),
            "ceilingMs": ms(budget.ceiling),
            "withinTarget": budget
                .target
                .filter(|_| !cfg!(debug_assertions))
                .map(|target| median <= target),
            "detail": detail,
        }),
    );
    eprintln!(
        "budget {}: median {:.3} ms (min {:.3}, max {:.3}; target {}, ceiling {:.0} ms, {} build)",
        budget.name,
        ms(median),
        ms(runs[0]),
        ms(runs[runs.len() - 1]),
        budget
            .target
            .map_or("none".to_owned(), |t| format!("{:.0} ms", ms(t))),
        ms(budget.ceiling),
        build(),
    );
    assert!(
        median <= budget.ceiling,
        "{}: the median of {} runs took {:.3} ms, over the {:.0} ms ceiling ({} build). \
         The ceiling is far above the roadmap's target on purpose; a path this far over it \
         has regressed, so find the change rather than raise the number.",
        budget.what,
        runs.len(),
        ms(median),
        ms(budget.ceiling),
        build(),
    );
}

// ---------------------------------------------------------------------------
// Fifty apps
// ---------------------------------------------------------------------------

/// How many apps the inventory budgets are measured over.
const APPS: usize = 50;
/// Of those, how many are the Flux example — the largest manifest this
/// repository ships — and the rest the Argo CD example.
const FLUX_APPS: usize = 5;

/// A shipped example manifest under an unreserved ID of its own.
fn example(source: &str, id: &str, index: usize) -> String {
    let mut manifest: Value = serde_json::from_str(source).unwrap();
    manifest["id"] = json!(format!("org.example.{id}-{index}"));
    manifest["name"] = json!(format!("{} {index}", manifest["name"].as_str().unwrap()));
    manifest.to_string()
}

/// Install `count` apps from the two shipped examples, as a person would: each
/// through `extensions.configure`'s own path, with the grants it asks for, and
/// Flux in the same proportion as in `APPS`. Returns their IDs, sorted.
fn install_examples(path: &Path, count: usize) -> Vec<String> {
    super::tests::configure(
        path,
        json!({"action": "unsignedApps", "allowUnsignedApps": true}),
    )
    .unwrap();
    let mut ids = Vec::new();
    for index in 0..count {
        let (source, id) = if index < count * FLUX_APPS / APPS {
            (
                include_str!("../../../../examples/extensions/flux.json"),
                "flux",
            )
        } else {
            (
                include_str!("../../../../examples/extensions/argocd.json"),
                "argocd",
            )
        };
        let manifest = example(source, id, index);
        let parsed = Manifest::parse(&manifest).unwrap();
        super::tests::configure(
            path,
            json!({"action": "install", "manifest": manifest, "grants": parsed.permissions}),
        )
        .unwrap_or_else(|error| panic!("install app {index}: {error}"));
        ids.push(parsed.id);
    }
    ids.sort();
    ids
}

/// Loading the installed apps is what every app list, and — through
/// `resolver_app` — every column, card, panel and stream tick waits on: the
/// file is read, parsed and every manifest re-validated.
#[tokio::test(flavor = "multi_thread")]
async fn an_inventory_of_fifty_apps_loads_within_budget() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("extensions.json");
    let ids = install_examples(&path, APPS);
    let mut reg = Registry::new();
    super::register(
        &mut reg,
        path.clone(),
        super::tests::fake_core(),
        srelens_kube::client_cache::ClientCache::new_many(vec![]),
    );
    let listed = reg.invoke("extensions.list", json!({})).await.unwrap();
    let plugins = listed["plugins"].as_array().unwrap();
    assert_eq!(plugins.len(), APPS);
    // Every one of them loaded as installed: none quarantined, none held back.
    assert!(
        plugins
            .iter()
            .all(|p| p["enabled"] == true && p.get("quarantined").is_none()),
        "{listed}"
    );
    let mut listed_ids: Vec<_> = plugins
        .iter()
        .map(|p| p["manifest"]["id"].as_str().unwrap().to_owned())
        .collect();
    listed_ids.sort();
    assert_eq!(listed_ids, ids);
    let bytes = fs::metadata(&path).unwrap().len();
    let timing = time(3, 25, || reg.invoke("extensions.list", json!({}))).await;
    hold(
        &Budget {
            name: "inventory-load-50-apps",
            what: "extensions.list over an inventory of 50 installed apps",
            target: Some(Duration::from_millis(50)),
            ceiling: Duration::from_millis(1_000),
        },
        &timing,
        json!({"apps": APPS, "fluxApps": FLUX_APPS, "inventoryBytes": bytes}),
    );
}

/// A kubeconfig in `dir` whose one context, `name`, is the API server on `port`.
fn kubeconfig(dir: &Path, name: &str, port: u16) -> PathBuf {
    let path = dir.join("config");
    fs::write(
        &path,
        format!(
            "apiVersion: v1\nkind: Config\nclusters:\n- name: c\n  cluster:\n    server: http://127.0.0.1:{port}\n\
             users:\n- name: u\n  user:\n    token: t\ncontexts:\n- name: {name}\n  context:\n    cluster: c\n    user: u\n\
             current-context: {name}\n"
        ),
    )
    .unwrap();
    path
}

/// The host registry as `build_registry_*` assembles it, with the broker's CRD
/// check beneath it, serving the inventory at `path` through `kubeconfig`.
fn host(path: &Path, kubeconfig: PathBuf) -> (Registry, Arc<streams::ExtensionStreams>) {
    let cache = srelens_kube::client_cache::ClientCache::new_many(vec![kubeconfig.clone()]);
    let mut core = crate::build_registry_with_paths(cache.clone(), vec![kubeconfig]);
    core.register(crd::check_capability(cache.clone()));
    let mut registry = Registry::new();
    let streams = super::register(&mut registry, path.to_owned(), Arc::new(core), cache);
    (registry, streams)
}

/// A host call's own cost: the bridge's entry (`invoke_audited`, as a UI call)
/// through the broker's authorization — which resolves the context and loads
/// the inventory — to an answer, for a request with nothing to read. What a
/// real call adds on top is its own work, measured on its own below.
///
/// Measured over one installed app and over fifty, because the broker loads
/// every app to authorize one: the overhead grows with the inventory.
#[tokio::test(flavor = "multi_thread")]
async fn a_host_calls_own_overhead_is_within_budget_with_one_app_and_with_fifty() {
    let payload = |id: &str, revision: u64| {
        json!({"id": id, "revision": revision, "context": "prod", "namespace": "team",
            "kind": "/ConfigMap", "uids": [{"name": "settings", "namespace": "team", "row": {}}]})
    };
    for (apps, name) in [
        (1, "host-call-overhead-1-app"),
        (APPS, "host-call-overhead-50-apps"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        // Port 1: nothing listens, and nothing here may try to reach it.
        let (registry, _streams) = host(&path, kubeconfig(dir.path(), "prod", 1));
        let ids = install_examples(&path, apps);
        let state = read(&path).unwrap();
        assert_eq!(state.plugins.len(), apps);
        let app = &state.plugins[0];
        let call = || {
            registry.invoke_audited(
                "extensions.resolveColumns",
                payload(&app.manifest.id, app.revision),
                &NoopAudit,
                Source::Ui,
                "auto",
            )
        };
        let out = call().await.unwrap();
        // Nothing to resolve for this kind: an answer, reached without a read.
        assert_eq!(out["columns"], json!([]), "{out}");
        assert_eq!(out["cells"].as_array().unwrap().len(), 1, "{out}");
        assert!(ids.contains(&app.manifest.id));
        let timing = time(3, 25, call).await;
        hold(
            &Budget {
                name,
                what: "a host call's own overhead (extensions.resolveColumns with nothing to read)",
                target: Some(Duration::from_millis(5)),
                ceiling: Duration::from_millis(1_000),
            },
            &timing,
            json!({"apps": apps}),
        );
    }
}

// ---------------------------------------------------------------------------
// Column resolvers over 1,000 rows (#538)
// ---------------------------------------------------------------------------

/// How many rows one table resolves, the most `extensions.resolveColumns` takes.
const ROWS: usize = 1_000;
const COLUMNS_APP: &str = "org.example.budget-columns";

/// Two readers, three joins over them — two share one reader — and a column
/// or a badge through every join, all on Deployments.
fn columns_manifest() -> Value {
    let reader = |name: &str, group: &str, kind: &str| {
        json!({"name": name, "title": format!("List {name}"), "target": "k8s.listCustomResource",
            "arguments": {"group": group, "version": "v1alpha1", "plural": name, "kind": kind, "namespaced": true},
            "inputs": ["context", "namespace"]})
    };
    json!({
        "id": COLUMNS_APP, "name": "Budget columns", "version": "0.1.0", "srelensApiVersion": "^0.3",
        "kind": "declarative", "permissions": ["k8s.listCustomResource"],
        "capabilities": [
            reader("vulnerabilityreports", "aquasecurity.github.io", "VulnerabilityReport"),
            reader("applications", "argoproj.io", "Application"),
        ],
        "contributions": {
            "pages": [{"id": "reports", "title": "Reports", "capability": "vulnerabilityreports"}],
            "detailTabs": [], "detailLinks": [],
            "joins": [
                {"id": "vulns", "capability": "vulnerabilityreports",
                 "match": {"label": "trivy-operator.resource.name", "kindLabel": "trivy-operator.resource.kind"}},
                {"id": "vulns-by-annotation", "capability": "vulnerabilityreports",
                 "match": {"annotation": "srelens.test/workload"}},
                {"id": "sync", "capability": "applications", "match": {"name": true}},
            ],
            "tableColumns": [
                {"id": "critical", "title": "Critical CVEs", "forKinds": ["apps/Deployment"],
                 "source": {"join": "vulns", "jsonPath": ".report.summary.criticalCount"}, "format": "number", "sortable": true},
                {"id": "high", "title": "High CVEs", "forKinds": ["apps/Deployment"],
                 "source": {"join": "vulns-by-annotation", "jsonPath": ".report.summary.highCount"}, "format": "number"},
                {"id": "sync", "title": "Sync", "forKinds": ["apps/Deployment"],
                 "source": {"join": "sync", "jsonPath": ".status.sync.status"}, "format": "status"},
            ],
            "badges": [{"id": "argo", "forKinds": ["apps/Deployment"], "join": "sync",
                "rules": [{"when": [{"jsonPath": ".status.sync.status", "equals": "OutOfSync"}],
                           "status": "warning", "label": "Out of sync"}]}],
        }
    })
}

/// The `index`th object of `plural` in `namespace`: one per Deployment row.
fn joined_object(plural: &str, namespace: &str, index: usize) -> Value {
    match plural {
        "vulnerabilityreports" => json!({
            "apiVersion": "aquasecurity.github.io/v1alpha1", "kind": "VulnerabilityReport",
            "metadata": {"name": format!("report-{index}"), "namespace": namespace, "resourceVersion": "1",
                "labels": {"trivy-operator.resource.name": format!("api-{index}"),
                           "trivy-operator.resource.kind": "Deployment"},
                "annotations": {"srelens.test/workload": format!("api-{index}")}},
            "report": {"summary": {"criticalCount": index % 7, "highCount": index % 11}}}),
        _ => json!({
            "apiVersion": "argoproj.io/v1alpha1", "kind": "Application",
            "metadata": {"name": format!("api-{index}"), "namespace": namespace, "resourceVersion": "1"},
            "status": {"sync": {"status": if index % 3 == 0 { "OutOfSync" } else { "Synced" }}}}),
    }
}

/// An API server on loopback serving both readers' CRDs and `ROWS` objects of
/// each in every namespace, paged by `limit` and `continue` as a real one
/// pages. Records every request as `path?query`.
struct Cluster {
    port: u16,
    seen: Arc<Mutex<Vec<String>>>,
}

impl Cluster {
    fn start() -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let record = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    continue;
                }
                let target = line.split(' ').nth(1).unwrap_or("").to_owned();
                let mut length = 0;
                loop {
                    let mut header = String::new();
                    if reader.read_line(&mut header).is_err()
                        || header == "\r\n"
                        || header.is_empty()
                    {
                        break;
                    }
                    if let Some((name, value)) = header.split_once(':') {
                        if name.eq_ignore_ascii_case("content-length") {
                            length = value.trim().parse().unwrap_or(0);
                        }
                    }
                }
                let _ = reader.read_exact(&mut vec![0; length]);
                record.lock().unwrap().push(target.clone());
                let (status, body) = Self::route(&target);
                let text = body.to_string();
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}",
                    text.len()
                );
            }
        });
        Self { port, seen }
    }

    fn route(target: &str) -> (u16, Value) {
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        let param = |key: &str| {
            query
                .split('&')
                .find_map(|pair| pair.strip_prefix(&format!("{key}=")))
                .map(str::to_owned)
        };
        const CRDS: &str = "/apis/apiextensions.k8s.io/v1/customresourcedefinitions/";
        if let Some(name) = path.strip_prefix(CRDS) {
            let (plural, group, kind) = match name {
                "vulnerabilityreports.aquasecurity.github.io" => (
                    "vulnerabilityreports",
                    "aquasecurity.github.io",
                    "VulnerabilityReport",
                ),
                "applications.argoproj.io" => ("applications", "argoproj.io", "Application"),
                _ => {
                    return (
                        404,
                        json!({"kind": "Status", "code": 404, "reason": "NotFound"}),
                    )
                }
            };
            return (
                200,
                json!({"apiVersion": "apiextensions.k8s.io/v1", "kind": "CustomResourceDefinition",
                    "metadata": {"name": name},
                    "spec": {"group": group, "scope": "Namespaced", "names": {"plural": plural, "kind": kind},
                        "versions": [{"name": "v1alpha1", "served": true, "storage": true}]}}),
            );
        }
        // /apis/{group}/v1alpha1/namespaces/{namespace}/{plural}
        let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        let ["apis", group, "v1alpha1", "namespaces", namespace, plural] = parts[..] else {
            return (
                404,
                json!({"kind": "Status", "code": 404, "reason": "NotFound"}),
            );
        };
        let start: usize = param("continue").and_then(|t| t.parse().ok()).unwrap_or(0);
        let limit: usize = param("limit").and_then(|l| l.parse().ok()).unwrap_or(ROWS);
        let end = (start + limit).min(ROWS);
        let items: Vec<Value> = (start..end)
            .map(|index| joined_object(plural, namespace, index))
            .collect();
        let mut metadata = json!({"resourceVersion": "1"});
        if end < ROWS {
            metadata["continue"] = json!(end.to_string());
        }
        (
            200,
            json!({"apiVersion": format!("{group}/v1alpha1"), "kind": "List", "metadata": metadata, "items": items}),
        )
    }

    /// The requests for `plural`'s objects: `(lists, pages)`. A list is its
    /// first page — a page asked for without a `continue` token.
    fn lists_of(&self, plural: &str) -> (usize, usize) {
        let seen = self.seen.lock().unwrap();
        let pages: Vec<_> = seen
            .iter()
            .filter(|target| {
                target
                    .split('?')
                    .next()
                    .is_some_and(|path| path.ends_with(&format!("/{plural}")))
            })
            .collect();
        let lists = pages.iter().filter(|t| !t.contains("continue=")).count();
        (lists, pages.len())
    }

    /// CRD lookups: the broker's per-call check that each reader is served.
    fn discovery(&self) -> usize {
        let seen = self.seen.lock().unwrap();
        seen.iter()
            .filter(|t| t.contains("/customresourcedefinitions/"))
            .count()
    }
}

/// The caller's payload for `ROWS` Deployments in `namespace`, as
/// `useResolvedColumns` sends it.
fn thousand_rows(revision: u64, namespace: &str) -> Value {
    let uids: Vec<Value> = (0..ROWS)
        .map(|index| {
            json!({"uid": format!("uid-api-{index}"), "name": format!("api-{index}"),
                "namespace": namespace, "row": {}})
        })
        .collect();
    json!({"id": COLUMNS_APP, "revision": revision, "context": "budget", "namespace": namespace,
        "kind": "apps/Deployment", "uids": uids})
}

/// #538's acceptance: a 1,000-row Deployment list is one resolve call per
/// refresh, and that call lists each joined reader once — not once per row,
/// per column, per badge or per join. Four tables asking at once share the
/// lists in flight, and a refresh inside the snapshot's life lists nothing.
#[tokio::test(flavor = "multi_thread")]
async fn resolving_a_thousand_rows_lists_each_joined_reader_once() {
    let cluster = Cluster::start();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("extensions.json");
    let (registry, _streams) = host(&path, kubeconfig(dir.path(), "budget", cluster.port));
    let installed = registry
        .invoke(
            "extensions.configure",
            json!({"action": "install", "manifest": columns_manifest().to_string(),
                "grants": ["k8s.listCustomResource"]}),
        )
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let revision = installed["plugins"][0]["revision"].as_u64().unwrap();
    let resolve = |namespace: &'static str| {
        registry.invoke(
            "extensions.resolveColumns",
            thousand_rows(revision, namespace),
        )
    };

    // Four tables open on one list at once, then one of them refreshes.
    let (a, b, c, d) = tokio::join!(
        resolve("team"),
        resolve("team"),
        resolve("team"),
        resolve("team")
    );
    let refreshed = resolve("team").await.unwrap();
    for answer in [a, b, c, d] {
        assert_eq!(answer.unwrap(), refreshed);
    }
    let cells = refreshed["cells"].as_array().unwrap();
    assert_eq!(cells.len(), ROWS);
    for index in [0, 1, 2, 3, 500, 999] {
        let cell = &cells[index];
        assert_eq!(cell["name"], format!("api-{index}"), "{cell}");
        assert_eq!(
            cell["values"]["critical"],
            (index % 7).to_string(),
            "{cell}"
        );
        assert_eq!(cell["values"]["high"], (index % 11).to_string(), "{cell}");
        let out_of_sync = index % 3 == 0;
        assert_eq!(
            cell["values"]["sync"],
            if out_of_sync { "OutOfSync" } else { "Synced" },
            "{cell}"
        );
        assert_eq!(
            cell.get("badges").is_some(),
            out_of_sync,
            "a badge exactly where its rule holds: {cell}"
        );
    }

    let reports = cluster.lists_of("vulnerabilityreports");
    let applications = cluster.lists_of("applications");
    let calls = 5;
    let joins = 3;
    let discovery = cluster.discovery();
    // Reported before it is held, so a run that breaks it says by how much.
    report(
        "resolve-columns-lists-per-join",
        json!({
            "kind": "count",
            "what": "Kubernetes lists for five resolves of 1,000 rows (four concurrent, one refresh) over three joins on two readers",
            "rows": ROWS,
            "calls": calls,
            "joins": joins,
            "readers": 2,
            "lists": reports.0 + applications.0,
            "pages": reports.1 + applications.1,
            "expectedLists": 2,
            "crdLookups": discovery,
        }),
    );
    // One list per joined reader: two readers, three joins over them.
    assert_eq!(
        (reports.0, applications.0),
        (1, 1),
        "each joined reader is listed once for five resolves of {ROWS} rows"
    );
    // A list of 1,000 is two of the API server's 500-object pages, no more.
    assert_eq!((reports.1, applications.1), (2, 2));
    // The CRD check runs per call and per join, never per row.
    assert!(discovery <= calls * joins, "{discovery} CRD lookups");

    // The same refresh, timed: the rows are resolved against the snapshot the
    // lists above left, so this is the resolver's own work over 1,000 rows.
    let timing = time(1, 10, || resolve("team")).await;
    hold(
        &Budget {
            name: "resolve-columns-1000-rows-warm",
            what: "extensions.resolveColumns over 1,000 rows, three joined columns and a badge, from the snapshot",
            target: None,
            ceiling: Duration::from_millis(1_000),
        },
        &timing,
        json!({"rows": ROWS, "joins": joins, "columns": 3, "badges": 1}),
    );
}

// ---------------------------------------------------------------------------
// Closing a view (#565, #566)
// ---------------------------------------------------------------------------

/// The watch sessions the host has started, and how many it still holds. A
/// session is the future that owns one Kubernetes watch (`watch_kind_once` in
/// the host); dropping it is what cancels that watch.
#[derive(Default)]
struct Sessions {
    started: AtomicUsize,
    alive: AtomicUsize,
}

/// Held by one session's future: counted alive until the host drops it.
struct Alive(Arc<Sessions>);

impl Drop for Alive {
    fn drop(&mut self) {
        self.0.alive.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Sessions that list once and then follow forever, as a quiet cluster's do,
/// and are counted while the host holds them.
fn counted(sessions: Arc<Sessions>) -> streams::WatchSession {
    Arc::new(move |scope: streams::WatchScope| {
        sessions.started.fetch_add(1, Ordering::SeqCst);
        sessions.alive.fetch_add(1, Ordering::SeqCst);
        let alive = Alive(sessions.clone());
        Box::pin(async move {
            let _alive = alive;
            let _ = scope.signals.send(KindSignal::Listed);
            std::future::pending::<Result<(), String>>().await
        })
    })
}

async fn eventually(what: &str, check: impl Fn() -> bool) {
    for _ in 0..500 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("never happened: {what}");
}

/// A view that closes ends every stream it opened, and ends each of them all
/// the way: a watch's session is dropped, so its Kubernetes watch is cancelled,
/// and each stream's slot under the app's cap is free again. Views are opened
/// to the cap and closed in turn — one leaked stream would have the cap refuse
/// the next view — beside a view that stays open and is never touched.
#[tokio::test(flavor = "multi_thread")]
async fn closing_a_view_releases_every_watch_and_stream_it_opened() {
    const APP: &str = "org.example.argocd";
    const VIEWS: usize = 5;
    const WATCHES: usize = 3;
    const READS: usize = 3;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("extensions.json");
    let mut registry = Registry::new();
    let streams = super::register(
        &mut registry,
        path.clone(),
        super::tests::fake_core(),
        srelens_kube::client_cache::ClientCache::new_many(vec![]),
    );
    let revision = super::tests::install(&path, super::tests::fake_core());
    let sessions = Arc::new(Sessions::default());
    streams.script_watches(counted(sessions.clone()), streams::WatchTiming::default());
    let limits = registry
        .invoke("extensions.streams", json!({}))
        .await
        .unwrap();
    let cap = limits["maxOpenPerApp"].as_u64().unwrap() as usize;
    // Every stream's first frame counts against the app's message rate. The
    // rounds must fit in one second's worth, or a fast build has the host stop
    // one for its rate — correctly — and this measures the limiter instead.
    let rate = limits["messagesPerSecond"].as_u64().unwrap() as usize;
    assert!(
        VIEWS * (WATCHES + READS) + 2 <= rate,
        "the rounds' first frames must fit in the app's {rate} messages per second"
    );
    let sink = Arc::new(TestSink::default());
    let open = |view: String, channel: String, source: Value| {
        let (streams, sink) = (&streams, sink.clone());
        async move {
            streams
                .open(
                    sink,
                    json!({"id": APP, "revision": revision, "view": view, "channel": channel,
                        "context": "cluster/a", "namespace": "team", "source": source}),
                )
                .await
                .unwrap_or_else(|error| panic!("{error}"))
        }
    };
    let watch = json!({"kind": "watch", "capability": "applications"});
    // The longest interval: a read stream's one tick, then nothing until it is ended.
    let read = json!({"kind": "read", "capability": "applications", "intervalSeconds": 300});
    let open_now = |streams: &streams::ExtensionStreams| -> usize {
        streams.metrics().iter().map(|app| app.open_streams).sum()
    };
    // The view that stays: one of each, the cap's last two slots.
    open("stays".into(), "extstream:stays-w".into(), watch.clone()).await;
    open("stays".into(), "extstream:stays-r".into(), read.clone()).await;
    assert_eq!(WATCHES + READS + 2, cap, "each view fills the app's cap");

    let _alone = TIMING.lock().await;
    let mut closing = Vec::new();
    for round in 0..VIEWS {
        let view = format!("page#{round}");
        let mut channels = Vec::new();
        for (count, source, kind) in [(WATCHES, &watch, "w"), (READS, &read, "r")] {
            for n in 0..count {
                let channel = format!("extstream:{round}-{kind}{n}");
                open(view.clone(), channel.clone(), source.clone()).await;
                channels.push(channel);
            }
        }
        eventually("every watch listed and every read answered", || {
            sessions.alive.load(Ordering::SeqCst) == 1 + WATCHES
                && channels.iter().all(|channel| {
                    sink.payloads_for(channel)
                        .iter()
                        .any(|frame| frame["type"] == "data")
                })
        })
        .await;
        assert_eq!(open_now(&streams), cap);
        let started = Instant::now();
        let closed = streams.close_view(&view);
        closing.push(started.elapsed());
        assert_eq!(closed, WATCHES + READS, "round {round}");
        // Every watch this view opened is dropped; the other view's is not.
        eventually("the closed view's watches dropped", || {
            sessions.alive.load(Ordering::SeqCst) == 1
        })
        .await;
        assert_eq!(open_now(&streams), 2, "round {round}");
        for channel in &channels {
            assert_eq!(
                sink.payloads_for(channel).last().unwrap()["reason"],
                "viewClosed",
                "{channel}"
            );
        }
        assert_eq!(streams.close_view(&view), 0, "closing twice ends nothing");
    }
    drop(_alone);
    for channel in ["extstream:stays-w", "extstream:stays-r"] {
        assert!(
            sink.payloads_for(channel)
                .iter()
                .all(|frame| frame["type"] != "close"),
            "the view that stayed open was not touched: {channel}"
        );
    }
    // No watch was restarted behind the view's back: one session per watch opened.
    let started = sessions.started.load(Ordering::SeqCst);
    assert_eq!(started, 1 + VIEWS * WATCHES);
    assert_eq!(streams.close_view("stays"), 2);
    eventually("the last watch dropped", || {
        sessions.alive.load(Ordering::SeqCst) == 0
    })
    .await;
    assert_eq!(open_now(&streams), 0);
    report(
        "close-view-releases",
        json!({
            "kind": "count",
            "what": "streams and watch sessions still held after views are closed",
            "views": VIEWS,
            "watchesPerView": WATCHES,
            "readsPerView": READS,
            "watchSessionsStarted": started,
            "watchSessionsHeldAfterClose": sessions.alive.load(Ordering::SeqCst),
            "streamsOpenAfterClose": open_now(&streams),
        }),
    );
    hold(
        &Budget {
            name: "close-view-6-streams",
            what:
                "the host's side of closing a view that holds three watches and three read streams",
            target: Some(Duration::from_millis(5)),
            ceiling: Duration::from_millis(50),
        },
        &Timing(closing),
        json!({"streams": WATCHES + READS}),
    );
}
