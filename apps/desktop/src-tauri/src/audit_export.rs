//! User-requested, local audit export. No web handler or MCP tool is exposed.

use std::io::Write;
use std::path::Path;
use tauri::{AppHandle, Runtime, State};
use tauri_plugin_dialog::DialogExt;
use srelens_capability::audit::RetainedAudit;
use crate::bridge::AppAudit;

#[tauri::command]
pub async fn mcp_audit_retained(audit: State<'_, AppAudit>) -> Result<RetainedAudit, String> {
    // Own the sink before crossing the await. Reading and parsing both logs
    // can be slow, and must occupy neither the UI thread nor an async worker.
    let audit = audit.0.clone();
    tauri::async_runtime::spawn_blocking(move || audit.retained())
        .await
        .map_err(|e| format!("The retained audit worker could not finish: {e}"))?
        .map_err(|e| format!("The retained audit trail could not be read: {e}"))
}

#[derive(Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat { Jsonl, Csv }

impl ExportFormat {
    fn extension(self) -> &'static str {
        match self { Self::Jsonl => "jsonl", Self::Csv => "csv" }
    }
}

/// The frontend supplies only content and format, never a destination path.
/// The native dialog is the sole authority for where the export is written.
#[tauri::command]
pub async fn save_audit_export<R: Runtime>(
    app: AppHandle<R>, content: String, format: ExportFormat,
) -> Result<Option<String>, String> {
    let extension = format.extension();
    let file = app.dialog().file()
        .set_file_name(format!("srelens-audit.{extension}"))
        .add_filter("Audit export", &[extension])
        .blocking_save_file();
    let Some(file) = file else { return Ok(None); };
    let path = file.as_path().ok_or("invalid save path")?;
    write_export(path, content.as_bytes()).map_err(|e| format!("The audit export could not be saved: {e}"))?;
    Ok(Some(path.to_string_lossy().into_owned()))
}

/// An owner-only temporary file replaces the chosen file only once complete.
/// A failed export leaves the destination intact and no partial trail behind.
fn write_export(path: &Path, content: &[u8]) -> std::io::Result<()> {
    let mut file = tempfile::NamedTempFile::new_in(path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid save path")
    })?)?;
    file.write_all(content)?;
    file.as_file().sync_all()?;
    file.persist(path).map(|_| ()).map_err(|e| e.error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tauri::Manager;
    use srelens_capability::audit::{JsonlAuditLog, NoopAudit};

    #[tokio::test]
    async fn command_reads_the_managed_sink_and_propagates_refusals() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        std::fs::write(&path, "{\"ts\":1,\"tool\":\"read\"}\n").unwrap();
        let app = tauri::test::mock_app();
        app.manage(AppAudit(Arc::new(JsonlAuditLog::new(path, 1024))));
        assert_eq!(mcp_audit_retained(app.state()).await.unwrap().entries.len(), 1);
        let app = tauri::test::mock_app();
        app.manage(AppAudit(Arc::new(NoopAudit)));
        assert!(mcp_audit_retained(app.state()).await.is_err());
    }

    /// Even an async command can freeze its caller if it performs the read
    /// directly. Observe where the sink runs, without sleeps or disk timing.
    #[tokio::test]
    async fn retained_read_runs_on_a_worker_not_the_callers_thread() {
        struct ThreadAudit(Arc<std::sync::Mutex<Option<std::thread::ThreadId>>>);
        impl srelens_capability::audit::AuditSink for ThreadAudit {
            fn record(&self, _: srelens_capability::audit::AuditRecord) {}
            fn retained(&self) -> std::io::Result<RetainedAudit> {
                *self.0.lock().unwrap() = Some(std::thread::current().id());
                Ok(RetainedAudit::default())
            }
        }
        let caller = std::thread::current().id();
        let seen = Arc::new(std::sync::Mutex::new(None));
        let app = tauri::test::mock_app();
        app.manage(AppAudit(Arc::new(ThreadAudit(seen.clone()))));
        mcp_audit_retained(app.state()).await.unwrap();
        let worker = seen.lock().unwrap().expect("the sink must have been read");
        assert_ne!(worker, caller, "file reads and parsing must not run on the command caller");
    }

    #[test]
    fn saves_atomically_and_does_not_replace_a_destination_on_failure() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        std::fs::write(&path, "previous").unwrap();
        write_export(&path, b"complete\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "complete\n");
        let directory = dir.path().join("directory");
        std::fs::create_dir(&directory).unwrap();
        assert!(write_export(&directory, b"refused").is_err());
        assert!(directory.is_dir());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        assert_eq!(ExportFormat::Jsonl.extension(), "jsonl");
        assert_eq!(ExportFormat::Csv.extension(), "csv");
    }
}
