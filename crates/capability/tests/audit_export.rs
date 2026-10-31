use serde_json::json;
use srelens_capability::audit::{AuditSink, JsonlAuditLog, NoopAudit};

fn scratch(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("audit-export-{label}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn retained_reads_beyond_the_tail_window_and_includes_rotation() {
    let dir = scratch("rotation");
    let path = dir.join("audit.jsonl");
    let old = json!({"ts": 1, "tool": "old", "decision": "approved", "outcome": "error"});
    std::fs::write(path.with_extension("jsonl.1"), format!("{old}\n")).unwrap();
    let mut body = format!("{}\n", json!({"ts": 2, "tool": "first"}));
    for _ in 0..2000 {
        body.push_str(&format!("{}\n", json!({"ts": 3, "tool": "read", "args": {"padding": "x".repeat(400)}})));
    }
    body.push_str("{torn\n");
    body.push_str(&format!("{}\n", json!({"ts": 4, "tool": "last", "args": {"token": "<redacted>"}})));
    std::fs::write(&path, body).unwrap();
    let records = JsonlAuditLog::new(path, 5 * 1024 * 1024).retained().unwrap();
    assert_eq!(records.entries.len(), 2003);
    assert_eq!(records.skipped_lines, 1);
    assert_eq!(records.entries[0]["tool"], "old");
    assert_eq!(records.entries[0]["outcome"], "failed");
    assert_eq!(records.entries[1]["tool"], "first");
    assert_eq!(records.entries.last().unwrap()["args"]["token"], "<redacted>");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn retained_distinguishes_missing_logs_from_unreadable_logs() {
    let dir = scratch("errors");
    let path = dir.join("audit.jsonl");
    let log = JsonlAuditLog::new(path.clone(), 1024);
    assert!(log.retained().unwrap().entries.is_empty());
    std::fs::create_dir(&path).unwrap();
    assert!(log.retained().is_err());
    std::fs::remove_dir(&path).unwrap();
    std::fs::create_dir(path.with_extension("jsonl.1")).unwrap();
    assert!(log.retained().is_err());
    assert!(NoopAudit.retained().is_err());
    std::fs::remove_dir_all(dir).unwrap();
}
