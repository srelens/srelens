//! Tauri adapter for app streams (#565): the contract and its lifecycle live
//! in `srelens_streams::app` and `srelens_registry`; this module only maps the
//! command surface and hands each stream a `TauriSink`, so its frames reach
//! the WebView as events on the channel the caller already listens on.

use std::sync::Arc;

use serde_json::Value;
use srelens_registry::{ExtensionStreams, OpenStreamOut};
use tauri::{AppHandle, Runtime, State};

use crate::sink::TauriSink;

/// The registry's app streams, or `None` on a host built without apps.
pub struct AppExtensionStreams(pub Option<Arc<ExtensionStreams>>);

impl AppExtensionStreams {
    fn get(&self) -> Result<&ExtensionStreams, String> {
        self.0
            .as_deref()
            .ok_or_else(|| "Apps are not available on this host".to_string())
    }
}

/// Open a stream for one view of an app. `input` is `@srelens/core`'s
/// `openExtensionView(…).open(…)` payload, parsed by the registry.
#[tauri::command]
pub async fn extension_stream_open<R: Runtime>(
    input: Value,
    app: AppHandle<R>,
    streams: State<'_, AppExtensionStreams>,
) -> Result<OpenStreamOut, String> {
    streams.get()?.open(Arc::new(TauriSink(app)), input).await
}

/// Cancel one stream. Idempotent: `false` when it had already ended.
#[tauri::command]
pub async fn extension_stream_cancel(
    stream: String,
    streams: State<'_, AppExtensionStreams>,
) -> Result<bool, String> {
    Ok(streams.get()?.cancel(&stream))
}

/// End every stream a view opened, as the view closes.
#[tauri::command]
pub async fn extension_stream_close_view(
    view: String,
    streams: State<'_, AppExtensionStreams>,
) -> Result<usize, String> {
    Ok(streams.get()?.close_view(&view))
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

        let input = json!({
            "id": "org.example.none", "revision": 1, "view": "v", "channel": "extstream:t",
            "context": "c", "source": {"kind": "read", "capability": "things"},
        });
        let refused = extension_stream_open(input, app.handle().clone(), app.state())
            .await
            .unwrap_err();
        assert!(refused.contains("removed"), "{refused}");
        assert!(!extension_stream_cancel("s-1".into(), app.state())
            .await
            .unwrap());
        assert_eq!(
            extension_stream_close_view("v".into(), app.state())
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn a_host_without_apps_says_so() {
        let app = tauri::test::mock_app();
        app.manage(AppExtensionStreams(None));
        let refused = extension_stream_open(json!({}), app.handle().clone(), app.state())
            .await
            .unwrap_err();
        assert_eq!(refused, "Apps are not available on this host");
        assert!(extension_stream_cancel("s".into(), app.state())
            .await
            .is_err());
        assert!(extension_stream_close_view("v".into(), app.state())
            .await
            .is_err());
    }
}
