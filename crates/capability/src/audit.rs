//! Append-only record of what was done through the capability registry, by an
//! agent over MCP **or** by a person clicking in the app. Values from sensitive
//! capabilities are never written: names and shapes only.
//!
//! **This lives beside the registry, not beside a transport.** It used to sit
//! in `srelens-mcp`, which made the MCP server the only thing that could write
//! a record — so an Argo CD sync clicked in the desktop UI went straight from
//! `invoke_capability` into [`crate::Registry::invoke`] and left no trace
//! (#555). The registry is the one place both surfaces meet, so the sink, the
//! redaction and the record's shape live here and
//! [`crate::Registry::invoke_audited`] is the single writer for an invocation.
//!
//! **The trail never leaves the machine.** `JsonlAuditLog` appends to one
//! `0600` file under the app's config directory and nothing reads it but the
//! Settings pane on the same host. There is no upload, no telemetry and no
//! second copy.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::{json, Value};

use crate::Annotations;

/// Which surface a capability call arrived on.
///
/// Two questions, one value: **who** called (`ui` or `mcp` — the `source`
/// field) and **how** they reached the registry (`ui`, `stdio` or `http` — the
/// `transport` field). An operator reading the trail after an incident asks
/// the first; an operator wondering which client to go turn off asks the
/// second.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Clicked in the desktop app, through the Tauri `invoke_capability`
    /// bridge.
    Ui,
    /// An MCP client over stdio (a CLI agent the user spawned).
    McpStdio,
    /// An MCP client over the loopback HTTP transport, including the
    /// in-process native agent, which calls `handle_request` directly.
    McpHttp,
}

impl Source {
    /// `ui` or `mcp`: who made the call.
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Ui => "ui",
            Source::McpStdio | Source::McpHttp => "mcp",
        }
    }

    /// `ui`, `stdio` or `http`: how the call reached the registry.
    pub fn transport(self) -> &'static str {
        match self {
            Source::Ui => "ui",
            Source::McpStdio => "stdio",
            Source::McpHttp => "http",
        }
    }
}

/// The app a call was made through, when it was made through one.
///
/// Both halves or neither: an app ID without the revision cannot answer "which
/// manifest and which grants was this", because an update rolls the revision
/// and leaves the ID alone (`crates/registry/src/extensions.rs`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppRef {
    pub id: String,
    pub revision: u64,
}

#[derive(Debug, Clone)]
pub struct AuditRecord {
    pub source: Source,
    pub tool: String,
    /// Already redacted — see [`redact`]. Never the caller's raw arguments.
    pub args: Value,
    /// The app this call was made through, when it was made through one.
    pub app: Option<AppRef>,
    /// The cluster context the call named, when it named one.
    pub cluster: Option<String>,
    /// The object the call named, as `namespace/name` or `name`.
    pub resource: Option<String>,
    /// "approved" | "denied" | "auto" (no consent needed) — never free text.
    pub decision: &'static str,
    /// [`OUTCOME_OK`], [`OUTCOME_REJECTED`] or [`OUTCOME_FAILED`] — never free
    /// text.
    pub outcome: &'static str,
    pub error: Option<String>,
}

/// The call ran and the capability answered.
pub const OUTCOME_OK: &str = "ok";
/// The call never ran: consent was refused, the capability is not registered,
/// or its arguments were refused before anything was touched.
pub const OUTCOME_REJECTED: &str = "rejected";
/// The call ran and the handler failed — a timeout, an RBAC refusal from the
/// cluster, an API error.
pub const OUTCOME_FAILED: &str = "failed";

pub trait AuditSink: Send + Sync {
    fn record(&self, rec: AuditRecord);
}

/// Default sink: records nothing. Used by tests and by hosts that opt out.
pub struct NoopAudit;

impl AuditSink for NoopAudit {
    fn record(&self, _rec: AuditRecord) {}
}

/// Whether a call from the **UI** belongs in the trail.
///
/// MCP records every call it handles, allowed or not: an agent is a third
/// party and the question the pane answers is "what did it do", including what
/// it tried and was refused. The UI is the user acting directly, and recording
/// every read there would bury the writes — a single resource screen fires
/// dozens of list and get calls a minute, and the trail is capped at 5 MB.
///
/// So from the UI the line is the capability's own safety class, the same one
/// the confirm gate reads: anything that is not a plain read, plus reads that
/// return secret material. That is exactly the set #555 asks for ("every
/// mutating or sensitive capability invocation") and it is read off the
/// annotations rather than a second list that would drift from them.
pub fn is_audited_from_ui(annotations: &Annotations) -> bool {
    !annotations.read_only
        || annotations.destructive
        || annotations.requires_confirm
        || annotations.sensitive
}

/// What a call names: the app it went through, the cluster, and the object.
///
/// Read off the arguments, because that is where a capability's target lives
/// and there is no second place to put it that every capability would fill in.
/// Two shapes are understood, and they are the two the registry actually has:
///
/// - flat — `context`, `namespace`, `name`/`node`, and `id` + `revision` for
///   the `extensions.*` capabilities;
/// - nested under `resource` — `extensions.action`, whose whole selection
///   (app ID, revision, context, namespace, name) is one object
///   (`crates/registry/src/extensions/resource.rs`).
///
/// Every field is optional and a missing one stays `None` rather than becoming
/// an empty string: "this call named no namespace" and "this call named the
/// empty namespace" are different facts, and the pane renders them apart.
pub fn describe_target(args: &Value) -> (Option<AppRef>, Option<String>, Option<String>) {
    fn text(v: &Value, key: &str) -> Option<String> {
        v.get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    }
    // The nested selection wins for the fields it carries: on
    // `extensions.action` the top level holds only `action`, `uid` and
    // `resourceVersion`, and everything that identifies the target is inside.
    let scopes: Vec<&Value> = args
        .get("resource")
        .filter(|v| v.is_object())
        .into_iter()
        .chain(std::iter::once(args))
        .collect();
    let first = |key: &str| scopes.iter().find_map(|v| text(v, key));

    let app = scopes.iter().find_map(|v| {
        let id = text(v, "id")?;
        let revision = v.get("revision").and_then(Value::as_u64)?;
        Some(AppRef { id, revision })
    });
    let cluster = first("context").or_else(|| first("cluster"));
    let name = first("name").or_else(|| first("node"));
    let resource = match (first("namespace"), name) {
        (Some(ns), Some(name)) => Some(format!("{ns}/{name}")),
        (None, Some(name)) => Some(name),
        // A namespace with no object named is still the target of a call:
        // `k8s.listPods` in `prod` is a fact worth keeping.
        (Some(ns), None) => Some(ns),
        (None, None) => None,
    };
    (app, cluster, resource)
}

/// Redact argument VALUES while keeping keys, so an operator can see the shape
/// of a call without its secrets. Sensitive-annotated tools redact everything;
/// otherwise a value goes only if its key names a credential, holds a
/// caller-supplied payload, or is a map of settings whose names are the shape
/// and whose values are the secrets. Recursively walks nested objects and
/// arrays to find and redact credentials at any depth.
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
    /// Fields holding a map of caller-chosen names to caller-chosen values,
    /// where the NAMES are the auditable shape and every VALUE is treated as a
    /// secret: `settings` on `extensions.configure` (#605). An app's settings
    /// are free-form JSON and nothing marks one as sensitive, so a value under
    /// `credential` or `certificate` — which no needle matches — would
    /// otherwise be written verbatim, even for a denied call, and persist
    /// through rotation. The map is redacted as a sensitive capability's
    /// arguments are: keys kept, values blanked. Anything but a map is blanked
    /// whole, since a denied call is audited before its arguments are checked
    /// against the schema. Matched exactly, like `PAYLOAD_FIELDS`, and no other
    /// capability takes a `settings` argument (`settings.set` takes `values`).
    const KEYED_PAYLOAD_FIELDS: [&str; 1] = ["settings"];
    /// Fields that promise a URL, so a value that is not one is a value the
    /// parser cannot pick the password out of: blanked whole rather than
    /// guessed at. Matched exactly, like the two sets above.
    const URL_FIELDS: [&str; 5] = ["url", "uri", "endpoint", "repourl", "webhook"];

    match args {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                let lower = k.to_ascii_lowercase();
                let is_credential_key = NEEDLES.iter().any(|n| lower.contains(n))
                    || PAYLOAD_FIELDS.contains(&lower.as_str());

                if sensitive || is_credential_key {
                    // Redact this value entirely
                    out.insert(k.clone(), json!(REDACTED));
                } else if URL_FIELDS.contains(&lower.as_str()) {
                    let cleaned = match v {
                        Value::String(s) => json!(redact_url(s).unwrap_or(REDACTED.into())),
                        other => redact(other, false),
                    };
                    out.insert(k.clone(), cleaned);
                } else if KEYED_PAYLOAD_FIELDS.contains(&lower.as_str()) {
                    // Keep the names, drop every value.
                    let redacted = match v {
                        Value::Object(_) => redact(v, true),
                        _ => json!(REDACTED),
                    };
                    out.insert(k.clone(), redacted);
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
        // A string that IS a URL carries its credentials whatever key it sits
        // under — `chart` on `k8s.helmInstall` takes `oci://…` — so the scrub
        // follows the value, not the name. Everything else is a scalar and
        // stays as it was.
        Value::String(s) if looks_like_a_url(s) => {
            json!(redact_url(s).unwrap_or(REDACTED.into()))
        }
        other => other.clone(),
    }
}

/// Whether a string announces itself as a URL, i.e. is worth taking apart.
///
/// The test is the scheme separator and nothing cleverer: a value with no
/// `://` in it is a chart name, a release, a namespace — things a reader of
/// the trail needs to see, and things with nowhere for a credential to hide
/// in the shape this function is looking for. `k8s.helmRepoAdd`'s own `url`
/// argument does not go through here; a field that promises a URL is held to
/// one (see `URL_FIELDS`).
fn looks_like_a_url(s: &str) -> bool {
    s.contains("://")
}

/// One URL with its credentials taken out, or `None` when it will not parse.
///
/// **What stays is what makes the record worth keeping**: the scheme, the
/// host, the port and the path say which repository was added, which registry
/// was pulled from. What goes is the two places a URL carries a secret:
///
/// - the **userinfo** — `https://deploy:s3cr3t@charts.example.com/stable` is
///   the form `helm repo add` documents for a private repository, and #555
///   made `k8s.helmRepoAdd` audited, so before this the token went to disk
///   verbatim and stayed there until rotation;
/// - **credential-bearing query parameters** — a pre-signed URL puts the
///   whole credential in the query (`sig`, `X-Amz-Signature`, `access_key`).
///   The parameter's NAME is kept and only its value is blanked, so the
///   record still says a signed URL was used. The name is classified
///   decoded, because `?to%6ben=` is a `token` parameter to every server
///   that reads it;
/// - the **fragment**, dropped whole — see below.
///
/// `None` (the caller writes [`REDACTED`]) when the string does not parse as
/// a URL **with a host**, which is the fail-closed direction: a string that
/// cannot be taken apart is one whose password cannot be located, and the
/// values this function is given are exactly the ones an audit record must
/// not leak. The host is part of the test because `Url::parse` happily reads
/// `deploy:s3cr3t@charts.example.com` as the scheme `deploy` over an opaque
/// path — a credential that would otherwise have passed straight through as
/// a URL the parser "understood".
fn redact_url(raw: &str) -> Option<String> {
    let mut url = url::Url::parse(raw).ok()?;
    if !url.has_host() {
        return None;
    }
    // Both return Err for a scheme that cannot carry userinfo, which is also
    // a scheme that has none to strip.
    let _ = url.set_username("");
    let _ = url.set_password(None);

    // Rebuilt by hand rather than through `query_pairs_mut`, which would
    // percent-encode the sentinel into `%3Credacted%3E` and re-encode every
    // value it kept. Splitting the raw query leaves the parameters a reader
    // needs exactly as they were sent and writes the sentinel plainly.
    let query = url.query().map(scrub_query);
    url.set_query(None);
    // The fragment goes whole: it is the one part of a URL a client never
    // sends, so it says nothing about which repository was contacted, and it
    // has no defined shape — `#access_token=hunter2` looks like a query, but
    // `#hunter2` is just as legal, and no `name=value` rule can classify
    // that. Scrubbing what cannot be parsed is guessing; dropping it costs
    // the record nothing it was keeping.
    url.set_fragment(None);
    let mut out = url.to_string();
    if let Some(q) = query {
        out.push('?');
        out.push_str(&q);
    }
    Some(out)
}

/// A URL's raw query with every credential-bearing parameter's value replaced
/// and every other parameter left byte for byte as it arrived.
///
/// The parameter's NAME survives, so the record still says a signed URL was
/// used — a pre-signed URL puts the whole credential in the query (`sig`,
/// `X-Amz-Signature`, `access_key`) and "there was a signature here" is the
/// part an operator reading the trail needs.
fn scrub_query(query: &str) -> String {
    /// Substring-matched against a lowercased parameter name, like `NEEDLES`.
    /// Over-matching costs a reader the value of a parameter that was not a
    /// credential; under-matching costs a credential on disk.
    const QUERY_NEEDLES: [&str; 9] = [
        "token",
        "secret",
        "password",
        "passwd",
        "key",
        "sig",
        "credential",
        "auth",
        "session",
    ];

    query
        .split('&')
        .map(|pair| {
            let Some((name, _)) = pair.split_once('=') else {
                // A bare flag has no value to hide.
                return pair.to_string();
            };
            // Classified as the server will read the name, not as it was
            // spelled: `to%6ben` is `token`, and `%53ECRET` is `secret` once
            // the decode runs before the case fold. Matching the raw bytes
            // let either write its value to disk. `form_urlencoded` is the
            // decoder the server uses — percent escapes and `+` for space,
            // both — and lossy UTF-8 keeps a malformed escape matchable
            // rather than dropping the parameter out of the check.
            let decoded = url::form_urlencoded::parse(name.as_bytes())
                .next()
                .map(|(decoded, _)| decoded.into_owned())
                .unwrap_or_default();
            let lower = decoded.to_ascii_lowercase();
            if QUERY_NEEDLES.iter().any(|n| lower.contains(n)) {
                format!("{name}={REDACTED}")
            } else {
                pair.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// Scrub from an error message every value that `redact` dropped from `args`.
///
/// A capability that refuses an argument tends to echo it — the registry maps
/// serde's error to a string as-is, and for a scalar `settings` that reads
/// `invalid type: string "hunter2", expected a map` — and `handle_request`
/// records the message beside the redacted arguments, which would put the
/// value straight back in the log. So every string or number that is in
/// `args` and not in `redacted` is replaced wherever it appears, longest
/// first so a value that contains another is not left half visible, and in
/// the escaped form serde's `{:?}` prints as well as verbatim. Values the
/// redaction kept are left alone, so a message naming the app or the
/// namespace still says which one.
///
/// Over-scrubbing is the safe direction: a hidden value that happens to be an
/// ordinary word costs a few characters of an error text, where the
/// alternative is a credential on disk.
///
/// The arguments are untrusted and can be large (up to `MAX_REQUEST_BYTES`,
/// 4 MiB, on either MCP transport — the constant is `srelens_mcp`'s, which
/// this crate cannot link to without depending on its own dependent), and a
/// denied call is scrubbed too, so this stays near-linear in the number of
/// values: membership is a hash lookup, and each distinct hidden value is
/// replaced once.
///
/// **Returns [`UNSCRUBBABLE`] rather than panicking or passing the message
/// through** when the matcher cannot be built from those values — see
/// [`scrub_or_drop`]. Failing closed here costs one sentence of an audit
/// record; failing open would write the value the redaction just removed.
pub fn redact_error(error: &str, args: &Value, redacted: &Value) -> String {
    use std::collections::HashSet;

    fn leaves(v: &Value, out: &mut HashSet<String>) {
        match v {
            Value::String(s) => {
                out.insert(s.clone());
            }
            Value::Number(n) => {
                out.insert(n.to_string());
            }
            Value::Object(m) => m.values().for_each(|v| leaves(v, out)),
            Value::Array(a) => a.iter().for_each(|v| leaves(v, out)),
            Value::Bool(_) | Value::Null => {}
        }
    }
    let mut kept = HashSet::new();
    leaves(redacted, &mut kept);
    let mut all = HashSet::new();
    leaves(args, &mut all);
    let hidden: Vec<String> = all
        .into_iter()
        .filter(|s| !s.is_empty() && !kept.contains(s))
        .collect();

    if hidden.is_empty() {
        return error.to_string();
    }

    let mut patterns = Vec::new();
    for value in hidden {
        let escaped = format!("{value:?}");
        let escaped = escaped[1..escaped.len() - 1].to_string();
        if escaped != value {
            patterns.push(escaped);
        }
        patterns.push(value);
    }

    patterns.sort_by_key(|s| std::cmp::Reverse(s.len()));
    patterns.dedup();

    // A matcher that will not build drops the message rather than passing it
    // through — see `scrub_or_drop`, which takes the `Result` so that policy
    // has a test.
    let built = aho_corasick::AhoCorasick::builder()
        .match_kind(aho_corasick::MatchKind::LeftmostFirst)
        .build(&patterns);
    scrub_or_drop(built, error, &patterns)
}

/// Replace every pattern in `error`, or drop the message if the matcher
/// could not be built.
///
/// **A scrub that cannot be built drops the message, it does not panic.**
///
/// `patterns` is built from the caller's own argument values, and this
/// repository's rule is no `unwrap`/`expect` on caller data
/// (`.coderabbit.yaml`). `AhoCorasick::build` returns `BuildError` when a
/// pattern is longer than `SmallIndex::MAX`, or when the patterns together
/// need more states or IDs than a 32-bit index holds — roughly two gigabytes
/// of argument values in one call. MCP cannot reach that (`MAX_REQUEST_BYTES`
/// caps a request at 4 MiB) but the desktop bridge takes whatever the WebView
/// hands it, with no transport limit of its own.
///
/// Two reasons this is a fallback rather than a panic, and the second is the
/// stronger one:
///
/// - the panic would unwind through [`crate::Registry::invoke_audited`], i.e.
///   the audit path would take down the capability call it was recording. The
///   log's whole posture is the opposite: "a lost log line must never break a
///   working cluster operation" ([`JsonlAuditLog::record`], which swallows
///   every I/O error for exactly this reason);
/// - there is nothing safe to fall back TO except silence. The reason
///   [`redact_error`] exists is that a capability which refuses an argument
///   echoes it, so writing the unscrubbed message would put the value the
///   redaction just hid straight back on disk. Failing closed means the
///   record keeps its redacted arguments and loses only the sentence that
///   could not be cleaned.
///
/// **The build's `Result` is a parameter so the fallback can be tested.** It
/// is generic in the error because the only cheap way to reach this branch is
/// to hand it one; provoking a real `BuildError` means allocating gigabytes.
fn scrub_or_drop<E>(
    built: Result<aho_corasick::AhoCorasick, E>,
    error: &str,
    patterns: &[String],
) -> String {
    let Ok(ac) = built else {
        return UNSCRUBBABLE.to_string();
    };
    let replacements = vec![REDACTED; patterns.len()];
    ac.replace_all(error, &replacements)
}

/// What a value the redaction removed is replaced by, in arguments and in the
/// error text alike.
const REDACTED: &str = "<redacted>";

/// The whole of an error message that could not be scrubbed. Not the original
/// text: the values this function was asked to hide are, by construction, the
/// ones an error is most likely to be echoing.
const UNSCRUBBABLE: &str = "<redacted: the message could not be scrubbed>";

/// The most recent `limit` entries, newest first.
///
/// Reads at most a bounded window from the END of the log rather than the whole
/// file: the log is capped at 5 MB and a caller only ever wants the last
/// handful, so parsing every line to discard nearly all of them is wasted work
/// on every Settings open. Unparseable lines are skipped — a torn final write,
/// or the fragment of a line the window's start lands inside, is not an error.
///
/// Only the live log is read; entries rotated into `.jsonl.1` are not included.
///
/// **An absent log is an empty trail; an unreadable one is an error.** This
/// used to return `Vec::new()` for three distinct failures — `File::open`,
/// `seek` and `read_to_end` — so a log that exists and cannot be read was
/// indistinguishable from a fresh install. The pane at the other end of the
/// call says "A fresh install has made none — this is not an error." for an
/// empty result, on the one screen whose purpose is answering what an agent did
/// after an incident, so that collapse turned a permissions change or a
/// filesystem fault into a clean bill of health. `NotFound` is the single
/// outcome still swallowed, because a fresh install genuinely has no log: it is
/// the one absence that is a fact rather than a gap in what srelens knows.
pub fn tail(path: &std::path::Path, limit: usize) -> std::io::Result<Vec<Value>> {
    /// Generous next to a realistic `limit` (tens of entries at a few hundred
    /// bytes each) while staying a small fraction of the 5 MB cap.
    const WINDOW: u64 = 512 * 1024;

    use std::io::{Read, Seek, SeekFrom};

    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        // The only silent case, and the only one that is really empty.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    // Propagated rather than falling back to 0: a metadata call that fails on
    // an open handle is a real fault, and answering "the log starts here" from
    // a length nobody could read is the same guess this function stopped
    // making.
    let len = f.metadata()?.len();
    let start = len.saturating_sub(WINDOW);
    if start > 0 {
        f.seek(SeekFrom::Start(start))?;
    }
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;
    // Lossy: a window start can land inside a multi-byte character, and a
    // mangled leading fragment is discarded below regardless.
    let text = String::from_utf8_lossy(&buf);
    let mut lines: Vec<&str> = text.lines().collect();
    if start > 0 && !lines.is_empty() {
        // Seeking mid-file almost certainly lands inside a line; that fragment
        // is not a record.
        lines.remove(0);
    }
    // Unparseable LINES are still skipped rather than raised: a torn final
    // write is a known property of an append-only log, not a failed read. A
    // line that IS valid JSON is kept whatever build wrote it — see
    // `upgrade_record`.
    Ok(lines
        .iter()
        .rev()
        .filter_map(|l| serde_json::from_str(l).ok())
        .map(upgrade_record)
        .take(limit)
        .collect())
}

/// Bring a line written by an older build up to the current record shape.
///
/// **The log is not new.** `audit.jsonl` shipped well before #555 split
/// `source` out of `transport` and gave `outcome` a three-word vocabulary, so
/// an installed copy of srelens has lines on disk reading `"outcome": "error"`
/// with no `source`, `app`, `cluster` or `resource` at all. Those are real
/// records of real calls and the pane has to keep showing them — and show them
/// *right*: the pane's verdict falls through an outcome it does not recognise
/// to "allowed", so last week's failed call would render as one that went
/// through. That is the same class of wrong answer — a failure rendered as a
/// fact — this pane exists to prevent, so the upgrade happens here, at the one
/// place that reads the format, rather than in each reader.
///
/// - **`source`** — every pre-#555 record was an MCP call by construction: the
///   UI path could not reach the sink at all. Missing means `mcp`.
/// - **`outcome`** — `error` becomes `rejected` when consent was denied
///   (nothing ran) and `failed` otherwise. The old format cannot tell a
///   refused argument from a handler failure; calling a refusal a failure
///   overstates what happened rather than understating it, which is the safer
///   direction on this screen.
/// - **`app` / `cluster` / `resource`** — filled with `null`, which is what
///   they mean: this record does not say. The pane falls back to reading the
///   arguments, as it did before these fields existed.
///
/// A record already in the current shape is returned untouched, and anything
/// that is not a JSON object is left exactly as it was: this upgrades what it
/// recognises and never invents a field it cannot justify.
fn upgrade_record(mut line: Value) -> Value {
    let Some(map) = line.as_object_mut() else {
        return line;
    };
    if !map.contains_key("source") {
        map.insert("source".into(), json!("mcp"));
    }
    if map.get("outcome") == Some(&json!("error")) {
        let denied = map.get("decision") == Some(&json!("denied"));
        map.insert(
            "outcome".into(),
            json!(if denied {
                OUTCOME_REJECTED
            } else {
                OUTCOME_FAILED
            }),
        );
    }
    for absent in ["app", "cluster", "resource"] {
        map.entry(absent).or_insert(Value::Null);
    }
    line
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
            // Both, and not one derived from the other at read time: `source`
            // is the question the pane asks ("was this me or an agent?") and
            // `transport` is the detail under it.
            "source": rec.source.as_str(),
            "transport": rec.source.transport(),
            "tool": rec.tool,
            "args": rec.args,
            // Always present, `null` when the call named none, so a reader
            // never has to tell "absent" from "unknown".
            "app": rec.app.as_ref().map(|a| json!({ "id": a.id, "revision": a.revision })),
            "cluster": rec.cluster,
            "resource": rec.resource,
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
            eprintln!("srelens: could not write the capability audit log at {}", self.path.display());
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
    fn a_url_keeps_its_scheme_host_and_path_and_loses_its_userinfo() {
        let args = json!({ "url": "https://deploy:s3cr3t@charts.example.com/stable" });
        let out = redact(&args, false);
        assert_eq!(out["url"], json!("https://charts.example.com/stable"));
    }

    #[test]
    fn a_urls_credential_query_parameters_are_blanked_and_the_rest_are_kept() {
        let args = json!({
            "url": "https://charts.example.com/s?access_key=AKIA1&region=eu&sig=abc&api_key=k",
        });
        let out = redact(&args, false);
        assert_eq!(
            out["url"],
            json!(
                "https://charts.example.com/s?access_key=<redacted>&region=eu\
                 &sig=<redacted>&api_key=<redacted>"
            )
        );
    }

    /// A parameter name is matched as the server will read it, not as it was
    /// spelled: `to%6ben` is `token`, and a check against the raw bytes hands
    /// the value to `audit.jsonl` intact.
    #[test]
    fn a_percent_encoded_credential_parameter_name_is_still_recognised() {
        let args = json!({ "url": "https://charts.example.com/s?to%6ben=hunter2" });
        let out = redact(&args, false);
        assert_eq!(
            out["url"],
            json!("https://charts.example.com/s?to%6ben=<redacted>")
        );
    }

    /// The same for a name that is both encoded and upper-cased — decoding
    /// happens before the case fold, not instead of it.
    #[test]
    fn an_encoded_upper_case_credential_parameter_name_is_still_recognised() {
        let args = json!({ "url": "https://charts.example.com/s?%53ECRET=pw&region=eu" });
        let out = redact(&args, false);
        assert_eq!(
            out["url"],
            json!("https://charts.example.com/s?%53ECRET=<redacted>&region=eu")
        );
    }

    /// A fragment never reaches the server, so it says nothing about which
    /// repository was contacted — and it has no `name=value` shape to hold a
    /// credential rule, so `#hunter2` could not be classified even in
    /// principle. It goes.
    #[test]
    fn a_urls_fragment_is_dropped() {
        let args = json!({ "url": "https://charts.example.com/s#access_token=hunter2" });
        let out = redact(&args, false);
        assert_eq!(out["url"], json!("https://charts.example.com/s"));
    }

    /// A credential does not become safe by sitting under a key the redaction
    /// has no name for: `chart` on `k8s.helmInstall` takes `oci://…` and
    /// `https://….tgz` as readily as `bitnami/nginx`.
    #[test]
    fn a_url_is_scrubbed_wherever_it_appears_not_only_under_a_url_named_key() {
        let args = json!({ "chart": "oci://robot:pw@registry.example.com/charts/app" });
        let out = redact(&args, false);
        assert_eq!(out["chart"], json!("oci://registry.example.com/charts/app"));
    }

    #[test]
    fn a_string_that_is_not_a_url_is_left_readable() {
        let args = json!({ "chart": "bitnami/nginx", "name": "web@1" });
        let out = redact(&args, false);
        assert_eq!(out["chart"], json!("bitnami/nginx"));
        assert_eq!(out["name"], json!("web@1"));
    }

    /// Fail closed: something that announces itself as a URL and will not
    /// parse is exactly the case where the parser cannot say which part was
    /// the password.
    #[test]
    fn an_unparseable_url_is_redacted_whole() {
        let args = json!({
            "url": "deploy:s3cr3t@charts.example.com",
            "endpoint": "https://user:pw@[not-an-address",
        });
        let out = redact(&args, false);
        assert_eq!(out["url"], json!("<redacted>"));
        assert_eq!(out["endpoint"], json!("<redacted>"));
    }

    #[test]
    fn writes_one_json_line_per_record() {
        let dir = std::env::temp_dir().join(format!("srelens-audit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.jsonl");
        let _ = std::fs::remove_file(&path);
        let log = JsonlAuditLog::new(path.clone(), 1024 * 1024);
        log.record(AuditRecord {
            source: Source::McpHttp,
            app: None,
            cluster: None,
            resource: None,
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
        assert_eq!(parsed["source"], json!("mcp"));
        assert_eq!(parsed["transport"], json!("http"));
        assert_eq!(parsed["decision"], json!("approved"));
        assert!(parsed["ts"].as_u64().unwrap() > 0);
    }

    fn write_entries(path: &std::path::Path, count: usize, pad: usize) {
        let log = JsonlAuditLog::new(path.to_path_buf(), u64::MAX); // never rotate
        for i in 0..count {
            log.record(AuditRecord {
                source: Source::McpStdio,
                app: None,
                cluster: None,
                resource: None,
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

        let out = tail(&path, 3).expect("a readable log is not an error");

        assert_eq!(out.len(), 3);
        assert_eq!(out[0]["tool"], json!("tool9"), "newest first");
        assert_eq!(out[1]["tool"], json!("tool8"));
        assert_eq!(out[2]["tool"], json!("tool7"));
    }

    /// A log that does not exist is an EMPTY trail, not a failure: a fresh
    /// install has made no capability calls, and the file is written on the
    /// first one. This is the only I/O outcome `tail` is entitled to swallow.
    #[test]
    fn tail_of_a_missing_log_is_an_empty_trail_and_not_an_error() {
        let out = tail(std::path::Path::new("/nonexistent/srelens/audit.jsonl"), 50)
            .expect("an absent log is a fresh install, not a failure");
        assert!(out.is_empty(), "a log that was never written is not an error");
    }

    /// The finding: three distinct failures — `File::open`, `seek` and
    /// `read_to_end` — all returned `Vec::new()`, so an audit file that exists
    /// and cannot be READ was indistinguishable from a fresh install. The pane
    /// on the other end of this call says "A fresh install has made none — this
    /// is not an error." for an empty vector, on the one screen whose whole
    /// purpose is answering what an agent did after an incident.
    ///
    /// A DIRECTORY at the log's path, rather than a `chmod 000` file: it is a
    /// path that exists and cannot be read as a file on every platform and at
    /// every euid, where a permission bit is simply ignored for root and the
    /// test would then pass by reading the file it meant to be refused.
    #[test]
    fn tail_of_a_log_that_cannot_be_read_is_an_error_not_an_empty_trail() {
        let dir = std::env::temp_dir().join(format!("srelens-tailunread-{}", std::process::id()));
        let path = dir.join("as-a-directory.jsonl");
        std::fs::create_dir_all(&path).unwrap();

        let out = tail(&path, 50);

        let err = out.expect_err("a log that exists and cannot be read is not an empty trail");
        assert_ne!(
            err.kind(),
            std::io::ErrorKind::NotFound,
            "the file is right there; only a genuinely absent log may read as empty"
        );
    }

    /// The `File::open` half of the same property, on a refusal that is not
    /// `NotFound`: the log's parent is a regular file, so opening it fails with
    /// ENOTDIR. A fresh install's absent log and a log srelens cannot get at
    /// are different answers and this is the one that must not be silent.
    #[test]
    fn tail_of_a_log_that_cannot_be_opened_is_an_error_not_an_empty_trail() {
        let dir = std::env::temp_dir().join(format!("srelens-tailopen-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let blocker = dir.join("not-a-directory");
        std::fs::write(&blocker, "i am a file\n").unwrap();

        let out = tail(&blocker.join("audit.jsonl"), 50);

        let err = out.expect_err("a log that cannot be opened is not an empty trail");
        assert_ne!(err.kind(), std::io::ErrorKind::NotFound);
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

        let out = tail(&path, 100_000).expect("a readable log is not an error");

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

        let out = tail(&path, 100_000).expect("a readable log is not an error");

        assert!(
            out.iter().all(|e| e.get("tool").is_some()),
            "every returned row must be a fully parsed entry, not a fragment"
        );
    }

    fn a_record() -> AuditRecord {
        AuditRecord {
            source: Source::McpHttp,
            app: None,
            cluster: None,
            resource: None,
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
                source: Source::McpStdio,
                app: None,
                cluster: None,
                resource: None,
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

    /// Issue #605. An app's settings are free-form JSON and nothing marks a
    /// value as secret, so a `settings` action on `extensions.configure` with
    /// a value under `credential` or `certificate` — no needle matches either —
    /// was written to the log verbatim, and stayed in `audit.jsonl.1` after
    /// rotation. The setting NAMES are the auditable shape (which knobs an
    /// agent turned); the values are the secret material and every one goes,
    /// at any depth, alongside the action and app ID an operator needs.
    #[test]
    fn redacts_every_extension_setting_value_but_keeps_the_action_id_and_setting_keys() {
        let args = json!({
            "action": "settings",
            "id": "org.example.argocd",
            "settings": {
                "credential": "hunter2",
                "endpoint": "https://argo.example",
                "tls": { "certificate": "-----BEGIN CERTIFICATE-----" }
            }
        });
        let out = redact(&args, false);
        assert_eq!(
            out["action"],
            json!("settings"),
            "the action must stay visible"
        );
        assert_eq!(
            out["id"],
            json!("org.example.argocd"),
            "the app ID must stay visible"
        );
        let settings = out["settings"]
            .as_object()
            .expect("setting keys must survive");
        assert_eq!(
            settings.len(),
            3,
            "every setting key must survive, got {settings:?}"
        );
        assert_eq!(settings["credential"], json!("<redacted>"));
        assert_eq!(
            settings["endpoint"],
            json!("<redacted>"),
            "no setting value is known safe"
        );
        assert_eq!(
            settings["tls"],
            json!("<redacted>"),
            "a nested map goes whole"
        );
        let line = out.to_string();
        assert!(!line.contains("hunter2"), "the credential leaked: {line}");
        assert!(
            !line.contains("BEGIN CERTIFICATE"),
            "the certificate leaked: {line}"
        );
        assert!(
            !line.contains("argo.example"),
            "a setting value leaked: {line}"
        );
    }

    /// A denied call is audited before its arguments are ever deserialized, so
    /// `settings` need not be the object the capability's schema demands. A
    /// scalar or array there is blanked whole rather than walked, where the
    /// scalars would come through untouched.
    #[test]
    fn redacts_a_settings_payload_that_is_not_an_object_whole() {
        for settings in [
            json!("hunter2"),
            json!(["hunter2"]),
            json!([{ "v": "hunter2" }]),
        ] {
            let args =
                json!({ "action": "settings", "id": "org.example.argocd", "settings": settings });
            let out = redact(&args, false);
            assert_eq!(out["settings"], json!("<redacted>"), "got {out}");
            assert!(!out.to_string().contains("hunter2"), "leaked: {out}");
        }
    }

    /// PR #625 review. A capability that refuses an argument tends to echo it:
    /// the registry maps serde's error straight to a string, and for a scalar
    /// `settings` that is `invalid type: string "hunter2", expected a map`.
    /// `handle_request` records the error beside the redacted arguments, which
    /// put the value straight back in the log. Every value redaction hid must
    /// be scrubbed from the message; everything it kept is left alone.
    #[test]
    fn redact_error_scrubs_the_values_redaction_hid_and_nothing_else() {
        let args =
            json!({ "action": "settings", "id": "org.example.argocd", "settings": "hunter2" });
        let redacted = redact(&args, false);
        let error = "invalid input: invalid type: string \"hunter2\", expected a map";

        let out = redact_error(error, &args, &redacted);

        assert!(!out.contains("hunter2"), "the refused value leaked: {out}");
        assert!(
            out.contains("expected a map"),
            "the rest of the message survives: {out}"
        );

        let visible = redact_error("no app org.example.argocd is installed", &args, &redacted);
        assert_eq!(
            visible, "no app org.example.argocd is installed",
            "kept values are not scrubbed"
        );
    }

    /// serde prints the value it echoes with `{:?}`, so a value holding a quote
    /// or a newline appears escaped, not verbatim; a number appears bare.
    #[test]
    fn redact_error_scrubs_escaped_and_numeric_values_too() {
        let args =
            json!({ "action": "settings", "id": "org.example.argocd", "settings": "hun\"ter\n2" });
        let redacted = redact(&args, false);
        let error = "invalid input: invalid type: string \"hun\\\"ter\\n2\", expected a map";
        let out = redact_error(error, &args, &redacted);
        assert!(!out.contains("ter"), "the escaped value leaked: {out}");

        let args = json!({ "action": "settings", "id": "org.example.argocd", "settings": { "pin": 4711 } });
        let redacted = redact(&args, false);
        let out = redact_error(
            "handler error: pin 4711 is not four digits",
            &args,
            &redacted,
        );
        assert!(!out.contains("4711"), "the numeric value leaked: {out}");
    }

    /// PR #625 review. The arguments of a denied call are untrusted and, on
    /// either transport, up to 4 MiB. A `settings` map of very many
    /// short values gives `redact_error` one hidden value per setting and one
    /// `<redacted>` kept leaf per setting, and a linear membership scan per
    /// hidden value made the scrub quadratic in the number of settings — a
    /// denial that took billions of comparisons to record. Ten-character values
    /// match `<redacted>`'s length, so a scan compares bytes, not just lengths.
    #[test]
    fn redact_error_stays_near_linear_in_the_number_of_hidden_values() {
        let settings: serde_json::Map<String, Value> = (0..100_000)
            .map(|i| (format!("k{i}"), json!(format!("v{i:09}"))))
            .collect();
        let args =
            json!({ "action": "settings", "id": "org.example.argocd", "settings": settings });
        let redacted = redact(&args, false);
        let error =
            "`extensions.configure` mutates the cluster and no consent mechanism is configured";

        let started = std::time::Instant::now();
        let out = redact_error(error, &args, &redacted);
        let took = started.elapsed();

        assert_eq!(out, error, "nothing hidden appears in this message");
        assert!(
            took < std::time::Duration::from_secs(5),
            "scrubbing 100k hidden values took {took:?}; the scan is not linear"
        );
    }

    /// The other `extensions.configure` actions carry no settings, and their
    /// audit shape is unchanged: an operator can still read which app was
    /// installed with which grants, enabled, or limited to which clusters.
    #[test]
    fn other_configure_actions_keep_their_audit_shape() {
        let install = redact(
            &json!({ "action": "install", "manifest": "{\"id\":\"org.example.argocd\"}", "grants": ["k8s.listCustomResource"] }),
            false,
        );
        assert_eq!(install["action"], json!("install"));
        assert_eq!(
            install["manifest"],
            json!("{\"id\":\"org.example.argocd\"}")
        );
        assert_eq!(install["grants"], json!(["k8s.listCustomResource"]));

        let enable = redact(
            &json!({ "action": "enable", "id": "org.example.argocd", "enabled": false }),
            false,
        );
        assert_eq!(enable["id"], json!("org.example.argocd"));
        assert_eq!(enable["enabled"], json!(false));

        let clusters = redact(
            &json!({ "action": "clusters", "id": "org.example.argocd", "contexts": ["prod", "staging"] }),
            false,
        );
        assert_eq!(clusters["contexts"], json!(["prod", "staging"]));
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

    /// The two questions the trail answers about where a call came from, and
    /// the two fields that answer them. An operator asks "was that me or an
    /// agent?" first — `ui` and `mcp` — and only then which client.
    #[test]
    fn a_source_says_who_called_and_how_they_reached_the_registry() {
        assert_eq!(Source::Ui.as_str(), "ui");
        assert_eq!(Source::Ui.transport(), "ui");
        assert_eq!(Source::McpStdio.as_str(), "mcp");
        assert_eq!(Source::McpStdio.transport(), "stdio");
        assert_eq!(Source::McpHttp.as_str(), "mcp");
        assert_eq!(Source::McpHttp.transport(), "http");
    }

    /// #555's line for the UI path, read off the annotations rather than a
    /// second list of capability names that would drift from them.
    #[test]
    fn the_ui_records_mutations_and_sensitive_reads_and_nothing_else() {
        assert!(!is_audited_from_ui(&Annotations::READ_ONLY));
        assert!(is_audited_from_ui(&Annotations::MUTATING));
        assert!(is_audited_from_ui(&Annotations::DESTRUCTIVE));
        assert!(
            is_audited_from_ui(&Annotations::SENSITIVE_READ),
            "a read that returns secret material is an event even though it changes nothing"
        );
        assert!(
            is_audited_from_ui(&Annotations {
                sensitive: true,
                ..Annotations::READ_ONLY
            }),
            "`k8s.diffManifest` is read-only and sensitive; what it can echo back is why"
        );
    }

    /// `extensions.action` carries its whole selection — app ID, revision,
    /// cluster and object — nested under `resource`, and that is the exact
    /// call #555 was opened about: an Argo CD sync clicked in the app.
    #[test]
    fn an_app_action_names_its_app_revision_cluster_and_object() {
        let (app, cluster, resource) = describe_target(&json!({
            "action": "sync",
            "uid": "1234",
            "resourceVersion": "9",
            "resource": {
                "id": "org.example.argocd", "revision": 4,
                "capability": "applications",
                "context": "prod", "namespace": "team", "name": "web"
            }
        }));
        assert_eq!(
            app,
            Some(AppRef {
                id: "org.example.argocd".into(),
                revision: 4
            })
        );
        assert_eq!(cluster.as_deref(), Some("prod"));
        assert_eq!(resource.as_deref(), Some("team/web"));
    }

    /// The flat shape, and the node-scoped capabilities that have no namespace
    /// at all (`node.cordon`, `node.drain`).
    #[test]
    fn a_flat_call_names_its_cluster_and_object_including_node_scoped_ones() {
        let (app, cluster, resource) =
            describe_target(&json!({ "context": "prod", "namespace": "team", "name": "web" }));
        assert_eq!(app, None, "a call made outside an app names none");
        assert_eq!(cluster.as_deref(), Some("prod"));
        assert_eq!(resource.as_deref(), Some("team/web"));

        let (_, _, node) = describe_target(&json!({ "context": "prod", "node": "ip-10-0-1-7" }));
        assert_eq!(node.as_deref(), Some("ip-10-0-1-7"));

        let (_, _, namespace_only) =
            describe_target(&json!({ "context": "prod", "namespace": "team" }));
        assert_eq!(
            namespace_only.as_deref(),
            Some("team"),
            "a namespace is a target"
        );

        let (app, cluster, resource) = describe_target(&json!({}));
        assert_eq!((app, cluster, resource), (None, None, None));
    }

    /// Read off the REDACTED arguments, never the caller's: a sensitive
    /// capability blanks every value, and pulling the Secret's own name out of
    /// the raw arguments into a top-level `resource` field would walk it
    /// straight back into the log through a door the redaction does not watch.
    #[test]
    fn a_sensitive_calls_target_is_whatever_survived_redaction() {
        let args = json!({ "context": "prod", "namespace": "team", "name": "db-creds" });
        let (_, cluster, resource) = describe_target(&redact(&args, true));
        assert_eq!(cluster.as_deref(), Some("<redacted>"));
        assert_eq!(resource.as_deref(), Some("<redacted>/<redacted>"));
    }

    /// A matcher that could not be built drops the message rather than
    /// passing it through, and rather than panicking through the capability
    /// call it was only supposed to be recording.
    ///
    /// The build failure is injected instead of provoked: `AhoCorasick::build`
    /// fails on a pattern past `SmallIndex::MAX` or on IDs past a 32-bit
    /// index, which takes on the order of two gigabytes of argument values in
    /// one call — allocating that is not a unit test, it is an OOM with an
    /// assertion attached. [`scrub_or_drop`] takes the build's `Result`, so
    /// the policy under test is reachable without the allocation that would
    /// produce a real `BuildError`.
    #[test]
    fn a_matcher_that_cannot_be_built_drops_the_message_instead_of_passing_it_on() {
        let patterns = vec!["hunter2".to_string()];
        let failed: Result<aho_corasick::AhoCorasick, &str> = Err("pattern too long");

        let out = scrub_or_drop(failed, "invalid token hunter2", &patterns);

        assert_eq!(out, UNSCRUBBABLE);
        assert!(
            !out.contains("hunter2"),
            "the value the redaction hid must not come back through the error"
        );
    }

    /// The same seam on the path that works: a built matcher replaces every
    /// pattern, so the fallback is the only thing the test above isolates.
    #[test]
    fn a_matcher_that_builds_replaces_every_hidden_value() {
        let patterns = vec!["hunter2".to_string()];
        let built = aho_corasick::AhoCorasick::builder()
            .match_kind(aho_corasick::MatchKind::LeftmostFirst)
            .build(&patterns);

        assert_eq!(
            scrub_or_drop(built, "invalid token hunter2", &patterns),
            format!("invalid token {REDACTED}")
        );
    }

    /// The two redactions have to speak one vocabulary: an operator reading a
    /// row sees the arguments and the reason beside each other, and two
    /// spellings of "gone" would read as two different things having happened.
    #[test]
    fn the_arguments_and_the_error_are_redacted_in_the_same_words() {
        let args = json!({ "name": "web", "apiToken": "hunter2" });
        let redacted = redact(&args, false);

        assert_eq!(redacted["apiToken"], json!(REDACTED));
        assert_eq!(
            redact_error("invalid token hunter2", &args, &redacted),
            format!("invalid token {REDACTED}")
        );
        assert_ne!(
            UNSCRUBBABLE, REDACTED,
            "a whole message that could not be cleaned is not the same event as one hidden value"
        );
    }

    /// The upgrade case, written as the old format actually wrote it. An
    /// installed srelens has these lines on disk — `audit.jsonl` predates
    /// #555 — and the pane's verdict falls through an unrecognised outcome to
    /// "allowed", so without this a failed call from before the upgrade would
    /// render as one that went through.
    #[test]
    fn a_record_from_before_the_source_field_reads_as_a_failed_mcp_call() {
        let dir = std::env::temp_dir().join(format!("srelens-legacy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("legacy.jsonl");
        let _ = std::fs::remove_file(&path);
        std::fs::write(
            &path,
            // Verbatim in the pre-#555 shape: no source, no app, no cluster,
            // no resource, and the outcome vocabulary of the day.
            "{\"ts\":1700000000,\"transport\":\"http\",\"tool\":\"k8s.deletePod\",\
             \"args\":{\"context\":\"prod\",\"name\":\"web-0\"},\"decision\":\"approved\",\
             \"outcome\":\"error\",\"err\":\"handler error: timed out\"}\n",
        )
        .unwrap();

        let out = tail(&path, 10).expect("a readable log is not an error");

        assert_eq!(out.len(), 1, "a legacy line is still a record: {out:?}");
        assert_eq!(
            out[0]["source"],
            json!("mcp"),
            "nothing else could have written it"
        );
        assert_eq!(
            out[0]["transport"],
            json!("http"),
            "what it did say survives"
        );
        assert_eq!(
            out[0]["outcome"],
            json!(OUTCOME_FAILED),
            "an approved call that errored ran and did not finish"
        );
        for absent in ["app", "cluster", "resource"] {
            assert_eq!(
                out[0][absent],
                Value::Null,
                "{absent} must say it does not know"
            );
        }
        assert_eq!(
            out[0]["args"]["name"],
            json!("web-0"),
            "the arguments are untouched"
        );
    }

    /// The other half of the old `error`: a call the consent policy refused
    /// never ran, so it is a rejection and not a failure.
    #[test]
    fn a_legacy_denied_record_reads_as_rejected_not_failed() {
        let dir = std::env::temp_dir().join(format!("srelens-legacy2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("denied.jsonl");
        let _ = std::fs::remove_file(&path);
        std::fs::write(
            &path,
            "{\"ts\":1700000000,\"transport\":\"stdio\",\"tool\":\"k8s.getSecret\",\"args\":{},\
             \"decision\":\"denied\",\"outcome\":\"error\",\"err\":\"the user declined\"}\n",
        )
        .unwrap();

        let out = tail(&path, 10).expect("a readable log is not an error");

        assert_eq!(out[0]["outcome"], json!(OUTCOME_REJECTED));
        assert_eq!(
            out[0]["decision"],
            json!("denied"),
            "the decision is left alone"
        );
    }

    /// The upgrade must not rewrite a record that is already current — and a
    /// current `failed` must not be confused with a legacy `error`.
    #[test]
    fn a_current_record_passes_through_the_upgrade_untouched() {
        let dir = std::env::temp_dir().join(format!("srelens-legacy3-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("current.jsonl");
        let _ = std::fs::remove_file(&path);
        JsonlAuditLog::new(path.clone(), u64::MAX).record(AuditRecord {
            source: Source::Ui,
            tool: "extensions.action".into(),
            args: json!({}),
            app: Some(AppRef {
                id: "org.example.flux".into(),
                revision: 2,
            }),
            cluster: Some("prod".into()),
            resource: Some("team/web".into()),
            decision: "auto",
            outcome: OUTCOME_REJECTED,
            error: Some("a resourceVersion is required".into()),
        });

        let out = tail(&path, 10).expect("a readable log is not an error");

        assert_eq!(
            out[0]["source"],
            json!("ui"),
            "a UI record is not relabelled mcp"
        );
        assert_eq!(out[0]["outcome"], json!(OUTCOME_REJECTED));
        assert_eq!(out[0]["app"]["revision"], json!(2));
        assert_eq!(out[0]["cluster"], json!("prod"));
    }

    /// A line that is not JSON at all is skipped, and skipping it does not
    /// cost the reader the lines around it — an append-only log's last write
    /// can be torn.
    #[test]
    fn one_unreadable_line_does_not_cost_the_rest_of_the_trail() {
        let dir = std::env::temp_dir().join(format!("srelens-legacy4-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("torn.jsonl");
        let _ = std::fs::remove_file(&path);
        let mut body = String::new();
        body.push_str("{\"ts\":1,\"transport\":\"http\",\"tool\":\"a\",\"args\":{},\"decision\":\"auto\",\"outcome\":\"ok\"}\n");
        // A half-written record: valid JSON never resumes on this line.
        body.push_str("{\"ts\":2,\"transport\":\"http\",\"tool\":\"b\n");
        body.push_str("{\"ts\":3,\"transport\":\"http\",\"tool\":\"c\",\"args\":{},\"decision\":\"auto\",\"outcome\":\"ok\"}\n");
        std::fs::write(&path, body).unwrap();

        let out = tail(&path, 10).expect("a torn line is not a failed read");

        let tools: Vec<&str> = out.iter().filter_map(|e| e["tool"].as_str()).collect();
        assert_eq!(
            tools,
            vec!["c", "a"],
            "the readable records survive: {out:?}"
        );
    }

    /// An app ID with no revision beside it is not an app reference: an update
    /// rolls the revision and leaves the ID alone, so half of the pair cannot
    /// say which manifest and which grants were in force.
    #[test]
    fn an_app_id_without_a_revision_is_not_an_app_reference() {
        let (app, _, _) =
            describe_target(&json!({ "action": "enable", "id": "org.example.argocd" }));
        assert_eq!(app, None);
    }
}
