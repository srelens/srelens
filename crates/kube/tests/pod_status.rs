//! `PodSummary::status`: the STATUS column `kubectl get pods` prints.
//!
//! Pods are written the way the API server sends them, so each case reads
//! like the `kubectl get pod -o json` it stands for, and the expected word is
//! the one kubectl prints for it. They go through `summarise_pod`, the same
//! summariser the pods watch and `k8s.listPods` use.

use serde_json::{json, Value};
use srelens_kube::workloads::summarise_pod;

fn status_of(pod: Value) -> String {
    summarise_pod(serde_json::from_value(pod).expect("a valid Pod")).status
}

/// A pod in `phase` whose containers report `statuses`.
fn pod_json(phase: &str, statuses: Vec<Value>) -> Value {
    let containers: Vec<Value> = statuses
        .iter()
        .map(|s| json!({ "name": s["name"] }))
        .collect();
    json!({
        "metadata": { "name": "web-1", "namespace": "default" },
        "spec": { "containers": containers },
        "status": { "phase": phase, "containerStatuses": statuses },
    })
}

/// A container status in `state`, with the fields the API always sends.
fn container(name: &str, ready: bool, state: Value) -> Value {
    json!({
        "name": name,
        "image": format!("{name}:1"),
        "imageID": "",
        "ready": ready,
        "restartCount": 0,
        "state": state,
    })
}

fn running() -> Value {
    json!({ "running": { "startedAt": "2026-10-03T10:00:00Z" } })
}

fn waiting(reason: &str) -> Value {
    json!({ "waiting": { "reason": reason } })
}

fn terminated(exit_code: i32, reason: Option<&str>) -> Value {
    json!({ "terminated": { "exitCode": exit_code, "reason": reason } })
}

fn condition(type_: &str, status: &str) -> Value {
    json!({ "type": type_, "status": status })
}

#[test]
fn status_shows_a_terminated_containers_reason_over_the_phase() {
    let pod = pod_json(
        "Running",
        vec![container("api", false, terminated(137, Some("OOMKilled")))],
    );
    assert_eq!(status_of(pod), "OOMKilled");
}

#[test]
fn status_shows_the_exit_code_or_signal_when_a_container_gives_no_reason() {
    let exited = pod_json(
        "Running",
        vec![container("api", false, terminated(1, None))],
    );
    assert_eq!(status_of(exited), "ExitCode:1");

    let mut signalled = pod_json(
        "Running",
        vec![container("api", false, terminated(137, None))],
    );
    signalled["status"]["containerStatuses"][0]["state"]["terminated"]["signal"] = json!(9);
    assert_eq!(status_of(signalled), "Signal:9");
}

#[test]
fn status_speaks_for_the_first_container_with_something_to_say() {
    // kubectl walks the containers last to first and keeps overwriting, so
    // the first one with a waiting or terminated reason is what it prints.
    let pod = pod_json(
        "Running",
        vec![
            container("app", false, terminated(1, Some("Error"))),
            container("sidecar", false, waiting("CrashLoopBackOff")),
        ],
    );
    assert_eq!(status_of(pod), "Error");
}

#[test]
fn a_finished_pod_reads_completed() {
    let pod = pod_json(
        "Succeeded",
        vec![container("job", false, terminated(0, Some("Completed")))],
    );
    assert_eq!(status_of(pod), "Completed");
}

#[test]
fn a_completed_container_beside_a_running_one_reads_running_or_not_ready() {
    // One container finished its work while another still serves: kubectl
    // does not call that pod Completed. It asks the Ready condition.
    let statuses = || {
        vec![
            container("setup", false, terminated(0, Some("Completed"))),
            container("server", true, running()),
        ]
    };
    let mut ready = pod_json("Running", statuses());
    ready["status"]["conditions"] = json!([condition("Ready", "True")]);
    assert_eq!(status_of(ready), "Running");

    let mut not_ready = pod_json("Running", statuses());
    not_ready["status"]["conditions"] = json!([condition("Ready", "False")]);
    assert_eq!(status_of(not_ready), "NotReady");
}

#[test]
fn a_completed_container_beside_a_failed_one_reads_the_failure() {
    // kubectl remembers the reason of the first container that exited
    // non-zero, and a pod that would otherwise read `Completed` reads that
    // instead, unless a container is still serving and the pod is Ready.
    let all_done = pod_json(
        "Running",
        vec![
            container("setup", false, terminated(0, Some("Completed"))),
            container("worker", false, terminated(1, Some("Error"))),
            container("cache", false, terminated(137, Some("OOMKilled"))),
        ],
    );
    assert_eq!(status_of(all_done), "Error");

    let mut one_serving = pod_json(
        "Running",
        vec![
            container("setup", false, terminated(0, Some("Completed"))),
            container("server", true, running()),
            container("worker", false, terminated(137, Some("OOMKilled"))),
        ],
    );
    one_serving["status"]["conditions"] = json!([condition("Ready", "False")]);
    assert_eq!(status_of(one_serving), "OOMKilled");
}

#[test]
fn a_failing_init_container_speaks_first_with_an_init_prefix() {
    let with_init = |init_state: Value| {
        let mut pod = pod_json(
            "Pending",
            vec![container("app", false, waiting("PodInitializing"))],
        );
        pod["spec"]["initContainers"] = json!([{ "name": "migrate" }]);
        pod["status"]["initContainerStatuses"] = json!([container("migrate", false, init_state)]);
        pod
    };
    assert_eq!(
        status_of(with_init(waiting("CrashLoopBackOff"))),
        "Init:CrashLoopBackOff"
    );
    assert_eq!(
        status_of(with_init(terminated(1, Some("Error")))),
        "Init:Error"
    );
    assert_eq!(status_of(with_init(terminated(2, None))), "Init:ExitCode:2");
}

#[test]
fn an_init_container_still_working_reads_as_progress() {
    let with_inits = |first: Value, second: Value| {
        let mut pod = pod_json(
            "Pending",
            vec![container("app", false, waiting("PodInitializing"))],
        );
        pod["spec"]["initContainers"] = json!([{ "name": "fetch" }, { "name": "migrate" }]);
        pod["status"]["initContainerStatuses"] = json!([
            container("fetch", false, first),
            container("migrate", false, second),
        ]);
        pod
    };
    assert_eq!(
        status_of(with_inits(running(), waiting("PodInitializing"))),
        "Init:0/2"
    );
    assert_eq!(
        status_of(with_inits(terminated(0, Some("Completed")), running())),
        "Init:1/2"
    );
}

#[test]
fn a_started_sidecar_does_not_hold_the_pod_in_init() {
    // A native sidecar is an init container with `restartPolicy: Always`.
    // Once started it runs for the pod's whole life, so kubectl steps past
    // it instead of reading it as an init container still working.
    let with_sidecar = |next_init: Option<Value>, app: Value| {
        let mut pod = pod_json("Running", vec![container("app", false, app)]);
        let mut specs = vec![json!({ "name": "proxy", "restartPolicy": "Always" })];
        let mut proxy = container("proxy", true, running());
        proxy["started"] = json!(true);
        let mut statuses = vec![proxy];
        if let Some(state) = next_init {
            specs.push(json!({ "name": "migrate" }));
            statuses.push(container("migrate", false, state));
        }
        pod["spec"]["initContainers"] = json!(specs);
        pod["status"]["initContainerStatuses"] = json!(statuses);
        pod
    };
    assert_eq!(
        status_of(with_sidecar(Some(running()), waiting("PodInitializing"))),
        "Init:1/2"
    );
    assert_eq!(status_of(with_sidecar(None, running())), "Running");
}

#[test]
fn an_initialized_pod_reads_its_containers_whatever_an_init_status_says() {
    // After a node restart an init container can report its old failure
    // while the pod is already Initialized; kubectl then reads the
    // regular containers, not the stale init status.
    let mut pod = pod_json(
        "Running",
        vec![container("app", false, waiting("CrashLoopBackOff"))],
    );
    pod["spec"]["initContainers"] = json!([{ "name": "migrate" }]);
    pod["status"]["initContainerStatuses"] =
        json!([container("migrate", false, terminated(1, Some("Error")))]);
    pod["status"]["conditions"] = json!([condition("Initialized", "True")]);
    assert_eq!(status_of(pod), "CrashLoopBackOff");
}

#[test]
fn a_pod_level_reason_replaces_the_phase() {
    let mut evicted = pod_json("Failed", vec![]);
    evicted["status"]["reason"] = json!("Evicted");
    assert_eq!(status_of(evicted), "Evicted");

    let mut gated = pod_json("Pending", vec![]);
    gated["status"]["conditions"] = json!([{
        "type": "PodScheduled",
        "status": "False",
        "reason": "SchedulingGated",
    }]);
    assert_eq!(status_of(gated), "SchedulingGated");
}

#[test]
fn a_pod_being_deleted_reads_terminating_unless_it_already_finished() {
    let deleting = |mut pod: Value| {
        pod["metadata"]["deletionTimestamp"] = json!("2026-10-03T10:05:00Z");
        pod
    };
    let serving = pod_json("Running", vec![container("api", true, running())]);
    assert_eq!(status_of(deleting(serving)), "Terminating");

    let finished = pod_json(
        "Succeeded",
        vec![container("job", false, terminated(0, Some("Completed")))],
    );
    assert_eq!(status_of(deleting(finished)), "Completed");

    let mut lost = pod_json("Running", vec![container("api", true, running())]);
    lost["status"]["reason"] = json!("NodeLost");
    assert_eq!(status_of(deleting(lost)), "Unknown");
}
