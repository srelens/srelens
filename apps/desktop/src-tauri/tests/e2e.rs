//! Full capability-registry e2e suite against a real cluster (kind), issue #27.
//!
//! Drives the app's REAL capability registry — the exact same
//! `(cap.handler)(json_value).await` path the Tauri bridge and the MCP server
//! use — against a live kind cluster, and ENFORCES COVERAGE: every capability
//! registered in `srelens_desktop_lib::capabilities::build_registry_with` must
//! be exercised by this suite, or explicitly listed in `EXCLUDED` with a
//! written reason. A capability added later with no e2e case fails this test.
//! Mirrors the philosophy of `every_capability_is_mcp_exposed` in
//! `apps/desktop/src-tauri/src/capabilities.rs`, but end-to-end against a real
//! apiserver + real `helm` binary instead of a static registry shape check.
//!
//! Ignored by default — needs a live cluster and `helm`/`kubectl` on PATH.
//! Run with:
//!
//! ```sh
//! kind create cluster --name srelens-helm-e2e
//! cargo test -p srelens-desktop --test e2e -- --ignored --nocapture --test-threads=1
//! ```
//!
//! Override the context with `SRELENS_E2E_CONTEXT` (default
//! `kind-srelens-helm-e2e`). The suite creates and tears down a `srelens-e2e`
//! namespace plus one CRD; it never touches the real `~/.kube/config` (the
//! `k8s.deleteContext` case operates on a throwaway copy). It cordons and
//! drains the (single) node near the end and always uncordons it again, even
//! on panic, so the cluster is left usable and the suite is re-runnable
//! back-to-back with no manual cleanup.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use futures::FutureExt;
use serde_json::{json, Value};
use srelens_capability::Registry;
use srelens_desktop_lib::capabilities::build_registry_with;
use srelens_kube::client_cache::ClientCache;

const NS: &str = "srelens-e2e";
const DEPLOY: &str = "e2e-web";
const SVC: &str = "e2e-web";
const HEADLESS_SVC: &str = "e2e-headless";
const CM: &str = "e2e-config";
const SECRET: &str = "e2e-secret";
const SA: &str = "e2e-sa";
const ROLE: &str = "e2e-role";
const ROLEBINDING: &str = "e2e-rolebinding";
const JOB: &str = "e2e-job";
const CRONJOB: &str = "e2e-cronjob";
const PVC: &str = "e2e-pvc";
const PVC_POD: &str = "e2e-pvc-user";
const STS: &str = "e2e-sts";
const DS: &str = "e2e-ds";
const NETPOL: &str = "e2e-netpol";
const INGRESS: &str = "e2e-ingress";
const QUOTA: &str = "e2e-quota";
const LIMITS: &str = "e2e-limits";

const CRD_GROUP: &str = "e2e.srelens.dev";
const CRD_KIND: &str = "Widget";
const CRD_PLURAL: &str = "widgets";
const CRD_NAME: &str = "widgets.e2e.srelens.dev";
const WIDGET: &str = "e2e-widget";

const HELM_RELEASE: &str = "e2e-cap-suite";

fn context() -> String {
    std::env::var("SRELENS_E2E_CONTEXT").unwrap_or_else(|_| "kind-srelens-helm-e2e".to_string())
}

fn kubeconfig_paths() -> Vec<PathBuf> {
    if let Ok(kc) = std::env::var("KUBECONFIG") {
        return std::env::split_paths(&kc).collect();
    }
    let home = std::env::var("HOME").expect("HOME");
    vec![PathBuf::from(home).join(".kube/config")]
}

fn cache() -> Arc<ClientCache> {
    ClientCache::new_many(kubeconfig_paths())
}

/// Drives the real capability registry and tracks which capability ids have
/// been exercised, so the end-of-suite completeness assertion has something
/// to check against `reg.ids()`.
struct Harness {
    reg: Registry,
    covered: HashSet<String>,
}

impl Harness {
    fn new(reg: Registry) -> Self {
        Self {
            reg,
            covered: HashSet::new(),
        }
    }

    /// Record `id` as covered without invoking it (used when the real call
    /// happened through a second registry, e.g. `k8s.deleteContext`).
    fn mark(&mut self, id: &str) {
        self.covered.insert(id.to_string());
    }

    /// Probe `id` WITHOUT recording coverage — for deciding how to assert (e.g.
    /// whether a metrics API is serving). The real call still has to happen.
    async fn try_call(&self, id: &str, input: Value) -> Result<Value, String> {
        self.reg
            .invoke(id, input)
            .await
            .map_err(|e| format!("{e:?}"))
    }

    /// Invoke `id`, recording it covered; panics with a clear message on Err.
    async fn ok(&mut self, id: &str, input: Value) -> Value {
        self.mark(id);
        match self.reg.invoke(id, input.clone()).await {
            Ok(v) => v,
            Err(e) => panic!("capability {id} failed on {input}: {e:?}"),
        }
    }

    /// Invoke `id`, recording it covered; asserts the call returns Err (for
    /// negative paths) and returns the error message.
    async fn err(&mut self, id: &str, input: Value) -> String {
        self.mark(id);
        match self.reg.invoke(id, input.clone()).await {
            Ok(v) => panic!("capability {id} was expected to fail on {input} but returned {v}"),
            Err(e) => format!("{e:?}"),
        }
    }

    /// Invoke `id`, recording it covered; accepts Ok OR a clean Err (for
    /// env-dependent capabilities like metrics/network access). Never panics.
    async fn any(&mut self, id: &str, input: Value) -> Option<Value> {
        self.mark(id);
        match self.reg.invoke(id, input.clone()).await {
            Ok(v) => Some(v),
            Err(e) => {
                println!("  {id}: acceptable error (environment-dependent): {e:?}");
                None
            }
        }
    }
}

/// Capabilities genuinely excluded from this suite, with a written reason.
/// A capability registered later with no case here fails the coverage
/// assertion at the end of `full_capability_suite`.
const EXCLUDED: &[(&str, &str)] = &[
    (
        "toolbox.installKubectl",
        "downloads a real ~50MB binary from dl.k8s.io and writes to ~/.srelens/bin; \
         covered by unit tests with an injected fetch instead of hitting the network in CI",
    ),
    (
        "toolbox.installHelm",
        "downloads a real release tarball from get.helm.sh and writes to ~/.srelens/bin; \
         covered by unit tests with an injected fetch instead of hitting the network in CI",
    ),
];

fn deadline(secs: u64) -> Instant {
    Instant::now() + Duration::from_secs(secs)
}

async fn poll_sleep() {
    tokio::time::sleep(Duration::from_millis(1500)).await;
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// Everything but the CRD's custom resource instance (which needs the CRD's
/// REST endpoint to actually be live first). One big multi-document
/// `k8s.applyManifest` call, so fixture setup itself dogfoods `applyManifest`.
fn fixtures_yaml() -> String {
    format!(
        r#"apiVersion: v1
kind: Namespace
metadata:
  name: {NS}
---
apiVersion: apiextensions.k8s.io/v1
kind: CustomResourceDefinition
metadata:
  name: {CRD_NAME}
spec:
  group: {CRD_GROUP}
  names:
    kind: {CRD_KIND}
    plural: {CRD_PLURAL}
    singular: widget
    listKind: WidgetList
  scope: Namespaced
  versions:
  - name: v1
    served: true
    storage: true
    schema:
      openAPIV3Schema:
        type: object
        properties:
          spec:
            type: object
            x-kubernetes-preserve-unknown-fields: true
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: {DEPLOY}
  namespace: {NS}
  labels:
    app: {DEPLOY}
spec:
  replicas: 2
  selector:
    matchLabels:
      app: {DEPLOY}
  template:
    metadata:
      labels:
        app: {DEPLOY}
    spec:
      serviceAccountName: {SA}
      containers:
      - name: app
        image: busybox:1.36
        command: ["sh", "-c", "while true; do echo hello; sleep 5; done"]
---
apiVersion: v1
kind: Service
metadata:
  name: {SVC}
  namespace: {NS}
spec:
  selector:
    app: {DEPLOY}
  ports:
  - port: 80
    targetPort: 8080
---
apiVersion: v1
kind: Service
metadata:
  name: {HEADLESS_SVC}
  namespace: {NS}
spec:
  clusterIP: None
  selector:
    app: {STS}
  ports:
  - port: 80
---
apiVersion: v1
kind: ConfigMap
metadata:
  name: {CM}
  namespace: {NS}
data:
  greeting: hello
---
apiVersion: v1
kind: Secret
metadata:
  name: {SECRET}
  namespace: {NS}
type: Opaque
stringData:
  password: hunter2
---
apiVersion: v1
kind: ServiceAccount
metadata:
  name: {SA}
  namespace: {NS}
---
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: {ROLE}
  namespace: {NS}
rules:
- apiGroups: [""]
  resources: ["pods"]
  verbs: ["get", "list"]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata:
  name: {ROLEBINDING}
  namespace: {NS}
subjects:
- kind: ServiceAccount
  name: {SA}
  namespace: {NS}
roleRef:
  kind: Role
  name: {ROLE}
  apiGroup: rbac.authorization.k8s.io
---
apiVersion: batch/v1
kind: Job
metadata:
  name: {JOB}
  namespace: {NS}
spec:
  backoffLimit: 1
  template:
    spec:
      restartPolicy: Never
      containers:
      - name: job
        image: busybox:1.36
        command: ["sh", "-c", "echo job-done"]
---
apiVersion: batch/v1
kind: CronJob
metadata:
  name: {CRONJOB}
  namespace: {NS}
spec:
  schedule: "0 0 1 1 *"
  jobTemplate:
    spec:
      template:
        spec:
          restartPolicy: OnFailure
          containers:
          - name: cron
            image: busybox:1.36
            command: ["sh", "-c", "echo cron-tick"]
---
apiVersion: v1
kind: PersistentVolumeClaim
metadata:
  name: {PVC}
  namespace: {NS}
spec:
  accessModes: ["ReadWriteOnce"]
  resources:
    requests:
      storage: 1Gi
---
apiVersion: v1
kind: Pod
metadata:
  name: {PVC_POD}
  namespace: {NS}
  labels:
    app: {PVC_POD}
spec:
  containers:
  - name: mounter
    image: busybox:1.36
    command: ["sh", "-c", "while true; do sleep 30; done"]
    volumeMounts:
    - name: data
      mountPath: /data
  volumes:
  - name: data
    persistentVolumeClaim:
      claimName: {PVC}
---
apiVersion: apps/v1
kind: StatefulSet
metadata:
  name: {STS}
  namespace: {NS}
spec:
  serviceName: {HEADLESS_SVC}
  replicas: 1
  selector:
    matchLabels:
      app: {STS}
  template:
    metadata:
      labels:
        app: {STS}
    spec:
      containers:
      - name: app
        image: busybox:1.36
        command: ["sh", "-c", "while true; do sleep 30; done"]
---
apiVersion: apps/v1
kind: DaemonSet
metadata:
  name: {DS}
  namespace: {NS}
spec:
  selector:
    matchLabels:
      app: {DS}
  template:
    metadata:
      labels:
        app: {DS}
    spec:
      containers:
      - name: app
        image: busybox:1.36
        command: ["sh", "-c", "while true; do sleep 30; done"]
---
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: {NETPOL}
  namespace: {NS}
spec:
  podSelector: {{}}
  policyTypes: ["Ingress"]
---
apiVersion: networking.k8s.io/v1
kind: Ingress
metadata:
  name: {INGRESS}
  namespace: {NS}
spec:
  rules:
  - host: e2e.example.com
    http:
      paths:
      - path: /
        pathType: Prefix
        backend:
          service:
            name: {SVC}
            port:
              number: 80
---
apiVersion: v1
kind: ResourceQuota
metadata:
  name: {QUOTA}
  namespace: {NS}
spec:
  hard:
    pods: "50"
---
apiVersion: v1
kind: LimitRange
metadata:
  name: {LIMITS}
  namespace: {NS}
spec:
  limits:
  - type: Container
    defaultRequest:
      cpu: "100m"
      memory: "64Mi"
    default:
      cpu: "200m"
      memory: "128Mi"
"#
    )
}

fn widget_yaml() -> String {
    format!(
        "apiVersion: {CRD_GROUP}/v1\nkind: {CRD_KIND}\nmetadata:\n  name: {WIDGET}\n  namespace: {NS}\nspec:\n  color: blue\n"
    )
}

/// A self-contained chart whose rendered ConfigMap echoes `.Values.message`,
/// so we can prove values actually reach helm (mirrors
/// `crates/kube/tests/helm_lifecycle.rs`).
fn write_chart(dir: &std::path::Path) {
    std::fs::create_dir_all(dir.join("templates")).unwrap();
    std::fs::write(
        dir.join("Chart.yaml"),
        "apiVersion: v2\nname: e2e-cap-suite-chart\nversion: 0.1.0\n",
    )
    .unwrap();
    std::fs::write(dir.join("values.yaml"), "message: default-from-chart\n").unwrap();
    std::fs::write(
        dir.join("templates/cm.yaml"),
        "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: e2e-cap-suite-chart\ndata:\n  message: {{ .Values.message | quote }}\n",
    )
    .unwrap();
}

// ---------------------------------------------------------------------------
// The suite
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a live cluster and helm/kubectl on PATH"]
async fn full_capability_suite() {
    // Teardown must run even if an assertion panics partway through, so the
    // suite is re-runnable back-to-back with no manual cleanup. `catch_unwind`
    // over the async body + `resume_unwind` afterward preserves the original
    // panic (and its test-failure reporting) once cleanup is done.
    let result = std::panic::AssertUnwindSafe(run_suite())
        .catch_unwind()
        .await;
    teardown().await;
    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
}

async fn run_suite() {
    let ctx = context();
    let reg = build_registry_with(cache());
    let mut h = Harness::new(reg);

    // === Fixtures: dogfood k8s.applyManifest to seed the namespace =========
    println!("=== fixtures ===");
    let out = h
        .ok(
            "k8s.applyManifest",
            json!({ "context": ctx, "yaml": fixtures_yaml() }),
        )
        .await;
    assert_eq!(
        out["applied"], true,
        "fixture apply must fully succeed: {out}"
    );

    // The CRD's REST endpoint isn't live the instant the CRD object is
    // created — retry the widget instance apply until the apiserver has
    // finished establishing it.
    let dl = deadline(60);
    loop {
        match h
            .reg
            .invoke(
                "k8s.applyManifest",
                json!({ "context": ctx, "yaml": widget_yaml() }),
            )
            .await
        {
            Ok(v) if v["applied"] == true => break,
            Ok(v) if Instant::now() > dl => {
                panic!("timed out waiting for CRD {CRD_NAME} to become available: {v}")
            }
            Err(e) if Instant::now() > dl => {
                panic!("timed out waiting for CRD {CRD_NAME} to become available: {e:?}")
            }
            _ => poll_sleep().await,
        }
    }
    println!("fixtures applied: namespace, CRD, workloads, widget instance");

    // Wait for the Deployment's pods to be Running before pod-dependent
    // assertions — poll listPods with a timeout, never a blind sleep.
    let dl = deadline(180);
    loop {
        let out = h
            .reg
            .invoke("k8s.listPods", json!({ "context": ctx, "namespace": NS }))
            .await
            .unwrap();
        let running = out["pods"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| {
                p["name"]
                    .as_str()
                    .unwrap_or_default()
                    .starts_with(&format!("{DEPLOY}-"))
                    && p["phase"] == "Running"
            })
            .count();
        if running >= 2 {
            break;
        }
        if Instant::now() > dl {
            panic!("timed out waiting for {DEPLOY} pods to be Running (have {running}/2)");
        }
        poll_sleep().await;
    }
    println!("{DEPLOY}: 2 pods Running");

    // Wait for the PVC-mounting pod to be Running (this is what makes the PVC
    // bind on kind's WaitForFirstConsumer default storage class).
    let dl = deadline(180);
    loop {
        let out = h
            .reg
            .invoke("k8s.listPods", json!({ "context": ctx, "namespace": NS }))
            .await
            .unwrap();
        let running = out["pods"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["name"] == PVC_POD && p["phase"] == "Running");
        if running {
            break;
        }
        if Instant::now() > dl {
            panic!("timed out waiting for {PVC_POD} to be Running");
        }
        poll_sleep().await;
    }
    h.mark("k8s.listPods");
    println!("{PVC_POD}: Running (PVC should bind)");

    let dl = deadline(60);
    loop {
        let out = h
            .reg
            .invoke(
                "k8s.listPersistentVolumeClaims",
                json!({ "context": ctx, "namespace": NS }),
            )
            .await
            .unwrap();
        let bound = out["persistentvolumeclaims"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["name"] == PVC && p["status"] == "Bound");
        if bound {
            break;
        }
        if Instant::now() > dl {
            panic!("timed out waiting for {PVC} to bind");
        }
        poll_sleep().await;
    }
    println!("{PVC}: Bound");

    // === 1. Reads ============================================================
    println!("=== reads ===");
    let out = h.ok("ping", json!("hello")).await;
    assert_eq!(out, json!({ "pong": "hello" }));

    let out = h.ok("k8s.listContexts", json!({})).await;
    assert!(
        out["contexts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["name"] == ctx),
        "listContexts must include {ctx}"
    );

    // The kind context authenticates with a client cert (no exec-auth), so it
    // has no external tool requirements — a healthy, empty diagnosis.
    let out = h.ok("toolbox.diagnoseContext", json!({ "context": ctx })).await;
    assert_eq!(out["context"], ctx);
    assert!(
        out["items"].as_array().unwrap().is_empty(),
        "kind context should need no exec-auth tools: {out}"
    );

    let out = h.ok("k8s.clusterInfo", json!({ "context": ctx })).await;
    assert_eq!(out["reachable"], true, "cluster must be reachable: {out}");
    assert!(out["version"].as_str().is_some());

    let out = h.ok("k8s.listNamespaces", json!({ "context": ctx })).await;
    assert!(out["namespaces"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n == NS));

    let out = h
        .ok("k8s.listPods", json!({ "context": ctx, "namespace": NS }))
        .await;
    assert!(
        out["pods"].as_array().unwrap().len() >= 5,
        "expected our fixture pods: {out}"
    );

    let out = h
        .ok(
            "k8s.listDeployments",
            json!({ "context": ctx, "namespace": NS }),
        )
        .await;
    assert!(out["deployments"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["name"] == DEPLOY));

    let out = h
        .ok(
            "k8s.listStatefulSets",
            json!({ "context": ctx, "namespace": NS }),
        )
        .await;
    assert!(out["statefulsets"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["name"] == STS));

    let out = h
        .ok(
            "k8s.listDaemonSets",
            json!({ "context": ctx, "namespace": NS }),
        )
        .await;
    assert!(out["daemonsets"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["name"] == DS));

    let out = h
        .ok(
            "k8s.listReplicaSets",
            json!({ "context": ctx, "namespace": NS, "ownerName": DEPLOY }),
        )
        .await;
    assert!(
        !out["replicasets"].as_array().unwrap().is_empty(),
        "the Deployment must own a ReplicaSet: {out}"
    );

    let out = h
        .ok("k8s.listJobs", json!({ "context": ctx, "namespace": NS }))
        .await;
    assert!(out["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|j| j["name"] == JOB));

    let out = h
        .ok(
            "k8s.listCronJobs",
            json!({ "context": ctx, "namespace": NS }),
        )
        .await;
    assert!(out["cronjobs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["name"] == CRONJOB));

    let out = h
        .ok(
            "k8s.listServices",
            json!({ "context": ctx, "namespace": NS }),
        )
        .await;
    assert!(out["services"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["name"] == SVC));

    // EndpointSlices are created asynchronously by the EndpointSlice
    // controller; poll rather than assume they exist the instant the
    // Service+pods exist.
    let dl = deadline(60);
    let out = loop {
        let out = h
            .reg
            .invoke(
                "k8s.listEndpointSlices",
                json!({ "context": ctx, "namespace": NS }),
            )
            .await
            .unwrap();
        if out["endpointslices"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["service"] == SVC)
        {
            break out;
        }
        if Instant::now() > dl {
            break out;
        }
        poll_sleep().await;
    };
    h.mark("k8s.listEndpointSlices");
    assert!(
        out["endpointslices"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["service"] == SVC),
        "expected an EndpointSlice for {SVC}: {out}"
    );

    let out = h
        .ok(
            "k8s.listIngresses",
            json!({ "context": ctx, "namespace": NS }),
        )
        .await;
    assert!(out["ingresses"]
        .as_array()
        .unwrap()
        .iter()
        .any(|i| i["name"] == INGRESS));

    let out = h
        .ok(
            "k8s.listConfigMaps",
            json!({ "context": ctx, "namespace": NS }),
        )
        .await;
    assert!(out["configmaps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["name"] == CM));

    let out = h
        .ok(
            "k8s.listSecrets",
            json!({ "context": ctx, "namespace": NS }),
        )
        .await;
    assert!(out["secrets"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["name"] == SECRET));

    let out = h
        .ok(
            "k8s.listServiceAccounts",
            json!({ "context": ctx, "namespace": NS }),
        )
        .await;
    assert!(out["serviceaccounts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["name"] == SA));

    let out = h
        .ok("k8s.listRoles", json!({ "context": ctx, "namespace": NS }))
        .await;
    assert!(out["roles"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["name"] == ROLE));

    let out = h
        .ok(
            "k8s.listRoleBindings",
            json!({ "context": ctx, "namespace": NS }),
        )
        .await;
    assert!(out["rolebindings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["name"] == ROLEBINDING));

    let out = h
        .ok("k8s.listClusterRoles", json!({ "context": ctx }))
        .await;
    assert!(
        !out["clusterroles"].as_array().unwrap().is_empty(),
        "a real cluster always has built-in ClusterRoles"
    );

    let out = h
        .ok("k8s.listClusterRoleBindings", json!({ "context": ctx }))
        .await;
    assert!(
        !out["clusterrolebindings"].as_array().unwrap().is_empty(),
        "a real cluster always has built-in ClusterRoleBindings"
    );

    let out = h.ok("k8s.listNodes", json!({ "context": ctx })).await;
    assert!(!out["nodes"].as_array().unwrap().is_empty());

    let out = h
        .ok("k8s.listEvents", json!({ "context": ctx, "namespace": NS }))
        .await;
    assert!(out["events"].is_array());

    let out = h
        .ok("k8s.listPersistentVolumes", json!({ "context": ctx }))
        .await;
    assert!(out["persistentvolumes"].is_array());

    let out = h
        .ok(
            "k8s.listPersistentVolumeClaims",
            json!({ "context": ctx, "namespace": NS }),
        )
        .await;
    assert!(out["persistentvolumeclaims"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["name"] == PVC));

    let out = h
        .ok("k8s.listStorageClasses", json!({ "context": ctx }))
        .await;
    assert!(
        !out["storageclasses"].as_array().unwrap().is_empty(),
        "kind ships a default StorageClass"
    );

    let out = h
        .ok(
            "k8s.listNetworkPolicies",
            json!({ "context": ctx, "namespace": NS }),
        )
        .await;
    assert!(out["networkpolicies"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n["name"] == NETPOL));

    let out = h
        .ok(
            "k8s.listResourceQuotas",
            json!({ "context": ctx, "namespace": NS }),
        )
        .await;
    assert!(out["resourcequotas"]
        .as_array()
        .unwrap()
        .iter()
        .any(|q| q["name"] == QUOTA));

    let out = h
        .ok(
            "k8s.listLimitRanges",
            json!({ "context": ctx, "namespace": NS }),
        )
        .await;
    assert!(out["limitranges"]
        .as_array()
        .unwrap()
        .iter()
        .any(|l| l["name"] == LIMITS));

    let out = h.ok("k8s.listCRDs", json!({ "context": ctx })).await;
    assert!(out["crds"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["name"] == CRD_NAME));

    let out = h
        .ok(
            "k8s.listCustomResource",
            json!({
                "context": ctx, "group": CRD_GROUP, "version": "v1",
                "plural": CRD_PLURAL, "kind": CRD_KIND, "namespaced": true, "namespace": NS
            }),
        )
        .await;
    assert!(out["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|i| i["name"] == WIDGET));

    let out = h
        .ok(
            "k8s.listResource",
            json!({ "context": ctx, "kind": "ConfigMap", "namespace": NS }),
        )
        .await;
    assert!(out["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|i| i["name"] == CM));

    // === 2. Object / manifest ================================================
    println!("=== object/manifest ===");
    let out = h
        .ok(
            "k8s.getManifest",
            json!({ "context": ctx, "kind": "Deployment", "namespace": NS, "name": DEPLOY }),
        )
        .await;
    let yaml = out["yaml"].as_str().unwrap();
    assert!(yaml.contains("busybox:1.36") && yaml.contains(DEPLOY));

    let out = h
        .ok(
            "k8s.getObject",
            json!({ "context": ctx, "kind": "Deployment", "namespace": NS, "name": DEPLOY }),
        )
        .await;
    assert_eq!(out["object"]["metadata"]["name"], DEPLOY);

    // validateManifest: this capability never propagates a raw Result::Err
    // for a well-formed-but-invalid document — it always returns
    // `Ok({ valid, errors })`, so both the valid and invalid case go through
    // `ok()`, asserting on `valid` rather than the harness's `err()` helper.
    let valid_yaml = format!(
        "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: e2e-validate-check\n  namespace: {NS}\ndata:\n  ok: \"true\"\n"
    );
    let out = h
        .ok(
            "k8s.validateManifest",
            json!({ "context": ctx, "yaml": valid_yaml }),
        )
        .await;
    assert_eq!(
        out["valid"], true,
        "a well-formed manifest must validate: {out}"
    );
    assert!(out["errors"].as_array().unwrap().is_empty());

    let invalid_yaml = format!(
        "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: e2e-validate-bad\n  namespace: {NS}\nspec:\n  replicas: \"three\"\n  selector:\n    matchLabels:\n      app: e2e-validate-bad\n  template:\n    metadata:\n      labels:\n        app: e2e-validate-bad\n    spec:\n      containers:\n      - name: app\n        image: busybox:1.36\n"
    );
    let out = h
        .reg
        .invoke(
            "k8s.validateManifest",
            json!({ "context": ctx, "yaml": invalid_yaml }),
        )
        .await
        .expect("validateManifest itself must not Err");
    assert_eq!(
        out["valid"], false,
        "a Deployment with a string replicas count must fail strict validation: {out}"
    );
    assert!(!out["errors"].as_array().unwrap().is_empty());

    let diff_yaml = format!(
        "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: {DEPLOY}\n  namespace: {NS}\nspec:\n  replicas: 99\n  selector:\n    matchLabels:\n      app: {DEPLOY}\n  template:\n    metadata:\n      labels:\n        app: {DEPLOY}\n    spec:\n      containers:\n      - name: app\n        image: busybox:1.36\n"
    );
    let out = h
        .ok(
            "k8s.diffManifest",
            json!({ "context": ctx, "yaml": diff_yaml }),
        )
        .await;
    let doc = &out["documents"][0];
    assert_eq!(doc["exists"], true);
    assert_eq!(
        doc["changed"], true,
        "replicas 2 -> 99 must be a diff: {doc}"
    );

    let out = h
        .ok(
            "k8s.getSecret",
            json!({ "context": ctx, "namespace": NS, "name": SECRET }),
        )
        .await;
    let encoded = out["data"]["password"].as_str().expect("password key");
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .unwrap();
    assert_eq!(String::from_utf8(decoded).unwrap(), "hunter2");

    let out = h
        .ok(
            "k8s.openApiSchema",
            json!({ "context": ctx, "api_version": "apps/v1", "kind": "Deployment" }),
        )
        .await;
    assert!(out["key"]
        .as_str()
        .unwrap_or_default()
        .contains("Deployment"));
    assert!(out["schemas"]
        .as_str()
        .unwrap_or_default()
        .contains("Deployment"));

    // === 3. Relations =========================================================
    println!("=== relations ===");
    let out = h
        .ok(
            "k8s.podsForSelector",
            json!({ "context": ctx, "namespace": NS, "selector": { "app": DEPLOY } }),
        )
        .await;
    assert_eq!(
        out["pods"].as_array().unwrap().len(),
        2,
        "expected the 2 Deployment pods: {out}"
    );

    let out = h
        .ok(
            "k8s.podsForServiceAccount",
            json!({ "context": ctx, "namespace": NS, "serviceaccount": SA }),
        )
        .await;
    assert_eq!(
        out["pods"].as_array().unwrap().len(),
        2,
        "only the Deployment pods run as {SA}: {out}"
    );

    let out = h
        .ok(
            "k8s.bindingsForServiceAccount",
            json!({ "context": ctx, "namespace": NS, "serviceaccount": SA }),
        )
        .await;
    assert!(out["bindings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|b| b["name"] == ROLEBINDING && b["kind"] == "RoleBinding"));

    let out = h
        .ok(
            "k8s.podsForPvc",
            json!({ "context": ctx, "namespace": NS, "pvc": PVC }),
        )
        .await;
    assert!(out["pods"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["name"] == PVC_POD));

    // === 4. Access =============================================================
    println!("=== access ===");
    let out = h
        .ok(
            "k8s.canI",
            json!({
                "context": ctx,
                "checks": [{ "verb": "get", "resource": "pods", "namespace": NS }]
            }),
        )
        .await;
    let result = &out["results"][0];
    assert_eq!(result["error"], false);
    assert_eq!(
        result["allowed"], true,
        "the kind-admin context must be able to get pods: {result}"
    );

    // === 5. Logs ===============================================================
    println!("=== logs ===");
    let pods = h
        .reg
        .invoke("k8s.listPods", json!({ "context": ctx, "namespace": NS }))
        .await
        .unwrap();
    let pod_name = pods["pods"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| {
            p["name"]
                .as_str()
                .unwrap_or_default()
                .starts_with(&format!("{DEPLOY}-"))
        })
        .and_then(|p| p["name"].as_str())
        .expect("a running deployment pod")
        .to_string();
    let out = h
        .ok(
            "k8s.podLogs",
            json!({ "context": ctx, "namespace": NS, "pod": pod_name }),
        )
        .await;
    assert!(
        out["logs"].as_str().unwrap_or_default().contains("hello"),
        "expected the busybox loop's output: {out}"
    );

    // === 6. Metrics ============================================================
    // A bare kind cluster ships no metrics-server, so the metrics API may be
    // absent. Don't let that make the happy path vacuous: probe once, and when
    // the API IS serving, assert we actually get readings back. Install it with:
    //   helm install metrics-server metrics-server/metrics-server -n kube-system \
    //     --set 'args={--kubelet-insecure-tls}'
    println!("=== metrics ===");
    let metrics_available = h
        .try_call("k8s.nodeMetrics", json!({ "context": ctx }))
        .await
        .is_ok();

    if metrics_available {
        let nodes = h.ok("k8s.nodeMetrics", json!({ "context": ctx })).await;
        let items = nodes["metrics"]
            .as_array()
            .expect("nodeMetrics should return a metrics array");
        assert!(
            !items.is_empty(),
            "metrics API is serving but nodeMetrics returned nothing"
        );
        assert!(
            items
                .iter()
                .any(|n| n["cpuMillicores"].as_i64().unwrap_or(0) > 0
                    || n["memoryMib"].as_i64().unwrap_or(0) > 0),
            "nodeMetrics returned nodes with no readings: {items:?}"
        );
        println!("  k8s.nodeMetrics: {} node(s) with readings", items.len());

        let pods = h
            .ok("k8s.podMetrics", json!({ "context": ctx, "namespace": NS }))
            .await;
        let items = pods["metrics"]
            .as_array()
            .expect("podMetrics should return a metrics array");
        println!("  k8s.podMetrics: {} pod(s) with readings", items.len());
    } else {
        // No metrics-server: the capabilities must degrade cleanly, not hang or panic.
        h.any("k8s.nodeMetrics", json!({ "context": ctx })).await;
        h.any("k8s.podMetrics", json!({ "context": ctx, "namespace": NS }))
            .await;
        println!("  metrics API absent — asserted clean degradation only");
    }

    // === 7. Writes ==============================================================
    println!("=== writes ===");
    let out = h
        .ok(
            "k8s.updateConfigData",
            json!({
                "context": ctx, "kind": "ConfigMap", "namespace": NS, "name": CM,
                "data": { "greeting": "updated-by-e2e" }
            }),
        )
        .await;
    assert_eq!(out["ok"], true);
    let confirm = h
        .reg
        .invoke(
            "k8s.getObject",
            json!({ "context": ctx, "kind": "ConfigMap", "namespace": NS, "name": CM }),
        )
        .await
        .unwrap();
    assert_eq!(confirm["object"]["data"]["greeting"], "updated-by-e2e");

    let out = h
        .ok(
            "k8s.scale",
            json!({ "context": ctx, "kind": "Deployment", "namespace": NS, "name": DEPLOY, "replicas": 3 }),
        )
        .await;
    assert_eq!(out["ok"], true);
    let dl = deadline(120);
    loop {
        let out = h
            .reg
            .invoke(
                "k8s.listDeployments",
                json!({ "context": ctx, "namespace": NS }),
            )
            .await
            .unwrap();
        let available = out["deployments"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["name"] == DEPLOY)
            .and_then(|d| d["available"].as_i64())
            .unwrap_or(0);
        if available >= 3 {
            break;
        }
        if Instant::now() > dl {
            panic!("timed out waiting for {DEPLOY} to scale to 3 (available={available})");
        }
        poll_sleep().await;
    }
    println!("{DEPLOY}: scaled to 3");

    let out = h
        .ok(
            "k8s.rolloutRestart",
            json!({ "context": ctx, "kind": "Deployment", "namespace": NS, "name": DEPLOY }),
        )
        .await;
    assert_eq!(out["ok"], true);

    let out = h
        .ok(
            "k8s.cronjobSetSuspend",
            json!({ "context": ctx, "namespace": NS, "name": CRONJOB, "suspend": true }),
        )
        .await;
    assert_eq!(out["ok"], true);
    let confirm = h
        .reg
        .invoke(
            "k8s.listCronJobs",
            json!({ "context": ctx, "namespace": NS }),
        )
        .await
        .unwrap();
    assert!(confirm["cronjobs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["name"] == CRONJOB && c["suspended"] == true));

    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .to_string();
    let out = h
        .ok(
            "k8s.cronjobTriggerNow",
            json!({ "context": ctx, "namespace": NS, "name": CRONJOB, "suffix": suffix }),
        )
        .await;
    assert_eq!(out["ok"], true);
    let triggered_job = out["jobName"].as_str().unwrap().to_string();
    let dl = deadline(60);
    loop {
        let out = h
            .reg
            .invoke("k8s.listJobs", json!({ "context": ctx, "namespace": NS }))
            .await
            .unwrap();
        if out["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|j| j["name"] == triggered_job)
        {
            break;
        }
        if Instant::now() > dl {
            panic!("timed out waiting for triggered job {triggered_job} to appear");
        }
        poll_sleep().await;
    }
    println!("cronjobTriggerNow: {triggered_job} appeared");

    // applyManifest's UPDATE path (fixtures already exercised create).
    let update_ns_yaml = format!(
        "apiVersion: v1\nkind: Namespace\nmetadata:\n  name: {NS}\n  labels:\n    e2e-marker: updated\n"
    );
    let out = h
        .ok(
            "k8s.applyManifest",
            json!({ "context": ctx, "yaml": update_ns_yaml }),
        )
        .await;
    assert_eq!(out["applied"], true);
    let confirm = h
        .reg
        .invoke(
            "k8s.getObject",
            json!({ "context": ctx, "kind": "Namespace", "name": NS }),
        )
        .await
        .unwrap();
    assert_eq!(
        confirm["object"]["metadata"]["labels"]["e2e-marker"],
        "updated"
    );

    // deletePod: delete one Deployment pod, confirm it's gone and the
    // Deployment self-heals back to 3 Running pods.
    let pods = h
        .reg
        .invoke("k8s.listPods", json!({ "context": ctx, "namespace": NS }))
        .await
        .unwrap();
    let victim = pods["pods"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| {
            p["name"]
                .as_str()
                .unwrap_or_default()
                .starts_with(&format!("{DEPLOY}-"))
        })
        .and_then(|p| p["name"].as_str())
        .unwrap()
        .to_string();
    let out = h
        .ok(
            "k8s.deletePod",
            json!({ "context": ctx, "namespace": NS, "pod": victim }),
        )
        .await;
    assert_eq!(out["deleted"], true);
    let dl = deadline(120);
    loop {
        let out = h
            .reg
            .invoke("k8s.listPods", json!({ "context": ctx, "namespace": NS }))
            .await
            .unwrap();
        let pods = out["pods"].as_array().unwrap();
        let gone = !pods.iter().any(|p| p["name"] == victim);
        let running = pods
            .iter()
            .filter(|p| {
                p["name"]
                    .as_str()
                    .unwrap_or_default()
                    .starts_with(&format!("{DEPLOY}-"))
                    && p["phase"] == "Running"
            })
            .count();
        if gone && running >= 3 {
            break;
        }
        if Instant::now() > dl {
            panic!(
                "timed out waiting for {victim} to be replaced (gone={gone}, running={running}/3)"
            );
        }
        poll_sleep().await;
    }
    println!("deletePod: {victim} replaced");

    // evictPod: evict a different (still-live) Deployment pod.
    let pods = h
        .reg
        .invoke("k8s.listPods", json!({ "context": ctx, "namespace": NS }))
        .await
        .unwrap();
    let evictee = pods["pods"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| {
            p["name"]
                .as_str()
                .unwrap_or_default()
                .starts_with(&format!("{DEPLOY}-"))
                && p["phase"] == "Running"
        })
        .and_then(|p| p["name"].as_str())
        .unwrap()
        .to_string();
    let out = h
        .ok(
            "k8s.evictPod",
            json!({ "context": ctx, "namespace": NS, "pod": evictee }),
        )
        .await;
    assert_eq!(out["ok"], true);

    // deleteResource: a negative path (unsupported kind) via `err()`, then
    // the real positive deletion of the NetworkPolicy fixture.
    let msg = h
        .err(
            "k8s.deleteResource",
            json!({ "context": ctx, "kind": "Bogus", "namespace": NS, "name": "whatever" }),
        )
        .await;
    assert!(msg.contains("unsupported kind"), "unexpected error: {msg}");

    let out = h
        .ok(
            "k8s.deleteResource",
            json!({ "context": ctx, "kind": "NetworkPolicy", "namespace": NS, "name": NETPOL }),
        )
        .await;
    assert_eq!(out["ok"], true);
    let confirm = h
        .reg
        .invoke(
            "k8s.listNetworkPolicies",
            json!({ "context": ctx, "namespace": NS }),
        )
        .await
        .unwrap();
    assert!(!confirm["networkpolicies"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n["name"] == NETPOL));

    // === 8. Helm ================================================================
    println!("=== helm ===");
    let out = h.ok("k8s.helmVersion", json!({ "context": ctx })).await;
    assert!(!out["version"].as_str().unwrap_or_default().is_empty());

    // Network-dependent: a real repo add/update/search. Best-effort — a
    // sandboxed/offline test run must not become flaky over this.
    h.any(
        "k8s.helmRepoAdd",
        json!({ "context": ctx, "name": "bitnami", "url": "https://charts.bitnami.com/bitnami" }),
    )
    .await;
    h.any("k8s.helmRepoUpdate", json!({ "context": ctx })).await;
    h.any(
        "k8s.helmSearchRepo",
        json!({ "context": ctx, "chart": "nginx" }),
    )
    .await;

    let chart_dir =
        std::env::temp_dir().join(format!("srelens-e2e-suite-chart-{}", std::process::id()));
    write_chart(&chart_dir);
    let chart = chart_dir.to_string_lossy().to_string();

    let out = h
        .ok(
            "k8s.helmTemplate",
            json!({
                "context": ctx, "name": HELM_RELEASE, "chart": chart,
                "namespace": NS, "values": "message: from-template\n"
            }),
        )
        .await;
    assert!(out["output"]
        .as_str()
        .unwrap_or_default()
        .contains("from-template"));

    h.ok(
        "k8s.helmInstall",
        json!({
            "context": ctx, "name": HELM_RELEASE, "chart": chart,
            "namespace": NS, "values": "message: hello-from-install\n"
        }),
    )
    .await;

    let out = h
        .ok(
            "k8s.getHelmRelease",
            json!({ "context": ctx, "namespace": NS, "name": HELM_RELEASE }),
        )
        .await;
    assert_eq!(out["revision"], 1);
    assert!(out["valuesYaml"]
        .as_str()
        .unwrap_or_default()
        .contains("hello-from-install"));

    let out = h
        .ok(
            "k8s.listHelmReleases",
            json!({ "context": ctx, "namespace": NS }),
        )
        .await;
    assert!(out["releases"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["name"] == HELM_RELEASE));

    h.ok(
        "k8s.helmUpgrade",
        json!({
            "context": ctx, "name": HELM_RELEASE, "chart": chart,
            "namespace": NS, "values": "message: upgraded-value\n"
        }),
    )
    .await;
    let out = h
        .reg
        .invoke(
            "k8s.getHelmRelease",
            json!({ "context": ctx, "namespace": NS, "name": HELM_RELEASE }),
        )
        .await
        .unwrap();
    assert_eq!(out["revision"], 2);
    assert!(out["valuesYaml"]
        .as_str()
        .unwrap_or_default()
        .contains("upgraded-value"));

    h.ok(
        "k8s.helmRollback",
        json!({ "context": ctx, "name": HELM_RELEASE, "namespace": NS, "revision": 1 }),
    )
    .await;
    let out = h
        .reg
        .invoke(
            "k8s.getHelmRelease",
            json!({ "context": ctx, "namespace": NS, "name": HELM_RELEASE }),
        )
        .await
        .unwrap();
    assert_eq!(out["revision"], 3, "rollback creates a new revision");
    assert!(out["valuesYaml"]
        .as_str()
        .unwrap_or_default()
        .contains("hello-from-install"));

    h.ok(
        "k8s.helmUninstall",
        json!({ "context": ctx, "name": HELM_RELEASE, "namespace": NS }),
    )
    .await;
    let out = h
        .reg
        .invoke(
            "k8s.listHelmReleases",
            json!({ "context": ctx, "namespace": NS }),
        )
        .await
        .unwrap();
    assert!(!out["releases"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["name"] == HELM_RELEASE));

    let _ = std::fs::remove_dir_all(&chart_dir);
    println!("helm lifecycle complete");

    // === 9. Node ops — LAST, they disrupt the cluster ==========================
    println!("=== node ops ===");
    let nodes = h
        .reg
        .invoke("k8s.listNodes", json!({ "context": ctx }))
        .await
        .unwrap();
    let node_name = nodes["nodes"].as_array().unwrap()[0]["name"]
        .as_str()
        .unwrap()
        .to_string();

    let out = h
        .ok(
            "k8s.cordonNode",
            json!({ "context": ctx, "name": node_name, "unschedulable": true }),
        )
        .await;
    assert_eq!(out["ok"], true);
    let confirm = h
        .reg
        .invoke("k8s.listNodes", json!({ "context": ctx }))
        .await
        .unwrap();
    assert!(confirm["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n["name"] == node_name && n["unschedulable"] == true));
    println!("{node_name}: cordoned");

    let out = h
        .ok(
            "k8s.drainNode",
            json!({ "context": ctx, "name": node_name }),
        )
        .await;
    println!(
        "{node_name}: drained (evicted={}, skipped={})",
        out["evicted"], out["skipped"]
    );

    let out = h
        .ok(
            "k8s.cordonNode",
            json!({ "context": ctx, "name": node_name, "unschedulable": false }),
        )
        .await;
    assert_eq!(out["ok"], true);
    let confirm = h
        .reg
        .invoke("k8s.listNodes", json!({ "context": ctx }))
        .await
        .unwrap();
    assert!(confirm["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n["name"] == node_name && n["unschedulable"] == false));
    println!("{node_name}: uncordoned");

    // === 10. deleteContext — DANGEROUS: only ever on a throwaway copy =========
    println!("=== deleteContext (throwaway kubeconfig copy) ===");
    delete_context_on_a_copy(&mut h, &ctx).await;

    // === Coverage completeness ==================================================
    println!("=== coverage ===");
    if EXCLUDED.is_empty() {
        println!("no excluded capabilities — full coverage required");
    } else {
        for (id, reason) in EXCLUDED {
            println!("excluded: {id} — {reason}");
        }
    }
    let excluded_ids: HashSet<&str> = EXCLUDED.iter().map(|(id, _)| *id).collect();
    let mut ids = h.reg.ids();
    ids.sort();
    let missing: Vec<&str> = ids
        .iter()
        .filter(|id| !h.covered.contains(**id) && !excluded_ids.contains(*id))
        .copied()
        .collect();
    println!("covered {}/{} capabilities", h.covered.len(), ids.len());
    assert!(
        missing.is_empty(),
        "capabilities registered but never exercised by this e2e suite (add a case or an EXCLUDED reason): {missing:?}"
    );
}

/// `k8s.deleteContext` REMOVES A CONTEXT FROM THE KUBECONFIG ON DISK. Never
/// run it against the real kubeconfig: copy the file that declares `ctx` to a
/// private temp file, build a SEPARATE `ClientCache`/registry pointing only at
/// that copy, delete the context from the COPY, and assert the real
/// kubeconfig is untouched.
async fn delete_context_on_a_copy(h: &mut Harness, ctx: &str) {
    let source = kubeconfig_paths()
        .into_iter()
        .find(|p| {
            std::fs::read_to_string(p)
                .map(|c| c.contains(ctx))
                .unwrap_or(false)
        })
        .expect("no kubeconfig file declares the e2e context");

    let tmp = std::env::temp_dir().join(format!(
        "srelens-e2e-kubeconfig-copy-{}-{}.yaml",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::copy(&source, &tmp).expect("copy kubeconfig for the deleteContext test");

    let copy_cache = ClientCache::new(tmp.clone());
    let copy_reg = build_registry_with(copy_cache);

    let out = copy_reg
        .invoke("k8s.deleteContext", json!({ "context": ctx }))
        .await
        .unwrap_or_else(|e| panic!("k8s.deleteContext failed on the throwaway copy: {e:?}"));
    assert_eq!(out["success"], true);
    h.mark("k8s.deleteContext");

    let copy_contents = std::fs::read_to_string(&tmp).unwrap();
    assert!(
        !copy_contents.contains(&format!("name: {ctx}")),
        "the context must be removed from the COPY"
    );

    let real_contents = std::fs::read_to_string(&source).unwrap();
    assert!(
        real_contents.contains(ctx),
        "the REAL kubeconfig must be untouched by deleting a context from the copy"
    );

    let _ = std::fs::remove_file(&tmp);
    println!("deleteContext: removed from the throwaway copy only; real kubeconfig untouched");
}

/// Delete the `srelens-e2e` namespace and its CRD, and make sure the node(s)
/// end up uncordoned — runs unconditionally (even after a panic) so the suite
/// is re-runnable back-to-back with no manual cleanup.
async fn teardown() {
    let ctx = context();
    println!("\n=== teardown ===");

    let ns_out = tokio::process::Command::new("kubectl")
        .args([
            "--context",
            &ctx,
            "delete",
            "namespace",
            NS,
            "--ignore-not-found",
            "--wait=true",
            "--timeout=120s",
        ])
        .output()
        .await;
    match ns_out {
        Ok(o) if o.status.success() => println!("teardown: namespace {NS} deleted"),
        Ok(o) => println!(
            "teardown: namespace delete non-zero exit: {}",
            String::from_utf8_lossy(&o.stderr)
        ),
        Err(e) => println!("teardown: failed to run kubectl to delete namespace: {e}"),
    }

    let crd_out = tokio::process::Command::new("kubectl")
        .args([
            "--context",
            &ctx,
            "delete",
            "crd",
            CRD_NAME,
            "--ignore-not-found",
            "--wait=true",
            "--timeout=60s",
        ])
        .output()
        .await;
    match crd_out {
        Ok(o) if o.status.success() => println!("teardown: CRD {CRD_NAME} deleted"),
        Ok(o) => println!(
            "teardown: CRD delete non-zero exit: {}",
            String::from_utf8_lossy(&o.stderr)
        ),
        Err(e) => println!("teardown: failed to run kubectl to delete CRD: {e}"),
    }

    // Belt-and-suspenders: make sure every node ends up uncordoned even if
    // the node-ops section panicked before reaching its own uncordon call.
    if let Ok(out) = tokio::process::Command::new("kubectl")
        .args([
            "--context",
            &ctx,
            "get",
            "nodes",
            "-o",
            "jsonpath={.items[*].metadata.name}",
        ])
        .output()
        .await
    {
        if out.status.success() {
            let names = String::from_utf8_lossy(&out.stdout);
            for name in names.split_whitespace() {
                let _ = tokio::process::Command::new("kubectl")
                    .args(["--context", &ctx, "uncordon", name])
                    .output()
                    .await;
            }
            println!("teardown: node(s) uncordoned");
        }
    }
}
