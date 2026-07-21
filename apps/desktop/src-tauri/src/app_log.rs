//! Application-log file access for the Settings "Application logs" view.
//!
//! The `tauri-plugin-log` `LogDir` target (configured in `lib.rs`) writes a
//! rotating `srelens.log` in the OS log directory. These commands expose it to
//! the WebView: read a bounded tail, get the path (to copy), and reveal it in
//! the file manager.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};

/// Default amount of the log tail to return (bytes) — enough for triage without
/// loading an arbitrarily large file into the WebView.
const DEFAULT_TAIL_BYTES: u64 = 1_000_000;

/// The current log file: `<app log dir>/srelens.log` (matches the plugin's
/// `LogDir { file_name: Some("srelens") }` target).
fn log_file(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_log_dir().map_err(|e| e.to_string())?;
    Ok(dir.join("srelens.log"))
}

/// The absolute path of the application log file (for copy / display).
#[tauri::command]
pub async fn app_log_path(app: AppHandle) -> Result<String, String> {
    Ok(log_file(&app)?.to_string_lossy().into_owned())
}

/// Return the tail of the log file (last `max_bytes`, default 1 MB), dropping a
/// partial first line. An absent file (nothing logged yet) reads as empty.
#[tauri::command]
pub async fn read_app_log(app: AppHandle, max_bytes: Option<u64>) -> Result<String, String> {
    let path = log_file(&app)?;
    let cap = max_bytes.unwrap_or(DEFAULT_TAIL_BYTES);
    let mut file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
        Err(e) => return Err(e.to_string()),
    };
    let len = file.metadata().map_err(|e| e.to_string())?.len();
    let start = len.saturating_sub(cap);
    file.seek(SeekFrom::Start(start)).map_err(|e| e.to_string())?;
    // Read as bytes: a byte-offset seek can split a UTF-8 char, so decode lossy
    // rather than risk a hard error on a valid log.
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    // Started mid-file — drop the partial first line.
    if start > 0 {
        if let Some(newline) = text.find('\n') {
            text.drain(..=newline);
        }
    }
    Ok(text)
}

/// Reveal the log file in the OS file manager (or its directory when the file
/// doesn't exist yet, or on Linux where selecting a file isn't portable).
#[tauri::command]
pub async fn reveal_app_log(app: AppHandle) -> Result<(), String> {
    reveal_in_file_manager(&log_file(&app)?)
}

fn reveal_in_file_manager(path: &Path) -> Result<(), String> {
    let dir = path.parent().unwrap_or(path);
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    let exists = path.exists();

    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = std::process::Command::new("open");
        if exists {
            command.arg("-R").arg(path);
        } else {
            command.arg(dir);
        }
        command
    };
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = std::process::Command::new("explorer");
        if exists {
            command.arg(format!("/select,{}", path.display()));
        } else {
            command.arg(dir);
        }
        command
    };
    // No portable "select a file" on Linux — open the containing directory.
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = std::process::Command::new("xdg-open");
        command.arg(dir);
        command
    };

    command.spawn().map_err(|e| e.to_string())?;
    Ok(())
}
