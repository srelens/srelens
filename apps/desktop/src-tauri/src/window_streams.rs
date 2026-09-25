//! A window's streams end with the window (#700).
//!
//! Every stream a WebView opens — an app stream, a built-in resource watch,
//! a pod exec — is recorded against the label of the window that opened it.
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
use srelens_streams::watch::WatchManager;
use tauri::{AppHandle, Manager, Runtime, Window, WindowEvent};

use crate::extension_streams::AppExtensionStreams;

/// The built-in streams each window opened, and each window's epoch.
#[derive(Default)]
pub struct WindowStreams {
    windows: Mutex<HashMap<String, Owned>>,
}

#[derive(Default)]
struct Owned {
    epoch: u64,
    watches: HashSet<String>,
    execs: HashSet<u64>,
}

/// What one ending stopped. Answered by [`window_streams_reset`], and logged.
#[derive(Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowStreamsEnded {
    pub app_streams: usize,
    pub watches: usize,
    pub execs: usize,
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

    /// Record watch `channel` as `window`'s. `false` when the window ended
    /// since `epoch`: the caller stops the watch, since nothing else will.
    pub fn own_watch(&self, window: &str, epoch: u64, channel: &str) -> bool {
        self.own(window, epoch, |owned| {
            owned.watches.insert(channel.to_owned());
        })
    }

    /// Record exec `session` as `window`'s; as [`WindowStreams::own_watch`].
    pub fn own_exec(&self, window: &str, epoch: u64, session: u64) -> bool {
        self.own(window, epoch, |owned| {
            owned.execs.insert(session);
        })
    }

    /// Forget watch `channel`, as the window stops it itself.
    pub fn disown_watch(&self, window: &str, channel: &str) {
        if let Some(owned) = self.windows.lock().unwrap().get_mut(window) {
            owned.watches.remove(channel);
        }
    }

    /// Forget exec `session`, as the window closes it itself.
    pub fn disown_exec(&self, window: &str, session: u64) {
        if let Some(owned) = self.windows.lock().unwrap().get_mut(window) {
            owned.execs.remove(&session);
        }
    }

    /// A watch that started on `channel` for `window` at `epoch`: recorded as
    /// the window's, or — the window ended while it started — stopped, and
    /// refused. The start commands end with this.
    pub fn keep_watch(
        &self,
        manager: &WatchManager,
        window: &str,
        epoch: u64,
        channel: String,
    ) -> Result<String, String> {
        if self.own_watch(window, epoch, &channel) {
            return Ok(channel);
        }
        manager.stop(&channel);
        Err(format!(
            "The window {window} closed or reloaded while this watch was starting"
        ))
    }

    /// [`WindowStreams::keep_watch`], for an exec session.
    pub fn keep_exec(
        &self,
        manager: &ExecManager,
        window: &str,
        epoch: u64,
        session: u64,
    ) -> Result<u64, String> {
        if self.own_exec(window, epoch, session) {
            return Ok(session);
        }
        manager.close(session);
        Err(format!(
            "The window {window} closed or reloaded while this shell was starting"
        ))
    }

    fn own(&self, window: &str, epoch: u64, record: impl FnOnce(&mut Owned)) -> bool {
        let mut windows = self.windows.lock().unwrap();
        let owned = windows.entry(window.to_owned()).or_default();
        if owned.epoch != epoch {
            return false;
        }
        record(owned);
        true
    }

    /// Bump `window`'s epoch and hand back everything it owned. The entry
    /// stays, holding the epoch: labels are reused (a context's window opens
    /// under the same label every time), and a start begun before this must
    /// still see that the window moved on.
    fn take(&self, window: &str) -> (Vec<String>, Vec<u64>) {
        let mut windows = self.windows.lock().unwrap();
        let owned = windows.entry(window.to_owned()).or_default();
        owned.epoch += 1;
        (
            owned.watches.drain().collect(),
            owned.execs.drain().collect(),
        )
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
    let (watches, execs) = app
        .try_state::<WindowStreams>()
        .map(|owned| owned.take(window))
        .unwrap_or_default();
    if let Some(manager) = app.try_state::<WatchManager>() {
        for channel in &watches {
            manager.stop(channel);
        }
    }
    if let Some(manager) = app.try_state::<ExecManager>() {
        for session in &execs {
            manager.close(*session);
        }
    }
    let app_streams = app
        .try_state::<AppExtensionStreams>()
        .and_then(|streams| {
            streams
                .0
                .as_ref()
                .map(|streams| streams.end_window(window, reason))
        })
        .unwrap_or(0);
    let ended = WindowStreamsEnded {
        app_streams,
        watches: watches.len(),
        execs: execs.len(),
    };
    if ended != WindowStreamsEnded::default() {
        log::info!(
            "window {window} ({reason:?}): ended {} app streams, {} watches, {} exec sessions",
            ended.app_streams,
            ended.watches,
            ended.execs
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

    /// An API server that accepts connections and never answers, so a watch
    /// or exec aimed at it stays running until it is stopped. Counts the
    /// connections it accepted and the ones the client has since dropped.
    struct Silent {
        kubeconfig: tempfile::TempDir,
        accepted: Arc<AtomicUsize>,
        dropped: Arc<AtomicUsize>,
    }

    impl Silent {
        fn start() -> Self {
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

        /// A MockRuntime app with every stream manager this module ends,
        /// reaching this server, and windows `main` and `ctx-1`.
        fn app(
            &self,
        ) -> (
            tauri::App<MockRuntime>,
            Window<MockRuntime>,
            Window<MockRuntime>,
        ) {
            let cache = srelens_kube::client_cache::ClientCache::new_many(vec![self
                .kubeconfig
                .path()
                .join("config")]);
            let app = tauri::test::mock_app();
            app.manage(WatchManager::new(cache.clone()));
            app.manage(ExecManager::new(cache));
            app.manage(WindowStreams::default());
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
            window.clone(),
            app.handle().clone(),
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
            window.clone(),
            app.handle().clone(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap()
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
                app_streams: 0,
                watches: 1,
                execs: 1
            }
        );
        let watches = app.state::<WatchManager>();
        assert!(!watches.has_channel("watch:1"));
        assert!(!app.state::<ExecManager>().has_session(shell));
        assert_eq!(
            serde_json::to_value(&ended).unwrap(),
            serde_json::json!({"appStreams": 0, "watches": 1, "execs": 1})
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
            .keep_watch(&watches, "main", epoch, channel)
            .unwrap_err();
        assert!(refused.contains("closed or reloaded"), "{refused}");
        let refused = owned.keep_exec(&execs, "main", epoch, session).unwrap_err();
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
        let owned = WindowStreams::default();
        assert_eq!(owned.epoch("main"), 0);
        assert!(owned.own_watch("main", 0, "watch:a"));
        assert!(owned.own_watch("main", 0, "watch:b"));
        assert!(owned.own_exec("main", 0, 7));
        assert!(owned.own_watch("ctx-1", 0, "watch:c"));
        owned.disown_watch("main", "watch:b");
        owned.disown_exec("nobody", 7);
        owned.disown_watch("nobody", "watch:a");

        let (watches, execs) = owned.take("main");
        assert_eq!((watches, execs), (vec!["watch:a".to_owned()], vec![7]));
        assert_eq!(owned.epoch("main"), 1);
        assert_eq!(owned.epoch("ctx-1"), 0, "another window's epoch is its own");
        // A start begun before the ending is refused, and records nothing.
        assert!(!owned.own_watch("main", 0, "watch:late"));
        assert!(!owned.own_exec("main", 0, 8));
        assert_eq!(owned.take("main"), (vec![], vec![]));
        let (watches, _) = owned.take("ctx-1");
        assert_eq!(watches, ["watch:c"]);
        owned.disown_exec("main", 7);
    }
}
