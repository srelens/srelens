//! Streaming Toolbox installs.
//!
//! The `toolbox.install*` capabilities install synchronously (one await, for
//! MCP). The GUI wants a progress bar, so `start_tool_install` runs the *same*
//! install core (`srelens_kube::toolbox::run_*_install`) but with a fetch that
//! streams the download and emits `toolbox://progress` as bytes arrive. One
//! implementation, two surfaces (spec §4).

use std::io::Read;

use serde::Serialize;
use srelens_kube::toolbox::{run_helm_install, run_krew_install, run_kubectl_install, srelens_bin_dir};
use srelens_kube::toolbox::InstallToolOut;
use srelens_kube::toolbox_install::InstallError;
use tauri::{AppHandle, Emitter};

/// Only report progress for real payload downloads, not the tiny checksum /
/// version / GitHub-API requests that also flow through the same fetch.
const PROGRESS_MIN_BYTES: u64 = 500_000;

#[derive(Serialize, Clone)]
struct ToolInstallProgress {
    tool: String,
    received: u64,
    total: Option<u64>,
}

/// Blocking GET that streams the body, emitting `toolbox://progress` for large
/// downloads. Returns the full bytes so the install core is unchanged.
fn fetch_with_progress(url: &str, app: &AppHandle, tool: &str) -> Result<Vec<u8>, InstallError> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("srelens/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| InstallError::Download(e.to_string()))?;
    let mut resp = client
        .get(url)
        .send()
        .map_err(|e| InstallError::Download(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(InstallError::Download(format!("{} for {url}", resp.status())));
    }

    let total = resp.content_length();
    let report = total.is_some_and(|t| t >= PROGRESS_MIN_BYTES);
    let mut buf = total.map_or_else(Vec::new, |t| Vec::with_capacity(t as usize));
    let mut chunk = vec![0u8; 64 * 1024];
    let mut received: u64 = 0;
    loop {
        let n = resp
            .read(&mut chunk)
            .map_err(|e| InstallError::Download(e.to_string()))?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        received += n as u64;
        if report {
            let _ = app.emit(
                "toolbox://progress",
                ToolInstallProgress { tool: tool.to_string(), received, total },
            );
        }
    }
    Ok(buf)
}

/// Install a managed tool (`kubectl` / `helm` / `krew`) with streaming download
/// progress on `toolbox://progress`. Same verified install as the capability.
#[tauri::command]
pub async fn start_tool_install(app: AppHandle, tool: String) -> Result<InstallToolOut, String> {
    tokio::task::spawn_blocking(move || {
        let fetch = |url: &str| fetch_with_progress(url, &app, &tool);
        let result = match tool.as_str() {
            "kubectl" => run_kubectl_install(&srelens_bin_dir(), &fetch),
            "helm" => run_helm_install(&srelens_bin_dir(), &fetch),
            "krew" => run_krew_install(&std::env::temp_dir(), &fetch, &crate::capabilities::run_tool),
            other => Err(InstallError::Download(format!("unknown tool: {other}"))),
        };
        result.map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
