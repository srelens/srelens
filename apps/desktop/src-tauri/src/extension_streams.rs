//! Tauri adapter for app streams (#565): the contract and its lifecycle live
//! in `srelens_streams::app` and `srelens_registry`; this module only maps the
//! command surface and hands each stream a `ChannelSink` over the `onEvent`
//! channel its page passed, so its frames reach that page and no other
//! window (#733).

use std::sync::Arc;

use serde_json::Value;
use srelens_registry::{ExtensionStreams, OpenStreamOut};
use tauri::ipc::Channel;
use tauri::{AppHandle, Runtime, State, Window};

use crate::bridge::AppAudit;
use crate::sink::{ChannelSink, TauriSink};

/// The registry's app streams, or `None` on a host built without apps.
pub struct AppExtensionStreams(pub Option<Arc<ExtensionStreams>>);

impl AppExtensionStreams {
    fn get(&self) -> Result<&ExtensionStreams, String> {
        self.0
            .as_deref()
            .ok_or_else(|| "Apps are not available on this host".to_string())
    }
}

/// Send every inventory write the registry announces to the WebView as the
/// `extensions:inventory` event (#566), so the app list reads again when it
/// changes instead of polling. Harmless on a host without apps.
pub fn listen_inventory<R: Runtime>(streams: &Option<Arc<ExtensionStreams>>, app: &AppHandle<R>) {
    if let Some(streams) = streams {
        streams.listen_inventory(Arc::new(TauriSink(app.clone())));
    }
}

/// Open a stream for one view of an app. `input` is `@srelens/core`'s
/// `openExtensionView(…).open(…)` payload, parsed by the registry. The
/// stream belongs to the calling window too, and ends when it closes or
/// reloads (#700); the label is Tauri's, never the page's. Exec and
/// port-forward sessions (#567) go to the same audit trail every other
/// mutating or sensitive call from the app does (#555).
#[tauri::command]
pub async fn extension_stream_open<R: Runtime>(
    input: Value,
    on_event: Channel<Value>,
    window: Window<R>,
    streams: State<'_, AppExtensionStreams>,
    audit: State<'_, AppAudit>,
) -> Result<OpenStreamOut, String> {
    streams
        .get()?
        .open_in_window(
            Arc::new(ChannelSink(on_event)),
            window.label(),
            audit.0.clone(),
            input,
        )
        .await
}

/// Cancel one stream the calling window opened (#733); another window's is
/// refused. Idempotent: `false` when it had already ended.
#[tauri::command]
pub async fn extension_stream_cancel<R: Runtime>(
    stream: String,
    window: Window<R>,
    streams: State<'_, AppExtensionStreams>,
) -> Result<bool, String> {
    streams.get()?.cancel_in_window(&stream, window.label())
}

/// End every stream a view of the calling window opened, as the view closes.
/// Another window's view of the same name is its own (#733).
#[tauri::command]
pub async fn extension_stream_close_view<R: Runtime>(
    view: String,
    window: Window<R>,
    streams: State<'_, AppExtensionStreams>,
) -> Result<usize, String> {
    Ok(streams.get()?.close_view_in_window(&view, window.label()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use srelens_kube::client_cache::ClientCache;
    use tauri::Manager;

    /// The three commands against a MockRuntime, over a real registry with an
    /// empty inventory: an open for an app that is not installed is refused
    /// with the broker's reason, and cancel and close are harmless no-ops.
    #[tokio::test(flavor = "multi_thread")]
    async fn commands_reach_the_registrys_streams() {
        let dir = tempfile::tempdir().unwrap();
        let (_, streams) = srelens_registry::build_registry_and_app_streams(
            ClientCache::new_many(vec![]),
            vec![],
            Some(dir.path().join("settings.json")),
        );
        assert!(streams.is_some(), "a desktop build has app streams");
        let app = tauri::test::mock_app();
        app.manage(AppExtensionStreams(streams));
        app.manage(AppAudit(Arc::new(srelens_capability::audit::NoopAudit)));
        let window = crate::window_streams::tests::mock_window(&app, "main");

        let input = json!({
            "id": "org.example.none", "revision": 1, "view": "v", "channel": "extstream:t",
            "context": "c", "source": {"kind": "read", "capability": "things"},
        });
        let refused = extension_stream_open(
            input,
            crate::sink::tests::recording().0,
            window.clone(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap_err();
        assert!(refused.contains("removed"), "{refused}");
        assert!(
            !extension_stream_cancel("s-1".into(), window.clone(), app.state())
                .await
                .unwrap()
        );
        assert_eq!(
            extension_stream_close_view("v".into(), window.clone(), app.state())
                .await
                .unwrap(),
            0
        );
    }

    /// An app stream's frames reach only the page that opened it (#733): on
    /// the channel passed with the open, tagged with the stream's channel
    /// name, and never as an event every window would hear. Another window
    /// cannot cancel it.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_app_streams_frames_go_only_to_the_window_that_opened_it() {
        use std::sync::Mutex;
        use tauri::Listener;
        let dir = tempfile::tempdir().unwrap();
        let (registry, streams) = srelens_registry::build_registry_and_app_streams(
            ClientCache::new_many(vec![]),
            vec![],
            Some(dir.path().join("settings.json")),
        );
        let manifest = json!({
            "id": "org.example.one", "name": "One", "version": "0.1.0",
            "srelensApiVersion": "^0.5", "kind": "declarative",
            "permissions": ["k8s.listDeployments"],
            "capabilities": [{"name": "workloads", "title": "Workloads",
                "target": "k8s.listDeployments", "inputs": ["context", "namespace"],
                "arguments": {}}],
            "contributions": {"pages": [], "detailTabs": [], "detailLinks": []}
        })
        .to_string();
        registry
            .invoke(
                "extensions.configure",
                json!({"action": "unsignedApps", "allowUnsignedApps": true}),
            )
            .await
            .unwrap();
        let installed = registry
            .invoke(
                "extensions.configure",
                json!({"action": "install", "manifest": manifest,
                       "grants": ["k8s.listDeployments"]}),
            )
            .await
            .unwrap();
        let revision = installed["plugins"][0]["revision"].as_u64().unwrap();
        let app = tauri::test::mock_app();
        app.manage(AppExtensionStreams(streams));
        app.manage(AppAudit(Arc::new(srelens_capability::audit::NoopAudit)));
        let main = crate::window_streams::tests::mock_window(&app, "main");
        let other = crate::window_streams::tests::mock_window(&app, "ctx-1");
        let broadcast = Arc::new(Mutex::new(Vec::<String>::new()));
        let heard = broadcast.clone();
        app.listen_any("extstream:one", move |e| {
            heard.lock().unwrap().push(e.payload().to_owned());
        });

        let (on_event, got) = crate::sink::tests::recording();
        let opened = extension_stream_open(
            json!({"id": "org.example.one", "revision": revision, "view": "page#1",
                   "channel": "extstream:one", "context": "c", "namespace": "ns",
                   "source": {"kind": "read", "capability": "workloads"}}),
            on_event,
            main.clone(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();

        let got = got.lock().unwrap().clone();
        assert_eq!(
            got.first(),
            Some(&json!({"event": "extstream:one",
                         "payload": {"type": "open", "stream": opened.stream}})),
            "the open frame reached its window: {got:?}"
        );
        let refused = extension_stream_cancel(opened.stream.clone(), other, app.state())
            .await
            .unwrap_err();
        assert!(refused.contains("not opened by this window"), "{refused}");
        assert!(extension_stream_cancel(opened.stream, main, app.state())
            .await
            .unwrap());
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(
            *broadcast.lock().unwrap(),
            Vec::<String>::new(),
            "nothing broadcast"
        );
    }

    /// An inventory write made through the registry reaches the window as
    /// the event the app list listens for.
    #[tokio::test(flavor = "multi_thread")]
    async fn inventory_writes_reach_the_window_as_an_event() {
        use std::sync::{Arc, Mutex};
        use tauri::Listener;
        let dir = tempfile::tempdir().unwrap();
        let (registry, streams) = srelens_registry::build_registry_and_app_streams(
            ClientCache::new_many(vec![]),
            vec![],
            Some(dir.path().join("settings.json")),
        );
        let app = tauri::test::mock_app();
        let heard = Arc::new(Mutex::new(Vec::<String>::new()));
        let record = heard.clone();
        app.listen(srelens_registry::INVENTORY_CHANNEL, move |event| {
            record.lock().unwrap().push(event.payload().to_owned());
        });
        listen_inventory(&streams, app.handle());
        registry
            .invoke(
                "extensions.configure",
                json!({"action": "unsignedApps", "allowUnsignedApps": true}),
            )
            .await
            .unwrap();
        for _ in 0..100 {
            if !heard.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(*heard.lock().unwrap(), [r#"{"type":"changed"}"#]);
        listen_inventory(&None, app.handle());
    }

    #[tokio::test]
    async fn a_host_without_apps_says_so() {
        let app = tauri::test::mock_app();
        app.manage(AppExtensionStreams(None));
        app.manage(AppAudit(Arc::new(srelens_capability::audit::NoopAudit)));
        let window = crate::window_streams::tests::mock_window(&app, "main");
        let refused = extension_stream_open(
            json!({}),
            crate::sink::tests::recording().0,
            window.clone(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap_err();
        assert_eq!(refused, "Apps are not available on this host");
        assert!(
            extension_stream_cancel("s".into(), window.clone(), app.state())
                .await
                .is_err()
        );
        assert!(
            extension_stream_close_view("v".into(), window.clone(), app.state())
                .await
                .is_err()
        );
    }
}
