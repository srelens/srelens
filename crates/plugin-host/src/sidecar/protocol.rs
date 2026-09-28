//! The wire protocol between srelens and a sidecar: JSON-RPC 2.0, one message
//! per line on the sidecar's stdin and stdout (#572).
//!
//! - **Framing.** Each message is one line of UTF-8 JSON ending in `\n`
//!   (`\r\n` is accepted). A line is at most [`MAX_MESSAGE_BYTES`]. Batches are
//!   not part of the protocol. stderr is the sidecar's log, never protocol.
//! - **Ids.** The host numbers its requests from 1. A sidecar may use any
//!   string or number id for the calls it makes to the host.
//! - **Strictness.** Anything a sidecar writes that is not a well-formed
//!   message is a [`Violation`], and the supervisor stops the process: once the
//!   framing is lost, no later answer can be trusted to belong to its request.
//!
//! The methods and their parameters are listed in
//! `docs/extensions/sidecar-protocol.md`.

use serde_json::{json, Map, Value};
use std::fmt;
use std::path::Path;
use tokio::io::{AsyncBufRead, AsyncBufReadExt};

use super::Limits;

pub use srelens_sidecar_protocol::{
    code, is_reserved, method, RpcError, MAX_MESSAGE_BYTES, SIDECAR_API_VERSIONS,
};

/// One message a sidecar wrote.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    /// The answer to the host's request `id`.
    Response {
        id: u64,
        outcome: Result<Value, RpcError>,
    },
    /// A call the sidecar makes to the host, which must be answered.
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    /// A message that expects no answer.
    Notification { method: String, params: Value },
}

/// Something a sidecar wrote that is not a well-formed message. The text
/// completes "The extension …".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation(pub String);

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn violation(what: impl Into<String>) -> Violation {
    Violation(what.into())
}

/// Parse one line a sidecar wrote.
pub fn parse(line: &[u8]) -> Result<Incoming, Violation> {
    let text = std::str::from_utf8(line)
        .map_err(|_| violation("wrote a line that is not UTF-8 to its standard output"))?;
    let value: Value = serde_json::from_str(text)
        .map_err(|_| violation("wrote a line that is not JSON to its standard output"))?;
    let Value::Object(message) = value else {
        return Err(violation(
            "wrote JSON that is not a JSON-RPC message object (batches are not supported)",
        ));
    };
    if message.get("jsonrpc") != Some(&Value::String("2.0".into())) {
        return Err(violation("wrote a message without \"jsonrpc\": \"2.0\""));
    }
    if let Some(method) = message.get("method") {
        let method = method
            .as_str()
            .ok_or_else(|| violation("wrote a message whose method is not a string"))?
            .to_owned();
        let params = params(&message)?;
        return match message.get("id") {
            None => Ok(Incoming::Notification { method, params }),
            Some(id @ (Value::String(_) | Value::Number(_))) => Ok(Incoming::Request {
                id: id.clone(),
                method,
                params,
            }),
            Some(_) => Err(violation(
                "sent a request whose id is not a string or number",
            )),
        };
    }
    let id = message
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| violation("sent a response to an id srelens never uses"))?;
    let outcome = match (message.get("result"), message.get("error")) {
        (Some(result), None) => Ok(result.clone()),
        (None, Some(error)) => Err(serde_json::from_value::<RpcError>(error.clone())
            .map_err(|_| violation("sent an error that is not a JSON-RPC error object"))?),
        _ => {
            return Err(violation(
                "sent a response without exactly one of \"result\" and \"error\"",
            ))
        }
    };
    Ok(Incoming::Response { id, outcome })
}

fn params(message: &Map<String, Value>) -> Result<Value, Violation> {
    match message.get("params") {
        None => Ok(Value::Null),
        Some(params @ (Value::Object(_) | Value::Array(_))) => Ok(params.clone()),
        Some(_) => Err(violation("sent params that are not an object or an array")),
    }
}

/// The line for request `id`. `serde_json` escapes every newline inside a
/// string, so a message is always exactly one line.
pub fn request(id: u64, method: &str, params: &Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string()
}

/// The line for a notification.
pub fn notification(method: &str, params: &Value) -> String {
    json!({"jsonrpc": "2.0", "method": method, "params": params}).to_string()
}

/// The line answering the sidecar's request `id`.
pub fn response(id: &Value, outcome: &Result<Value, RpcError>) -> String {
    match outcome {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err(error) => json!({"jsonrpc": "2.0", "id": id, "error": error}),
    }
    .to_string()
}

/// `initialize`'s params: every version the host speaks, the limits the
/// sidecar runs under, so an SDK can hold itself to them, and its data
/// directory (#573), the one path it may write.
pub fn initialize_params(offered: &[&str], limits: &Limits, data_dir: &Path) -> Value {
    json!({
        "apiVersions": offered,
        "host": {"name": "srelens", "version": env!("CARGO_PKG_VERSION")},
        "limits": {
            "requestTimeoutMs": limits.request_timeout.as_millis() as u64,
            "maxConcurrentRequests": limits.max_concurrent_requests,
            "maxStreams": limits.max_streams,
            "memoryBytes": limits.memory_bytes,
            "cpus": limits.cpus,
            "dataBytes": limits.data_bytes,
            "dataEntries": limits.data_entries,
        },
        "dataDirectory": data_dir.to_string_lossy(),
    })
}

/// The version the sidecar chose in its answer to `initialize`, if it is one
/// the host offered. Otherwise why not, as a sentence.
pub fn negotiated(offered: &[&str], result: &Value) -> Result<semver::Version, String> {
    let offered_text = offered.join(", ");
    let Some(chosen) = result.get("apiVersion").and_then(Value::as_str) else {
        return Err(format!(
            "The extension's answer to initialize names no apiVersion; srelens supports sidecar API {offered_text}"
        ));
    };
    let version = semver::Version::parse(chosen).map_err(|_| {
        format!("The extension chose sidecar API \"{chosen}\", which is not a version; srelens supports {offered_text}")
    })?;
    let known = offered
        .iter()
        .filter_map(|v| semver::Version::parse(v).ok())
        .any(|v| v == version);
    if !known {
        return Err(format!(
            "The extension chose sidecar API {version}, which srelens did not offer; srelens supports {offered_text}"
        ));
    }
    Ok(version)
}

/// When `error`, the answer to `initialize`, says the sidecar speaks none of
/// the offered versions: the sentence to show. `None` for any other error.
pub fn incompatible(offered: &[&str], error: &RpcError) -> Option<String> {
    if error.code != code::UNSUPPORTED_API_VERSION {
        return None;
    }
    let supported: Vec<&str> = error
        .data
        .as_ref()
        .and_then(|data| data.get("supported"))
        .and_then(Value::as_array)
        .map(|versions| versions.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let theirs = if supported.is_empty() {
        "none of the versions srelens offered".to_owned()
    } else {
        format!("sidecar API {}", supported.join(", "))
    };
    Some(format!(
        "The extension supports {theirs}; srelens supports sidecar API {}. Update the extension or srelens",
        offered.join(", ")
    ))
}

/// One line read by [`BoundedLines`].
#[derive(Debug, PartialEq, Eq)]
pub enum Line {
    /// A line of at most the limit, without its line ending. Not checked for
    /// UTF-8: the protocol refuses what is not, a log shows it lossily.
    Bytes(Vec<u8>),
    /// A line over the limit. Everything past the limit was dropped as it was
    /// read, never held.
    TooLong,
}

/// Newline-delimited lines of at most `max` bytes each, not counting the line
/// ending. A sidecar cannot make the host buffer a line of any length: past
/// the limit, the rest of the line is consumed and dropped chunk by chunk.
///
/// The MCP server's `BoundedLines` (`crates/mcp/src/stdio.rs`), which this
/// crate cannot depend on, returning bytes rather than text.
pub struct BoundedLines<R> {
    reader: R,
    max: usize,
    buf: Vec<u8>,
    discarding: bool,
}

impl<R: AsyncBufRead + Unpin> BoundedLines<R> {
    pub fn new(reader: R, max: usize) -> Self {
        Self {
            reader,
            max,
            buf: Vec::new(),
            discarding: false,
        }
    }

    /// The next line, or `None` at end of input. Cancel-safe: the partial line
    /// lives in the struct, and bytes are consumed only once recorded.
    pub async fn next_line(&mut self) -> std::io::Result<Option<Line>> {
        loop {
            let available = self.reader.fill_buf().await?;
            if available.is_empty() {
                if std::mem::take(&mut self.discarding) {
                    return Ok(Some(Line::TooLong));
                }
                if self.buf.is_empty() {
                    return Ok(None);
                }
                return Ok(Some(self.finish()));
            }
            let (chunk, used, ends) = match available.iter().position(|&b| b == b'\n') {
                Some(i) => (&available[..i], i + 1, true),
                None => (available, available.len(), false),
            };
            if !self.discarding {
                // One byte of headroom for a `\r` ending the line.
                if self.buf.len() + chunk.len() > self.max + 1 {
                    self.discarding = true;
                    self.buf = Vec::new();
                } else {
                    self.buf.extend_from_slice(chunk);
                }
            }
            self.reader.consume(used);
            if ends {
                if std::mem::take(&mut self.discarding) {
                    return Ok(Some(Line::TooLong));
                }
                return Ok(Some(self.finish()));
            }
        }
    }

    fn finish(&mut self) -> Line {
        let mut raw = std::mem::take(&mut self.buf);
        if raw.last() == Some(&b'\r') {
            raw.pop();
        }
        if raw.len() > self.max {
            return Line::TooLong;
        }
        Line::Bytes(raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn parsed(line: &str) -> Result<Incoming, Violation> {
        parse(line.as_bytes())
    }

    #[test]
    fn a_response_carries_its_id_and_result() {
        assert_eq!(
            parsed(r#"{"jsonrpc":"2.0","id":7,"result":{"ok":true}}"#),
            Ok(Incoming::Response {
                id: 7,
                outcome: Ok(json!({"ok": true}))
            })
        );
    }

    #[test]
    fn an_error_response_carries_the_code_message_and_data() {
        let line = r#"{"jsonrpc":"2.0","id":3,"error":{"code":-32601,"message":"no such method","data":{"m":"x"}}}"#;
        let Ok(Incoming::Response {
            id: 3,
            outcome: Err(error),
        }) = parsed(line)
        else {
            panic!("{:?}", parsed(line));
        };
        assert_eq!(error.code, code::METHOD_NOT_FOUND);
        assert_eq!(error.message, "no such method");
        assert_eq!(error.data, Some(json!({"m": "x"})));
    }

    #[test]
    fn a_call_from_the_sidecar_keeps_its_own_id() {
        assert_eq!(
            parsed(r#"{"jsonrpc":"2.0","id":"a-1","method":"k8s.list","params":{"x":1}}"#),
            Ok(Incoming::Request {
                id: json!("a-1"),
                method: "k8s.list".into(),
                params: json!({"x": 1})
            })
        );
    }

    #[test]
    fn a_message_with_a_method_and_no_id_is_a_notification() {
        assert_eq!(
            parsed(r#"{"jsonrpc":"2.0","method":"stream/close","params":{"stream":1}}"#),
            Ok(Incoming::Notification {
                method: "stream/close".into(),
                params: json!({"stream": 1})
            })
        );
    }

    #[test]
    fn absent_params_are_null() {
        assert_eq!(
            parsed(r#"{"jsonrpc":"2.0","method":"x"}"#),
            Ok(Incoming::Notification {
                method: "x".into(),
                params: Value::Null
            })
        );
    }

    #[test]
    fn what_is_not_a_json_rpc_message_is_a_violation_and_says_what_it_was() {
        for (line, says) in [
            ("thread 'main' panicked at src/main.rs", "not JSON"),
            ("[1,2]", "batches are not supported"),
            (
                r#"{"jsonrpc":"1.0","id":1,"result":1}"#,
                "\"jsonrpc\": \"2.0\"",
            ),
            (r#"{"id":1,"result":1}"#, "\"jsonrpc\": \"2.0\""),
            (
                r#"{"jsonrpc":"2.0","id":1,"result":1,"error":{"code":1,"message":"m"}}"#,
                "exactly one",
            ),
            (r#"{"jsonrpc":"2.0","id":1}"#, "exactly one"),
            (r#"{"jsonrpc":"2.0","id":"1","result":1}"#, "never uses"),
            (r#"{"jsonrpc":"2.0","id":-1,"result":1}"#, "never uses"),
            (
                r#"{"jsonrpc":"2.0","id":null,"error":{"code":1,"message":"m"}}"#,
                "never uses",
            ),
            (
                r#"{"jsonrpc":"2.0","id":1,"error":"bad"}"#,
                "not a JSON-RPC error",
            ),
            (r#"{"jsonrpc":"2.0","method":7}"#, "not a string"),
            (
                r#"{"jsonrpc":"2.0","method":"m","params":"text"}"#,
                "not an object or an array",
            ),
            (
                r#"{"jsonrpc":"2.0","id":{},"method":"m"}"#,
                "not a string or number",
            ),
        ] {
            let Err(Violation(text)) = parsed(line) else {
                panic!("{line} parsed: {:?}", parsed(line));
            };
            assert!(text.contains(says), "{line}: {text}");
        }
        let Err(Violation(text)) = parse(b"\xff\xfe") else {
            panic!("bytes that are not UTF-8 parsed");
        };
        assert!(text.contains("not UTF-8"), "{text}");
    }

    #[test]
    fn every_outgoing_message_is_exactly_one_line() {
        let params = json!({"text": "two\nlines\r\n"});
        for line in [
            request(1, "m", &params),
            notification("m", &params),
            response(&json!("id"), &Ok(params.clone())),
            response(&json!(2), &Err(RpcError::new(1, "a\nb"))),
        ] {
            assert!(!line.contains('\n') && !line.contains('\r'), "{line}");
            let value: Value = serde_json::from_str(&line).expect("one JSON message");
            assert_eq!(value["jsonrpc"], "2.0", "{line}");
        }
    }

    #[test]
    fn a_request_the_host_writes_reads_back_as_the_sidecar_would_see_it() {
        let line = request(4, "health", &json!({}));
        let value: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["jsonrpc"], "2.0");
        assert_eq!(value["id"], 4);
        assert_eq!(value["method"], "health");
        assert_eq!(value["params"], json!({}));
    }

    #[test]
    fn initialize_offers_every_version_and_the_limits_in_camel_case() {
        let limits = Limits {
            request_timeout: Duration::from_secs(30),
            max_concurrent_requests: 8,
            max_streams: 5,
            memory_bytes: 256 * 1024 * 1024,
            cpus: 1.0,
            data_bytes: 1 << 30,
            data_entries: 100_000,
        };
        let data = std::path::Path::new("/srv/srelens/data/0123");
        let params = initialize_params(&["0.1.0", "0.2.0"], &limits, data);
        assert_eq!(params["apiVersions"], json!(["0.1.0", "0.2.0"]));
        assert_eq!(params["host"]["name"], "srelens");
        assert_eq!(
            params["limits"],
            json!({
                "requestTimeoutMs": 30000,
                "maxConcurrentRequests": 8,
                "maxStreams": 5,
                "memoryBytes": 268435456u64,
                "cpus": 1.0,
                "dataBytes": 1073741824u64,
                "dataEntries": 100000,
            })
        );
        // The one path it may write, which is also its working directory, so
        // an SDK need not rely on the latter.
        assert_eq!(params["dataDirectory"], "/srv/srelens/data/0123");
    }

    #[test]
    fn a_version_the_host_offered_is_the_negotiated_one() {
        let chosen = negotiated(&["0.1.0", "0.2.0"], &json!({"apiVersion": "0.1.0"}));
        assert_eq!(chosen, Ok(semver::Version::new(0, 1, 0)));
    }

    #[test]
    fn a_version_the_host_did_not_offer_is_refused_naming_both() {
        let why = negotiated(&["0.1.0"], &json!({"apiVersion": "9.0.0"})).unwrap_err();
        assert!(why.contains("9.0.0") && why.contains("0.1.0"), "{why}");
        assert!(why.contains("did not offer"), "{why}");
    }

    #[test]
    fn an_answer_without_a_usable_version_is_refused() {
        for result in [json!({}), json!({"apiVersion": 1}), json!(null)] {
            let why = negotiated(&["0.1.0"], &result).unwrap_err();
            assert!(why.contains("names no apiVersion"), "{result}: {why}");
        }
        let why = negotiated(&["0.1.0"], &json!({"apiVersion": "latest"})).unwrap_err();
        assert!(why.contains("not a version"), "{why}");
    }

    #[test]
    fn a_sidecar_that_speaks_none_of_the_offered_versions_says_which_it_does() {
        let mut error = RpcError::new(code::UNSUPPORTED_API_VERSION, "no common version");
        error.data = Some(json!({"supported": ["1.0.0", "1.1.0"]}));
        let why = incompatible(&["0.1.0"], &error).expect("an incompatibility");
        assert!(
            why.contains("1.0.0, 1.1.0") && why.contains("0.1.0"),
            "{why}"
        );
        error.data = None;
        let why = incompatible(&["0.1.0"], &error).expect("an incompatibility");
        assert!(why.contains("none of the versions"), "{why}");
        // Any other failure of initialize is not an incompatibility.
        let other = RpcError::new(code::INTERNAL_ERROR, "boom");
        assert_eq!(incompatible(&["0.1.0"], &other), None);
    }

    async fn lines(input: &[u8], max: usize) -> Vec<Line> {
        let mut reader = BoundedLines::new(input, max);
        let mut out = Vec::new();
        while let Some(line) = reader.next_line().await.expect("in-memory reads succeed") {
            out.push(line);
        }
        out
    }

    #[tokio::test]
    async fn lines_end_at_newlines_and_a_carriage_return_is_framing() {
        assert_eq!(
            lines(b"one\r\ntwo\nthree", 16).await,
            vec![
                Line::Bytes(b"one".to_vec()),
                Line::Bytes(b"two".to_vec()),
                Line::Bytes(b"three".to_vec())
            ]
        );
    }

    #[tokio::test]
    async fn a_line_over_the_limit_is_reported_and_the_next_one_still_reads() {
        assert_eq!(
            lines(b"0123456789\nok\n", 4).await,
            vec![Line::TooLong, Line::Bytes(b"ok".to_vec())]
        );
        assert_eq!(
            lines(b"abcd\n", 4).await,
            vec![Line::Bytes(b"abcd".to_vec())]
        );
        assert_eq!(lines(b"abcde", 4).await, vec![Line::TooLong]);
    }

    #[tokio::test]
    async fn bytes_that_are_not_utf8_are_returned_for_the_caller_to_judge() {
        assert_eq!(
            lines(b"\xff\xfe\n", 16).await,
            vec![Line::Bytes(vec![0xff, 0xfe])]
        );
    }
}
