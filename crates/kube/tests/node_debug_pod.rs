//! The node shell's privileged debug pod, as the host cleans it up (#734).
//!
//! A node shell runs in a pod `k8s.createNodeDebugPod` made: host PID,
//! network and IPC namespaces, privileged. The desktop host deletes it when
//! the shell ends, and these are the two halves of that it relies on: the pod
//! names itself for what it is, and a delete aimed at it is pinned to the one
//! object it was, so a different pod that took the name since is never hit.

use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use srelens_kube::client_cache::ClientCache;
use srelens_kube::debug::{
    delete_node_debug_pod, node_debug_pod_capability, node_debug_pod_spec, DebugPodDeleted,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// One request the fake API server was sent.
#[derive(Clone, Debug)]
struct Seen {
    method: String,
    path: String,
    body: Value,
}

/// An API server that creates any pod it is sent as `srelens-node-debug-x1`
/// with uid `uid-1`, and answers every delete with `delete_status`.
struct Api {
    kubeconfig: tempfile::TempDir,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Api {
    async fn start(delete_status: u16) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let record = seen.clone();
        tokio::spawn(async move {
            while let Ok((mut conn, _)) = listener.accept().await {
                let record = record.clone();
                tokio::spawn(async move {
                    let Some((method, path, body)) = read_request(&mut conn).await else {
                        return;
                    };
                    record.lock().unwrap().push(Seen {
                        method: method.clone(),
                        path: path.clone(),
                        body,
                    });
                    let (status, reply) = match method.as_str() {
                        "POST" => (201, created_pod()),
                        "DELETE" => (delete_status, delete_reply(delete_status)),
                        _ => (404, status_body(404, "NotFound", "no such thing")),
                    };
                    let reply = reply.to_string();
                    let response = format!(
                        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n\
                         Content-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                        reply.len()
                    );
                    let _ = conn.write_all(response.as_bytes()).await;
                });
            }
        });
        let kubeconfig = tempfile::tempdir().unwrap();
        std::fs::write(
            kubeconfig.path().join("config"),
            format!(
                "apiVersion: v1\nkind: Config\ncurrent-context: fake\n\
                 clusters:\n- name: c\n  cluster:\n    server: http://127.0.0.1:{port}\n\
                 users:\n- name: u\n  user:\n    token: t\n\
                 contexts:\n- name: fake\n  context:\n    cluster: c\n    user: u\n"
            ),
        )
        .unwrap();
        Self { kubeconfig, seen }
    }

    fn cache(&self) -> Arc<ClientCache> {
        ClientCache::new_many(vec![self.kubeconfig.path().join("config")])
    }

    async fn client(&self) -> kube::Client {
        self.cache().get("fake").await.unwrap()
    }

    fn deletes(&self) -> Vec<Seen> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.method == "DELETE")
            .cloned()
            .collect()
    }
}

async fn read_request(conn: &mut tokio::net::TcpStream) -> Option<(String, String, Value)> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        let n = conn.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(at) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break at + 4;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut words = head.lines().next()?.split(' ');
    let (method, path) = (words.next()?.to_owned(), words.next()?.to_owned());
    let length: usize = head
        .lines()
        .find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.eq_ignore_ascii_case("content-length")
                .then(|| v.trim().parse().ok())
                .flatten()
        })
        .unwrap_or(0);
    while buf.len() < head_end + length {
        let n = conn.read(&mut chunk).await.ok()?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let body = serde_json::from_slice(&buf[head_end..]).unwrap_or(Value::Null);
    Some((method, path, body))
}

fn created_pod() -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": { "name": "srelens-node-debug-x1", "namespace": "default", "uid": "uid-1" },
    })
}

fn status_body(code: u16, reason: &str, message: &str) -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "Status",
        "status": "Failure",
        "reason": reason,
        "code": code,
        "message": message,
    })
}

fn delete_reply(status: u16) -> Value {
    match status {
        200 => json!({ "apiVersion": "v1", "kind": "Status", "status": "Success" }),
        404 => status_body(404, "NotFound", "pods \"srelens-node-debug-x1\" not found"),
        409 => status_body(
            409,
            "Conflict",
            "Precondition failed: UID in precondition: uid-1, UID in object meta: uid-2",
        ),
        _ => status_body(status, "Forbidden", "pods is forbidden: user cannot delete pods"),
    }
}

/// What a crashed session left behind is found by this label, so srelens can
/// offer to delete it: the pod says what it is, not only what it is called.
#[test]
fn a_debug_pod_is_labelled_as_one() {
    let spec = node_debug_pod_spec("node-1", "busybox");
    assert_eq!(spec["metadata"]["labels"]["srelens.dev/node-debug"], "true");
    assert_eq!(spec["metadata"]["generateName"], "srelens-node-debug-");
}

/// The host deletes the pod by the identity it was created with, so the
/// capability has to hand that identity back: the uid, not only the name.
#[tokio::test(flavor = "multi_thread")]
async fn creating_a_debug_pod_answers_its_uid() {
    let api = Api::start(200).await;
    let mut registry = srelens_capability::Registry::new();
    registry.register(node_debug_pod_capability(api.cache()));
    let out = registry
        .invoke("k8s.createNodeDebugPod", json!({ "context": "fake", "node": "node-1" }))
        .await
        .unwrap();
    assert_eq!(
        out,
        json!({ "namespace": "default", "pod": "srelens-node-debug-x1", "uid": "uid-1" })
    );
}

/// The delete names the uid as a precondition, so the API server deletes
/// that pod and no other.
#[tokio::test(flavor = "multi_thread")]
async fn a_delete_is_pinned_to_the_pods_uid() {
    let api = Api::start(200).await;
    let deleted = delete_node_debug_pod(api.client().await, "default", "srelens-node-debug-x1", "uid-1")
        .await
        .unwrap();
    assert_eq!(deleted, DebugPodDeleted::Deleted);
    let deletes = api.deletes();
    assert_eq!(deletes.len(), 1);
    assert_eq!(
        deletes[0].path.split('?').next(),
        Some("/api/v1/namespaces/default/pods/srelens-node-debug-x1")
    );
    assert_eq!(deletes[0].body["preconditions"]["uid"], "uid-1");
}

/// A pod that is already gone needs nothing more, and is not a failure.
#[tokio::test(flavor = "multi_thread")]
async fn a_pod_already_gone_is_not_a_failure() {
    let api = Api::start(404).await;
    let deleted = delete_node_debug_pod(api.client().await, "default", "srelens-node-debug-x1", "uid-1")
        .await
        .unwrap();
    assert_eq!(deleted, DebugPodDeleted::AlreadyGone);
}

/// A different pod has the name now — the precondition saw a different uid —
/// and it is not this session's to delete. Left alone, and not a failure.
#[tokio::test(flavor = "multi_thread")]
async fn a_pod_replaced_under_the_same_name_is_left_alone() {
    let api = Api::start(409).await;
    let deleted = delete_node_debug_pod(api.client().await, "default", "srelens-node-debug-x1", "uid-1")
        .await
        .unwrap();
    assert_eq!(deleted, DebugPodDeleted::Replaced);
    assert_eq!(api.deletes().len(), 1, "no second, unpinned delete");
}

/// Anything else is a failure the caller must report, with the server's
/// reason in it: the pod is still on the node.
#[tokio::test(flavor = "multi_thread")]
async fn any_other_refusal_is_an_error() {
    let api = Api::start(403).await;
    let err = delete_node_debug_pod(api.client().await, "default", "srelens-node-debug-x1", "uid-1")
        .await
        .unwrap_err();
    assert!(err.contains("forbidden"), "{err}");
}
