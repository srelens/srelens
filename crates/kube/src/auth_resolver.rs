//! Pluggable per-context auth for the ClientCache. On desktop there's no
//! resolver (kube-rs handles auth from the kubeconfig). In web mode the server
//! installs a resolver that supplies a managed OIDC Bearer, or signals that an
//! interactive cluster login is required.

/// Marker prefix the resolver returns (as an error) when a context needs an
/// interactive OIDC login. The HTTP layer detects it and returns 401.
pub const NEEDS_CLUSTER_LOGIN: &str = "NEEDS_CLUSTER_LOGIN";

pub fn needs_login_marker(oidc_key: &str, context: &str) -> String {
    format!("{NEEDS_CLUSTER_LOGIN}:{oidc_key}:{context}")
}

/// Parse `(oidc_key, context)` from a needs-login marker error, if it is one.
pub fn parse_needs_login(msg: &str) -> Option<(String, String)> {
    let rest = msg.strip_prefix(&format!("{NEEDS_CLUSTER_LOGIN}:"))?;
    let (key, context) = rest.split_once(':')?;
    Some((key.to_string(), context.to_string()))
}

pub enum AuthMode {
    /// Build the client the normal way (kube-rs resolves auth from kubeconfig).
    Default,
    /// Build the client with this Bearer token (OIDC cluster).
    Bearer(String),
}

#[async_trait::async_trait]
pub trait AuthResolver: Send + Sync {
    /// Decide how to authenticate `context`. Returning
    /// `Err(needs_login_marker(..))` signals an interactive login is required.
    async fn resolve(&self, context: &str) -> Result<AuthMode, String>;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn marker_round_trips() {
        let m = needs_login_marker("abc", "prod");
        assert_eq!(parse_needs_login(&m), Some(("abc".into(), "prod".into())));
        assert_eq!(parse_needs_login("connection refused"), None);
    }

    #[test]
    fn marker_round_trips_with_colon_in_context() {
        // A context name containing ':' puts the colon in the context half
        // (split_once takes the FIRST ':' in the remainder, after the key).
        let m = needs_login_marker("abc", "prod:us-east");
        assert_eq!(m, "NEEDS_CLUSTER_LOGIN:abc:prod:us-east");
        assert_eq!(
            parse_needs_login(&m),
            Some(("abc".into(), "prod:us-east".into()))
        );
    }
}
