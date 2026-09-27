//! A sidecar calling back into the host (#573), against the real app facade:
//! `srelens_plugin_host::sidecar::CapabilityBroker` in front of the
//! `extensions.read`, `extensions.resource` and `extensions.action` this module
//! registers, so every refusal here is the one the UI gets. The last test runs
//! a whole supervisor with a scripted sidecar and reads every byte srelens
//! wrote to it, for the credentials that must never be there.
use super::tests::{fake_core, manifest};
use super::*;
use srelens_capability::audit::{AuditRecord, AuditSink};
use srelens_plugin_host::sidecar::data::DataDir;
use srelens_plugin_host::sidecar::protocol::code;
use srelens_plugin_host::sidecar::{
    AppIdentity, Broker, CapabilityBroker, Consent, ConsentRequest, Enforcement, Exit, LaunchError,
    Launched, Launcher, Limits, NoConsent, Policy, Process, SidecarCommand, SidecarConfig,
    SidecarStatus, Supervisor,
};
use srelens_plugin_host::{SecretStore, SecretValue, SECRET_STORE_PERMISSION};
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const WRITER: &str = "org.example.argocd";
const METRICS: &str = "org.example.metrics";

/// A value shaped like a credential and made at run time, so none is written in
/// the source. No assertion message prints one (CodeQL rust/cleartext-logging).
fn stand_in(label: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{label}-not-a-credential-{nanos:x}")
}

/// The credentials a host holds that a sidecar must never see.
struct Credentials {
    /// The kubeconfig user's bearer token.
    token: String,
    /// Its client key, as a kubeconfig carries it.
    client_key: String,
    /// The metrics app's API token, in the host's secret store (#543).
    api_token: String,
}

impl Credentials {
    fn new() -> Credentials {
        Credentials {
            token: stand_in("kube-token"),
            client_key: stand_in("client-key"),
            api_token: stand_in("api-token"),
        }
    }

    /// Whether `text` holds any of them, or the kubeconfig's own contents.
    fn found_in(&self, text: &str) -> bool {
        [
            self.token.as_str(),
            self.client_key.as_str(),
            self.api_token.as_str(),
            "client-key-data",
            "kind: Config",
        ]
        .iter()
        .any(|secret| text.contains(secret))
    }
}

/// A kubeconfig with one context, `kind-dev`, whose user holds the token and,
/// with `key`, a client key. Without one a real client is built from the
/// token alone and sends it, to a server that is not there.
fn kubeconfig(dir: &Path, credentials: &Credentials, key: bool) -> PathBuf {
    let path = dir.join("config");
    let key = if key {
        format!("    client-key-data: {}\n", credentials.client_key)
    } else {
        String::new()
    };
    std::fs::write(
        &path,
        format!(
            "apiVersion: v1\nkind: Config\nclusters:\n- name: c\n  cluster:\n    server: https://127.0.0.1:1\n\
             users:\n- name: u\n  user:\n    token: {}\n{key}\
             contexts:\n- name: kind-dev\n  context: {{cluster: c, user: u}}\ncurrent-context: kind-dev\n",
            credentials.token
        ),
    )
    .unwrap();
    path
}

/// The fake cluster, with the two primitives the writer's calls reach answering
/// with what they were given: whatever the facade passes a capability comes
/// back to the sidecar, so a credential in it would too.
fn core() -> Arc<Registry> {
    let mut core = (*fake_core()).clone();
    for id in ["k8s.annotate", "k8s.getCustomResource"] {
        let mut cap = core.get(id).unwrap().clone();
        cap.handler = Arc::new(|args| Box::pin(async move { Ok(json!({"given": args})) }));
        core.register(cap);
    }
    Arc::new(core)
}

/// The same, but with the host's real `k8s.getCustomResource`, reaching the
/// kubeconfig's cluster through `cache`: a kube client is built from the
/// user's token and key, and its failure to connect is what the sidecar hears.
fn core_on(cache: Arc<srelens_kube::client_cache::ClientCache>) -> Arc<Registry> {
    let real = crate::build_registry_with_paths(cache, vec![]);
    let mut core = (*core()).clone();
    core.register(real.get("k8s.getCustomResource").unwrap().clone());
    Arc::new(core)
}

/// An app that reads Argo CD applications and may refresh one.
fn writer() -> String {
    let mut value: Value = serde_json::from_str(&manifest()).unwrap();
    value["permissions"] = json!(["k8s.listCustomResource", "k8s.annotate"]);
    value["actions"] = json!([{"name":"refresh","title":"Refresh","target":"k8s.annotate",
        "resource":"applications","arguments":{"key":"argocd.argoproj.io/refresh","value":"normal"}}]);
    value.to_string()
}

/// One Prometheus query, its URL a setting and its token a secret sent as a
/// bearer header (#568).
fn metrics() -> String {
    json!({
        "id":METRICS, "name":"Metrics", "version":"0.1.0", "srelensApiVersion":"^0.4",
        "kind":"declarative",
        "permissions":[
            {"capability":"network.http","hosts":["${settings.prometheusUrl}"]},
            SECRET_STORE_PERMISSION
        ],
        "settings":[
            {"id":"prometheusUrl","type":"url","title":"Prometheus URL","required":true},
            {"id":"token","type":"secret-reference","title":"API token"}
        ],
        "capabilities":[{"name":"up","title":"Targets up","target":"network.http",
            "arguments":{"url":"${settings.prometheusUrl}","path":"/api/v1/query","query":{"query":"up"},
                "secretHeaders":{"Authorization":{"secret":"token","prefix":"Bearer "}}},
            "inputs":[]}],
        "contributions":{"pages":[],"detailTabs":[],"detailLinks":[]}
    })
    .to_string()
}

/// The desktop vault as an unlocked one behaves.
#[derive(Default)]
struct Vault(Mutex<BTreeMap<String, String>>);

impl SecretStore for Vault {
    fn status(&self) -> Result<(), String> {
        Ok(())
    }
    fn put(&self, key: &str, value: &SecretValue) -> Result<(), String> {
        self.0
            .lock()
            .unwrap()
            .insert(key.into(), value.expose().into());
        Ok(())
    }
    fn contains(&self, key: &str) -> Result<bool, String> {
        Ok(self.0.lock().unwrap().contains_key(key))
    }
    fn retain(&self, keep: &BTreeSet<String>) -> Result<(), String> {
        self.0.lock().unwrap().retain(|key, _| keep.contains(key));
        Ok(())
    }
    fn reveal(&self, key: &str) -> Result<Option<SecretValue>, String> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .get(key)
            .cloned()
            .map(SecretValue::new))
    }
}

/// A local server standing in for Prometheus: it answers every request with
/// the same JSON, and never echoes what it was sent. It keeps the
/// `Authorization` headers it received, to show the secret was sent to it.
async fn prometheus(authorizations: Arc<Mutex<Vec<String>>>) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let authorizations = authorizations.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                while reader.read_line(&mut line).await.unwrap_or(0) > 2 {
                    if let Some((name, value)) = line.split_once(':') {
                        if name.eq_ignore_ascii_case("authorization") {
                            authorizations.lock().unwrap().push(value.trim().to_owned());
                        }
                    }
                    line.clear();
                }
                let body = json!({"status": "success"}).to_string();
                let mut stream = reader.into_inner();
                let _ = stream
                    .write_all(
                        format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
                            .as_bytes(),
                    )
                    .await;
                let _ = stream.shutdown().await;
            });
        }
    });
    addr
}

#[derive(Default)]
struct Spy(Mutex<Vec<AuditRecord>>);

impl AuditSink for Spy {
    fn record(&self, rec: AuditRecord) {
        self.0.lock().unwrap().push(rec);
    }
}

impl Spy {
    fn records(&self) -> Vec<AuditRecord> {
        self.0.lock().unwrap().clone()
    }
}

struct Approve;

impl Consent for Approve {
    fn confirm<'a>(
        &'a self,
        _request: &'a ConsentRequest,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }
}

/// A host with the writer and the metrics app installed and set up, and the
/// facade registered against the kubeconfig above.
struct Host {
    dir: tempfile::TempDir,
    path: PathBuf,
    registry: Arc<Registry>,
    audit: Arc<Spy>,
    credentials: Credentials,
    /// What the stand-in Prometheus was sent as `Authorization`.
    authorizations: Arc<Mutex<Vec<String>>>,
}

impl Host {
    async fn new() -> Host {
        Host::with(false).await
    }

    /// With `real_cluster`, `host/resource` and the check `host/action` makes
    /// first reach the kubeconfig's cluster through a real kube client.
    async fn with(real_cluster: bool) -> Host {
        let dir = tempfile::tempdir().unwrap();
        let credentials = Credentials::new();
        let path = dir.path().join("settings.extensions.json");
        let config = kubeconfig(dir.path(), &credentials, !real_cluster);
        let cache = srelens_kube::client_cache::ClientCache::new_many(vec![config]);
        let core = if real_cluster {
            core_on(cache.clone())
        } else {
            core()
        };
        let mut registry = Registry::new();
        register_with_secrets(
            &mut registry,
            path.clone(),
            core,
            cache,
            Arc::new(Vault::default()),
        );
        let host = Host {
            dir,
            path,
            registry: Arc::new(registry),
            audit: Arc::new(Spy::default()),
            credentials,
            authorizations: Arc::default(),
        };
        let server = prometheus(host.authorizations.clone()).await;
        for step in [
            json!({"action":"unsignedApps","allowUnsignedApps":true}),
            json!({"action":"install","manifest":writer(),"grants":["k8s.listCustomResource","k8s.annotate"]}),
            json!({"action":"install","manifest":metrics(),"grants":["network.http", SECRET_STORE_PERMISSION]}),
            json!({"action":"settings","id":METRICS,"settings":{"prometheusUrl": format!("http://{server}")}}),
            json!({"action":"loopbackHttp","id":METRICS,"allowLoopbackHttp":true}),
        ] {
            host.registry
                .invoke("extensions.configure", step)
                .await
                .unwrap();
        }
        host.registry
            .invoke(
                "extension.secretStore",
                json!({"action":"set","id":METRICS,"setting":"token","secret":host.credentials.api_token}),
            )
            .await
            .unwrap();
        host
    }

    fn identity(&self, id: &str) -> AppIdentity {
        let state = read(&self.path).unwrap();
        let app = state.plugins.iter().find(|p| p.manifest.id == id).unwrap();
        AppIdentity {
            id: id.into(),
            revision: app.revision,
            name: app.manifest.name.clone(),
            publisher: None,
        }
    }

    fn broker(&self, id: &str, consent: Arc<dyn Consent>) -> CapabilityBroker {
        CapabilityBroker::new(
            self.registry.clone(),
            self.identity(id),
            self.audit.clone(),
            consent,
        )
    }

    /// Change the installed app as a hand-edited inventory would.
    fn edit(&self, id: &str, change: impl FnOnce(&mut Installed)) {
        let mut state = read(&self.path).unwrap();
        change(
            state
                .plugins
                .iter_mut()
                .find(|p| p.manifest.id == id)
                .unwrap(),
        );
        write(&self.path, &state).unwrap();
    }
}

fn context() -> Value {
    json!({"clusterId": "kind-dev", "namespace": "team"})
}

fn read_params(capability: &str) -> Value {
    json!({"context": context(), "capability": capability})
}

fn refresh() -> Value {
    json!({"context": context(), "capability": "applications", "name": "web",
           "action": "refresh", "uid": "u-1", "resourceVersion": "42"})
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sidecar_reads_through_the_facade_on_the_cluster_it_names() {
    let host = Host::new().await;
    let broker = host.broker(WRITER, Arc::new(NoConsent));
    let answer = broker
        .call("host/read", read_params("applications"))
        .await
        .unwrap();
    // The fake cluster answers with what the reader was given: the app's own
    // bound selector, on the pinned context and the namespace the call named.
    let text = answer.to_string();
    assert!(text.contains("argoproj.io"), "{text}");
    assert!(text.contains("kind-dev") && text.contains("team"), "{text}");
    assert!(
        !host.credentials.found_in(&text),
        "a credential reached the answer"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_ungranted_capability_is_refused() {
    let host = Host::new().await;
    let broker = host.broker(WRITER, Arc::new(Approve));
    // A host capability named directly is no call at all.
    for method in [
        "k8s.annotate",
        "k8s.getSecret",
        "extensions.action",
        "network.http",
    ] {
        let error = broker
            .call(method, read_params("applications"))
            .await
            .unwrap_err();
        assert_eq!(error.code, code::METHOD_NOT_FOUND, "{method}: {error:?}");
    }
    // A binding the app does not declare.
    let error = broker
        .call("host/read", read_params("secrets"))
        .await
        .unwrap_err();
    assert_eq!(error.code, code::INVALID_PARAMS, "{error:?}");
    assert!(
        error.message.contains("declares no capability `secrets`"),
        "{error}"
    );
    // A grant taken back, as a hand-edited inventory would: both the write and
    // the read that depend on it are refused by the facade's own check.
    host.edit(WRITER, |app| app.grants.retain(|g| g != "k8s.annotate"));
    let error = broker.call("host/action", refresh()).await.unwrap_err();
    assert_eq!(error.code, code::CAPABILITY_FAILED, "{error:?}");
    assert!(
        error.message.contains("k8s.annotate was not granted"),
        "{error}"
    );
    host.edit(WRITER, |app| app.grants.clear());
    let error = broker
        .call("host/read", read_params("applications"))
        .await
        .unwrap_err();
    assert_eq!(error.code, code::CAPABILITY_FAILED, "{error:?}");
    assert!(error.message.contains("granted"), "{error}");
    // Writes are what the trail records, and none ran.
    let wrote = host.audit.records().iter().any(|r| {
        r.tool == "extensions.action" && r.outcome == srelens_capability::audit::OUTCOME_OK
    });
    assert!(!wrote, "an ungranted write ran");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_app_not_enabled_for_the_cluster_is_refused_there() {
    let host = Host::new().await;
    host.edit(WRITER, |app| {
        app.contexts = Some(vec!["elsewhere#other".into()])
    });
    let broker = host.broker(WRITER, Arc::new(NoConsent));
    let error = broker
        .call("host/read", read_params("applications"))
        .await
        .unwrap_err();
    assert_eq!(error.code, code::CAPABILITY_FAILED);
    assert_eq!(error.message, NOT_ENABLED_FOR_CLUSTER);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sidecar_of_an_updated_or_disabled_app_is_refused() {
    let host = Host::new().await;
    let broker = host.broker(WRITER, Arc::new(NoConsent));
    host.registry
        .invoke(
            "extensions.configure",
            json!({"action":"enable","id":WRITER,"enabled":false}),
        )
        .await
        .unwrap();
    let error = broker
        .call("host/read", read_params("applications"))
        .await
        .unwrap_err();
    assert!(error.message.contains("disabled"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_call_without_explicit_context_is_refused_by_the_real_broker() {
    let host = Host::new().await;
    let broker = host.broker(WRITER, Arc::new(Approve));
    for params in [
        json!({"capability": "applications"}),
        json!({"capability": "applications", "context": {"clusterId": "kind-dev"}}),
    ] {
        let error = broker.call("host/read", params).await.unwrap_err();
        assert_eq!(error.code, code::INVALID_PARAMS);
    }
    let mut action = refresh();
    action.as_object_mut().unwrap().remove("context");
    assert_eq!(
        broker.call("host/action", action).await.unwrap_err().code,
        code::INVALID_PARAMS
    );
    assert!(
        host.audit.records().is_empty(),
        "{:?}",
        host.audit.records()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_action_is_confirmed_then_run_and_recorded_as_the_app() {
    let host = Host::new().await;
    let revision = host.identity(WRITER).revision;
    let refused = host.broker(WRITER, Arc::new(NoConsent));
    let error = refused.call("host/action", refresh()).await.unwrap_err();
    assert_eq!(error.code, code::CONSENT_DENIED);

    let broker = host.broker(WRITER, Arc::new(Approve));
    let answer = broker.call("host/action", refresh()).await.unwrap();
    assert_eq!(answer["given"]["name"], "web");
    assert_eq!(answer["given"]["namespace"], "team");

    let records = host.audit.records();
    assert_eq!(records.len(), 2, "{records:?}");
    let app = Some(srelens_capability::audit::AppRef {
        id: WRITER.into(),
        revision,
    });
    for (record, decision, outcome) in [
        (
            &records[0],
            "denied",
            srelens_capability::audit::OUTCOME_REJECTED,
        ),
        (
            &records[1],
            "approved",
            srelens_capability::audit::OUTCOME_OK,
        ),
    ] {
        assert_eq!(record.source.as_str(), "app");
        assert_eq!(record.source.transport(), "sidecar");
        assert_eq!(record.tool, "extensions.action");
        assert_eq!(record.decision, decision);
        assert_eq!(record.outcome, outcome);
        assert_eq!(record.app, app);
        assert_eq!(record.cluster.as_deref(), Some("kind-dev"));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn network_http_is_reached_through_the_broker_with_the_secret_injected_and_not_answered() {
    let host = Host::new().await;
    let broker = host.broker(METRICS, Arc::new(NoConsent));
    let answer = broker.call("host/read", read_params("up")).await.unwrap();
    assert_eq!(answer["status"], 200);
    assert_eq!(answer["body"], json!({"status": "success"}));
    assert!(
        !host.credentials.found_in(&answer.to_string()),
        "a credential reached the answer"
    );
}

/// Answers the host's lifecycle calls, then makes `script`'s calls one at a
/// time, recording every line srelens writes to it and every answer it gets.
#[derive(Clone)]
struct Scripted {
    script: Vec<(&'static str, Value)>,
    heard: Arc<Mutex<String>>,
    answers: Arc<Mutex<Vec<Value>>>,
    done: Arc<tokio::sync::Notify>,
}

impl Scripted {
    fn new(script: Vec<(&'static str, Value)>) -> Scripted {
        Scripted {
            script,
            heard: Arc::default(),
            answers: Arc::default(),
            done: Arc::default(),
        }
    }
}

impl Launcher for Scripted {
    fn enforcement(&self) -> Enforcement {
        Enforcement::Kernel
    }

    fn launch(&self, _command: &SidecarCommand, _limits: &Limits) -> Result<Launched, LaunchError> {
        let (stdin, host_stdin) = tokio::io::duplex(1 << 20);
        let (host_stdout, mut stdout) = tokio::io::duplex(1 << 20);
        let (_stderr_writer, host_stderr) = tokio::io::duplex(1024);
        let me = self.clone();
        let task = tokio::spawn(async move {
            let _stderr = _stderr_writer;
            let mut lines = BufReader::new(host_stdin).lines();
            let mut script = me.script.clone().into_iter().enumerate();
            let send = |value: Value| format!("{value}\n");
            while let Ok(Some(line)) = lines.next_line().await {
                me.heard.lock().unwrap().push_str(&line);
                me.heard.lock().unwrap().push('\n');
                let message: Value = serde_json::from_str(&line).unwrap();
                let next_call = |script: &mut dyn Iterator<
                    Item = (usize, (&'static str, Value)),
                >| {
                    script.next().map(|(n, (method, params))| {
                        send(json!({"jsonrpc":"2.0","id":format!("c-{n}"),"method":method,"params":params}))
                    })
                };
                let reply = match (message["method"].as_str(), message.get("id")) {
                    (Some("initialize"), Some(id)) => {
                        send(json!({"jsonrpc":"2.0","id":id,"result":{"apiVersion":"0.1.0"}}))
                    }
                    (Some("activate"), Some(id)) => {
                        let mut out = send(json!({"jsonrpc":"2.0","id":id,"result":{}}));
                        out.push_str(&next_call(&mut script).unwrap_or_default());
                        out
                    }
                    (Some("shutdown"), Some(id)) => {
                        let _ = stdout
                            .write_all(
                                send(json!({"jsonrpc":"2.0","id":id,"result":{}})).as_bytes(),
                            )
                            .await;
                        return;
                    }
                    (Some(_), Some(id)) => send(json!({"jsonrpc":"2.0","id":id,"result":{}})),
                    (Some(_), None) => continue,
                    // The answer to one of its calls: make the next.
                    (None, Some(_)) => {
                        me.answers.lock().unwrap().push(message.clone());
                        match next_call(&mut script) {
                            Some(call) => call,
                            None => {
                                me.done.notify_one();
                                continue;
                            }
                        }
                    }
                    (None, None) => continue,
                };
                if stdout.write_all(reply.as_bytes()).await.is_err() {
                    return;
                }
            }
        });
        let abort = task.abort_handle();
        let exit = async move {
            let killed = task.await.is_err();
            Exit {
                description: if killed {
                    "was killed".into()
                } else {
                    "exited with status 0".into()
                },
                code: (!killed).then_some(0),
                signal: None,
                memory_limit: false,
            }
        };
        Ok(Launched {
            stdin: Box::new(stdin),
            stdout: Box::new(host_stdout),
            stderr: Box::new(host_stderr),
            process: Process::new(Some(1), exit, move || abort.abort()),
        })
    }
}

/// Run `script` in a supervised sidecar of `app` and return what srelens wrote
/// to it, and the answers its calls got.
async fn run(host: &Host, app: &str, script: Vec<(&'static str, Value)>) -> (String, Vec<Value>) {
    let sidecar = Scripted::new(script);
    let data = DataDir::for_app(&host.dir.path().join("data"), app).unwrap();
    let config = SidecarConfig {
        command: SidecarCommand {
            app_id: app.into(),
            program: "/opt/example/sidecar".into(),
            args: Vec::new(),
            env: Vec::new(),
            data_dir: data.path().to_owned(),
        },
        limits: Limits::default(),
        policy: Policy::default(),
    };
    let broker = host.broker(app, Arc::new(Approve));
    let supervisor = Supervisor::start(config, Arc::new(sidecar.clone()), Arc::new(broker));
    tokio::time::timeout(std::time::Duration::from_secs(60), sidecar.done.notified())
        .await
        .unwrap_or_else(|_| {
            panic!(
                "the script did not finish; status {:?}",
                supervisor.status()
            )
        });
    assert!(matches!(supervisor.status(), SidecarStatus::Running { .. }));
    supervisor.stop().await;
    let heard = sidecar.heard.lock().unwrap().clone();
    let answers = sidecar.answers.lock().unwrap().clone();
    (heard, answers)
}

/// What #573 promises most plainly: nothing srelens writes on a sidecar's pipe
/// carries a kubeconfig, a token or a secret's value. Every kind of call is
/// made, answered and refused, and the whole of what the host wrote is read.
#[tokio::test(flavor = "multi_thread")]
async fn no_credential_material_appears_on_the_rpc_channel() {
    let host = Host::new().await;
    // The check can see a credential where there is one.
    let kubeconfig = std::fs::read_to_string(host.dir.path().join("config")).unwrap();
    assert!(
        host.credentials.found_in(&kubeconfig),
        "the check sees nothing"
    );
    let secret_probe = json!({"context": context(), "namespace": "team", "name": "db-password"});
    let (heard, answers) = run(
        &host,
        WRITER,
        vec![
            ("host/read", read_params("applications")),
            (
                "host/resource",
                json!({"context": context(), "capability": "applications", "name": "web"}),
            ),
            ("host/action", refresh()),
            // Refused ones: their errors are on the channel too.
            ("host/read", json!({"capability": "applications"})),
            ("host/read", read_params("secrets")),
            ("k8s.getSecret", secret_probe),
            ("k8s.listContexts", json!({})),
            ("k8s.synthesizeClusterKubeconfig", json!({})),
        ],
    )
    .await;
    assert_eq!(answers.len(), 8, "{answers:?}");
    assert!(
        answers[0].get("result").is_some(),
        "the read was answered: {}",
        answers[0]
    );
    assert!(
        answers[1].get("result").is_some(),
        "the resource was answered: {}",
        answers[1]
    );
    assert!(
        answers[2].get("result").is_some(),
        "the action ran: {}",
        answers[2]
    );
    for refused in &answers[3..] {
        assert!(refused.get("error").is_some(), "{refused}");
    }
    assert!(
        heard.contains("\"initialize\""),
        "the transcript is the whole channel"
    );
    assert!(
        !host.credentials.found_in(&heard),
        "a credential was written to the sidecar"
    );

    // The host's own client, built from the token and key, failing to reach
    // the cluster: its error is on the channel too.
    let real = Host::with(true).await;
    let (heard, answers) = run(
        &real,
        WRITER,
        vec![
            (
                "host/resource",
                json!({"context": context(), "capability": "applications", "name": "web"}),
            ),
            ("host/action", refresh()),
        ],
    )
    .await;
    for failed in &answers {
        assert!(failed.get("error").is_some(), "{failed}");
    }
    assert!(
        !real.credentials.found_in(&heard),
        "a credential was written to the sidecar"
    );

    let (heard, answers) = run(&host, METRICS, vec![("host/read", read_params("up"))]).await;
    assert_eq!(answers[0]["result"]["body"], json!({"status": "success"}));
    let sent = host.authorizations.lock().unwrap().clone();
    let bearer = format!("Bearer {}", host.credentials.api_token);
    assert!(
        sent.contains(&bearer),
        "the host did not send the app's secret"
    );
    assert!(
        !host.credentials.found_in(&heard),
        "a credential was written to the sidecar"
    );
}

#[test]
fn removing_an_app_removes_its_data_directory_and_keeps_the_others() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.extensions.json");
    let apps = Apps::from(path.clone());
    let root = apps
        .data_root()
        .expect("the desktop keeps app data")
        .to_owned();
    assert_eq!(root, path.with_extension("data"));
    super::tests::install(&path, fake_core());
    let kept = DataDir::for_app(&root, WRITER).unwrap();
    std::fs::write(kept.path().join("cache.db"), "x").unwrap();
    // Left behind by an app removed while srelens was not running.
    let stale = DataDir::for_app(&root, "org.example.gone").unwrap();
    super::tests::configure(
        &path,
        json!({"action":"enable","id":WRITER,"enabled":false}),
    )
    .unwrap();
    assert!(
        kept.path().join("cache.db").is_file(),
        "a change that keeps the app keeps its data"
    );
    assert!(
        !stale.path().exists(),
        "an uninstalled app's data outlived it"
    );
    super::tests::configure(&path, json!({"action":"remove","id":WRITER})).unwrap();
    assert!(!kept.path().exists(), "the removed app's data outlived it");
}

#[test]
fn a_host_that_keeps_no_app_files_has_no_data_root() {
    let apps = Apps::with_shared_catalog(
        Arc::new(PathBuf::from("/nonexistent/inventory.json")),
        SharedCatalog::new(PathBuf::from("/nonexistent/catalog.json")),
    );
    assert!(apps.data_root().is_none());
}
