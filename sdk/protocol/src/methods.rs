//! Every method of the protocol, once: who sends it, whether it is answered,
//! and the types of its params and result. The schema's `x-srelens-methods`
//! is generated from this table, and [`is_reserved`] reads it.

use schemars::gen::SchemaGenerator;
use schemars::schema::Schema;
use schemars::JsonSchema;
use serde_json::Value;

use crate::messages::{
    CancelParams, Empty, HostActionParams, HostReadParams, HostResourceParams, InitializeParams,
    InitializeResult, StreamCancelParams, StreamCloseParams, StreamDataParams, StreamErrorParams,
    StreamOpenParams,
};
use crate::method;

/// Who sends a method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    HostToSidecar,
    SidecarToHost,
    /// `$/cancelRequest`: each side cancels its own requests.
    Both,
}

impl Direction {
    /// Its name in the schema's method table.
    pub fn wire_name(self) -> &'static str {
        match self {
            Direction::HostToSidecar => "hostToSidecar",
            Direction::SidecarToHost => "sidecarToHost",
            Direction::Both => "both",
        }
    }

    pub fn from_host(self) -> bool {
        matches!(self, Direction::HostToSidecar | Direction::Both)
    }

    pub fn from_sidecar(self) -> bool {
        matches!(self, Direction::SidecarToHost | Direction::Both)
    }
}

/// Whether a method is answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Request,
    Notification,
}

impl Kind {
    /// Its name in the schema's method table.
    pub fn wire_name(self) -> &'static str {
        match self {
            Kind::Request => "request",
            Kind::Notification => "notification",
        }
    }
}

/// One method of the protocol.
#[derive(Debug, Clone, Copy)]
pub struct MethodSpec {
    pub name: &'static str,
    pub direction: Direction,
    pub kind: Kind,
    /// Its params' schema, as a reference into the definitions.
    pub params: fn(&mut SchemaGenerator) -> Schema,
    /// A request's result schema. `None` for a notification.
    pub result: Option<fn(&mut SchemaGenerator) -> Schema>,
}

fn of<T: JsonSchema>(generator: &mut SchemaGenerator) -> Schema {
    generator.subschema_for::<T>()
}

const fn request(
    name: &'static str,
    direction: Direction,
    params: fn(&mut SchemaGenerator) -> Schema,
    result: fn(&mut SchemaGenerator) -> Schema,
) -> MethodSpec {
    MethodSpec {
        name,
        direction,
        kind: Kind::Request,
        params,
        result: Some(result),
    }
}

const fn notification(
    name: &'static str,
    direction: Direction,
    params: fn(&mut SchemaGenerator) -> Schema,
) -> MethodSpec {
    MethodSpec {
        name,
        direction,
        kind: Kind::Notification,
        params,
        result: None,
    }
}

use Direction::{Both, HostToSidecar, SidecarToHost};

/// Every method, in the order `docs/extensions/sidecar-protocol.md` gives
/// them. An app operation is a request from srelens with any method name
/// that is not reserved, and any params; it has no entry.
pub static METHODS: &[MethodSpec] = &[
    request(
        method::INITIALIZE,
        HostToSidecar,
        of::<InitializeParams>,
        of::<InitializeResult>,
    ),
    request(method::ACTIVATE, HostToSidecar, of::<Empty>, of::<Empty>),
    request(method::HEALTH, HostToSidecar, of::<Empty>, of::<Empty>),
    request(method::DEACTIVATE, HostToSidecar, of::<Empty>, of::<Empty>),
    request(method::SHUTDOWN, HostToSidecar, of::<Empty>, of::<Empty>),
    request(
        method::STREAM_OPEN,
        HostToSidecar,
        of::<StreamOpenParams>,
        of::<Value>,
    ),
    notification(method::CANCEL, Both, of::<CancelParams>),
    notification(
        method::STREAM_CANCEL,
        HostToSidecar,
        of::<StreamCancelParams>,
    ),
    notification(method::STREAM_DATA, SidecarToHost, of::<StreamDataParams>),
    notification(method::STREAM_CLOSE, SidecarToHost, of::<StreamCloseParams>),
    notification(method::STREAM_ERROR, SidecarToHost, of::<StreamErrorParams>),
    request(
        method::HOST_READ,
        SidecarToHost,
        of::<HostReadParams>,
        of::<Value>,
    ),
    request(
        method::HOST_RESOURCE,
        SidecarToHost,
        of::<HostResourceParams>,
        of::<Value>,
    ),
    request(
        method::HOST_ACTION,
        SidecarToHost,
        of::<HostActionParams>,
        of::<Value>,
    ),
];

/// Whether `name` is one of the host's own methods, which an app request may
/// not name: an app-facing path that forwarded `shutdown` would let a caller
/// stop the sidecar, and one that forwarded `stream/data` would forge frames.
/// That is every method srelens sends, and anything under `$/` or `stream/`,
/// the protocol's own namespaces.
pub fn is_reserved(name: &str) -> bool {
    name.starts_with("$/")
        || name.starts_with("stream/")
        || METHODS
            .iter()
            .any(|spec| spec.name == name && spec.direction.from_host())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::method;

    // the_host_methods_are_reserved: moved here verbatim from lib.rs.
    #[test]
    fn the_host_methods_are_reserved() {
        for name in [
            "initialize",
            "activate",
            "deactivate",
            "health",
            "shutdown",
            "$/cancelRequest",
            "$/anything",
            "stream/open",
            "stream/data",
        ] {
            assert!(is_reserved(name), "{name}");
        }
        for name in ["scan", "streams", "health-report", "k8s.list", "host/read"] {
            assert!(!is_reserved(name), "{name}");
        }
    }

    #[test]
    fn every_method_is_in_the_table_once() {
        let all = [
            method::INITIALIZE,
            method::ACTIVATE,
            method::DEACTIVATE,
            method::HEALTH,
            method::SHUTDOWN,
            method::CANCEL,
            method::STREAM_OPEN,
            method::STREAM_DATA,
            method::STREAM_CLOSE,
            method::STREAM_ERROR,
            method::STREAM_CANCEL,
            method::HOST_READ,
            method::HOST_RESOURCE,
            method::HOST_ACTION,
        ];
        for name in all {
            let entries = METHODS.iter().filter(|m| m.name == name).count();
            assert_eq!(entries, 1, "{name} is in the table {entries} times");
        }
        assert_eq!(METHODS.len(), all.len(), "a method the list above lacks");
    }

    #[test]
    fn a_request_has_a_result_and_a_notification_has_none() {
        for spec in METHODS {
            assert_eq!(
                spec.result.is_some(),
                spec.kind == Kind::Request,
                "{}",
                spec.name
            );
        }
    }

    #[test]
    fn the_table_says_who_sends_what() {
        let direction = |name: &str| METHODS.iter().find(|m| m.name == name).unwrap().direction;
        assert_eq!(direction(method::INITIALIZE), Direction::HostToSidecar);
        assert_eq!(direction(method::STREAM_CANCEL), Direction::HostToSidecar);
        assert_eq!(direction(method::STREAM_DATA), Direction::SidecarToHost);
        assert_eq!(direction(method::HOST_ACTION), Direction::SidecarToHost);
        assert_eq!(direction(method::CANCEL), Direction::Both);
        assert!(Direction::Both.from_host() && Direction::Both.from_sidecar());
        assert!(!Direction::SidecarToHost.from_host());
    }
}
