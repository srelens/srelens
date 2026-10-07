//! A window's streams end with the window (#700, #735).
//!
//! Every stream a WebView opens — an app stream, a built-in resource watch,
//! a pod exec, a log tail, a port-forward, a local terminal, a helm
//! operation — is recorded against the label of the window that opened it.
//! The label is Tauri's, taken from the window the command was invoked from,
//! never from anything the page sends, so one window cannot end another's.
//!
//! Two things end them:
//!
//! - **The window is destroyed** ([`on_window_event`]): every stream it opened
//!   ends, app streams with `close: windowClosed`.
//! - **The window reloads** ([`window_streams_reset`]): the window survives,
//!   but the page that owned its streams is gone and never said so. The new
//!   page calls this before it opens anything (`@srelens/core`'s transport
//!   does), and it ends every stream the window held, app streams with
//!   `close: windowReloaded`.
//!
//! Ending one stops it — a forward's listener is dropped, so its local port
//! is free again, and a terminal's shell is killed — except a helm operation,
//! which is let go of and runs to the end: see `crate::helm` for why, and for
//! how its outcome is reported.
//!
//! Each ending bumps the window's epoch. A stream whose open began before the
//! ending — the old page asked, and the host was still starting it when the
//! page went away — is stopped as soon as its start returns, rather than left
//! running for nobody. App streams keep their own epoch, in
//! `srelens_streams::app`, and refuse such an open before it sends a frame.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use serde::Serialize;
use srelens_streams::app::CloseReason;
use srelens_streams::exec::ExecManager;
use srelens_streams::forward::ForwardManager;
use srelens_streams::logs::LogStreamManager;
use srelens_streams::terminal::TerminalManager;
use srelens_streams::watch::WatchManager;
use tauri::{AppHandle, Manager, Runtime, Window, WindowEvent};

use crate::extension_streams::AppExtensionStreams;
use crate::helm::HelmOps;
use crate::node_shells::{self, DebugPod, NodeShells};

/// The built-in streams each window opened, and each window's epoch.
#[derive(Default)]
pub struct WindowStreams {
    windows: Mutex<HashMap<String, Owned>>,
}

/// One stream a window opened, by the key its manager holds it under.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Stream {
    /// A built-in resource watch, by channel.
    Watch(String),
    /// A pod exec session.
    Exec(u64),
    /// A log tail, by channel (#735).
    Log(String),
    /// A port-forward, which holds its local port until it ends (#735).
    Forward(u64),
    /// A local terminal: a shell on this machine (#735).
    Terminal(u64),
    /// A helm operation (#735). Let go of, never killed: see `crate::helm`.
    Helm(u64),
    /// A node debug pod the window created and no shell runs in yet (#734).
    /// Deleted with the window; see `crate::node_shells`.
    DebugPod(DebugPod),
}

impl Stream {
    /// How a refusal names it.
    fn describe(&self) -> String {
        match self {
            Stream::Watch(channel) => format!("Watch {channel}"),
            Stream::Exec(session) => format!("Shell session {session}"),
            Stream::Log(channel) => format!("Log stream {channel}"),
            Stream::Forward(id) => format!("Port-forward {id}"),
            Stream::Terminal(session) => format!("Terminal {session}"),
            Stream::Helm(session) => format!("Helm operation {session}"),
            Stream::DebugPod(pod) => format!("Debug pod {}", pod.name),
        }
    }

    /// What a late start's refusal calls it.
    fn noun(&self) -> &'static str {
        match self {
            Stream::Watch(_) => "watch",
            Stream::Exec(_) => "shell",
            Stream::Log(_) => "log stream",
            Stream::Forward(_) => "port-forward",
            Stream::Terminal(_) => "terminal",
            Stream::Helm(_) => "helm operation",
            Stream::DebugPod(_) => "debug pod",
        }
    }
}

#[derive(Default)]
struct Owned {
    epoch: u64,
    streams: HashSet<Stream>,
}

/// What one ending stopped. Answered by [`window_streams_reset`], and logged.
#[derive(Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowStreamsEnded {
    pub app_streams: usize,
    pub watches: usize,
    pub execs: usize,
    pub log_streams: usize,
    pub forwards: usize,
    pub terminals: usize,
    pub helm_left_running: usize,
    pub debug_pods: usize,
}

impl WindowStreams {
    /// `window`'s epoch now. Read before a start does anything that awaits.
    pub fn epoch(&self, window: &str) -> u64 {
        self.windows
            .lock()
            .unwrap()
            .get(window)
            .map_or(0, |owned| owned.epoch)
    }

    /// Record `stream` as `window`'s. `false` when the window ended since
    /// `epoch`: the caller stops the stream, since nothing else will.
    pub fn own(&self, window: &str, epoch: u64, stream: Stream) -> bool {
        let mut windows = self.windows.lock().unwrap();
        let owned = windows.entry(window.to_owned()).or_default();
        if owned.epoch != epoch {
            return false;
        }
        owned.streams.insert(stream);
        true
    }

    /// Whether `window` may act on `stream` (#733): `Ok(true)` when it opened
    /// it, `Ok(false)` when no window holds it — it ended, or never started —
    /// and refused when another window did.
    pub fn check(&self, window: &str, stream: &Stream) -> Result<bool, String> {
        let windows = self.windows.lock().unwrap();
        if windows
            .get(window)
            .is_some_and(|owned| owned.streams.contains(stream))
        {
            return Ok(true);
        }
        if windows.values().any(|owned| owned.streams.contains(stream)) {
            return Err(format!(
                "{} was not opened by this window",
                stream.describe()
            ));
        }
        Ok(false)
    }

    /// Forget `stream`, as the window stops it itself.
    pub fn disown(&self, window: &str, stream: &Stream) {
        if let Some(owned) = self.windows.lock().unwrap().get_mut(window) {
            owned.streams.remove(stream);
        }
    }

    /// Forget `stream` for whichever window holds it: one any window may stop.
    pub fn disown_everywhere(&self, stream: &Stream) {
        for owned in self.windows.lock().unwrap().values_mut() {
            owned.streams.remove(stream);
        }
    }

    /// A stream that started for `window` at `epoch`: recorded as the
    /// window's, or — the window ended while it started — ended with `stop`,
    /// and refused. The start commands end with this.
    pub fn keep(
        &self,
        window: &str,
        epoch: u64,
        stream: Stream,
        stop: impl FnOnce(),
    ) -> Result<(), String> {
        let noun = stream.noun();
        if self.own(window, epoch, stream) {
            return Ok(());
        }
        stop();
        Err(format!(
            "The window {window} closed or reloaded while this {noun} was starting"
        ))
    }

    /// Take the debug pod `context`/`namespace`/`name` out of `window`'s, if
    /// it created it: a shell is opening into it, and owns it from here.
    pub fn take_debug_pod(
        &self,
        window: &str,
        context: &str,
        namespace: &str,
        name: &str,
    ) -> Option<DebugPod> {
        let mut windows = self.windows.lock().unwrap();
        let owned = windows.get_mut(window)?;
        let held = owned.streams.iter().find_map(|stream| match stream {
            Stream::DebugPod(pod)
                if pod.context == context && pod.namespace == namespace && pod.name == name =>
            {
                Some(pod.clone())
            }
            _ => None,
        })?;
        owned.streams.remove(&Stream::DebugPod(held.clone()));
        Some(held)
    }

    /// Take every debug pod any window holds: srelens is quitting.
    pub fn take_debug_pods(&self) -> Vec<DebugPod> {
        let mut pods = Vec::new();
        for owned in self.windows.lock().unwrap().values_mut() {
            owned.streams.retain(|stream| match stream {
                Stream::DebugPod(pod) => {
                    pods.push(pod.clone());
                    false
                }
                _ => true,
            });
        }
        pods
    }

    /// Bump `window`'s epoch and hand back everything it owned. The entry
    /// stays, holding the epoch: labels are reused (a context's window opens
    /// under the same label every time), and a start begun before this must
    /// still see that the window moved on.
    fn take(&self, window: &str) -> Vec<Stream> {
        let mut windows = self.windows.lock().unwrap();
        let owned = windows.entry(window.to_owned()).or_default();
        owned.epoch += 1;
        owned.streams.drain().collect()
    }
}

/// End every stream `window` opened: stop its watches, close its exec
/// sessions (aborting the task drops the connection to the pod), and end its
/// app streams with `reason`. Each manager is optional, so a host that did
/// not manage one — or a test — still ends the rest.
pub fn end_window<R: Runtime>(
    app: &AppHandle<R>,
    window: &str,
    reason: CloseReason,
) -> WindowStreamsEnded {
    // Held while the window's sessions are taken and their pods released, so
    // a shell still starting cannot attach its pod in between (#734).
    let shells = app.try_state::<NodeShells>();
    let mut attached = shells.as_ref().map(|shells| shells.lock());
    let streams = app
        .try_state::<WindowStreams>()
        .map(|owned| owned.take(window))
        .unwrap_or_default();
    let mut ended = WindowStreamsEnded::default();
    for stream in &streams {
        match stream {
            Stream::Watch(channel) => {
                ended.watches += 1;
                if let Some(manager) = app.try_state::<WatchManager>() {
                    manager.stop(channel);
                }
            }
            Stream::Exec(session) => {
                ended.execs += 1;
                if let Some(manager) = app.try_state::<ExecManager>() {
                    manager.close(*session);
                }
                // A node shell's pod goes with its shell (#734).
                if let Some(pod) = attached.as_mut().and_then(|pods| pods.remove(session)) {
                    ended.debug_pods += 1;
                    node_shells::delete_soon(app, pod);
                }
            }
            Stream::DebugPod(pod) => {
                ended.debug_pods += 1;
                node_shells::delete_soon(app, pod.clone());
            }
            Stream::Log(channel) => {
                ended.log_streams += 1;
                if let Some(manager) = app.try_state::<LogStreamManager>() {
                    manager.stop(channel);
                }
            }
            // Aborting the forward's task drops its listener, which is what
            // gives the local port back.
            Stream::Forward(id) => {
                ended.forwards += 1;
                if let Some(manager) = app.try_state::<ForwardManager>() {
                    manager.stop(*id);
                }
            }
            // Kills the shell and removes its overlay kubeconfig.
            Stream::Terminal(session) => {
                ended.terminals += 1;
                if let Some(manager) = app.try_state::<TerminalManager>() {
                    manager.close(*session);
                }
            }
            // Never killed: helm runs to the end, and how it ended is
            // reported, since the page that would have said is gone.
            Stream::Helm(session) => {
                let reason = match reason {
                    CloseReason::WindowReloaded => "reloaded",
                    _ => "closed",
                };
                if app
                    .try_state::<HelmOps>()
                    .is_some_and(|ops| ops.let_go(app, *session, window, reason))
                {
                    ended.helm_left_running += 1;
                }
            }
        }
    }
    drop(attached);
    ended.app_streams = app
        .try_state::<AppExtensionStreams>()
        .and_then(|streams| {
            streams
                .0
                .as_ref()
                .map(|streams| streams.end_window(window, reason))
        })
        .unwrap_or(0);
    if ended != WindowStreamsEnded::default() {
        log::info!(
            "window {window} ({reason:?}): ended {} app streams, {} watches, {} exec sessions, \
             {} log streams, {} port-forwards, {} terminals; left {} helm operations running; \
             deleting {} node debug pods",
            ended.app_streams,
            ended.watches,
            ended.execs,
            ended.log_streams,
            ended.forwards,
            ended.terminals,
            ended.helm_left_running,
            ended.debug_pods
        );
    }
    ended
}

/// End every stream the calling window opened: its page reloaded, so
/// nothing is left to receive them. The page calls this before it opens a
/// stream; the window is the one Tauri says invoked it.
#[tauri::command]
pub async fn window_streams_reset<R: Runtime>(
    window: Window<R>,
    app: AppHandle<R>,
) -> Result<WindowStreamsEnded, String> {
    Ok(end_window(
        &app,
        window.label(),
        CloseReason::WindowReloaded,
    ))
}

/// The app's window-event handler: a destroyed window ends its streams.
pub fn on_window_event<R: Runtime>(window: &Window<R>, event: &WindowEvent) {
    if let WindowEvent::Destroyed = event {
        end_window(
            window.app_handle(),
            window.label(),
            CloseReason::WindowClosed,
        );
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Read;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use srelens_streams::helm::HelmManager;
    use tauri::Listener;
    use std::time::Duration;
    use tauri::test::MockRuntime;

    /// A window labelled `label` in a MockRuntime app, as a command receives it.
    pub(crate) fn mock_window(app: &tauri::App<MockRuntime>, label: &str) -> Window<MockRuntime> {
        tauri::WebviewWindowBuilder::new(app, label, tauri::WebviewUrl::default())
            .build()
            .unwrap()
            .as_ref()
            .window()
    }

    /// An API server that accepts connections and never answers, so a watch,
    /// exec or read aimed at it stays running until it is stopped. Counts the
    /// connections it accepted and the ones the client has since dropped. Its
    /// context is `silent`.
    pub(crate) struct Silent {
        kubeconfig: tempfile::TempDir,
        accepted: Arc<AtomicUsize>,
        dropped: Arc<AtomicUsize>,
    }

    impl Silent {
        pub(crate) fn start() -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let accepted = Arc::new(AtomicUsize::new(0));
            let dropped = Arc::new(AtomicUsize::new(0));
            let (a, d) = (accepted.clone(), dropped.clone());
            std::thread::spawn(move || {
                for mut conn in listener.incoming().flatten() {
                    a.fetch_add(1, Ordering::SeqCst);
                    let d = d.clone();
                    std::thread::spawn(move || {
                        let mut buf = [0u8; 4096];
                        while matches!(conn.read(&mut buf), Ok(n) if n > 0) {}
                        d.fetch_add(1, Ordering::SeqCst);
                    });
                }
            });
            let kubeconfig = tempfile::tempdir().unwrap();
            std::fs::write(
                kubeconfig.path().join("config"),
                format!(
                    "apiVersion: v1\nkind: Config\ncurrent-context: silent\n\
                     clusters:\n- name: c\n  cluster:\n    server: https://127.0.0.1:{port}\n    insecure-skip-tls-verify: true\n\
                     users:\n- name: u\n  user:\n    token: t\n\
                     contexts:\n- name: silent\n  context:\n    cluster: c\n    user: u\n"
                ),
            )
            .unwrap();
            Self {
                kubeconfig,
                accepted,
                dropped,
            }
        }

        fn open(&self) -> usize {
            self.accepted.load(Ordering::SeqCst) - self.dropped.load(Ordering::SeqCst)
        }

        /// The kubeconfig naming this server, as a page passes an extra one.
        fn kubeconfig_path(&self) -> String {
            self.kubeconfig
                .path()
                .join("config")
                .display()
                .to_string()
        }

        /// A client cache whose one context, `silent`, reaches this server.
        pub(crate) fn cache(&self) -> Arc<srelens_kube::client_cache::ClientCache> {
            srelens_kube::client_cache::ClientCache::new_many(vec![self
                .kubeconfig
                .path()
                .join("config")])
        }

        /// A MockRuntime app with every stream manager this module ends,
        /// reaching this server, and windows `main` and `ctx-1`.
        fn app(
            &self,
        ) -> (
            tauri::App<MockRuntime>,
            Window<MockRuntime>,
            Window<MockRuntime>,
        ) {
            let cache = self.cache();
            let app = tauri::test::mock_app();
            app.manage(WatchManager::new(cache.clone()));
            app.manage(LogStreamManager::new(cache.clone()));
            app.manage(ForwardManager::new(cache.clone()));
            app.manage(TerminalManager::new());
            app.manage(HelmOps::default());
            app.manage(ExecManager::new(cache));
            app.manage(WindowStreams::default());
            app.manage(crate::node_shells::NodeShells::default());
            app.manage(AppExtensionStreams(None));
            let main = mock_window(&app, "main");
            let other = mock_window(&app, "ctx-1");
            (app, main, other)
        }
    }

    async fn eventually(what: &str, check: impl Fn() -> bool) {
        for _ in 0..300 {
            if check() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("never happened: {what}");
    }

    async fn watch_in(app: &tauri::App<MockRuntime>, window: &Window<MockRuntime>, channel: &str) {
        crate::watch::start_resource_watch(
            "silent".into(),
            "ns".into(),
            "pods".into(),
            channel.into(),
            vec![],
            crate::sink::tests::recording().0,
            window.clone(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
    }

    async fn exec_in(
        app: &tauri::App<MockRuntime>,
        window: &Window<MockRuntime>,
        channel: &str,
    ) -> u64 {
        crate::exec::start_pod_exec(
            "silent".into(),
            "ns".into(),
            "pod".into(),
            None,
            None,
            None,
            channel.into(),
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

    async fn logs_in(app: &tauri::App<MockRuntime>, window: &Window<MockRuntime>, channel: &str) {
        crate::logs::start_log_stream(
            "silent".into(),
            "ns".into(),
            vec![srelens_streams::logs::LogTarget {
                pod: "pod".into(),
                container: None,
                label: String::new(),
            }],
            channel.into(),
            None,
            None,
            None,
            app.handle().clone(),
            window.clone(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
    }

    /// A log stream follows its pod until something ends it, so a window that
    /// closes or reloads ends its own (#735) — the follow connections are
    /// dropped, not just forgotten — and leaves another window's following.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_closed_or_reloaded_window_ends_its_log_streams_and_no_other_windows() {
        let server = Silent::start();
        let (app, main, other) = server.app();
        logs_in(&app, &main, "logs:main").await;
        logs_in(&app, &other, "logs:other").await;
        eventually("both streams reached the cluster", || server.open() >= 2).await;

        on_window_event(&main, &WindowEvent::Destroyed);
        eventually("the closed window's stream dropped", || {
            server.dropped.load(Ordering::SeqCst) >= 1
        })
        .await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(server.open(), 1, "the other window's stream follows on");

        let ended = window_streams_reset(other, app.handle().clone())
            .await
            .unwrap();
        assert_eq!(ended.log_streams, 1);
        eventually("the reloaded window's stream dropped", || server.open() == 0).await;
    }

    async fn forward_in(
        app: &tauri::App<MockRuntime>,
        window: &Window<MockRuntime>,
        local_port: Option<u16>,
    ) -> srelens_streams::forward::ForwardInfo {
        crate::forward::start_port_forward(
            "silent".into(),
            "ns".into(),
            "Pod".into(),
            "pod".into(),
            8080,
            local_port,
            app.handle().clone(),
            window.clone(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap()
    }

    /// Whether nothing on this machine is listening on loopback `port`.
    fn port_is_free(port: u16) -> bool {
        std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
    }

    /// A port-forward holds its local port for as long as it runs, so one a
    /// closed or reloaded window left behind kept the port bound and the next
    /// forward to it failed (#735). The window's forwards end and their ports
    /// come free; another window's keeps its own.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_closed_or_reloaded_window_ends_its_forwards_and_frees_their_ports() {
        let server = Silent::start();
        let (app, main, other) = server.app();
        let mine = forward_in(&app, &main, None).await;
        let theirs = forward_in(&app, &other, None).await;
        eventually("both forwards reached the cluster", || server.open() >= 2).await;
        assert!(!port_is_free(mine.local_port) && !port_is_free(theirs.local_port));

        on_window_event(&main, &WindowEvent::Destroyed);
        eventually("the closed window's port came free", || {
            port_is_free(mine.local_port)
        })
        .await;
        assert!(
            !port_is_free(theirs.local_port),
            "the other window's forward keeps its port"
        );
        // What a leftover forward used to refuse: a new one on the same port.
        let again = forward_in(&app, &main, Some(mine.local_port)).await;
        assert_eq!(again.local_port, mine.local_port);

        let ended = window_streams_reset(other, app.handle().clone())
            .await
            .unwrap();
        assert_eq!(ended.forwards, 1);
        eventually("the reloaded window's port came free", || {
            port_is_free(theirs.local_port)
        })
        .await;
    }

    /// Each of `events` the app has broadcast, with its payload, in order.
    type Heard = Arc<Mutex<Vec<(String, serde_json::Value)>>>;

    fn heard(app: &tauri::App<MockRuntime>, events: &[&str]) -> Heard {
        let heard = Heard::default();
        for event in events {
            let (heard, name) = (heard.clone(), (*event).to_owned());
            app.listen_any(*event, move |e| {
                let payload = serde_json::from_str(e.payload()).unwrap_or_default();
                heard.lock().unwrap().push((name.clone(), payload));
            });
        }
        heard
    }

    fn names(heard: &Heard) -> Vec<String> {
        heard.lock().unwrap().iter().map(|(n, _)| n.clone()).collect()
    }

    /// A terminal runs `$SHELL`, else `/bin/bash`, and on Windows neither is
    /// there, so these tests give it `cmd.exe`. Nothing else in this crate's
    /// tests reads `SHELL`.
    fn a_local_shell() {
        #[cfg(windows)]
        std::env::set_var(
            "SHELL",
            std::env::var("COMSPEC").unwrap_or_else(|_| r"C:\Windows\System32\cmd.exe".into()),
        );
    }

    async fn terminal_in(
        app: &tauri::App<MockRuntime>,
        window: &Window<MockRuntime>,
        server: &Silent,
        channel: &str,
    ) -> u64 {
        crate::terminal::start_terminal(
            "silent".into(),
            vec![server.kubeconfig_path()],
            channel.into(),
            None,
            None,
            None,
            app.handle().clone(),
            window.clone(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap()
    }

    /// A local terminal is a shell on this machine, running until it is
    /// killed, so a window that closes or reloads kills its own (#735) — its
    /// exit is heard only once the shell is really gone — and leaves another
    /// window's running.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_closed_or_reloaded_window_ends_its_terminals_and_no_other_windows() {
        a_local_shell();
        let server = Silent::start();
        let (app, main, other) = server.app();
        let exited = heard(&app, &["term:exit:t-main", "term:exit:t-other"]);
        terminal_in(&app, &main, &server, "t-main").await;
        terminal_in(&app, &other, &server, "t-other").await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(names(&exited).is_empty(), "both shells are running");

        on_window_event(&main, &WindowEvent::Destroyed);
        eventually("the closed window's shell exited", || {
            !names(&exited).is_empty()
        })
        .await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            names(&exited),
            ["term:exit:t-main"],
            "the other window's shell runs on"
        );

        let ended = window_streams_reset(other, app.handle().clone())
            .await
            .unwrap();
        assert_eq!(ended.terminals, 1);
        eventually("the reloaded window's shell exited", || {
            names(&exited).len() == 2
        })
        .await;
    }

    /// A stand-in for `helm` that takes a second, then exits 1 for release
    /// `web-fail` and 0 for any other: an upgrade still running when its
    /// window goes.
    fn slow_helm(dir: &std::path::Path) -> std::path::PathBuf {
        #[cfg(windows)]
        let (path, script) = (
            dir.join("helm.cmd"),
            "@echo off\r\nping -n 2 127.0.0.1 >nul\r\necho helm %1 %2\r\n\
             if \"%2\"==\"web-fail\" exit /b 1\r\nexit /b 0\r\n",
        );
        #[cfg(not(windows))]
        let (path, script) = (
            dir.join("helm"),
            "#!/bin/sh\nsleep 1\necho helm $1 $2\n[ \"$2\" = web-fail ] && exit 1\nexit 0\n",
        );
        std::fs::write(&path, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    async fn helm_in(
        app: &tauri::App<MockRuntime>,
        window: &Window<MockRuntime>,
        server: &Silent,
        release: &str,
        channel: &str,
    ) -> u64 {
        crate::helm::start_helm_op(
            "silent".into(),
            vec![server.kubeconfig_path()],
            vec!["upgrade".into(), release.into(), "./chart".into()],
            String::new(),
            channel.into(),
            app.handle().clone(),
            window.clone(),
            app.state(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap()
    }

    /// A helm operation is a change to the cluster partway through, and
    /// killing helm can leave the release half-applied. So a window that
    /// closes or reloads lets its operations run to the end — each exits on
    /// its own — and the host reports how each ended, since no page is left
    /// to (#735). An operation whose window is still open is its page's.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_helm_operation_outlives_its_window_and_how_it_ended_is_reported() {
        let server = Silent::start();
        let (app, main, other) = server.app();
        let bin = tempfile::tempdir().unwrap();
        app.manage(HelmManager::with_helm(slow_helm(bin.path())));
        let heard = heard(
            &app,
            &[
                "helm:exit:h-ok",
                "helm:exit:h-fail",
                "helm:exit:h-other",
                "host-notice",
            ],
        );
        helm_in(&app, &main, &server, "web-ok", "h-ok").await;
        helm_in(&app, &main, &server, "web-fail", "h-fail").await;
        helm_in(&app, &other, &server, "api", "h-other").await;

        let ended = end_window(app.handle(), "main", CloseReason::WindowClosed);
        eventually("every operation ran to its end", || {
            names(&heard).iter().filter(|n| n.starts_with("helm:exit")).count() == 3
        })
        .await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        let notices: Vec<serde_json::Value> = heard
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == "host-notice")
            .map(|(_, p)| p.clone())
            .collect();
        assert_eq!(notices.len(), 2, "one report per operation: {notices:?}");
        let finished = notices.iter().find(|n| n["level"] == "info").unwrap();
        assert_eq!(finished["title"], "helm upgrade web-ok finished");
        let failed = notices.iter().find(|n| n["level"] == "error").unwrap();
        assert_eq!(failed["title"], "helm upgrade web-fail failed");
        let detail = failed["detail"].as_str().unwrap();
        assert!(detail.contains("helm exited with code 1"), "{detail}");
        assert_eq!(ended.helm_left_running, 2);
    }

    /// A destroyed window's watches and shells end — the connections to the
    /// cluster are dropped, not just forgotten — and another window's, over
    /// the same cluster, keep running.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_closed_window_ends_its_watches_and_shells_and_no_other_windows() {
        let server = Silent::start();
        let (app, main, other) = server.app();
        watch_in(&app, &main, "watch:main").await;
        let shell = exec_in(&app, &main, "exec-main").await;
        watch_in(&app, &other, "watch:other").await;
        let other_shell = exec_in(&app, &other, "exec-other").await;
        let watches = app.state::<WatchManager>();
        let execs = app.state::<ExecManager>();
        eventually("every stream reached the cluster", || server.open() >= 4).await;
        assert!(watches.has_channel("watch:main") && execs.has_session(shell));

        on_window_event(&main, &WindowEvent::Focused(true));
        assert!(
            watches.has_channel("watch:main"),
            "only Destroyed ends streams"
        );
        on_window_event(&main, &WindowEvent::Destroyed);

        assert!(!watches.has_channel("watch:main"));
        assert!(!execs.has_session(shell));
        assert!(
            watches.has_channel("watch:other"),
            "another window's watch runs on"
        );
        assert!(execs.has_session(other_shell), "and its shell");
        eventually("the closed window's connections dropped", || {
            server.dropped.load(Ordering::SeqCst) >= 2
        })
        .await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(server.open() >= 2, "the other window's connections stay up");

        on_window_event(&other, &WindowEvent::Destroyed);
        eventually("every connection dropped", || server.open() == 0).await;
        assert!(!watches.has_channel("watch:other") && !execs.has_session(other_shell));
    }

    /// A reload keeps the window and loses the page: the new page's reset
    /// ends what the old one opened, answers what it ended, and leaves what
    /// the window opens next alone. A stream the old page stopped itself is
    /// not counted again.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_reset_ends_what_the_window_held_before_its_reload() {
        let server = Silent::start();
        let (app, main, _) = server.app();
        watch_in(&app, &main, "watch:1").await;
        watch_in(&app, &main, "watch:2").await;
        let shell = exec_in(&app, &main, "exec-1").await;
        crate::watch::stop_watch("watch:2".into(), main.clone(), app.state(), app.state())
            .await
            .unwrap();

        let ended = window_streams_reset(main.clone(), app.handle().clone())
            .await
            .unwrap();
        assert_eq!(
            ended,
            WindowStreamsEnded {
                watches: 1,
                execs: 1,
                ..Default::default()
            }
        );
        let watches = app.state::<WatchManager>();
        assert!(!watches.has_channel("watch:1"));
        assert!(!app.state::<ExecManager>().has_session(shell));
        assert_eq!(
            serde_json::to_value(&ended).unwrap(),
            serde_json::json!({"appStreams": 0, "watches": 1, "execs": 1, "logStreams": 0, "forwards": 0, "terminals": 0, "helmLeftRunning": 0, "debugPods": 0})
        );

        // The reloaded page opens afresh; the next reset ends only that.
        watch_in(&app, &main, "watch:3").await;
        assert!(watches.has_channel("watch:3"));
        let ended = window_streams_reset(main.clone(), app.handle().clone())
            .await
            .unwrap();
        assert_eq!(ended.watches, 1);
        assert!(!watches.has_channel("watch:3"));
    }

    /// The race a reload opens: the old page asked for a watch and a shell,
    /// and the window reset while the host was starting them. What the old
    /// page asked for must not survive it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_start_that_lands_after_its_window_reset_is_stopped() {
        let server = Silent::start();
        let (app, main, _) = server.app();
        let owned = app.state::<WindowStreams>();
        let watches = app.state::<WatchManager>();
        let execs = app.state::<ExecManager>();
        // What the start commands do, with the reset landing between their
        // epoch read and their record: the start itself is already running.
        let epoch = owned.epoch("main");
        let sink = Arc::new(crate::sink::TauriSink(app.handle().clone()));
        let channel = watches
            .start(
                sink.clone(),
                "silent".into(),
                "ns".into(),
                "pods".into(),
                "watch:late".into(),
                vec![],
            )
            .await
            .unwrap();
        let session = execs
            .start(
                sink,
                "silent".into(),
                "ns".into(),
                "pod".into(),
                "exec-late".into(),
                Default::default(),
            )
            .await
            .unwrap();
        eventually("both reached the cluster", || server.open() >= 2).await;
        end_window(app.handle(), "main", CloseReason::WindowReloaded);

        let refused = owned
            .keep("main", epoch, Stream::Watch(channel.clone()), || {
                watches.stop(&channel)
            })
            .unwrap_err();
        assert!(refused.contains("closed or reloaded"), "{refused}");
        let refused = owned
            .keep("main", epoch, Stream::Exec(session), || execs.close(session))
            .unwrap_err();
        assert!(refused.contains("closed or reloaded"), "{refused}");
        assert!(!watches.has_channel("watch:late"));
        assert!(!execs.has_session(session));
        eventually("their connections dropped", || server.open() == 0).await;

        // A start after the reset is the new page's, and runs.
        watch_in(&app, &main, "watch:ok").await;
        assert!(watches.has_channel("watch:ok"));
        assert_eq!(
            window_streams_reset(main, app.handle().clone())
                .await
                .unwrap()
                .watches,
            1,
            "and is the window's to end"
        );
    }

    /// The #735 kinds are held the way #733 holds watches and shells: another
    /// window cannot stop a log stream, type into, resize or close a local
    /// terminal — a shell on this machine — or abort a helm operation, which
    /// killed partway leaves a release half-applied. Each is refused by name
    /// and runs on. A port-forward is the exception: every window's Forwards
    /// screen lists every forward (`list_forwards`), so any window may stop one
    /// — and the window that opened it then no longer counts it as its own.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_window_cannot_stop_another_windows_log_stream_terminal_or_helm_operation() {
        a_local_shell();
        let server = Silent::start();
        let (app, main, other) = server.app();
        let bin = tempfile::tempdir().unwrap();
        app.manage(HelmManager::with_helm(slow_helm(bin.path())));
        let heard = heard(&app, &["term:exit:t-main", "helm:exit:h-main"]);
        logs_in(&app, &main, "logs:main").await;
        let terminal = terminal_in(&app, &main, &server, "t-main").await;
        let helm = helm_in(&app, &main, &server, "web", "h-main").await;
        let forward = forward_in(&app, &main, None).await;
        eventually("the log stream and the forward reached the cluster", || {
            server.open() >= 2
        })
        .await;

        let refusals = [
            crate::logs::stop_log_stream("logs:main".into(), other.clone(), app.state(), app.state())
                .await,
            crate::terminal::terminal_input(
                terminal,
                "exit\r\n".into(),
                other.clone(),
                app.state(),
                app.state(),
            )
            .await,
            crate::terminal::terminal_resize(terminal, 1, 1, other.clone(), app.state(), app.state())
                .await,
            crate::terminal::terminal_close(terminal, other.clone(), app.state(), app.state()).await,
            crate::helm::helm_op_close(helm, other.clone(), app.state(), app.state(), app.state())
                .await,
        ];
        for refused in refusals {
            let refused = refused.unwrap_err();
            assert!(refused.contains("not opened by this window"), "{refused}");
        }
        eventually("helm ran to its end, never aborted", || {
            names(&heard).iter().any(|n| n == "helm:exit:h-main")
        })
        .await;
        assert!(
            !names(&heard).iter().any(|n| n == "term:exit:t-main"),
            "the shell runs on"
        );
        assert!(server.open() >= 2, "the log stream follows on");

        crate::forward::stop_port_forward(forward.id, other.clone(), app.state(), app.state())
            .await
            .unwrap();
        eventually("the forward's port came free", || {
            port_is_free(forward.local_port)
        })
        .await;
        let ended = window_streams_reset(main, app.handle().clone())
            .await
            .unwrap();
        assert_eq!(
            (ended.log_streams, ended.terminals, ended.forwards),
            (1, 1, 0),
            "main ends its own, and not the forward another window stopped"
        );
    }

    /// Another window cannot stop a watch or drive, resize or close a shell
    /// it did not open (#733), though it knows the channel or the session id.
    /// Each is refused by name, the stream runs on, and the window that
    /// opened it still can.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_window_cannot_stop_or_drive_another_windows_streams() {
        let server = Silent::start();
        let (app, main, other) = server.app();
        watch_in(&app, &main, "watch:main").await;
        let shell = exec_in(&app, &main, "exec-main").await;
        let watches = app.state::<WatchManager>();
        let execs = app.state::<ExecManager>();
        eventually("both reached the cluster", || server.open() >= 2).await;

        let refused =
            crate::watch::stop_watch("watch:main".into(), other.clone(), app.state(), app.state())
                .await
                .unwrap_err();
        assert!(refused.contains("not opened by this window"), "{refused}");
        let refused = crate::exec::exec_input(
            shell,
            "rm -rf /\n".into(),
            other.clone(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap_err();
        assert!(refused.contains("not opened by this window"), "{refused}");
        assert!(
            crate::exec::exec_resize(shell, 1, 1, other.clone(), app.state(), app.state())
                .await
                .is_err()
        );
        assert!(
            crate::exec::exec_close(
                shell,
                other.clone(),
                app.handle().clone(),
                app.state(),
                app.state(),
                app.state()
            )
            .await
            .is_err()
        );
        assert!(watches.has_channel("watch:main"), "the watch runs on");
        assert!(execs.has_session(shell), "and the shell");
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(server.open() >= 2, "no connection was dropped");

        crate::exec::exec_input(shell, "ls\n".into(), main.clone(), app.state(), app.state())
            .await
            .unwrap();
        crate::exec::exec_close(
            shell,
            main.clone(),
            app.handle().clone(),
            app.state(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
        crate::watch::stop_watch("watch:main".into(), main.clone(), app.state(), app.state())
            .await
            .unwrap();
        assert!(!watches.has_channel("watch:main") && !execs.has_session(shell));
        eventually("the owner's stops dropped the connections", || {
            server.open() == 0
        })
        .await;
    }

    #[test]
    fn a_host_without_the_managers_ends_nothing_and_does_not_panic() {
        let app = tauri::test::mock_app();
        assert_eq!(
            end_window(app.handle(), "main", CloseReason::WindowClosed),
            WindowStreamsEnded::default()
        );
    }

    #[test]
    fn an_ending_hands_back_what_the_window_owned_and_moves_its_epoch() {
        let watch = |channel: &str| Stream::Watch(channel.to_owned());
        let owned = WindowStreams::default();
        assert_eq!(owned.epoch("main"), 0);
        assert!(owned.own("main", 0, watch("watch:a")));
        assert!(owned.own("main", 0, watch("watch:b")));
        assert!(owned.own("main", 0, Stream::Exec(7)));
        assert!(owned.own("ctx-1", 0, watch("watch:c")));
        owned.disown("main", &watch("watch:b"));
        owned.disown("nobody", &Stream::Exec(7));
        owned.disown("nobody", &watch("watch:a"));

        let mut taken = owned.take("main");
        taken.sort();
        assert_eq!(taken, [watch("watch:a"), Stream::Exec(7)]);
        assert_eq!(owned.epoch("main"), 1);
        assert_eq!(owned.epoch("ctx-1"), 0, "another window's epoch is its own");
        // A start begun before the ending is refused, and records nothing.
        assert!(!owned.own("main", 0, watch("watch:late")));
        assert!(!owned.own("main", 0, Stream::Exec(8)));
        assert_eq!(owned.take("main"), []);
        assert_eq!(owned.take("ctx-1"), [watch("watch:c")]);
        owned.disown("main", &Stream::Exec(7));
    }
}
