//! Resource relationship links (#545): what an Inspector's "Related" section
//! shows, shaped as edges a topology can draw later (#524).
use super::columns::{join_objects, match_joined, JoinCache};
use super::panels::check_resource_scope;
use super::*;
use srelens_plugin_host::{namespace_name, Join, JoinMatch, LinkRelation, ResourceLink};

/// A resource named by a link's match: the target's namespace (`None` when
/// the reference does not say, as a bare Argo CD application name does not),
/// its name, and the uid an owner reference also carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Reference {
    namespace: Option<String>,
    name: String,
    uid: Option<String>,
    /// Nothing says the target's namespace (a bare Argo CD application name
    /// with no declared `defaultNamespace`), so the host does not look for
    /// it: it is named, unverified, never searched for across namespaces.
    unplaced: bool,
}

/// One endpoint of an edge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct ResourceRef {
    /// Qualified, `group/Kind`.
    kind: String,
    /// `None` for a cluster-scoped resource, or a target whose namespace is unknown.
    namespace: Option<String>,
    name: String,
}

/// A target a link names, and whether the host found it in the granted list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct LinkTarget {
    namespace: Option<String>,
    name: String,
    /// False: the resource names a target the cluster does not have — unless
    /// `unverified` says the host did not look.
    exists: bool,
    /// Why the host did not look this target up, e.g. its namespace is unknown.
    #[serde(skip_serializing_if = "Option::is_none")]
    unverified: Option<String>,
}

/// One declared link resolved for one resource.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct ResolvedLink {
    id: String,
    relation: LinkRelation,
    /// The target kind, qualified.
    to: String,
    /// The reader binding that lists `to`, which the UI opens a target through.
    capability: String,
    targets: Vec<LinkTarget>,
    /// Why the host could not answer. Never an empty `targets` in disguise.
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ResolvedLinks {
    from: ResourceRef,
    links: Vec<ResolvedLink>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ResolveLinks {
    id: String,
    revision: u64,
    context: String,
    namespace: String,
    kind: String,
    resource: Value,
}

/// Most owner references a link reads on one resource. Real resources carry
/// one or two; the bound keeps a caller-sent list from costing a lookup each.
pub(super) const MAX_OWNER_REFERENCES: usize = 64;

/// The targets `link` names on `resource`, read from its own metadata.
///
/// No key present is no reference, an answer. A key that cannot be read
/// truthfully — a Secret's annotation, which the host redacts — is an error.
pub(super) fn references(link: &ResourceLink, resource: &Value) -> Result<Vec<Reference>, String> {
    let rule = &link.match_by;
    let metadata = &resource["metadata"];
    // A cluster-scoped resource has no namespace. That is `None`, not a
    // namespace called "": a target it names is found by name.
    let own_namespace = metadata["namespace"]
        .as_str()
        .filter(|namespace| !namespace.is_empty())
        .map(str::to_owned);
    let here = |name: &str| Reference {
        namespace: own_namespace.clone(),
        name: name.to_owned(),
        uid: None,
        unplaced: false,
    };
    if let Some(label) = &rule.label {
        let Some(name) = metadata["labels"][label].as_str().filter(|n| !n.is_empty()) else {
            return Ok(vec![]);
        };
        let namespace = match &rule.namespace_label {
            None => own_namespace.clone(),
            Some(key) => match metadata["labels"][key].as_str().filter(|n| !n.is_empty()) {
                Some(namespace) => Some(namespace.to_owned()),
                // The name without its namespace names no one resource.
                None => {
                    return Err(format!(
                        "label {key} is not set; the target's namespace is unknown"
                    ))
                }
            },
        };
        return Ok(vec![Reference {
            namespace,
            name: name.to_owned(),
            uid: None,
            unplaced: false,
        }]);
    }
    if let Some(annotation) = &rule.annotation {
        if link.from == "/Secret" {
            return Err("a Secret's annotation values are redacted on every read".into());
        }
        let Some(value) = metadata["annotations"][annotation]
            .as_str()
            .filter(|v| !v.is_empty())
        else {
            return Ok(vec![]);
        };
        return Ok(match rule.parse {
            None => vec![here(value)],
            // `<namespace>_<name>` says where the application is; a bare
            // name is in Argo CD's own namespace, which only the manifest
            // can say. Without it the name is unplaced, not searched for.
            Some(format) => format
                .owner(value, resource)
                .map(|(namespace, name)| {
                    let namespace = namespace.or_else(|| rule.default_namespace.clone());
                    Reference {
                        unplaced: namespace.is_none(),
                        namespace,
                        name,
                        uid: None,
                    }
                })
                .into_iter()
                .collect(),
        });
    }
    if rule.owner_reference {
        let (group, kind) = link.to.split_once('/').unwrap_or(("", &link.to));
        // The resource is the caller's: bound the list before a lookup each.
        let owners = match metadata["ownerReferences"].as_array() {
            Some(owners) if owners.len() > MAX_OWNER_REFERENCES => {
                return Err(format!(
                    "the resource lists {} ownerReferences, more than the {MAX_OWNER_REFERENCES} a link reads",
                    owners.len()
                ));
            }
            Some(owners) => owners.as_slice(),
            None => &[],
        };
        return Ok(owners
            .iter()
            .filter(|owner| {
                let owner_group = owner["apiVersion"]
                    .as_str()
                    .map(|v| v.rsplit_once('/').map_or("", |(g, _)| g))
                    .unwrap_or("");
                owner["kind"].as_str() == Some(kind) && owner_group == group
            })
            .filter_map(|owner| {
                Some(Reference {
                    uid: owner["uid"].as_str().map(str::to_owned),
                    ..here(owner["name"].as_str()?)
                })
            })
            .collect());
    }
    if rule.name {
        return Ok(metadata["name"].as_str().map(here).into_iter().collect());
    }
    Err("the link declares no match".into())
}

/// Finds each reference in `objects`, the granted list of `kind`.
///
/// A namespaced reference goes through the join index (`match_joined` with a
/// name rule), so a link and a joined column agree on what "the resource named
/// X" is, ambiguity included. A reference with no namespace is found by name
/// across the list, and only when exactly one resource has that name.
pub(super) fn lookup(
    references: &[Reference],
    objects: &[Value],
    kind: &str,
    namespaced: bool,
) -> Result<Vec<LinkTarget>, String> {
    let by_name = JoinMatch {
        label: None,
        kind_label: None,
        owner_reference: false,
        annotation: None,
        name: true,
    };
    let mut targets: Vec<LinkTarget> = Vec::new();
    for reference in references {
        if reference.unplaced {
            let resolved = LinkTarget {
                namespace: None,
                name: reference.name.clone(),
                exists: false,
                unverified: Some(
                    "namespace unknown: the app declares no defaultNamespace for a bare name"
                        .into(),
                ),
            };
            if !targets.contains(&resolved) {
                targets.push(resolved);
            }
            continue;
        }
        let wanted = if namespaced {
            reference.namespace.clone()
        } else {
            Some(String::new())
        };
        let found = match &wanted {
            Some(namespace) => {
                match_joined(&by_name, objects, None, &reference.name, namespace, kind)
                    .map_err(|()| format!("more than one {kind} is named {}", reference.name))?
            }
            None => {
                let mut named = objects
                    .iter()
                    .filter(|object| object["metadata"]["name"].as_str() == Some(&reference.name));
                let first = named.next();
                if named.next().is_some() {
                    return Err(format!(
                        "more than one {kind} is named {} and the reference does not say which namespace",
                        reference.name
                    ));
                }
                first
            }
        };
        let found =
            found.filter(
                |object| match (&reference.uid, object["metadata"]["uid"].as_str()) {
                    (Some(want), Some(have)) => want == have,
                    _ => true,
                },
            );
        let namespace = match found {
            Some(object) => object["metadata"]["namespace"].as_str().map(str::to_owned),
            None => wanted,
        }
        .filter(|namespace| !namespace.is_empty());
        let resolved = LinkTarget {
            namespace,
            name: reference.name.clone(),
            exists: found.is_some(),
            unverified: None,
        };
        if !targets.contains(&resolved) {
            targets.push(resolved);
        }
    }
    Ok(targets)
}

/// One link for one resource: its references, then (only when there are any)
/// the granted list of `to`, read once through the join snapshot cache.
#[allow(clippy::too_many_arguments)]
async fn resolve_link(
    cache: &JoinCache,
    client_cache: &srelens_kube::client_cache::ClientCache,
    core: &Registry,
    plugin: &Installed,
    link: &ResourceLink,
    context: &str,
    resource: &Value,
) -> Result<(String, Vec<LinkTarget>), (String, String)> {
    let binding = plugin
        .manifest
        .capabilities
        .iter()
        .find(|binding| Manifest::reader_kind(binding).as_deref() == Some(link.to.as_str()))
        .ok_or_else(|| {
            (
                String::new(),
                "No declared reader lists the link's target".to_owned(),
            )
        })?;
    let capability = binding.name.clone();
    let failed = |why: String| (capability.clone(), why);
    let references = references(link, resource).map_err(failed)?;
    if references.is_empty() {
        return Ok((capability, vec![]));
    }
    // Only unplaced names: nothing to look for, so no list is read.
    if needs_no_list(&references) {
        let targets = lookup(&references, &[], &link.to, true).map_err(failed)?;
        return Ok((capability, targets));
    }
    let namespaced = binding
        .arguments
        .get("namespaced")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    // One namespace's list when every reference names the same one, so a
    // namespace-scoped grant is enough; otherwise the whole cluster's.
    let scope = list_scope(&references, namespaced).map_err(failed)?;
    let join = Join {
        id: link.id.clone(),
        capability: capability.clone(),
        match_by: JoinMatch {
            label: None,
            kind_label: None,
            owner_reference: false,
            annotation: None,
            name: true,
        },
    };
    // Listed at the version the target's reader resolves to on this cluster (#547). A
    // link reads only names from it, so no path depends on which version that is.
    let (objects, _version) =
        join_objects(cache, client_cache, core, plugin, &join, context, &scope)
            .await
            .map_err(|error| failed(error.to_string()))?;
    let targets = lookup(&references, &objects, &link.to, namespaced).map_err(failed)?;
    Ok((capability, targets))
}

pub(super) fn register(
    reg: &mut Registry,
    path: PathBuf,
    core: Arc<Registry>,
    client_cache: Arc<srelens_kube::client_cache::ClientCache>,
    cache: JoinCache,
) {
    reg.register(Capability::typed::<ResolveLinks, ResolvedLinks, _, _>(
        "extensions.resolveLinks",
        "Resolve an app's resource relationship links for a resource Inspector",
        Annotations::READ_ONLY,
        move |input| {
            let path = path.clone();
            let core = core.clone();
            let client_cache = client_cache.clone();
            let cache = cache.clone();
            async move {
                check_resource_scope(
                    "Link",
                    &input.id,
                    &input.context,
                    &input.namespace,
                    &input.kind,
                    &input.resource,
                )?;
                let (state, index, context) = resolver_app(
                    path,
                    &core,
                    &client_cache,
                    &input.id,
                    input.revision,
                    input.context,
                )
                .await?;
                let plugin = &state.plugins[index];
                let mut links = Vec::new();
                for link in plugin
                    .manifest
                    .contributions
                    .resource_links
                    .iter()
                    .filter(|link| link.from == input.kind)
                {
                    let resolved = resolve_link(
                        &cache,
                        &client_cache,
                        &core,
                        plugin,
                        link,
                        &context,
                        &input.resource,
                    )
                    .await;
                    let (capability, targets, error) = match resolved {
                        Ok((capability, targets)) => (capability, targets, None),
                        Err((capability, why)) => (capability, vec![], Some(why)),
                    };
                    links.push(ResolvedLink {
                        id: link.id.clone(),
                        relation: link.relation,
                        to: link.to.clone(),
                        capability,
                        targets,
                        error,
                    });
                }
                Ok(ResolvedLinks {
                    from: ResourceRef {
                        kind: input.kind,
                        namespace: Some(input.namespace).filter(|n| !n.is_empty()),
                        name: input.resource["metadata"]["name"]
                            .as_str()
                            .unwrap_or("")
                            .to_owned(),
                    },
                    links,
                })
            }
        },
    ));
}

/// The namespace to list `to` in: the one every reference names, or `""`
/// (the whole cluster) when they name several or one names none.
///
/// A reference's namespace can come from the caller's resource (a
/// `namespaceLabel`), which the host never validated, so every one is held
/// to a namespace name before anything is listed — and one that is not a
/// namespace name is an error on the link, never widened to a cluster list.
pub(super) fn list_scope(references: &[Reference], namespaced: bool) -> Result<String, String> {
    for namespace in references.iter().filter_map(|r| r.namespace.as_deref()) {
        if !namespace_name(namespace) {
            return Err(format!(
                "the resource names the target namespace {:?}, which is not a namespace name",
                namespace.chars().take(64).collect::<String>()
            ));
        }
    }
    if !namespaced {
        return Ok(String::new());
    }
    // An unplaced name is not looked for, so it widens no list.
    let mut named = references
        .iter()
        .filter(|r| !r.unplaced)
        .map(|r| r.namespace.as_deref());
    let first = named.next().flatten();
    Ok(match first {
        Some(namespace) if named.all(|other| other == Some(namespace)) => namespace.to_owned(),
        _ => String::new(),
    })
}

/// Whether every reference is one the host does not look for, so no list is read.
pub(super) fn needs_no_list(references: &[Reference]) -> bool {
    references.iter().all(|r| r.unplaced)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(value: Value) -> ResourceLink {
        serde_json::from_value(value).expect("a link deserializes")
    }

    fn argo_link() -> ResourceLink {
        link(
            json!({"id":"argocd-owner","from":"apps/Deployment","to":"argoproj.io/Application",
            "relation":"managedBy",
            "match":{"annotation":"argocd.argoproj.io/tracking-id","parse":"argocd-tracking-id"}}),
        )
    }

    fn deployment(labels: Value, annotations: Value, owners: Value) -> Value {
        json!({"apiVersion":"apps/v1","kind":"Deployment","metadata":{"name":"api","namespace":"team",
            "uid":"uid-api","labels":labels,"annotations":annotations,"ownerReferences":owners}})
    }

    fn reference(namespace: Option<&str>, name: &str) -> Reference {
        Reference {
            namespace: namespace.map(str::to_owned),
            name: name.into(),
            uid: None,
            unplaced: false,
        }
    }

    /// A name whose namespace nothing says, and which is not looked for.
    fn unplaced(name: &str) -> Reference {
        Reference {
            unplaced: true,
            ..reference(None, name)
        }
    }

    #[test]
    fn a_bare_argo_cd_name_is_looked_up_only_in_the_declared_namespace() {
        // Two Applications called guestbook: the one in Argo CD's namespace is
        // the owner; one in `team` is someone else's and no ambiguity.
        let objects = vec![
            app("team", "guestbook", "u-team"),
            app("argocd", "guestbook", "u-argo"),
        ];
        let kind = "argoproj.io/Application";
        let placed = [reference(Some("argocd"), "guestbook")];
        assert_eq!(list_scope(&placed, true), Ok("argocd".to_owned()));
        assert_eq!(
            lookup(&placed, &objects, kind, true),
            Ok(vec![target(Some("argocd"), "guestbook", true)])
        );
        // Without a default: never `exists`, never searched, and says why.
        let bare = [unplaced("guestbook")];
        let targets = lookup(&bare, &objects, kind, true).unwrap();
        assert_eq!(targets.len(), 1);
        assert!(!targets[0].exists);
        assert_eq!(targets[0].namespace, None);
        assert!(targets[0]
            .unverified
            .as_deref()
            .is_some_and(|why| why.contains("namespace")));
        // An unplaced name widens no list: beside a placed one it is ignored.
        assert_eq!(
            list_scope(&[unplaced("guestbook"), placed[0].clone()], true),
            Ok("argocd".to_owned())
        );
        assert!(needs_no_list(&bare));
        assert!(!needs_no_list(&placed));
    }

    #[test]
    fn a_tracking_id_names_its_application_only_when_it_names_the_resource() {
        let tracked = |id: &str| {
            deployment(
                json!({}),
                json!({"argocd.argoproj.io/tracking-id": id}),
                json!([]),
            )
        };
        let link = argo_link();
        // A bare name is an application in Argo CD's own namespace, which
        // the id does not say: without a declared one it stays unplaced.
        assert_eq!(
            references(&link, &tracked("guestbook:apps/Deployment:team/api")),
            Ok(vec![unplaced("guestbook")])
        );
        assert_eq!(
            references(&link, &tracked("apps_guestbook:apps/Deployment:team/api")),
            Ok(vec![reference(Some("apps"), "guestbook")])
        );
        // With a declared default, a bare name is there; a prefixed name
        // keeps its own namespace.
        let mut defaulted = argo_link();
        defaulted.match_by.default_namespace = Some("argocd".into());
        assert_eq!(
            references(&defaulted, &tracked("guestbook:apps/Deployment:team/api")),
            Ok(vec![reference(Some("argocd"), "guestbook")])
        );
        assert_eq!(
            references(
                &defaulted,
                &tracked("apps_guestbook:apps/Deployment:team/api")
            ),
            Ok(vec![reference(Some("apps"), "guestbook")])
        );
        // Copied from another workload: this one is not managed by it.
        assert_eq!(
            references(&link, &tracked("guestbook:apps/Deployment:team/web")),
            Ok(vec![])
        );
        // No annotation: an answer (nothing), not an error.
        assert_eq!(
            references(&link, &deployment(json!({}), json!({}), json!([]))),
            Ok(vec![])
        );
    }

    #[test]
    fn labels_names_and_owner_references_name_their_targets() {
        let flux = link(json!({"id":"flux","from":"apps/Deployment",
            "to":"kustomize.toolkit.fluxcd.io/Kustomization","relation":"managedBy",
            "match":{"label":"kustomize.toolkit.fluxcd.io/name",
                "namespaceLabel":"kustomize.toolkit.fluxcd.io/namespace"}}));
        let labelled = deployment(
            json!({"kustomize.toolkit.fluxcd.io/name":"apps",
                "kustomize.toolkit.fluxcd.io/namespace":"flux-system"}),
            json!({}),
            json!([]),
        );
        assert_eq!(
            references(&flux, &labelled),
            Ok(vec![reference(Some("flux-system"), "apps")])
        );
        // Without a namespace label the target is in the resource's namespace.
        let mut same = flux.clone();
        same.match_by.namespace_label = None;
        assert_eq!(
            references(&same, &labelled),
            Ok(vec![reference(Some("team"), "apps")])
        );
        let by_name = link(
            json!({"id":"same","from":"apps/Deployment","to":"acme.io/Deployment",
            "relation":"references","match":{"name":true}}),
        );
        assert_eq!(
            references(&by_name, &labelled),
            Ok(vec![reference(Some("team"), "api")])
        );
        let owner = link(
            json!({"id":"owner","from":"apps/Deployment","to":"acme.io/Stack",
            "relation":"ownedBy","match":{"ownerReference":true}}),
        );
        let owned = deployment(
            json!({}),
            json!({}),
            json!([
                {"apiVersion":"acme.io/v1","kind":"Stack","name":"web","uid":"uid-web"},
                // Same kind name in another group is not this link's target.
                {"apiVersion":"other.io/v1","kind":"Stack","name":"x","uid":"uid-x"},
                {"apiVersion":"apps/v1","kind":"ReplicaSet","name":"rs","uid":"uid-rs"}
            ]),
        );
        assert_eq!(
            references(&owner, &owned),
            Ok(vec![Reference {
                namespace: Some("team".into()),
                name: "web".into(),
                uid: Some("uid-web".into()),
                unplaced: false,
            }])
        );
    }

    fn app(namespace: &str, name: &str, uid: &str) -> Value {
        json!({"metadata":{"name":name,"namespace":namespace,"uid":uid}})
    }

    fn target(namespace: Option<&str>, name: &str, exists: bool) -> LinkTarget {
        LinkTarget {
            namespace: namespace.map(str::to_owned),
            name: name.into(),
            exists,
            unverified: None,
        }
    }

    #[test]
    fn a_reference_is_looked_up_in_the_granted_list_through_the_join_index() {
        let objects = vec![
            app("argocd", "guestbook", "u1"),
            app("apps", "guestbook", "u2"),
            app("argocd", "only", "u3"),
        ];
        let kind = "argoproj.io/Application";
        // A namespaced reference finds exactly that one.
        assert_eq!(
            lookup(
                &[reference(Some("apps"), "guestbook")],
                &objects,
                kind,
                true
            ),
            Ok(vec![target(Some("apps"), "guestbook", true)])
        );
        // A bare name found once, in any namespace, is that one.
        assert_eq!(
            lookup(&[reference(None, "only")], &objects, kind, true),
            Ok(vec![target(Some("argocd"), "only", true)])
        );
        // Found twice: the host does not guess which.
        assert!(
            lookup(&[reference(None, "guestbook")], &objects, kind, true)
                .unwrap_err()
                .contains("more than one")
        );
        // Named but not there: said, not dropped.
        assert_eq!(
            lookup(&[reference(Some("team"), "gone")], &objects, kind, true),
            Ok(vec![target(Some("team"), "gone", false)])
        );
        // An owner reference whose uid is another object's is a gone owner.
        let stale = Reference {
            uid: Some("other".into()),
            ..reference(Some("argocd"), "only")
        };
        assert_eq!(
            lookup(&[stale], &objects, kind, true),
            Ok(vec![target(Some("argocd"), "only", false)])
        );
        // Cluster-scoped targets have no namespace.
        let cluster = vec![json!({"metadata":{"name":"prod"}})];
        assert_eq!(
            lookup(
                &[reference(Some("team"), "prod")],
                &cluster,
                "acme.io/Env",
                false
            ),
            Ok(vec![target(None, "prod", true)])
        );
        // Two references to one target are one link.
        assert_eq!(
            lookup(
                &[
                    reference(Some("apps"), "guestbook"),
                    reference(Some("apps"), "guestbook")
                ],
                &objects,
                kind,
                true
            )
            .unwrap()
            .len(),
            1
        );
    }

    /// Installs the test app with an Argo CD link, and a registry holding the
    /// link resolver, on a host that cannot reach any cluster.
    async fn installed() -> (tempfile::TempDir, Registry, u64) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("apps.json");
        let core = super::super::tests::fake_core();
        let mut source: Value = serde_json::from_str(&super::super::tests::manifest()).unwrap();
        source["contributions"]["resourceLinks"] =
            json!([serde_json::to_value(argo_link()).unwrap()]);
        let revision = mutate(
            &path,
            core.clone(),
            Configure::Install {
                signature: None,
                manifest: source.to_string(),
                grants: vec!["k8s.listCustomResource".into()],
                reviewed_revision: None,
            },
        )
        .unwrap()
        .plugins[0]
            .revision;
        let mut reg = Registry::new();
        super::super::register(
            &mut reg,
            path,
            core,
            srelens_kube::client_cache::ClientCache::new_many(vec![]),
        );
        (dir, reg, revision)
    }

    /// The payload `@srelens/core`'s `resolveExtensionLinks` sends.
    fn payload(revision: u64, annotations: Value) -> Value {
        json!({"id":"org.example.argocd","revision":revision,"context":"cluster/a",
            "namespace":"team","kind":"apps/Deployment",
            "resource":{"apiVersion":"apps/v1","kind":"Deployment",
                "metadata":{"name":"api","namespace":"team","annotations":annotations}}})
    }

    #[tokio::test]
    async fn an_unreadable_target_list_is_an_error_on_the_link_not_no_links() {
        let (_dir, reg, revision) = installed().await;
        let out = reg
            .invoke(
                "extensions.resolveLinks",
                payload(
                    revision,
                    json!({"argocd.argoproj.io/tracking-id":"argocd_guestbook:apps/Deployment:team/api"}),
                ),
            )
            .await
            .unwrap();
        assert_eq!(
            out["from"],
            json!({"kind":"apps/Deployment","namespace":"team","name":"api"})
        );
        let link = &out["links"][0];
        assert_eq!(link["id"], "argocd-owner");
        assert_eq!(link["relation"], "managedBy");
        assert_eq!(link["to"], "argoproj.io/Application");
        assert_eq!(link["capability"], "applications");
        assert_eq!(link["targets"], json!([]));
        assert!(link["error"].is_string(), "{out}");
    }

    #[tokio::test]
    async fn a_bare_argo_cd_name_without_a_default_namespace_is_named_not_read() {
        // The installed link declares no defaultNamespace, and this host
        // reaches no cluster: an answer here proves nothing was listed.
        let (_dir, reg, revision) = installed().await;
        let out = reg
            .invoke(
                "extensions.resolveLinks",
                payload(
                    revision,
                    json!({"argocd.argoproj.io/tracking-id":"guestbook:apps/Deployment:team/api"}),
                ),
            )
            .await
            .unwrap();
        let link = &out["links"][0];
        assert!(link.get("error").is_none(), "{out}");
        assert_eq!(link["targets"][0]["name"], "guestbook");
        assert_eq!(link["targets"][0]["exists"], false);
        assert!(link["targets"][0]["namespace"].is_null());
        assert!(link["targets"][0]["unverified"]
            .as_str()
            .is_some_and(|why| why.contains("namespace unknown")));
    }

    #[tokio::test]
    async fn a_resource_naming_no_target_answers_without_a_read() {
        let (_dir, reg, revision) = installed().await;
        let out = reg
            .invoke("extensions.resolveLinks", payload(revision, json!({})))
            .await
            .unwrap();
        // The host could answer: nothing is related. No error, no read.
        assert_eq!(out["links"][0]["targets"], json!([]));
        assert!(out["links"][0].get("error").is_none(), "{out}");
        // A kind no link starts from has no links at all.
        let mut other = payload(revision, json!({}));
        other["kind"] = json!("apps/StatefulSet");
        other["resource"]["kind"] = json!("StatefulSet");
        let out = reg.invoke("extensions.resolveLinks", other).await.unwrap();
        assert_eq!(out["links"], json!([]));
        // A stale revision is refused, like every resolver.
        let mut stale = payload(revision + 1, json!({}));
        stale["revision"] = json!(revision + 1);
        assert!(reg.invoke("extensions.resolveLinks", stale).await.is_err());
        // A field the wrapper does not send is refused, not ignored.
        let mut wrong = payload(revision, json!({}));
        wrong["resourceKind"] = json!("apps/Deployment");
        assert!(reg.invoke("extensions.resolveLinks", wrong).await.is_err());
    }

    #[test]
    fn the_target_list_is_scoped_to_the_one_namespace_the_references_name() {
        // Flux: a Deployment in `team` names a Kustomization in
        // `flux-system`. Listing that namespace, not the cluster, is what a
        // namespace-scoped grant allows and what keeps under the list cap.
        assert_eq!(
            list_scope(&[reference(Some("flux-system"), "apps")], true),
            Ok("flux-system".to_owned())
        );
        assert_eq!(
            list_scope(
                &[reference(Some("team"), "a"), reference(Some("team"), "b")],
                true
            ),
            Ok("team".to_owned())
        );
        // Two namespaces, or one reference with none, need the whole cluster.
        assert_eq!(
            list_scope(
                &[reference(Some("team"), "a"), reference(Some("apps"), "b")],
                true
            ),
            Ok(String::new())
        );
        assert_eq!(
            list_scope(&[reference(None, "guestbook")], true),
            Ok(String::new())
        );
        // A cluster-scoped target has no namespace to list in.
        assert_eq!(
            list_scope(&[reference(Some("team"), "prod")], false),
            Ok(String::new())
        );
    }

    #[test]
    fn a_namespace_read_from_the_resources_labels_is_checked_before_any_list() {
        // The namespace comes from the caller's resource, not from a scope the
        // host validated, so a malformed one is an error on the link.
        for bad in [
            "Flux-System",
            "-flux",
            "flux-",
            "flux_system",
            "a/b",
            "",
            &"x".repeat(64),
        ] {
            let error = list_scope(&[reference(Some(bad), "apps")], true).expect_err(bad);
            assert!(error.contains("namespace"), "{bad}: {error}");
        }
        // Even beside a good one: it is never folded into a cluster-wide list.
        assert!(list_scope(
            &[reference(Some("team"), "a"), reference(Some("Bad"), "b")],
            true
        )
        .is_err());
    }

    #[test]
    fn a_cluster_scoped_resource_names_targets_without_a_namespace() {
        // A Namespace owned by a cluster-scoped Stack: the source has no
        // namespace, which is not a namespace called "".
        let owner = link(
            json!({"id":"owner","from":"/Namespace","to":"acme.io/Stack",
            "relation":"ownedBy","match":{"ownerReference":true}}),
        );
        let namespace = json!({"apiVersion":"v1","kind":"Namespace","metadata":{"name":"team",
            "ownerReferences":[{"apiVersion":"acme.io/v1","kind":"Stack","name":"platform","uid":"u-1"}]}});
        let refs = references(&owner, &namespace).unwrap();
        assert_eq!(
            refs,
            vec![Reference {
                namespace: None,
                name: "platform".into(),
                uid: Some("u-1".into()),
                unplaced: false,
            }]
        );
        assert_eq!(list_scope(&refs, false), Ok(String::new()));
        let stacks = vec![json!({"metadata":{"name":"platform","uid":"u-1"}})];
        assert_eq!(
            lookup(&refs, &stacks, "acme.io/Stack", false),
            Ok(vec![target(None, "platform", true)])
        );
        // A namespaced target from a cluster-scoped source by label: no
        // namespace to look in, so it is found by name across the list, and
        // a name two namespaces share is reported rather than guessed.
        let labelled = link(
            json!({"id":"app","from":"/Namespace","to":"argoproj.io/Application",
            "relation":"managedBy","match":{"label":"example.io/app"}}),
        );
        let tagged = json!({"apiVersion":"v1","kind":"Namespace",
            "metadata":{"name":"team","labels":{"example.io/app":"guestbook"}}});
        let refs = references(&labelled, &tagged).unwrap();
        assert_eq!(refs, vec![reference(None, "guestbook")]);
        assert_eq!(list_scope(&refs, true), Ok(String::new()));
        let one = vec![app("argocd", "guestbook", "u1")];
        assert_eq!(
            lookup(&refs, &one, "argoproj.io/Application", true),
            Ok(vec![target(Some("argocd"), "guestbook", true)])
        );
        let two = vec![
            app("argocd", "guestbook", "u1"),
            app("apps", "guestbook", "u2"),
        ];
        assert!(lookup(&refs, &two, "argoproj.io/Application", true)
            .unwrap_err()
            .contains("more than one"));
        // By name, from a cluster-scoped kind to another cluster-scoped one.
        let same = link(json!({"id":"same","from":"/Namespace","to":"acme.io/Env",
            "relation":"references","match":{"name":true}}));
        assert_eq!(
            references(&same, &tagged),
            Ok(vec![reference(None, "team")])
        );
    }

    #[test]
    fn an_owner_reference_list_past_the_bound_is_an_error_not_a_slow_read() {
        // The caller sends the resource; a compact list of thousands of owners
        // fits in its 1 MiB and would cost a lookup each.
        let owner = link(
            json!({"id":"owner","from":"apps/Deployment","to":"acme.io/Stack",
            "relation":"ownedBy","match":{"ownerReference":true}}),
        );
        let owners = |count: usize| {
            deployment(json!({}), json!({}), json!((0..count).map(|i| json!(
                {"apiVersion":"acme.io/v1","kind":"Stack","name":format!("s-{i}"),"uid":format!("u-{i}")}
            )).collect::<Vec<_>>()))
        };
        assert_eq!(
            references(&owner, &owners(MAX_OWNER_REFERENCES)).map(|r| r.len()),
            Ok(MAX_OWNER_REFERENCES)
        );
        let error = references(&owner, &owners(MAX_OWNER_REFERENCES + 1)).unwrap_err();
        assert!(error.contains("ownerReferences"), "{error}");
        assert!(error.contains(&MAX_OWNER_REFERENCES.to_string()), "{error}");
    }

    #[test]
    fn a_secrets_redacted_annotation_is_an_error_not_an_absent_link() {
        let secret = link(json!({"id":"owner","from":"/Secret","to":"acme.io/Stack",
            "relation":"ownedBy","match":{"annotation":"acme.io/stack"}}));
        let resource = json!({"apiVersion":"v1","kind":"Secret",
            "metadata":{"name":"s","namespace":"team","annotations":{"acme.io/stack":"<redacted>"}}});
        assert!(references(&secret, &resource)
            .unwrap_err()
            .contains("redacted"));
    }
}
