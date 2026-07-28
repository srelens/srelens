//! Tauri adapter for helm write operations: the streaming core lives in
//! srelens_streams::helm; this module only maps the Tauri command surface
//! (and desktop kubeconfig discovery) onto it.

use std::sync::Arc;

use srelens_streams::helm::HelmManager;
use tauri::{AppHandle, State};

use crate::sink::TauriSink;

/// Run `helm <args>` scoped to `context`, streaming stdout+stderr on
/// `helm:out:<channel>`; `helm:exit:<channel>` fires with None on success or an
/// error string on failure. Returns the session id.
#[tauri::command]
pub async fn start_helm_op(
    context: String,
    extra_kubeconfigs: Vec<String>,
    args: Vec<String>,
    values: String,
    channel: String,
    app: AppHandle,
    manager: State<'_, HelmManager>,
) -> Result<u64, String> {
    let mut paths = crate::capabilities::default_kubeconfig_paths();
    paths.extend(extra_kubeconfigs.iter().map(std::path::PathBuf::from));
    manager
        .start(
            Arc::new(TauriSink(app)),
            context,
            paths,
            args,
            values,
            channel,
            // Desktop is single-user: keep helm's default home unchanged.
            None,
        )
        .await
}

/// Abort a running helm operation (best-effort) and drop its session.
#[tauri::command]
pub async fn helm_op_close(session: u64, manager: State<'_, HelmManager>) -> Result<(), String> {
    manager.close(session);
    Ok(())
}
