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

/// Starts a watch for one object URI, calling `on_change` on every change.
///
/// A trait rather than a concrete type because the implementation needs kube,
/// and `crates/mcp` must not depend on `crates/kube` — `crates/registry` wires
/// the real one, exactly as it does for `KindResolver`.
pub trait ObjectWatcher: Send + Sync {
    fn watch(
        &self,
        uri: &ResourceUri,
        on_change: Box<dyn FnMut() + Send>,
    ) -> Result<tokio::task::AbortHandle, String>;
}

/// Fail-closed default: refuse a subscription rather than accept one that can
/// never fire.
pub struct NoWatcher;

impl ObjectWatcher for NoWatcher {
    fn watch(
        &self,
        _uri: &ResourceUri,
        _on_change: Box<dyn FnMut() + Send>,
    ) -> Result<tokio::task::AbortHandle, String> {
        Err("this server has no cluster watcher wired, so resources cannot be subscribed to"
            .to_string())
    }
}

/// Whether a URI can be subscribed to. The catalog is static, and logs are a
/// stream rather than a state change.
pub fn is_subscribable(uri: &ResourceUri) -> Result<(), String> {
    match uri {
        ResourceUri::Catalog => {
            Err("`k8s://catalog` is static and cannot be subscribed to".to_string())
        }
        ResourceUri::Contexts => Ok(()),
        ResourceUri::Object { sub: Some(SubResource::Logs), .. } => {
            Err("pod logs are a stream, not a state change; subscribe to the \
                 pod's manifest instead"
                .to_string())
        }
        ResourceUri::Object { .. } => Ok(()),
    }
}

/// Sentinel capability id for `k8s://catalog`, which is assembled in-process
/// from the registry and prompt library rather than by invoking a capability.
pub const CATALOG_IN_PROCESS: &str = "<catalog>";

/// Capability IDs named by plan_read. Private to keep the single source of truth
/// centralized — both MAPPED_CAPABILITIES and plan_read use these.
const CAP_MANIFEST: &str = "k8s.getManifest";
const CAP_EVENTS: &str = "k8s.listEvents";
const CAP_LOGS: &str = "k8s.podLogs";
const CAP_CONTEXTS: &str = "k8s.listContexts";

/// Every capability a resource read can invoke. Task 6's guard asserts each is
/// registered, `read_only`, and NOT consent-gated.
pub const MAPPED_CAPABILITIES: [&str; 4] = [CAP_MANIFEST, CAP_EVENTS, CAP_LOGS, CAP_CONTEXTS];

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
                capability: CAP_CONTEXTS,
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
            Ok(ResourceRead { capability: CAP_MANIFEST, args, mime: "application/yaml" })
        }
        Some(SubResource::Events) => Ok(ResourceRead {
            capability: CAP_EVENTS,
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
                capability: CAP_LOGS,
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

    /// Fail closed: a host that wires no watcher refuses every subscription
    /// rather than accepting one that can never fire, mirroring `NoKinds` and
    /// `AlwaysDeny`.
    #[test]
    fn the_default_watcher_refuses_every_subscription() {
        let uri = ResourceUri::parse("k8s://c/ns/Pod/web-0").unwrap();
        let err = NoWatcher.watch(&uri, Box::new(|| {})).unwrap_err();
        assert!(err.contains("no cluster watcher"), "got: {err}");
    }

    #[test]
    fn only_state_bearing_uris_are_subscribable() {
        assert!(is_subscribable(&ResourceUri::parse("k8s://c/ns/Pod/p").unwrap()).is_ok());
        assert!(is_subscribable(&ResourceUri::parse("k8s://c/ns/Pod/p/events").unwrap()).is_ok());
        assert!(is_subscribable(&ResourceUri::parse("k8s://c/ns/Pod/p/logs").unwrap()).is_err());
        assert!(is_subscribable(&ResourceUri::Catalog).is_err());
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

    /// Every id in MAPPED_CAPABILITIES must be registered, read-only, and NOT
    /// consent-gated. Clients auto-fetch resources to populate context, so a
    /// resource read that raised a confirm dialog would be a consent-fatigue
    /// vector — the hazard #23's design warns about. This test guards the
    /// listed ids. The companion test `every_plan_read_branch_is_mapped_and_nothing_extra`
    /// enforces that MAPPED_CAPABILITIES contains exactly the ids that plan_read
    /// can actually name, preventing drift as the code changes.
    #[test]
    fn every_mapped_capability_is_read_only_and_ungated() {
        let registry = std::sync::Arc::new(srelens_registry::build_registry());
        let server = crate::McpServer::new(registry.clone());
        for id in MAPPED_CAPABILITIES {
            let cap = registry
                .get(id)
                .unwrap_or_else(|| panic!("{id} is mapped for resource reads but not registered"));
            assert!(cap.annotations.read_only, "{id} must be read_only");
            assert_eq!(
                server.consent_kind(id),
                None,
                "{id} is consent-gated, so a resource read would raise a confirm dialog"
            );
        }
    }

    /// The curation that the guard above depends on. If this ever passes with
    /// `Some(_)`, secrets became addressable and the gate could be bypassed.
    #[test]
    fn the_real_resolver_still_excludes_secrets() {
        assert_eq!(srelens_registry::kind_resolver().scope("Secret"), None);
    }

    /// Every capability that plan_read can name must appear in MAPPED_CAPABILITIES,
    /// and nothing in MAPPED_CAPABILITIES must be unreachable. This test uses
    /// compiler-enforced exhaustive matching to ensure the property holds as
    /// code changes: adding a new ResourceUri or SubResource variant fails the
    /// build until this test is updated to cover it.
    #[test]
    fn every_plan_read_branch_is_mapped_and_nothing_extra() {
        // Compiler enforcement: if a new SubResource variant is added,
        // this match becomes non-exhaustive and the build fails.
        match SubResource::Events {
            SubResource::Events | SubResource::Logs => {}
        }
        let sub_variants = [None, Some(SubResource::Events), Some(SubResource::Logs)];

        // Resolver that allows all kinds we need to exercise every plan_read path:
        // Pod (namespaced, can have logs), and Node (cluster-scoped).
        struct AllowPodAndNode;
        impl KindResolver for AllowPodAndNode {
            fn scope(&self, kind: &str) -> Option<KindScope> {
                match kind {
                    "Pod" => Some(KindScope::Namespaced),
                    "Node" => Some(KindScope::ClusterScoped),
                    _ => None,
                }
            }
        }

        let mut capabilities_named = std::collections::HashSet::new();

        // Compiler enforcement: if a new ResourceUri variant is added,
        // this match becomes non-exhaustive and the build fails.
        match ResourceUri::Contexts {
            ResourceUri::Contexts | ResourceUri::Catalog | ResourceUri::Object { .. } => {}
        }

        // ResourceUri::Contexts
        let read = plan_read(&ResourceUri::Contexts, &AllowPodAndNode).unwrap();
        assert_ne!(read.capability, CATALOG_IN_PROCESS, "Contexts should map to a real capability");
        capabilities_named.insert(read.capability);

        // ResourceUri::Catalog (should NOT be in MAPPED_CAPABILITIES; it's a sentinel)
        let read = plan_read(&ResourceUri::Catalog, &AllowPodAndNode).unwrap();
        assert_eq!(read.capability, CATALOG_IN_PROCESS);
        // Don't add it to the set; MAPPED_CAPABILITIES explicitly excludes it.

        // ResourceUri::Object with all sub variants
        for sub in sub_variants {
            // Use Pod (namespaced) for most tests
            let kind = "Pod";
            let uri = ResourceUri::Object {
                context: "ctx".into(),
                namespace: Some("ns".into()),
                kind: kind.into(),
                name: "obj".into(),
                sub,
            };
            let read = plan_read(&uri, &AllowPodAndNode).unwrap();
            capabilities_named.insert(read.capability);

            // Also test cluster-scoped (Node) with the object endpoint (no sub)
            if sub.is_none() {
                let uri = ResourceUri::Object {
                    context: "ctx".into(),
                    namespace: None,
                    kind: "Node".into(),
                    name: "node1".into(),
                    sub,
                };
                let read = plan_read(&uri, &AllowPodAndNode).unwrap();
                capabilities_named.insert(read.capability);
            }
        }

        // Now verify that the set of capabilities named by plan_read
        // exactly equals MAPPED_CAPABILITIES.
        let mapped_set: std::collections::HashSet<&str> = MAPPED_CAPABILITIES.iter().copied().collect();
        assert_eq!(
            capabilities_named, mapped_set,
            "plan_read named capabilities differ from MAPPED_CAPABILITIES: \
             named={:?}, mapped={:?}",
            capabilities_named, mapped_set
        );
    }
}
