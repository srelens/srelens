//! Every method of the protocol, once: who sends it and whether it is
//! answered. The schema's `x-srelens-methods` is generated from this table
//! and the one beside the schema generator that gives each method's params
//! and result types (behind the `schema` feature), and [`is_reserved`] reads
//! this one.

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
}

const fn request(name: &'static str, direction: Direction) -> MethodSpec {
    MethodSpec {
        name,
        direction,
        kind: Kind::Request,
    }
}

const fn notification(name: &'static str, direction: Direction) -> MethodSpec {
    MethodSpec {
        name,
        direction,
        kind: Kind::Notification,
    }
}

use Direction::{Both, HostToSidecar, SidecarToHost};

/// Every method, in the order `docs/extensions/sidecar-protocol.md` gives
/// them. An app operation is a request from srelens with any method name
/// that is not reserved, and any params; it has no entry.
pub static METHODS: &[MethodSpec] = &[
    request(method::INITIALIZE, HostToSidecar),
    request(method::ACTIVATE, HostToSidecar),
    request(method::HEALTH, HostToSidecar),
    request(method::DEACTIVATE, HostToSidecar),
    request(method::SHUTDOWN, HostToSidecar),
    request(method::STREAM_OPEN, HostToSidecar),
    notification(method::CANCEL, Both),
    notification(method::STREAM_CANCEL, HostToSidecar),
    notification(method::STREAM_DATA, SidecarToHost),
    notification(method::STREAM_CLOSE, SidecarToHost),
    notification(method::STREAM_ERROR, SidecarToHost),
    request(method::HOST_READ, SidecarToHost),
    request(method::HOST_RESOURCE, SidecarToHost),
    request(method::HOST_ACTION, SidecarToHost),
    request(method::HOST_BINDING_AVAILABILITY, SidecarToHost),
    request(method::HOST_RUN_JOB, SidecarToHost),
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
            method::HOST_BINDING_AVAILABILITY,
            method::HOST_RUN_JOB,
        ];
        for name in all {
            let entries = METHODS.iter().filter(|m| m.name == name).count();
            assert_eq!(entries, 1, "{name} is in the table {entries} times");
        }
        assert_eq!(METHODS.len(), all.len(), "a method the list above lacks");
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
