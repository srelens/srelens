//! Tauri implementation of the shared EventSink: events go to the WebView
//! over the exact same channels the frontend already subscribes to.
//!
//! Generic over the runtime so the unit suites can construct it against
//! `tauri::test::MockRuntime`; the app itself always instantiates it with
//! the default (Wry) runtime.

use srelens_streams::EventSink;
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, Runtime};

pub struct TauriSink<R: Runtime>(pub AppHandle<R>);

impl<R: Runtime> EventSink for TauriSink<R> {
    fn emit(&self, channel: &str, payload: serde_json::Value) {
        let _ = self.0.emit(channel, payload);
    }
}

/// A stream's frames, sent only to the page that opened it (#733), over the
/// `tauri::ipc::Channel` it passed with the open. Tauri answers a channel in
/// the webview that made the call, and nowhere else, whereas an emitted event
/// reaches every window listening on its name (even `emit_to` does, to a
/// listener registered for any target). Each frame carries the event name
/// the page subscribed to, as `{ event, payload }`, and `@srelens/core`'s
/// transport hands it to that subscription.
pub struct ChannelSink(pub Channel<serde_json::Value>);

impl EventSink for ChannelSink {
    fn emit(&self, channel: &str, payload: serde_json::Value) {
        let _ = self
            .0
            .send(serde_json::json!({ "event": channel, "payload": payload }));
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use serde_json::Value;
    use srelens_kube::client_cache::ClientCache;
    use srelens_streams::exec::ExecManager;
    use srelens_streams::watch::WatchManager;
    use tauri::ipc::{Channel, InvokeResponseBody};
    use tauri::{Listener, Manager};

    use crate::window_streams::tests::mock_window;
    use crate::window_streams::WindowStreams;

    /// A channel as the page passes one, recording each message it is sent.
    pub(crate) fn recording() -> (Channel<Value>, Arc<Mutex<Vec<Value>>>) {
        let got = Arc::new(Mutex::new(Vec::new()));
        let record = got.clone();
        let channel = Channel::new(move |body| {
            if let InvokeResponseBody::Json(json) = body {
                record
                    .lock()
                    .unwrap()
                    .push(serde_json::from_str(&json).unwrap());
            }
            Ok(())
        });
        (channel, got)
    }

    /// A stream's frames reach only the window that opened it (#733): they
    /// travel on the channel its page passed with the open, tagged with the
    /// event name the page subscribed to, and nothing is broadcast where
    /// every other window's listeners would hear it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_streams_frames_go_only_to_the_channel_of_the_window_that_opened_it() {
        let app = tauri::test::mock_app();
        app.manage(WatchManager::new(ClientCache::new_many(vec![])));
        app.manage(ExecManager::new(ClientCache::new_many(vec![])));
        app.manage(WindowStreams::default());
        app.manage(crate::node_shells::NodeShells::default());
        let main = mock_window(&app, "main");
        let broadcast = Arc::new(Mutex::new(Vec::<String>::new()));
        for event in ["watch:w1", "exec:exit:x1"] {
            let heard = broadcast.clone();
            app.listen_any(event, move |e| {
                heard.lock().unwrap().push(e.payload().to_owned());
            });
        }

        let (on_event, got) = recording();
        // An unresolvable context fails inside each stream's task, so each
        // sends a frame straight away: the watch its error, the shell its exit.
        crate::watch::start_resource_watch(
            "no-such-context".into(),
            "ns".into(),
            "pods".into(),
            "watch:w1".into(),
            vec![],
            on_event.clone(),
            main.clone(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
        crate::exec::start_pod_exec(
            "no-such-context".into(),
            "ns".into(),
            "pod".into(),
            None,
            None,
            None,
            "x1".into(),
            None,
            None,
            on_event,
            main.clone(),
            app.handle().clone(),
            app.state(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();

        for _ in 0..300 {
            let events: Vec<_> = got
                .lock()
                .unwrap()
                .iter()
                .map(|m| m["event"].clone())
                .collect();
            if events.contains(&"watch:w1".into()) && events.contains(&"exec:exit:x1".into()) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let got = got.lock().unwrap().clone();
        let watch = got.iter().find(|m| m["event"] == "watch:w1");
        assert!(
            watch.is_some_and(|m| m["payload"]["error"].is_string()),
            "the watch's error reached its window: {got:?}"
        );
        assert!(
            got.iter().any(|m| m["event"] == "exec:exit:x1"),
            "the shell's exit reached its window: {got:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(
            *broadcast.lock().unwrap(),
            Vec::<String>::new(),
            "nothing broadcast"
        );
    }
}
