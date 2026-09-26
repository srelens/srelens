//! Full capability-registry e2e suite against a real cluster (kind), issue #27.
//!
//! Drives the app's REAL capability registry — the exact same
//! `(cap.handler)(json_value).await` path the Tauri bridge and the MCP server
//! use — against a live kind cluster, and ENFORCES COVERAGE: every capability
//! registered in the desktop capability registry must
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
//! namespace, one CRD of its own, and the minimal Flux and Argo CD CRDs in
//! `tests/fixtures/gitops-crds.yaml`, which it refuses to apply over a
//! cluster's real ones; it never touches the real `~/.kube/config` (the
//! `k8s.deleteContext` case operates on a throwaway copy). It cordons and
//! drains the (single) node near the end and always uncordons it again, even
//! on panic, so the cluster is left usable and the suite is re-runnable
//! back-to-back with no manual cleanup.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use futures::FutureExt;
use serde_json::{json, Value};
use srelens_capability::Registry;
use srelens_desktop_lib::capabilities::{
    build_registry_with, build_registry_with_paths_and_settings,
};
use srelens_kube::client_cache::ClientCache;

const NS: &str = "srelens-e2e";
const DEPLOY: &str = "e2e-web";
/// The one fixture that actually listens on a port: busybox's httpd serving a
/// two-line /metrics, for `k8s.queryPodEndpoint`. Separate from {DEPLOY} so
/// the log and relation cases keep their exact pod counts and output.
const HTTP_DEPLOY: &str = "e2e-http";
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

// Extension apps and host GitOps actions (#536). The CRDs are a pinned fixture the
// suite applies itself; the manifests are the examples app authors start from.
const GITOPS_CRDS: &str = include_str!("fixtures/gitops-crds.yaml");
/// Every fixture CRD carries this label. Teardown deletes by it, never by name.
const GITOPS_CRD_SELECTOR: &str = "srelens-e2e-fixture=gitops";
const GITOPS_CRD_NAMES: [&str; 2] = [
    "kustomizations.kustomize.toolkit.fluxcd.io",
    "applications.argoproj.io",
];
const KUSTOMIZATION: &str = "e2e-kustomization";
const ARGO_APP: &str = "e2e-application";
const FLUX_EXAMPLE: &str = include_str!("../../../../examples/extensions/flux.json");
const ARGOCD_EXAMPLE: &str = include_str!("../../../../examples/extensions/argocd.json");
/// The published Argo CD release bytes and their publisher signature. The signature
/// covers these exact bytes, which `.gitattributes` keeps from line-ending conversion.
const SIGNED_ARGOCD: &str =
    include_str!("../../../../crates/registry/tests/fixtures/argocd-manifest.json");
const SIGNED_ARGOCD_SIG: &[u8] =
    include_bytes!("../../../../crates/registry/tests/fixtures/argocd-manifest.sig");
/// A catalog that lists that release by its checksum.
const CATALOG: &str =
    include_str!("../../../../crates/registry/tests/fixtures/extension-catalog.json");

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

/// Keeps the settings and extension capabilities isolated from a developer's
/// real app data, including when the suite unwinds after a failed assertion.
/// The host keeps the extension inventory, its catalog cache and their lock
/// files beside the settings file, so the whole directory is the suite's own.
struct TempSettings(PathBuf);

impl TempSettings {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("srelens-e2e-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create the e2e settings directory");
        Self(dir.join("settings.json"))
    }

    /// Where the host caches the extension catalog for this settings file:
    /// `crates/registry` puts the inventory at `<settings>.extensions.json` and
    /// the cache beside it as `<settings>.extensions.catalog.json`.
    fn catalog_cache(&self) -> PathBuf {
        self.0.with_extension("extensions.catalog.json")
    }
}

impl Drop for TempSettings {
    fn drop(&mut self) {
        if let Some(dir) = self.0.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
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

    /// Controllers and node heartbeats can invalidate a review between the GET
    /// and PATCH. Only this live-test helper re-reads and reviews on an explicit
    /// stale-review refusal or API conflict. Production still refuses the write.
    /// Return the accepted payload so the old review can be checked afterwards.
    async fn reviewed_request(
        &mut self,
        id: &str,
        mut input: Value,
    ) -> Result<(Value, Value), srelens_capability::CapabilityError> {
        for attempt in 0..8 {
            let current = self
                .reg
                .invoke(
                    "k8s.getObject",
                    json!({
                        "context":input["context"],"kind":input["kind"],
                        "namespace":input["namespace"],"name":input["name"]
                    }),
                )
                .await?;
            input["uid"] = current["object"]["metadata"]["uid"].clone();
            input["resourceVersion"] = current["object"]["metadata"]["resourceVersion"].clone();
            match self.reg.invoke(id, input.clone()).await {
                Ok(out) => {
                    self.mark(id);
                    return Ok((out, input));
                }
                Err(error) => {
                    let review_race = matches!(&error, srelens_capability::CapabilityError::Handler(message)
                        if message == "Resource changed or was replaced; refresh and review the action again"
                            || (message.starts_with("ApiError:") && message.contains(": Conflict (Status {")
                                && message.contains("code: 409,")));
                    if !review_race || attempt == 7 {
                        return Err(error);
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
        unreachable!("the final attempt returns its result")
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
    ("k8s.nodeJournalLogs", "requires SSH access to the node host; exercised via unit tests with mocked sessions"),
    ("k8s.nodeRuntimeDiagnostics", "requires host-level runtime CLI (crictl/containerd) via SSH; exercised via unit tests"),
    ("k8s.nodeServiceRestart", "requires node host systemd access via SSH; exercised via unit tests"),
    ("k8s.nodeServiceStatus", "requires node host systemctl access via SSH; exercised via unit tests"),
    (
        "toolbox.installKubectl",
        "downloads a real ~50MB binary from dl.k8s.io; kubectl is already provided by the \
         CI image, so this is covered by unit tests with an injected fetch rather than the network",
    ),
    (
        "toolbox.installHelm",
        "downloads a real release tarball from get.helm.sh; helm is already provided by the CI \
         image, so this is covered by unit tests with an injected fetch rather than the network",
    ),
    (
        "toolbox.upgradePlugin",
        "an upgrade is a no-op on a freshly-installed plugin (already latest); the install/remove \
         cases below exercise the same kubectl-krew subprocess path",
    ),
];
// installKrew, searchPlugins, installPlugin and removePlugin are exercised for
// real in `toolbox_krew_lifecycle` below — the spec's "real krew bootstrap +
// small-plugin install" integration test.

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
apiVersion: apps/v1
kind: Deployment
metadata:
  name: {HTTP_DEPLOY}
  namespace: {NS}
  labels:
    app: {HTTP_DEPLOY}
spec:
  replicas: 1
  selector:
    matchLabels:
      app: {HTTP_DEPLOY}
  template:
    metadata:
      labels:
        app: {HTTP_DEPLOY}
    spec:
      containers:
      - name: app
        image: busybox:1.36
        command: ["sh", "-c", "mkdir -p /www && echo ok > /www/index.html && {{ echo '# HELP e2e_up 1 when the fixture serves'; echo 'e2e_up 1'; }} > /www/metrics && exec httpd -f -p 8080 -h /www"]
        ports:
        - name: http
          containerPort: 8080
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

/// One object of each fixture GitOps kind. Nothing reconciles them; the suite
/// only reads them and requests actions on them.
fn gitops_resources_yaml() -> String {
    format!(
        r#"apiVersion: kustomize.toolkit.fluxcd.io/v1
kind: Kustomization
metadata:
  name: {KUSTOMIZATION}
  namespace: {NS}
spec:
  interval: 10m
  path: ./deploy
  prune: true
  sourceRef:
    kind: GitRepository
    name: e2e-source
---
apiVersion: argoproj.io/v1alpha1
kind: Application
metadata:
  name: {ARGO_APP}
  namespace: {NS}
spec:
  project: default
  source:
    repoURL: https://example.com/e2e.git
    path: deploy
    targetRevision: HEAD
  destination:
    server: https://kubernetes.default.svc
    namespace: {NS}
"#
    )
}

/// Applies custom resources whose CRDs were just created. A new CRD's REST
/// endpoint is not served the instant the CRD exists, so retry until the
/// apiserver has established it.
async fn apply_once_served(h: &Harness, ctx: &str, yaml: &str, what: &str) {
    let dl = deadline(60);
    loop {
        match h
            .reg
            .invoke("k8s.applyManifest", json!({ "context": ctx, "yaml": yaml }))
            .await
        {
            Ok(v) if v["applied"] == true => break,
            Ok(v) if Instant::now() > dl => {
                panic!("timed out waiting for {what} to become available: {v}")
            }
            Err(e) if Instant::now() > dl => {
                panic!("timed out waiting for {what} to become available: {e:?}")
            }
            _ => poll_sleep().await,
        }
    }
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
    let settings = TempSettings::new();
    let reg = build_registry_with_paths_and_settings(
        cache(),
        kubeconfig_paths(),
        Some(settings.0.clone()),
    );
    let mut h = Harness::new(reg);

    // === Durable settings: real file round-trip, isolated from app data ===
    println!("=== settings ===");
    let saved = h
        .ok(
            "settings.set",
            json!({
                "values": { "e2e.roundTrip": { "theme": "dark", "scale": 120 } },
                "localStorageMigrated": true
            }),
        )
        .await;
    assert_eq!(saved["saved"], true);
    let loaded = h
        .ok("settings.get", json!({ "key": "e2e.roundTrip" }))
        .await;
    assert_eq!(
        loaded["values"]["e2e.roundTrip"],
        json!({ "theme": "dark", "scale": 120 })
    );
    assert_eq!(loaded["localStorageMigrated"], true);
    h.ok(
        "settings.set",
        json!({ "remove": ["e2e.roundTrip"] }),
    )
    .await;

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

    apply_once_served(&h, &ctx, &widget_yaml(), &format!("CRD {CRD_NAME}")).await;
    println!("fixtures applied: namespace, CRD, workloads, widget instance");
    apply_gitops_fixtures(&mut h, &ctx).await;

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

    // Tool inventory: kubectl and helm are on PATH in this suite (per the
    // module docstring), so status must find kubectl installed with a version.
    let out = h.ok("toolbox.status", json!({})).await;
    let tools = out["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 3, "kubectl/krew/helm: {out}");
    let kubectl = tools.iter().find(|t| t["name"] == "kubectl").unwrap();
    assert_eq!(kubectl["installed"], true, "kubectl must be on PATH here: {out}");
    assert!(kubectl["version"].as_str().is_some(), "kubectl version should resolve: {out}");

    toolbox_krew_lifecycle(&mut h).await;

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

    // === k8s.podCount / k8s.podOverview (#339) ===============================
    // Both count/group pods cluster-wide WITHOUT listing pod bodies (see
    // crates/kube/src/pod_count.rs and pod_overview.rs). Cross-checked here
    // against a real cluster-wide k8s.listPods (namespace "") rather than
    // trusting the counts on their own.
    println!("=== pod count / overview ===");
    let all_pods_out = h
        .reg
        .invoke("k8s.listPods", json!({ "context": ctx, "namespace": "" }))
        .await
        .unwrap();
    h.mark("k8s.listPods");
    let all_pods = all_pods_out["pods"].as_array().unwrap();

    // podCount's own definition of its denominator (every phase except
    // Succeeded) and numerator (Running only) — matched here, not
    // re-derived, so the test fails if either capability's counting ever
    // drifts from what a plain list of the same pods shows.
    let plain_still_running = all_pods.iter().filter(|p| p["phase"] != "Succeeded").count() as i64;
    let plain_running = all_pods.iter().filter(|p| p["phase"] == "Running").count() as i64;

    let pod_count_out = h.ok("k8s.podCount", json!({ "context": ctx })).await;
    assert_eq!(
        pod_count_out["total"].as_i64().unwrap(),
        plain_still_running,
        "podCount total must match a plain cluster-wide pod list, minus Succeeded pods: {pod_count_out}"
    );
    assert_eq!(
        pod_count_out["running"].as_i64().unwrap(),
        plain_running,
        "podCount running must match the Running pods in a plain cluster-wide list: {pod_count_out}"
    );
    assert!(
        pod_count_out["total"].as_i64().unwrap() >= 5,
        "expected at least our fixture pods counted: {pod_count_out}"
    );

    // podOverview's own "short of ready" rule (mirrors the READY-column check
    // in crates/kube/src/pod_overview.rs), applied to the same ready count
    // k8s.listPods reports, so "unsettled" can be checked against the plain
    // list rather than trusted blind.
    fn ready_cell_is_short(ready: &str) -> bool {
        let Some((r, t)) = ready.split_once('/') else {
            return true;
        };
        match (r.trim().parse::<i64>(), t.trim().parse::<i64>()) {
            (Ok(r), Ok(t)) => r < t,
            _ => true,
        }
    }

    let pod_overview_out = h.ok("k8s.podOverview", json!({ "context": ctx })).await;
    assert_eq!(
        pod_overview_out["total"].as_i64().unwrap(),
        all_pods.len() as i64,
        "podOverview total must match a plain cluster-wide pod list: {pod_overview_out}"
    );
    assert!(
        !pod_overview_out["truncated"].as_bool().unwrap(),
        "the e2e namespace is far below the 200-pod unsettled cap: {pod_overview_out}"
    );

    // Every pod podOverview counted must be accounted for in exactly one
    // node's group (or none, if unscheduled) — the property the module
    // exists to answer without ever listing a pod body to do it.
    let mut expected_by_node: BTreeMap<String, i64> = BTreeMap::new();
    for p in all_pods {
        let node = p["node"].as_str().unwrap_or_default();
        if !node.is_empty() {
            *expected_by_node.entry(node.to_string()).or_insert(0) += 1;
        }
    }
    let actual_by_node: BTreeMap<String, i64> = pod_overview_out["byNode"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| (n["node"].as_str().unwrap().to_string(), n["pods"].as_i64().unwrap()))
        .collect();
    assert_eq!(
        actual_by_node, expected_by_node,
        "podOverview's per-node groups must account for every scheduled pod in a plain list: {pod_overview_out}"
    );

    let expected_unsettled: HashSet<(String, String)> = all_pods
        .iter()
        .filter(|p| p["phase"] != "Running" || ready_cell_is_short(p["ready"].as_str().unwrap_or_default()))
        .map(|p| {
            (
                p["namespace"].as_str().unwrap_or_default().to_string(),
                p["name"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect();
    let actual_unsettled: HashSet<(String, String)> = pod_overview_out["unsettled"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p["namespace"].as_str().unwrap_or_default().to_string(),
                p["name"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect();
    assert_eq!(
        actual_unsettled, expected_unsettled,
        "podOverview's unsettled set must match pods that are not simply Running in a plain list: {pod_overview_out}"
    );

    // #17: attach an ephemeral debug container to a fixture pod. It can't be
    // removed once added, but the whole namespace is torn down after the suite.
    let debug_pod = out["pods"].as_array().unwrap()[0]["name"].as_str().unwrap().to_string();
    let dbg = h
        .ok(
            "k8s.debugPod",
            json!({ "context": ctx, "namespace": NS, "pod": debug_pod, "image": "busybox" }),
        )
        .await;
    assert!(
        dbg["container"].as_str().unwrap().starts_with("debugger-"),
        "debug container name: {dbg}",
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
        .ok(
            "k8s.listChanges",
            json!({ "context": ctx, "namespace": NS, "since": "1h" }),
        )
        .await;
    assert!(out["deployments"].is_array());

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

    // #17: create a privileged node debug pod, then tear it down immediately.
    let node_name = out["nodes"].as_array().unwrap()[0]["name"].as_str().unwrap().to_string();
    let nd = h
        .ok(
            "k8s.createNodeDebugPod",
            json!({ "context": ctx, "node": node_name, "namespace": NS, "image": "busybox" }),
        )
        .await;
    assert_eq!(nd["namespace"], NS);
    let node_debug_pod = nd["pod"].as_str().unwrap().to_string();
    assert!(node_debug_pod.starts_with("srelens-node-debug-"), "generated name: {nd}");
    h.ok(
        "k8s.deletePod",
        json!({ "context": ctx, "namespace": NS, "pod": node_debug_pod }),
    )
    .await;

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

    extensions_and_gitops(&mut h, &ctx, &settings).await;

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
            // `apiVersion`, the spelling `core`'s wrapper sends. This case
            // used to send `api_version` — the struct's own field name — and
            // so passed while every real call failed to deserialize.
            json!({ "context": ctx, "apiVersion": "apps/v1", "kind": "Deployment" }),
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
    let deployment_node = out["pods"][0]["node"]
        .as_str()
        .filter(|node| !node.is_empty())
        .expect("a scheduled Deployment pod must name its node")
        .to_string();

    let on_node = h
        .ok(
            "k8s.podsOnNode",
            json!({ "context": ctx, "node": deployment_node }),
        )
        .await;
    let scheduled = on_node["pods"].as_array().expect("podsOnNode pods array");
    assert!(
        scheduled
            .iter()
            .all(|pod| { pod["node"].as_str() == Some(deployment_node.as_str()) }),
        "podsOnNode returned a pod from another node: {on_node}"
    );
    assert_eq!(
        scheduled
            .iter()
            .filter(|pod| {
                pod["namespace"] == NS
                    && pod["name"]
                        .as_str()
                        .is_some_and(|name| name.starts_with(DEPLOY))
            })
            .count(),
        2,
        "both Deployment pods should be found on their node: {on_node}"
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

    // --- topologyGraph: the three joins, against objects the fixtures made ---
    // The unit tests in `crates/kube/src/topology.rs` prove the join rules on
    // hand-built objects. What only a cluster can prove is that the fields
    // those rules read are the fields a real API server fills in — the
    // Service selector, the Deployment template labels the controller copies
    // onto its pods, and the ownerReference the Deployment controller writes
    // on the ReplicaSet it makes.
    // The full input, spelled out: several namespaces at once is the
    // capability's shape, and the two optional sources are named as absent
    // rather than left to a default that may not exist.
    let out = h
        .ok(
            "k8s.topologyGraph",
            json!({ "context": ctx, "namespaces": [NS], "prometheus": [] }),
        )
        .await;
    let nodes = out["nodes"].as_array().unwrap();
    let edges = out["edges"].as_array().unwrap();
    let node_id = |kind: &str, name: &str| format!("{kind}/{NS}/{name}");
    let deploy_id = node_id("Deployment", DEPLOY);
    let svc_id = node_id("Service", SVC);

    assert!(
        nodes.iter().any(|n| n["id"] == json!(deploy_id)),
        "the fixture Deployment must be a node: {out}"
    );
    assert!(
        nodes.iter().any(|n| n["id"] == json!(svc_id)),
        "the fixture Service must be a node: {out}"
    );
    // Service -> workload, which is the selector subset test against labels a
    // real controller wrote.
    assert!(
        edges
            .iter()
            .any(|e| e["from"] == json!(svc_id) && e["to"] == json!(deploy_id) && e["kind"] == json!("routes")),
        "the Service must route to the Deployment it selects: {out}"
    );
    // Deployment -> ReplicaSet, from the ownerReference. The ReplicaSet's name
    // is generated, so this asserts the SHAPE of the edge rather than an id
    // the test cannot know.
    assert!(
        edges.iter().any(|e| {
            e["from"] == json!(deploy_id)
                && e["kind"] == json!("owns")
                && e["to"].as_str().is_some_and(|to| to.starts_with(&node_id("ReplicaSet", DEPLOY)))
        }),
        "the Deployment must own a ReplicaSet: {out}"
    );
    // Both fixture replicas are up by now, so the node reads healthy — the one
    // assertion here that would catch ready/desired being read off the wrong
    // field, which no hand-built object can.
    let deploy = nodes.iter().find(|n| n["id"] == json!(deploy_id)).unwrap();
    assert_eq!(deploy["desired"], json!(2), "{out}");
    assert_eq!(deploy["health"], json!("ok"), "{out}");

    // The probe is the same graph read with one exec per pod, and a
    // capability of its own so the consent layer can gate it. On the fixture
    // pods (busybox) it reads, and it always answers with its report.
    let out = h
        .ok("k8s.topologyProbe", json!({ "context": ctx, "namespaces": [NS], "prometheus": [] }))
        .await;
    assert!(out["probe"].is_object(), "the probe must report on itself: {out}");
    assert!(
        out["nodes"].as_array().unwrap().iter().any(|n| n["id"] == json!(deploy_id)),
        "{out}"
    );

    // --- the topology's optional sources ---------------------------------------
    // The e2e cluster runs no metrics backend, and that is the ordinary case
    // the capability is written for: discovery answers an empty list, not an
    // error. Nothing the fixtures made looks like a query API, so it must not
    // be listed either.
    let out = h.ok("k8s.prometheusDiscover", json!({ "context": ctx })).await;
    let candidates = out["candidates"].as_array().unwrap();
    assert!(
        candidates.iter().all(|c| c["namespace"] != json!(NS)),
        "nothing in the fixture namespace serves PromQL: {out}"
    );
    // A query at a Service that does not exist is refused by the API server's
    // proxy, and the capability reports that as an error rather than as a
    // graph with no traffic in it.
    let msg = h
        .err(
            "k8s.prometheusQuery",
            json!({
                "context": ctx,
                "namespace": NS,
                "service": "no-such-prometheus",
                "port": 9090,
                "query": "up"
            }),
        )
        .await;
    assert!(!msg.is_empty());
    // The socket table of a fixture pod, over exec. busybox has `cat`, so the
    // pod reads; what it reports is whatever the pod has open, which the test
    // cannot know — the assertion is that the pod is accounted for, in one
    // list or the other, and never silently missing from both.
    let out = h
        .ok("k8s.listPods", json!({ "context": ctx, "namespace": NS }))
        .await;
    let pod = out["pods"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["phase"] == "Running" && p["name"].as_str().is_some_and(|n| n.starts_with(DEPLOY)))
        .map(|p| p["name"].as_str().unwrap().to_string())
        .expect("a running fixture pod");
    let out = h
        .ok(
            "k8s.podConnections",
            json!({ "context": ctx, "namespace": NS, "pods": [pod] }),
        )
        .await;
    let read = out["connections"].as_array().unwrap();
    let unread = out["unreadable"].as_array().unwrap();
    assert_eq!(
        read.iter().filter(|c| c["pod"] == json!(pod)).count()
            + unread.iter().filter(|u| u["pod"] == json!(pod)).count(),
        1,
        "the pod must be reported exactly once: {out}"
    );

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

    // === MCP resources (#24) ===================================================
    // srelens's k8s:// resource-addressing feature, exercised end to end
    // against this real cluster. Reuses `pod_name`, the Deployment pod
    // already discovered above for the logs check, rather than provisioning
    // new cluster state: the fixtures create only a Deployment (no
    // directly-named pod), so any pod used here has to be discovered via
    // listPods the same way the logs check above does.
    // === queryPodEndpoint: a real GET through an API-server port-forward ====
    println!("=== queryPodEndpoint ===");
    // The fixture pod can be Running a beat before httpd is listening, and a
    // port-forward to a closed port comes back as a clean error — so poll the
    // capability itself rather than the pod phase, never a blind sleep.
    let dl = deadline(120);
    let out = loop {
        match h
            .try_call(
                "k8s.queryPodEndpoint",
                json!({ "context": ctx, "namespace": NS, "selector": format!("app={HTTP_DEPLOY}") }),
            )
            .await
        {
            Ok(v) if v["statusCode"] == 200 => break v,
            Ok(v) if Instant::now() > dl => {
                panic!("queryPodEndpoint never got HTTP 200 from {HTTP_DEPLOY}: {v}")
            }
            Err(e) if Instant::now() > dl => {
                panic!("queryPodEndpoint never reached {HTTP_DEPLOY}: {e}")
            }
            _ => poll_sleep().await,
        }
    };
    h.mark("k8s.queryPodEndpoint");
    // Nothing but a selector: the named `http` container port must be found
    // on its own, the default path must be /metrics, and the body must reach
    // the caller intact through the tunnel.
    assert!(
        out["pod"]
            .as_str()
            .unwrap_or_default()
            .starts_with(&format!("{HTTP_DEPLOY}-")),
        "the selector must resolve to a fixture pod: {out}"
    );
    assert_eq!(
        out["port"],
        json!(8080),
        "the declared container port must be auto-detected: {out}"
    );
    assert_eq!(
        out["path"], "/metrics",
        "the default path is /metrics: {out}"
    );
    let lines = out["metrics"].as_array().unwrap();
    assert!(
        lines.iter().any(|l| l == "e2e_up 1"),
        "the sample line must come back exactly as served: {out}"
    );
    assert_eq!(
        out["totalLines"],
        json!(lines.len()),
        "with no filter and no cap, total and returned agree: {out}"
    );
    println!(
        "queryPodEndpoint: {}",
        out["summary"].as_str().unwrap_or_default()
    );

    // By pod name, explicit port, a path missing its slash, a filter and a cap:
    // both served lines mention e2e_up, so total is 2 and the cap returns one.
    let http_pod = out["pod"].as_str().unwrap().to_string();
    let out = h
        .ok(
            "k8s.queryPodEndpoint",
            json!({
                "context": ctx, "namespace": NS, "pod": http_pod,
                "port": 8080, "path": "metrics", "filter": "e2e_up", "maxLines": 1
            }),
        )
        .await;
    assert_eq!(out["statusCode"], json!(200), "{out}");
    assert_eq!(
        out["path"], "/metrics",
        "a bare path gets its leading slash: {out}"
    );
    assert_eq!(
        out["totalLines"],
        json!(2),
        "the filter is a substring match over every line: {out}"
    );
    assert_eq!(
        out["returnedLines"],
        json!(1),
        "max_lines caps what comes back: {out}"
    );
    assert_eq!(
        out["metrics"],
        json!(["# HELP e2e_up 1 when the fixture serves"]),
        "the cap keeps the first matching lines: {out}"
    );

    // A path the server does not have is a successful query with a 404 in
    // it, not an error: the status code is the answer.
    let out = h
        .ok(
            "k8s.queryPodEndpoint",
            json!({ "context": ctx, "namespace": NS, "pod": http_pod, "path": "/nope" }),
        )
        .await;
    assert_eq!(out["statusCode"], json!(404), "{out}");

    // A selector nothing matches is a clean error, not an 8-second timeout.
    let msg = h
        .err(
            "k8s.queryPodEndpoint",
            json!({ "context": ctx, "namespace": NS, "selector": "app=nothing-has-this-label" }),
        )
        .await;
    assert!(msg.contains("No running pods"), "{msg}");

    println!("=== mcp resources (#24) ===");
    mcp_resource_reads(&ctx, &pod_name).await;
    mcp_resource_subscription(&ctx, &pod_name).await;

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

    // === k8s.clusterFacts (#339) ===============================================
    // The overview rail's control-plane facts: provider, region and
    // metrics-server availability. metrics_server's own probe is API-group
    // discovery, a different path than k8s.nodeMetrics above, but the two
    // must agree on whether metrics-server is there — checked against
    // `metrics_available` rather than asserting only that a key exists.
    println!("=== cluster facts ===");
    let out = h.ok("k8s.clusterFacts", json!({ "context": ctx })).await;
    assert_eq!(out["context"], ctx);
    assert!(out["provider"].is_string(), "provider must be reported, possibly empty: {out}");
    assert!(out["region"].is_string(), "region must be reported, possibly empty: {out}");
    let state = out["metricsServer"]["state"].as_str().unwrap();
    assert!(
        ["present", "absent", "unknown"].contains(&state),
        "unexpected metrics-server state: {out}"
    );
    if metrics_available {
        assert_eq!(
            state, "present",
            "k8s.nodeMetrics served readings above, so clusterFacts must see metrics-server too: {out}"
        );
        assert!(
            !out["metricsServer"]["version"].as_str().unwrap_or_default().is_empty(),
            "a present metrics-server must report a version: {out}"
        );
    } else {
        assert_ne!(
            state, "present",
            "k8s.nodeMetrics found nothing above, so clusterFacts should not claim metrics-server is present: {out}"
        );
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

    // Review afresh only if a controller races this live fixture's pinned write.
    let (out, reviewed) = h
        .reviewed_request(
            "k8s.requestRolloutRestart",
            json!({
                "context":ctx,"group":"apps","version":"v1","kind":"Deployment",
                "plural":"deployments","namespaced":true,"namespace":NS,"name":DEPLOY
            }),
        )
        .await
        .expect("reviewed rollout restart");
    assert_eq!(out["requested"], true);
    let stale = h.err("k8s.requestRolloutRestart", reviewed).await;
    assert!(
        stale.contains("Resource changed or was replaced"),
        "{stale}"
    );

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

    // Both scheduling directions are reviewed and never evict pods. A heartbeat
    // can invalidate either review, just as a workload controller can above.
    for unschedulable in [true, false] {
        let (out, reviewed) = h
            .reviewed_request(
                "k8s.requestCordonNode",
                json!({
                    "context":ctx,"group":"","version":"v1","kind":"Node","plural":"nodes",
                    "namespaced":false,"namespace":"","name":node_name,"unschedulable":unschedulable
                }),
            )
            .await
            .expect("reviewed node scheduling request");
        assert_eq!(out["requested"], true);
        let stale = h.err("k8s.requestCordonNode", reviewed).await;
        assert!(
            stale.contains("Resource changed or was replaced"),
            "{stale}"
        );
    }

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

    // === cluster-OIDC add-cluster capabilities =================================
    // synthesizeClusterKubeconfig is a pure form→kubeconfig transform: assert it
    // produces a parseable exec-kubelogin kubeconfig naming the given context.
    println!("=== synthesizeClusterKubeconfig ===");
    let synth = h
        .ok(
            "k8s.synthesizeClusterKubeconfig",
            json!({
                "name": "e2e-synth",
                "server": "https://api.example:6443",
                "insecureSkipTlsVerify": true,
                "oidc": { "issuer": "https://dex.example", "clientId": "k8s", "extraScopes": ["groups"] }
            }),
        )
        .await;
    let synth_yaml = synth["yaml"].as_str().expect("synthesized kubeconfig yaml");
    assert!(
        synth_yaml.contains("e2e-synth"),
        "synthesized kubeconfig names the context: {synth_yaml}"
    );
    assert!(
        synth_yaml.contains("command: kubelogin"),
        "synthesized kubeconfig uses the exec kubelogin form: {synth_yaml}"
    );

    // testClusterConnection probes reachability WITHOUT running exec plugins; the
    // real kind cluster responds (even to a stripped-auth request), so it must
    // report reachable against the e2e context's own kubeconfig.
    println!("=== testClusterConnection ===");
    let kube_yaml = kubeconfig_paths()
        .iter()
        .find_map(|p| std::fs::read_to_string(p).ok().filter(|c| c.contains(ctx.as_str())))
        .expect("no kubeconfig file declares the e2e context");
    let probe = h
        .ok(
            "k8s.testClusterConnection",
            json!({ "yaml": kube_yaml, "context": ctx }),
        )
        .await;
    assert_eq!(
        probe["reachable"],
        json!(true),
        "kind cluster must be reachable: {probe}"
    );

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

// ---------------------------------------------------------------------------
// Extension apps and host GitOps actions, issue #536
// ---------------------------------------------------------------------------

/// Refuses a cluster whose Flux or Argo CD CRDs are real. Applying the fixtures
/// would replace their schemas, and teardown deletes the fixture CRDs, which
/// deletes every object of that kind: for Argo CD, every Application.
async fn refuse_real_gitops_crds(ctx: &str) {
    let kubectl = |args: &[&str]| {
        let mut command = tokio::process::Command::new("kubectl");
        command.arg("--context").arg(ctx).args(args);
        async move {
            let out = command.output().await.expect("run kubectl");
            assert!(
                out.status.success(),
                "kubectl failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8_lossy(&out.stdout).into_owned()
        }
    };
    let fixtures = kubectl(&["get", "crd", "-l", GITOPS_CRD_SELECTOR, "-o", "name"]).await;
    for name in GITOPS_CRD_NAMES {
        let present = kubectl(&["get", "crd", name, "--ignore-not-found", "-o", "name"]).await;
        assert!(
            present.trim().is_empty() || fixtures.contains(name),
            "{ctx} already has a real {name} CRD. This suite would replace it and then delete \
             it, which deletes every object of that kind; run it against a throwaway cluster"
        );
    }
}

/// The Flux and Argo CD fixture CRDs, and one object of each kind.
async fn apply_gitops_fixtures(h: &mut Harness, ctx: &str) {
    refuse_real_gitops_crds(ctx).await;
    let out = h
        .ok(
            "k8s.applyManifest",
            json!({ "context": ctx, "yaml": GITOPS_CRDS }),
        )
        .await;
    assert_eq!(
        out["applied"], true,
        "GitOps fixture CRDs must apply: {out}"
    );
    apply_once_served(h, ctx, &gitops_resources_yaml(), "the GitOps fixture CRDs").await;
    println!("fixtures applied: Flux Kustomization and Argo CD Application CRDs and objects");
}

/// An example manifest under a local ID. `org.srelens.` IDs install only with the
/// publisher's signature, and no signature covers the examples' bytes.
fn local_copy(manifest: &str) -> String {
    let copy = manifest.replacen("\"id\": \"org.srelens.", "\"id\": \"org.example.", 1);
    assert_ne!(
        copy, manifest,
        "the example no longer declares an org.srelens. ID"
    );
    copy
}

/// What the install review grants: exactly the permissions the manifest declares.
fn declared_permissions(manifest: &str) -> Value {
    serde_json::from_str::<Value>(manifest).expect("manifest JSON")["permissions"].clone()
}

fn resource_version(inspected: &Value) -> String {
    inspected["resource"]["metadata"]["resourceVersion"]
        .as_str()
        .unwrap_or_else(|| panic!("no resourceVersion in {inspected}"))
        .to_owned()
}

fn item_names(list: &Value) -> Vec<&str> {
    list["items"]
        .as_array()
        .unwrap_or_else(|| panic!("no items in {list}"))
        .iter()
        .map(|i| i["name"].as_str().unwrap_or_default())
        .collect()
}

/// Every extension capability, and the host GitOps capabilities beneath them,
/// against the fixture CRDs. Each payload is the one
/// `packages/core/src/lib/extensions.ts` sends, spelled as it spells it, so a
/// renamed field fails here instead of in the app (AGENTS.md).
/// App streams (#565) against the live cluster: a `read` stream on the Flux
/// app's Kustomization reader delivers the reader's rows, `extensions.streams`
/// counts it, and closing the view ends it with `viewClosed`.
///
/// The host's handle comes from a second build over the same settings path:
/// the streams are shared per inventory in a process, which is also what lets
/// a lifecycle change made through any registry end them.
async fn app_stream(h: &mut Harness, ctx: &str, settings: &TempSettings, flux_revision: u64) {
    println!("=== extensions: app streams ===");
    let (_, streams) = srelens_desktop_lib::capabilities::build_registry_and_app_streams(
        cache(),
        kubeconfig_paths(),
        Some(settings.0.clone()),
    );
    let streams = streams.expect("a build with settings has app streams");
    let sink = Arc::new(srelens_streams::test_util::TestSink::default());
    let channel = "extstream:e2e-1";
    let opened = streams
        .open(
            sink.clone(),
            json!({
                "id": "org.example.flux", "revision": flux_revision, "view": "e2e/page#1",
                "channel": channel, "context": ctx, "namespace": NS,
                "source": {"kind": "read", "capability": "kustomizations", "intervalSeconds": 5},
            }),
        )
        .await
        .expect("the stream opens");
    let mut data = None;
    for _ in 0..100 {
        if let Some(frame) = sink.payloads_for(channel).into_iter().find(|f| f["type"] != "open") {
            data = Some(frame);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let data = data.expect("a first frame within ten seconds");
    assert_eq!(data["type"], "data", "the first tick must be data, not a failure: {data}");
    assert!(
        data["data"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["name"] == KUSTOMIZATION),
        "{data}"
    );
    let metrics = h.ok("extensions.streams", json!({})).await;
    let flux = metrics["apps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|app| app["app"] == "org.example.flux")
        .unwrap_or_else(|| panic!("the harness registry sees the stream: {metrics}"))
        .clone();
    assert_eq!(flux["openStreams"], 1, "{metrics}");
    assert_eq!(flux["streams"][0]["stream"], json!(opened.stream), "{metrics}");
    assert_eq!(streams.close_view("e2e/page#1"), 1);
    let last = sink.payloads_for(channel).pop().unwrap();
    assert_eq!(last, json!({"type": "close", "stream": opened.stream, "reason": "viewClosed"}));
}

/// #566: a `watch` stream on the Flux app's Kustomization reader lists the
/// kind, then reports a change to the fixture Kustomization — and nothing of
/// the object itself. Run after the actions, whose reviews hold the object's
/// `resourceVersion`: the annotation here moves it.
async fn app_watch_stream(ctx: &str, settings: &TempSettings, flux_revision: u64) {
    println!("=== extensions: watch stream ===");
    let (_, streams) = srelens_desktop_lib::capabilities::build_registry_and_app_streams(
        cache(),
        kubeconfig_paths(),
        Some(settings.0.clone()),
    );
    let streams = streams.expect("a build with settings has app streams");
    let sink = Arc::new(srelens_streams::test_util::TestSink::default());
    let channel = "extstream:e2e-watch";
    let watch = streams
        .open(
            sink.clone(),
            json!({
                "id": "org.example.flux", "revision": flux_revision, "view": "e2e/page#2",
                "channel": channel, "context": ctx, "namespace": NS,
                "source": {"kind": "watch", "capability": "kustomizations"},
            }),
        )
        .await
        .expect("the watch opens");
    let events = |sink: &srelens_streams::test_util::TestSink| -> Vec<Value> {
        sink.payloads_for(channel)
            .into_iter()
            .filter(|f| f["type"] == "data")
            .map(|f| f["data"].clone())
            .collect()
    };
    let wait_for = |event: &'static str, count: usize| {
        let sink = sink.clone();
        async move {
            for _ in 0..200 {
                if events(&sink).iter().filter(|e| e["event"] == event).count() >= count {
                    return;
                }
                assert!(
                    !sink.payloads_for(channel).iter().any(|f| f["type"] == "error"),
                    "the watch failed: {:?}",
                    sink.payloads_for(channel)
                );
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            panic!("no {event} within twenty seconds: {:?}", sink.payloads_for(channel));
        }
    };
    wait_for("synced", 1).await;
    let annotated = tokio::process::Command::new("kubectl")
        .args(["--context", ctx, "-n", NS, "annotate", "--overwrite"])
        .arg(format!("kustomizations.kustomize.toolkit.fluxcd.io/{KUSTOMIZATION}"))
        .arg("srelens.io/e2e-watch=1")
        .output()
        .await
        .expect("run kubectl");
    assert!(annotated.status.success(), "{}", String::from_utf8_lossy(&annotated.stderr));
    wait_for("changed", 1).await;
    let wire = serde_json::to_string(&sink.payloads_for(channel)).unwrap();
    assert!(!wire.contains(KUSTOMIZATION), "a watch frame names no object: {wire}");
    assert_eq!(streams.close_view("e2e/page#2"), 1);
    let last = sink.payloads_for(channel).pop().unwrap();
    assert_eq!(last, json!({"type": "close", "stream": watch.stream, "reason": "viewClosed"}));
}

async fn extensions_and_gitops(h: &mut Harness, ctx: &str, settings: &TempSettings) {
    println!("=== extensions: validate, catalog, install ===");
    // As shipped, the examples carry reserved IDs. Unsigned, that is refused, and
    // the refusal names the field.
    let out = h
        .ok(
            "extensions.validate",
            json!({ "manifest": FLUX_EXAMPLE, "grants": declared_permissions(FLUX_EXAMPLE) }),
        )
        .await;
    assert!(
        out["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["code"] == "EXTENSION_RESERVED_ID" && e["path"] == "id"),
        "an unsigned org.srelens. manifest must be refused for its ID: {out}"
    );
    let flux = local_copy(FLUX_EXAMPLE);
    let argocd = local_copy(ARGOCD_EXAMPLE);
    let apps = [
        json!({ "manifest": flux, "grants": declared_permissions(&flux) }),
        json!({ "manifest": argocd, "grants": declared_permissions(&argocd) }),
    ];
    // The local examples declare writes, so permission grants also need the
    // explicit unsigned-app policy. Read-only apps do not need this setting.
    h.ok("extensions.configure", json!({"action":"unsignedApps","allowUnsignedApps":true})).await;
    for app in &apps {
        let out = h.ok("extensions.validate", app.clone()).await;
        assert_eq!(out["errors"], json!([]), "must validate: {out}");
    }

    // The catalog is a cache seeded with the committed fixture, so this suite never
    // depends on the public catalog. extension-catalog.yml checks the live one.
    let fetched_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let snapshot = json!({
        "catalog": serde_json::from_str::<Value>(CATALOG).unwrap(),
        "fetchedAt": fetched_at, "stale": false, "error": null, "incompatible": [],
    });
    std::fs::write(
        settings.catalog_cache(),
        serde_json::to_vec(&snapshot).unwrap(),
    )
    .unwrap();
    let catalog = h
        .ok("extensions.catalog", json!({ "refresh": false }))
        .await;
    assert_eq!(
        catalog["fetchedAt"], fetched_at,
        "the seeded cache must be what was read: {catalog}"
    );
    assert_eq!(catalog["stale"], false, "{catalog}");
    let sha256 = catalog["catalog"]["extensions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "org.srelens.argocd")
        .unwrap_or_else(|| panic!("the catalog lists Argo CD: {catalog}"))["release"]["sha256"]
        .clone();
    // A release the catalog does not list is refused before anything downloads.
    let err = h
        .err(
            "extensions.catalogManifest",
            json!({ "id": "org.srelens.argocd", "sha256": "0".repeat(64) }),
        )
        .await;
    assert!(err.contains("Catalog release changed"), "{err}");
    // Authentic historical bytes still verify cryptographically, but API 0.1
    // cannot be installed on this API 0.3 host.
    let old = h.ok("extensions.validate", json!({"manifest":SIGNED_ARGOCD,"grants":declared_permissions(SIGNED_ARGOCD),"signature":SIGNED_ARGOCD_SIG})).await;
    assert!(old["errors"].as_array().unwrap().iter().any(|e| e["code"] == "EXTENSION_API_INCOMPATIBLE"), "{old}");
    let err = h.err("extensions.catalogManifest", json!({"id":"org.srelens.argocd","sha256":sha256})).await;
    assert!(err.contains("different host API version"), "{err}");

    for app in &apps {
        let mut install = app.clone();
        install["action"] = json!("install");
        h.ok("extensions.configure", install).await;
    }
    let listed = h.ok("extensions.list", json!({})).await;
    let installed = |id: &str| {
        listed["plugins"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["manifest"]["id"] == id)
            .cloned()
            .unwrap_or_else(|| panic!("{id} is not installed: {listed}"))
    };
    let flux_app = installed("org.example.flux");
    let argocd_app = installed("org.example.argocd");
    for app in [&flux_app, &argocd_app] {
        assert_eq!(app["enabled"], true, "{app}");
        assert_eq!(app["source"], "local", "{app}");
    }
    let revision = |app: &Value| app["revision"].as_u64().expect("revision");

    // #543. This registry is built with no secret store (the desktop hands
    // its vault to the GUI's): clearing is always allowed, and a set is
    // refused — here because the app declares no secret setting — never kept
    // anywhere else, and the refusal does not repeat the value.
    println!("=== extensions: secret store ===");
    let cleared = h
        .ok("extension.secretStore", json!({"action": "clear", "id": "org.example.flux"}))
        .await;
    assert_eq!(cleared, json!({"set": false}), "{cleared}");
    let err = h
        .err(
            "extension.secretStore",
            json!({"action": "set", "id": "org.example.flux", "setting": "token", "secret": "e2e-secret-value"}),
        )
        .await;
    // Absence first, with a message that prints nothing of the refusal; then
    // the cause, which the refusal may be printed for once the value is known
    // not to be in it.
    assert!(!err.contains("e2e-secret-value"), "the refusal repeated the secret");
    assert!(
        err.contains("declares no secret setting"),
        "refused for another reason than an undeclared secret setting: {err}"
    );

    // #568. A network.http app reaching a one-request HTTP server on this
    // machine's loopback: the per-app switch as `@srelens/core` sends it, and the
    // request through `extensions.read`, the one path that sends one.
    println!("=== extensions: network.http ===");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().unwrap().port();
    let served = std::thread::spawn(move || {
        use std::io::{BufRead, BufReader, Write};
        let (stream, _) = listener.accept().expect("accept");
        let mut reader = BufReader::new(stream);
        let mut request_line = String::new();
        reader.read_line(&mut request_line).unwrap();
        loop {
            let mut header = String::new();
            if reader.read_line(&mut header).unwrap() == 0 || header.trim().is_empty() {
                break;
            }
        }
        let body = r#"{"status":"success"}"#;
        write!(
            reader.get_mut(),
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        request_line
    });
    let metrics = json!({
        "id": "org.example.metrics", "name": "Metrics", "version": "0.1.0",
        "srelensApiVersion": "^0.4", "kind": "declarative",
        "permissions": [{"capability": "network.http", "hosts": ["${settings.prometheusUrl}"]}],
        "settings": [{"id": "prometheusUrl", "type": "url", "title": "Prometheus URL", "required": true}],
        "capabilities": [{"name": "up", "title": "Targets up", "target": "network.http", "inputs": [],
            "arguments": {"url": "${settings.prometheusUrl}", "path": "/api/v1/query", "query": {"query": "up"}}}],
        "contributions": {"pages": [], "detailTabs": [], "detailLinks": []}
    });
    h.ok(
        "extensions.configure",
        json!({"action": "install", "manifest": metrics.to_string(), "grants": ["network.http"]}),
    )
    .await;
    h.ok(
        "extensions.configure",
        json!({"action": "settings", "id": "org.example.metrics",
               "settings": {"prometheusUrl": format!("http://127.0.0.1:{port}")}}),
    )
    .await;
    let listed = h.ok("extensions.list", json!({})).await;
    let metrics_revision = listed["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["manifest"]["id"] == "org.example.metrics")
        .and_then(|p| p["revision"].as_u64())
        .unwrap_or_else(|| panic!("the metrics app is installed: {listed}"));
    let request = json!({"id": "org.example.metrics", "revision": metrics_revision,
                         "capability": "up", "context": ctx});
    // Plain HTTP to this computer is off until a person turns it on for the app.
    let err = h.err("extensions.read", request.clone()).await;
    assert!(err.contains("Allow plain HTTP"), "{err}");
    // The wrapper's camelCase is what the host reads; the Rust spelling is refused.
    let snake = h
        .err(
            "extensions.configure",
            json!({"action": "loopbackHttp", "id": "org.example.metrics", "allow_loopback_http": true}),
        )
        .await;
    assert!(snake.contains("allow_loopback_http"), "{snake}");
    h.ok(
        "extensions.configure",
        json!({"action": "loopbackHttp", "id": "org.example.metrics", "allowLoopbackHttp": true}),
    )
    .await;
    let out = h.ok("extensions.read", request).await;
    assert_eq!(
        out,
        json!({"status": 200, "contentType": "application/json", "body": {"status": "success"}}),
        "{out}"
    );
    let request_line = served.join().expect("the server thread");
    assert!(
        request_line.starts_with("GET /api/v1/query?query=up "),
        "{request_line}"
    );
    h.ok(
        "extensions.configure",
        json!({"action": "remove", "id": "org.example.metrics"}),
    )
    .await;

    println!("=== extensions: read ===");
    let out = h
        .ok(
            "extensions.read",
            json!({
                "id": "org.example.flux", "revision": revision(&flux_app),
                "capability": "kustomizations", "context": ctx, "namespace": NS
            }),
        )
        .await;
    assert!(item_names(&out).contains(&KUSTOMIZATION), "{out}");
    let columns = h
        .ok(
            "extensions.resolveColumns",
            json!({
                "id": "org.example.flux", "revision": revision(&flux_app),
                "context": ctx, "namespace": NS,
                "kind": "kustomize.toolkit.fluxcd.io/Kustomization",
                "uids": [{ "name": KUSTOMIZATION, "namespace": NS,
                    "row": { "name": KUSTOMIZATION } }],
            }),
        )
        .await;
    assert_eq!(columns["columns"], json!([]), "{columns}");
    assert_eq!(columns["cells"][0]["name"], KUSTOMIZATION, "{columns}");
    // The dashboard card and its target page answer from one snapshot, so the
    // page shows exactly as many rows as the card counted, whatever it counted.
    let cards = h
        .ok(
            "extensions.resolveCards",
            json!({
                "id": "org.example.flux", "revision": revision(&flux_app),
                "context": ctx, "namespaces": [NS],
            }),
        )
        .await;
    let suspended = cards["cards"]
        .as_array()
        .unwrap()
        .iter()
        .find(|card| card["id"] == "suspended-kustomizations")
        .unwrap_or_else(|| panic!("the Flux example declares its card: {cards}"))
        .clone();
    assert_eq!(suspended["state"], "count", "{cards}");
    // Counted by the example's own status resolver (#541): every object once.
    let by_status = cards["cards"]
        .as_array()
        .unwrap()
        .iter()
        .find(|card| card["id"] == "kustomizations-by-status")
        .unwrap_or_else(|| panic!("the Flux example declares its status card: {cards}"))
        .clone();
    assert_eq!(by_status["state"], "countByStatus", "{cards}");
    let per_status: u64 = by_status["statuses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["count"].as_u64().unwrap())
        .sum();
    assert_eq!(per_status, by_status["total"].as_u64().unwrap(), "{cards}");
    assert!(by_status["total"].as_u64().unwrap() >= 1, "{cards}");
    let counted = h
        .ok(
            "extensions.read",
            json!({
                "id": "org.example.flux", "revision": revision(&flux_app),
                "capability": "kustomizations", "context": ctx, "namespace": NS,
                "card": "suspended-kustomizations",
            }),
        )
        .await;
    assert_eq!(
        counted["items"].as_array().unwrap().len() as u64,
        suspended["count"].as_u64().unwrap(),
        "{counted}"
    );
    for app in [&argocd_app] {
        let out = h
            .ok(
                "extensions.read",
                json!({
                    "id": app["manifest"]["id"], "revision": revision(app),
                    "capability": "applications", "context": ctx, "namespace": NS
                }),
            )
            .await;
        assert!(item_names(&out).contains(&ARGO_APP), "{out}");
    }

    println!("=== extensions: inspect, suspend and resume (Flux) ===");
    let flux_selection = json!({
        "id": "org.example.flux", "revision": revision(&flux_app),
        "capability": "kustomizations", "context": ctx, "namespace": NS, "name": KUSTOMIZATION
    });
    let detail = h.ok("extensions.resource", flux_selection.clone()).await;
    assert_eq!(
        detail["resource"]["metadata"]["name"], KUSTOMIZATION,
        "{detail}"
    );
    let panels = h
        .ok(
            "extensions.resolvePanels",
            json!({
                "id": "org.example.flux", "revision": revision(&flux_app),
                "context": ctx, "namespace": NS,
                "kind": "kustomize.toolkit.fluxcd.io/Kustomization",
                "resource": detail["resource"],
            }),
        )
        .await;
    assert_eq!(
        panels["panels"][0]["id"], "kustomization-summary",
        "{panels}"
    );
    assert_eq!(
        panels["panels"][0]["sections"][0]["fields"][0]["label"],
        "Source reference",
        "{panels}"
    );
    // A Deployment Flux applied carries the Kustomization's name and
    // namespace as labels; the link finds that Kustomization in the granted
    // list (#545). The Deployment is the caller's, as the Inspector's is.
    let links = h
        .ok(
            "extensions.resolveLinks",
            json!({
                "id": "org.example.flux", "revision": revision(&flux_app),
                "context": ctx, "namespace": NS, "kind": "apps/Deployment",
                "resource": {"apiVersion": "apps/v1", "kind": "Deployment",
                    "metadata": {"name": "e2e-flux-managed", "namespace": NS, "labels": {
                        "kustomize.toolkit.fluxcd.io/name": KUSTOMIZATION,
                        "kustomize.toolkit.fluxcd.io/namespace": NS}}},
            }),
        )
        .await;
    let kustomization = links["links"]
        .as_array()
        .and_then(|links| links.iter().find(|link| link["id"] == "kustomization"))
        .unwrap_or_else(|| panic!("no kustomization link: {links}"));
    assert_eq!(
        kustomization["targets"],
        json!([{"namespace": NS, "name": KUSTOMIZATION, "exists": true}]),
        "{links}"
    );
    app_stream(h, ctx, settings, revision(&flux_app)).await;
    assert_eq!(
        detail["actions"],
        json!(["kustomizations-suspend", "kustomizations-resume", "kustomizations-reconcile"]),
        "{detail}"
    );
    assert!(
        detail["eventsError"].is_null(),
        "the object's events must be readable: {detail}"
    );
    let uid = detail["resource"]["metadata"]["uid"]
        .as_str()
        .expect("uid")
        .to_owned();
    let reviewed = resource_version(&detail);
    let act = |action: &str, version: &str| json!({ "resource": flux_selection, "action": format!("kustomizations-{action}"), "uid": uid, "resourceVersion": version });
    let out = h.ok("extensions.action", act("suspend", &reviewed)).await;
    assert_eq!(out, json!({ "requested": true }));
    // Read back through the host capability itself, not the app, to see what landed.
    let flux_object = json!({
        "context": ctx, "group": "kustomize.toolkit.fluxcd.io", "version": "v1",
        "plural": "kustomizations", "kind": "Kustomization", "namespaced": true,
        "namespace": NS, "name": KUSTOMIZATION
    });
    let suspended = h.ok("k8s.getCustomResource", flux_object.clone()).await;
    assert_eq!(
        suspended["resource"]["spec"]["suspend"], true,
        "{suspended}"
    );
    let current = resource_version(&suspended);
    assert_ne!(
        current, reviewed,
        "the suspend patch must have written the object"
    );
    // The field is `resourceVersion`, as the wrapper sends it. The struct's own
    // spelling is refused, not silently ignored.
    let mut misspelled = act("resume", &current);
    let version = misspelled
        .as_object_mut()
        .unwrap()
        .remove("resourceVersion")
        .unwrap();
    misspelled["resource_version"] = version;
    let err = h.err("extensions.action", misspelled).await;
    assert!(err.contains("resource_version"), "{err}");
    // An action reviewed against a version that is no longer current is refused,
    // and nothing is written.
    let err = h.err("extensions.action", act("resume", &reviewed)).await;
    assert!(err.contains("Resource changed or was replaced"), "{err}");
    let unchanged = h.ok("k8s.getCustomResource", flux_object.clone()).await;
    assert_eq!(
        unchanged["resource"]["spec"]["suspend"], true,
        "{unchanged}"
    );
    assert_eq!(
        resource_version(&unchanged),
        current,
        "a refused action must not write"
    );
    let out = h.ok("extensions.action", act("resume", &current)).await;
    assert_eq!(out, json!({ "requested": true }));
    let resumed = h.ok("k8s.getCustomResource", flux_object).await;
    assert_eq!(resumed["resource"]["spec"]["suspend"], false, "{resumed}");

    println!("=== GitOps: inspect and refresh (Argo CD) ===");
    let argo_detail = h
        .ok(
            "extensions.resource",
            json!({
                "id": "org.example.argocd", "revision": revision(&argocd_app),
                "capability": "applications", "context": ctx, "namespace": NS, "name": ARGO_APP
            }),
        )
        .await;
    assert_eq!(
        argo_detail["actions"],
        json!(["refresh", "hard-refresh", "sync"]),
        "{argo_detail}"
    );
    // The same object through the host capabilities, the identity spelled out.
    let argo_object = json!({
        "context": ctx, "group": "argoproj.io", "version": "v1alpha1",
        "plural": "applications", "kind": "Application", "namespaced": true,
        "namespace": NS, "name": ARGO_APP
    });
    let before = h.ok("k8s.getCustomResource", argo_object.clone()).await;
    assert!(before.get("actions").is_none(), "Ungated readers never invent actions: {before}");
    let argo_uid = before["resource"]["metadata"]["uid"]
        .as_str()
        .expect("uid")
        .to_owned();
    let reviewed = resource_version(&before);
    let out = h
        .ok(
            "extensions.action",
            json!({ "resource": {"id":"org.example.argocd","revision":revision(&argocd_app),"capability":"applications","context":ctx,"namespace":NS,"name":ARGO_APP}, "action": "refresh", "uid": argo_uid, "resourceVersion": reviewed }),
        )
        .await;
    assert_eq!(out, json!({ "requested": true }));
    let refreshed = h.ok("k8s.getCustomResource", argo_object.clone()).await;
    assert_eq!(
        refreshed["resource"]["metadata"]["annotations"]["argocd.argoproj.io/refresh"], "normal",
        "{refreshed}"
    );
    let err = h
        .err(
            "extensions.action",
            json!({ "resource": {"id":"org.example.argocd","revision":revision(&argocd_app),"capability":"applications","context":ctx,"namespace":NS,"name":ARGO_APP}, "action": "hard-refresh", "uid": argo_uid, "resourceVersion": reviewed }),
        )
        .await;
    assert!(err.contains("Resource changed or was replaced"), "{err}");
    let unchanged = h.ok("k8s.getCustomResource", argo_object).await;
    assert_eq!(
        unchanged["resource"]["metadata"]["annotations"]["argocd.argoproj.io/refresh"], "normal",
        "a refused hard refresh must not write: {unchanged}"
    );
    assert_eq!(resource_version(&unchanged), resource_version(&refreshed));

    println!("=== host action primitives (#549) ===");
    // The writes an app's declared action makes, against the same live CRD.
    // Each one re-reads first: every primitive pins the patch to the UID and
    // resourceVersion it was handed, so the previous write invalidates them.
    let ks_object = json!({
        "context": ctx, "group": "kustomize.toolkit.fluxcd.io", "version": "v1",
        "plural": "kustomizations", "kind": "Kustomization", "namespaced": true,
        "namespace": NS, "name": KUSTOMIZATION
    });
    let reviewed = |inspected: &Value| {
        let mut input = ks_object.clone();
        input["uid"] = inspected["resource"]["metadata"]["uid"].clone();
        input["resourceVersion"] = json!(resource_version(inspected));
        input
    };

    let before = h.ok("k8s.getCustomResource", ks_object.clone()).await;
    let mut input = reviewed(&before);
    input["key"] = json!("reconcile.fluxcd.io/requestedAt");
    input["value"] = json!("$now");
    assert_eq!(
        h.ok("k8s.annotate", input).await,
        json!({"requested": true})
    );
    let annotated = h.ok("k8s.getCustomResource", ks_object.clone()).await;
    let requested_at = annotated["resource"]["metadata"]["annotations"]
        ["reconcile.fluxcd.io/requestedAt"]
        .as_str()
        .unwrap_or_else(|| panic!("no requestedAt annotation: {annotated}"));
    assert!(
        requested_at.ends_with('Z') && requested_at.contains('.'),
        "$now is RFC 3339 nanoseconds in UTC: {requested_at}"
    );
    // The review is spent: the same UID and resourceVersion cannot write twice.
    let mut stale = reviewed(&before);
    stale["key"] = json!("reconcile.fluxcd.io/requestedAt");
    stale["value"] = json!("$now");
    let err = h.err("k8s.annotate", stale).await;
    assert!(err.contains("Resource changed or was replaced"), "{err}");

    let mut input = reviewed(&annotated);
    input["fields"] = json!({"/spec/suspend": true});
    h.ok("k8s.setFields", input).await;
    let suspended = h.ok("k8s.getCustomResource", ks_object.clone()).await;
    assert_eq!(
        suspended["resource"]["spec"]["suspend"], true,
        "{suspended}"
    );
    // And back, so the object is left as the rest of the suite found it.
    // A declared precondition (#550), against the object as it now is. The
    // host reads it fresh and refuses before any patch, with the app's reason.
    let reconcile_unless_suspended = |inspected: &Value| {
        let mut input = reviewed(inspected);
        input["key"] = json!("reconcile.fluxcd.io/requestedAt");
        input["value"] = json!("$now");
        input["preconditions"] = json!([{
            "jsonPath": ".spec.suspend", "notEquals": true,
            "reason": "Resume this resource before requesting reconciliation"
        }]);
        input
    };
    let err = h
        .err("k8s.annotate", reconcile_unless_suspended(&suspended))
        .await;
    assert!(
        err.contains("Resume this resource before requesting reconciliation"),
        "{err}"
    );

    let mut input = reviewed(&suspended);
    input["fields"] = json!({"/spec/suspend": false});
    h.ok("k8s.setFields", input).await;
    let resumed = h.ok("k8s.getCustomResource", ks_object.clone()).await;
    assert_eq!(resumed["resource"]["spec"]["suspend"], false, "{resumed}");
    // The same action, the same predicate, on a resource it now admits.
    assert_eq!(
        h.ok("k8s.annotate", reconcile_unless_suspended(&resumed))
            .await,
        json!({"requested": true})
    );
    let resumed = h.ok("k8s.getCustomResource", ks_object.clone()).await;

    let mut input = reviewed(&resumed);
    input["patch"] = json!({"spec": {"prune": false}});
    h.ok("k8s.mergePatch", input).await;
    let patched = h.ok("k8s.getCustomResource", ks_object.clone()).await;
    assert_eq!(patched["resource"]["spec"]["prune"], false, "{patched}");
    assert_eq!(
        patched["resource"]["spec"]["path"], "./deploy",
        "a merge patch leaves the fields it does not name: {patched}"
    );

    let mut input = reviewed(&patched);
    input["conditionType"] = json!("Issuing");
    input["conditionStatus"] = json!("True");
    input["reason"] = json!("ManuallyTriggered");
    input["message"] = json!("Requested from the srelens e2e suite");
    h.ok("k8s.setStatusCondition", input).await;
    let conditioned = h.ok("k8s.getCustomResource", ks_object.clone()).await;
    let conditions = conditioned["resource"]["status"]["conditions"]
        .as_array()
        .unwrap_or_else(|| panic!("no conditions: {conditioned}"));
    let issuing = conditions
        .iter()
        .find(|c| c["type"] == "Issuing")
        .unwrap_or_else(|| panic!("no Issuing condition: {conditioned}"));
    assert_eq!(issuing["status"], "True", "{issuing}");
    assert_eq!(issuing["reason"], "ManuallyTriggered", "{issuing}");
    assert!(issuing["lastTransitionTime"].is_string(), "{issuing}");

    app_watch_stream(ctx, settings, revision(&flux_app)).await;

    println!("=== extensions: disable and remove ===");
    // A disabled app's views stop reading, and say why.
    h.ok(
        "extensions.configure",
        json!({ "action": "enable", "id": "org.example.flux", "enabled": false }),
    )
    .await;
    let err = h.err("extensions.resource", flux_selection).await;
    assert!(err.contains("disabled"), "{err}");
    for id in [
        "org.example.flux",
        "org.example.argocd",
    ] {
        h.ok(
            "extensions.configure",
            json!({ "action": "remove", "id": id }),
        )
        .await;
    }
    let listed = h.ok("extensions.list", json!({})).await;
    assert_eq!(listed["plugins"], json!([]), "{listed}");
}

/// `k8s.deleteContext` REMOVES A CONTEXT FROM THE KUBECONFIG ON DISK. Never
/// run it against the real kubeconfig: copy the file that declares `ctx` to a
/// private temp file, build a SEPARATE `ClientCache`/registry pointing only at
/// that copy, delete the context from the COPY, and assert the real
/// kubeconfig is untouched.
/// The spec's krew integration test: really bootstrap krew from GitHub and run
/// a small plugin through the full install → search → remove lifecycle, driving
/// the same `toolbox.*` capabilities the GUI and MCP use. Network-dependent by
/// nature (that's the point — it proves the real subprocess + download path).
async fn toolbox_krew_lifecycle(h: &mut Harness) {
    println!("=== toolbox: krew bootstrap + plugin lifecycle ===");
    let krew_home = PathBuf::from(std::env::var("HOME").expect("HOME")).join(".krew/bin");

    // Real bootstrap: download krew and run `krew install krew` into ~/.krew.
    let out = h.ok("toolbox.installKrew", json!({})).await;
    assert_eq!(out["tool"], "krew");
    assert!(out["version"].as_str().is_some(), "krew version should resolve: {out}");
    assert!(krew_home.join("kubectl-krew").exists(), "krew shim should be installed");

    // The index lists `ns` (kubens) — a small, stable plugin.
    let out = h.ok("toolbox.searchPlugins", json!({ "query": "ns" })).await;
    assert!(
        out["plugins"].as_array().unwrap().iter().any(|p| p["name"] == "ns"),
        "krew index should list ns: {out}",
    );

    // Install it (its binary lands in ~/.krew/bin), then remove it.
    let out = h.ok("toolbox.installPlugin", json!({ "plugin": "ns" })).await;
    assert_eq!(out["plugin"], "ns");
    assert!(krew_home.join("kubectl-ns").exists(), "kubectl-ns should be installed");

    let out = h.ok("toolbox.removePlugin", json!({ "plugin": "ns" })).await;
    assert_eq!(out["plugin"], "ns");
    assert!(!krew_home.join("kubectl-ns").exists(), "kubectl-ns should be removed");
    println!("krew lifecycle OK");
}

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

/// #24's read acceptance criterion: a manifest, an events list and pod logs
/// all read successfully over MCP's `resources/read`, plus the curation
/// guarantee that a Secret which genuinely exists in this cluster is still
/// not addressable. CI can only exercise the error paths (parse/plan
/// failures pinned by unit tests in `crates/mcp/src/resources.rs` and
/// `stdio.rs`) since a successful read needs a real object; this is the
/// real-cluster half.
///
/// The `logs` expectation is an empty string deliberately: a pod may
/// legitimately have produced no output, so the assertion is that the read
/// *succeeds* and returns text, not that the text contains anything specific
/// (this suite's Deployment pods do log "hello", so the substring check
/// against "" is trivially satisfied either way).
async fn mcp_resource_reads(ctx: &str, pod: &str) {
    println!("=== mcp resources: reads ===");
    let server = srelens_mcp::McpServer::new(Arc::new(build_registry_with(cache())))
        .with_kind_resolver(srelens_registry::kind_resolver());

    for (uri, expect) in [
        (format!("k8s://{ctx}/{NS}/Pod/{pod}"), "kind: Pod"),
        (format!("k8s://{ctx}/{NS}/Pod/{pod}/events"), "["),
        (format!("k8s://{ctx}/{NS}/Pod/{pod}/logs"), ""),
    ] {
        let resp = srelens_mcp::stdio::handle_request(
            &server,
            &json!({"jsonrpc":"2.0","id":1,"method":"resources/read",
                                "params":{"uri":uri}}),
            srelens_mcp::Transport::Stdio,
        )
        .await
        .expect("a response");
        let contents = &resp["result"]["contents"][0];
        let text = contents["text"]
            .as_str()
            .unwrap_or_else(|| panic!("read of {uri} failed: {resp}"));
        assert!(text.contains(expect), "read of {uri} returned {text}");
        assert!(contents["mimeType"].is_string());
    }

    // The curation guarantee against a real cluster: the SECRET fixture
    // genuinely exists in NS, and is still not addressable.
    let resp = srelens_mcp::stdio::handle_request(
        &server,
        &json!({"jsonrpc":"2.0","id":2,"method":"resources/read",
                            "params":{"uri":format!("k8s://{ctx}/{NS}/Secret/{SECRET}")}}),
        srelens_mcp::Transport::Stdio,
    )
    .await
    .expect("a response");
    assert_eq!(resp["error"]["code"], -32602, "unexpected response: {resp}");
    println!("mcp resources: manifest/events/logs read OK; Secret still unaddressable");
}

/// #24's subscription acceptance criterion: subscribe to a pod's manifest and
/// see a notification when it changes.
///
/// The initial list a subscribe triggers DOES notify: classification in
/// `is_object_change` (`crates/kube/src/watch.rs`) fires `on_change` for
/// every `InitDone`, including the very first one, because a read followed by
/// a subscribe is not atomic — the object can already differ from what the
/// client last saw by the time the subscription's first list completes, and
/// staying silent would leave the client trusting stale state with no
/// correction. So `hits` is already nonzero once the watch is confirmed
/// running, before any mutation. To still prove that a REAL subsequent change
/// produces its own notification (not just the guaranteed initial one), this
/// test records the hit count as a baseline after the watch is up, then
/// mutates the watched pod, and asserts the count rises *above that
/// baseline* — not merely `> 0`, which the initial notification alone would
/// already satisfy and would make this test pass without exercising the
/// mutation path at all.
///
/// The mutation is a label added via server-side apply (an `Apply` event)
/// rather than deleting the pod: the pod is the live Deployment replica the
/// logs check above already exercised, and a label patch leaves it running
/// under the same name, so nothing else in the suite has to wait for the
/// Deployment to notice and recreate a differently-named replacement.
async fn mcp_resource_subscription(ctx: &str, pod: &str) {
    println!("=== mcp resources: subscription ===");
    let server = srelens_mcp::McpServer::new(Arc::new(build_registry_with(cache())))
        .with_kind_resolver(srelens_registry::kind_resolver())
        .with_watcher(Arc::new(srelens_desktop_lib::mcp_watch::CacheWatcher::new(
            cache(),
        )));

    let uri = srelens_mcp::resources::ResourceUri::parse(&format!("k8s://{ctx}/{NS}/Pod/{pod}"))
        .unwrap();

    let hits = Arc::new(std::sync::Mutex::new(0usize));
    let counter = hits.clone();
    let handle = srelens_mcp::resources::ObjectWatcher::watch(
        server.watcher().as_ref(),
        &uri,
        Box::new(move || *counter.lock().unwrap() += 1),
        Box::new(|reason| eprintln!("e2e watch reported dead: {reason}")),
    )
    .expect("watch spawns");

    // Establish the watch first, and confirm it is actually running, before
    // mutating anything: the ordering is what makes the assertion below mean
    // "the notification followed our change" rather than "something fired at
    // some point".
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(
        !handle.is_finished(),
        "the watch task ended before the mutation ran; it should still be watching"
    );

    // Baseline after the initial list, which itself notifies (see the doc
    // comment above) — the mutation must push the count *past* this, not
    // merely make it nonzero.
    let baseline = *hits.lock().unwrap();

    let label_yaml = format!(
        "apiVersion: v1\nkind: Pod\nmetadata:\n  name: {pod}\n  namespace: {NS}\n  labels:\n    e2e-subscription-marker: touched\n"
    );
    let out = server
        .call_tool(
            "k8s.applyManifest",
            json!({ "context": ctx, "yaml": label_yaml }),
        )
        .await
        .expect("labeling the watched pod must succeed");
    assert_eq!(out["applied"], true, "label apply must succeed: {out}");

    let dl = deadline(30);
    loop {
        if *hits.lock().unwrap() > baseline {
            break;
        }
        if Instant::now() > dl {
            handle.abort();
            panic!(
                "expected the hit count to rise above the post-initial-list baseline \
                 ({baseline}) after labeling {pod}, but it did not"
            );
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    handle.abort();
    println!("mcp resources: subscription fired on Apply after a label change");
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

    // Only the GitOps CRDs the suite applied, by their fixture label. A real Flux
    // or Argo CD CRD never matches, and the suite refuses to run over one.
    let gitops_out = tokio::process::Command::new("kubectl")
        .args([
            "--context",
            &ctx,
            "delete",
            "crd",
            "-l",
            GITOPS_CRD_SELECTOR,
            "--ignore-not-found",
            "--wait=true",
            "--timeout=60s",
        ])
        .output()
        .await;
    match gitops_out {
        Ok(o) if o.status.success() => println!("teardown: GitOps fixture CRDs deleted"),
        Ok(o) => println!(
            "teardown: GitOps fixture CRD delete non-zero exit: {}",
            String::from_utf8_lossy(&o.stderr)
        ),
        Err(e) => println!("teardown: failed to run kubectl to delete GitOps fixture CRDs: {e}"),
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

// ---------------------------------------------------------------------------
// MCP prompts, issue #25
// ---------------------------------------------------------------------------

/// #25's acceptance criterion: a client can run a CrashLoop triage end-to-end
/// using only advertised tools. This checks the prompt half — the flow is
/// advertised, renders against a real context, and every tool it names is a
/// capability that actually exists in the registry. A prompt that instructs a
/// call to a renamed tool sends the agent down a dead end, and nothing else in
/// the suite would notice.
///
/// Unlike its neighbour above, this test is NOT `#[ignore]`d: `build_registry`
/// only constructs a lazy client cache and registers capability closures (no
/// connection attempt), `Registry::ids()` is a synchronous map-key read, and
/// `PromptLibrary`/`prompts/list`/`prompts/get` are pure in-memory string
/// work — nothing here touches a cluster, `helm`, or `kubectl`. It runs safely
/// in CI with no kubeconfig at all (verified with a nonexistent `KUBECONFIG`
/// and an empty `HOME`), and it is the only check that would notice a
/// referenced capability being renamed or removed out from under a prompt
/// body — `every_referenced_tool_is_on_the_read_only_allowlist` in
/// `crates/mcp/src/prompts.rs` only checks against a hardcoded allowlist, so
/// it can't see the registry drift.
#[tokio::test(flavor = "multi_thread")]
async fn mcp_prompts_name_only_real_capabilities() {
    let registry = srelens_desktop_lib::build_registry();
    let ids: Vec<String> = registry.ids().into_iter().map(str::to_string).collect();
    let server = srelens_mcp::McpServer::new(std::sync::Arc::new(registry))
        .with_prompts(srelens_mcp::prompts::PromptLibrary::new(None));

    let listed = srelens_mcp::stdio::handle_request(
        &server,
        &serde_json::json!({"jsonrpc":"2.0","id":1,"method":"prompts/list"}),
        srelens_mcp::Transport::Stdio,
    )
    .await
    .expect("prompts/list responds");
    let names: Vec<String> = listed["result"]["prompts"]
        .as_array()
        .expect("prompts array")
        .iter()
        .map(|p| p["name"].as_str().unwrap().to_string())
        .collect();
    assert!(names.contains(&"pod-crashloop".to_string()), "got {names:?}");

    for name in &names {
        for arguments in [
            serde_json::json!({ "context": context() }),
            serde_json::json!({ "context": context(), "namespace": NS,
                                "pod": "any", "node": "any", "service": "any" }),
        ] {
            let resp = srelens_mcp::stdio::handle_request(
                &server,
                &serde_json::json!({"jsonrpc":"2.0","id":2,"method":"prompts/get",
                    "params": { "name": name, "arguments": arguments }}),
                srelens_mcp::Transport::Stdio,
            )
            .await
            .expect("prompts/get responds");
            let text = resp["result"]["messages"][0]["content"]["text"]
                .as_str()
                .unwrap_or_else(|| panic!("{name} did not render: {resp}"));
            assert!(!text.contains("{{"), "{name} left a placeholder: {text}");

            // Every `k8s.foo` the prompt tells the agent to call must exist.
            for token in text.split_whitespace() {
                // Trim all non-alphanumeric edge punctuation, including '.':
                // an internal dot (`k8s.getObject`) sits between two
                // alphanumeric runs, so it is never at an edge and survives
                // the trim untouched. Only a glued sentence-ending period (or
                // backtick) at the token's edge gets stripped. Mirrors
                // `tool_tokens` in `crates/mcp/src/prompts.rs`.
                let candidate = token.trim_matches(|c: char| !c.is_ascii_alphanumeric());
                if candidate.starts_with("k8s.") || candidate.starts_with("toolbox.") {
                    assert!(
                        ids.iter().any(|id| id == candidate),
                        "{name} names `{candidate}`, which is not a registered capability"
                    );
                }
            }
        }
    }
}

#[tokio::test]
async fn reviewed_request_refreshes_only_explicit_review_races_and_bounds_retries() {
    use srelens_capability::{Annotations, Capability, CapabilityError};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    };
    const STALE: &str = "Resource changed or was replaced; refresh and review the action again";
    const CONFLICT: &str = "ApiError: the object has been modified: Conflict (Status { status: Some(Failure), code: 409, message: modified })";
    for (errors, expected_reads, succeeds) in [
        (vec![STALE, CONFLICT], 3, true),
        (vec!["Resource is being deleted"], 1, false),
        (vec!["Action request timed out"], 1, false),
        (vec!["Forbidden"], 1, false),
        (vec!["This action is not available: wait"], 1, false),
        (vec!["409 unrelated error"], 1, false),
        (vec![STALE; 20], 8, false),
    ] {
        let reads = Arc::new(AtomicUsize::new(0));
        let seen = Arc::new(Mutex::new(Vec::<Value>::new()));
        let mut reg = Registry::new();
        let count = reads.clone();
        reg.register(Capability::typed::<Value, Value, _, _>("k8s.getObject", "read", Annotations::READ_ONLY, move |_| {
            let version = count.fetch_add(1, Ordering::SeqCst) + 1;
            async move {Ok(json!({"object":{"metadata":{"uid":"u","resourceVersion":version.to_string()}}}))}
        }));
        let captured = seen.clone();
        reg.register(Capability::typed::<Value, Value, _, _>(
            "reviewed",
            "write",
            Annotations::MUTATING,
            move |input| {
                let mut seen = captured.lock().unwrap();
                let error = errors.get(seen.len()).copied();
                seen.push(input);
                async move {
                    match error {
                        Some(error) => Err(CapabilityError::Handler(error.into())),
                        None => Ok(json!({"requested":true})),
                    }
                }
            },
        ));
        let mut h = Harness::new(reg);
        let result = h
            .reviewed_request(
                "reviewed",
                json!({"context":"c","kind":"Node","namespace":"","name":"n","unschedulable":true}),
            )
            .await;
        assert_eq!(result.is_ok(), succeeds, "{result:?}");
        assert_eq!(reads.load(Ordering::SeqCst), expected_reads);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), expected_reads);
        for (index, input) in seen.iter().enumerate() {
            assert_eq!(input["resourceVersion"], (index + 1).to_string());
            assert_eq!(input["uid"], "u");
            assert_eq!(input["unschedulable"], true);
        }
        assert_eq!(h.covered.contains("reviewed"), succeeds);
        if let Ok((out, review)) = result {
            assert_eq!(out["requested"], true);
            assert_eq!(review, *seen.last().unwrap());
        }
    }
}
