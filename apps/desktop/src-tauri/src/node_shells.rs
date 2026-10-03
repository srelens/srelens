//! A node shell's privileged debug pod is the host's to delete (#734).
//!
//! A node shell is an exec into a pod `k8s.createNodeDebugPod` made on the
//! node: privileged, sharing the host's PID, network and IPC namespaces. The
//! page used to delete it when its terminal closed, so a window that closed
//! or reloaded first — which #700 makes close the exec — left the pod on the
//! node, with host access, for no one. The host owns it now, from the moment
//! it is created:
//!
//! - The capability bridge records each debug pod a window creates, by its
//!   uid, as that window's ([`adopt`]).
//! - A shell opened into it moves the pod to that exec session.
//! - It is deleted, pinned to its uid, when the session ends — the terminal
//!   closed, the shell exited or failed to start, its window closed or
//!   reloaded — or when the window ends before any shell ran in it, or when
//!   srelens quits ([`delete_all`]).
//!
//! Each record is taken out before its delete, so a pod is deleted once. A
//! different pod that has taken the name since is left alone. A delete that
//! fails is logged and reported to every open window, since it is a
//! privileged pod still on a node.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::Value;
use srelens_kube::client_cache::ClientCache;
use srelens_kube::debug::{delete_node_debug_pod, DebugPodDeleted};
use srelens_streams::EventSink;
use tauri::{AppHandle, Manager, Runtime};

use crate::host_notice::{self, Level};
use crate::window_streams::{Stream, WindowStreams};

/// The capability whose pods the host adopts.
pub const NODE_DEBUG_CAPABILITY: &str = "k8s.createNodeDebugPod";

/// A node debug pod, by the identity it was created with.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DebugPod {
    pub context: String,
    pub namespace: String,
    pub name: String,
    pub uid: String,
}

impl DebugPod {
    /// The pod a `k8s.createNodeDebugPod` call made, from its input and its
    /// output.
    fn created(input: &Value, out: &Value) -> Option<Self> {
        let field = |v: &Value, key: &str| v[key].as_str().map(str::to_owned);
        Some(Self {
            context: field(input, "context")?,
            namespace: field(out, "namespace")?,
            name: field(out, "pod")?,
            uid: field(out, "uid")?,
        })
    }
}

/// The debug pod each node shell runs in, by exec session.
#[derive(Default)]
pub struct NodeShells(Mutex<HashMap<u64, DebugPod>>);

impl NodeShells {
    pub fn attach(&self, session: u64, pod: DebugPod) {
        self.0.lock().unwrap().insert(session, pod);
    }

    /// Take the pod `session` ran in, if it was a node shell.
    pub fn release(&self, session: u64) -> Option<DebugPod> {
        self.0.lock().unwrap().remove(&session)
    }

    fn release_all(&self) -> Vec<DebugPod> {
        self.0.lock().unwrap().drain().map(|(_, pod)| pod).collect()
    }
}

/// Record the debug pod a `k8s.createNodeDebugPod` call from `window` just
/// made as that window's. If the window closed or reloaded since `epoch` —
/// the page that asked is gone — the pod is deleted at once and the call
/// refused.
pub fn adopt<R: Runtime>(
    app: &AppHandle<R>,
    owned: &WindowStreams,
    window: &str,
    epoch: u64,
    input: &Value,
    out: &Value,
) -> Result<(), String> {
    let Some(pod) = DebugPod::created(input, out) else {
        log::warn!("a node debug pod was created without its identity: {out}");
        return Ok(());
    };
    owned.keep(window, epoch, Stream::DebugPod(pod.clone()), || {
        delete_soon(app, pod)
    })
}

/// Delete `pod` in the background.
pub fn delete_soon<R: Runtime>(app: &AppHandle<R>, pod: DebugPod) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move { delete(&app, pod).await });
}

/// Delete `pod`, pinned to its uid, and say how that went.
async fn delete<R: Runtime>(app: &AppHandle<R>, pod: DebugPod) {
    let deleted = match app.try_state::<Arc<ClientCache>>() {
        Some(cache) => match cache.get(&pod.context).await {
            Ok(client) => delete_node_debug_pod(client, &pod.namespace, &pod.name, &pod.uid).await,
            Err(e) => Err(e),
        },
        None => Err("srelens has no cluster connection to delete it with".to_owned()),
    };
    let DebugPod {
        context,
        namespace,
        name,
        ..
    } = &pod;
    match deleted {
        Ok(DebugPodDeleted::Deleted) => {
            log::info!("deleted node debug pod {namespace}/{name} on {context}")
        }
        Ok(DebugPodDeleted::AlreadyGone) => {
            log::info!("node debug pod {namespace}/{name} on {context} was already gone")
        }
        Ok(DebugPodDeleted::Replaced) => log::info!(
            "left {namespace}/{name} on {context} alone: a different pod has the name now"
        ),
        Err(error) => {
            log::warn!("could not delete node debug pod {namespace}/{name} on {context}: {error}");
            host_notice::notify(
                app,
                Level::Error,
                &format!("Couldn't delete debug pod {name}"),
                &format!(
                    "{error}. It is still on {context}, in namespace {namespace}, with host \
                     access: delete it yourself."
                ),
            );
        }
    }
}

/// Delete every debug pod still on a node: srelens is quitting.
pub async fn delete_all<R: Runtime>(app: &AppHandle<R>) {
    let mut pods = app
        .try_state::<NodeShells>()
        .map(|shells| shells.release_all())
        .unwrap_or_default();
    if let Some(owned) = app.try_state::<WindowStreams>() {
        pods.extend(owned.take_debug_pods());
    }
    for pod in pods {
        delete(app, pod).await;
    }
}

/// How an exec session ends on its own, so a node shell's pod goes with it.
/// The exit can arrive before the session's id does; whichever comes second
/// releases the pod.
#[derive(Default)]
pub struct ShellEnd {
    session: OnceLock<u64>,
    exited: AtomicBool,
}

impl ShellEnd {
    /// The session started as `session`.
    pub fn started<R: Runtime>(&self, app: &AppHandle<R>, session: u64) {
        let _ = self.session.set(session);
        if self.exited.load(Ordering::SeqCst) {
            release(app, session);
        }
    }

    fn exited<R: Runtime>(&self, app: &AppHandle<R>) {
        self.exited.store(true, Ordering::SeqCst);
        if let Some(&session) = self.session.get() {
            release(app, session);
        }
    }
}

/// Delete the pod `session` ran in, if it was a node shell.
pub fn release<R: Runtime>(app: &AppHandle<R>, session: u64) {
    if let Some(pod) = app
        .try_state::<NodeShells>()
        .and_then(|shells| shells.release(session))
    {
        delete_soon(app, pod);
    }
}

/// An exec's sink: everything goes to `inner`, and its exit also ends the
/// session's node shell.
pub struct ShellSink<R: Runtime, S> {
    pub inner: S,
    pub exit: String,
    pub end: Arc<ShellEnd>,
    pub app: AppHandle<R>,
}

impl<R: Runtime, S: EventSink> EventSink for ShellSink<R, S> {
    fn emit(&self, channel: &str, payload: Value) {
        self.inner.emit(channel, payload);
        if channel == self.exit {
            self.end.exited(&self.app);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use serde_json::{json, Value};
    use srelens_capability::audit::NoopAudit;
    use srelens_capability::Registry;
    use srelens_kube::client_cache::ClientCache;
    use srelens_streams::exec::ExecManager;
    use tauri::test::MockRuntime;
    use tauri::{Listener, Manager, Window, WindowEvent};

    use crate::bridge::{AppAudit, AppRegistry};
    use crate::extension_streams::AppExtensionStreams;
    use crate::window_streams::tests::mock_window;
    use crate::window_streams::{on_window_event, window_streams_reset, WindowStreams};

    /// What the fake API server does with an exec.
    #[derive(Clone, Copy)]
    enum Exec {
        /// Holds the connection open and never answers: a shell that runs.
        Hangs,
        /// Refuses it: a shell that ends as soon as it starts.
        Fails,
    }

    /// One delete the fake API server was sent.
    #[derive(Clone, Debug)]
    struct Delete {
        name: String,
        uid: Value,
    }

    /// An API server that creates each pod it is sent as
    /// `srelens-node-debug-x<n>` with uid `uid-<n>`, answers every delete with
    /// `delete_status`, and does `exec` with an exec. Its context is `fake`.
    struct Cluster {
        kubeconfig: tempfile::TempDir,
        deletes: Arc<Mutex<Vec<Delete>>>,
    }

    impl Cluster {
        fn start(delete_status: u16, exec: Exec) -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let deletes = Arc::new(Mutex::new(Vec::new()));
            let created = Arc::new(AtomicUsize::new(0));
            let record = deletes.clone();
            std::thread::spawn(move || {
                for mut conn in listener.incoming().flatten() {
                    let (record, created) = (record.clone(), created.clone());
                    std::thread::spawn(move || {
                        let Some((method, path, body)) = read_request(&mut conn) else {
                            return;
                        };
                        let (status, reply) = if path.contains("/exec") {
                            match exec {
                                Exec::Hangs => {
                                    let mut buf = [0u8; 1024];
                                    while matches!(conn.read(&mut buf), Ok(n) if n > 0) {}
                                    return;
                                }
                                Exec::Fails => (403, failure(403, "Forbidden", "exec is forbidden")),
                            }
                        } else if method == "GET" && path.contains("/pods/") {
                            // What the exec waits for before it starts: the
                            // pod's `debug` container, running.
                            (200, running_pod(&path))
                        } else if method == "POST" {
                            let n = created.fetch_add(1, Ordering::SeqCst) + 1;
                            let namespace = path.split('/').nth(4).unwrap_or("default").to_owned();
                            (
                                201,
                                json!({
                                    "apiVersion": "v1", "kind": "Pod",
                                    "metadata": {
                                        "name": format!("srelens-node-debug-x{n}"),
                                        "namespace": namespace,
                                        "uid": format!("uid-{n}"),
                                    },
                                }),
                            )
                        } else if method == "DELETE" {
                            let name = path.split('?').next().unwrap_or("").rsplit('/').next();
                            record.lock().unwrap().push(Delete {
                                name: name.unwrap_or("").to_owned(),
                                uid: body["preconditions"]["uid"].clone(),
                            });
                            let reply = match delete_status {
                                200 => json!({ "apiVersion": "v1", "kind": "Status", "status": "Success" }),
                                409 => failure(409, "Conflict", "Precondition failed: UID in precondition: uid-1, UID in object meta: uid-9"),
                                code => failure(code, "Forbidden", "pods is forbidden: user cannot delete pods"),
                            };
                            (delete_status, reply)
                        } else {
                            (404, failure(404, "NotFound", "not found"))
                        };
                        let reply = reply.to_string();
                        let _ = write!(
                            conn,
                            "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n\
                             Content-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                            reply.len()
                        );
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
            Self { kubeconfig, deletes }
        }

        fn deletes(&self) -> Vec<Delete> {
            self.deletes.lock().unwrap().clone()
        }

        /// A MockRuntime app holding what a node shell touches, reaching this
        /// server, with windows `main` and `ctx-1`.
        fn app(&self) -> (tauri::App<MockRuntime>, Window<MockRuntime>, Window<MockRuntime>) {
            let cache = ClientCache::new_many(vec![self.kubeconfig.path().join("config")]);
            let mut registry = Registry::new();
            registry.register(srelens_kube::debug::node_debug_pod_capability(cache.clone()));
            let app = tauri::test::mock_app();
            app.manage(AppRegistry(registry));
            app.manage(AppAudit(Arc::new(NoopAudit)));
            app.manage(cache.clone());
            app.manage(ExecManager::new(cache));
            app.manage(WindowStreams::default());
            app.manage(NodeShells::default());
            app.manage(AppExtensionStreams(None));
            let main = mock_window(&app, "main");
            let other = mock_window(&app, "ctx-1");
            (app, main, other)
        }
    }

    fn running_pod(path: &str) -> Value {
        let name = path.split('?').next().unwrap_or("").rsplit('/').next().unwrap_or("");
        json!({
            "apiVersion": "v1", "kind": "Pod",
            "metadata": { "name": name, "namespace": "default" },
            "status": {
                "phase": "Running",
                "containerStatuses": [{
                    "name": "debug", "image": "busybox", "imageID": "", "ready": true,
                    "restartCount": 0, "state": { "running": { "startedAt": "2026-10-03T00:00:00Z" } },
                }],
            },
        })
    }

    fn failure(code: u16, reason: &str, message: &str) -> Value {
        json!({
            "apiVersion": "v1", "kind": "Status", "status": "Failure",
            "reason": reason, "code": code, "message": message,
        })
    }

    fn read_request(conn: &mut std::net::TcpStream) -> Option<(String, String, Value)> {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        let head_end = loop {
            let n = conn.read(&mut chunk).ok()?;
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
            let n = conn.read(&mut chunk).ok()?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        Some((method, path, serde_json::from_slice(&buf[head_end..]).unwrap_or(Value::Null)))
    }

    async fn eventually(what: &str, check: impl Fn() -> bool) {
        for _ in 0..500 {
            if check() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("never happened: {what}");
    }

    /// Create a node debug pod from `window`, as the page does: through the
    /// capability bridge. Answers the pod's name.
    async fn debug_pod_in(app: &tauri::App<MockRuntime>, window: &Window<MockRuntime>) -> String {
        let out = crate::bridge::invoke_capability(
            "k8s.createNodeDebugPod".into(),
            json!({ "context": "fake", "node": "node-1" }),
            window.clone(),
            app.handle().clone(),
            app.state(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
        out["pod"].as_str().unwrap().to_owned()
    }

    /// Open the node shell in `pod` from `window`, as the page does.
    async fn shell_in(app: &tauri::App<MockRuntime>, window: &Window<MockRuntime>, pod: &str) -> u64 {
        crate::exec::start_pod_exec(
            "fake".into(),
            "default".into(),
            pod.into(),
            Some("debug".into()),
            None,
            Some(vec!["nsenter".into(), "--target".into(), "1".into()]),
            format!("x-{pod}"),
            None,
            None,
            crate::sink::tests::recording().0,
            window.clone(),
            app.handle().clone(),
            app.state(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap()
    }

    /// The host notices the app has broadcast.
    fn notices(app: &tauri::App<MockRuntime>) -> Arc<Mutex<Vec<Value>>> {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let record = heard.clone();
        app.listen_any("host-notice", move |e| {
            record
                .lock()
                .unwrap()
                .push(serde_json::from_str(e.payload()).unwrap_or_default());
        });
        heard
    }

    /// The window a node shell ran in closed before its page could delete the
    /// pod. The host deletes it — once, pinned to the uid it was created with
    /// — and leaves another window's node shell, and its pod, alone.
    #[tokio::test(flavor = "multi_thread")]
    async fn closing_a_window_deletes_its_node_shells_pod_once_and_no_other() {
        let cluster = Cluster::start(200, Exec::Hangs);
        let (app, main, other) = cluster.app();
        let mine = debug_pod_in(&app, &main).await;
        shell_in(&app, &main, &mine).await;
        let theirs = debug_pod_in(&app, &other).await;
        shell_in(&app, &other, &theirs).await;

        on_window_event(&main, &WindowEvent::Destroyed);
        eventually("the closed window's pod was deleted", || {
            !cluster.deletes().is_empty()
        })
        .await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        let deletes = cluster.deletes();
        assert_eq!(deletes.len(), 1, "exactly once, and only its own: {deletes:?}");
        assert_eq!(deletes[0].name, mine);
        assert_eq!(deletes[0].uid, "uid-1", "pinned to the pod it created");

        on_window_event(&other, &WindowEvent::Destroyed);
        eventually("the other window's pod went with it", || cluster.deletes().len() == 2).await;
        assert_eq!(cluster.deletes()[1].name, theirs);
    }

    /// Closing the shell deletes its pod, and so does a reload — including a
    /// pod no shell was ever opened in, which only the window held.
    #[tokio::test(flavor = "multi_thread")]
    async fn closing_the_shell_or_reloading_the_window_deletes_the_pod() {
        let cluster = Cluster::start(200, Exec::Hangs);
        let (app, main, _) = cluster.app();
        let shelled = debug_pod_in(&app, &main).await;
        let session = shell_in(&app, &main, &shelled).await;
        crate::exec::exec_close(session, main.clone(), app.handle().clone(), app.state(), app.state(), app.state())
            .await
            .unwrap();
        eventually("closing the shell deleted its pod", || cluster.deletes().len() == 1).await;
        assert_eq!(cluster.deletes()[0].name, shelled);

        let unshelled = debug_pod_in(&app, &main).await;
        window_streams_reset(main.clone(), app.handle().clone()).await.unwrap();
        eventually("the reload deleted the pod no shell ran in", || {
            cluster.deletes().len() == 2
        })
        .await;
        assert_eq!(cluster.deletes()[1].name, unshelled);
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(cluster.deletes().len(), 2, "each once");
    }

    /// A shell that ends on its own — the exec refused, or the shell exited —
    /// leaves nothing for its pod to do. The host deletes it then, without
    /// waiting for the page to close anything.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_shell_that_ends_on_its_own_deletes_its_pod() {
        let cluster = Cluster::start(200, Exec::Fails);
        let (app, main, _) = cluster.app();
        let pod = debug_pod_in(&app, &main).await;
        shell_in(&app, &main, &pod).await;
        eventually("the pod was deleted when its shell ended", || {
            cluster.deletes().len() == 1
        })
        .await;
        assert_eq!(cluster.deletes()[0].name, pod);
    }

    /// A different pod has the name now. The delete is pinned to the uid the
    /// host recorded, so the API server refuses it, and the host leaves that
    /// pod alone: no second, unpinned delete, and no failure reported.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_pod_replaced_under_its_name_is_not_deleted() {
        let cluster = Cluster::start(409, Exec::Hangs);
        let (app, main, _) = cluster.app();
        let heard = notices(&app);
        let pod = debug_pod_in(&app, &main).await;
        shell_in(&app, &main, &pod).await;

        on_window_event(&main, &WindowEvent::Destroyed);
        eventually("the pinned delete was sent", || !cluster.deletes().is_empty()).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        let deletes = cluster.deletes();
        assert_eq!(deletes.len(), 1, "{deletes:?}");
        assert_eq!(deletes[0].uid, "uid-1");
        assert!(heard.lock().unwrap().is_empty(), "{:?}", heard.lock().unwrap());
    }

    /// A delete the cluster refuses leaves a privileged pod on the node, and
    /// the person who opened the shell has to hear it — every open window is
    /// told, by name.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_delete_that_fails_is_reported() {
        let cluster = Cluster::start(403, Exec::Hangs);
        let (app, main, _) = cluster.app();
        let heard = notices(&app);
        let pod = debug_pod_in(&app, &main).await;
        shell_in(&app, &main, &pod).await;

        on_window_event(&main, &WindowEvent::Destroyed);
        eventually("the failure was reported", || !heard.lock().unwrap().is_empty()).await;
        let notice = heard.lock().unwrap()[0].clone();
        assert_eq!(notice["level"], "error");
        let title = notice["title"].as_str().unwrap();
        assert!(title.contains(&pod), "{title}");
        let detail = notice["detail"].as_str().unwrap();
        assert!(detail.contains("forbidden"), "{detail}");
    }

    /// srelens quitting is the last chance: every debug pod still on a node —
    /// one a shell runs in, one no shell was opened in — is deleted first.
    #[tokio::test(flavor = "multi_thread")]
    async fn quitting_deletes_every_debug_pod_still_on_a_node() {
        let cluster = Cluster::start(200, Exec::Hangs);
        let (app, main, other) = cluster.app();
        let shelled = debug_pod_in(&app, &main).await;
        shell_in(&app, &main, &shelled).await;
        let unshelled = debug_pod_in(&app, &other).await;

        delete_all(app.handle()).await;

        let mut deleted: Vec<String> = cluster.deletes().into_iter().map(|d| d.name).collect();
        deleted.sort();
        let mut expected = vec![shelled, unshelled];
        expected.sort();
        assert_eq!(deleted, expected, "both, before it returned");
    }
}
