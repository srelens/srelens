use super::*;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt as _, AsyncWriteExt, BufReader};

type Requests = Arc<Mutex<Vec<(String, String, Value)>>>;
#[derive(Clone, Copy)]
enum Scenario {
    Complete,
    Failed,
    Pending,
    DeniedReader,
}

async fn server(scenario: Scenario) -> (Client, Requests, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let requests: Requests = Arc::new(Mutex::new(vec![]));
    let seen = requests.clone();
    let deleted = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let gone = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let task = tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let (seen, deleted, gone) = (seen.clone(), deleted.clone(), gone.clone());
            tokio::spawn(async move {
                let mut reader = BufReader::new(socket);
                let mut line = String::new();
                reader.read_line(&mut line).await.unwrap();
                let words: Vec<_> = line.split_whitespace().collect();
                let (method, path) = (words[0].to_owned(), words[1].to_owned());
                let resource = path.split('?').next().unwrap().to_owned();
                let mut length = 0;
                loop {
                    line.clear();
                    reader.read_line(&mut line).await.unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some((name, value)) = line.split_once(':') {
                        if name.eq_ignore_ascii_case("content-length") {
                            length = value.trim().parse::<usize>().unwrap();
                        }
                    }
                }
                let mut bytes = vec![0; length];
                reader.read_exact(&mut bytes).await.unwrap();
                let input: Value = if bytes.is_empty() {
                    Value::Null
                } else {
                    serde_json::from_slice(&bytes).unwrap()
                };
                seen.lock()
                    .unwrap()
                    .push((method.clone(), path.clone(), input.clone()));
                let mut code = "200 OK";
                let body = if method == "DELETE" {
                    deleted.store(true, std::sync::atomic::Ordering::SeqCst);
                    json!({"kind":"Status","apiVersion":"v1","status":"Success"}).to_string()
                } else if resource.ends_with("/jobs/scan-1")
                    && method == "GET"
                    && deleted.load(std::sync::atomic::Ordering::SeqCst)
                {
                    if gone.swap(true, std::sync::atomic::Ordering::SeqCst) {
                        code = "404 Not Found";
                        json!({"kind":"Status","apiVersion":"v1","status":"Failure","reason":"NotFound","message":"gone","code":404}).to_string()
                    } else {
                        json!({"metadata":{"name":"scan-1","uid":"j-1","deletionTimestamp":"2026-10-06T12:00:00Z"}}).to_string()
                    }
                } else if matches!(scenario, Scenario::DeniedReader) && resource.ends_with("/roles")
                {
                    code = "403 Forbidden";
                    json!({"kind":"Status","apiVersion":"v1","status":"Failure","reason":"Forbidden","message":"cannot grant reader permissions","code":403}).to_string()
                } else if resource.ends_with("/jobs") || resource.ends_with("/jobs/scan-1") {
                    let status = match scenario {
                        Scenario::Complete => {
                            json!({"conditions":[{"type":"Complete","status":"True"}],"succeeded":1})
                        }
                        Scenario::Failed => {
                            json!({"conditions":[{"type":"Failed","status":"True"}],"failed":1})
                        }
                        _ => json!({"active":1}),
                    };
                    json!({"metadata":{"name":"scan-1","uid":"j-1"},"status":status}).to_string()
                } else if resource == "/api/v1/namespaces/team/pods" {
                    json!({"kind":"PodList","apiVersion":"v1","metadata":{},"items":[{"metadata":{"name":"worker-1","uid":"p-1","ownerReferences":[{"apiVersion":"batch/v1","kind":"Job","name":"scan-1","uid":"j-1","controller":true}]},"spec":{"containers":[{"name":"worker"}]},"status":{"containerStatuses":[{"name":"worker","image":"trivy","imageID":"sha256:a","ready":false,"restartCount":0,"state":{"terminated":{"exitCode":0}}}]}}]}).to_string()
                } else if resource.ends_with("/log") {
                    json!({"SchemaVersion":2,"Results":[]}).to_string()
                } else {
                    input.to_string()
                };
                let mut socket = reader.into_inner();
                socket.write_all(format!("HTTP/1.1 {code}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
                socket.shutdown().await.unwrap();
            });
        }
    });
    let config = srelens_kube::kube::Config::new(format!("http://{address}").parse().unwrap());
    (Client::try_from(config).unwrap(), requests, task)
}

fn template() -> JobTemplate {
    serde_json::from_value(json!({"image":format!("aquasec/trivy@sha256:{}","a".repeat(64)),"command":["trivy"],"args":["k8s","${inputs.namespace}"],"inputNames":["namespace"],"readRules":[{"apiGroups":["apps"],"resources":["deployments"],"verbs":["get","list"]}]})).unwrap()
}
fn active() -> Active {
    Active::take(Arc::new(Mutex::new(HashSet::new())), "org.example.app").unwrap()
}
fn job(template: &JobTemplate) -> Job {
    build_job(
        "org.example.app",
        "team",
        "run-1",
        template,
        &BTreeMap::from([("namespace".into(), "team".into())]),
    )
    .unwrap()
}

#[tokio::test]
async fn facade_rechecks_enabled_revision_scope_grants_and_binding_before_any_cluster_write() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("apps.json");
    let core = super::super::tests::fake_core();
    super::super::tests::install(&path, core.clone());
    let mut stored: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    stored["allowUnsignedApps"] = json!(true);
    let app = &mut stored["plugins"][0];
    app["enabled"] = json!(true);
    app["package"] = json!("a".repeat(64));
    app["grants"] = json!(["k8s.runJob"]);
    app["manifest"]["srelensApiVersion"] = json!("^0.8");
    app["manifest"]["kind"] = json!("executable");
    app["manifest"]["permissions"] = json!(["k8s.runJob"]);
    app["manifest"]["sidecar"] = json!({"binaries":{"darwin-arm64":"bin/darwin-arm64/controller"},"operations":[{"name":"scan","title":"Scan","inputs":[]}]});
    app["manifest"]["capabilities"] = json!([{"name":"scan-worker","title":"Worker","target":"k8s.runJob","inputs":[],"arguments":{"image":format!("aquasec/trivy@sha256:{}","a".repeat(64)),"command":["trivy"],"args":["image","${inputs.image}"],"inputNames":["image"]}}]);
    app["manifest"]["contributions"] = json!({"pages":[],"detailTabs":[],"detailLinks":[]});
    let revision = app["revision"].as_u64().unwrap();
    let apps = super::super::Apps::from(path.clone());
    let (client, requests, server) = server(Scenario::Complete).await;
    let cache = srelens_kube::client_cache::ClientCache::new_many(vec![]);
    cache.preload("prod", client).await;
    let mut registry = Registry::new();
    register(&mut registry, &apps, core, cache);
    let valid = json!({"id":"org.example.argocd","revision":revision,"context":"prod","namespace":"team","capability":"scan-worker","inputs":{"image":"alpine:3.10"}});
    for (key, bad) in [
        ("enabled", json!(false)),
        ("revision", json!(revision + 1)),
        ("grants", json!([])),
        ("contexts", json!(["another-cluster"])),
    ] {
        let mut invalid = stored.clone();
        invalid["plugins"][0][key] = bad;
        std::fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
        assert!(
            registry
                .invoke("extensions.runJob", valid.clone())
                .await
                .is_err(),
            "{key}"
        );
    }
    std::fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();
    for (key, bad) in [
        ("capability", json!("other")),
        ("namespace", json!("")),
        ("inputs", json!({"image":"x","extra":"redirect"})),
    ] {
        let mut invalid = valid.clone();
        invalid[key] = bad;
        assert!(
            registry.invoke("extensions.runJob", invalid).await.is_err(),
            "{key}"
        );
    }
    assert!(
        requests.lock().unwrap().is_empty(),
        "an unauthorized call reached Kubernetes"
    );
    let result = registry.invoke("extensions.runJob", valid).await.unwrap();
    assert_eq!(result["namespace"], "team");
    assert!(requests
        .lock()
        .unwrap()
        .iter()
        .any(|(method, _, _)| method == "POST"));
    server.abort();
}

#[tokio::test]
async fn successful_job_owns_reader_access_collects_json_and_waits_for_foreground_cleanup() {
    let (client, seen, server) = server(Scenario::Complete).await;
    let data = tempfile::tempdir().unwrap();
    let template = template();
    let result = run_job(
        client,
        "team",
        "run-1",
        job(&template),
        &template,
        data.path(),
        active(),
    )
    .await
    .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&std::fs::read(data.path().join(&result.path)).unwrap())
            .unwrap()["SchemaVersion"],
        2
    );
    let seen = seen.lock().unwrap();
    let creates: Vec<_> = seen
        .iter()
        .filter(|(method, _, _)| method == "POST")
        .collect();
    assert_eq!(creates.len(), 5);
    assert!(creates[0].1.split('?').next().unwrap().ends_with("/jobs"));
    assert!(creates[4]
        .1
        .split('?')
        .next()
        .unwrap()
        .ends_with("/configmaps"));
    for (_, path, body) in creates.iter().skip(1) {
        assert!(path.contains("/namespaces/team/"));
        assert_eq!(body["metadata"]["ownerReferences"][0]["uid"], "j-1");
    }
    let deletion = seen
        .iter()
        .position(|(method, _, _)| method == "DELETE")
        .unwrap();
    assert_eq!(seen[deletion].2["preconditions"]["uid"], "j-1");
    assert_eq!(seen[deletion].2["propagationPolicy"], "Foreground");
    assert!(
        seen.iter()
            .skip(deletion + 1)
            .any(|(method, path, _)| method == "GET" && path.ends_with("/jobs/scan-1")),
        "cleanup returned before deletion completed"
    );
    server.abort();
}

#[tokio::test]
async fn denied_reader_access_and_failed_jobs_never_publish_a_result_and_clean_up() {
    for scenario in [Scenario::DeniedReader, Scenario::Failed] {
        let (client, seen, server) = server(scenario).await;
        let data = tempfile::tempdir().unwrap();
        let template = template();
        assert!(run_job(
            client,
            "team",
            "run-1",
            job(&template),
            &template,
            data.path(),
            active()
        )
        .await
        .is_err());
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if seen.lock().unwrap().iter().any(|(m, _, _)| m == "DELETE") {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(std::fs::read_dir(data.path()).unwrap().count(), 0);
        assert!(!seen
            .lock()
            .unwrap()
            .iter()
            .any(|(_, path, _)| path.contains("/log?")));
        server.abort();
    }
}

#[tokio::test]
async fn dropping_a_running_job_deletes_only_its_uid_and_releases_the_active_slot_after_cleanup() {
    let (client, seen, server) = server(Scenario::Pending).await;
    let data = tempfile::tempdir().unwrap();
    let template = template();
    let slots = Arc::new(Mutex::new(HashSet::new()));
    let permit = Active::take(slots.clone(), "org.example.app").unwrap();
    assert!(Active::take(slots.clone(), "org.example.app").is_err());
    {
        let running = run_job(
            client,
            "team",
            "run-1",
            job(&template),
            &template,
            data.path(),
            permit,
        );
        tokio::select! {
            _ = running => panic!("pending Job finished"),
            _ = async { loop { if seen.lock().unwrap().iter().any(|(_,path,_)|path.contains("/pods?")) { break; } tokio::time::sleep(Duration::from_millis(5)).await; } } => {}
        }
    }
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if slots.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let seen = seen.lock().unwrap();
    let deletes: Vec<_> = seen.iter().filter(|(m, _, _)| m == "DELETE").collect();
    assert_eq!(deletes.len(), 1);
    assert_eq!(deletes[0].2["preconditions"]["uid"], "j-1");
    assert!(seen
        .iter()
        .any(|(m, p, _)| m == "GET" && p.ends_with("/jobs/scan-1")));
    assert_eq!(std::fs::read_dir(data.path()).unwrap().count(), 0);
    server.abort();
}

// Explicit local acceptance only: never runs in the default suite or against
// the current context. The dedicated kind namespace is created by the caller.
#[tokio::test]
#[ignore = "creates a scoped scanner Job in kind-srelens-demo/srelens-trivy-test"]
async fn live_kind_namespace_scan_uses_the_constrained_runner() {
    let client =
        srelens_kube::client_cache::ClientCache::new_many(crate::default_kubeconfig_paths())
            .get("kind-srelens-demo")
            .await
            .unwrap();
    let template: JobTemplate = serde_json::from_value(json!({
        "image":"aquasec/trivy@sha256:af6acf9a6b85dfe389a1941505c0ce9efef52a4719635e1a962f022a3d855daa",
        "command":["trivy"],
        "args":["k8s","--format","json","--report","all","--scanners","vuln,misconfig","--disable-node-collector","--include-namespaces","${inputs.namespace}","--include-kinds","Deployment,StatefulSet,DaemonSet,Pod,Job,CronJob,Service,ConfigMap,ServiceAccount,Role,RoleBinding,NetworkPolicy,Ingress,PersistentVolumeClaim,LimitRange,ResourceQuota,PodDisruptionBudget","--exclude-owned","--timeout","15m","--parallel","1","--no-progress","--quiet"],
        "inputNames":["namespace"],
        "readRules":[
            {"apiGroups":[""],"resources":["pods","services","configmaps","serviceaccounts","persistentvolumeclaims","limitranges","resourcequotas"],"verbs":["get","list"]},
            {"apiGroups":["apps"],"resources":["deployments","statefulsets","daemonsets"],"verbs":["get","list"]},
            {"apiGroups":["batch"],"resources":["jobs","cronjobs"],"verbs":["get","list"]},
            {"apiGroups":["networking.k8s.io"],"resources":["ingresses","networkpolicies"],"verbs":["get","list"]},
            {"apiGroups":["rbac.authorization.k8s.io"],"resources":["roles","rolebindings"],"verbs":["get","list"]},
            {"apiGroups":["policy"],"resources":["poddisruptionbudgets"],"verbs":["get","list"]}
        ]
    })).unwrap();
    let run = run_id();
    let job = build_job(
        "org.example.job-proof",
        "srelens-trivy-test",
        &run,
        &template,
        &BTreeMap::from([("namespace".into(), "srelens-trivy-test".into())]),
    )
    .unwrap();
    let data = tempfile::tempdir().unwrap();
    let result = run_job(
        client,
        "srelens-trivy-test",
        &run,
        job,
        &template,
        data.path(),
        active(),
    )
    .await
    .unwrap();
    let raw = std::fs::read(data.path().join(&result.path)).unwrap();
    let evidence = Path::new("/private/tmp/srelens-executable-512-evidence");
    std::fs::write(evidence.join("trivy-job-live-namespace.json"), &raw).unwrap();
    std::fs::write(
        evidence.join("trivy-job-live-result.json"),
        serde_json::to_vec_pretty(&result).unwrap(),
    )
    .unwrap();
    let parsed: Value = serde_json::from_slice(&raw)
        .expect("complete JSON, without ignored listing errors or truncated logs");
    let resources = parsed["Resources"]
        .as_array()
        .expect("namespace resource results");
    assert!(resources.iter().any(|r| r["Name"] == "vulnerable-alpine"));
}
