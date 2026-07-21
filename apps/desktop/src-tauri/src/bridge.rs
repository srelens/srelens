//! Bridges the capability registry to the Tauri WebView as a command.
//!
//! This is the Tauri half of "one definition, two surfaces": the same
//! `Registry` that backs the MCP server is invoked here from the frontend.

use srelens_capability::Registry;
use serde_json::Value;
use tauri::State;

/// Tauri-managed state holding the capability registry.
pub struct AppRegistry(pub Registry);

/// Invoke a backend capability by id. The WebView calls this via
/// `invoke('invoke_capability', { id, input })`.
#[tauri::command]
pub async fn invoke_capability(
    id: String,
    input: Value,
    registry: State<'_, AppRegistry>,
) -> Result<Value, String> {
    registry.0.invoke(&id, input).await.map_err(|e| {
        // Surface capability failures (connection timeouts, denied RBAC, bad
        // input) to the application log for post-hoc diagnosis.
        let message = e.to_string();
        log::warn!("capability '{id}' failed: {message}");
        message
    })
}
