//! Pod tunnels for an app's program (#1091): a sidecar reaches only a running
//! pod its binding's scope admits, on the binding's port; every connect is
//! recorded; and a tunnel ends when its pod or the app's authority goes away.
//!
//! The cluster is the scripted one of `pods_tests.rs`; the inventory is real.

use super::pods_tests::{self, pod, Cluster, Trail};
use super::streams::{ExtensionStreams, PodTiming};
use super::tests::{configure, fake_core, kubeconfig};
use super::tunnels::PodTunnels;
use serde_json::{json, Value};
use srelens_capability::audit::{AppRef, Source, OUTCOME_OK, OUTCOME_REJECTED};
use srelens_capability::Registry;
use srelens_kube::app_pods::{PortTarget, ReadError, ServiceFacts, ServicePort};
use srelens_plugin_host::sidecar::protocol::code;
use srelens_plugin_host::sidecar::{AppIdentity, PodConnector, Tunnel};
use srelens_sidecar_protocol::{HostConnectParams, JobContext};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const KAFKA: &str = "org.example.kafka";
const TICK: Duration = Duration::from_secs(2);
const TIMING: PodTiming = PodTiming {
    batch: Duration::from_millis(250),
    reconnect: Duration::from_secs(2),
    exec_limit: Duration::from_secs(300),
    monitor: TICK,
};

/// A Kafka console: it reads StatefulSets, and its program may connect to port
/// 9092 of the pods one selects (directly or through a Service), or of any pod
/// in `team`.
fn manifest() -> Value {
    let binaries: serde_json::Map<String, Value> = srelens_plugin_host::SIDECAR_PLATFORMS
        .iter()
        .map(|platform| (platform.to_string(), json!(format!("bin/{platform}/kafka"))))
        .collect();
    json!({
        "id": KAFKA, "name": "Kafka", "version": "0.1.0", "srelensApiVersion": "^0.9",
        "kind": "executable",
        "permissions": ["k8s.listStatefulSets",
            {"capability": "k8s.connectPod", "namespaces": ["team"]}],
        "capabilities": [
            {"name": "sets", "title": "Kafka sets", "target": "k8s.listStatefulSets",
                "inputs": ["context", "namespace"], "arguments": {}},
            {"name": "broker", "title": "Kafka broker", "target": "k8s.connectPod", "inputs": [],
                "arguments": {"resource": "sets", "port": 9092}},
            {"name": "bootstrap", "title": "Kafka bootstrap", "target": "k8s.connectPod", "inputs": [],
                "arguments": {"resource": "sets", "port": 9092, "service": true}},
            {"name": "anyPod", "title": "Any pod in team", "target": "k8s.connectPod", "inputs": [],
                "arguments": {"port": 9092}}
        ],
        "sidecar": {"binaries": binaries, "operations": [{"name": "scan", "title": "Scan", "inputs": []}]},
        "contributions": {"pages": [], "detailTabs": [], "detailLinks": []}
    })
}

pub(super) struct Host {
    _dir: tempfile::TempDir,
    pub(super) path: PathBuf,
    kubeconfig: PathBuf,
    _registry: Registry,
    _streams: Arc<ExtensionStreams>,
    cluster: Arc<Cluster>,
    pub(super) trail: Arc<Trail>,
    tunnels: PodTunnels,
    pub(super) app: AppIdentity,
}

/// StatefulSet `team/kafka` selects `app=kafka`: `kafka-0` and `kafka-1` run,
/// `kafka-2` has not started. `web-1` is in `team` but not selected, and is the
/// first pod Service `kafka-bootstrap` sends to.
pub(super) fn host() -> Host {
    host_with("", "")
}

/// The same, its kubeconfig's user holding `token` and `key` (when not empty), as a
/// real one does and a sidecar must never see.
pub(super) fn host_with(token: &str, key: &str) -> Host {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("extensions.json");
    let config = kubeconfig(dir.path(), "clusters.yaml", &["cluster/a", "cluster/b"]);
    if !token.is_empty() {
        let yaml = std::fs::read_to_string(&config).unwrap();
        let user = format!("user:\n    token: {token}\n    client-key-data: {key}");
        std::fs::write(&config, yaml.replace("user: {}", &user)).unwrap();
    }
    let mut registry = Registry::new();
    let streams = super::register(
        &mut registry,
        path.clone(),
        fake_core(),
        srelens_kube::client_cache::ClientCache::new_many(vec![config.clone()]),
    );
    let cluster = Arc::new(Cluster::default());
    cluster.objects.lock().unwrap().insert(
        ("StatefulSet".into(), "team".into(), "kafka".into()),
        json!({"metadata": {"name": "kafka", "namespace": "team"},
               "spec": {"selector": {"matchLabels": {"app": "kafka"}}}}),
    );
    let mut stopped = pod("kafka-2", "team", &[("app", "kafka"), ("tier", "broker")]);
    stopped.phase = Some("Pending".into());
    *cluster.pods.lock().unwrap() = vec![
        pod("web-1", "team", &[("app", "web"), ("tier", "broker")]),
        pod("kafka-0", "team", &[("app", "kafka"), ("tier", "broker")]),
        pod("kafka-1", "team", &[("app", "kafka"), ("tier", "broker")]),
        stopped,
    ];
    *cluster.services.lock().unwrap() = vec![ServiceFacts {
        name: "kafka-bootstrap".into(),
        namespace: "team".into(),
        selector: [("tier".to_owned(), "broker".to_owned())].into(),
        ports: vec![ServicePort {
            name: None,
            port: 9092,
            target: PortTarget::Number(19092),
        }],
    }];
    configure(
        &path,
        json!({"action": "unsignedApps", "allowUnsignedApps": true}),
    )
    .unwrap();
    let archive = super::executable_tests::package_with_binaries(&manifest());
    let state = configure(
        &path,
        json!({"action": "installPackage", "grants": ["k8s.listStatefulSets", "k8s.connectPod"],
            "package": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, archive)}),
    )
    .unwrap_or_else(|e| panic!("install: {e}"));
    let revision = state
        .plugins
        .iter()
        .find(|p| p.manifest.id == KAFKA)
        .unwrap()
        .revision;
    let trail = Arc::new(Trail::default());
    let tunnels = PodTunnels::with_cluster(&streams, trail.clone(), cluster.clone(), TIMING);
    Host {
        _dir: dir,
        path,
        kubeconfig: config,
        _registry: registry,
        _streams: streams,
        cluster,
        trail,
        tunnels,
        app: AppIdentity {
            id: KAFKA.into(),
            revision,
            name: "Kafka".into(),
            publisher: None,
        },
    }
}

fn ask(
    capability: &str,
    name: Option<&str>,
    pod: Option<&str>,
    service: Option<&str>,
) -> HostConnectParams {
    HostConnectParams {
        context: JobContext {
            cluster_id: "cluster/a".into(),
            namespace: "team".into(),
        },
        capability: capability.into(),
        name: name.map(str::to_owned),
        pod: pod.map(str::to_owned),
        service: service.map(str::to_owned),
        tunnel: 1,
    }
}

impl Host {
    /// A connector over the same cluster and trail, for a host to hand its
    /// sidecars.
    pub(super) fn pods(&self) -> Arc<dyn PodConnector> {
        Arc::new(PodTunnels::with_cluster(
            &self._streams,
            self.trail.clone(),
            self.cluster.clone(),
            TIMING,
        ))
    }

    pub(super) fn registry(&self) -> Arc<Registry> {
        Arc::new(self._registry.clone())
    }

    async fn connect(&self, ask: HostConnectParams) -> Tunnel {
        match self.tunnels.connect(&self.app, &ask).await {
            Ok(tunnel) => tunnel,
            Err(error) => panic!("refused: {error:?}"),
        }
    }

    /// Refused with `code`, with one rejected record and nothing opened. The
    /// message the sidecar is told.
    async fn refused(&self, app: &AppIdentity, ask: HostConnectParams, code: i64) -> String {
        let before = self.trail.seen().len();
        let connected = self.cluster.connected.lock().unwrap().len();
        let error = match self.tunnels.connect(app, &ask).await {
            Ok(tunnel) => panic!("connected to {}", tunnel.pod),
            Err(error) => error,
        };
        assert_eq!(error.code, code, "{error:?}");
        let seen = self.trail.seen();
        assert_eq!(seen.len(), before + 1, "one record per refusal: {seen:#?}");
        let record = seen.last().unwrap();
        assert_eq!(record.outcome, OUTCOME_REJECTED, "{record:?}");
        assert_eq!(record.source, Source::Sidecar, "{record:?}");
        assert_eq!(record.tool, "k8s.connectPod");
        assert!(record.error.is_some(), "{record:?}");
        assert!(
            self.cluster.connected.lock().unwrap().len() == connected,
            "a refused connect opened a connection"
        );
        error.message
    }

    /// The pinned ID `cluster/a` resolves to, which records name once it has.
    fn pinned(&self) -> String {
        let all = srelens_kube::context_resolve::resolve_contexts(&[self.kubeconfig.clone()]);
        srelens_kube::context_resolve::find_context(&all, "cluster/a")
            .and_then(|context| context.pinned_id())
            .unwrap()
    }

    fn last_cluster(&self) -> Option<String> {
        self.trail
            .seen()
            .last()
            .and_then(|record| record.cluster.clone())
    }
}

fn relabel(cluster: &Cluster, name: &str, labels: &[(&str, &str)]) {
    let mut pods = cluster.pods.lock().unwrap();
    let found = pods.iter_mut().find(|p| p.name == name).unwrap();
    *found = pod(name, "team", labels);
}

// ---- Opened ----

#[tokio::test]
async fn an_in_scope_pod_connects_and_echoes() {
    let host = host();
    let mut tunnel = host
        .connect(ask("broker", Some("kafka"), Some("kafka-0"), None))
        .await;
    assert_eq!(tunnel.pod, "kafka-0");
    tunnel.io.write_all(b"ping").await.unwrap();
    let mut echoed = [0u8; 4];
    tunnel.io.read_exact(&mut echoed).await.unwrap();
    assert_eq!(&echoed, b"ping");
    assert_eq!(
        *host.cluster.connected.lock().unwrap(),
        [("kafka-0".to_owned(), 9092)]
    );
    let seen = host.trail.seen();
    assert_eq!(seen.len(), 1, "{seen:#?}");
    let record = &seen[0];
    assert_eq!(record.source, Source::Sidecar);
    assert_eq!(record.tool, "k8s.connectPod");
    assert_eq!(record.decision, "auto");
    assert_eq!(record.outcome, OUTCOME_OK);
    assert_eq!(record.error, None);
    assert_eq!(
        record.args,
        json!({"capability": "broker", "pod": "kafka-0", "service": null, "port": 9092})
    );
    assert_eq!(record.resource.as_deref(), Some("team/kafka-0"));
    // The sidecar named the context by its name; the record names the cluster
    // it resolved to, as a view's forward does.
    assert_eq!(record.cluster, Some(host.pinned()));
    assert_ne!(host.pinned(), "cluster/a");
    assert_eq!(
        record.app,
        Some(AppRef {
            id: KAFKA.into(),
            revision: host.app.revision
        })
    );
}

#[tokio::test]
async fn a_service_resolves_to_a_pod_the_scope_admits() {
    let host = host();
    // web-1 is the Service's first pod, but the StatefulSet does not select it.
    let tunnel = host
        .connect(ask(
            "bootstrap",
            Some("kafka"),
            None,
            Some("kafka-bootstrap"),
        ))
        .await;
    assert_eq!(tunnel.pod, "kafka-0");
    assert_eq!(
        *host.cluster.connected.lock().unwrap(),
        [("kafka-0".to_owned(), 19092)]
    );
    let record = host.trail.seen().pop().unwrap();
    assert_eq!(record.outcome, OUTCOME_OK);
    assert_eq!(record.resource.as_deref(), Some("team/kafka-0"));
    assert_eq!(
        record.args,
        json!({"capability": "bootstrap", "pod": "kafka-0", "service": "kafka-bootstrap", "port": 9092, "targetPort": 19092})
    );
}

// ---- Refused by grants, scope or cluster: -32003 ----

#[tokio::test]
async fn a_pod_outside_the_scope() {
    let host = host();
    let outside = host
        .refused(
            &host.app,
            ask("broker", Some("kafka"), Some("web-1"), None),
            code::CAPABILITY_FAILED,
        )
        .await;
    assert!(outside.contains("web-1"), "{outside}");
    // A pod that does not exist is told the same, so nothing outside the scope
    // can be learned by asking.
    let missing = host
        .refused(
            &host.app,
            ask("broker", Some("kafka"), Some("nope"), None),
            code::CAPABILITY_FAILED,
        )
        .await;
    assert_eq!(missing, outside.replace("web-1", "nope"));
    // A namespace the permission does not grant.
    let mut elsewhere = ask("anyPod", None, Some("kafka-0"), None);
    elsewhere.context.namespace = "other".into();
    host.refused(&host.app, elsewhere, code::CAPABILITY_FAILED)
        .await;
}

#[tokio::test]
async fn a_pod_that_is_not_running() {
    let host = host();
    let why = host
        .refused(
            &host.app,
            ask("broker", Some("kafka"), Some("kafka-2"), None),
            code::CAPABILITY_FAILED,
        )
        .await;
    assert!(
        why.contains("kafka-2") && why.contains("not running"),
        "{why}"
    );
}

#[tokio::test]
async fn an_ungranted_permission() {
    let host = host();
    // A grant taken back, as a hand-edited inventory would.
    let mut state = super::read(&host.path).unwrap();
    let app = state
        .plugins
        .iter_mut()
        .find(|p| p.manifest.id == KAFKA)
        .unwrap();
    app.grants.retain(|g| g != "k8s.connectPod");
    super::write(&host.path, &state).unwrap();
    let why = host
        .refused(
            &host.app,
            ask("broker", Some("kafka"), Some("kafka-0"), None),
            code::CAPABILITY_FAILED,
        )
        .await;
    assert!(why.contains("k8s.connectPod"), "{why}");
}

#[tokio::test]
async fn an_app_disabled_for_this_cluster() {
    let host = host();
    configure(
        &host.path,
        json!({"action": "clusters", "id": KAFKA,
            "contexts": [format!("{}#cluster/b", host.kubeconfig.display())]}),
    )
    .unwrap();
    let why = host
        .refused(
            &host.app,
            ask("broker", Some("kafka"), Some("kafka-0"), None),
            code::CAPABILITY_FAILED,
        )
        .await;
    assert!(why.contains("not enabled for this cluster"), "{why}");
    // Refused before it resolved: the record keeps the name the sidecar sent.
    assert_eq!(host.last_cluster().as_deref(), Some("cluster/a"));
}

#[tokio::test]
async fn a_declarative_app_is_refused() {
    let host = host();
    let revision = pods_tests::install(&host.path);
    let declarative = AppIdentity {
        id: pods_tests::APP.into(),
        revision,
        name: "cert-manager".into(),
        publisher: None,
    };
    // Its port-forward binding is a pod binding a view may open; a program may
    // not, and it has none anyway.
    let why = host
        .refused(
            &declarative,
            ask("metrics", Some("web"), Some("web-1"), None),
            code::CAPABILITY_FAILED,
        )
        .await;
    assert!(why.contains("executable"), "{why}");
}

#[tokio::test]
async fn the_cluster_refusing_the_forward_is_reported() {
    let host = host();
    *host.cluster.refuse_connect.lock().unwrap() =
        Some("pods \"kafka-0\" is forbidden: cannot create resource \"pods/portforward\"");
    let error = match host
        .tunnels
        .connect(
            &host.app,
            &ask("broker", Some("kafka"), Some("kafka-0"), None),
        )
        .await
    {
        Ok(_) => panic!("connected through a refused forward"),
        Err(error) => error,
    };
    assert_eq!(error.code, code::CAPABILITY_FAILED, "{error:?}");
    assert!(
        error.message.contains("pods/portforward") && error.message.contains("forbidden"),
        "{error:?}"
    );
    let seen = host.trail.seen();
    assert_eq!(seen.len(), 1, "{seen:#?}");
    assert_eq!(seen[0].outcome, OUTCOME_REJECTED);
    assert_eq!(seen[0].resource.as_deref(), Some("team/kafka-0"));
}

// ---- Not a call this binding answers: -32602 ----

#[tokio::test]
async fn an_undeclared_binding() {
    let host = host();
    let why = host
        .refused(
            &host.app,
            ask("nope", Some("kafka"), Some("kafka-0"), None),
            code::INVALID_PARAMS,
        )
        .await;
    assert!(why.contains("nope"), "{why}");
    // Refused after the context resolved: the record names the pinned cluster.
    assert_eq!(host.last_cluster(), Some(host.pinned()));
}

#[tokio::test]
async fn a_binding_of_another_target() {
    let host = host();
    let why = host
        .refused(
            &host.app,
            ask("sets", Some("kafka"), Some("kafka-0"), None),
            code::INVALID_PARAMS,
        )
        .await;
    assert!(why.contains("k8s.listStatefulSets"), "{why}");
}

#[tokio::test]
async fn a_resource_binding_without_name() {
    let host = host();
    let why = host
        .refused(
            &host.app,
            ask("broker", None, Some("kafka-0"), None),
            code::INVALID_PARAMS,
        )
        .await;
    assert!(why.contains("broker"), "{why}");
}

#[tokio::test]
async fn a_namespace_binding_with_name() {
    let host = host();
    let why = host
        .refused(
            &host.app,
            ask("anyPod", Some("kafka"), Some("kafka-0"), None),
            code::INVALID_PARAMS,
        )
        .await;
    assert!(why.contains("anyPod"), "{why}");
}

#[tokio::test]
async fn pod_on_a_service_binding() {
    let host = host();
    let why = host
        .refused(
            &host.app,
            ask("bootstrap", Some("kafka"), Some("kafka-0"), None),
            code::INVALID_PARAMS,
        )
        .await;
    assert!(why.contains("Service"), "{why}");
    // And the other way round.
    let why = host
        .refused(
            &host.app,
            ask("broker", Some("kafka"), None, Some("kafka-bootstrap")),
            code::INVALID_PARAMS,
        )
        .await;
    assert!(why.contains("pod"), "{why}");
}

// ---- Ended ----

/// Waits out `ticks` monitor ticks, and just past the last, so its checks
/// have run; and says the tunnel is still open.
async fn still_open(tunnel: &mut Tunnel, ticks: u32) {
    let past = TICK * ticks + Duration::from_millis(1);
    if let Ok(why) = tokio::time::timeout(past, &mut tunnel.ended).await {
        panic!("ended early: {why}");
    }
}

/// The reason the tunnel ends with, within one monitor tick.
async fn ends(tunnel: &mut Tunnel) -> String {
    tokio::time::timeout(TICK + Duration::from_millis(1), &mut tunnel.ended)
        .await
        .expect("the tunnel ended within one monitor tick")
}

#[tokio::test(start_paused = true)]
async fn the_tunnel_ends_when_the_pod_leaves_the_scope() {
    let host = host();
    let mut tunnel = host
        .connect(ask("broker", Some("kafka"), Some("kafka-0"), None))
        .await;
    still_open(&mut tunnel, 3).await;
    relabel(&host.cluster, "kafka-0", &[("app", "other")]);
    let why = ends(&mut tunnel).await;
    assert!(
        why.contains("team/kafka-0") && why.contains("not selected"),
        "{why}"
    );
}

#[tokio::test(start_paused = true)]
async fn the_tunnel_ends_when_the_app_is_disabled() {
    let host = host();
    let mut tunnel = host
        .connect(ask("broker", Some("kafka"), Some("kafka-0"), None))
        .await;
    still_open(&mut tunnel, 2).await;
    configure(
        &host.path,
        json!({"action": "enable", "id": KAFKA, "enabled": false}),
    )
    .unwrap();
    let why = ends(&mut tunnel).await;
    // Said to a program: what happened, with no view to refresh.
    assert_eq!(
        why,
        "The connection to pod team/kafka-0 ended: Extension was disabled or updated"
    );
    let refused = host
        .refused(
            &host.app,
            ask("broker", Some("kafka"), Some("kafka-0"), None),
            code::CAPABILITY_FAILED,
        )
        .await;
    assert_eq!(refused, "Extension was disabled or updated");
}

#[tokio::test(start_paused = true)]
async fn a_service_tunnel_does_not_move() {
    let host = host();
    let mut tunnel = host
        .connect(ask(
            "bootstrap",
            Some("kafka"),
            None,
            Some("kafka-bootstrap"),
        ))
        .await;
    assert_eq!(tunnel.pod, "kafka-0");
    still_open(&mut tunnel, 2).await;
    // kafka-1 is still behind the Service; the tunnel ends rather than move.
    host.cluster
        .pods
        .lock()
        .unwrap()
        .retain(|p| p.name != "kafka-0");
    let why = ends(&mut tunnel).await;
    assert!(
        why.contains("team/kafka-0") && why.contains("no longer running"),
        "{why}"
    );
    assert_eq!(
        *host.cluster.connected.lock().unwrap(),
        [("kafka-0".to_owned(), 19092)],
        "no reconnect"
    );
}

/// Another process — the app's settings window, while this is a headless
/// `--mcp-stdio` — disables the app: nothing announces it here, and the monitor
/// still ends the tunnel within one tick, because each tick rechecks the app.
#[tokio::test(start_paused = true)]
async fn the_tunnel_ends_when_another_process_disables_the_app() {
    let host = host();
    let mut tunnel = host
        .connect(ask("broker", Some("kafka"), Some("kafka-0"), None))
        .await;
    still_open(&mut tunnel, 2).await;
    // Written to the store as another process writes it, with no announcement.
    let mut state = super::read(&host.path).unwrap();
    state
        .plugins
        .iter_mut()
        .find(|p| p.manifest.id == KAFKA)
        .unwrap()
        .enabled = false;
    super::write(&host.path, &state).unwrap();
    let why = ends(&mut tunnel).await;
    assert_eq!(
        why,
        "The connection to pod team/kafka-0 ended: Extension was disabled or updated"
    );
}

const FORBIDDEN: &str = "pods \"kafka-0\" is forbidden: User \"me\" cannot get resource \"pods\"";

fn failed_read() -> Option<ReadError> {
    Some(ReadError::Answered(FORBIDDEN.into()))
}

/// A pod tunnel is unattended: a pod that cannot be read, tick after tick,
/// is not taken to be still there. Three failed reads in a row end it, and
/// not one fewer.
#[tokio::test(start_paused = true)]
async fn the_tunnel_ends_after_three_failed_pod_reads() {
    let host = host();
    let mut tunnel = host
        .connect(ask("broker", Some("kafka"), Some("kafka-0"), None))
        .await;
    let before = host.cluster.pod_reads.load(Ordering::SeqCst);
    host.cluster
        .pod_answers
        .lock()
        .unwrap()
        .extend([failed_read(), failed_read(), failed_read()]);
    still_open(&mut tunnel, 2).await;
    assert_eq!(host.cluster.pod_reads.load(Ordering::SeqCst), before + 2);
    let why = ends(&mut tunnel).await;
    assert_eq!(
        why,
        format!("The connection to pod team/kafka-0 ended: Pod team/kafka-0 could not be read: {FORBIDDEN}")
    );
}

/// A read that answers starts the count again: a blip must not drop a
/// tunnel.
#[tokio::test(start_paused = true)]
async fn a_pod_read_that_recovers_keeps_the_tunnel() {
    let host = host();
    let mut tunnel = host
        .connect(ask("broker", Some("kafka"), Some("kafka-0"), None))
        .await;
    let before = host.cluster.pod_reads.load(Ordering::SeqCst);
    host.cluster.pod_answers.lock().unwrap().extend([
        failed_read(),
        failed_read(),
        None,
        failed_read(),
        failed_read(),
    ]);
    still_open(&mut tunnel, 5).await;
    assert_eq!(host.cluster.pod_reads.load(Ordering::SeqCst), before + 5);
    assert!(host.cluster.pod_answers.lock().unwrap().is_empty());
}

/// A pod read that does not answer — an API server that is slow, up to the
/// request timeout — never holds up the app's recheck: access taken away on
/// disk by another process ends the tunnel within one tick.
#[tokio::test(start_paused = true)]
async fn access_removed_during_a_held_pod_read_ends_the_tunnel() {
    let host = host();
    let mut tunnel = host
        .connect(ask("broker", Some("kafka"), Some("kafka-0"), None))
        .await;
    let before = host.cluster.pod_reads.load(Ordering::SeqCst);
    host.cluster.hold_pod_reads.store(true, Ordering::SeqCst);
    still_open(&mut tunnel, 1).await;
    assert_eq!(host.cluster.pod_reads.load(Ordering::SeqCst), before + 1);
    // The cluster taken off the app's list, as another process saves it.
    let mut state = super::read(&host.path).unwrap();
    state
        .plugins
        .iter_mut()
        .find(|p| p.manifest.id == KAFKA)
        .unwrap()
        .contexts = Some(vec![format!("{}#cluster/b", host.kubeconfig.display())]);
    super::write(&host.path, &state).unwrap();
    let why = ends(&mut tunnel).await;
    assert_eq!(
        why,
        "The connection to pod team/kafka-0 ended: App is not enabled for this cluster"
    );
}

/// One pod read at a time: a read still pending at the next tick is waited
/// for, not joined by another. Only a read that finishes counts towards the
/// failed reads that end a tunnel; the cluster's request timeout bounds it.
#[tokio::test(start_paused = true)]
async fn a_held_pod_read_does_not_start_another() {
    let host = host();
    let mut tunnel = host
        .connect(ask("broker", Some("kafka"), Some("kafka-0"), None))
        .await;
    let before = host.cluster.pod_reads.load(Ordering::SeqCst);
    host.cluster.hold_pod_reads.store(true, Ordering::SeqCst);
    still_open(&mut tunnel, 5).await;
    assert_eq!(host.cluster.pod_reads.load(Ordering::SeqCst), before + 1);
}
