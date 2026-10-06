//! Constrained, scoped Kubernetes Jobs run for an installed app.

use futures::AsyncReadExt;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use srelens_capability::{Annotations, Capability, CapabilityError, Registry};
use srelens_kube::k8s_openapi::api::batch::v1::Job;
use srelens_kube::k8s_openapi::api::core::v1::{ConfigMap, Pod, ServiceAccount};
use srelens_kube::k8s_openapi::api::rbac::v1::{Role, RoleBinding};
use srelens_kube::kube::api::{
    DeleteParams, ListParams, LogParams, PostParams, Preconditions, PropagationPolicy,
};
use srelens_kube::kube::{Api, Client};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MAX_RESULT: usize = 8 * 1024 * 1024;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RunIn {
    id: String,
    revision: u64,
    context: String,
    namespace: String,
    capability: String,
    inputs: BTreeMap<String, String>,
}

#[derive(Serialize, JsonSchema)]
struct RunOut {
    state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    path: String,
    job: String,
    uid: String,
    namespace: String,
    image: String,
    bytes: usize,
    #[serde(rename = "finishedAt")]
    finished_at: String,
}

pub(crate) fn capability() -> Capability {
    Capability::typed::<JobTemplate, RunOut, _, _>(
        "k8s.runJob",
        "Run a declared scoped app Job through the extension broker",
        Annotations::MUTATING,
        |_| async {
            Err(CapabilityError::Handler(
                "Jobs run only through the extension broker for an installed executable app".into(),
            ))
        },
    )
    .checking_bound_arguments(|args| {
        let template = serde_json::from_value(serde_json::Value::Object(args.clone()))
            .map_err(|_| "Invalid Job template".to_owned())?;
        check_template(&template)
    })
}

pub(super) fn register(
    reg: &mut Registry,
    apps: &super::Apps,
    core: Arc<Registry>,
    cache: Arc<srelens_kube::client_cache::ClientCache>,
) {
    let store = apps.inventory.clone();
    let root = apps.data.clone();
    let active = Arc::new(Mutex::new(HashSet::new()));
    reg.register(Capability::typed::<RunIn, RunOut, _, _>("extensions.runJob", "Run a scoped container Job for an installed app and collect its bounded result", Annotations::MUTATING.with_confirm("Run this app's container Job[ in namespace {namespace}][ in cluster {cluster}]? It reads only its declared resources and is cleaned up after the scan."), move |input| {
        let (store, root, core, cache, active) = (store.clone(), root.clone(), core.clone(), cache.clone(), active.clone());
        async move {
            if !namespace_name(&input.namespace) || input.context.trim().is_empty() || input.context.len() > 4096 {
                return Err(CapabilityError::InvalidInput("Select a cluster and one namespace for the Job".into()));
            }
            let root = root.ok_or_else(|| CapabilityError::Handler("This host cannot keep an executable app's Job results".into()))?;
            let (state, index, context) = super::resolver_app(store, &core, &cache, &input.id, input.revision, input.context).await?;
            let app = &state.plugins[index];
            let binding = app.manifest.capabilities.iter().find(|binding| binding.name == input.capability)
                .ok_or_else(|| CapabilityError::InvalidInput("The app declares no such Job binding".into()))?;
            if app.manifest.kind != srelens_plugin_host::ManifestKind::Executable || binding.target != "k8s.runJob" || !app.grants.iter().any(|grant| grant == "k8s.runJob") {
                return Err(CapabilityError::InvalidInput("This executable app has no grant for the declared Job".into()));
            }
            let template: JobTemplate = serde_json::from_value(serde_json::Value::Object(binding.arguments.clone()))
                .map_err(|_| CapabilityError::InvalidInput("Invalid declared Job template".into()))?;
            let run = run_id();
            let job = build_job(&input.id, &input.namespace, &run, &template, &input.inputs).map_err(CapabilityError::InvalidInput)?;
            let permit = Active::take(active, &input.id).map_err(CapabilityError::Handler)?;
            let data = srelens_plugin_host::sidecar::data::DataDir::for_app(&root, &input.id).map_err(|e| CapabilityError::Handler(e.to_string()))?;
            let limits = srelens_plugin_host::sidecar::Limits::default();
            let usage = srelens_plugin_host::sidecar::data::measure(data.path(), limits.data_entries).map_err(|e| CapabilityError::Handler(e.to_string()))?;
            if usage.entries >= limits.data_entries || usage.bytes.saturating_add(MAX_RESULT as u64) > limits.data_bytes {
                return Err(CapabilityError::Handler("The app's report storage is full; remove old results before scanning".into()));
            }
            let client = cache.get(&context).await.map_err(CapabilityError::Handler)?;
            run_job(client, &input.namespace, &run, job, &template, data.path(), permit).await.map_err(CapabilityError::Handler)
        }
    }));
}

struct Active {
    apps: Arc<Mutex<HashSet<String>>>,
    id: String,
}
impl Active {
    fn take(apps: Arc<Mutex<HashSet<String>>>, id: &str) -> Result<Self, String> {
        if !apps.lock().unwrap().insert(id.to_owned()) {
            return Err("This app already has an active scan; finish or cancel it first".into());
        }
        Ok(Self {
            apps,
            id: id.to_owned(),
        })
    }
}
impl Drop for Active {
    fn drop(&mut self) {
        self.apps.lock().unwrap().remove(&self.id);
    }
}

fn run_id() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    format!(
        "{:x}-{:x}-{:x}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

fn completed_worker(job: &Job, uid: &str, pods: &[Pod]) -> Result<Option<String>, String> {
    if job.metadata.uid.as_deref() != Some(uid) {
        return Err("The scan Job was replaced or removed; its result cannot be used".into());
    }
    let status = job.status.as_ref();
    let failed = status
        .and_then(|s| s.conditions.as_ref())
        .is_some_and(|conditions| {
            conditions
                .iter()
                .any(|c| c.status == "True" && c.type_ == "Failed")
        });
    let complete = status
        .and_then(|s| s.conditions.as_ref())
        .is_some_and(|conditions| {
            conditions
                .iter()
                .any(|c| c.status == "True" && c.type_ == "Complete")
        });
    let owned: Vec<_> = pods
        .iter()
        .filter(|pod| {
            pod.metadata
                .owner_references
                .as_ref()
                .is_some_and(|owners| {
                    owners.iter().any(|owner| {
                        owner.uid == uid && owner.kind == "Job" && owner.controller == Some(true)
                    })
                })
        })
        .collect();
    if owned.len() > 1 || (complete && owned.len() != 1) {
        return Err("The scan Job has no single owned worker; its result cannot be used".into());
    }
    let Some(pod) = owned.first() else {
        if failed {
            return Err("The scan Job failed without a worker".into());
        }
        return Ok(None);
    };
    let terminated = pod
        .status
        .as_ref()
        .and_then(|s| s.container_statuses.as_ref())
        .and_then(|statuses| statuses.iter().find(|s| s.name == "worker"))
        .and_then(|s| s.state.as_ref())
        .and_then(|s| s.terminated.as_ref());
    if let Some(exit) = terminated {
        if exit.exit_code != 0 {
            return Err(format!(
                "The scan container failed with exit code {} ({})",
                exit.exit_code,
                exit.reason.as_deref().unwrap_or("unknown reason")
            ));
        }
        if complete {
            return pod
                .metadata
                .name
                .clone()
                .map(Some)
                .ok_or_else(|| "The completed scan worker has no name".into());
        }
    }
    if failed {
        if let Some(condition) = pod
            .status
            .as_ref()
            .and_then(|s| s.conditions.as_ref())
            .and_then(|conditions| {
                conditions
                    .iter()
                    .find(|c| c.type_ == "PodScheduled" && c.status == "False")
            })
        {
            return Err(format!(
                "The scan Job could not be scheduled: {}",
                condition
                    .message
                    .as_deref()
                    .unwrap_or("insufficient scheduling capacity")
                    .chars()
                    .take(2000)
                    .collect::<String>()
            ));
        }
        return Err(
            "The scan Job failed or exceeded its deadline; no successful scan result exists".into(),
        );
    }
    if complete {
        return Err("The scan Job completed without a successful worker exit".into());
    }
    Ok(None)
}

async fn write_result<R: futures::AsyncRead + Unpin>(
    path: &Path,
    reader: R,
) -> Result<usize, String> {
    let mut bytes = Vec::new();
    reader
        .take((MAX_RESULT + 1) as u64)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| "Could not read the scan container result".to_owned())?;
    if bytes.is_empty() || bytes.len() > MAX_RESULT {
        return Err("The scan result is empty or exceeds eight MiB; narrow the namespace".into());
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|_| "Could not create the private scan result file".to_owned())?;
    use std::io::Write;
    if file
        .write_all(&bytes)
        .and_then(|_| file.sync_all())
        .is_err()
    {
        let _ = std::fs::remove_file(path);
        return Err("Could not save the scan result".into());
    }
    Ok(bytes.len())
}

async fn delete_owned(api: &Api<Job>, name: &str, uid: &str) -> Result<(), String> {
    let params = DeleteParams {
        propagation_policy: Some(PropagationPolicy::Foreground),
        preconditions: Some(Preconditions {
            uid: Some(uid.to_owned()),
            resource_version: None,
        }),
        ..Default::default()
    };
    let cleanup = async {
        match api.delete(name, &params).await {
            Ok(_) => {}
            Err(srelens_kube::kube::Error::Api(status)) if matches!(status.code, 404 | 409) => {
                return Ok(())
            }
            Err(e) => return Err(format!("Could not clean up scan Job {name}: {e}")),
        }
        loop {
            match api.get(name).await {
                Ok(job) if job.metadata.uid.as_deref() != Some(uid) => return Ok(()),
                Ok(_) => tokio::time::sleep(Duration::from_millis(250)).await,
                Err(srelens_kube::kube::Error::Api(status)) if status.code == 404 => return Ok(()),
                Err(e) => return Err(format!("Could not confirm scan Job cleanup: {e}")),
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(30), cleanup)
        .await
        .map_err(|_| format!("Cleaning up scan Job {name} timed out"))?
}

struct Cleanup {
    api: Api<Job>,
    owned: Option<(String, String)>,
    active: Option<Active>,
    raw: Option<PathBuf>,
}
impl Drop for Cleanup {
    fn drop(&mut self) {
        let Some((name, uid)) = self.owned.take() else {
            return;
        };
        let api = self.api.clone();
        let active = self.active.take();
        let raw = self.raw.take();
        tokio::spawn(async move {
            if let Err(error) = delete_owned(&api, &name, &uid).await {
                log::warn!("{error}; Kubernetes deadline and TTL remain in place");
            }
            if let Some(path) = raw {
                let _ = std::fs::remove_file(path);
            }
            drop(active);
        });
    }
}

async fn run_job(
    client: Client,
    namespace: &str,
    run: &str,
    job: Job,
    template: &JobTemplate,
    data: &Path,
    active: Active,
) -> Result<RunOut, String> {
    let api: Api<Job> = Api::namespaced(client.clone(), namespace);
    let mut cleanup = Cleanup {
        api: api.clone(),
        owned: None,
        active: Some(active),
        raw: None,
    };
    let task = async {
        let created = api
            .create(&PostParams::default(), &job)
            .await
            .map_err(|e| format!("Could not create scan Job in {namespace}: {e}"))?;
        let name = created
            .metadata
            .name
            .ok_or_else(|| "The API returned a scan Job without a name".to_owned())?;
        let uid = created
            .metadata
            .uid
            .ok_or_else(|| "The API returned a scan Job without its UID".to_owned())?;
        cleanup.owned = Some((name.clone(), uid.clone()));
        if !template.read_rules.is_empty() {
            let reader = format!("srelens-run-{run}");
            let metadata = json!({"name":reader,"namespace":namespace,"ownerReferences":[{"apiVersion":"batch/v1","kind":"Job","name":name,"uid":uid}]});
            let sa: ServiceAccount = serde_json::from_value(json!({"metadata":metadata})).unwrap();
            Api::<ServiceAccount>::namespaced(client.clone(), namespace)
                .create(&PostParams::default(), &sa)
                .await
                .map_err(|e| format!("Could not create the scan reader account: {e}"))?;
            let role: Role =
                serde_json::from_value(json!({"metadata":metadata,"rules":template.read_rules}))
                    .unwrap();
            Api::<Role>::namespaced(client.clone(), namespace)
                .create(&PostParams::default(), &role)
                .await
                .map_err(|e| format!("Could not grant the scan's namespace reads: {e}"))?;
            let binding: RoleBinding = serde_json::from_value(json!({"metadata":metadata,"roleRef":{"apiGroup":"rbac.authorization.k8s.io","kind":"Role","name":reader},"subjects":[{"kind":"ServiceAccount","name":reader,"namespace":namespace}]})).unwrap();
            Api::<RoleBinding>::namespaced(client.clone(), namespace)
                .create(&PostParams::default(), &binding)
                .await
                .map_err(|e| format!("Could not bind the scan reader: {e}"))?;
            let ready: ConfigMap =
                serde_json::from_value(json!({"metadata":metadata,"data":{"ready":"true"}}))
                    .unwrap();
            Api::<ConfigMap>::namespaced(client.clone(), namespace)
                .create(&PostParams::default(), &ready)
                .await
                .map_err(|e| format!("Could not start the scan worker: {e}"))?;
        }
        let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
        let list = ListParams::default().labels(&format!("srelens.io/run={run}"));
        let (worker, failure) = loop {
            let current = api
                .get(&name)
                .await
                .map_err(|e| format!("Could not observe the scan Job: {e}"))?;
            let workers = pods
                .list(&list)
                .await
                .map_err(|e| format!("Could not observe the scan worker: {e}"))?;
            match completed_worker(&current, &uid, &workers.items) {
                Ok(Some(worker)) => break (worker, None),
                Ok(None) => {}
                Err(reason) => {
                    let owned: Vec<_> = workers
                        .items
                        .iter()
                        .filter(|pod| {
                            pod.metadata
                                .owner_references
                                .as_ref()
                                .is_some_and(|owners| {
                                    owners.iter().any(|owner| {
                                        owner.uid == uid
                                            && owner.kind == "Job"
                                            && owner.controller == Some(true)
                                    })
                                })
                        })
                        .collect();
                    if current.metadata.uid.as_deref() == Some(&uid)
                        && owned.len() == 1
                        && owned[0]
                            .status
                            .as_ref()
                            .and_then(|s| s.container_statuses.as_ref())
                            .is_some_and(|statuses| {
                                statuses.iter().any(|s| {
                                    s.name == "worker"
                                        && s.state
                                            .as_ref()
                                            .and_then(|s| s.terminated.as_ref())
                                            .is_some()
                                })
                            })
                    {
                        let name = owned[0]
                            .metadata
                            .name
                            .clone()
                            .ok_or_else(|| "The failed worker has no name".to_owned())?;
                        break (name, Some(reason));
                    }
                    return Err(reason);
                }
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        };
        let logs = pods
            .log_stream(
                &worker,
                &LogParams {
                    container: Some("worker".into()),
                    limit_bytes: Some((MAX_RESULT + 1) as i64),
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| format!("Could not collect the completed scan result: {e}"))?;
        let path = format!(
            "job-result-{}.json",
            &format!("{:x}", Sha256::digest(uid.as_bytes()))[..32]
        );
        let file = data.join(&path);
        let bytes = write_result(&file, logs)
            .await
            .map_err(|why| match &failure {
                Some(reason) => format!("{reason}; {why}"),
                None => why,
            })?;
        cleanup.raw = Some(file);
        delete_owned(&api, &name, &uid).await?;
        cleanup.owned = None;
        cleanup.raw = None;
        Ok(RunOut {
            state: if failure.is_some() {
                "failed"
            } else {
                "completed"
            }
            .into(),
            error: failure,
            path,
            job: name,
            uid,
            namespace: namespace.into(),
            image: template.image.clone(),
            bytes,
            finished_at: chrono::DateTime::<chrono::Utc>::from(SystemTime::now()).to_rfc3339(),
        })
    };
    tokio::time::timeout(Duration::from_secs(1230), task)
        .await
        .map_err(|_| {
            "The scan exceeded its twenty-minute deadline; its Job is being cleaned up".to_owned()
        })?
}

#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ReadRule {
    #[serde(rename = "apiGroups")]
    api_groups: Vec<String>,
    resources: Vec<String>,
    verbs: Vec<String>,
}

#[derive(Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct JobTemplate {
    image: String,
    command: Vec<String>,
    args: Vec<String>,
    #[serde(rename = "inputNames", default)]
    input_names: Vec<String>,
    #[serde(rename = "readRules", default)]
    read_rules: Vec<ReadRule>,
}

fn namespace_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 63
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value.as_bytes()[value.len() - 1].is_ascii_alphanumeric()
}

fn placeholder(value: &str) -> Option<&str> {
    value.strip_prefix("${inputs.")?.strip_suffix('}')
}

fn check_template(template: &JobTemplate) -> Result<(), String> {
    let pinned = template
        .image
        .split_once("@sha256:")
        .is_some_and(|(name, digest)| {
            !name.is_empty()
                && name.len() <= 400
                && !name.contains("..")
                && name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"/._-:".contains(&b))
                && digest.len() == 64
                && digest
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        });
    if !pinned {
        return Err("A Job needs a digest-pinned container image".into());
    }
    if template.command.is_empty()
        || template.command.len() > 8
        || template.args.len() > 64
        || template
            .command
            .iter()
            .chain(&template.args)
            .any(|arg| arg.len() > 4096 || arg.contains('\0'))
        || template
            .command
            .iter()
            .any(|arg| arg.is_empty() || arg.contains("${"))
    {
        return Err("The Job command or arguments exceed their allowed shape".into());
    }
    let mut names = HashSet::new();
    if template.input_names.len() > 16
        || template.input_names.iter().any(|name| {
            !srelens_sidecar_protocol::shape::is_identifier(name) || !names.insert(name.as_str())
        })
    {
        return Err("A Job may declare at most sixteen unique input names".into());
    }
    if template
        .args
        .iter()
        .any(|arg| arg.contains("${") && !placeholder(arg).is_some_and(|name| names.contains(name)))
    {
        return Err("Job inputs must occupy whole, declared arguments".into());
    }
    if template.read_rules.len() > 16 {
        return Err("Too many Job reader rules".into());
    }
    for rule in &template.read_rules {
        if rule.api_groups.is_empty()
            || rule.resources.is_empty()
            || rule.verbs.is_empty()
            || rule.api_groups.len() > 8
            || rule.resources.len() > 32
            || rule.verbs.len() > 3
            || rule
                .verbs
                .iter()
                .any(|verb| !matches!(verb.as_str(), "get" | "list" | "watch"))
        {
            return Err("Job reader access must name bounded, read-only resources".into());
        }
        for group in &rule.api_groups {
            let allowed: &[&str] = match group.as_str() {
                "" => &[
                    "pods",
                    "services",
                    "configmaps",
                    "serviceaccounts",
                    "replicationcontrollers",
                    "persistentvolumeclaims",
                    "limitranges",
                    "resourcequotas",
                ],
                "apps" => &["deployments", "statefulsets", "daemonsets", "replicasets"],
                "batch" => &["jobs", "cronjobs"],
                "networking.k8s.io" => &["ingresses", "networkpolicies"],
                "rbac.authorization.k8s.io" => &["roles", "rolebindings"],
                "policy" => &["poddisruptionbudgets"],
                _ => return Err("The Job asks for an unsupported reader API group".into()),
            };
            if rule
                .resources
                .iter()
                .any(|resource| !allowed.contains(&resource.as_str()))
            {
                return Err(
                    "The Job asks for a resource outside the non-secret reader allowlist".into(),
                );
            }
        }
    }
    Ok(())
}

fn build_job(
    app: &str,
    namespace: &str,
    run: &str,
    template: &JobTemplate,
    inputs: &BTreeMap<String, String>,
) -> Result<Job, String> {
    check_template(template)?;
    if !namespace_name(namespace) || !namespace_name(run) || run.len() > 40 {
        return Err("A Job needs a named namespace and valid run identity".into());
    }
    if inputs.len() != template.input_names.len()
        || template
            .input_names
            .iter()
            .any(|name| !inputs.contains_key(name))
        || inputs.values().any(|value| {
            value.is_empty()
                || value.len() > 512
                || !value.bytes().all(|b| (b' '..=b'~').contains(&b))
        })
        || inputs
            .get("namespace")
            .is_some_and(|value| value != namespace)
    {
        return Err("Job inputs must match the declared inputs and selected namespace".into());
    }
    let args: Vec<&str> = template
        .args
        .iter()
        .map(|arg| {
            placeholder(arg)
                .map(|name| inputs[name].as_str())
                .unwrap_or(arg)
        })
        .collect();
    let access = !template.read_rules.is_empty();
    let name = format!("srelens-run-{run}");
    let app_hash = format!("{:x}", Sha256::digest(app.as_bytes()));
    let mut volumes = vec![
        json!({"name":"data","emptyDir":{"sizeLimit":"16Gi"}}),
        json!({"name":"tmp","emptyDir":{"sizeLimit":"16Gi"}}),
    ];
    let mut mounts = vec![
        json!({"name":"data","mountPath":"/data"}),
        json!({"name":"tmp","mountPath":"/tmp"}),
    ];
    if access {
        volumes.push(json!({"name":"ready","configMap":{"name":name,"optional":false}}));
        mounts.push(json!({"name":"ready","mountPath":"/run/srelens-ready","readOnly":true}));
    }
    let mut pod = json!({
        "restartPolicy":"Never", "automountServiceAccountToken": access,
        "securityContext":{"runAsUser":10001,"runAsGroup":10001,"runAsNonRoot":true,"fsGroup":10001,"seccompProfile":{"type":"RuntimeDefault"}},
        "volumes":volumes,
        "containers":[{
            "name":"worker", "image":template.image, "imagePullPolicy":"IfNotPresent", "command":template.command, "args":args,
            "securityContext":{"runAsUser":10001,"runAsNonRoot":true,"allowPrivilegeEscalation":false,"readOnlyRootFilesystem":true,"capabilities":{"drop":["ALL"]}},
            "resources":{"requests":{"cpu":"100m","memory":"256Mi","ephemeral-storage":"2Gi"},"limits":{"cpu":"1","memory":"1Gi","ephemeral-storage":"16Gi"}},
            "volumeMounts":mounts,
            "env":[{"name":"TRIVY_CACHE_DIR","value":"/data/cache"},{"name":"TMPDIR","value":"/tmp"},{"name":"GOMEMLIMIT","value":"800MiB"},{"name":"GOMAXPROCS","value":"1"}]
        }]
    });
    if access {
        pod["serviceAccountName"] = json!(name);
    }
    serde_json::from_value(json!({
        "apiVersion":"batch/v1","kind":"Job",
        "metadata":{"generateName":"srelens-scan-","namespace":namespace,"labels":{"srelens.io/app":&app_hash[..32],"srelens.io/run":run},"annotations":{"srelens.io/app-id":app}},
        "spec":{"backoffLimit":0,"activeDeadlineSeconds":1200,"ttlSecondsAfterFinished":600,"template":{"metadata":{"labels":{"srelens.io/app":&app_hash[..32],"srelens.io/run":run}},"spec":pod}}
    })).map_err(|_| "The constrained Job template could not be built".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn template() -> JobTemplate {
        serde_json::from_value(json!({
            "image": format!("aquasec/trivy@sha256:{}", "a".repeat(64)),
            "command": ["trivy"],
            "args": ["k8s", "--include-namespaces", "${inputs.namespace}"],
            "inputNames": ["namespace"],
            "readRules": [{"apiGroups":["apps"], "resources":["deployments"], "verbs":["get","list"]}]
        })).unwrap()
    }

    #[test]
    fn worker_is_scoped_bounded_and_waits_for_reader_access() {
        let job = build_job(
            "org.srelens.trivy",
            "team",
            "run-1",
            &template(),
            &BTreeMap::from([("namespace".into(), "team".into())]),
        )
        .unwrap();
        let spec = job.spec.unwrap();
        assert_eq!(spec.active_deadline_seconds, Some(1200));
        assert_eq!(spec.ttl_seconds_after_finished, Some(600));
        assert_eq!(spec.backoff_limit, Some(0));
        assert_ne!(spec.suspend, Some(true));
        let pod = spec.template.spec.unwrap();
        assert_eq!(pod.restart_policy.as_deref(), Some("Never"));
        assert_ne!(pod.host_network, Some(true));
        assert_ne!(pod.host_pid, Some(true));
        assert_ne!(pod.host_ipc, Some(true));
        assert_eq!(
            pod.service_account_name.as_deref(),
            Some("srelens-run-run-1")
        );
        assert!(pod.volumes.as_ref().unwrap().iter().any(|v| v
            .config_map
            .as_ref()
            .is_some_and(|c| c.name == "srelens-run-run-1" && c.optional != Some(true))));
        let worker = &pod.containers[0];
        assert_eq!(worker.args.as_ref().unwrap().last().unwrap(), "team");
        let security = worker.security_context.as_ref().unwrap();
        assert_eq!(security.run_as_user, Some(10001));
        assert_eq!(security.allow_privilege_escalation, Some(false));
        assert_eq!(security.read_only_root_filesystem, Some(true));
        assert_eq!(
            security
                .capabilities
                .as_ref()
                .unwrap()
                .drop
                .as_ref()
                .unwrap(),
            &["ALL"]
        );
        assert_eq!(
            worker.resources.as_ref().unwrap().limits.as_ref().unwrap()["memory"].0,
            "1Gi"
        );
        assert_eq!(
            worker.resources.as_ref().unwrap().limits.as_ref().unwrap()["ephemeral-storage"].0,
            "16Gi"
        );
    }

    #[test]
    fn manual_image_worker_gets_no_cluster_token_or_reader_resources() {
        let mut spec = template();
        spec.read_rules.clear();
        let job = build_job(
            "org.srelens.trivy",
            "team",
            "run-1",
            &spec,
            &BTreeMap::from([("namespace".into(), "team".into())]),
        )
        .unwrap();
        let pod = job.spec.unwrap().template.spec.unwrap();
        assert_eq!(pod.automount_service_account_token, Some(false));
        assert!(pod.service_account_name.is_none());
    }

    #[test]
    fn namespace_arguments_cannot_redirect_a_scoped_job() {
        for namespace in ["", "other", "../team", "team\nother"] {
            assert!(build_job(
                "org.srelens.trivy",
                "team",
                "run-1",
                &template(),
                &BTreeMap::from([("namespace".into(), namespace.into())])
            )
            .is_err());
        }
        assert!(build_job(
            "org.srelens.trivy",
            "",
            "run-1",
            &template(),
            &BTreeMap::from([("namespace".into(), "team".into())])
        )
        .is_err());
    }

    #[test]
    fn secrets_wildcards_and_write_rbac_cannot_be_granted_to_a_worker() {
        for (group, resource, verb) in [
            ("", "secrets", "get"),
            ("", "serviceaccounts/token", "create"),
            ("apps", "*", "list"),
            ("*", "deployments", "list"),
            ("apps", "deployments", "patch"),
            ("", "nodes/proxy", "get"),
        ] {
            let mut spec = template();
            spec.read_rules = vec![serde_json::from_value(
                json!({"apiGroups":[group],"resources":[resource],"verbs":[verb]}),
            )
            .unwrap()];
            assert!(check_template(&spec).is_err(), "{group}/{resource}/{verb}");
        }
    }

    #[test]
    fn only_declared_whole_arguments_are_interpolated() {
        let mut spec = template();
        for arg in [
            "prefix-${inputs.namespace}",
            "${inputs.undeclared}",
            "${settings.token}",
        ] {
            spec.args = vec![arg.into()];
            assert!(check_template(&spec).is_err(), "{arg}");
        }
        let mut spec = template();
        spec.image = "aquasec/trivy:latest".into();
        assert!(check_template(&spec).is_err());
        let input = BTreeMap::from([
            ("namespace".into(), "team".into()),
            ("extra".into(), "value".into()),
        ]);
        assert!(build_job("org.srelens.trivy", "team", "run-1", &template(), &input).is_err());
    }

    #[test]
    fn job_binding_requires_an_executable_app_a_grant_and_fixed_arguments() {
        let mut value: serde_json::Value =
            serde_json::from_str(&super::super::tests::manifest()).unwrap();
        value["srelensApiVersion"] = json!("^0.8");
        value["kind"] = json!("executable");
        value["sidecar"] = json!({"binaries":{"darwin-arm64":"bin/darwin-arm64/controller"},"operations":[{"name":"scan","title":"Scan","inputs":[]}]});
        value["permissions"] = json!(["k8s.runJob"]);
        value["capabilities"] = json!([{"name":"scan-worker","title":"Scan worker","target":"k8s.runJob","inputs":[],"arguments":{
            "image":format!("aquasec/trivy@sha256:{}", "a".repeat(64)),"command":["trivy"],"args":["image","${inputs.image}"],"inputNames":["image"]
        }}]);
        value["contributions"] = json!({"pages":[],"detailTabs":[],"detailLinks":[]});
        let core = super::super::tests::fake_core();
        let manifest: srelens_plugin_host::Manifest =
            serde_json::from_value(value.clone()).unwrap();
        super::super::validate_app(&manifest, &["k8s.runJob".into()], core.clone()).unwrap();
        assert!(super::super::validate_app(&manifest, &[], core.clone()).is_err());
        for (key, bad) in [
            ("inputs", json!(["image"])),
            (
                "arguments",
                json!({"image":"aquasec/trivy:latest","command":["trivy"],"args":[]}),
            ),
        ] {
            let mut invalid = value.clone();
            invalid["capabilities"][0][key] = bad;
            assert!(super::super::validate_app(
                &serde_json::from_value(invalid).unwrap(),
                &["k8s.runJob".into()],
                core.clone()
            )
            .is_err());
        }
        value["kind"] = json!("declarative");
        value.as_object_mut().unwrap().remove("sidecar");
        assert!(super::super::validate_app(
            &serde_json::from_value(value).unwrap(),
            &["k8s.runJob".into()],
            core
        )
        .is_err());
    }

    #[test]
    fn a_report_requires_an_owned_successful_worker_and_cannot_follow_a_replacement() {
        let job: Job = serde_json::from_value(json!({"metadata":{"name":"scan","uid":"j-1"},"status":{"succeeded":1,"conditions":[{"type":"Complete","status":"True"}]}})).unwrap();
        let pod: srelens_kube::k8s_openapi::api::core::v1::Pod = serde_json::from_value(json!({"metadata":{"name":"worker","uid":"p-1","ownerReferences":[{"apiVersion":"batch/v1","kind":"Job","name":"scan","uid":"j-1","controller":true}]},"spec":{"containers":[{"name":"worker"}]},"status":{"phase":"Succeeded","containerStatuses":[{"name":"worker","image":"trivy","imageID":"sha256:a","ready":false,"restartCount":0,"state":{"terminated":{"exitCode":0,"reason":"Completed"}}}]}})).unwrap();
        assert_eq!(
            completed_worker(&job, "j-1", &[pod.clone()])
                .unwrap()
                .as_deref(),
            Some("worker")
        );
        assert!(completed_worker(&job, "replaced", &[pod.clone()]).is_err());
        let mut failed = pod.clone();
        failed
            .status
            .as_mut()
            .unwrap()
            .container_statuses
            .as_mut()
            .unwrap()[0]
            .state
            .as_mut()
            .unwrap()
            .terminated
            .as_mut()
            .unwrap()
            .exit_code = 137;
        assert!(completed_worker(&job, "j-1", &[failed]).is_err());
        let mut foreign = pod;
        foreign.metadata.owner_references.as_mut().unwrap()[0].uid = "other".into();
        assert!(completed_worker(&job, "j-1", &[foreign]).is_err());
    }

    #[tokio::test]
    async fn raw_result_is_bounded_and_cannot_overwrite_a_file_or_follow_a_link() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("job-result.json");
        assert!(
            write_result(&path, futures::io::Cursor::new(vec![b'x'; MAX_RESULT + 1]))
                .await
                .is_err()
        );
        assert!(!path.exists());
        assert_eq!(
            write_result(&path, futures::io::Cursor::new(b"{}".to_vec()))
                .await
                .unwrap(),
            2
        );
        assert!(
            write_result(&path, futures::io::Cursor::new(b"other".to_vec()))
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"{}");
        #[cfg(unix)]
        {
            let link = dir.path().join("linked.json");
            std::os::unix::fs::symlink(&path, &link).unwrap();
            assert!(
                write_result(&link, futures::io::Cursor::new(b"other".to_vec()))
                    .await
                    .is_err()
            );
            assert_eq!(std::fs::read(&path).unwrap(), b"{}");
        }
    }
    #[test]
    fn failed_unschedulable_job_retains_the_scheduling_reason() {
        let job: Job = serde_json::from_value(json!({"metadata":{"uid":"j-1"},"status":{"conditions":[{"type":"Failed","status":"True"}]}})).unwrap();
        let pod: Pod = serde_json::from_value(json!({"metadata":{"name":"worker","ownerReferences":[{"apiVersion":"batch/v1","kind":"Job","name":"scan","uid":"j-1","controller":true}]},"status":{"conditions":[{"type":"PodScheduled","status":"False","reason":"Unschedulable","message":"0/3 nodes available: Insufficient memory"}]}})).unwrap();
        let why = completed_worker(&job, "j-1", &[pod]).unwrap_err();
        assert!(why.contains("Insufficient memory"), "{why}");
    }
}

#[cfg(test)]
#[path = "jobs_tests.rs"]
mod lifecycle;
