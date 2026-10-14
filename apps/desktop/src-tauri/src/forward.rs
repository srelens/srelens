//! Tauri adapter for port-forwards: the core lives in
//! srelens_streams::forward; this module only maps the Tauri command surface.
//!
//! Commands are generic over the runtime (#28): the unit suite below drives
//! them through `tauri::test::MockRuntime`, so this surface counts toward
//! coverage instead of hiding behind the ignore-regex.

use std::sync::Arc;

use serde::Serialize;
use srelens_streams::forward::{ForwardEntry, ForwardInfo, ForwardManager};
use tauri::{AppHandle, Runtime, State, Window};

use crate::sink::TauriSink;
use crate::window_streams::{Stream, WindowStreams};

/// Start forwarding a local port to a Pod or Service. `kind` is "Pod" or
/// "Service"; a Service is resolved to a backing pod and target port first.
/// Returns the id + bound local port; a `forward:closed:<id>` event fires
/// (with an optional error string) if the forward loop ends on its own.
///
/// The forward belongs to the calling window and stops when it closes or
/// reloads, freeing its local port (#735). One whose window reloaded while it
/// was starting is stopped at once and refused.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn start_port_forward<R: Runtime>(
    context: String,
    namespace: String,
    kind: String,
    name: String,
    remote_port: u16,
    local_port: Option<u16>,
    app: AppHandle<R>,
    window: Window<R>,
    manager: State<'_, ForwardManager>,
    owned: State<'_, WindowStreams>,
) -> Result<ForwardInfo, String> {
    let epoch = owned.epoch(window.label());
    let info = manager
        .start(
            Arc::new(TauriSink(app)),
            context,
            namespace,
            kind,
            name,
            remote_port,
            local_port,
        )
        .await?;
    owned.keep(window.label(), epoch, Stream::Forward(info.id), || {
        manager.stop(info.id)
    })?;
    Ok(info)
}

/// Stop a port-forward and abort its task. Any window may: each window's
/// Forwards screen lists every forward ([`list_forwards`]) and offers to stop
/// it. The window that started it then no longer holds it.
#[tauri::command]
pub async fn stop_port_forward<R: Runtime>(
    id: u64,
    window: Window<R>,
    manager: State<'_, ForwardManager>,
    owned: State<'_, WindowStreams>,
) -> Result<(), String> {
    let _ = window;
    manager.stop(id);
    owned.disown_everywhere(&Stream::Forward(id));
    Ok(())
}

/// The response shape for `list_forwards`, shared with the web command of
/// the same name.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListForwardsResponse {
    pub forwards: Vec<ForwardEntry>,
}

/// List every forward the manager currently holds. On desktop the process
/// dies with the app, so this manager never outlives the frontend store —
/// but the command exists here too so a client has exactly one way to ask,
/// on both platforms, rather than a web-only special case.
#[tauri::command]
pub fn list_forwards(manager: State<'_, ForwardManager>) -> ListForwardsResponse {
    ListForwardsResponse {
        forwards: manager.list(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use srelens_kube::client_cache::ClientCache;
    use tauri::Manager;

    /// start binds the local listener synchronously — an ephemeral port comes
    /// back even though the forward loop itself will die on the unresolvable
    /// context — and stop tears the forward down (twice: unknown id no-ops).
    #[tokio::test(flavor = "multi_thread")]
    async fn commands_run_against_a_mock_runtime() {
        let app = tauri::test::mock_app();
        app.manage(ForwardManager::new(ClientCache::new_many(vec![])));
        app.manage(WindowStreams::default());
        let window = crate::window_streams::tests::mock_window(&app, "main");

        let info = start_port_forward(
            "no-such-context".into(),
            "ns".into(),
            "Pod".into(),
            "pod-0".into(),
            8080,
            None,
            app.handle().clone(),
            window.clone(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
        assert_ne!(info.local_port, 0, "an ephemeral port must have been bound");
        assert!(info.started_at > 0, "the start response must carry its stamp");

        stop_port_forward(info.id, window.clone(), app.state(), app.state())
            .await
            .unwrap();
        stop_port_forward(info.id + 1, window, app.state(), app.state())
            .await
            .unwrap();
    }

    /// list_forwards is what the frontend store rehydrates from, so it must
    /// hand back exactly what the manager holds — not an empty stand-in.
    #[tokio::test(flavor = "multi_thread")]
    async fn list_forwards_returns_what_the_manager_holds() {
        let app = tauri::test::mock_app();
        app.manage(ForwardManager::new(ClientCache::new_many(vec![])));
        let manager: State<ForwardManager> = app.state();
        manager.insert_test_forward(7, 55555);

        let resp = list_forwards(app.state());
        assert_eq!(resp.forwards.len(), 1);
        assert_eq!(resp.forwards[0].id, 7);
        assert_eq!(resp.forwards[0].local_port, 55555);
    }
}
