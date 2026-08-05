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
/// otherwise a value goes only if its key names a credential or holds a
/// caller-supplied payload. Recursively walks nested objects and arrays to find
/// and redact credentials at any depth.
pub fn redact(args: &Value, sensitive: bool) -> Value {
    /// Substring-matched: a key admitting it holds a credential, at any depth
    /// and in any casing (`apiToken`, `tls.key`, `rootPassword`).
    const NEEDLES: [&str; 4] = ["token", "secret", "password", "key"];
    /// Whole fields whose value is a caller-supplied payload that can carry
    /// secret material with no credential-shaped key inside it to catch:
    /// `data`/`stringData` on a Secret write (`k8s.updateConfigData` — a
    /// Secret's own keys are things like `username` and `ca.crt`), `yaml` on
    /// `k8s.applyManifest` (one opaque string holding a whole manifest), and
    /// `values` on the helm install/upgrade/template capabilities (user YAML
    /// that routinely holds registry credentials and database passwords).
    ///
    /// Matched EXACTLY, not as substrings, so `metadata` stays readable — the
    /// point is to keep the shape of a call auditable while dropping the part
    /// that carries secrets.
    const PAYLOAD_FIELDS: [&str; 4] = ["data", "stringdata", "yaml", "values"];

    match args {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                let lower = k.to_ascii_lowercase();
                let is_credential_key = NEEDLES.iter().any(|n| lower.contains(n))
                    || PAYLOAD_FIELDS.contains(&lower.as_str());

                if sensitive || is_credential_key {
                    // Redact this value entirely
                    out.insert(k.clone(), json!("<redacted>"));
                } else {
                    // Recurse into the value to find nested credentials
                    out.insert(k.clone(), redact(v, false));
                }
            }
            Value::Object(out)
        }
        Value::Array(arr) => {
            // Recurse into array elements with the same sensitivity
            Value::Array(arr.iter().map(|v| redact(v, sensitive)).collect())
        }
        // Scalars stay as-is (no redaction needed)
        other => other.clone(),
    }
}

/// The most recent `limit` entries, newest first.
///
/// Reads at most a bounded window from the END of the log rather than the whole
/// file: the log is capped at 5 MB and a caller only ever wants the last
/// handful, so parsing every line to discard nearly all of them is wasted work
/// on every Settings open. Unparseable lines are skipped — a torn final write,
/// or the fragment of a line the window's start lands inside, is not an error.
///
/// Only the live log is read; entries rotated into `.jsonl.1` are not included.
pub fn tail(path: &std::path::Path, limit: usize) -> Vec<Value> {
    /// Generous next to a realistic `limit` (tens of entries at a few hundred
    /// bytes each) while staying a small fraction of the 5 MB cap.
    const WINDOW: u64 = 512 * 1024;

    use std::io::{Read, Seek, SeekFrom};

    let Ok(mut f) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let start = len.saturating_sub(WINDOW);
    if start > 0 && f.seek(SeekFrom::Start(start)).is_err() {
        return Vec::new();
    }
    let mut buf = Vec::new();
    if f.read_to_end(&mut buf).is_err() {
        return Vec::new();
    }
    // Lossy: a window start can land inside a multi-byte character, and a
    // mangled leading fragment is discarded below regardless.
    let text = String::from_utf8_lossy(&buf);
    let mut lines: Vec<&str> = text.lines().collect();
    if start > 0 && !lines.is_empty() {
        // Seeking mid-file almost certainly lands inside a line; that fragment
        // is not a record.
        lines.remove(0);
    }
    lines
        .iter()
        .rev()
        .filter_map(|l| serde_json::from_str(l).ok())
        .take(limit)
        .collect()
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
        // 0600: the log holds every tool call's arguments, so it is at least
        // as sensitive as the token file beside it. `mode` applies only when
        // the file is created, so an existing log is tightened after the write
        // — a log an older build left 0644 must not stay readable forever.
        let mut opts = std::fs::OpenOptions::new();
        opts.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let opened = opts.open(&self.path);
        if let Ok(mut f) = opened {
            let _ = writeln!(f, "{line}");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ =
                    std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600));
            }
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

    fn write_entries(path: &std::path::Path, count: usize, pad: usize) {
        let log = JsonlAuditLog::new(path.to_path_buf(), u64::MAX); // never rotate
        for i in 0..count {
            log.record(AuditRecord {
                transport: Transport::Stdio,
                tool: format!("tool{i}"),
                args: json!({ "pad": "x".repeat(pad) }),
                decision: "auto",
                outcome: "ok",
                error: None,
            });
        }
    }

    #[test]
    fn tail_returns_the_newest_entries_first() {
        let dir = std::env::temp_dir().join(format!("srelens-tail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.jsonl");
        let _ = std::fs::remove_file(&path);
        write_entries(&path, 10, 0);

        let out = tail(&path, 3);

        assert_eq!(out.len(), 3);
        assert_eq!(out[0]["tool"], json!("tool9"), "newest first");
        assert_eq!(out[1]["tool"], json!("tool8"));
        assert_eq!(out[2]["tool"], json!("tool7"));
    }

    #[test]
    fn tail_of_a_missing_log_is_empty() {
        let out = tail(std::path::Path::new("/nonexistent/srelens/audit.jsonl"), 50);
        assert!(out.is_empty(), "a log that was never written is not an error");
    }

    /// The log is capped at 5 MB, so reading and parsing all of it to show 50
    /// rows is wasted work on every Settings open. Reading a bounded window from
    /// the end means the oldest entries are never touched — which is what this
    /// asserts, by demanding an entry from the far past be absent even when the
    /// caller asks for far more rows than exist.
    #[test]
    fn tail_reads_only_a_bounded_window_from_the_end() {
        let dir = std::env::temp_dir().join(format!("srelens-tailwin-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("big.jsonl");
        let _ = std::fs::remove_file(&path);
        // ~1 KB per entry x 2000 = ~2 MB, comfortably past any sane window.
        write_entries(&path, 2000, 1000);
        assert!(std::fs::metadata(&path).unwrap().len() > 1024 * 1024);

        let out = tail(&path, 100_000);

        assert!(!out.is_empty());
        assert_eq!(out[0]["tool"], json!("tool1999"), "newest entry must be present");
        assert!(
            out.iter().all(|e| e["tool"] != json!("tool0")),
            "the oldest entry must not be read at all"
        );
    }

    /// Seeking to a byte offset lands mid-line. That fragment is not valid JSON
    /// and must be dropped rather than surfacing as a missing row or an error.
    #[test]
    fn tail_discards_the_partial_line_at_the_window_boundary() {
        let dir = std::env::temp_dir().join(format!("srelens-tailfrag-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("frag.jsonl");
        let _ = std::fs::remove_file(&path);
        write_entries(&path, 3000, 1000);

        let out = tail(&path, 100_000);

        assert!(
            out.iter().all(|e| e.get("tool").is_some()),
            "every returned row must be a fully parsed entry, not a fragment"
        );
    }

    fn a_record() -> AuditRecord {
        AuditRecord {
            transport: Transport::Http,
            tool: "k8s.updateConfigData".into(),
            args: json!({ "name": "db-creds" }),
            decision: "approved",
            outcome: "ok",
            error: None,
        }
    }

    /// The log records every tool call's arguments, so it is at least as
    /// sensitive as the token file sitting beside it — which is explicitly
    /// 0600 (see `auth::FileTokenStore::save`). `create(true).append(true)`
    /// with no mode yields 0644 under a standard umask, i.e. readable by
    /// every other account on the machine.
    #[cfg(unix)]
    #[test]
    fn audit_log_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("srelens-audit-perm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("perm.jsonl");
        let _ = std::fs::remove_file(&path);

        JsonlAuditLog::new(path.clone(), 1024 * 1024).record(a_record());

        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "the audit log must not be group/world readable");
    }

    /// An upgrade case: a log already on disk from a build that created it
    /// 0644 must be tightened, not left permanently readable because the file
    /// happened to exist before.
    #[cfg(unix)]
    #[test]
    fn audit_log_tightens_loose_permissions_on_an_existing_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("srelens-audit-perm2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("loose.jsonl");
        std::fs::write(&path, "{}\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        JsonlAuditLog::new(path.clone(), 1024 * 1024).record(a_record());

        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "an existing loose log must be tightened");
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

    /// `k8s.updateConfigData` takes `kind: "Secret"` and a `data` map of
    /// *plaintext* values. None of a Secret's own key names need look like a
    /// credential (`username`, `host`, `ca.crt`), so key-name matching alone
    /// writes them verbatim. The field that holds a payload is the thing to
    /// redact, not just the keys that admit to being secrets.
    #[test]
    fn redacts_the_data_payload_of_a_secret_write() {
        let args = json!({
            "kind": "Secret",
            "namespace": "prod",
            "name": "db-creds",
            "data": { "username": "admin" }
        });
        let out = redact(&args, false);
        // The shape an operator needs to audit the call must survive.
        assert_eq!(out["kind"], json!("Secret"));
        assert_eq!(out["namespace"], json!("prod"));
        assert_eq!(out["name"], json!("db-creds"));
        // The payload must not.
        assert_eq!(out["data"], json!("<redacted>"));
    }

    /// `k8s.applyManifest`'s `yaml` is the entire manifest, so applying a
    /// Secret puts its whole body in the log. `yaml` is one opaque string —
    /// there are no nested keys for the needle list to catch.
    #[test]
    fn redacts_the_yaml_payload_of_an_apply() {
        let args = json!({
            "context": "prod",
            "yaml": "apiVersion: v1\nkind: Secret\nstringData:\n  password: hunter2\n"
        });
        let out = redact(&args, false);
        assert_eq!(out["context"], json!("prod"), "which cluster must stay visible");
        assert_eq!(out["yaml"], json!("<redacted>"));
    }

    /// Helm values are user-supplied YAML and routinely carry registry
    /// credentials and database passwords.
    #[test]
    fn redacts_helm_values() {
        let args = json!({ "release": "web", "chart": "bitnami/nginx", "values": "auth:\n  rootPassword: hunter2\n" });
        let out = redact(&args, false);
        assert_eq!(out["release"], json!("web"));
        assert_eq!(out["chart"], json!("bitnami/nginx"));
        assert_eq!(out["values"], json!("<redacted>"));
    }

    /// Deliberately nests under `spec`/`template` rather than `data`: those are
    /// ordinary structural keys, so this keeps testing what it's named for —
    /// that a credential key is found at *depth* — instead of being short-
    /// circuited by `PAYLOAD_FIELDS` redacting the wrapper wholesale.
    #[test]
    fn redaction_recurses_into_nested_objects() {
        let args = json!({
            "spec": {
                "template": {
                    "password": "hunter2"
                }
            }
        });
        let out = redact(&args, false);
        // Keys must survive at all levels
        assert!(out["spec"].is_object());
        assert!(out["spec"]["template"].is_object());
        // But the credential value must be redacted
        assert_eq!(out["spec"]["template"]["password"], json!("<redacted>"));
    }

    #[test]
    fn redaction_recurses_into_array_elements() {
        let args = json!({
            "items": [
                { "apiKey": "secret123" },
                { "name": "safe" }
            ]
        });
        let out = redact(&args, false);
        // Array structure is preserved
        assert!(out["items"].is_array());
        // Credential in first element is redacted
        assert_eq!(out["items"][0]["apiKey"], json!("<redacted>"));
        // Non-credential in second element is preserved
        assert_eq!(out["items"][1]["name"], json!("safe"));
    }

    #[test]
    fn redaction_preserves_deep_non_credential_values() {
        let args = json!({
            "spec": {
                "replicas": 3,
                "image": "nginx:1.14"
            }
        });
        let out = redact(&args, false);
        // Non-credential scalar values must survive redaction
        assert_eq!(out["spec"]["replicas"], json!(3));
        assert_eq!(out["spec"]["image"], json!("nginx:1.14"));
    }
}
