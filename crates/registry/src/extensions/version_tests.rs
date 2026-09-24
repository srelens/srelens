//! A reader that accepts several served versions (#547), through every broker consumer.
//!
//! Each test installs one app against a real host registry whose kubeconfig points at
//! loopback API servers, so CRD discovery, the list, the object read and the patch are
//! real requests, and asserts the exact paths they went to. The HelmRelease at `v2beta2`
//! keeps its revision under another field than at `v2`: a consumer that reads the wrong
//! version, or the right one through the wrong path, comes back without the revision.
use super::*;
use std::io::{BufRead, BufReader, Read as _, Write as _};
use std::sync::Mutex;

const MOVED: &str = ".status.lastAttemptedRevision";
const THERE: &str = ".status.lastReleaseRevision";
const REVISION: &str = "4.2.0";
const APP: &str = "org.example.flux";

fn manifest() -> Value {
    json!({
        "id":APP, "name":"Flux", "version":"0.1.0", "srelensApiVersion":"^0.3",
        "kind":"declarative", "permissions":["k8s.listCustomResource","k8s.setFields"],
        "capabilities":[{"name":"helmreleases","title":"List Helm releases","target":"k8s.listCustomResource",
            "versions":["v2","v2beta2"],
            "jsonPathOverrides":{"v2beta2":{MOVED: THERE}},
            "arguments":{"group":"helm.toolkit.fluxcd.io","plural":"helmreleases","kind":"HelmRelease",
                "namespaced":true,"printerColumns":[{"name":"Revision","jsonPath":MOVED,"type":"string"}]},
            "inputs":["context","namespace"]}],
        "actions":[{"name":"suspend","title":"Suspend","target":"k8s.setFields","resource":"helmreleases",
            "arguments":{"fields":{"/spec/suspend":true}},
            "preconditions":[{"jsonPath":MOVED,"present":true,"reason":"Not yet released"}],
            "availableWhen":[{"jsonPath":MOVED,"present":true,"reason":"Not yet released"}]}],
        "contributions":{
            "pages":[{"id":"helmreleases","title":"Helm releases","capability":"helmreleases"}],
            "detailTabs":[],"detailLinks":[],
            "joins":[{"id":"release","capability":"helmreleases","match":{"name":true}}],
            "tableColumns":[{"id":"revision","title":"Revision","forKinds":["apps/Deployment"],
                "source":{"join":"release","jsonPath":MOVED},"format":"text"}],
            "badges":[{"id":"helm","forKinds":["apps/Deployment"],"join":"release",
                "rules":[{"when":[{"jsonPath":MOVED,"present":true}],"status":"healthy","label":"Helm","reason":MOVED}]}],
            "detailPanels":[
                {"id":"release","title":"Release","forKinds":["helm.toolkit.fluxcd.io/HelmRelease"],
                 "sections":[{"type":"fields","fields":[{"label":"Revision","jsonPath":MOVED}]}]},
                {"id":"deployment-release","title":"Release","forKinds":["apps/Deployment"],
                 "sections":[{"type":"fields","fields":[{"label":"Revision","jsonPath":MOVED,"join":"release"}]}]}],
            "statusResolvers":[{"forKinds":["helm.toolkit.fluxcd.io/HelmRelease"],
                "rules":[{"when":[{"jsonPath":MOVED,"present":true}],"status":"healthy","label":"Released","reason":MOVED},
                         {"when":[],"status":"unknown","label":"Unknown"}]}],
            "dashboardCards":[{"id":"released","title":"Released","size":"s","type":"list",
                "source":"helmreleases","list":{"jsonPath":MOVED}},
                {"id":"unreleased","title":"Never released","size":"s","type":"count",
                "source":"helmreleases","predicate":{"jsonPath":MOVED,"absent":true},
                "target":{"page":"helmreleases"}}]
        }
    })
}

/// The HelmRelease `team/web` as served at `version`.
fn helm_release(version: &str) -> Value {
    let status = if version == "v2beta2" {
        json!({ "lastReleaseRevision": REVISION })
    } else {
        json!({ "lastAttemptedRevision": REVISION })
    };
    json!({"apiVersion":format!("helm.toolkit.fluxcd.io/{version}"),"kind":"HelmRelease",
        "metadata":{"name":"web","namespace":"team","uid":"u-web","resourceVersion":"7",
            "creationTimestamp":"2026-09-01T00:00:00Z"},
        "spec":{"suspend":false},"status":status})
}

/// One loopback API server: a HelmRelease CRD serving `served` (changeable), and the
/// HelmRelease `team/web` at each served version. Records `METHOD path` of each request.
struct Cluster {
    port: u16,
    served: Arc<Mutex<Vec<&'static str>>>,
    seen: Arc<Mutex<Vec<String>>>,
}

impl Cluster {
    fn serving(served: &[&'static str]) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let served = Arc::new(Mutex::new(served.to_vec()));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (versions, record) = (served.clone(), seen.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    continue;
                }
                let mut parts = line.split(' ');
                let method = parts.next().unwrap_or("").to_owned();
                let target = parts.next().unwrap_or("").to_owned();
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
                let mut body = vec![0; length];
                let _ = reader.read_exact(&mut body);
                let path = target.split('?').next().unwrap_or("").to_owned();
                record.lock().unwrap().push(format!("{method} {path}"));
                let served = versions.lock().unwrap().clone();
                let (status, answer) = route(&method, &path, &served);
                let text = answer.to_string();
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}",
                    text.len()
                );
            }
        });
        Self { port, served, seen }
    }

    /// Serve these versions from now on, as after an operator upgrade.
    fn now_serving(&self, served: &[&'static str]) {
        *self.served.lock().unwrap() = served.to_vec();
    }

    /// Every HelmRelease request (not the CRD lookup) made so far.
    fn resource_requests(&self) -> Vec<String> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.contains("/apis/helm.toolkit.fluxcd.io/"))
            .cloned()
            .collect()
    }
}

fn route(method: &str, path: &str, served: &[&str]) -> (u16, Value) {
    let missing = (
        404,
        json!({"apiVersion":"v1","kind":"Status","status":"Failure","code":404,"reason":"NotFound","message":"not found"}),
    );
    if path == "/apis/apiextensions.k8s.io/v1/customresourcedefinitions/helmreleases.helm.toolkit.fluxcd.io" {
        let versions: Vec<Value> = ["v2beta1", "v2beta2", "v2"]
            .iter()
            .map(|name| json!({"name":name,"served":served.contains(name),"storage":*name == "v2"}))
            .collect();
        return (
            200,
            json!({"apiVersion":"apiextensions.k8s.io/v1","kind":"CustomResourceDefinition",
                "metadata":{"name":"helmreleases.helm.toolkit.fluxcd.io"},
                "spec":{"group":"helm.toolkit.fluxcd.io","scope":"Namespaced",
                    "names":{"plural":"helmreleases","kind":"HelmRelease"},"versions":versions}}),
        );
    }
    let Some(rest) = path.strip_prefix("/apis/helm.toolkit.fluxcd.io/") else {
        return missing;
    };
    let Some((version, rest)) = rest.split_once('/') else {
        return missing;
    };
    if !served.contains(&version) {
        return missing;
    }
    match (method, rest) {
        ("GET", "namespaces/team/helmreleases") => (
            200,
            json!({"apiVersion":format!("helm.toolkit.fluxcd.io/{version}"),"kind":"HelmReleaseList",
                "metadata":{"resourceVersion":"9"},"items":[helm_release(version)]}),
        ),
        ("GET" | "PATCH", "namespaces/team/helmreleases/web") => (200, helm_release(version)),
        _ => missing,
    }
}

/// A host registry whose kubeconfig names one context per cluster, with the app installed.
async fn host(dir: &Path, clusters: &[(&str, &Cluster)]) -> (Registry, u64) {
    let mut config = String::from("apiVersion: v1\nkind: Config\nclusters:\n");
    for (name, cluster) in clusters {
        config += &format!(
            "- name: {name}\n  cluster:\n    server: http://127.0.0.1:{}\n",
            cluster.port
        );
    }
    config += "users:\n- name: u\n  user:\n    token: t\ncontexts:\n";
    for (name, _) in clusters {
        config += &format!("- name: {name}\n  context:\n    cluster: {name}\n    user: u\n");
    }
    let kubeconfig = dir.join("config");
    fs::write(&kubeconfig, config).unwrap();
    let registry = crate::build_registry_with_paths_and_settings(
        srelens_kube::client_cache::ClientCache::new_many(vec![kubeconfig.clone()]),
        vec![kubeconfig],
        Some(dir.join("settings.json")),
    );
    // The app declares a write, which an unsigned app may make only with this on.
    registry
        .invoke(
            "extensions.configure",
            json!({"action":"unsignedApps","allowUnsignedApps":true}),
        )
        .await
        .unwrap();
    let installed = registry
        .invoke(
            "extensions.configure",
            json!({"action":"install","manifest":manifest().to_string(),
                "grants":["k8s.listCustomResource","k8s.setFields"]}),
        )
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let revision = installed["plugins"][0]["revision"].as_u64().unwrap();
    (registry, revision)
}

async fn read(registry: &Registry, revision: u64, context: &str) -> Result<Value, CapabilityError> {
    registry
        .invoke(
            "extensions.read",
            json!({"id":APP,"revision":revision,"capability":"helmreleases","context":context,"namespace":"team"}),
        )
        .await
}

async fn cards(registry: &Registry, revision: u64, context: &str) -> Value {
    registry
        .invoke(
            "extensions.resolveCards",
            json!({"id":APP,"revision":revision,"context":context,"namespaces":["team"]}),
        )
        .await
        .unwrap()
}

fn selection(revision: u64, context: &str) -> Value {
    json!({"id":APP,"revision":revision,"capability":"helmreleases","context":context,
        "namespace":"team","name":"web"})
}

const LIST_V2BETA2: &str = "GET /apis/helm.toolkit.fluxcd.io/v2beta2/namespaces/team/helmreleases";
const LIST_V2: &str = "GET /apis/helm.toolkit.fluxcd.io/v2/namespaces/team/helmreleases";

#[tokio::test(flavor = "multi_thread")]
async fn a_page_reads_the_first_listed_version_the_cluster_serves_through_its_overrides() {
    let dir = tempfile::tempdir().unwrap();
    let older = Cluster::serving(&["v2beta2"]);
    let (registry, revision) = host(dir.path(), &[("older", &older)]).await;
    let out = read(&registry, revision, "older").await.unwrap();
    assert_eq!(older.resource_requests(), [LIST_V2BETA2]);
    let row = &out["items"][0];
    // The printer column and the status rule both read the moved field.
    assert_eq!(row["columns"][0], REVISION, "{out}");
    assert_eq!(row["status"]["label"], "Released", "{out}");
    assert_eq!(row["status"]["reason"], REVISION, "{out}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_preferred_version_wins_whatever_order_the_crd_lists_them_in() {
    let dir = tempfile::tempdir().unwrap();
    // The CRD declares v2beta2 before v2; the binding prefers v2.
    let current = Cluster::serving(&["v2beta2", "v2"]);
    let (registry, revision) = host(dir.path(), &[("current", &current)]).await;
    let out = read(&registry, revision, "current").await.unwrap();
    assert_eq!(current.resource_requests(), [LIST_V2]);
    assert_eq!(out["items"][0]["columns"][0], REVISION, "{out}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cluster_serving_none_of_the_versions_is_refused_and_never_read() {
    let dir = tempfile::tempdir().unwrap();
    let ancient = Cluster::serving(&["v2beta1"]);
    let (registry, revision) = host(dir.path(), &[("ancient", &ancient)]).await;
    let refused = read(&registry, revision, "ancient")
        .await
        .unwrap_err()
        .to_string();
    assert!(
        refused.contains("helmreleases.helm.toolkit.fluxcd.io"),
        "{refused}"
    );
    assert!(refused.contains("v2, v2beta2"), "{refused}");
    for (capability, input) in [
        ("extensions.resource", selection(revision, "ancient")),
        (
            "extensions.action",
            json!({"resource":selection(revision, "ancient"),"action":"suspend","uid":"u-web","resourceVersion":"7"}),
        ),
    ] {
        assert!(
            registry.invoke(capability, input).await.is_err(),
            "{capability}"
        );
    }
    let out = cards(&registry, revision, "ancient").await;
    assert_eq!(out["cards"][0]["state"], "error", "{out}");
    // Not v2beta1, not anything: no listed version is served, so nothing is read.
    assert_eq!(ancient.resource_requests(), Vec::<String>::new());
}

#[tokio::test(flavor = "multi_thread")]
async fn inspection_and_a_declared_action_go_to_the_resolved_version() {
    let dir = tempfile::tempdir().unwrap();
    let older = Cluster::serving(&["v2beta2"]);
    let (registry, revision) = host(dir.path(), &[("older", &older)]).await;
    let detail = registry
        .invoke("extensions.resource", selection(revision, "older"))
        .await
        .unwrap();
    // The control is offered by the path at the version read.
    assert_eq!(
        detail["actionMeta"]["suspend"]["availableWhen"][0]["jsonPath"],
        THERE
    );
    let done = registry
        .invoke(
            "extensions.action",
            json!({"resource":selection(revision, "older"),"action":"suspend","uid":"u-web","resourceVersion":"7"}),
        )
        .await;
    // The precondition held only because it read the field where v2beta2 keeps it.
    done.unwrap();
    let object = "/apis/helm.toolkit.fluxcd.io/v2beta2/namespaces/team/helmreleases/web";
    assert_eq!(
        older.resource_requests(),
        [
            format!("GET {object}"),
            format!("GET {object}"),
            format!("PATCH {object}")
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn joined_columns_badges_and_panels_read_the_resolved_version() {
    let dir = tempfile::tempdir().unwrap();
    let older = Cluster::serving(&["v2beta2"]);
    let (registry, revision) = host(dir.path(), &[("older", &older)]).await;
    let out = registry
        .invoke(
            "extensions.resolveColumns",
            json!({"id":APP,"revision":revision,"context":"older","namespace":"team","kind":"apps/Deployment",
                "uids":[{"uid":"d-web","name":"web","namespace":"team","row":{}}]}),
        )
        .await
        .unwrap();
    let cell = &out["cells"][0];
    assert_eq!(cell["values"]["revision"], REVISION, "{out}");
    assert_eq!(cell["badges"][0]["label"], "Helm", "{out}");
    assert_eq!(cell["badges"][0]["reason"], REVISION, "{out}");
    assert_eq!(older.resource_requests(), [LIST_V2BETA2]);

    let deployment = json!({"apiVersion":"apps/v1","kind":"Deployment",
        "metadata":{"name":"web","namespace":"team","uid":"d-web"}});
    let panels = registry
        .invoke(
            "extensions.resolvePanels",
            json!({"id":APP,"revision":revision,"context":"older","namespace":"team",
                "kind":"apps/Deployment","resource":deployment}),
        )
        .await
        .unwrap();
    assert_eq!(
        panels["panels"][0]["sections"][0]["fields"][0]["value"], REVISION,
        "{panels}"
    );
    // Every list went to v2beta2; the second came from the shared five-second snapshot.
    assert!(older
        .resource_requests()
        .iter()
        .all(|request| request == LIST_V2BETA2));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_panel_on_the_resource_itself_reads_it_as_the_version_it_was_read_at() {
    let dir = tempfile::tempdir().unwrap();
    let older = Cluster::serving(&["v2beta2"]);
    let (registry, revision) = host(dir.path(), &[("older", &older)]).await;
    let panel = |resource: Value| {
        let registry = &registry;
        async move {
            let out = registry
                .invoke(
                    "extensions.resolvePanels",
                    json!({"id":APP,"revision":revision,"context":"older","namespace":"team",
                        "kind":"helm.toolkit.fluxcd.io/HelmRelease","resource":resource}),
                )
                .await
                .unwrap();
            out["panels"][0]["sections"][0]["fields"][0].clone()
        }
    };
    assert_eq!(panel(helm_release("v2beta2")).await["value"], REVISION);
    assert_eq!(panel(helm_release("v2")).await["value"], REVISION);
    // Read at a version the app does not list: its paths are not known to hold there.
    let unlisted = panel(helm_release("v2beta1")).await;
    assert_eq!(unlisted["value"], Value::Null, "{unlisted}");
    assert!(
        unlisted["error"].as_str().unwrap_or("").contains("v2beta1"),
        "{unlisted}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_card_counts_the_resolved_version_through_its_overrides() {
    let dir = tempfile::tempdir().unwrap();
    let older = Cluster::serving(&["v2beta2"]);
    let (registry, revision) = host(dir.path(), &[("older", &older)]).await;
    let out = cards(&registry, revision, "older").await;
    assert_eq!(out["cards"][0]["rows"][0]["value"], REVISION, "{out}");
    assert_eq!(older.resource_requests(), [LIST_V2BETA2]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cards_target_page_keeps_what_the_card_counted_at_the_resolved_version() {
    let dir = tempfile::tempdir().unwrap();
    let older = Cluster::serving(&["v2beta2"]);
    let (registry, revision) = host(dir.path(), &[("older", &older)]).await;
    // `web` has been released: at v2beta2 its revision is where the override says.
    let out = cards(&registry, revision, "older").await;
    assert_eq!(
        out["cards"][1],
        json!({"id":"unreleased","state":"count","count":0}),
        "{out}"
    );
    let page = registry
        .invoke(
            "extensions.read",
            json!({"id":APP,"revision":revision,"capability":"helmreleases","context":"older",
                "namespace":"team","card":"unreleased"}),
        )
        .await
        .unwrap();
    assert_eq!(page["items"], json!([]), "{page}");
    assert!(older
        .resource_requests()
        .iter()
        .all(|request| request == LIST_V2BETA2));
}

#[tokio::test(flavor = "multi_thread")]
async fn another_accepted_version_or_a_moved_checked_path_is_a_permission_change() {
    let dir = tempfile::tempdir().unwrap();
    let older = Cluster::serving(&["v2beta2"]);
    let (registry, _) = host(dir.path(), &[("older", &older)]).await;
    let diff = |manifest: Value| {
        let registry = &registry;
        async move {
            let report = registry
                .invoke(
                    "extensions.validate",
                    json!({"manifest":manifest.to_string(),
                        "grants":["k8s.listCustomResource","k8s.setFields"]}),
                )
                .await
                .unwrap();
            assert_eq!(report["errors"], json!([]), "{report}");
            report["permissionDiff"]["added"].clone()
        }
    };
    assert_eq!(diff(manifest()).await, json!([]));
    // Reading one more API version reads more than was reviewed.
    let mut wider = manifest();
    wider["capabilities"][0]["versions"] = json!(["v2", "v2beta2", "v2beta1"]);
    let added = diff(wider).await;
    assert!(added.to_string().contains("v2beta1"), "{added}");
    // At v2beta2 the precondition now checks another field.
    let mut moved = manifest();
    moved["capabilities"][0]["jsonPathOverrides"]["v2beta2"][MOVED] = json!(".status.history");
    let added = diff(moved).await;
    assert!(added.to_string().contains(".status.history"), "{added}");
}

#[tokio::test(flavor = "multi_thread")]
async fn each_cluster_resolves_its_own_version_and_none_leaks_into_another() {
    let dir = tempfile::tempdir().unwrap();
    let current = Cluster::serving(&["v2", "v2beta2"]);
    let older = Cluster::serving(&["v2beta2"]);
    let (registry, revision) = host(dir.path(), &[("current", &current), ("older", &older)]).await;
    for context in ["current", "older", "current", "older"] {
        let out = read(&registry, revision, context).await.unwrap();
        assert_eq!(out["items"][0]["columns"][0], REVISION, "{context}: {out}");
        let out = cards(&registry, revision, context).await;
        assert_eq!(
            out["cards"][0]["rows"][0]["value"], REVISION,
            "{context}: {out}"
        );
    }
    assert!(
        current.resource_requests().iter().all(|r| r == LIST_V2),
        "{:?}",
        current.resource_requests()
    );
    assert!(
        older.resource_requests().iter().all(|r| r == LIST_V2BETA2),
        "{:?}",
        older.resource_requests()
    );
    assert!(!current.resource_requests().is_empty() && !older.resource_requests().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_discovery_change_is_followed_on_the_next_read_even_within_the_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let cluster = Cluster::serving(&["v2"]);
    let (registry, revision) = host(dir.path(), &[("prod", &cluster)]).await;
    let out = cards(&registry, revision, "prod").await;
    assert_eq!(out["cards"][0]["rows"][0]["value"], REVISION, "{out}");
    // The operator is rolled back: v2 is no longer served. Within the snapshot's five
    // seconds, a card must not read the v2 objects it holds through v2beta2's paths.
    cluster.now_serving(&["v2beta2"]);
    let out = cards(&registry, revision, "prod").await;
    assert_eq!(out["cards"][0]["rows"][0]["value"], REVISION, "{out}");
    assert_eq!(cluster.resource_requests(), [LIST_V2, LIST_V2BETA2]);
}
