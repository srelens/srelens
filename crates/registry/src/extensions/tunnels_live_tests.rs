//! A pod tunnel against a real cluster, from a real sidecar process (#1091).

use super::sidecars::SidecarHost;
use super::tunnels::PodTunnels;
use serde_json::{json, Value};
use srelens_capability::audit::{AuditSink, JsonlAuditLog, Source};
use srelens_plugin_host::sidecar::{
    Enforcement, Exit, LaunchError, Launched, Launcher, Limits, NoConsent, SidecarCommand,
};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

/// Starts the sidecar with no sandbox, as the hello-world `supervised` test
/// does: it claims the kernel enforces limits it does not set, so that the
/// supervisor runs it.
struct Unconfined;

impl Launcher for Unconfined {
    fn enforcement(&self) -> Enforcement {
        Enforcement::Kernel
    }

    fn launch(&self, command: &SidecarCommand, _limits: &Limits) -> Result<Launched, LaunchError> {
        let child = tokio::process::Command::new(&command.program)
            .args(&command.args)
            .env_clear()
            .envs(command.env.iter().map(|(k, v)| (k, v)))
            .current_dir(&command.data_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| LaunchError::Failed(e.to_string()))?;
        Launched::from_child(child, Exit::from_status)
    }
}

fn var(name: &str, how: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("set {name}: {how}"))
}

/// The grant each of the manifest's permissions asks for.
fn grants(manifest: &Value) -> Vec<String> {
    manifest["permissions"]
        .as_array()
        .expect("the manifest's permissions")
        .iter()
        .map(|p| p.as_str().or(p["capability"].as_str()).unwrap().to_owned())
        .collect()
}

/// The `k8s.connectPod` records the audit log holds.
fn connects(audit: &JsonlAuditLog) -> Vec<Value> {
    match audit.retained() {
        Ok(retained) => retained.entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => panic!("the audit log: {e}"),
    }
    .into_iter()
    .filter(|r| r["tool"] == "k8s.connectPod")
    .collect()
}

/// An executable app's pod tunnels end to end, against a real cluster: the
/// manifest is installed with the given binary in a temporary inventory,
/// granted exactly what it asks for and enabled for the context; each call
/// goes through `extensions.callOperation`, as the desktop runs an app
/// operation; the sidecar dials through the production `PodTunnels` over the
/// real port-forward, audited to a temporary JSONL log. The kubeconfig the
/// environment names is only read. The sidecar runs under the real supervisor
/// WITHOUT the OS sandbox (see `Unconfined`).
///
/// - `SRELENS_LIVE_CONTEXT`: the kube context.
/// - `SRELENS_LIVE_SIDECAR`: the sidecar executable, an absolute path.
/// - `SRELENS_LIVE_MANIFEST`: its `extension.json` (default: the Go pod-connect example's).
/// - `SRELENS_LIVE_CALLS`: a JSON array of `{"operation", "input", "expect"}` or
///   `{"operation", "input", "expectError"}`, each a substring of the result's JSON
///   or of the error message.
///
/// `SRELENS_LIVE_CONTEXT=kind-x SRELENS_LIVE_SIDECAR=… SRELENS_LIVE_CALLS='[…]' cargo test -p srelens-registry --lib live_pod_connect -- --ignored --nocapture`
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a live cluster (SRELENS_LIVE_CONTEXT), a sidecar (SRELENS_LIVE_SIDECAR) and SRELENS_LIVE_CALLS"]
async fn live_pod_connect() {
    let context = var("SRELENS_LIVE_CONTEXT", "the kube context to dial pods in");
    let sidecar = var(
        "SRELENS_LIVE_SIDECAR",
        "the sidecar executable, an absolute path",
    );
    let manifest_path = std::env::var("SRELENS_LIVE_MANIFEST")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../sdk/examples/pod-connect/go/extension.json")
        });
    let calls: Vec<Value> =
        serde_json::from_str(&var("SRELENS_LIVE_CALLS", "a JSON array of calls"))
            .expect("SRELENS_LIVE_CALLS is a JSON array");
    let manifest: Value = serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    let id = manifest["id"].as_str().unwrap().to_owned();

    // The desktop's registry over the real kubeconfig, read only, and a
    // settings path under `dir`, where everything written goes.
    let dir = tempfile::tempdir().unwrap();
    let paths = crate::default_kubeconfig_paths();
    let cache = srelens_kube::client_cache::ClientCache::new_many(paths.clone());
    let (reg, streams) = crate::build_registry_and_app_streams(
        cache,
        paths.clone(),
        Some(dir.path().join("settings.json")),
    );
    let streams = streams.expect("the app streams");
    let audit = Arc::new(JsonlAuditLog::new(dir.path().join("audit.jsonl"), u64::MAX));

    // The package: the given binary for this platform, a stand-in for the rest.
    let platform = srelens_plugin_host::host_platform().expect("a sidecar platform");
    let program = std::fs::read(&sidecar).unwrap_or_else(|e| panic!("{sidecar}: {e}"));
    let binaries: Vec<(String, Vec<u8>)> = manifest["sidecar"]["binaries"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(on, at)| {
            let bytes = if on == platform {
                program.clone()
            } else {
                b"#!/bin/false\n".to_vec()
            };
            (at.as_str().unwrap().to_owned(), bytes)
        })
        .collect();
    let binaries: Vec<(&str, &[u8])> = binaries
        .iter()
        .map(|(p, b)| (p.as_str(), b.as_slice()))
        .collect();
    let archive = super::executable_tests::package_of(&manifest, &binaries);
    let key = srelens_kube::context_resolve::find_context(
        &srelens_kube::context_resolve::resolve_contexts(&paths),
        &context,
    )
    .unwrap_or_else(|| panic!("no context {context} in {paths:?}"))
    .key();
    let configure = |input: Value| {
        let reg = &reg;
        async move {
            reg.invoke("extensions.configure", input.clone())
                .await
                .unwrap_or_else(|e| panic!("{}: {e}", input["action"]))
        }
    };
    configure(json!({"action": "unsignedApps", "allowUnsignedApps": true})).await;
    configure(
        json!({"action": "installPackage", "grants": grants(&manifest),
        "package": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, archive)}),
    )
    .await;
    let state = configure(json!({"action": "clusters", "id": id, "contexts": [key]})).await;
    let revision = state["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|app| app["manifest"]["id"] == id)
        .expect("the installed app")["revision"]
        .as_u64()
        .unwrap();

    // The desktop's sidecar host, with the production pod connector.
    let tools = streams.app_tools();
    tools.script_sidecars(Arc::new(Unconfined));
    tools.serve_sidecars(SidecarHost {
        registry: Arc::new(reg.clone()),
        consent: Arc::new(NoConsent),
        audit: audit.clone(),
        pods: Arc::new(PodTunnels::new(&streams, audit.clone())),
    });

    let succeeds = calls.iter().any(|call| call["expect"].is_string());
    let mut failures = Vec::new();
    for call in &calls {
        let before = connects(&audit).len();
        let started = std::time::Instant::now();
        let input = json!({"id": id, "revision": revision, "context": context,
            "operation": call["operation"], "params": call["input"]});
        let answer = tokio::time::timeout(
            Duration::from_secs(120),
            reg.invoke_audited(
                "extensions.callOperation",
                input,
                audit.as_ref(),
                Source::Ui,
                "auto",
            ),
        )
        .await
        .expect("the call finished within two minutes");
        let records: Vec<Value> = connects(&audit).split_off(before);
        println!(
            "call {call}\n  took {:?}\n  answer {answer:?}",
            started.elapsed()
        );
        for record in &records {
            println!(
                "  audit outcome={} resource={} args={} cluster={} err={}",
                record["outcome"],
                record["resource"],
                record["args"],
                record["cluster"],
                record["err"]
            );
        }
        let ok = records.iter().filter(|r| r["outcome"] == "ok").count();
        let rejected = records
            .iter()
            .filter(|r| r["outcome"] == "rejected")
            .count();
        match (&call["expect"], &call["expectError"], &answer) {
            (Value::String(want), _, Ok(result)) => {
                let got = result.to_string();
                if !got.contains(want.as_str()) {
                    failures.push(format!("{call}: {got} does not contain {want:?}"));
                }
                if ok == 0 {
                    failures.push(format!("{call}: no ok k8s.connectPod record"));
                }
            }
            (_, Value::String(want), Err(error)) => {
                let got = error.to_string();
                if !got.contains(want.as_str()) {
                    failures.push(format!("{call}: {got:?} does not contain {want:?}"));
                }
                if ok != 0 || rejected != 1 {
                    failures.push(format!(
                        "{call}: {ok} ok and {rejected} rejected records, not one refusal"
                    ));
                }
            }
            _ => failures.push(format!("{call}: answered {answer:?}")),
        }
    }
    // Stop the sidecar before its directory goes.
    configure(json!({"action": "enable", "id": id, "enabled": false})).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let all = connects(&audit);
    println!(
        "k8s.connectPod records: {}",
        serde_json::to_string_pretty(&all).unwrap()
    );
    assert!(
        !succeeds || all.iter().any(|r| r["outcome"] == "ok"),
        "no ok record"
    );
    assert!(failures.is_empty(), "{failures:#?}");
}
