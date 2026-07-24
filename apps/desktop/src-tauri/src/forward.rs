//! Tauri adapter for port-forwards: the core lives in
//! srelens_streams::forward; this module only maps the Tauri command surface.

use std::sync::Arc;

use srelens_streams::forward::{ForwardInfo, ForwardManager};
use tauri::{AppHandle, State};

use crate::sink::TauriSink;

/// Start forwarding a local port to a Pod or Service. `kind` is "Pod" or
/// "Service"; a Service is resolved to a backing pod and target port first.
/// Returns the id + bound local port; a `forward:closed:<id>` event fires
/// (with an optional error string) if the forward loop ends on its own.
#[tauri::command]
pub async fn start_port_forward(
    context: String,
    namespace: String,
    kind: String,
    name: String,
    remote_port: u16,
    local_port: Option<u16>,
    app: AppHandle,
    manager: State<'_, ForwardManager>,
) -> Result<ForwardInfo, String> {
    manager
        .start(
            Arc::new(TauriSink(app)),
            context,
            namespace,
            kind,
            name,
            remote_port,
            local_port,
        )
        .await
}

/// Stop a port-forward and abort its task.
#[tauri::command]
pub async fn stop_port_forward(id: u64, manager: State<'_, ForwardManager>) -> Result<(), String> {
    manager.stop(id);
    Ok(())
}
