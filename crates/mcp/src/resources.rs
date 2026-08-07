//! `k8s://` resource addressing. A URI names a piece of cluster state; reads
//! map to existing capabilities so annotations and the audit log apply.

use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, CONTROLS};

/// Everything that is not unreserved per RFC 3986 gets encoded. Deliberately
/// aggressive: `/` and `:` MUST be encoded (EKS context ARNs contain both), and
/// encoding a few extra characters is harmless while missing one is not.
const SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ').add(b'"').add(b'#').add(b'<').add(b'>').add(b'?').add(b'`')
    .add(b'{').add(b'}').add(b'/').add(b':').add(b'%').add(b'[').add(b']')
    .add(b'@').add(b'!').add(b'$').add(b'&').add(b'\'').add(b'(').add(b')')
    .add(b'*').add(b'+').add(b',').add(b';').add(b'=');

fn encode(s: &str) -> String {
    utf8_percent_encode(s, SEGMENT).to_string()
}

fn decode(s: &str) -> Result<String, String> {
    percent_decode_str(s)
        .decode_utf8()
        .map(|c| c.to_string())
        .map_err(|e| format!("segment is not valid UTF-8 after decoding: {e}"))
}

/// A subresource hanging off an object URI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubResource {
    Events,
    Logs,
}

impl SubResource {
    fn as_str(self) -> &'static str {
        match self {
            SubResource::Events => "events",
            SubResource::Logs => "logs",
        }
    }
}

/// What a `k8s://` URI addresses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceUri {
    /// `k8s://contexts`
    Contexts,
    /// `k8s://catalog`
    Catalog,
    /// `k8s://<context>/<namespace|->/<kind>/<name>[/<sub>]`
    Object {
        context: String,
        /// `None` for a cluster-scoped kind — the `-` sentinel.
        namespace: Option<String>,
        kind: String,
        name: String,
        sub: Option<SubResource>,
    },
}

/// The sentinel standing in for "no namespace" on a cluster-scoped kind.
pub const CLUSTER_SCOPED: &str = "-";

impl ResourceUri {
    pub fn parse(uri: &str) -> Result<Self, String> {
        let rest = uri
            .strip_prefix("k8s://")
            .ok_or_else(|| format!("`{uri}` is not a k8s:// resource URI"))?;

        match rest {
            "contexts" => return Ok(ResourceUri::Contexts),
            "catalog" => return Ok(ResourceUri::Catalog),
            _ => {}
        }

        let parts: Vec<&str> = rest.split('/').collect();
        if parts.len() < 4 || parts.len() > 5 {
            return Err(format!(
                "`{uri}` has {} segments; expected \
                 k8s://<context>/<namespace|->/<kind>/<name>[/events|/logs], \
                 or the fixed k8s://contexts or k8s://catalog",
                parts.len()
            ));
        }
        if parts.iter().any(|p| p.is_empty()) {
            return Err(format!(
                "`{uri}` has an empty segment; use `-` for a cluster-scoped kind's namespace"
            ));
        }

        let context = decode(parts[0])?;
        let namespace_raw = decode(parts[1])?;
        let kind = decode(parts[2])?;
        let name = decode(parts[3])?;
        let sub = match parts.get(4) {
            None => None,
            Some(&"events") => Some(SubResource::Events),
            Some(&"logs") => Some(SubResource::Logs),
            Some(other) => {
                return Err(format!(
                    "unknown subresource `{other}`; only `events` and `logs` exist"
                ))
            }
        };

        Ok(ResourceUri::Object {
            context,
            namespace: if namespace_raw == CLUSTER_SCOPED { None } else { Some(namespace_raw) },
            kind,
            name,
            sub,
        })
    }
}

/// Whether a kind is namespaced. Returned only for kinds that are addressable
/// as resources at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KindScope {
    Namespaced,
    ClusterScoped,
}

/// Answers "is this kind addressable as a resource, and is it namespaced".
///
/// Injected because `crates/mcp` cannot depend on `crates/kube`, where the kind
/// table lives. `crates/registry` supplies the real implementation over
/// `gvk_for`; a second table here would drift.
pub trait KindResolver: Send + Sync {
    fn scope(&self, kind: &str) -> Option<KindScope>;
}

/// The default. A host that wires no resolver exposes no object resources —
/// only the two fixed ones. Fail closed, as `AlwaysDeny` is for consent.
pub struct NoKinds;

impl KindResolver for NoKinds {
    fn scope(&self, _kind: &str) -> Option<KindScope> {
        None
    }
}

impl std::fmt::Display for ResourceUri {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResourceUri::Contexts => write!(f, "k8s://contexts"),
            ResourceUri::Catalog => write!(f, "k8s://catalog"),
            ResourceUri::Object { context, namespace, kind, name, sub } => {
                write!(
                    f,
                    "k8s://{}/{}/{}/{}",
                    encode(context),
                    namespace.as_deref().map(encode).unwrap_or_else(|| CLUSTER_SCOPED.to_string()),
                    encode(kind),
                    encode(name)
                )?;
                if let Some(s) = sub {
                    write!(f, "/{}", s.as_str())?;
                }
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StubKinds;
    impl KindResolver for StubKinds {
        fn scope(&self, kind: &str) -> Option<KindScope> {
            match kind {
                "Pod" | "Deployment" => Some(KindScope::Namespaced),
                "Node" => Some(KindScope::ClusterScoped),
                _ => None,
            }
        }
    }

    #[test]
    fn a_stub_resolver_answers_scope_and_addressability() {
        let r = StubKinds;
        assert_eq!(r.scope("Pod"), Some(KindScope::Namespaced));
        assert_eq!(r.scope("Node"), Some(KindScope::ClusterScoped));
        assert_eq!(r.scope("Secret"), None, "not addressable");
        assert_eq!(r.scope("Nonsense"), None);
    }

    /// Fail closed: a host that wires no resolver addresses nothing, mirroring
    /// how `AlwaysDeny` is the default `ConfirmPolicy`.
    #[test]
    fn the_default_resolver_addresses_nothing() {
        assert_eq!(NoKinds.scope("Pod"), None);
        assert_eq!(NoKinds.scope("Node"), None);
    }

    #[test]
    fn parses_a_namespaced_object() {
        let u = ResourceUri::parse("k8s://kind-demo/prod/Pod/web-0").unwrap();
        assert_eq!(
            u,
            ResourceUri::Object {
                context: "kind-demo".into(),
                namespace: Some("prod".into()),
                kind: "Pod".into(),
                name: "web-0".into(),
                sub: None,
            }
        );
    }

    /// `-` is the cluster-scoped sentinel. An empty segment (`//`) is ambiguous
    /// and invisible in a bug report, so it is rejected rather than accepted.
    #[test]
    fn parses_a_cluster_scoped_object_via_the_dash_sentinel() {
        let u = ResourceUri::parse("k8s://kind-demo/-/Node/node-1").unwrap();
        match u {
            ResourceUri::Object { namespace, kind, .. } => {
                assert_eq!(namespace, None);
                assert_eq!(kind, "Node");
            }
            other => panic!("expected an object, got {other:?}"),
        }
    }

    #[test]
    fn parses_the_events_and_logs_subresources() {
        let e = ResourceUri::parse("k8s://c/ns/Pod/p/events").unwrap();
        let l = ResourceUri::parse("k8s://c/ns/Pod/p/logs").unwrap();
        match (e, l) {
            (
                ResourceUri::Object { sub: Some(SubResource::Events), .. },
                ResourceUri::Object { sub: Some(SubResource::Logs), .. },
            ) => {}
            other => panic!("expected events + logs, got {other:?}"),
        }
    }

    #[test]
    fn parses_the_two_fixed_resources() {
        assert_eq!(ResourceUri::parse("k8s://contexts").unwrap(), ResourceUri::Contexts);
        assert_eq!(ResourceUri::parse("k8s://catalog").unwrap(), ResourceUri::Catalog);
    }

    /// The load-bearing case: an EKS context name is an ARN containing `/` and
    /// `:`. Unencoded, the `/` silently reshapes the path into a different
    /// resource, so encoding is not optional.
    #[test]
    fn round_trips_a_context_name_containing_a_slash_and_colon() {
        let original = ResourceUri::Object {
            context: "arn:aws:eks:eu-west-1:123456789012:cluster/prod".into(),
            namespace: Some("ns".into()),
            kind: "Pod".into(),
            name: "web-0".into(),
            sub: None,
        };
        let rendered = original.to_string();
        assert!(!rendered.contains("cluster/prod"), "the slash must be encoded: {rendered}");
        assert_eq!(ResourceUri::parse(&rendered).unwrap(), original);
    }

    #[test]
    fn round_trips_every_shape() {
        for uri in [
            "k8s://contexts",
            "k8s://catalog",
            "k8s://c/ns/Pod/p",
            "k8s://c/-/Node/n",
            "k8s://c/ns/Pod/p/events",
            "k8s://c/ns/Pod/p/logs",
        ] {
            let parsed = ResourceUri::parse(uri).unwrap();
            assert_eq!(parsed.to_string(), uri, "round trip failed for {uri}");
        }
    }

    #[test]
    fn rejects_malformed_uris() {
        for (uri, expect) in [
            ("http://c/ns/Pod/p", "k8s://"),
            ("k8s://c/ns/Pod", "segments"),
            ("k8s://c/ns/Pod/p/x/y", "segments"),
            ("k8s://c/ns/Pod/p/status", "events"),
            ("k8s://c//Pod/p", "empty"),
            ("k8s://nonsense", "k8s://"),
        ] {
            let e = ResourceUri::parse(uri).unwrap_err();
            assert!(e.contains(expect), "for {uri}, message {e:?} should mention {expect:?}");
        }
    }
}
