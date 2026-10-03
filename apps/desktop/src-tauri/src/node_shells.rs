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
use srelens_kube::debug::{delete_node_debug_pod, node_debug_pod_uid, DebugPodDeleted};
use srelens_kube::kube::Client;
use srelens_streams::EventSink;
use tauri::async_runtime::JoinHandle;
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

/// The debug pod each node shell runs in, by exec session, the client each
/// pod was made through, and the deletes under way.
#[derive(Default)]
pub struct NodeShells {
    shells: Mutex<HashMap<u64, DebugPod>>,
    /// By pod uid: a pod is deleted through the client that made it, never
    /// through a fresh lookup of its context's name, which a kubeconfig
    /// change can point at another cluster.
    clients: Mutex<HashMap<String, Client>>,
    /// Deletes started and not yet awaited: quitting waits for these too.
    deleting: Mutex<Vec<JoinHandle<()>>>,
}

impl NodeShells {
    /// `pod` reached exec `session`, which `window` started: the session owns
    /// it from here. Unless the window has ended since — a window's ending
    /// closes its sessions, and a closed session sends no exit, so the pod
    /// would sit on it until quitting — in which case it is deleted. The
    /// ending holds this lock while it takes the window's sessions (see
    /// [`NodeShells::lock`]), so the two cannot interleave.
    pub fn attach<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        owned: &WindowStreams,
        window: &str,
        session: u64,
        pod: DebugPod,
    ) {
        let mut shells = self.shells.lock().unwrap();
        if owned.check(window, &Stream::Exec(session)) == Ok(true) {
            shells.insert(session, pod);
        } else {
            drop(shells);
            delete_soon(app, pod);
        }
    }

    /// Hold the sessions' pods while a window's ending takes its sessions, so
    /// [`NodeShells::attach`] cannot attach a pod in between.
    pub(crate) fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u64, DebugPod>> {
        self.shells.lock().unwrap()
    }

    /// Take the pod `session` ran in, if it was a node shell.
    pub fn release(&self, session: u64) -> Option<DebugPod> {
        self.shells.lock().unwrap().remove(&session)
    }

    fn release_all(&self) -> Vec<DebugPod> {
        self.shells.lock().unwrap().drain().map(|(_, pod)| pod).collect()
    }

    fn deleting(&self, delete: JoinHandle<()>) {
        let mut deleting = self.deleting.lock().unwrap();
        deleting.retain(|d| !d.inner().is_finished());
        deleting.push(delete);
    }
}

/// Record the debug pod a `k8s.createNodeDebugPod` call from `window` just
/// made as that window's. If the window closed or reloaded since `epoch` —
/// the page that asked is gone — the pod is deleted at once and the call
/// refused.
pub async fn adopt<R: Runtime>(
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
    // The client the capability just made it through, for its delete — once
    // it is shown to reach the pod. The kubeconfig can change while the pod
    // is being created, and then the context's name already means another
    // cluster, whose 404 would read as "already gone" while the pod stays.
    let cache = app
        .try_state::<Arc<ClientCache>>()
        .map(|cache| cache.inner().clone());
    if let (Some(cache), Some(shells)) = (cache, app.try_state::<NodeShells>()) {
        if let Ok(client) = cache.get(&pod.context).await {
            match node_debug_pod_uid(client.clone(), &pod.namespace, &pod.name).await {
                Ok(Some(uid)) if uid == pod.uid => {
                    shells.clients.lock().unwrap().insert(pod.uid.clone(), client);
                }
                Ok(_) => {
                    lost_track(app, &pod);
                    return Ok(());
                }
                // Not checked: most likely the right cluster, and keeping it
                // beats deleting by a name looked up again later.
                Err(error) => {
                    log::warn!("could not check node debug pod {}: {error}", pod.name);
                    shells.clients.lock().unwrap().insert(pod.uid.clone(), client);
                }
            }
        }
    }
    owned.keep(window, epoch, Stream::DebugPod(pod.clone()), || {
        delete_soon(app, pod)
    })
}

/// The pod is not where its context now leads: the kubeconfig changed while
/// it was being created. srelens cannot tell which cluster holds it, so it
/// does not try to delete it, and says so.
fn lost_track<R: Runtime>(app: &AppHandle<R>, pod: &DebugPod) {
    let DebugPod {
        context,
        namespace,
        name,
        ..
    } = pod;
    log::warn!(
        "node debug pod {namespace}/{name} is not on the cluster {context} now names: \
         the kubeconfig changed while it was created; not deleting it"
    );
    host_notice::notify(
        app,
        Level::Error,
        &format!("Couldn't keep track of debug pod {name}"),
        &format!(
            "The kubeconfig changed while it was being created, so srelens can't tell which \
             cluster it is on and won't delete it. It is in namespace {namespace} on the \
             cluster {context} named before the change, with host access: delete it yourself."
        ),
    );
}

/// Delete `pod` in the background. Quitting waits for it.
pub fn delete_soon<R: Runtime>(app: &AppHandle<R>, pod: DebugPod) {
    let handle = app.clone();
    let delete = tauri::async_runtime::spawn(async move { delete(&handle, pod).await });
    if let Some(shells) = app.try_state::<NodeShells>() {
        shells.deleting(delete);
    }
}

/// Delete `pod`, pinned to its uid, and say how that went.
async fn delete<R: Runtime>(app: &AppHandle<R>, pod: DebugPod) {
    let pinned = app
        .try_state::<NodeShells>()
        .and_then(|shells| shells.clients.lock().unwrap().remove(&pod.uid));
    let client = match (pinned, app.try_state::<Arc<ClientCache>>()) {
        (Some(client), _) => Ok(client),
        (None, Some(cache)) => cache.get(&pod.context).await,
        (None, None) => Err("srelens has no cluster connection to delete it with".to_owned()),
    };
    let deleted = match client {
        Ok(client) => delete_node_debug_pod(client, &pod.namespace, &pod.name, &pod.uid).await,
        Err(e) => Err(e),
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

/// How long quitting waits for its deletes: one request's whole timeout, since
/// they run side by side, and a moment more for them to start.
pub fn quit_deadline(request_timeout: std::time::Duration) -> std::time::Duration {
    request_timeout + std::time::Duration::from_secs(2)
}

/// Delete every debug pod still on a node, and finish the deletes already
/// under way — the last window closing starts its own as srelens quits.
pub async fn delete_all<R: Runtime>(app: &AppHandle<R>) {
    let (mut pods, under_way) = match app.try_state::<NodeShells>() {
        Some(shells) => (
            shells.release_all(),
            std::mem::take(&mut *shells.deleting.lock().unwrap()),
        ),
        None => Default::default(),
    };
    if let Some(owned) = app.try_state::<WindowStreams>() {
        pods.extend(owned.take_debug_pods());
    }
    // Side by side: one slow cluster must not use up the others' time.
    let started: Vec<JoinHandle<()>> = pods
        .into_iter()
        .map(|pod| {
            let app = app.clone();
            tauri::async_runtime::spawn(async move { delete(&app, pod).await })
        })
        .collect();
    for delete in started.into_iter().chain(under_way) {
        let _ = delete.await;
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
    use crate::window_streams::{on_window_event, window_streams_reset, Stream, WindowStreams};

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
        answered: Arc<AtomicUsize>,
    }

    impl Cluster {
        fn start(delete_status: u16, exec: Exec) -> Self {
            Self::start_slow(delete_status, exec, Duration::ZERO)
        }

        /// As [`Cluster::start`], answering each delete only after `delete_delay`.
        fn start_slow(delete_status: u16, exec: Exec, delete_delay: Duration) -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let deletes = Arc::new(Mutex::new(Vec::new()));
            let answered = Arc::new(AtomicUsize::new(0));
            let created = Arc::new(AtomicUsize::new(0));
            let (record, answers) = (deletes.clone(), answered.clone());
            std::thread::spawn(move || {
                for mut conn in listener.incoming().flatten() {
                    let (record, created, answered) =
                        (record.clone(), created.clone(), answers.clone());
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
                            // A pod this server made, its `debug` container
                            // running — what an exec waits for — and nothing
                            // else, as a real cluster has only its own pods.
                            let name = path.split('?').next().unwrap_or("").rsplit('/').next();
                            let name = name.unwrap_or("").to_owned();
                            let made = name
                                .strip_prefix("srelens-node-debug-x")
                                .and_then(|n| n.parse::<usize>().ok())
                                .filter(|n| *n <= created.load(Ordering::SeqCst));
                            match made {
                                Some(n) => (200, running_pod(&name, &format!("uid-{n}"))),
                                None => (404, failure(404, "NotFound", "pods not found")),
                            }
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
                            std::thread::sleep(delete_delay);
                            let reply = reply.to_string();
                            let _ = write!(
                                conn,
                                "HTTP/1.1 {delete_status} X\r\nContent-Type: application/json\r\n\
                                 Content-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                                reply.len()
                            );
                            answered.fetch_add(1, Ordering::SeqCst);
                            return;
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
            Self {
                kubeconfig,
                deletes,
                answered,
            }
        }

        fn deletes(&self) -> Vec<Delete> {
            self.deletes.lock().unwrap().clone()
        }

        /// How many deletes the server has finished answering.
        fn answered(&self) -> usize {
            self.answered.load(Ordering::SeqCst)
        }

        /// A MockRuntime app holding what a node shell touches, reaching this
        /// server, with windows `main` and `ctx-1`.
        fn app(&self) -> (tauri::App<MockRuntime>, Window<MockRuntime>, Window<MockRuntime>) {
            let cache = self.cache();
            let mut registry = Registry::new();
            registry.register(srelens_kube::debug::node_debug_pod_capability(cache.clone()));
            Self::app_with(cache, registry)
        }

        fn cache(&self) -> Arc<ClientCache> {
            ClientCache::new_many(vec![self.kubeconfig.path().join("config")])
        }

        /// As [`Cluster::app`], over `cache`, with the capabilities in `registry`.
        fn app_with(
            cache: Arc<ClientCache>,
            registry: Registry,
        ) -> (tauri::App<MockRuntime>, Window<MockRuntime>, Window<MockRuntime>) {
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

    fn running_pod(name: &str, uid: &str) -> Value {
        json!({
            "apiVersion": "v1", "kind": "Pod",
            "metadata": { "name": name, "namespace": "default", "uid": uid },
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

    /// The race a window closing opens while a shell starts (#799 review): the
    /// shell's session is recorded as the window's, the window ends — which
    /// closes the session, and a closed session sends no exit — and only then
    /// does the pod reach the session. It must not stay attached to a session
    /// no window holds, where nothing would ever delete it before quitting.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_pod_that_reaches_its_session_after_the_window_ended_is_deleted() {
        let cluster = Cluster::start(200, Exec::Hangs);
        let (app, main, _) = cluster.app();
        let owned = app.state::<WindowStreams>();
        let shells = app.state::<NodeShells>();
        let pod = DebugPod {
            context: "fake".into(),
            namespace: "default".into(),
            name: "srelens-node-debug-x9".into(),
            uid: "uid-9".into(),
        };
        let epoch = owned.epoch("main");
        owned.keep("main", epoch, Stream::Exec(42), || {}).unwrap();
        on_window_event(&main, &WindowEvent::Destroyed);

        shells.attach(app.handle(), &owned, "main", 42, pod);

        assert!(shells.release(42).is_none(), "nothing attached to a session no window holds");
        eventually("the pod was deleted", || cluster.deletes().len() == 1).await;
        assert_eq!(cluster.deletes()[0].name, "srelens-node-debug-x9");
    }

    /// The kubeconfig changes while a node shell is open, and its context's
    /// name now names another cluster (#799 review). The pod is deleted on
    /// the cluster it was made on — by the client that made it, not by a
    /// lookup of a name that may mean something else now, where a 404 would
    /// read as "already gone".
    #[tokio::test(flavor = "multi_thread")]
    async fn a_pod_is_deleted_on_the_cluster_it_was_made_on_though_its_context_moved() {
        let first = Cluster::start(200, Exec::Hangs);
        let second = Cluster::start(200, Exec::Hangs);
        let (app, main, _) = first.app();
        let pod = debug_pod_in(&app, &main).await;
        app.state::<Arc<ClientCache>>()
            .set_paths(vec![second.kubeconfig.path().join("config")])
            .await;

        on_window_event(&main, &WindowEvent::Destroyed);
        eventually("the pod was deleted where it was made", || {
            first.deletes().len() == 1
        })
        .await;
        assert_eq!(first.deletes()[0].name, pod);
        assert!(second.deletes().is_empty(), "{:?}", second.deletes());
    }

    /// The kubeconfig changed as the pod was being created (#799 review): by
    /// the time the host looks its context up to pin a client, the name
    /// already means another cluster. The host checks by uid that the pod is
    /// where that client reaches, and finding it is not, tells the user and
    /// lets it be — rather than pinning a cluster whose 404 would read as
    /// "already gone" while the privileged pod stays on its node.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_pod_made_as_the_kubeconfig_changed_is_reported_not_pinned_elsewhere() {
        let first = Cluster::start(200, Exec::Hangs);
        let second = Cluster::start(200, Exec::Hangs);
        let cache = first.cache();
        // The create lands on the first cluster, and the kubeconfig moves to
        // the second the moment it returns.
        let mut racing = srelens_kube::debug::node_debug_pod_capability(cache.clone());
        let create = racing.handler.clone();
        let moved = (cache.clone(), second.kubeconfig.path().join("config"));
        racing.handler = Arc::new(move |input| {
            let (create, (cache, moved)) = (create.clone(), moved.clone());
            Box::pin(async move {
                let out = create(input).await;
                cache.set_paths(vec![moved]).await;
                out
            })
        });
        let mut registry = Registry::new();
        registry.register(racing);
        let (app, main, _) = Cluster::app_with(cache, registry);
        let heard = notices(&app);

        let pod = debug_pod_in(&app, &main).await;
        on_window_event(&main, &WindowEvent::Destroyed);

        eventually("the user was told", || !heard.lock().unwrap().is_empty()).await;
        let notice = heard.lock().unwrap()[0].clone();
        assert_eq!(notice["level"], "error");
        assert!(notice["title"].as_str().unwrap().contains(&pod), "{notice}");
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(second.deletes().is_empty(), "{:?}", second.deletes());
    }

    /// One delete may take a whole request timeout, so quitting waits longer
    /// than that, however the timeout is set (#799 review).
    #[test]
    fn the_quit_deadline_outlasts_one_delete() {
        for timeout in [Duration::from_secs(8), Duration::from_secs(120)] {
            assert!(quit_deadline(timeout) > timeout, "{timeout:?}");
        }
    }

    /// Quitting deletes its pods side by side, not one after another, so
    /// several slow deletes take no longer than one (#799 review).
    #[tokio::test(flavor = "multi_thread")]
    async fn quitting_deletes_its_pods_side_by_side() {
        let cluster = Cluster::start_slow(200, Exec::Hangs, Duration::from_millis(700));
        let (app, main, other) = cluster.app();
        debug_pod_in(&app, &main).await;
        debug_pod_in(&app, &other).await;
        debug_pod_in(&app, &main).await;

        let started = std::time::Instant::now();
        delete_all(app.handle()).await;

        assert_eq!(cluster.answered(), 3);
        let took = started.elapsed();
        assert!(took < Duration::from_millis(1600), "one after another: {took:?}");
    }

    /// Quitting as the last window closes: that window's delete is already
    /// under way, and srelens must not exit before it has finished — against a
    /// slow cluster, exiting first leaves the privileged pod on the node.
    #[tokio::test(flavor = "multi_thread")]
    async fn quitting_waits_for_a_delete_already_under_way() {
        let cluster = Cluster::start_slow(200, Exec::Hangs, Duration::from_millis(500));
        let (app, main, _) = cluster.app();
        let pod = debug_pod_in(&app, &main).await;
        shell_in(&app, &main, &pod).await;

        on_window_event(&main, &WindowEvent::Destroyed);
        delete_all(app.handle()).await;

        assert_eq!(
            cluster.answered(),
            1,
            "the window's delete finished before quitting returned"
        );
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
