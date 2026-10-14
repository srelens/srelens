//! One JSON-RPC 2.0 message, as either side writes it. The host keeps its own
//! stricter reader (`crates/plugin-host/src/sidecar/protocol.rs`), whose
//! sentences name a sidecar's violations; these are for SDKs and tests.

use serde::de::{self, Deserializer};
use serde::ser::{SerializeMap, Serializer};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{RequestId, RpcError};

/// The `"jsonrpc": "2.0"` every message carries.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct JsonRpc;

impl Serialize for JsonRpc {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str("2.0")
    }
}

impl<'de> Deserialize<'de> for JsonRpc {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let version = String::deserialize(deserializer)?;
        if version == "2.0" {
            Ok(JsonRpc)
        } else {
            Err(de::Error::custom(format!(
                "\"jsonrpc\" must be \"2.0\", not {version:?}"
            )))
        }
    }
}

/// A request: it has an id and is answered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: JsonRpc,
    pub id: RequestId,
    pub method: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub params: Value,
}

impl Request {
    pub fn new(id: RequestId, method: impl Into<String>, params: Value) -> Request {
        Request {
            jsonrpc: JsonRpc,
            id,
            method: method.into(),
            params,
        }
    }
}

/// A notification: no id, never answered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notification {
    pub jsonrpc: JsonRpc,
    pub method: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub params: Value,
}

impl Notification {
    pub fn new(method: impl Into<String>, params: Value) -> Notification {
        Notification {
            jsonrpc: JsonRpc,
            method: method.into(),
            params,
        }
    }
}

/// The answer to request `id`: its result, or why it failed.
#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    pub id: RequestId,
    pub outcome: Result<Value, RpcError>,
}

impl Response {
    pub fn ok(id: RequestId, result: Value) -> Response {
        Response {
            id,
            outcome: Ok(result),
        }
    }

    pub fn err(id: RequestId, error: RpcError) -> Response {
        Response {
            id,
            outcome: Err(error),
        }
    }
}

impl Serialize for Response {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(3))?;
        map.serialize_entry("jsonrpc", &JsonRpc)?;
        map.serialize_entry("id", &self.id)?;
        match &self.outcome {
            Ok(result) => map.serialize_entry("result", result)?,
            Err(error) => map.serialize_entry("error", error)?,
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for Response {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            #[allow(dead_code)]
            jsonrpc: JsonRpc,
            id: RequestId,
            // Present-but-null is a result; only absent is `None`.
            #[serde(default, deserialize_with = "present")]
            result: Option<Value>,
            #[serde(default)]
            error: Option<RpcError>,
        }
        let raw = Raw::deserialize(deserializer)?;
        let outcome = match (raw.result, raw.error) {
            (Some(result), None) => Ok(result),
            (None, Some(error)) => Err(error),
            _ => {
                return Err(de::Error::custom(
                    "a response has exactly one of \"result\" and \"error\"",
                ))
            }
        };
        Ok(Response {
            id: raw.id,
            outcome,
        })
    }
}

fn present<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

/// One line either side writes.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    Request(Request),
    Notification(Notification),
    Response(Response),
}

impl Message {
    /// Read one line: a request (method and id), a notification (method, no
    /// id) or a response (no method), or why it is none of them.
    pub fn parse(line: &str) -> Result<Message, String> {
        let value: Value = serde_json::from_str(line).map_err(|e| format!("not JSON: {e}"))?;
        if !value.is_object() {
            return Err("not a JSON-RPC message object".to_owned());
        }
        let parsed = match (value.get("method").is_some(), value.get("id").is_some()) {
            (true, true) => serde_json::from_value(value).map(Message::Request),
            (true, false) => serde_json::from_value(value).map(Message::Notification),
            (false, _) => serde_json::from_value(value).map(Message::Response),
        };
        parsed.map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{code, RpcError};
    use serde_json::json;

    #[test]
    fn a_request_and_a_notification_carry_jsonrpc_2_0_and_their_params() {
        let request = Request::new(RequestId::from(3), "health", json!({}));
        assert_eq!(
            serde_json::to_value(&request).unwrap(),
            json!({"jsonrpc": "2.0", "id": 3, "method": "health", "params": {}})
        );
        let note = Notification::new("stream/close", json!({"stream": 1}));
        assert_eq!(
            serde_json::to_value(&note).unwrap(),
            json!({"jsonrpc": "2.0", "method": "stream/close", "params": {"stream": 1}})
        );
        // Absent params are not written.
        let bare = Notification::new("telemetry", Value::Null);
        assert_eq!(
            serde_json::to_value(&bare).unwrap(),
            json!({"jsonrpc": "2.0", "method": "telemetry"})
        );
    }

    #[test]
    fn a_response_has_exactly_one_of_result_and_error() {
        let ok = Response::ok(RequestId::String("c-1".into()), json!({"rows": []}));
        assert_eq!(
            serde_json::to_value(&ok).unwrap(),
            json!({"jsonrpc": "2.0", "id": "c-1", "result": {"rows": []}})
        );
        let err = Response::err(
            RequestId::from(2),
            RpcError::new(code::METHOD_NOT_FOUND, "no"),
        );
        assert_eq!(
            serde_json::to_value(&err).unwrap(),
            json!({"jsonrpc": "2.0", "id": 2, "error": {"code": -32601, "message": "no"}})
        );
        for line in [
            r#"{"jsonrpc":"2.0","id":1,"result":1,"error":{"code":1,"message":"m"}}"#,
            r#"{"jsonrpc":"2.0","id":1}"#,
        ] {
            let why = serde_json::from_str::<Response>(line)
                .unwrap_err()
                .to_string();
            assert!(why.contains("exactly one"), "{line}: {why}");
        }
    }

    #[test]
    fn a_null_result_is_a_result() {
        let parsed: Response =
            serde_json::from_str(r#"{"jsonrpc":"2.0","id":4,"result":null}"#).unwrap();
        assert_eq!(parsed.outcome, Ok(Value::Null));
    }

    #[test]
    fn any_jsonrpc_but_2_0_is_refused() {
        for line in [
            r#"{"jsonrpc":"1.0","id":1,"method":"health","params":{}}"#,
            r#"{"id":1,"method":"health","params":{}}"#,
        ] {
            assert!(Message::parse(line).is_err(), "{line}");
        }
    }

    #[test]
    fn a_line_parses_as_the_kind_of_message_it_is() {
        let request =
            Message::parse(r#"{"jsonrpc":"2.0","id":7,"method":"greet","params":{"name":"x"}}"#)
                .unwrap();
        assert!(
            matches!(request, Message::Request(ref r) if r.method == "greet" && r.id == RequestId::from(7))
        );
        let note =
            Message::parse(r#"{"jsonrpc":"2.0","method":"$/cancelRequest","params":{"id":7}}"#)
                .unwrap();
        assert!(matches!(note, Message::Notification(ref n) if n.method == "$/cancelRequest"));
        let answer = Message::parse(r#"{"jsonrpc":"2.0","id":"c-1","result":{}}"#).unwrap();
        assert!(
            matches!(answer, Message::Response(ref r) if r.id == RequestId::String("c-1".into()))
        );
        let absent = Message::parse(r#"{"jsonrpc":"2.0","method":"telemetry"}"#).unwrap();
        assert!(matches!(absent, Message::Notification(ref n) if n.params == Value::Null));
        assert!(Message::parse("not json").is_err());
        assert!(Message::parse("[1,2]").is_err());
    }
}
