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

/// Sentinel capability id for `k8s://catalog`, which is assembled in-process
/// from the registry and prompt library rather than by invoking a capability.
pub const CATALOG_IN_PROCESS: &str = "<catalog>";

/// Every capability a resource read can invoke. Task 6's guard asserts each is
/// registered, `read_only`, and NOT consent-gated.
pub const MAPPED_CAPABILITIES: [&str; 4] =
    ["k8s.getManifest", "k8s.listEvents", "k8s.podLogs", "k8s.listContexts"];

/// A resolved read: which capability to invoke, with what arguments, and the
/// mimeType to report to the client.
#[derive(Debug, Clone, PartialEq)]
pub struct ResourceRead {
    pub capability: &'static str,
    pub args: serde_json::Value,
    pub mime: &'static str,
}

/// Resolve a URI to the capability call that serves it.
///
/// Every error here is a `-32602` message the caller must be able to act on, so
/// each names the offending value and, where there is one, the alternative.
pub fn plan_read(uri: &ResourceUri, kinds: &dyn KindResolver) -> Result<ResourceRead, String> {
    use serde_json::json;

    let (context, namespace, kind, name, sub) = match uri {
        ResourceUri::Contexts => {
            return Ok(ResourceRead {
                capability: "k8s.listContexts",
                args: json!({}),
                mime: "application/json",
            })
        }
        ResourceUri::Catalog => {
            return Ok(ResourceRead {
                capability: CATALOG_IN_PROCESS,
                args: json!({}),
                mime: "application/json",
            })
        }
        ResourceUri::Object { context, namespace, kind, name, sub } => {
            (context, namespace, kind, name, sub)
        }
    };

    let scope = kinds.scope(kind).ok_or_else(|| {
        if kind == "Secret" {
            "`Secret` is not addressable as a resource; read secrets with the \
             `k8s.getSecret` tool, which is consent-gated"
                .to_string()
        } else {
            format!("kind `{kind}` is not addressable as a resource")
        }
    })?;

    match (scope, namespace) {
        (KindScope::Namespaced, None) => {
            return Err(format!(
                "`{kind}` is namespaced; supply a namespace instead of `{CLUSTER_SCOPED}`"
            ))
        }
        (KindScope::ClusterScoped, Some(ns)) => {
            return Err(format!(
                "`{kind}` is cluster-scoped; use `{CLUSTER_SCOPED}` for the namespace, not `{ns}`"
            ))
        }
        _ => {}
    }

    match sub {
        None => {
            let mut args = json!({ "context": context, "kind": kind, "name": name });
            if let Some(ns) = namespace {
                args["namespace"] = json!(ns);
            }
            Ok(ResourceRead { capability: "k8s.getManifest", args, mime: "application/yaml" })
        }
        Some(SubResource::Events) => Ok(ResourceRead {
            capability: "k8s.listEvents",
            args: json!({
                "context": context,
                "namespace": namespace.clone().unwrap_or_default(),
                "objectKind": kind,
                "objectName": name,
            }),
            mime: "application/json",
        }),
        Some(SubResource::Logs) => {
            if kind != "Pod" {
                return Err(format!(
                    "logs exist only for Pods; `{kind}` has none"
                ));
            }
            Ok(ResourceRead {
                capability: "k8s.podLogs",
                args: json!({
                    "context": context,
                    "namespace": namespace.clone().unwrap_or_default(),
                    "pod": name,
                }),
                mime: "text/plain",
            })
        }
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

/// The only resources `resources/list` returns. Enumerating cluster objects
/// would be unbounded and would need a cluster round-trip to answer a
/// discovery call, so object addressing is advertised via templates instead.
pub fn fixed_resources() -> Vec<serde_json::Value> {
    use serde_json::json;
    vec![
        json!({
            "uri": "k8s://contexts",
            "name": "Kube contexts",
            "description": "Contexts srelens can connect to, and which is current.",
            "mimeType": "application/json"
        }),
        json!({
            "uri": "k8s://catalog",
            "name": "srelens catalog",
            "description": "Every tool, prompt and resource template this server exposes.",
            "mimeType": "application/json"
        }),
    ]
}

/// Parameterised URI shapes, which is what makes object addressing
/// discoverable without enumerating anything.
pub fn templates() -> Vec<serde_json::Value> {
    use serde_json::json;
    vec![
        json!({
            "uriTemplate": "k8s://{context}/{namespace}/{kind}/{name}",
            "name": "Object manifest",
            "description": "A resource's manifest as YAML. Use `-` as the namespace for \
                            cluster-scoped kinds. Secrets are not addressable — read them \
                            with the k8s.getSecret tool.",
            "mimeType": "application/yaml"
        }),
        json!({
            "uriTemplate": "k8s://{context}/{namespace}/{kind}/{name}/events",
            "name": "Object events",
            "description": "Events whose involved object is this resource.",
            "mimeType": "application/json"
        }),
        json!({
            "uriTemplate": "k8s://{context}/{namespace}/Pod/{name}/logs",
            "name": "Pod logs",
            "description": "Recent log output for a pod's default container.",
            "mimeType": "text/plain"
        }),
    ]
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

    use serde_json::json;

    #[test]
    fn a_manifest_read_maps_to_get_manifest_as_yaml() {
        let u = ResourceUri::parse("k8s://c/ns/Pod/web-0").unwrap();
        let r = plan_read(&u, &StubKinds).unwrap();
        assert_eq!(r.capability, "k8s.getManifest");
        assert_eq!(r.mime, "application/yaml");
        assert_eq!(r.args, json!({"context":"c","kind":"Pod","namespace":"ns","name":"web-0"}));
    }

    #[test]
    fn a_cluster_scoped_manifest_read_sends_no_namespace() {
        let u = ResourceUri::parse("k8s://c/-/Node/node-1").unwrap();
        let r = plan_read(&u, &StubKinds).unwrap();
        assert_eq!(r.args, json!({"context":"c","kind":"Node","name":"node-1"}));
    }

    #[test]
    fn an_events_read_filters_by_object() {
        let u = ResourceUri::parse("k8s://c/ns/Pod/web-0/events").unwrap();
        let r = plan_read(&u, &StubKinds).unwrap();
        assert_eq!(r.capability, "k8s.listEvents");
        assert_eq!(r.mime, "application/json");
        assert_eq!(
            r.args,
            json!({"context":"c","namespace":"ns","objectKind":"Pod","objectName":"web-0"})
        );
    }

    #[test]
    fn a_logs_read_maps_to_pod_logs_as_text() {
        let u = ResourceUri::parse("k8s://c/ns/Pod/web-0/logs").unwrap();
        let r = plan_read(&u, &StubKinds).unwrap();
        assert_eq!(r.capability, "k8s.podLogs");
        assert_eq!(r.mime, "text/plain");
        assert_eq!(r.args, json!({"context":"c","namespace":"ns","pod":"web-0"}));
    }

    #[test]
    fn the_fixed_resources_map_to_their_readers() {
        let c = plan_read(&ResourceUri::Contexts, &StubKinds).unwrap();
        assert_eq!(c.capability, "k8s.listContexts");
        let cat = plan_read(&ResourceUri::Catalog, &StubKinds).unwrap();
        assert_eq!(cat.capability, CATALOG_IN_PROCESS);
    }

    #[test]
    fn logs_are_rejected_for_a_non_pod_kind() {
        let u = ResourceUri::parse("k8s://c/ns/Deployment/api/logs").unwrap();
        let e = plan_read(&u, &StubKinds).unwrap_err();
        assert!(e.contains("Pod"), "got: {e}");
    }

    /// The curation guarantee, surfaced as an actionable error rather than a
    /// bare "unknown kind": the caller is told where secrets actually live.
    #[test]
    fn a_secret_uri_is_rejected_and_points_at_the_gated_tool() {
        let u = ResourceUri::parse("k8s://c/ns/Secret/db-creds").unwrap();
        let e = plan_read(&u, &StubKinds).unwrap_err();
        assert!(e.contains("k8s.getSecret"), "got: {e}");
    }

    #[test]
    fn an_unknown_kind_is_rejected() {
        let u = ResourceUri::parse("k8s://c/ns/Nonsense/x").unwrap();
        let e = plan_read(&u, &StubKinds).unwrap_err();
        assert!(e.contains("Nonsense"), "got: {e}");
    }

    #[test]
    fn a_scope_mismatch_is_rejected_in_both_directions() {
        let cluster_kind_with_ns = ResourceUri::parse("k8s://c/ns/Node/n").unwrap();
        let e = plan_read(&cluster_kind_with_ns, &StubKinds).unwrap_err();
        assert!(e.contains("cluster-scoped"), "got: {e}");

        let namespaced_without_ns = ResourceUri::parse("k8s://c/-/Pod/p").unwrap();
        let e = plan_read(&namespaced_without_ns, &StubKinds).unwrap_err();
        assert!(e.contains("namespaced"), "got: {e}");
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

    #[test]
    fn list_returns_only_the_two_fixed_resources() {
        let list = fixed_resources();
        assert_eq!(list.len(), 2, "cluster objects must never be enumerated: {list:?}");
        let uris: Vec<&str> = list.iter().map(|r| r["uri"].as_str().unwrap()).collect();
        assert!(uris.contains(&"k8s://contexts"));
        assert!(uris.contains(&"k8s://catalog"));
        for r in &list {
            assert!(r["name"].is_string());
            assert!(r["description"].is_string());
            assert!(r["mimeType"].is_string());
        }
    }

    #[test]
    fn templates_describe_the_three_parameterised_shapes() {
        let t = templates();
        assert_eq!(t.len(), 3);
        let patterns: Vec<&str> = t.iter().map(|x| x["uriTemplate"].as_str().unwrap()).collect();
        assert!(patterns.contains(&"k8s://{context}/{namespace}/{kind}/{name}"));
        assert!(patterns.contains(&"k8s://{context}/{namespace}/{kind}/{name}/events"));
        assert!(patterns.contains(&"k8s://{context}/{namespace}/Pod/{name}/logs"));
        for x in &t {
            assert!(x["name"].is_string());
            assert!(x["description"].is_string());
        }
    }
}
