//! Append-only record of what agents did. Values from sensitive capabilities
//! are never written: names and shapes only.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::{json, Value};

use crate::Transport;

#[derive(Debug, Clone)]
pub struct AuditRecord {
    pub transport: Transport,
    pub tool: String,
    pub args: Value,
    /// "approved" | "denied" | "auto" (no consent needed) — never free text.
    pub decision: &'static str,
    pub outcome: &'static str,
    pub error: Option<String>,
}

pub trait AuditSink: Send + Sync {
    fn record(&self, rec: AuditRecord);
}

/// Default sink: records nothing. Used by tests and by hosts that opt out.
pub struct NoopAudit;

impl AuditSink for NoopAudit {
    fn record(&self, _rec: AuditRecord) {}
}

/// Redact argument VALUES while keeping keys, so an operator can see the shape
/// of a call without its secrets. Sensitive-annotated tools redact everything;
/// otherwise only keys that name a credential.
pub fn redact(args: &Value, sensitive: bool) -> Value {
    const NEEDLES: [&str; 4] = ["token", "secret", "password", "key"];
    match args {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                let lower = k.to_ascii_lowercase();
                let hit = sensitive || NEEDLES.iter().any(|n| lower.contains(n));
                out.insert(
                    k.clone(),
                    if hit { json!("<redacted>") } else { v.clone() },
                );
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

pub struct JsonlAuditLog {
    path: PathBuf,
    cap_bytes: u64,
    lock: Mutex<()>,
}

impl JsonlAuditLog {
    pub fn new(path: PathBuf, cap_bytes: u64) -> Self {
        Self { path, cap_bytes, lock: Mutex::new(()) }
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl AuditSink for JsonlAuditLog {
    fn record(&self, rec: AuditRecord) {
        // Bookkeeping fails open: a lost log line must never break a working
        // cluster operation, so every error here is swallowed after logging.
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        if let Ok(meta) = std::fs::metadata(&self.path) {
            if meta.len() >= self.cap_bytes {
                let _ = std::fs::rename(&self.path, self.path.with_extension("jsonl.1"));
            }
        }
        let line = json!({
            "ts": unix_now(),
            "transport": rec.transport.as_str(),
            "tool": rec.tool,
            "args": rec.args,
            "decision": rec.decision,
            "outcome": rec.outcome,
            "err": rec.error,
        });
        let opened = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path);
        if let Ok(mut f) = opened {
            let _ = writeln!(f, "{line}");
        } else {
            eprintln!("srelens: could not write MCP audit log at {}", self.path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_only_credential_keys_by_default() {
        let args = json!({ "namespace": "prod", "apiToken": "abc", "name": "web" });
        let out = redact(&args, false);
        assert_eq!(out["namespace"], json!("prod"));
        assert_eq!(out["name"], json!("web"));
        assert_eq!(out["apiToken"], json!("<redacted>"));
    }

    #[test]
    fn sensitive_capability_redacts_every_value_but_keeps_keys() {
        let args = json!({ "namespace": "prod", "name": "db-creds" });
        let out = redact(&args, true);
        assert_eq!(out["namespace"], json!("<redacted>"));
        assert_eq!(out["name"], json!("<redacted>"));
        assert!(out.get("namespace").is_some(), "keys must survive redaction");
    }

    #[test]
    fn writes_one_json_line_per_record() {
        let dir = std::env::temp_dir().join(format!("srelens-audit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.jsonl");
        let _ = std::fs::remove_file(&path);
        let log = JsonlAuditLog::new(path.clone(), 1024 * 1024);
        log.record(AuditRecord {
            transport: Transport::Http,
            tool: "k8s_deletePod".into(),
            args: json!({ "name": "web" }),
            decision: "approved",
            outcome: "ok",
            error: None,
        });
        let body = std::fs::read_to_string(&path).unwrap();
        assert_eq!(body.lines().count(), 1);
        let parsed: Value = serde_json::from_str(body.lines().next().unwrap()).unwrap();
        assert_eq!(parsed["tool"], json!("k8s_deletePod"));
        assert_eq!(parsed["transport"], json!("http"));
        assert_eq!(parsed["decision"], json!("approved"));
        assert!(parsed["ts"].as_u64().unwrap() > 0);
    }

    #[test]
    fn rotates_once_past_the_cap() {
        let dir = std::env::temp_dir().join(format!("srelens-rot-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("b.jsonl");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(dir.join("b.jsonl.1"));
        let log = JsonlAuditLog::new(path.clone(), 200); // tiny cap
        for i in 0..40 {
            log.record(AuditRecord {
                transport: Transport::Stdio,
                tool: format!("tool{i}"),
                args: json!({}),
                decision: "auto",
                outcome: "ok",
                error: None,
            });
        }
        assert!(dir.join("b.jsonl.1").exists(), "expected a rotated file");
        let live = std::fs::metadata(&path).unwrap().len();
        assert!(live <= 200 * 2, "live file should stay near the cap, was {live}");
    }
}
