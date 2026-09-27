//! Logs, exec and port-forwards for apps (#567), through the broker: every
//! session is held to its binding's pod scope, exec runs only what the manifest
//! fixes and only once the host confirmation named it, sessions are recorded,
//! and closing a view ends every stream and forward it opened.
//!
//! The cluster is scripted ([`Cluster`]); everything else is the host's.

use super::pods::{ExecAsk, KindRef, LogEvent, LogsAsk, PodCluster};
use super::streams::{ExtensionStreams, PodTiming};
use super::tests::{configure, fake_core};
use serde_json::{json, Value};
use srelens_capability::audit::{AuditRecord, AuditSink};
use srelens_capability::Registry;
use srelens_kube::app_pods::{
    ContainerPort, Output, PodFacts, PortTarget, ReadError, ServiceFacts, ServicePort, Upstream,
};
use srelens_streams::test_util::TestSink;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc::UnboundedSender;

const APP: &str = "org.example.certmanager";

/// A cert-manager app: it reads Deployments and Argo Rollouts, and reaches the
/// pods they select; one more logs binding reaches any pod in `cert-manager`.
fn manifest() -> String {
    json!({
        "id": APP, "name":"cert-manager", "version":"0.1.0", "srelensApiVersion":"^0.5",
        "kind":"declarative",
        "permissions":["k8s.listDeployments", "k8s.listCustomResource",
            {"capability":"k8s.streamLogs","namespaces":["cert-manager"]},
            "k8s.exec", "k8s.portForward"],
        "capabilities":[
            {"name":"controllers","title":"Controllers","target":"k8s.listDeployments",
                "inputs":["context","namespace"],"arguments":{}},
            {"name":"applications","title":"Applications","target":"k8s.listCustomResource",
                "inputs":["context","namespace"],
                "arguments":{"group":"argoproj.io","version":"v1alpha1","plural":"applications",
                    "kind":"Application","namespaced":true}},
            {"name":"controllerLogs","title":"Controller logs","target":"k8s.streamLogs",
                "inputs":[],"arguments":{"resource":"controllers"}},
            {"name":"appLogs","title":"Application logs","target":"k8s.streamLogs",
                "inputs":[],"arguments":{"resource":"applications","selector":".spec.workload.selector"}},
            {"name":"namespaceLogs","title":"Logs in cert-manager","target":"k8s.streamLogs",
                "inputs":[],"arguments":{}},
            {"name":"status","title":"cmctl status","target":"k8s.exec","inputs":[],
                "arguments":{"resource":"controllers","container":"controller",
                    "command":["cmctl","status","certificate","--all-namespaces"]}},
            {"name":"metrics","title":"Controller metrics","target":"k8s.portForward","inputs":[],
                "arguments":{"resource":"controllers","port":9402}},
            {"name":"webhook","title":"Webhook","target":"k8s.portForward","inputs":[],
                "arguments":{"resource":"controllers","port":443,"service":true}}
        ],
        "contributions":{"pages":[],"detailTabs":[],"detailLinks":[]}
    })
    .to_string()
}

const GRANTS: &[&str] = &[
    "k8s.listDeployments",
    "k8s.listCustomResource",
    "k8s.streamLogs",
    "k8s.exec",
    "k8s.portForward",
];

const COMMAND: &[&str] = &["cmctl", "status", "certificate", "--all-namespaces"];

// ---- A scripted cluster ----

/// What a scripted log session does.
#[derive(Clone)]
enum LogStep {
    Line(&'static str),
    Lines(usize),
    /// End the session, as the cluster ending the stream would.
    End,
    Fail(&'static str),
}

fn pod(name: &str, namespace: &str, labels: &[(&str, &str)]) -> PodFacts {
    PodFacts {
        name: name.into(),
        namespace: namespace.into(),
        labels: labels
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect(),
        containers: vec!["controller".into(), "sidecar".into()],
        ports: vec![ContainerPort {
            name: Some("https".into()),
            port: 10250,
        }],
        phase: Some("Running".into()),
        ready: true,
        deleting: false,
    }
}

#[derive(Default)]
struct Cluster {
    objects: Mutex<BTreeMap<(String, String, String), Value>>,
    pods: Mutex<Vec<PodFacts>>,
    services: Mutex<Vec<ServiceFacts>>,
    logs: Mutex<std::collections::VecDeque<Vec<LogStep>>>,
    log_asks: Mutex<Vec<LogsAsk>>,
    exec_output: Mutex<Vec<(Output, &'static str)>>,
    exec_exit: Mutex<Option<Result<i32, String>>>,
    exec_asks: Mutex<Vec<ExecAsk>>,
    /// Upstream connections open now, and ever opened.
    open: Arc<AtomicUsize>,
    connected: Mutex<Vec<(String, u16)>>,
    /// Exec never ends by itself when set.
    exec_hangs: std::sync::atomic::AtomicBool,
    /// Every port-forward connection is refused with this, when set.
    refuse_connect: Mutex<Option<&'static str>>,
    /// Object reads so far, and which of them (counted from 1) fail, and how.
    object_reads: AtomicUsize,
    failed_reads: Mutex<BTreeMap<usize, ReadError>>,
}

/// Counts itself out of `open` when the connection it echoes on ends.
struct Live(Arc<AtomicUsize>);
impl Drop for Live {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Cluster {
    /// The web Deployment in `team`, whose pods are `app=web`; `web-1` and
    /// `web-2` are two of them, `db-1` is not, and `stray` is `app=web` in
    /// another namespace.
    fn standard() -> Arc<Self> {
        let cluster = Self::default();
        cluster.objects.lock().unwrap().insert(
            ("Deployment".into(), "team".into(), "web".into()),
            json!({"metadata":{"name":"web","namespace":"team"},
                   "spec":{"selector":{"matchLabels":{"app":"web"}}}}),
        );
        cluster.objects.lock().unwrap().insert(
            ("Application".into(), "team".into(), "shop".into()),
            json!({"metadata":{"name":"shop","namespace":"team"},
                   "spec":{"workload":{"selector":{"matchExpressions":[
                       {"key":"app","operator":"In","values":["web"]}]}}}}),
        );
        *cluster.pods.lock().unwrap() = vec![
            pod("web-1", "team", &[("app", "web")]),
            pod("web-2", "team", &[("app", "web")]),
            pod("db-1", "team", &[("app", "db")]),
            pod("stray", "other", &[("app", "web")]),
            pod("webhook-1", "cert-manager", &[("app", "webhook")]),
        ];
        *cluster.services.lock().unwrap() = vec![
            ServiceFacts {
                name: "web".into(),
                namespace: "team".into(),
                selector: [("app".to_owned(), "web".to_owned())].into(),
                ports: vec![ServicePort {
                    name: Some("https".into()),
                    port: 443,
                    target: PortTarget::Name("https".into()),
                }],
            },
            ServiceFacts {
                name: "db".into(),
                namespace: "team".into(),
                selector: [("app".to_owned(), "db".to_owned())].into(),
                ports: vec![ServicePort {
                    name: None,
                    port: 443,
                    target: PortTarget::Number(5432),
                }],
            },
        ];
        Arc::new(cluster)
    }

    fn remove_pod(&self, name: &str) {
        self.pods.lock().unwrap().retain(|p| p.name != name);
    }
}

#[async_trait::async_trait]
impl PodCluster for Cluster {
    async fn object(
        &self,
        _context: &str,
        kind: &KindRef,
        namespace: &str,
        name: &str,
    ) -> Result<Option<Value>, ReadError> {
        let read = self.object_reads.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some(failure) = self.failed_reads.lock().unwrap().get(&read) {
            return Err(failure.clone());
        }
        Ok(self
            .objects
            .lock()
            .unwrap()
            .get(&(kind.kind.clone(), namespace.to_owned(), name.to_owned()))
            .cloned())
    }
    async fn pods(
        &self,
        _context: &str,
        namespace: &str,
        _labels: &str,
    ) -> Result<(Vec<PodFacts>, bool), ReadError> {
        // Every pod in the namespace, whatever the query: the host must match
        // them itself, and these tests would catch it trusting the cluster.
        Ok((
            self.pods
                .lock()
                .unwrap()
                .iter()
                .filter(|p| p.namespace == namespace)
                .cloned()
                .collect(),
            false,
        ))
    }
    async fn pod(
        &self,
        _context: &str,
        namespace: &str,
        name: &str,
    ) -> Result<Option<PodFacts>, ReadError> {
        Ok(self
            .pods
            .lock()
            .unwrap()
            .iter()
            .find(|p| p.namespace == namespace && p.name == name)
            .cloned())
    }
    async fn services(
        &self,
        _context: &str,
        namespace: &str,
    ) -> Result<Vec<ServiceFacts>, ReadError> {
        Ok(self
            .services
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.namespace == namespace)
            .cloned()
            .collect())
    }
    async fn logs(&self, ask: &LogsAsk, events: UnboundedSender<LogEvent>) -> Result<(), String> {
        self.log_asks.lock().unwrap().push(ask.clone());
        let script = self.logs.lock().unwrap().pop_front();
        let _ = events.send(LogEvent::Connected);
        for step in script.unwrap_or_default() {
            match step {
                LogStep::Line(line) => {
                    let _ = events.send(LogEvent::Line(line.into()));
                }
                LogStep::Lines(n) => {
                    for i in 0..n {
                        let _ = events.send(LogEvent::Line(format!("line {i}")));
                    }
                }
                LogStep::End => return Ok(()),
                LogStep::Fail(why) => return Err(why.into()),
            }
        }
        std::future::pending().await
    }
    async fn exec(
        &self,
        ask: &ExecAsk,
        output: UnboundedSender<(Output, String)>,
    ) -> Result<i32, String> {
        self.exec_asks.lock().unwrap().push(ask.clone());
        for (stream, text) in self.exec_output.lock().unwrap().iter() {
            let _ = output.send((*stream, (*text).to_owned()));
        }
        if self.exec_hangs.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        self.exec_exit.lock().unwrap().clone().unwrap_or(Ok(0))
    }
    async fn connect(
        &self,
        _context: &str,
        _namespace: &str,
        pod: &str,
        port: u16,
    ) -> Result<Box<dyn Upstream>, String> {
        self.connected.lock().unwrap().push((pod.to_owned(), port));
        if let Some(why) = *self.refuse_connect.lock().unwrap() {
            return Err(why.into());
        }
        // The far side echoes what it is sent, for as long as the connection lives.
        let (near, mut far) = tokio::io::duplex(4096);
        self.open.fetch_add(1, Ordering::SeqCst);
        let live = Live(self.open.clone());
        tokio::spawn(async move {
            let _live = live;
            let mut buf = [0u8; 1024];
            while let Ok(n) = far.read(&mut buf).await {
                if n == 0 || far.write_all(&buf[..n]).await.is_err() {
                    break;
                }
            }
        });
        Ok(Box::new(near))
    }
}

// ---- The host under test ----

const FAST: PodTiming = PodTiming {
    batch: Duration::from_millis(20),
    reconnect: Duration::from_millis(10),
    exec_limit: Duration::from_secs(5),
    monitor: Duration::from_millis(20),
};

#[derive(Default)]
struct Trail(Mutex<Vec<AuditRecord>>);
impl AuditSink for Trail {
    fn record(&self, rec: AuditRecord) {
        self.0.lock().unwrap().push(rec);
    }
}
impl Trail {
    fn seen(&self) -> Vec<AuditRecord> {
        self.0.lock().unwrap().clone()
    }
}

struct Host {
    _dir: tempfile::TempDir,
    path: PathBuf,
    registry: Registry,
    streams: Arc<ExtensionStreams>,
    cluster: Arc<Cluster>,
    sink: Arc<TestSink>,
    trail: Arc<Trail>,
    revision: u64,
}

fn install(path: &Path) -> u64 {
    configure(
        path,
        json!({"action": "unsignedApps", "allowUnsignedApps": true}),
    )
    .unwrap();
    let state = configure(
        path,
        json!({"action": "install", "manifest": manifest(), "grants": GRANTS}),
    )
    .unwrap_or_else(|e| panic!("install: {e}"));
    state
        .plugins
        .iter()
        .find(|p| p.manifest.id == APP)
        .unwrap()
        .revision
}

fn host() -> Host {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("extensions.json");
    let mut registry = Registry::new();
    let streams = super::register(
        &mut registry,
        path.clone(),
        fake_core(),
        srelens_kube::client_cache::ClientCache::new_many(vec![]),
    );
    let cluster = Cluster::standard();
    streams.script_pods(cluster.clone(), FAST);
    let revision = install(&path);
    Host {
        _dir: dir,
        path,
        registry,
        streams,
        cluster,
        sink: Arc::new(TestSink::default()),
        trail: Arc::new(Trail::default()),
        revision,
    }
}

impl Host {
    fn request(&self, view: &str, channel: &str, namespace: &str, source: Value) -> Value {
        json!({
            "id": APP, "revision": self.revision, "view": view, "channel": channel,
            "context": "cluster/a", "namespace": namespace, "source": source,
        })
    }

    async fn open(
        &self,
        view: &str,
        channel: &str,
        namespace: &str,
        source: Value,
    ) -> Result<String, String> {
        self.streams
            .open_audited(
                self.sink.clone(),
                self.trail.clone(),
                self.request(view, channel, namespace, source),
            )
            .await
            .map(|out| out.stream)
    }

    fn frames(&self, channel: &str) -> Vec<Value> {
        self.sink.payloads_for(channel)
    }

    fn data(&self, channel: &str) -> Vec<Value> {
        self.frames(channel)
            .into_iter()
            .filter(|f| f["type"] == "data")
            .map(|f| f["data"].clone())
            .collect()
    }

    fn last(&self, channel: &str) -> Value {
        self.frames(channel).last().cloned().unwrap_or(Value::Null)
    }
}

async fn eventually(what: &str, check: impl Fn() -> bool) {
    for _ in 0..300 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("never happened: {what}");
}

fn logs(capability: &str, name: Option<&str>, pod: &str) -> Value {
    let mut source =
        json!({"kind": "logs", "capability": capability, "pod": pod, "container": "controller"});
    if let Some(name) = name {
        source["name"] = json!(name);
    }
    source
}

fn exec(pod: &str, confirmed: Option<Value>) -> Value {
    let mut source = json!({"kind": "exec", "capability": "status", "name": "web", "pod": pod});
    if let Some(confirmed) = confirmed {
        source["confirmed"] = confirmed;
    }
    source
}

fn confirmed(pod: &str) -> Value {
    json!({"pod": pod, "container": "controller", "command": COMMAND})
}

// ---- Scope ----

/// The heart of it: a pod the object's own selector does not select is
/// refused — in the object's namespace, or selected but elsewhere — and so is
/// a pod outside the namespaces a permission grants. Nothing is opened.
#[tokio::test(flavor = "multi_thread")]
async fn a_pod_outside_the_selector_or_the_grant_is_refused() {
    let host = host();
    host.cluster
        .logs
        .lock()
        .unwrap()
        .push_back(vec![LogStep::Line("ok")]);
    host.open(
        "v",
        "extstream:in",
        "team",
        logs("controllerLogs", Some("web"), "web-1"),
    )
    .await
    .expect("a pod the Deployment selects");
    let refused = host
        .open(
            "v",
            "extstream:db",
            "team",
            logs("controllerLogs", Some("web"), "db-1"),
        )
        .await
        .unwrap_err();
    assert!(
        refused.contains("db-1") && refused.contains("not selected by Deployment web"),
        "{refused}"
    );
    // `stray` carries the labels, but in another namespace; the object's namespace
    // is the scope's, whatever the view says.
    let refused = host
        .open(
            "v",
            "extstream:stray",
            "other",
            logs("controllerLogs", Some("web"), "stray"),
        )
        .await
        .unwrap_err();
    assert!(
        refused.contains("Deployment other/web does not exist"),
        "{refused}"
    );
    // A custom resource's selector, read where the binding says, with expressions.
    host.open(
        "v",
        "extstream:app",
        "team",
        logs("appLogs", Some("shop"), "web-2"),
    )
    .await
    .expect("a pod the Application's selector selects");
    let refused = host
        .open(
            "v",
            "extstream:appdb",
            "team",
            logs("appLogs", Some("shop"), "db-1"),
        )
        .await
        .unwrap_err();
    assert!(
        refused.contains("not selected by Application shop"),
        "{refused}"
    );
    // The namespace grant: cert-manager, and nothing else.
    host.open(
        "v",
        "extstream:ns",
        "cert-manager",
        logs("namespaceLogs", None, "webhook-1"),
    )
    .await
    .expect("a pod in the granted namespace");
    let refused = host
        .open(
            "v",
            "extstream:ns2",
            "team",
            logs("namespaceLogs", None, "web-1"),
        )
        .await
        .unwrap_err();
    assert!(
        refused.contains("in cert-manager, not in team"),
        "{refused}"
    );
    for channel in [
        "extstream:db",
        "extstream:stray",
        "extstream:appdb",
        "extstream:ns2",
    ] {
        assert!(host.frames(channel).is_empty(), "{channel} was opened");
    }
    // Every follow the cluster was asked for was one the scope admitted. Each
    // follow starts on its own task, after its open returned: wait for all three,
    // in whatever order they reached the cluster.
    eventually("the three follows", || {
        host.cluster.log_asks.lock().unwrap().len() >= 3
    })
    .await;
    let mut asked: Vec<String> = host
        .cluster
        .log_asks
        .lock()
        .unwrap()
        .iter()
        .map(|a| format!("{}/{}", a.namespace, a.pod))
        .collect();
    asked.sort();
    assert_eq!(
        asked,
        ["cert-manager/webhook-1", "team/web-1", "team/web-2"]
    );
}

/// The view names the object, never its selector, and the host reads the
/// selector each time: an object whose selector selects everything, or has
/// none, is no scope at all.
#[tokio::test(flavor = "multi_thread")]
async fn an_object_that_selects_every_pod_or_none_is_no_scope() {
    let host = host();
    host.cluster.objects.lock().unwrap().insert(
        ("Deployment".into(), "team".into(), "all".into()),
        json!({"spec":{"selector":{"matchLabels":{}}}}),
    );
    host.cluster.objects.lock().unwrap().insert(
        ("Deployment".into(), "team".into(), "bare".into()),
        json!({"spec":{}}),
    );
    let refused = host
        .open(
            "v",
            "extstream:all",
            "team",
            logs("controllerLogs", Some("all"), "web-1"),
        )
        .await
        .unwrap_err();
    assert!(refused.contains("selects every pod"), "{refused}");
    let refused = host
        .open(
            "v",
            "extstream:bare",
            "team",
            logs("controllerLogs", Some("bare"), "web-1"),
        )
        .await
        .unwrap_err();
    assert!(
        refused.contains("has no pod selector at .spec.selector"),
        "{refused}"
    );
    let refused = host
        .open(
            "v",
            "extstream:none",
            "team",
            logs("controllerLogs", None, "web-1"),
        )
        .await
        .unwrap_err();
    assert!(refused.contains("Name the Deployment"), "{refused}");
}

/// `extensions.pods` offers exactly what an open would admit: matched by the
/// host, not by the cluster's answer to its query.
#[tokio::test(flavor = "multi_thread")]
async fn the_pods_a_view_is_offered_are_the_ones_the_scope_admits() {
    let host = host();
    let ask = |capability: &str, namespace: &str, name: Option<&str>| {
        let mut input = json!({"id": APP, "revision": host.revision, "capability": capability,
            "context": "cluster/a", "namespace": namespace});
        if let Some(name) = name {
            input["name"] = json!(name);
        }
        input
    };
    let out = host
        .registry
        .invoke(
            "extensions.pods",
            ask("controllerLogs", "team", Some("web")),
        )
        .await
        .unwrap();
    let names: Vec<&str> = out["pods"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["web-1", "web-2"]);
    assert_eq!(
        out["pods"][0]["containers"],
        json!(["controller", "sidecar"])
    );
    assert_eq!(out["scope"], "pods selected by Deployment web");
    assert!(out.get("services").is_none(), "{out}");
    // A forward through a Service is offered the Services that reach a pod in scope.
    let out = host
        .registry
        .invoke("extensions.pods", ask("webhook", "team", Some("web")))
        .await
        .unwrap();
    assert_eq!(out["services"], json!([{"name": "web", "port": 443}]));
    let out = host
        .registry
        .invoke(
            "extensions.pods",
            ask("namespaceLogs", "cert-manager", None),
        )
        .await
        .unwrap();
    assert_eq!(out["pods"][0]["name"], "webhook-1");
    assert!(host
        .registry
        .invoke("extensions.pods", ask("namespaceLogs", "team", None))
        .await
        .is_err());
    // A reader is no pod binding.
    let refused = host
        .registry
        .invoke("extensions.pods", ask("controllers", "team", Some("web")))
        .await
        .unwrap_err()
        .to_string();
    assert!(refused.contains("not a pod binding"), "{refused}");
}

// ---- Logs ----

/// Lines arrive batched, each tagged with its pod and container, and the
/// history asked for is the first follow's only: a reconnect asks for none.
#[tokio::test(flavor = "multi_thread")]
async fn a_log_stream_batches_lines_and_follows_again_without_repeating() {
    let host = host();
    host.cluster.logs.lock().unwrap().extend([
        vec![LogStep::Lines(1200), LogStep::End],
        vec![LogStep::Line("after the restart")],
    ]);
    let mut source = logs("controllerLogs", Some("web"), "web-1");
    source["tailLines"] = json!(50);
    host.open("v", "extstream:l", "team", source).await.unwrap();
    eventually("the second follow", || {
        host.data("extstream:l")
            .iter()
            .any(|d| d["event"] == "lines" && d["lines"][0]["line"] == "after the restart")
    })
    .await;
    let data = host.data("extstream:l");
    let lines: Vec<&Value> = data
        .iter()
        .filter(|d| d["event"] == "lines")
        .flat_map(|d| d["lines"].as_array().unwrap())
        .collect();
    assert_eq!(lines.len(), 1201);
    assert_eq!(
        lines[0],
        &json!({"source": "web-1/controller", "line": "line 0"})
    );
    assert!(
        data.iter()
            .filter(|d| d["event"] == "lines")
            .all(|d| d["lines"].as_array().unwrap().len() <= super::streams::MAX_LINES_PER_FRAME),
        "a frame carries at most the batch size"
    );
    let statuses: Vec<&str> = data
        .iter()
        .filter(|d| d["event"] == "status")
        .map(|d| d["status"].as_str().unwrap())
        .collect();
    assert_eq!(statuses, ["live", "reconnecting", "live"]);
    let asks = host.cluster.log_asks.lock().unwrap().clone();
    assert_eq!(asks[0].tail_lines, 50);
    assert_eq!(asks[1].tail_lines, 0, "a reconnect repeats no history");
    assert_eq!(asks[0].container, "controller");
}

/// A pod that goes away while its stream reconnects ends it, and says why;
/// one relabelled out of the selector is refused, not followed on.
#[tokio::test(flavor = "multi_thread")]
async fn a_log_stream_ends_when_its_pod_leaves_the_scope_or_the_cluster() {
    let host = host();
    host.cluster
        .logs
        .lock()
        .unwrap()
        .extend([vec![LogStep::End], vec![LogStep::End]]);
    host.open(
        "v",
        "extstream:gone",
        "team",
        logs("controllerLogs", Some("web"), "web-1"),
    )
    .await
    .unwrap();
    host.cluster.remove_pod("web-1");
    eventually("the stream ends", || {
        host.last("extstream:gone")["type"] == "close"
    })
    .await;
    let last_status = host
        .data("extstream:gone")
        .into_iter()
        .rev()
        .find(|d| d["event"] == "status")
        .unwrap();
    assert_eq!(last_status["status"], "completed");
    assert!(last_status["message"]
        .as_str()
        .unwrap()
        .contains("no longer exists"));

    host.open(
        "v",
        "extstream:moved",
        "team",
        logs("controllerLogs", Some("web"), "web-2"),
    )
    .await
    .unwrap();
    for pod in host.cluster.pods.lock().unwrap().iter_mut() {
        if pod.name == "web-2" {
            pod.labels.insert("app".into(), "moved".into());
        }
    }
    eventually("the stream fails", || {
        host.last("extstream:moved")["type"] == "error"
    })
    .await;
    assert!(host.last("extstream:moved")["message"]
        .as_str()
        .unwrap()
        .contains("not selected"));
}

/// A re-read of the object that gets no answer — the API server restarting,
/// the network gone, the usual reason a follow is lost — says nothing of the
/// app's reach: the stream says it is reconnecting, and why, and follows again
/// once the cluster answers, on a scope read again. An answer that refuses —
/// Forbidden — ends it.
#[tokio::test(flavor = "multi_thread")]
async fn a_log_stream_reconnects_through_a_read_that_gets_no_answer() {
    let host = host();
    host.cluster.logs.lock().unwrap().extend([
        vec![LogStep::Line("before"), LogStep::End],
        vec![LogStep::Line("after")],
    ]);
    // The open's read is the first; the reconnect's is the second.
    host.cluster.failed_reads.lock().unwrap().insert(
        2,
        ReadError::Unanswered("the API server did not answer in time".into()),
    );
    host.open(
        "v",
        "extstream:r",
        "team",
        logs("controllerLogs", Some("web"), "web-1"),
    )
    .await
    .unwrap();
    eventually("the second follow", || {
        host.data("extstream:r")
            .iter()
            .any(|d| d["event"] == "lines" && d["lines"][0]["line"] == "after")
    })
    .await;
    let statuses: Vec<Value> = host
        .data("extstream:r")
        .into_iter()
        .filter(|d| d["event"] == "status")
        .collect();
    let names: Vec<&str> = statuses
        .iter()
        .map(|d| d["status"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["live", "reconnecting", "live"]);
    assert!(
        statuses[1]["message"]
            .as_str()
            .unwrap()
            .contains("did not answer"),
        "{}",
        statuses[1]
    );
    assert_eq!(host.cluster.object_reads.load(Ordering::SeqCst), 3);

    let host = self::host();
    host.cluster
        .logs
        .lock()
        .unwrap()
        .push_back(vec![LogStep::End]);
    host.cluster.failed_reads.lock().unwrap().insert(
        2,
        ReadError::Answered("deployments.apps \"web\" is forbidden".into()),
    );
    host.open(
        "v",
        "extstream:f",
        "team",
        logs("controllerLogs", Some("web"), "web-1"),
    )
    .await
    .unwrap();
    eventually("the stream fails", || {
        host.last("extstream:f")["type"] == "error"
    })
    .await;
    assert!(host.last("extstream:f")["message"]
        .as_str()
        .unwrap()
        .contains("forbidden"));
    assert_eq!(host.cluster.log_asks.lock().unwrap().len(), 1);
}

/// Forbidden does not heal, and is not a close.
#[tokio::test(flavor = "multi_thread")]
async fn a_forbidden_log_stream_is_an_error_not_a_close() {
    let host = host();
    host.cluster
        .logs
        .lock()
        .unwrap()
        .push_back(vec![LogStep::Fail(
            "pods \"web-1\" is forbidden: User cannot get resource \"pods/log\"",
        )]);
    host.open(
        "v",
        "extstream:f",
        "team",
        logs("controllerLogs", Some("web"), "web-1"),
    )
    .await
    .unwrap();
    eventually("the error", || host.last("extstream:f")["type"] == "error").await;
    assert_eq!(host.last("extstream:f")["code"], "source");
}

// ---- Exec ----

/// No confirmation, no session: refused before anything runs, and recorded.
#[tokio::test(flavor = "multi_thread")]
async fn exec_is_refused_without_the_host_confirmation() {
    let host = host();
    let refused = host
        .open("v", "extstream:e", "team", exec("web-1", None))
        .await
        .unwrap_err();
    assert!(
        refused.contains("needs the host confirmation")
            && refused.contains("web-1")
            && refused.contains("controller"),
        "{refused}"
    );
    assert!(
        host.cluster.exec_asks.lock().unwrap().is_empty(),
        "nothing ran"
    );
    assert!(host.frames("extstream:e").is_empty());
    let seen = host.trail.seen();
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(seen[0].tool, "k8s.exec");
    assert_eq!(seen[0].outcome, "rejected");
    assert_eq!(seen[0].decision, "denied");
    assert_eq!(seen[0].resource.as_deref(), Some("team/web-1"));
    assert_eq!(seen[0].app.as_ref().unwrap().id, APP);
    assert_eq!(seen[0].args["command"], json!(COMMAND));
}

/// A confirmation covers exactly one session: another pod, another container
/// or another command than the manifest's is refused, and a request cannot
/// carry a command of its own at all.
#[tokio::test(flavor = "multi_thread")]
async fn exec_is_refused_for_anything_but_the_confirmed_manifest_command() {
    let host = host();
    for (why, confirmation) in [
        (
            "another command",
            json!({"pod": "web-1", "container": "controller", "command": ["sh", "-c", "id"]}),
        ),
        (
            "a shortened command",
            json!({"pod": "web-1", "container": "controller", "command": ["cmctl", "status"]}),
        ),
        ("another pod", confirmed("web-2")),
        (
            "another container",
            json!({"pod": "web-1", "container": "sidecar", "command": COMMAND}),
        ),
    ] {
        let refused = host
            .open(
                "v",
                "extstream:e",
                "team",
                exec("web-1", Some(confirmation)),
            )
            .await
            .unwrap_err();
        assert!(refused.contains("nothing ran"), "{why}: {refused}");
    }
    // The command is the manifest's; there is no field to send another in.
    let mut own = exec("web-1", Some(confirmed("web-1")));
    own["command"] = json!(["sh", "-c", "id"]);
    let refused = host
        .open("v", "extstream:own", "team", own)
        .await
        .unwrap_err();
    assert!(refused.contains("unknown field `command`"), "{refused}");
    // Nor a binding that is not an exec binding, however it is confirmed.
    let mut other = exec("web-1", Some(confirmed("web-1")));
    other["capability"] = json!("controllerLogs");
    let refused = host
        .open("v", "extstream:o", "team", other)
        .await
        .unwrap_err();
    assert!(refused.contains("not a k8s.exec one"), "{refused}");
    // A pod out of scope is refused even when confirmed.
    let refused = host
        .open(
            "v",
            "extstream:db",
            "team",
            exec("db-1", Some(confirmed("db-1"))),
        )
        .await
        .unwrap_err();
    assert!(refused.contains("not selected"), "{refused}");
    assert!(
        host.cluster.exec_asks.lock().unwrap().is_empty(),
        "nothing ran"
    );
    let seen = host.trail.seen();
    assert!(seen.iter().all(|r| r.outcome == "rejected"), "{seen:?}");
    // A confirmation that names another session approved nothing this one runs.
    assert_eq!(seen[0].decision, "denied", "{:?}", seen[0]);
    // A name Kubernetes would refuse is refused before anything reads it, the
    // trail included: the desktop bridge bounds nothing a view sends.
    let before = host.trail.seen().len();
    let huge = "x".repeat(100_000);
    let mut bad_container = exec("web-1", Some(confirmed("web-1")));
    bad_container["container"] = json!("Not A Container");
    for bad in [exec(&huge, Some(confirmed(&huge))), bad_container] {
        let refused = host
            .open("v", "extstream:bad", "team", bad)
            .await
            .unwrap_err();
        assert!(refused.starts_with("Name the"), "{refused}");
    }
    assert_eq!(
        host.trail.seen().len(),
        before,
        "nothing unchecked reaches the trail"
    );
}

/// Confirmed: the manifest's command runs in the confirmed pod and container,
/// its output streams as it arrives, and its exit code ends the stream. A
/// failing exit is the command's answer, not the stream's failure.
#[tokio::test(flavor = "multi_thread")]
async fn a_confirmed_exec_runs_the_manifest_command_and_reports_its_exit() {
    let host = host();
    *host.cluster.exec_output.lock().unwrap() = vec![
        (Output::Stdout, "Name: web-cert\n"),
        (Output::Stdout, "Ready: False\n"),
        (Output::Stderr, "warning: expired\n"),
    ];
    *host.cluster.exec_exit.lock().unwrap() = Some(Ok(3));
    host.open(
        "v",
        "extstream:run",
        "team",
        exec("web-1", Some(confirmed("web-1"))),
    )
    .await
    .unwrap();
    eventually("the exit", || host.last("extstream:run")["type"] == "close").await;
    assert_eq!(host.last("extstream:run")["reason"], "completed");
    let ran = host.cluster.exec_asks.lock().unwrap().clone();
    assert_eq!(ran.len(), 1);
    assert_eq!(
        (
            ran[0].pod.as_str(),
            ran[0].container.as_str(),
            &ran[0].command
        ),
        (
            "web-1",
            "controller",
            &COMMAND.iter().map(|s| s.to_string()).collect::<Vec<_>>()
        )
    );
    let data = host.data("extstream:run");
    let chunks: Vec<Value> = data
        .iter()
        .filter(|d| d["event"] == "output")
        .flat_map(|d| d["chunks"].as_array().unwrap().clone())
        .collect();
    assert_eq!(
        chunks,
        [
            json!({"stream": "stdout", "text": "Name: web-cert\nReady: False\n"}),
            json!({"stream": "stderr", "text": "warning: expired\n"}),
        ]
    );
    assert_eq!(data.last().unwrap(), &json!({"event": "exit", "code": 3}));
    let seen = host.trail.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!((seen[0].outcome, seen[0].decision), ("ok", "approved"));
    assert_eq!(seen[0].args["container"], "controller");
}

/// A command the cluster could not run is a failure with the cluster's words.
#[tokio::test(flavor = "multi_thread")]
async fn a_command_that_does_not_run_is_an_error() {
    let host = host();
    *host.cluster.exec_exit.lock().unwrap() = Some(Err(
        "exec: \"cmctl\": executable file not found in $PATH".into(),
    ));
    host.open(
        "v",
        "extstream:x",
        "team",
        exec("web-1", Some(confirmed("web-1"))),
    )
    .await
    .unwrap();
    eventually("the error", || host.last("extstream:x")["type"] == "error").await;
    assert!(host.last("extstream:x")["message"]
        .as_str()
        .unwrap()
        .contains("executable file not found"));
}

/// A command past its limit is left, not killed: the host stops following it
/// and closes the connection, and says the command may still be running, since
/// closing an exec connection does not stop the process in the container.
#[tokio::test(flavor = "multi_thread")]
async fn a_command_past_its_limit_says_what_the_host_did_and_did_not_do() {
    let host = host();
    host.cluster.exec_hangs.store(true, Ordering::SeqCst);
    host.streams.script_pods(
        host.cluster.clone(),
        PodTiming {
            exec_limit: Duration::from_millis(50),
            ..FAST
        },
    );
    host.open(
        "v",
        "extstream:slow",
        "team",
        exec("web-1", Some(confirmed("web-1"))),
    )
    .await
    .unwrap();
    eventually("the limit", || {
        host.last("extstream:slow")["type"] == "error"
    })
    .await;
    let message = host.last("extstream:slow")["message"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        message.contains("stopped following it") && message.contains("may still be running"),
        "{message}"
    );
    assert!(!message.contains("the host stopped it"), "{message}");
}

/// Output that arrived before a session failed is still sent, before the error:
/// the last lines before a failure are usually the ones that say why.
#[tokio::test(flavor = "multi_thread")]
async fn output_before_a_failure_is_sent_before_the_error() {
    let host = host();
    *host.cluster.exec_output.lock().unwrap() = vec![(Output::Stderr, "panic: lost the lease\n")];
    *host.cluster.exec_exit.lock().unwrap() = Some(Err(
        "The connection ended before the command reported how it exited".into(),
    ));
    host.open(
        "v",
        "extstream:lost",
        "team",
        exec("web-1", Some(confirmed("web-1"))),
    )
    .await
    .unwrap();
    eventually("the error", || {
        host.last("extstream:lost")["type"] == "error"
    })
    .await;
    let frames = host.frames("extstream:lost");
    let output = frames
        .iter()
        .position(|f| f["data"]["event"] == "output")
        .expect("the output was sent");
    assert_eq!(
        frames[output]["data"]["chunks"][0]["text"],
        "panic: lost the lease\n"
    );
    assert_eq!(
        output,
        frames.len() - 2,
        "right before the error: {frames:?}"
    );
}

// ---- Port-forwards ----

async fn ready(host: &Host, channel: &str) -> Value {
    eventually("ready", || {
        host.data(channel).iter().any(|d| d["event"] == "ready")
    })
    .await;
    host.data(channel)
        .into_iter()
        .find(|d| d["event"] == "ready")
        .unwrap()
}

/// The host picks the local port; connections reach the binding's port on
/// the pod, and nothing else.
#[tokio::test(flavor = "multi_thread")]
async fn a_forward_listens_on_a_port_the_host_picks_and_reaches_the_pod() {
    let host = host();
    host.open(
        "v",
        "extstream:f",
        "team",
        json!({"kind": "portForward", "capability": "metrics", "name": "web", "pod": "web-1"}),
    )
    .await
    .unwrap();
    let ready = ready(&host, "extstream:f").await;
    assert_eq!(
        (&ready["pod"], &ready["port"]),
        (&json!("web-1"), &json!(9402))
    );
    let local = ready["localPort"].as_u64().unwrap() as u16;
    assert_ne!(local, 0);
    let mut client = tokio::net::TcpStream::connect(("127.0.0.1", local))
        .await
        .unwrap();
    client.write_all(b"ping").await.unwrap();
    let mut back = [0u8; 4];
    client.read_exact(&mut back).await.unwrap();
    assert_eq!(&back, b"ping");
    assert_eq!(
        *host.cluster.connected.lock().unwrap(),
        [("web-1".to_owned(), 9402)]
    );
    let seen = host.trail.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].tool, "k8s.portForward");
    assert_eq!(seen[0].outcome, "ok");
    assert_eq!(seen[0].args["localPort"], json!(local));
    // A view cannot pick the local port: there is no field for it.
    let refused = host
        .open(
            "v",
            "extstream:p",
            "team",
            json!({"kind": "portForward", "capability": "metrics", "name": "web", "pod": "web-1", "localPort": 8080}),
        )
        .await
        .unwrap_err();
    assert!(refused.contains("unknown field `localPort`"), "{refused}");
}

/// Through a Service: resolved to a pod in scope, at the Service's target
/// port. A Service whose pods are not in scope reaches nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_forward_through_a_service_reaches_only_a_pod_in_scope() {
    let host = host();
    host.open(
        "v",
        "extstream:s",
        "team",
        json!({"kind": "portForward", "capability": "webhook", "name": "web", "service": "web"}),
    )
    .await
    .unwrap();
    let ready = ready(&host, "extstream:s").await;
    assert_eq!(ready["service"], "web");
    assert_eq!(ready["port"], 10250, "the named target port, on the pod");
    assert!(["web-1", "web-2"].contains(&ready["pod"].as_str().unwrap()));
    let refused = host
        .open(
            "v",
            "extstream:db",
            "team",
            json!({"kind": "portForward", "capability": "webhook", "name": "web", "service": "db"}),
        )
        .await
        .unwrap_err();
    assert!(refused.contains("sends to no running pod"), "{refused}");
    let refused = host
        .open(
            "v",
            "extstream:pod",
            "team",
            json!({"kind": "portForward", "capability": "webhook", "name": "web", "pod": "web-1"}),
        )
        .await
        .unwrap_err();
    assert!(refused.contains("name the Service"), "{refused}");
    let refused = host
        .open(
            "v",
            "extstream:out",
            "team",
            json!({"kind": "portForward", "capability": "metrics", "name": "web", "pod": "db-1"}),
        )
        .await
        .unwrap_err();
    assert!(refused.contains("not selected"), "{refused}");
    assert_eq!(
        host.trail
            .seen()
            .iter()
            .filter(|r| r.outcome == "rejected")
            .count(),
        3
    );
}

/// A forward through a Service whose pod goes away follows the Service to
/// another pod in scope, and says which. A read that gets no answer on the way
/// does not end it: it is asked again on the next check.
#[tokio::test(flavor = "multi_thread")]
async fn a_service_forward_follows_to_another_pod_through_a_read_that_gets_no_answer() {
    let host = host();
    // The open's read is the first; the first retarget's is the second.
    host.cluster.failed_reads.lock().unwrap().insert(
        2,
        ReadError::Unanswered("the API server did not answer in time".into()),
    );
    host.open(
        "v",
        "extstream:follow",
        "team",
        json!({"kind": "portForward", "capability": "webhook", "name": "web", "service": "web"}),
    )
    .await
    .unwrap();
    let first = ready(&host, "extstream:follow").await;
    let gone = first["pod"].as_str().unwrap().to_owned();
    host.cluster.remove_pod(&gone);
    let readies = || -> Vec<Value> {
        host.data("extstream:follow")
            .into_iter()
            .filter(|d| d["event"] == "ready")
            .collect()
    };
    eventually("the forward on another pod", || readies().len() == 2).await;
    let next = readies().pop().unwrap();
    assert_ne!(next["pod"], gone);
    assert!(["web-1", "web-2"].contains(&next["pod"].as_str().unwrap()));
    assert_eq!(next["port"], 10250);
    assert!(host.cluster.object_reads.load(Ordering::SeqCst) >= 3);
    assert_ne!(host.last("extstream:follow")["type"], "error");
}

/// A forward to a pod ends when the pod does.
#[tokio::test(flavor = "multi_thread")]
async fn a_forward_ends_when_its_pod_goes_away() {
    let host = host();
    host.open(
        "v",
        "extstream:g",
        "team",
        json!({"kind": "portForward", "capability": "metrics", "name": "web", "pod": "web-2"}),
    )
    .await
    .unwrap();
    ready(&host, "extstream:g").await;
    host.cluster.remove_pod("web-2");
    eventually("the error", || host.last("extstream:g")["type"] == "error").await;
    assert!(host.last("extstream:g")["message"]
        .as_str()
        .unwrap()
        .contains("no longer running"));
}

// ---- Closing the view ----

/// Closing the view ends every stream it opened — a log follow, a running
/// command and a forward — and releases the forward: its port stops
/// listening and every connection through it closes. Another view's session
/// is untouched.
#[tokio::test(flavor = "multi_thread")]
async fn closing_the_view_releases_every_stream_and_forward_it_opened() {
    let host = host();
    host.cluster.exec_hangs.store(true, Ordering::SeqCst);
    host.open(
        "view#1",
        "extstream:logs",
        "team",
        logs("controllerLogs", Some("web"), "web-1"),
    )
    .await
    .unwrap();
    host.open(
        "view#1",
        "extstream:exec",
        "team",
        exec("web-1", Some(confirmed("web-1"))),
    )
    .await
    .unwrap();
    host.open(
        "view#1",
        "extstream:fwd",
        "team",
        json!({"kind": "portForward", "capability": "metrics", "name": "web", "pod": "web-1"}),
    )
    .await
    .unwrap();
    host.open(
        "view#2",
        "extstream:other",
        "team",
        json!({"kind": "portForward", "capability": "metrics", "name": "web", "pod": "web-2"}),
    )
    .await
    .unwrap();
    let local = ready(&host, "extstream:fwd").await["localPort"]
        .as_u64()
        .unwrap() as u16;
    let other = ready(&host, "extstream:other").await["localPort"]
        .as_u64()
        .unwrap() as u16;
    let mut client = tokio::net::TcpStream::connect(("127.0.0.1", local))
        .await
        .unwrap();
    client.write_all(b"ping").await.unwrap();
    let mut back = [0u8; 4];
    client.read_exact(&mut back).await.unwrap();
    eventually("one upstream connection", || {
        host.cluster.open.load(Ordering::SeqCst) == 1
    })
    .await;

    assert_eq!(host.streams.close_view("view#1"), 3);
    for channel in ["extstream:logs", "extstream:exec", "extstream:fwd"] {
        assert_eq!(host.last(channel)["reason"], "viewClosed", "{channel}");
    }
    // The connection through the forward is closed, on both sides.
    let mut rest = [0u8; 16];
    let read = tokio::time::timeout(Duration::from_secs(2), client.read(&mut rest))
        .await
        .expect("the local connection closes");
    assert!(matches!(read, Ok(0) | Err(_)), "{read:?}");
    eventually("the upstream closes", || {
        host.cluster.open.load(Ordering::SeqCst) == 0
    })
    .await;
    // And the port no longer listens.
    eventually("the port closes", || {
        std::net::TcpStream::connect(("127.0.0.1", local)).is_err()
    })
    .await;
    // The other view's forward still does.
    assert!(host
        .frames("extstream:other")
        .iter()
        .all(|f| f["type"] != "close"));
    let mut still = tokio::net::TcpStream::connect(("127.0.0.1", other))
        .await
        .unwrap();
    still.write_all(b"pong").await.unwrap();
    let mut back = [0u8; 4];
    still.read_exact(&mut back).await.unwrap();
    assert_eq!(&back, b"pong");
    let out = host
        .registry
        .invoke("extensions.streams", json!({}))
        .await
        .unwrap();
    assert_eq!(out["apps"][0]["openStreams"], 1, "{out}");
}

/// Disabling the app ends its pod sessions too, as every lifecycle change does.
#[tokio::test(flavor = "multi_thread")]
async fn disabling_the_app_ends_its_forwards() {
    let host = host();
    host.open(
        "v",
        "extstream:d",
        "team",
        json!({"kind": "portForward", "capability": "metrics", "name": "web", "pod": "web-1"}),
    )
    .await
    .unwrap();
    let local = ready(&host, "extstream:d").await["localPort"]
        .as_u64()
        .unwrap() as u16;
    configure(
        &host.path,
        json!({"action": "enable", "id": APP, "enabled": false}),
    )
    .unwrap();
    eventually("the close", || {
        host.last("extstream:d")["reason"] == "appDisabled"
    })
    .await;
    eventually("the port closes", || {
        std::net::TcpStream::connect(("127.0.0.1", local)).is_err()
    })
    .await;
}

// ---- The contract, as the wrapper sends it ----

const LOGS_PAYLOAD: &str =
    include_str!("../../../../packages/core/src/lib/extension-stream-logs.json");
const EXEC_PAYLOAD: &str =
    include_str!("../../../../packages/core/src/lib/extension-stream-exec.json");
const FORWARD_PAYLOAD: &str =
    include_str!("../../../../packages/core/src/lib/extension-stream-port-forward.json");

/// What `@srelens/core` sends, byte for byte: `extensionStreams.test.ts`
/// holds the wrapper to these same files. The snake_case spellings are refused.
#[test]
fn the_wrappers_pod_payloads_deserialize_and_snake_case_is_rejected() {
    use super::streams::{OpenStreamIn, StreamSourceIn};
    for payload in [LOGS_PAYLOAD, EXEC_PAYLOAD, FORWARD_PAYLOAD] {
        let value: Value = serde_json::from_str(payload).unwrap();
        serde_json::from_value::<OpenStreamIn>(value.clone())
            .unwrap_or_else(|e| panic!("{payload}: {e}"));
    }
    let logs: OpenStreamIn = serde_json::from_str(LOGS_PAYLOAD).unwrap();
    assert!(matches!(
        logs.source,
        StreamSourceIn::Logs {
            tail_lines: Some(200),
            timestamps: true,
            ..
        }
    ));
    let exec: OpenStreamIn = serde_json::from_str(EXEC_PAYLOAD).unwrap();
    assert!(matches!(
        exec.source,
        StreamSourceIn::Exec {
            confirmed: Some(_),
            ..
        }
    ));
    let mut snake: Value = serde_json::from_str(LOGS_PAYLOAD).unwrap();
    let tail = snake["source"]
        .as_object_mut()
        .unwrap()
        .remove("tailLines")
        .unwrap();
    snake["source"]["tail_lines"] = tail;
    assert!(serde_json::from_value::<OpenStreamIn>(snake).is_err());
}

// ---- The access review (#554) ----

fn parsed(value: &Value) -> srelens_plugin_host::Manifest {
    srelens_plugin_host::Manifest::parse(&value.to_string()).unwrap_or_else(|e| panic!("{e}"))
}

fn grants() -> Vec<String> {
    GRANTS.iter().map(|g| (*g).to_owned()).collect()
}

/// An update's review says what each pod binding reaches and runs, in words:
/// another command, another namespace, or a scope that moves to another kind
/// is changed access under the same grants.
#[test]
fn the_access_review_names_each_command_scope_and_granted_namespace() {
    let old: Value = serde_json::from_str(&manifest()).unwrap();
    let current = super::access_items(&parsed(&old), &grants());
    for expected in [
        "Reach any pod in namespace cert-manager with k8s.streamLogs",
        "Run [\"cmctl\",\"status\",\"certificate\",\"--all-namespaces\"] in container controller of pods selected by each Deployment k8s.listDeployments {} reads, with k8s.exec",
        "Forward port 9402 of pods selected by each Deployment k8s.listDeployments {} reads to a local port, with k8s.portForward",
        "Forward port 443 through a Service to pods selected by each Deployment k8s.listDeployments {} reads to a local port, with k8s.portForward",
        "Stream logs of pods in a granted namespace, with k8s.streamLogs",
    ] {
        assert!(current.contains(expected), "{expected}\n{current:#?}");
    }
    assert!(
        current.iter().any(|item| item
            .starts_with("Stream logs of pods selected by each Application")
            && item.contains("at .spec.workload.selector")),
        "{current:#?}"
    );
    // Another command is another item.
    let mut changed = old.clone();
    changed["capabilities"][5]["arguments"]["command"] = json!(["cmctl", "renew", "--all"]);
    changed["permissions"][2]["namespaces"] = json!(["cert-manager", "kube-system"]);
    let diff = super::permission_diff(
        Some((&parsed(&old), &grants(), 1)),
        &parsed(&changed),
        &grants(),
    );
    assert_eq!(
        diff.added,
        [
            "Reach any pod in namespace kube-system with k8s.streamLogs",
            "Run [\"cmctl\",\"renew\",\"--all\"] in container controller of pods selected by each Deployment k8s.listDeployments {} reads, with k8s.exec",
        ],
        "{diff:?}"
    );
    assert_eq!(diff.removed.len(), 1, "{diff:?}");
    assert!(
        diff.removed[0].starts_with("Run [\"cmctl\",\"status\""),
        "{diff:?}"
    );
}

/// An exec binding runs code in the cluster, so an unsigned app that binds one
/// needs "Allow unsigned apps to modify clusters and run code", as one that
/// declares actions does; one that only follows logs or forwards does not.
#[test]
fn an_unsigned_app_that_binds_exec_needs_the_unsigned_app_setting() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("extensions.json");
    let Err(refused) = configure(
        &path,
        json!({"action": "install", "manifest": manifest(), "grants": GRANTS}),
    ) else {
        panic!("an unsigned exec binding installed without the setting");
    };
    assert!(refused.contains("Allow unsigned apps"), "{refused}");
    let mut without_exec: Value = serde_json::from_str(&manifest()).unwrap();
    without_exec["capabilities"]
        .as_array_mut()
        .unwrap()
        .remove(5);
    without_exec["permissions"]
        .as_array_mut()
        .unwrap()
        .remove(3);
    let grants: Vec<&str> = GRANTS
        .iter()
        .copied()
        .filter(|g| *g != "k8s.exec")
        .collect();
    configure(
        &path,
        json!({"action": "install", "manifest": without_exec.to_string(), "grants": grants}),
    )
    .unwrap_or_else(|e| panic!("logs and forwards need no policy: {e}"));
}

/// Exec and port-forward sessions are recorded, so the entry point that takes no
/// audit trail refuses them rather than opening one unrecorded. A log stream is a
/// read, and opens there.
#[tokio::test(flavor = "multi_thread")]
async fn a_session_that_must_be_recorded_is_not_opened_without_a_trail() {
    let host = host();
    for source in [
        exec("web-1", Some(confirmed("web-1"))),
        json!({"kind": "portForward", "capability": "metrics", "name": "web", "pod": "web-1"}),
    ] {
        let refused = host
            .streams
            .open(
                host.sink.clone(),
                host.request("v", "extstream:unaudited", "team", source.clone()),
            )
            .await
            .unwrap_err();
        assert!(refused.contains("audit trail"), "{source}: {refused}");
    }
    assert!(host.cluster.exec_asks.lock().unwrap().is_empty());
    assert!(host.frames("extstream:unaudited").is_empty());
    host.cluster
        .logs
        .lock()
        .unwrap()
        .push_back(vec![LogStep::Line("ok")]);
    host.streams
        .open(
            host.sink.clone(),
            host.request(
                "v",
                "extstream:read",
                "team",
                logs("controllerLogs", Some("web"), "web-1"),
            ),
        )
        .await
        .expect("a log stream is a read");
}

/// A connection the cluster refuses is said, not dropped in silence: the view
/// keeps saying "Forwarding" otherwise while every connection fails. However
/// many fail, they are reported at most once a check, with how many.
#[tokio::test(flavor = "multi_thread")]
async fn a_forward_says_when_its_connections_fail() {
    let host = host();
    *host.cluster.refuse_connect.lock().unwrap() =
        Some("pods \"web-1\" is forbidden: cannot create resource \"pods/portforward\"");
    host.open(
        "v",
        "extstream:fail",
        "team",
        json!({"kind": "portForward", "capability": "metrics", "name": "web", "pod": "web-1"}),
    )
    .await
    .unwrap();
    let local = ready(&host, "extstream:fail").await["localPort"]
        .as_u64()
        .unwrap() as u16;
    for _ in 0..3 {
        let mut client = tokio::net::TcpStream::connect(("127.0.0.1", local))
            .await
            .unwrap();
        let mut rest = [0u8; 4];
        let read = tokio::time::timeout(Duration::from_secs(2), client.read(&mut rest))
            .await
            .expect("a refused connection is closed");
        assert!(matches!(read, Ok(0) | Err(_)), "{read:?}");
    }
    eventually("the failure is said", || {
        host.data("extstream:fail")
            .iter()
            .map(|d| {
                if d["event"] == "connectionFailed" {
                    d["count"].as_u64().unwrap_or(0)
                } else {
                    0
                }
            })
            .sum::<u64>()
            == 3
    })
    .await;
    let failed = host
        .data("extstream:fail")
        .into_iter()
        .find(|d| d["event"] == "connectionFailed")
        .unwrap();
    assert!(
        failed["message"].as_str().unwrap().contains("forbidden"),
        "{failed}"
    );
    // The forward itself is still open: the next connection may succeed.
    assert!(host
        .frames("extstream:fail")
        .iter()
        .all(|f| f["type"] != "close" && f["type"] != "error"));
}
