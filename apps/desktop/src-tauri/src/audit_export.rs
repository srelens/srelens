//! User-requested, local audit export. No web handler or MCP tool is exposed.

use std::io::Write;
use std::path::Path;
use tauri::{AppHandle, Runtime, State};
use tauri_plugin_dialog::DialogExt;
use srelens_capability::audit::RetainedAudit;
use crate::bridge::AppAudit;

#[tauri::command]
pub fn mcp_audit_retained(audit: State<'_, AppAudit>) -> Result<RetainedAudit, String> {
    audit.0.retained().map_err(|e| format!("The retained audit trail could not be read: {e}"))
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

    #[test]
    fn command_reads_the_managed_sink_and_propagates_refusals() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        std::fs::write(&path, "{\"ts\":1,\"tool\":\"read\"}\n").unwrap();
        let app = tauri::test::mock_app();
        app.manage(AppAudit(Arc::new(JsonlAuditLog::new(path, 1024))));
        assert_eq!(mcp_audit_retained(app.state()).unwrap().entries.len(), 1);
        let app = tauri::test::mock_app();
        app.manage(AppAudit(Arc::new(NoopAudit)));
        assert!(mcp_audit_retained(app.state()).is_err());
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
