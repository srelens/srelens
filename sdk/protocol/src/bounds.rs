//! The longest values a sidecar's call may carry. srelens refuses anything
//! longer; the schema states each bound too (in characters, where srelens
//! counts bytes: the two differ only past ASCII).

/// The longest `id` (as a string) and `method` a sidecar's call may carry.
/// Both are echoed in the answer, so a longer one is refused as a protocol
/// violation rather than copied.
pub const MAX_CALL_FIELD_BYTES: usize = 256;

/// The longest `context.clusterId` a call may carry: a pinned ID is a
/// kubeconfig's absolute path and a context name, encoded.
pub const MAX_CLUSTER_ID_BYTES: usize = 4096;

/// The longest binding (`capability`) or `action` name: ASCII letters, digits
/// and hyphens, as the manifest writes them.
pub const MAX_IDENTIFIER_LEN: usize = 64;

/// The longest object `name`: a Kubernetes object name, ASCII letters,
/// digits, dots and hyphens.
pub const MAX_OBJECT_NAME_LEN: usize = 253;

/// The longest `uid` or `resourceVersion`: visible ASCII characters, as the
/// object carries them.
pub const MAX_TOKEN_LEN: usize = 128;

/// The longest `context.namespace`: a Kubernetes namespace name, an RFC 1123
/// label.
pub const MAX_NAMESPACE_LEN: usize = 63;
