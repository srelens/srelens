//! The `k8s.runKubeBench` capability: run the CIS Kubernetes Benchmark
//! (aquasecurity/kube-bench) as a one-off Job and return its findings.
//!
//! The assistant has no shell, so it cannot run kube-bench itself. This does
//! what `kubectl apply -f job.yaml; kubectl logs job/kube-bench` does: create
//! the Job, wait for it, read its log, parse the JSON report and clean up. The
//! Job is a privileged-adjacent workload (`hostPID` and read-only host
//! mounts), which is why the capability is confirm-gated and High impact.

use std::sync::Arc;
use std::time::{Duration, Instant};

use k8s_openapi::api::batch::v1::Job;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{DeleteParams, ListParams, LogParams, PostParams, PropagationPolicy};
use kube::Api;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use srelens_capability::{Annotations, Capability, CapabilityError, Impact};

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;

/// The image kube-bench's own `job.yaml` uses.
pub const DEFAULT_IMAGE: &str = "docker.io/aquasec/kube-bench:latest";
/// How long the capability waits for the benchmark by default, and at most.
const DEFAULT_WAIT_SECS: u64 = 300;
const MAX_WAIT_SECS: u64 = 900;
/// Slack past the client's wait before the API server stops the Job itself.
const DEADLINE_MARGIN_SECS: u64 = 60;

/// How long the caller will wait, within bounds.
fn wait_secs(input: &RunKubeBenchIn) -> u64 {
    input.timeout_seconds.unwrap_or(DEFAULT_WAIT_SECS).clamp(10, MAX_WAIT_SECS)
}

/// Deletes the Job when dropped unless disarmed. Cancelling a scan aborts the
/// task at an await point, so code after `wait_and_read` never runs; a drop
/// guard still does.
struct JobCleanup {
    jobs: Api<Job>,
    name: String,
    armed: bool,
}

impl JobCleanup {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for JobCleanup {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        // Best effort: without a runtime (shutdown) the Job's deadline reaps it.
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let jobs = self.jobs.clone();
            let name = std::mem::take(&mut self.name);
            handle.spawn(async move {
                let dp = DeleteParams { propagation_policy: Some(PropagationPolicy::Background), ..Default::default() };
                let _ = tokio::time::timeout(request_timeout(), jobs.delete(&name, &dp)).await;
            });
        }
    }
}
/// A Job that outlives a crashed srectl removes itself after this long.
const JOB_TTL_SECS: i64 = 600;
/// Findings listed in full; the rest are counted.
const MAX_FAILURES: usize = 50;
const MAX_WARNINGS: usize = 20;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunKubeBenchIn {
    pub context: String,
    /// Namespace for the Job. Defaults to `default`.
    pub namespace: Option<String>,
    /// Run on this node (kube-bench checks the node it runs on). Default: the
    /// scheduler picks one.
    pub node: Option<String>,
    /// Comma-separated kube-bench targets, e.g. `node,policies`. Default: all
    /// the benchmark applies to the node.
    pub targets: Option<String>,
    /// kube-bench image. Defaults to the one kube-bench's own job.yaml uses.
    pub image: Option<String>,
    /// How long to wait for the benchmark, in seconds (default 300, max 900).
    #[serde(rename = "timeoutSeconds")]
    pub timeout_seconds: Option<u64>,
    /// Leave the Job in the cluster after reading it (default: delete it).
    #[serde(rename = "keepJob")]
    pub keep_job: Option<bool>,
}

/// One check from the report.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct CheckOut {
    /// The CIS check number, e.g. `1.2.1`.
    pub id: String,
    pub description: String,
    pub remediation: String,
    pub scored: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct KubeBenchOut {
    #[serde(rename = "jobName")]
    pub job_name: String,
    pub namespace: String,
    /// The benchmark version the report used, e.g. `cis-1.9`.
    pub benchmark: Option<String>,
    pub passed: u64,
    pub failed: u64,
    pub warned: u64,
    pub info: u64,
    /// Failed checks, up to 50.
    pub failures: Vec<CheckOut>,
    /// Checks that need a manual look, up to 20.
    pub warnings: Vec<CheckOut>,
    /// More failures or warnings were found than are listed here.
    pub truncated: bool,
    /// The Job was left in the cluster.
    #[serde(rename = "jobKept")]
    pub job_kept: bool,
}

/// The Job: kube-bench's own `job.yaml` (host PID namespace, the config and
/// data directories mounted read-only), with a TTL so it cleans itself up.
pub fn build_job(name: &str, namespace: &str, input: &RunKubeBenchIn) -> Result<Job, String> {
    let image = input.image.as_deref().unwrap_or(DEFAULT_IMAGE);
    let mut args = vec!["--json".to_string()];
    if let Some(targets) = input.targets.as_deref().filter(|t| !t.trim().is_empty()) {
        args.push("--targets".to_string());
        args.push(targets.trim().to_string());
    }

    // (volume name, host path, mount path). `usr-bin` is mounted where
    // kube-bench looks for the node's binaries.
    let mounts = [
        ("var-lib-etcd", "/var/lib/etcd", "/var/lib/etcd"),
        ("var-lib-kubelet", "/var/lib/kubelet", "/var/lib/kubelet"),
        ("var-lib-kube-scheduler", "/var/lib/kube-scheduler", "/var/lib/kube-scheduler"),
        (
            "var-lib-kube-controller-manager",
            "/var/lib/kube-controller-manager",
            "/var/lib/kube-controller-manager",
        ),
        ("etc-systemd", "/etc/systemd", "/etc/systemd"),
        ("lib-systemd", "/lib/systemd", "/lib/systemd"),
        ("srv-kubernetes", "/srv/kubernetes", "/srv/kubernetes"),
        ("etc-kubernetes", "/etc/kubernetes", "/etc/kubernetes"),
        ("usr-bin", "/usr/bin", "/usr/local/mount-from-host/bin"),
        ("etc-cni-netd", "/etc/cni/net.d", "/etc/cni/net.d"),
        ("opt-cni-bin", "/opt/cni/bin", "/opt/cni/bin"),
    ];
    let volume_mounts: Vec<Value> = mounts
        .iter()
        .map(|(vol, _, mount)| json!({ "name": vol, "mountPath": mount, "readOnly": true }))
        .collect();
    let volumes: Vec<Value> = mounts
        .iter()
        .map(|(vol, host, _)| json!({ "name": vol, "hostPath": { "path": host } }))
        .collect();

    let mut pod_spec = json!({
        "hostPID": true,
        "restartPolicy": "Never",
        "tolerations": [
            { "key": "node-role.kubernetes.io/control-plane", "operator": "Exists", "effect": "NoSchedule" },
            { "key": "node-role.kubernetes.io/master", "operator": "Exists", "effect": "NoSchedule" }
        ],
        "containers": [{
            "name": "kube-bench",
            "image": image,
            "command": ["kube-bench"],
            "args": args,
            "volumeMounts": volume_mounts,
        }],
        "volumes": volumes,
    });
    if let Some(node) = input.node.as_deref().filter(|n| !n.trim().is_empty()) {
        pod_spec["nodeName"] = json!(node.trim());
    }

    serde_json::from_value(json!({
        "apiVersion": "batch/v1",
        "kind": "Job",
        "metadata": {
            "name": name,
            "namespace": namespace,
            "labels": { "app.kubernetes.io/name": "kube-bench", "app.kubernetes.io/managed-by": "srelens" }
        },
        "spec": {
            "backoffLimit": 0,
            "ttlSecondsAfterFinished": JOB_TTL_SECS,
            // Server-side stop: a Job stuck pulling its image or waiting to be
            // scheduled never "finishes", so the TTL alone would never reap it,
            // and a cancelled client cannot clean up after itself.
            "activeDeadlineSeconds": wait_secs(input) + DEADLINE_MARGIN_SECS,
            "template": {
                "metadata": { "labels": { "app.kubernetes.io/name": "kube-bench" } },
                "spec": pod_spec
            }
        }
    }))
    .map_err(|e| format!("could not build the kube-bench Job: {e}"))
}

/// Where a Job stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobState {
    Running,
    Succeeded,
    Failed(String),
}

pub fn job_state(job: &Job) -> JobState {
    let status = match &job.status {
        Some(s) => s,
        None => return JobState::Running,
    };
    if status.succeeded.unwrap_or(0) > 0 {
        return JobState::Succeeded;
    }
    if status.failed.unwrap_or(0) > 0 {
        let why = status
            .conditions
            .as_ref()
            .and_then(|cs| cs.iter().find(|c| c.type_ == "Failed"))
            .and_then(|c| c.message.clone().or_else(|| c.reason.clone()))
            .unwrap_or_else(|| "the Job failed".to_string());
        return JobState::Failed(why);
    }
    JobState::Running
}

fn count(v: &Value, key: &str) -> Option<u64> {
    v.get(key).and_then(Value::as_u64)
}

fn check_of(result: &Value) -> CheckOut {
    let text = |k: &str| result.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
    CheckOut {
        id: text("test_number"),
        description: text("test_desc"),
        remediation: text("remediation"),
        scored: result.get("scored").and_then(Value::as_bool).unwrap_or(true),
    }
}

/// Parse kube-bench's `--json` output. It may be preceded by log lines, may be
/// an object with `Controls` and `Totals` or a bare list of controls, and its
/// totals may be absent (older versions), in which case they are counted.
pub fn parse_report(output: &str) -> Result<ParsedReport, String> {
    // Log lines come first and look like "[INFO] ...", so the first bracket is
    // not necessarily the report. Try each line that opens a JSON value until
    // one parses.
    let mut starts = Vec::new();
    let mut at = 0;
    for line in output.split_inclusive('\n') {
        if line.trim_start().starts_with(['{', '[']) {
            starts.push(at + (line.len() - line.trim_start().len()));
        }
        at += line.len();
    }
    if starts.is_empty() {
        return Err(format!("kube-bench printed no JSON report. It said: {}", excerpt(output)));
    }
    let mut last_error = String::new();
    let mut parsed: Option<Value> = None;
    for start in starts {
        match serde_json::Deserializer::from_str(&output[start..]).into_iter::<Value>().next() {
            Some(Ok(v)) => {
                parsed = Some(v);
                break;
            }
            Some(Err(e)) => last_error = e.to_string(),
            None => {}
        }
    }
    let value = parsed.ok_or_else(|| {
        format!("kube-bench's report was not valid JSON ({last_error}). It said: {}", excerpt(output))
    })?;

    let controls: &[Value] = match &value {
        Value::Array(a) => a,
        Value::Object(o) => o.get("Controls").and_then(Value::as_array).map_or(&[], Vec::as_slice),
        _ => &[],
    };
    if controls.is_empty() {
        return Err(format!(
            "kube-bench's report had no checks, usually because no benchmark applies to this node. It said: {}",
            excerpt(output)
        ));
    }

    let mut failures = Vec::new();
    let mut warnings = Vec::new();
    let (mut failed_total, mut warned_total) = (0u64, 0u64);
    let (mut pass, mut fail, mut warn, mut info) = (0u64, 0u64, 0u64, 0u64);
    for control in controls {
        for test in control.get("tests").and_then(Value::as_array).into_iter().flatten() {
            for result in test.get("results").and_then(Value::as_array).into_iter().flatten() {
                match result.get("status").and_then(Value::as_str) {
                    Some("PASS") => pass += 1,
                    Some("FAIL") => {
                        fail += 1;
                        failed_total += 1;
                        if failures.len() < MAX_FAILURES {
                            failures.push(check_of(result));
                        }
                    }
                    Some("WARN") => {
                        warn += 1;
                        warned_total += 1;
                        if warnings.len() < MAX_WARNINGS {
                            warnings.push(check_of(result));
                        }
                    }
                    Some("INFO") => info += 1,
                    _ => {}
                }
            }
        }
    }

    // kube-bench's own totals win when it printed them.
    let totals = value.get("Totals");
    let pick = |key: &str, counted: u64| totals.and_then(|t| count(t, key)).unwrap_or(counted);
    Ok(ParsedReport {
        benchmark: controls
            .first()
            .and_then(|c| c.get("version"))
            .and_then(Value::as_str)
            .map(String::from),
        passed: pick("total_pass", pass),
        failed: pick("total_fail", fail),
        warned: pick("total_warn", warn),
        info: pick("total_info", info),
        truncated: failed_total as usize > failures.len() || warned_total as usize > warnings.len(),
        failures,
        warnings,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedReport {
    pub benchmark: Option<String>,
    pub passed: u64,
    pub failed: u64,
    pub warned: u64,
    pub info: u64,
    pub failures: Vec<CheckOut>,
    pub warnings: Vec<CheckOut>,
    pub truncated: bool,
}

/// The start of `text`, for an error the caller has to act on.
fn excerpt(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return "(nothing)".to_string();
    }
    let cut: String = trimmed.chars().take(300).collect();
    if cut.len() < trimmed.len() {
        format!("{cut}...")
    } else {
        cut
    }
}

fn unique_name() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("kube-bench-{secs:x}")
}

/// Why a pod that never finished is stuck, from its container states.
fn stuck_reason(pod: &Pod) -> Option<String> {
    let status = pod.status.as_ref()?;
    for c in status.container_statuses.iter().flatten() {
        if let Some(w) = c.state.as_ref().and_then(|s| s.waiting.as_ref()) {
            let reason = w.reason.clone().unwrap_or_default();
            return Some(match &w.message {
                Some(m) if !m.is_empty() => format!("{reason}: {m}"),
                _ => reason,
            });
        }
    }
    status.phase.clone()
}

/// `k8s.runKubeBench` — run the CIS benchmark on a node as a Job and return the
/// report's findings. Creates a Job, so it requires confirmation.
pub fn run_kube_bench_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<RunKubeBenchIn, KubeBenchOut, _, _>(
        "k8s.runKubeBench",
        "run the CIS Kubernetes Benchmark (kube-bench) on a node as a one-off Job, \
         wait for it, and return the failed and manual-review checks with their \
         remediation",
        Annotations::MUTATING
            .with_impact(Impact::High)
            .with_confirm("Run kube-bench, a Job with host PID access[ in namespace {namespace}][ in cluster {cluster}]?"),
        move |input: RunKubeBenchIn| {
            let cache = cache.clone();
            async move { run(cache, input).await }
        },
    )
}

async fn run(cache: Arc<ClientCache>, input: RunKubeBenchIn) -> Result<KubeBenchOut, CapabilityError> {
    let handler = |e: String| CapabilityError::Handler(e);
    let namespace = input.namespace.clone().unwrap_or_else(|| "default".to_string());
    let name = unique_name();
    let job = build_job(&name, &namespace, &input).map_err(handler)?;
    let keep = input.keep_job.unwrap_or(false);
    let wait = Duration::from_secs(wait_secs(&input));

    let client = cache.get(&input.context).await.map_err(handler)?;
    let jobs: Api<Job> = crate::scoped_api(client.clone(), &namespace);
    let pods: Api<Pod> = crate::scoped_api(client, &namespace);

    tokio::time::timeout(request_timeout(), jobs.create(&PostParams::default(), &job))
        .await
        .map_err(|_| handler("creating the kube-bench Job timed out".into()))?
        .map_err(|e| handler(format!("creating the kube-bench Job failed: {e}")))?;

    // From here on the Job exists: clean it up however this ends, including
    // this future being dropped mid-wait by a cancel or the watchdog.
    let mut cleanup = JobCleanup { jobs: jobs.clone(), name: name.clone(), armed: !keep };
    let outcome = wait_and_read(&jobs, &pods, &name, wait).await;
    if !keep {
        let dp = DeleteParams { propagation_policy: Some(PropagationPolicy::Background), ..Default::default() };
        let _ = tokio::time::timeout(request_timeout(), jobs.delete(&name, &dp)).await;
    }
    cleanup.disarm();
    let logs = outcome.map_err(handler)?;
    let report = parse_report(&logs).map_err(handler)?;
    Ok(KubeBenchOut {
        job_name: name,
        namespace,
        benchmark: report.benchmark,
        passed: report.passed,
        failed: report.failed,
        warned: report.warned,
        info: report.info,
        failures: report.failures,
        warnings: report.warnings,
        truncated: report.truncated,
        job_kept: keep,
    })
}

/// Wait for the Job to end and return its pod's log. Errors say where it got
/// stuck, since "timed out" alone tells the caller nothing.
async fn wait_and_read(
    jobs: &Api<Job>,
    pods: &Api<Pod>,
    name: &str,
    wait: Duration,
) -> Result<String, String> {
    let selector = ListParams::default().labels(&format!("job-name={name}"));
    let deadline = Instant::now() + wait;
    loop {
        let job = tokio::time::timeout(request_timeout(), jobs.get(name))
            .await
            .map_err(|_| "reading the kube-bench Job timed out".to_string())?
            .map_err(|e| format!("reading the kube-bench Job failed: {e}"))?;
        let state = job_state(&job);
        let listed = tokio::time::timeout(request_timeout(), pods.list(&selector))
            .await
            .map_err(|_| "listing the kube-bench pod timed out".to_string())?
            .map_err(|e| format!("listing the kube-bench pod failed: {e}"))?;
        let pod = listed.items.into_iter().next();

        match state {
            JobState::Succeeded | JobState::Failed(_) => {
                let pod_name = pod
                    .and_then(|p| p.metadata.name)
                    .ok_or_else(|| "the kube-bench pod is gone, so there is no report to read".to_string())?;
                let logs = tokio::time::timeout(request_timeout(), pods.logs(&pod_name, &LogParams::default()))
                    .await
                    .map_err(|_| "reading kube-bench's log timed out".to_string())?
                    .map_err(|e| format!("reading kube-bench's log failed: {e}"))?;
                return match state {
                    JobState::Failed(why) if logs.trim().is_empty() => {
                        Err(format!("kube-bench failed and printed nothing: {why}"))
                    }
                    _ => Ok(logs),
                };
            }
            JobState::Running => {}
        }
        if Instant::now() >= deadline {
            let stuck = pod
                .as_ref()
                .and_then(stuck_reason)
                .unwrap_or_else(|| "no pod was created for it".to_string());
            return Err(format!(
                "kube-bench did not finish within {}s. The pod is: {stuck}",
                wait.as_secs()
            ));
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn input() -> RunKubeBenchIn {
        serde_json::from_value(json!({ "context": "kind-x" })).unwrap()
    }

    fn job_json(job: &Job) -> Value {
        serde_json::to_value(job).unwrap()
    }

    #[test]
    fn the_job_carries_a_server_side_deadline_past_the_clients_wait() {
        let v = job_json(&build_job("j", "ns", &input()).unwrap());
        assert_eq!(v["spec"]["activeDeadlineSeconds"], DEFAULT_WAIT_SECS + DEADLINE_MARGIN_SECS);
        let mut i = input();
        i.timeout_seconds = Some(100_000);
        let v = job_json(&build_job("j", "ns", &i).unwrap());
        assert_eq!(v["spec"]["activeDeadlineSeconds"], MAX_WAIT_SECS + DEADLINE_MARGIN_SECS);
    }

    fn recording_jobs() -> (Api<Job>, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let captured = seen.clone();
        let service = tower::service_fn(move |req: http::Request<kube::client::Body>| {
            let captured = captured.clone();
            async move {
                captured.lock().unwrap().push(format!("{} {}", req.method(), req.uri().path()));
                Ok::<_, std::convert::Infallible>(
                    http::Response::builder()
                        .status(200)
                        .header("content-type", "application/json")
                        .body(kube::client::Body::from(b"{}".to_vec()))
                        .unwrap(),
                )
            }
        });
        let client = kube::Client::new(service, "default");
        (Api::namespaced(client, "ns"), seen)
    }

    #[tokio::test]
    async fn a_dropped_scan_still_deletes_its_job() {
        let (jobs, seen) = recording_jobs();
        let guard = JobCleanup { jobs, name: "kube-bench-x".into(), armed: true };
        drop(guard); // what aborting the task does to the future that holds it
        for _ in 0..50 {
            if !seen.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let calls = seen.lock().unwrap().clone();
        assert_eq!(calls.len(), 1, "{calls:?}");
        assert!(calls[0].starts_with("DELETE ") && calls[0].ends_with("/jobs/kube-bench-x"), "{calls:?}");
    }

    #[tokio::test]
    async fn a_finished_scan_does_not_delete_twice_or_when_the_job_is_kept() {
        let (jobs, seen) = recording_jobs();
        let mut done = JobCleanup { jobs: jobs.clone(), name: "a".into(), armed: true };
        done.disarm();
        drop(done);
        drop(JobCleanup { jobs, name: "b".into(), armed: false });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(seen.lock().unwrap().is_empty());
    }

    #[test]
    fn the_job_shares_the_host_pid_namespace_and_mounts_the_host_read_only() {
        let job = build_job("kube-bench-1", "default", &input()).unwrap();
        let v = job_json(&job);
        let spec = &v["spec"]["template"]["spec"];
        assert_eq!(spec["hostPID"], true);
        assert_eq!(spec["restartPolicy"], "Never");
        let mounts = spec["containers"][0]["volumeMounts"].as_array().unwrap();
        assert!(mounts.len() >= 8);
        assert!(
            mounts.iter().all(|m| m["readOnly"] == true),
            "a benchmark must never be able to write to the host"
        );
        assert_eq!(v["spec"]["backoffLimit"], 0);
        assert_eq!(v["spec"]["ttlSecondsAfterFinished"], JOB_TTL_SECS);
        assert_eq!(v["metadata"]["name"], "kube-bench-1");
    }

    #[test]
    fn the_job_is_privileged_in_no_other_way() {
        let v = job_json(&build_job("j", "default", &input()).unwrap());
        let c = &v["spec"]["template"]["spec"]["containers"][0];
        assert!(c.get("securityContext").is_none(), "no privileged flag, no added capabilities");
        let spec = &v["spec"]["template"]["spec"];
        assert!(spec.get("hostNetwork").is_none() && spec.get("hostIPC").is_none());
    }

    #[test]
    fn image_node_and_targets_are_honoured() {
        let mut i = input();
        i.image = Some("registry.example/kube-bench:v1".into());
        i.node = Some("worker-2".into());
        i.targets = Some("node,policies".into());
        let v = job_json(&build_job("j", "ns", &i).unwrap());
        let spec = &v["spec"]["template"]["spec"];
        assert_eq!(spec["containers"][0]["image"], "registry.example/kube-bench:v1");
        assert_eq!(spec["nodeName"], "worker-2");
        assert_eq!(spec["containers"][0]["args"], json!(["--json", "--targets", "node,policies"]));
    }

    #[test]
    fn defaults_use_the_official_image_json_output_and_no_node_pin() {
        let v = job_json(&build_job("j", "ns", &input()).unwrap());
        let spec = &v["spec"]["template"]["spec"];
        assert_eq!(spec["containers"][0]["image"], DEFAULT_IMAGE);
        assert_eq!(spec["containers"][0]["args"], json!(["--json"]));
        assert!(spec.get("nodeName").is_none());
    }

    #[test]
    fn blank_optional_fields_are_ignored() {
        let mut i = input();
        i.node = Some("  ".into());
        i.targets = Some("".into());
        let v = job_json(&build_job("j", "ns", &i).unwrap());
        let spec = &v["spec"]["template"]["spec"];
        assert!(spec.get("nodeName").is_none());
        assert_eq!(spec["containers"][0]["args"], json!(["--json"]));
    }

    #[test]
    fn the_input_is_read_in_the_callers_spelling_and_the_wrong_one_is_refused() {
        let ok: RunKubeBenchIn = serde_json::from_value(json!({
            "context": "c", "timeoutSeconds": 120, "keepJob": true
        }))
        .unwrap();
        assert_eq!(ok.timeout_seconds, Some(120));
        assert_eq!(ok.keep_job, Some(true));
        for wrong in [json!({"context": "c", "timeout_seconds": 1}), json!({"context": "c", "keep_job": true})] {
            assert!(serde_json::from_value::<RunKubeBenchIn>(wrong).is_err());
        }
    }

    #[test]
    fn it_is_a_confirm_gated_high_impact_write() {
        let cap = run_kube_bench_capability(ClientCache::new(PathBuf::from("/x")));
        assert_eq!(cap.id, "k8s.runKubeBench");
        assert!(cap.annotations.requires_confirm);
        assert!(!cap.annotations.read_only);
        assert_eq!(cap.annotations.impact, Impact::High);
    }

    fn report() -> Value {
        json!({
            "Controls": [{
                "id": "4", "version": "cis-1.9", "text": "Worker Node Security", "node_type": "node",
                "tests": [{
                    "section": "4.1", "desc": "Worker Node Configuration Files",
                    "results": [
                        {"test_number": "4.1.1", "test_desc": "Ensure the kubelet service file permissions are 600", "status": "PASS", "scored": true, "remediation": ""},
                        {"test_number": "4.1.2", "test_desc": "Ensure kubelet service file ownership is root:root", "status": "FAIL", "scored": true, "remediation": "Run chown root:root /etc/systemd/system/kubelet.service.d/10-kubeadm.conf"},
                        {"test_number": "4.2.9", "test_desc": "Ensure that the --event-qps argument is set", "status": "WARN", "scored": false, "remediation": "Review event rate limiting"},
                        {"test_number": "4.2.99", "test_desc": "informational", "status": "INFO", "scored": false, "remediation": ""}
                    ]
                }]
            }],
            "Totals": {"total_pass": 1, "total_fail": 1, "total_warn": 1, "total_info": 1}
        })
    }

    #[test]
    fn a_report_is_summarised_with_the_failures_and_their_remediation() {
        let parsed = parse_report(&report().to_string()).unwrap();
        assert_eq!((parsed.passed, parsed.failed, parsed.warned, parsed.info), (1, 1, 1, 1));
        assert_eq!(parsed.benchmark.as_deref(), Some("cis-1.9"));
        assert_eq!(parsed.failures.len(), 1);
        assert_eq!(parsed.failures[0].id, "4.1.2");
        assert!(parsed.failures[0].remediation.contains("chown root:root"));
        assert_eq!(parsed.warnings[0].id, "4.2.9");
        assert!(!parsed.warnings[0].scored);
        assert!(!parsed.truncated);
    }

    #[test]
    fn log_lines_before_the_json_are_skipped() {
        let out = format!("[INFO] using benchmark cis-1.9\n[WARN] something\n{}\n", report());
        assert_eq!(parse_report(&out).unwrap().failed, 1);
    }

    #[test]
    fn a_bare_list_of_controls_and_missing_totals_are_counted() {
        let list = json!(report()["Controls"]);
        let parsed = parse_report(&list.to_string()).unwrap();
        assert_eq!((parsed.passed, parsed.failed, parsed.warned, parsed.info), (1, 1, 1, 1));
    }

    #[test]
    fn listed_findings_are_capped_but_the_counts_are_not() {
        let results: Vec<Value> = (0..80)
            .map(|i| json!({"test_number": format!("1.{i}"), "test_desc": "d", "status": "FAIL", "scored": true, "remediation": "r"}))
            .collect();
        let big = json!({"Controls": [{"version": "v", "tests": [{"results": results}]}]});
        let parsed = parse_report(&big.to_string()).unwrap();
        assert_eq!(parsed.failed, 80);
        assert_eq!(parsed.failures.len(), MAX_FAILURES);
        assert!(parsed.truncated);
    }

    #[test]
    fn output_that_is_not_a_report_is_an_error_that_shows_what_it_said() {
        let err = parse_report("error: unable to determine benchmark version").unwrap_err();
        assert!(err.contains("no JSON") && err.contains("unable to determine benchmark"), "{err}");
        let err = parse_report("").unwrap_err();
        assert!(err.contains("(nothing)"), "{err}");
        let err = parse_report("{\"Controls\": []}").unwrap_err();
        assert!(err.contains("no checks"), "{err}");
        let err = parse_report("{ not json").unwrap_err();
        assert!(err.contains("not valid JSON"), "{err}");
    }

    fn job_with(status: Value) -> Job {
        serde_json::from_value(json!({
            "apiVersion": "batch/v1", "kind": "Job",
            "metadata": {"name": "j"}, "spec": {"template": {"spec": {"containers": []}}},
            "status": status
        }))
        .unwrap()
    }

    #[test]
    fn a_job_is_running_until_it_succeeds_or_fails() {
        assert_eq!(job_state(&job_with(json!({}))), JobState::Running);
        assert_eq!(job_state(&job_with(json!({"active": 1}))), JobState::Running);
        assert_eq!(job_state(&job_with(json!({"succeeded": 1}))), JobState::Succeeded);
        match job_state(&job_with(json!({
            "failed": 1,
            "conditions": [{"type": "Failed", "status": "True", "message": "BackoffLimitExceeded"}]
        }))) {
            JobState::Failed(why) => assert_eq!(why, "BackoffLimitExceeded"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn names_are_unique_per_second_and_valid() {
        let n = unique_name();
        assert!(n.starts_with("kube-bench-") && n.len() <= 63);
        assert!(n.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'));
    }
}
