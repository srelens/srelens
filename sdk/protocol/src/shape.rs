//! The shapes srelens holds a sidecar's call fields to
//! (`crates/plugin-host/src/sidecar/broker.rs`), shared so an SDK checks a
//! call before sending it. The committed schema states the same shapes, and
//! the broker's conformance test holds the two to each other.

use crate::bounds::{
    MAX_CLUSTER_ID_BYTES, MAX_IDENTIFIER_LEN, MAX_NAMESPACE_LEN, MAX_OBJECT_NAME_LEN, MAX_TOKEN_LEN,
};

/// A binding or action name, as the manifest holds one to (`identifier` in
/// `manifest.rs`).
pub fn is_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_IDENTIFIER_LEN
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// An object name, as `ResourceIn::validate` (`crates/kube/src/gitops.rs`)
/// holds one to.
pub fn is_object_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_OBJECT_NAME_LEN
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
}

/// A `uid` or `resourceVersion`: short, and nothing that is not a visible
/// character.
pub fn is_token(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_TOKEN_LEN && value.bytes().all(|b| b.is_ascii_graphic())
}

/// A Kubernetes namespace name: an RFC 1123 label. The rule `extensions.read`
/// holds a namespace to.
pub fn is_namespace(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAMESPACE_LEN
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !name.starts_with('-')
        && !name.ends_with('-')
}

/// A `clusterId`: names a cluster (not blank) in at most
/// [`MAX_CLUSTER_ID_BYTES`] bytes.
pub fn is_cluster_id(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= MAX_CLUSTER_ID_BYTES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shapes_are_the_brokers() {
        assert!(is_identifier("applications") && is_identifier(&"a".repeat(64)));
        assert!(!is_identifier("") && !is_identifier(&"a".repeat(65)) && !is_identifier("a_b"));
        assert!(is_object_name("web.v1-2") && is_object_name("..."));
        assert!(!is_object_name(".") && !is_object_name("..") && !is_object_name("web/1"));
        assert!(is_token("!") && is_token(&"~".repeat(128)));
        assert!(!is_token("u 1") && !is_token("u\u{7f}") && !is_token(&"x".repeat(129)));
        assert!(is_namespace("team-1") && is_namespace(&"a".repeat(63)));
        assert!(
            !is_namespace("Team")
                && !is_namespace("-a")
                && !is_namespace("a-")
                && !is_namespace("")
        );
        assert!(is_cluster_id("  prod") && is_cluster_id(&"x".repeat(4096)));
        assert!(!is_cluster_id("   ") && !is_cluster_id("") && !is_cluster_id(&"x".repeat(4097)));
    }
}
