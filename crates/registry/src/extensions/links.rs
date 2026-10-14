//! Resource relationship links (#545): what an Inspector's "Related" section
//! shows, shaped as edges a topology can draw later (#524).
//!
//! A link may point at a built-in kind the host lists, and may name its target
//! through a path on the resource it starts from (#728). The target's own
//! Inspector reads every declaration the other way round: which resources name
//! it, through the same lists and the same snapshot cache.
use super::columns::{
    cached_objects, match_joined, read_at, reader_key, reader_listing, JoinCache,
};
use super::panels::check_resource_scope;
use super::*;
use srelens_plugin_host::{
    builtin_link_kind, namespace_name, Binding, BuiltinKind, JoinMatch, LinkRelation, ResourceLink,
};

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
    /// Empty for a built-in kind (#728), which opens in the host's own Inspector.
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

/// One declared link read the other way round, for the Inspector of a resource of
/// its `to` kind (#728): the resources of kind `from` whose link names this one.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReverseLink {
    id: String,
    relation: LinkRelation,
    /// The kind the link is read from, qualified.
    from: String,
    /// The reader binding that lists `from`, which the UI opens a source through.
    /// Empty for a built-in kind, which opens in the host's own Inspector.
    capability: String,
    /// The resources whose link names this one. `unverified` says why one may not:
    /// its reference leaves the namespace unsaid.
    sources: Vec<LinkTarget>,
    /// The read of `from` stopped at its 2,000-object limit: `sources` are what
    /// the host found among the objects it read, and there may be more.
    truncated: bool,
    /// How many resources of `from` the host read but could not read this link
    /// on, and why the first could not. They are not in `sources`, and not
    /// known to be unrelated either.
    #[serde(skip_serializing_if = "Option::is_none")]
    unreadable: Option<String>,
    /// Why the host could not answer. Never an empty `sources` in disguise.
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ReverseLinks {
    to: ResourceRef,
    links: Vec<ReverseLink>,
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

/// The targets a path link names on `object` (#728), the resource the host read
/// through the declared reader of the link's `from`.
///
/// Each value the path reaches is the target's name, or an object reference with
/// a `name` and, optionally, a `namespace`, a `kind`, and a `group` or an
/// `apiVersion`. A reference whose kind or group is not `to`'s names another
/// kind of resource and is not this link's: an HTTPRoute backend may be a
/// ServiceImport, a Flux source an OCIRepository. Without a namespace the
/// target is in the resource's own. An unset field, an empty name and a null
/// are no reference; a value that is none of these is an error, since the host
/// cannot say what it names.
pub(super) fn path_references(
    link: &ResourceLink,
    object: &Value,
) -> Result<Vec<Reference>, String> {
    let Some(path) = link.match_by.path.as_deref() else {
        return Err("the link declares no path".into());
    };
    let own_namespace = object["metadata"]["namespace"]
        .as_str()
        .filter(|namespace| !namespace.is_empty())
        .map(str::to_owned);
    let (group, kind) = link.to.split_once('/').unwrap_or(("", &link.to));
    let mut named = Vec::new();
    for value in srelens_capability::resolve_each(object, path)? {
        let reference = match value {
            Value::String(name) if name.is_empty() => continue,
            Value::String(name) => Reference {
                namespace: own_namespace.clone(),
                name: name.clone(),
                uid: None,
                unplaced: false,
            },
            Value::Object(fields) => {
                let text = |key: &str| fields.get(key).and_then(Value::as_str);
                if fields.get("kind").is_some_and(|k| k.as_str() != Some(kind)) {
                    continue;
                }
                let referenced_group = match (fields.get("group"), text("apiVersion")) {
                    (Some(group), _) => Some(group.as_str().unwrap_or("\u{0}")),
                    (None, Some(api_version)) => {
                        Some(api_version.rsplit_once('/').map_or("", |(group, _)| group))
                    }
                    (None, None) => None,
                };
                if referenced_group.is_some_and(|referenced| referenced != group) {
                    continue;
                }
                let Some(name) = text("name").filter(|name| !name.is_empty()) else {
                    return Err(format!("`{path}` holds a reference with no name"));
                };
                Reference {
                    namespace: text("namespace")
                        .filter(|namespace| !namespace.is_empty())
                        .map(str::to_owned)
                        .or_else(|| own_namespace.clone()),
                    name: name.to_owned(),
                    uid: text("uid").map(str::to_owned),
                    unplaced: false,
                }
            }
            Value::Array(_) => {
                return Err(format!(
                    "`{path}` holds a list, not a name or a reference; write `[*]` to read each element"
                ))
            }
            _ => {
                return Err(format!(
                    "`{path}` holds neither a name nor an object reference"
                ))
            }
        };
        named.push(reference);
    }
    Ok(named)
}

/// How the host reads a kind for a link: through the app's declared reader of
/// it, or as a built-in kind whose metadata the host lists itself (#728).
#[derive(Clone, Copy)]
enum Listed<'a> {
    Reader(&'a Binding),
    Builtin(&'static BuiltinKind),
}

impl<'a> Listed<'a> {
    /// A declared reader first: a kind one lists is read through the app's grant.
    fn of(manifest: &'a Manifest, kind: &str) -> Option<Self> {
        manifest
            .capabilities
            .iter()
            .find(|binding| Manifest::reader_kind(binding).as_deref() == Some(kind))
            .map(Listed::Reader)
            .or_else(|| builtin_link_kind(kind).map(Listed::Builtin))
    }

    /// The reader the UI opens a resource of this kind through; empty for a
    /// built-in kind, which opens in the host's own Inspector.
    fn capability(self) -> String {
        match self {
            Listed::Reader(binding) => binding.name.clone(),
            Listed::Builtin(_) => String::new(),
        }
    }

    fn namespaced(self) -> bool {
        match self {
            Listed::Reader(binding) => binding
                .arguments
                .get("namespaced")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            Listed::Builtin(kind) => kind.namespaced,
        }
    }
}

/// Every object of `kind` in `namespace` (`""` is all), whether the read stopped
/// at the 2,000-object cap, and the version a reader resolved to on this cluster
/// (#547; none for a built-in kind). Through the snapshot cache joins, panels and
/// cards share, so an Inspector and a table over one reader list it once.
#[allow(clippy::too_many_arguments)]
async fn read_listed(
    cache: &JoinCache,
    client_cache: &srelens_kube::client_cache::ClientCache,
    core: &Registry,
    plugin: &Installed,
    listed: Listed<'_>,
    kind: &str,
    context: &str,
    namespace: &str,
) -> Result<(Arc<Vec<Value>>, bool, Option<String>), CapabilityError> {
    match listed {
        Listed::Reader(binding) => {
            let (objects, truncated, version) = reader_listing(
                cache,
                client_cache,
                core,
                plugin,
                &binding.name,
                context,
                namespace,
            )
            .await?;
            Ok((objects, truncated, Some(version)))
        }
        Listed::Builtin(_) => {
            // Metadata alone, and of a Secret its identity, labels and owners:
            // `list_builtin_link_metadata` never asks the API server for a value.
            let key = reader_key(
                &plugin.manifest.id,
                plugin.revision,
                context,
                namespace,
                &format!("builtin-link:{kind}"),
                "",
            );
            let (objects, truncated) = cached_objects(cache, key, || {
                srelens_kube::crds::list_builtin_link_metadata(
                    client_cache,
                    context,
                    namespace,
                    kind,
                )
            })
            .await?;
            Ok((objects, truncated, None))
        }
    }
}

/// The `from` resource as the host reads it through the app's reader of its
/// kind, and the link as the manifest reads that reader's objects at the version
/// it resolved to (#547). A path link reads the resource's body, which the
/// Inspector does not send and the host does not take from the caller.
#[allow(clippy::too_many_arguments)]
async fn read_from(
    cache: &JoinCache,
    client_cache: &srelens_kube::client_cache::ClientCache,
    core: &Registry,
    plugin: &Installed,
    link: &ResourceLink,
    context: &str,
    resource: &Value,
) -> Result<(Value, ResourceLink), String> {
    let Some(listed @ Listed::Reader(reader)) = Listed::of(&plugin.manifest, &link.from) else {
        return Err(format!(
            "No declared reader lists {}, so its path cannot be read",
            link.from
        ));
    };
    let metadata = &resource["metadata"];
    let name = metadata["name"].as_str().unwrap_or("");
    let namespace = metadata["namespace"].as_str().unwrap_or("");
    let scope = if listed.namespaced() { namespace } else { "" };
    // A list cut off at the cap still holds the first 2,000: the resource may be among them.
    let (objects, truncated, version) = reader_listing(
        cache,
        client_cache,
        core,
        plugin,
        &reader.name,
        context,
        scope,
    )
    .await
    .map_err(|error| error.to_string())?;
    let link = link_at_version(plugin, &reader.name, &version, &link.id)?;
    let uid = metadata["uid"].as_str();
    let object = objects
        .iter()
        .find(|object| {
            let found = &object["metadata"];
            found["name"].as_str() == Some(name)
                && found["namespace"].as_str().unwrap_or("") == namespace
                && match (uid, found["uid"].as_str()) {
                    (Some(want), Some(have)) => want == have,
                    _ => true,
                }
        })
        .cloned()
        .ok_or_else(|| {
            let at = if namespace.is_empty() {
                name.to_owned()
            } else {
                format!("{namespace}/{name}")
            };
            if truncated {
                // Not among what was read is not absent: the read stopped short.
                format!(
                    "The host read the first 2,000 {} and {at} is not among them, so its path cannot be read",
                    link.from
                )
            } else {
                format!(
                    "The host's read of {} does not hold {at}; refresh the view",
                    link.from
                )
            }
        })?;
    Ok((object, link))
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
/// the list of `to` — the declared reader's, or the built-in kind's metadata —
/// read once through the join snapshot cache.
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
    let target = Listed::of(&plugin.manifest, &link.to).ok_or_else(|| {
        (
            String::new(),
            "No declared reader lists the link's target, and it is not a built-in kind this host lists"
                .to_owned(),
        )
    })?;
    let capability = target.capability();
    let failed = |why: String| (capability.clone(), why);
    let references = if link.match_by.path.is_some() {
        let (object, link) = read_from(cache, client_cache, core, plugin, link, context, resource)
            .await
            .map_err(failed)?;
        path_references(&link, &object).map_err(failed)?
    } else {
        references(link, resource).map_err(failed)?
    };
    if references.is_empty() {
        return Ok((capability, vec![]));
    }
    // Only unplaced names: nothing to look for, so no list is read.
    if needs_no_list(&references) {
        let targets = lookup(&references, &[], &link.to, true).map_err(failed)?;
        return Ok((capability, targets));
    }
    let namespaced = target.namespaced();
    // One namespace's list when every reference names the same one, so a
    // namespace-scoped grant is enough; otherwise the whole cluster's.
    let scope = list_scope(&references, namespaced).map_err(failed)?;
    // Listed at the version the target's reader resolves to on this cluster (#547). A
    // link reads only names from it, so no path depends on which version that is.
    let (objects, truncated, _version) = read_listed(
        cache,
        client_cache,
        core,
        plugin,
        target,
        &link.to,
        context,
        &scope,
    )
    .await
    .map_err(|error| failed(error.to_string()))?;
    // A target past the cut could exist: "not found" would be a guess.
    if truncated {
        return Err(failed(format!(
            "The {} list reached its 2,000-object limit, so a target past it cannot be looked up",
            link.to
        )));
    }
    let targets = lookup(&references, &objects, &link.to, namespaced).map_err(failed)?;
    Ok((capability, targets))
}

/// The resource a reverse view is shown for: the target a link's references
/// are held against.
pub(super) struct Target<'a> {
    namespace: Option<&'a str>,
    name: &'a str,
    uid: Option<&'a str>,
}

/// Whether one reference names the target: yes, no, or maybe and why.
#[derive(Debug, PartialEq, Eq)]
enum Names {
    Yes,
    Maybe(String),
    No,
}

/// Whether `reference` names `target`, a resource of a namespaced kind or not.
///
/// The forward lookup's rule held the other way round: the name, the uid an
/// owner reference also carries, and the namespace, which a cluster-scoped
/// target has none of. A reference that leaves the namespace unsaid — an
/// unplaced Argo CD name, or a cluster-scoped resource's label — might name
/// this target or a namesake elsewhere, and is reported as that.
fn names(reference: &Reference, target: &Target, namespaced: bool) -> Names {
    if reference.name != target.name {
        return Names::No;
    }
    if let (Some(want), Some(have)) = (reference.uid.as_deref(), target.uid) {
        if want != have {
            return Names::No;
        }
    }
    if !namespaced {
        return Names::Yes;
    }
    if reference.unplaced {
        return Names::Maybe(
            "namespace unknown: the app declares no defaultNamespace for a bare name".into(),
        );
    }
    match reference.namespace.as_deref() {
        Some(namespace) if Some(namespace) == target.namespace => Names::Yes,
        Some(_) => Names::No,
        None => Names::Maybe(
            "the reference names no namespace, so a namesake in another namespace may be the one meant"
                .into(),
        ),
    }
}

/// The resources in `objects` whose `link` names `target`, and a sentence for
/// those whose link could not be read on them.
///
/// Each object's references are read exactly as the forward link reads them —
/// its metadata, or its path — so the two views cannot disagree about a
/// resource the host read. One whose link could not be read is counted rather
/// than dropped: it is neither a source nor known not to be one.
pub(super) fn sources(
    link: &ResourceLink,
    objects: &[Value],
    target: &Target,
    namespaced: bool,
) -> (Vec<LinkTarget>, Option<String>) {
    let mut found = Vec::new();
    let mut unread = 0usize;
    let mut first = None;
    for object in objects {
        let metadata = &object["metadata"];
        let name = metadata["name"].as_str().unwrap_or("");
        let namespace = metadata["namespace"]
            .as_str()
            .filter(|namespace| !namespace.is_empty());
        let read = if link.match_by.path.is_some() {
            path_references(link, object)
        } else {
            references(link, object)
        };
        let references = match read {
            Ok(references) => references,
            Err(why) => {
                unread += 1;
                first.get_or_insert_with(|| match namespace {
                    Some(namespace) => format!("{namespace}/{name}: {why}"),
                    None => format!("{name}: {why}"),
                });
                continue;
            }
        };
        // A definite reference outweighs an uncertain one on the same resource.
        let mut verdict: Option<Option<String>> = None;
        for reference in &references {
            match names(reference, target, namespaced) {
                Names::Yes => {
                    verdict = Some(None);
                    break;
                }
                Names::Maybe(why) => {
                    verdict.get_or_insert(Some(why));
                }
                Names::No => {}
            }
        }
        if let Some(unverified) = verdict {
            found.push(LinkTarget {
                namespace: namespace.map(str::to_owned),
                name: name.to_owned(),
                exists: true,
                unverified,
            });
        }
    }
    let unreadable = first.map(|first| {
        format!(
            "{unread} {} could not be read for this link ({first})",
            if unread == 1 { "resource" } else { "resources" }
        )
    });
    (found, unreadable)
}

/// The namespace to list `from` in for a target's reverse view: the target's
/// own when the link can only name a target beside the resource it is read
/// from, and the whole cluster otherwise.
///
/// A label without a namespace label, an annotation read as a name, an owner
/// reference and a same-name match all name a target in the resource's own
/// namespace — when both kinds are namespaced. A namespace label, an Argo CD
/// tracking id and a path may name one anywhere, and a cluster-scoped target is
/// named from every namespace.
fn reverse_scope(
    link: &ResourceLink,
    from_namespaced: bool,
    target_namespaced: bool,
    target_namespace: Option<&str>,
) -> String {
    let matching = &link.match_by;
    let beside =
        matching.path.is_none() && matching.namespace_label.is_none() && matching.parse.is_none();
    match target_namespace {
        Some(namespace) if beside && from_namespaced && target_namespaced => namespace.to_owned(),
        _ => String::new(),
    }
}

/// One link read the other way round for `target`, a resource of the link's `to`:
/// the resources whose link names it, whether the read of `from` stopped at its
/// cap, and a sentence for those the link could not be read on.
#[allow(clippy::too_many_arguments)]
async fn reverse_sources(
    cache: &JoinCache,
    client_cache: &srelens_kube::client_cache::ClientCache,
    core: &Registry,
    plugin: &Installed,
    link: &ResourceLink,
    context: &str,
    target: &Target<'_>,
    target_namespaced: bool,
) -> Result<(Vec<LinkTarget>, bool, Option<String>), String> {
    let from = Listed::of(&plugin.manifest, &link.from).ok_or_else(|| {
        format!(
            "The host lists no {}: no declared reader lists it and it is not a built-in kind, so what names this resource is unknown",
            link.from
        )
    })?;
    let scope = reverse_scope(link, from.namespaced(), target_namespaced, target.namespace);
    let (objects, truncated, version) = read_listed(
        cache,
        client_cache,
        core,
        plugin,
        from,
        &link.from,
        context,
        &scope,
    )
    .await
    .map_err(|error| error.to_string())?;
    let link = match (from, &version) {
        (Listed::Reader(reader), Some(version)) => {
            link_at_version(plugin, &reader.name, version, &link.id)?
        }
        _ => link.clone(),
    };
    let (sources, unreadable) = sources(&link, &objects, target, target_namespaced);
    Ok((sources, truncated, unreadable))
}

/// The link `id` as the manifest reads `reader`'s objects at `version`, where they
/// were read (#547): a path is one of the reader's paths, and an override may move it.
fn link_at_version(
    plugin: &Installed,
    reader: &str,
    version: &str,
    id: &str,
) -> Result<ResourceLink, String> {
    read_at(&plugin.manifest, reader, version)
        .map_err(|error| error.to_string())?
        .contributions
        .resource_links
        .into_iter()
        .find(|declared| declared.id == id)
        .ok_or_else(|| "The link is no longer declared; refresh the view".to_owned())
}

pub(super) fn register(
    reg: &mut Registry,
    path: Store,
    core: Arc<Registry>,
    client_cache: Arc<srelens_kube::client_cache::ClientCache>,
    cache: JoinCache,
) {
    register_reverse(
        reg,
        path.clone(),
        core.clone(),
        client_cache.clone(),
        cache.clone(),
    );
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

/// `extensions.resolveReverseLinks` (#728): an app's links read the other way
/// round, for the Inspector of a resource of their `to` kind.
fn register_reverse(
    reg: &mut Registry,
    path: Store,
    core: Arc<Registry>,
    client_cache: Arc<srelens_kube::client_cache::ClientCache>,
    cache: JoinCache,
) {
    reg.register(Capability::typed::<ResolveLinks, ReverseLinks, _, _>(
        "extensions.resolveReverseLinks",
        "Resolve the resources whose app resource links name a resource, for its Inspector",
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
                let metadata = &input.resource["metadata"];
                let target = Target {
                    namespace: Some(input.namespace.as_str()).filter(|n| !n.is_empty()),
                    name: metadata["name"].as_str().unwrap_or(""),
                    uid: metadata["uid"].as_str(),
                };
                let target_namespaced = Listed::of(&plugin.manifest, &input.kind)
                    .map_or(target.namespace.is_some(), Listed::namespaced);
                let mut links = Vec::new();
                for link in plugin
                    .manifest
                    .contributions
                    .resource_links
                    .iter()
                    .filter(|link| link.to == input.kind)
                {
                    let resolved = reverse_sources(
                        &cache,
                        &client_cache,
                        &core,
                        plugin,
                        link,
                        &context,
                        &target,
                        target_namespaced,
                    )
                    .await;
                    let (sources, truncated, unreadable, error) = match resolved {
                        Ok((sources, truncated, unreadable)) => {
                            (sources, truncated, unreadable, None)
                        }
                        Err(why) => (vec![], false, None, Some(why)),
                    };
                    links.push(ReverseLink {
                        id: link.id.clone(),
                        relation: link.relation,
                        from: link.from.clone(),
                        capability: Listed::of(&plugin.manifest, &link.from)
                            .map(Listed::capability)
                            .unwrap_or_default(),
                        sources,
                        truncated,
                        unreadable,
                        error,
                    });
                }
                Ok(ReverseLinks {
                    to: ResourceRef {
                        kind: input.kind.clone(),
                        namespace: target.namespace.map(str::to_owned),
                        name: target.name.to_owned(),
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
                key_id: None,
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

    // ---- Built-in targets, spec paths and reverse views (#728) ----

    fn path_link(from: &str, to: &str, path: &str) -> ResourceLink {
        link(
            json!({"id":"path","from":from,"to":to,"relation":"references",
            "match":{"path":path}}),
        )
    }

    fn route() -> Value {
        json!({"apiVersion":"gateway.networking.k8s.io/v1","kind":"HTTPRoute",
        "metadata":{"name":"web","namespace":"team"},
        "spec":{"rules":[
            {"backendRefs":[{"name":"api","port":80}]},
            {"backendRefs":[
                {"group":"","kind":"Service","name":"web","port":80},
                {"name":"old","namespace":"legacy","port":80},
                {"group":"multicluster.x-k8s.io","kind":"ServiceImport","name":"far","port":80}
            ]}
        ]}})
    }

    #[test]
    fn a_path_names_its_targets_by_name_or_by_a_reference_to_the_links_kind() {
        // HTTPRoute → Service: every backend of every rule; a backend without a
        // kind is a Service, as Gateway API defaults it, and one of another kind
        // or group is some other link's.
        let backends = path_link(
            "gateway.networking.k8s.io/HTTPRoute",
            "/Service",
            ".spec.rules[*].backendRefs[*]",
        );
        assert_eq!(
            path_references(&backends, &route()),
            Ok(vec![
                reference(Some("team"), "api"),
                reference(Some("team"), "web"),
                reference(Some("legacy"), "old"),
            ])
        );
        // Kustomization → GitRepository: the source reference's kind and
        // apiVersion group choose the link, and its namespace defaults to the
        // Kustomization's own.
        let source = path_link(
            "kustomize.toolkit.fluxcd.io/Kustomization",
            "source.toolkit.fluxcd.io/GitRepository",
            ".spec.sourceRef",
        );
        let kustomization = |source_ref: Value| {
            json!({"apiVersion":"kustomize.toolkit.fluxcd.io/v1","kind":"Kustomization",
                "metadata":{"name":"apps","namespace":"flux-system"},"spec":{"sourceRef":source_ref}})
        };
        assert_eq!(
            path_references(
                &source,
                &kustomization(json!({"kind":"GitRepository","name":"repo"}))
            ),
            Ok(vec![reference(Some("flux-system"), "repo")])
        );
        assert_eq!(
            path_references(
                &source,
                &kustomization(
                    json!({"kind":"GitRepository","name":"repo","namespace":"infra",
                    "apiVersion":"source.toolkit.fluxcd.io/v1"})
                )
            ),
            Ok(vec![reference(Some("infra"), "repo")])
        );
        for other in [
            json!({"kind":"OCIRepository","name":"repo"}),
            json!({"kind":"GitRepository","name":"repo","apiVersion":"example.io/v1"}),
        ] {
            assert_eq!(
                path_references(&source, &kustomization(other.clone())),
                Ok(vec![]),
                "{other}"
            );
        }
        // ExternalSecret → Secret: the path holds the name itself.
        let secret = path_link(
            "external-secrets.io/ExternalSecret",
            "/Secret",
            ".spec.target.name",
        );
        let external = json!({"apiVersion":"external-secrets.io/v1beta1","kind":"ExternalSecret",
            "metadata":{"name":"db","namespace":"team"},"spec":{"target":{"name":"db-creds"}}});
        assert_eq!(
            path_references(&secret, &external),
            Ok(vec![reference(Some("team"), "db-creds")])
        );
        // Unset, null or empty: no reference, and an answer.
        for unset in [
            json!({}),
            json!({"spec":{"target":null}}),
            json!({"spec":{"target":{"name":""}}}),
        ] {
            assert_eq!(path_references(&secret, &unset), Ok(vec![]), "{unset}");
        }
    }

    #[test]
    fn a_path_value_that_is_no_reference_is_an_error_not_no_link() {
        let backends = path_link(
            "gateway.networking.k8s.io/HTTPRoute",
            "/Service",
            ".spec.rules[*].backendRefs",
        );
        let error = path_references(&backends, &route()).unwrap_err();
        assert!(error.contains("[*]"), "{error}");
        let nameless = path_link(
            "gateway.networking.k8s.io/HTTPRoute",
            "/Service",
            ".spec.rules[*].backendRefs[*]",
        );
        let route = json!({"metadata":{"name":"web","namespace":"team"},
            "spec":{"rules":[{"backendRefs":[{"port":80}]}]}});
        assert!(path_references(&nameless, &route)
            .unwrap_err()
            .contains("no name"));
        let number = path_link(
            "gateway.networking.k8s.io/HTTPRoute",
            "/Service",
            ".spec.port",
        );
        assert!(path_references(&number, &json!({"spec":{"port":80}})).is_err());
    }

    /// Deployments as the host's metadata read of them holds them.
    fn tracked(namespace: &str, name: &str, id: Option<&str>) -> Value {
        let annotations = match id {
            Some(id) => json!({"argocd.argoproj.io/tracking-id": id}),
            None => json!({}),
        };
        json!({"apiVersion":"apps/v1","kind":"Deployment","metadata":{"name":name,
            "namespace":namespace,"annotations":annotations}})
    }

    fn guestbook() -> Target<'static> {
        Target {
            namespace: Some("argocd"),
            name: "guestbook",
            uid: Some("u-app"),
        }
    }

    #[test]
    fn a_reverse_view_finds_the_resources_whose_link_names_the_target() {
        let mut link = argo_link();
        link.match_by.default_namespace = Some("argocd".into());
        let objects = vec![
            tracked("team", "api", Some("guestbook:apps/Deployment:team/api")),
            tracked(
                "web",
                "front",
                Some("argocd_guestbook:apps/Deployment:web/front"),
            ),
            // Another application, and a copied id that names another workload.
            tracked("team", "other", Some("billing:apps/Deployment:team/other")),
            tracked("team", "clone", Some("guestbook:apps/Deployment:team/api")),
            // A namesake application in another namespace.
            tracked(
                "team",
                "elsewhere",
                Some("apps_guestbook:apps/Deployment:team/elsewhere"),
            ),
            tracked("team", "plain", None),
        ];
        let (found, unreadable) = sources(&link, &objects, &guestbook(), true);
        assert_eq!(
            found,
            vec![
                target(Some("team"), "api", true),
                target(Some("web"), "front", true)
            ]
        );
        assert_eq!(unreadable, None);
        // Without a default namespace a bare name may be this application or a
        // namesake: reported, and said why, never counted as certain.
        let (found, _) = sources(&argo_link(), &objects, &guestbook(), true);
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found[0]
            .unverified
            .as_deref()
            .is_some_and(|why| why.contains("namespace")));
        assert_eq!(found[1], target(Some("web"), "front", true));
    }

    #[test]
    fn a_reverse_view_holds_owner_references_to_the_targets_uid() {
        let owned = link(
            json!({"id":"owner","from":"/Secret","to":"external-secrets.io/ExternalSecret",
            "relation":"ownedBy","match":{"ownerReference":true}}),
        );
        let secret = |name: &str, uid: &str| {
            json!({"apiVersion":"v1","kind":"Secret","metadata":{"name":name,"namespace":"team",
                "ownerReferences":[{"apiVersion":"external-secrets.io/v1beta1",
                    "kind":"ExternalSecret","name":"db","uid":uid}]}})
        };
        let target = Target {
            namespace: Some("team"),
            name: "db",
            uid: Some("u-es"),
        };
        // The second is owned by an ExternalSecret of the same name that was
        // deleted and recreated: not this one.
        let (found, _) = sources(
            &owned,
            &[secret("db-creds", "u-es"), secret("stale", "u-old")],
            &target,
            true,
        );
        assert_eq!(found, vec![self::target(Some("team"), "db-creds", true)]);
    }

    #[test]
    fn a_reverse_view_counts_what_it_could_not_read_instead_of_dropping_it() {
        let flux = link(json!({"id":"flux","from":"apps/Deployment",
            "to":"kustomize.toolkit.fluxcd.io/Kustomization","relation":"managedBy",
            "match":{"label":"kustomize.toolkit.fluxcd.io/name",
                "namespaceLabel":"kustomize.toolkit.fluxcd.io/namespace"}}));
        let labelled = |name: &str, labels: Value| json!({"metadata":{"name":name,"namespace":"team","labels":labels}});
        let objects = vec![
            labelled(
                "api",
                json!({"kustomize.toolkit.fluxcd.io/name":"apps",
                "kustomize.toolkit.fluxcd.io/namespace":"flux-system"}),
            ),
            // Named, but with no namespace label: which Kustomization is unknown.
            labelled("half", json!({"kustomize.toolkit.fluxcd.io/name":"apps"})),
            labelled("plain", json!({})),
        ];
        let apps = Target {
            namespace: Some("flux-system"),
            name: "apps",
            uid: None,
        };
        let (found, unreadable) = sources(&flux, &objects, &apps, true);
        assert_eq!(found, vec![target(Some("team"), "api", true)]);
        let unreadable = unreadable.expect("the half-labelled Deployment is reported");
        assert!(
            unreadable.starts_with("1 resource could not be read"),
            "{unreadable}"
        );
        assert!(unreadable.contains("team/half"), "{unreadable}");
    }

    #[test]
    fn a_reverse_view_lists_one_namespace_only_when_the_link_cannot_leave_it() {
        let by = |matching: Value| {
            link(
                json!({"id":"l","from":"apps/Deployment","to":"acme.io/Stack",
                "relation":"ownedBy","match":matching}),
            )
        };
        // Named beside the resource: the target's namespace is enough.
        for beside in [
            json!({"ownerReference":true}),
            json!({"name":true}),
            json!({"label":"acme.io/stack"}),
            json!({"annotation":"acme.io/stack"}),
        ] {
            assert_eq!(
                reverse_scope(&by(beside.clone()), true, true, Some("team")),
                "team",
                "{beside}"
            );
            // A cluster-scoped target is named from every namespace, and a
            // cluster-scoped source has none.
            assert_eq!(
                reverse_scope(&by(beside.clone()), true, false, None),
                "",
                "{beside}"
            );
            assert_eq!(
                reverse_scope(&by(beside.clone()), false, true, Some("team")),
                "",
                "{beside}"
            );
        }
        // Named from anywhere: the whole cluster.
        for anywhere in [
            json!({"label":"acme.io/stack","namespaceLabel":"acme.io/namespace"}),
            json!({"annotation":"argocd.argoproj.io/tracking-id","parse":"argocd-tracking-id"}),
            json!({"path":".spec.stackRef"}),
        ] {
            assert_eq!(
                reverse_scope(&by(anywhere.clone()), true, true, Some("team")),
                "",
                "{anywhere}"
            );
        }
    }

    #[test]
    fn every_builtin_link_kind_is_one_kube_reads_in_that_group() {
        for builtin in srelens_plugin_host::BUILTIN_LINK_KINDS {
            let (gvk, namespaced) = srelens_kube::manifest::gvk_for(builtin.kind)
                .unwrap_or_else(|| panic!("{} is not a kind kube knows", builtin.kind));
            assert_eq!(
                (
                    gvk.group.as_str(),
                    gvk.version.as_str(),
                    gvk.kind.as_str(),
                    namespaced
                ),
                (
                    builtin.group,
                    builtin.version,
                    builtin.kind,
                    builtin.namespaced
                ),
            );
        }
    }

    // ---- The resolvers end to end, against a fake API server (#728) ----

    const PLAINTEXT: [&str; 3] = ["hunter2", "aHVudGVyMg==", "password"];

    /// A Secret as a server that ignored the metadata-only request would send it.
    fn secret_object(name: &str, owner_uid: &str) -> Value {
        json!({"apiVersion":"v1","kind":"Secret",
            "metadata":{"name":name,"namespace":"team","uid":format!("u-{name}"),
                "annotations":{"note":"hunter2"},
                "ownerReferences":[{"apiVersion":"external-secrets.io/v1beta1",
                    "kind":"ExternalSecret","name":"db","uid":owner_uid}]},
            "data":{"password":"aHVudGVyMg=="},"stringData":{"password":"hunter2"}})
    }

    fn external_secret() -> Value {
        json!({"apiVersion":"external-secrets.io/v1beta1","kind":"ExternalSecret",
            "metadata":{"name":"db","namespace":"team","uid":"u-es"},
            "spec":{"target":{"name":"db-creds"}}})
    }

    /// One page of 2,500 Deployments, 500 at a time: `api` on the first tracks the
    /// Argo CD Application argocd/guestbook, and `late` on the last does too.
    fn deployments_page(query: &str) -> Value {
        let page = page_of(query);
        let mut items: Vec<Value> = (0..500)
            .map(|i| tracked("team", &format!("d-{page}-{i}"), None))
            .collect();
        if page == 1 {
            items[0] = tracked(
                "team",
                "api",
                Some("argocd_guestbook:apps/Deployment:team/api"),
            );
        }
        if page == 5 {
            items[0] = tracked(
                "team",
                "late",
                Some("argocd_guestbook:apps/Deployment:team/late"),
            );
        }
        let next = (page < 5).then(|| format!("p{}", page + 1));
        json!({"apiVersion":"v1","kind":"List","metadata":{"continue":next},"items":items})
    }

    /// Which of five pages a list request asks for, by its continue token.
    fn page_of(query: &str) -> usize {
        query
            .split('&')
            .find_map(|pair| pair.strip_prefix("continue=p"))
            .and_then(|page| page.parse().ok())
            .unwrap_or(1)
    }

    /// One page of the 2,500 ExternalSecrets in namespace `busy`, 500 at a time:
    /// `early` is on the first page, and `late`, on the last, is past the cut.
    fn busy_external_secrets_page(query: &str) -> Value {
        let page = page_of(query);
        let secret = |name: String| {
            json!({"apiVersion":"external-secrets.io/v1beta1","kind":"ExternalSecret",
                "metadata":{"name":name,"namespace":"busy"},"spec":{"target":{"name":"db-creds"}}})
        };
        let mut items: Vec<Value> = (0..500).map(|i| secret(format!("es-{page}-{i}"))).collect();
        if page == 1 {
            items[0] = secret("early".into());
        }
        if page == 5 {
            items[0] = secret("late".into());
        }
        let next = (page < 5).then(|| format!("p{}", page + 1));
        json!({"apiVersion":"v1","kind":"List","metadata":{"continue":next},"items":items})
    }

    /// A cluster with ExternalSecrets (2,500 of them in `busy`), Secrets, an
    /// HTTPRoute, Services and 2,500 Deployments, and every request it was asked.
    fn cluster() -> (
        srelens_kube::test_support::Client,
        Arc<std::sync::Mutex<Vec<srelens_kube::test_support::Seen>>>,
    ) {
        srelens_kube::test_support::fake_api(|seen| {
            let list = |items: Vec<Value>| json!({"apiVersion":"v1","kind":"List","metadata":{},"items":items});
            match seen.path.as_str() {
                "/apis/external-secrets.io/v1beta1/namespaces/team/externalsecrets" => {
                    list(vec![external_secret()])
                }
                "/api/v1/namespaces/team/secrets" => list(vec![
                    secret_object("db-creds", "u-es"),
                    // Owned by an ExternalSecret `db` since deleted and recreated.
                    secret_object("stale", "u-old"),
                ]),
                "/apis/apps/v1/deployments" | "/apis/apps/v1/namespaces/team/deployments" => {
                    deployments_page(&seen.query)
                }
                "/apis/external-secrets.io/v1beta1/namespaces/busy/externalsecrets" => {
                    busy_external_secrets_page(&seen.query)
                }
                "/apis/gateway.networking.k8s.io/v1/namespaces/team/httproutes"
                | "/apis/gateway.networking.k8s.io/v1/httproutes" => list(vec![route()]),
                "/api/v1/services" | "/api/v1/namespaces/team/services" => list(vec![
                    json!({"apiVersion":"v1","kind":"Service","metadata":{"name":"api","namespace":"team"}}),
                    json!({"apiVersion":"v1","kind":"Service","metadata":{"name":"old","namespace":"legacy"}}),
                ]),
                _ => list(vec![]),
            }
        })
    }

    /// Installs `links` in an app with ExternalSecret and HTTPRoute readers, on a
    /// host whose context `fake` is the fake cluster.
    async fn installed_on_fake(
        links: Value,
    ) -> (
        tempfile::TempDir,
        Registry,
        u64,
        Arc<std::sync::Mutex<Vec<srelens_kube::test_support::Seen>>>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("apps.json");
        let mut core = crate::build_registry_with_paths(
            srelens_kube::client_cache::ClientCache::new_many(vec![]),
            vec![],
        );
        super::super::tests::serve_crds(
            &mut core,
            &[
                "externalsecrets.external-secrets.io/v1beta1",
                "httproutes.gateway.networking.k8s.io/v1",
                "applications.argoproj.io/v1alpha1",
            ],
        );
        let core = Arc::new(core);
        let reader = |name: &str, group: &str, version: &str, kind: &str| {
            json!({"name":name,"title":format!("List {name}"),"target":"k8s.listCustomResource",
                "arguments":{"group":group,"version":version,"plural":name,"kind":kind,"namespaced":true},
                "inputs":["context","namespace"]})
        };
        let manifest = json!({"id":"org.example.links","name":"Links","version":"0.1.0",
            "srelensApiVersion":"^0.5","kind":"declarative","permissions":["k8s.listCustomResource"],
            "capabilities":[
                reader("externalsecrets","external-secrets.io","v1beta1","ExternalSecret"),
                reader("httproutes","gateway.networking.k8s.io","v1","HTTPRoute"),
                reader("applications","argoproj.io","v1alpha1","Application"),
            ],
            "contributions":{"pages":[
                {"id":"externalsecrets","title":"External secrets","capability":"externalsecrets"},
                {"id":"httproutes","title":"Routes","capability":"httproutes"}],
                "detailTabs":[],"detailLinks":[],"resourceLinks":links}});
        let revision = mutate(
            &path,
            core.clone(),
            Configure::Install {
                signature: None,
                key_id: None,
                manifest: manifest.to_string(),
                grants: vec!["k8s.listCustomResource".into()],
                reviewed_revision: None,
            },
        )
        .unwrap()
        .plugins[0]
            .revision;
        let (client, seen) = cluster();
        let clients = srelens_kube::client_cache::ClientCache::new_many(vec![]);
        clients.preload("fake", client).await;
        let mut reg = Registry::new();
        super::super::register(&mut reg, path, core, clients);
        (dir, reg, revision, seen)
    }

    fn call(revision: u64, kind: &str, resource: Value) -> Value {
        json!({"id":"org.example.links","revision":revision,"context":"fake",
            "namespace":"team","kind":kind,"resource":resource})
    }

    /// What the Inspector sends: identity and metadata, never the body.
    fn identity(object: &Value) -> Value {
        json!({"apiVersion":object["apiVersion"],"kind":object["kind"],"metadata":object["metadata"]})
    }

    fn assert_no_plaintext(out: &Value) {
        let text = out.to_string();
        for secret in PLAINTEXT {
            assert!(!text.contains(secret), "{secret} left the host: {text}");
        }
    }

    #[tokio::test]
    async fn a_path_link_reads_its_resource_through_the_reader_and_finds_a_secret_by_identity() {
        let (_dir, reg, revision, seen) = installed_on_fake(json!([{"id":"target",
            "from":"external-secrets.io/ExternalSecret","to":"/Secret","relation":"references",
            "match":{"path":".spec.target.name"}}]))
        .await;
        let out = reg
            .invoke(
                "extensions.resolveLinks",
                call(
                    revision,
                    "external-secrets.io/ExternalSecret",
                    identity(&external_secret()),
                ),
            )
            .await
            .unwrap();
        let link = &out["links"][0];
        assert!(link.get("error").is_none(), "{out}");
        // A built-in target opens in the host's own Inspector: no reader names it.
        assert_eq!(link["capability"], "");
        assert_eq!(link["to"], "/Secret");
        assert_eq!(
            link["targets"],
            json!([{"namespace":"team","name":"db-creds","exists":true}])
        );
        assert_no_plaintext(&out);
        let seen = seen.lock().unwrap();
        // The spec came from the host's own read, not from the caller, which sent none.
        assert!(
            seen.iter()
                .any(|s| s.path.ends_with("/namespaces/team/externalsecrets")),
            "{seen:?}"
        );
        let secrets = seen
            .iter()
            .find(|s| s.path == "/api/v1/namespaces/team/secrets")
            .expect("secrets listed");
        assert!(
            secrets.accept.contains("PartialObjectMetadataList"),
            "{secrets:?}"
        );
    }

    #[tokio::test]
    async fn a_secrets_reverse_view_reads_its_owner_references_and_no_value() {
        let (_dir, reg, revision, seen) = installed_on_fake(json!([{"id":"owner","from":"/Secret",
            "to":"external-secrets.io/ExternalSecret","relation":"ownedBy",
            "match":{"ownerReference":true}}]))
        .await;
        let out = reg
            .invoke(
                "extensions.resolveReverseLinks",
                call(
                    revision,
                    "external-secrets.io/ExternalSecret",
                    identity(&external_secret()),
                ),
            )
            .await
            .unwrap();
        assert_eq!(
            out["to"],
            json!({"kind":"external-secrets.io/ExternalSecret","namespace":"team","name":"db"})
        );
        let link = &out["links"][0];
        assert!(link.get("error").is_none(), "{out}");
        assert_eq!(link["from"], "/Secret");
        assert_eq!(link["relation"], "ownedBy");
        assert_eq!(link["capability"], "");
        assert_eq!(link["truncated"], false);
        // Both name an owner `db`; only the one whose uid is this ExternalSecret's counts.
        assert_eq!(
            link["sources"],
            json!([{"namespace":"team","name":"db-creds","exists":true}])
        );
        assert_no_plaintext(&out);
        // Owner references name a target beside them: one namespace was listed.
        let seen = seen.lock().unwrap();
        let secrets = seen.iter().find(|s| s.path.ends_with("/secrets")).unwrap();
        assert_eq!(secrets.path, "/api/v1/namespaces/team/secrets");
    }

    #[tokio::test]
    async fn a_services_reverse_view_reads_every_routes_backends() {
        let (_dir, reg, revision, _seen) = installed_on_fake(json!([{"id":"backends",
            "from":"gateway.networking.k8s.io/HTTPRoute","to":"/Service","relation":"references",
            "match":{"path":".spec.rules[*].backendRefs[*]"}}]))
        .await;
        let seen = _seen;
        let service = json!({"apiVersion":"v1","kind":"Service","metadata":{"name":"web","namespace":"team"}});
        let out = reg
            .invoke(
                "extensions.resolveReverseLinks",
                call(revision, "/Service", service),
            )
            .await
            .unwrap();
        let link = &out["links"][0];
        // A backend may name another namespace, so every route in the cluster is read.
        assert!(seen
            .lock()
            .unwrap()
            .iter()
            .any(|s| s.path == "/apis/gateway.networking.k8s.io/v1/httproutes"));
        // The HTTPRoute is read through the app's reader and its capability is the page's.
        assert_eq!(link["capability"], "httproutes");
        assert_eq!(
            link["sources"],
            json!([{"namespace":"team","name":"web","exists":true}]),
            "{out}"
        );
        // Forward, from the route: three backends are Services, one is not.
        let out = reg
            .invoke(
                "extensions.resolveLinks",
                call(
                    revision,
                    "gateway.networking.k8s.io/HTTPRoute",
                    identity(&route()),
                ),
            )
            .await
            .unwrap();
        assert_eq!(
            out["links"][0]["targets"],
            json!([{"namespace":"team","name":"api","exists":true},
                   {"namespace":"team","name":"web","exists":false},
                   {"namespace":"legacy","name":"old","exists":true}]),
            "{out}"
        );
    }

    #[tokio::test]
    async fn a_path_link_on_a_resource_the_host_cannot_find_is_an_error_not_no_link() {
        let (_dir, reg, revision, _seen) = installed_on_fake(json!([{"id":"target",
            "from":"external-secrets.io/ExternalSecret","to":"/Secret","relation":"references",
            "match":{"path":".spec.target.name"}}]))
        .await;
        let mut gone = external_secret();
        gone["metadata"]["name"] = json!("gone");
        let out = reg
            .invoke(
                "extensions.resolveLinks",
                call(
                    revision,
                    "external-secrets.io/ExternalSecret",
                    identity(&gone),
                ),
            )
            .await
            .unwrap();
        let error = out["links"][0]["error"].as_str().expect("an error");
        assert!(error.contains("does not hold team/gone"), "{error}");
        assert_eq!(out["links"][0]["targets"], json!([]));
    }

    #[tokio::test]
    async fn the_reverse_resolver_takes_the_forward_payload_and_refuses_anything_else() {
        let (_dir, reg, revision, _seen) =
            installed_on_fake(json!([{"id":"owner","from":"/Secret",
            "to":"external-secrets.io/ExternalSecret","relation":"ownedBy",
            "match":{"ownerReference":true}}]))
            .await;
        let payload = call(
            revision,
            "external-secrets.io/ExternalSecret",
            identity(&external_secret()),
        );
        // A kind no link points at has no reverse links, and needs no read.
        let mut other = payload.clone();
        other["kind"] = json!("gateway.networking.k8s.io/HTTPRoute");
        other["resource"] = identity(&route());
        let out = reg
            .invoke("extensions.resolveReverseLinks", other)
            .await
            .unwrap();
        assert_eq!(out["links"], json!([]));
        let mut stale = payload.clone();
        stale["revision"] = json!(revision + 1);
        assert!(reg
            .invoke("extensions.resolveReverseLinks", stale)
            .await
            .is_err());
        let mut wrong = payload.clone();
        wrong["targetKind"] = json!("external-secrets.io/ExternalSecret");
        assert!(reg
            .invoke("extensions.resolveReverseLinks", wrong)
            .await
            .is_err());
        let mut mismatched = payload;
        mismatched["namespace"] = json!("other");
        assert!(reg
            .invoke("extensions.resolveReverseLinks", mismatched)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn a_reverse_view_past_the_list_cap_says_so_and_keeps_what_it_found() {
        let (_dir, reg, revision, _seen) = installed_on_fake(json!([{"id":"argocd-owner",
            "from":"apps/Deployment","to":"argoproj.io/Application","relation":"managedBy",
            "match":{"annotation":"argocd.argoproj.io/tracking-id","parse":"argocd-tracking-id",
                "defaultNamespace":"argocd"}}]))
        .await;
        let application = json!({"apiVersion":"argoproj.io/v1alpha1","kind":"Application",
            "metadata":{"name":"guestbook","namespace":"argocd"}});
        let mut payload = call(revision, "argoproj.io/Application", application);
        payload["namespace"] = json!("argocd");
        let out = reg
            .invoke("extensions.resolveReverseLinks", payload)
            .await
            .unwrap();
        let link = &out["links"][0];
        assert!(link.get("error").is_none(), "{out}");
        // 2,000 of 2,500 were read: `api` is among them, `late` is past the cut, and
        // the answer says there may be more rather than claiming it is whole.
        assert_eq!(link["truncated"], true);
        assert_eq!(
            link["sources"],
            json!([{"namespace":"team","name":"api","exists":true}])
        );
    }

    #[tokio::test]
    async fn a_target_past_the_list_cap_is_an_error_not_missing() {
        // An HTTPRoute backend read as a Deployment's name, so the target list is
        // one namespace of 2,500 Deployments: `d-5-1` exists, past the cut.
        let (_dir, reg, revision, _seen) = installed_on_fake(json!([{"id":"deploy",
            "from":"gateway.networking.k8s.io/HTTPRoute","to":"apps/Deployment",
            "relation":"references","match":{"path":".spec.rules[0].backendRefs[0].name"}}]))
        .await;
        let out = reg
            .invoke(
                "extensions.resolveLinks",
                call(
                    revision,
                    "gateway.networking.k8s.io/HTTPRoute",
                    identity(&route()),
                ),
            )
            .await
            .unwrap();
        let link = &out["links"][0];
        assert!(
            link["error"]
                .as_str()
                .is_some_and(|why| why.contains("2,000")),
            "{out}"
        );
        assert_eq!(link["targets"], json!([]));
    }

    #[tokio::test]
    async fn a_path_link_reads_its_resource_from_a_cut_off_list_and_names_the_cut() {
        // 2,500 ExternalSecrets in one namespace: the host reads 2,000 of them.
        let (_dir, reg, revision, _seen) = installed_on_fake(json!([{"id":"target",
            "from":"external-secrets.io/ExternalSecret","to":"/Secret","relation":"references",
            "match":{"path":".spec.target.name"}}]))
        .await;
        let inspect = |name: &str| {
            let mut payload = call(
                revision,
                "external-secrets.io/ExternalSecret",
                json!({"apiVersion":"external-secrets.io/v1beta1","kind":"ExternalSecret",
                    "metadata":{"name":name,"namespace":"busy"}}),
            );
            payload["namespace"] = json!("busy");
            payload
        };
        // Among what was read: answered, though the list was cut.
        let out = reg
            .invoke("extensions.resolveLinks", inspect("early"))
            .await
            .unwrap();
        let link = &out["links"][0];
        assert!(link.get("error").is_none(), "{out}");
        assert_eq!(
            link["targets"],
            json!([{"namespace":"busy","name":"db-creds","exists":false}])
        );
        // Past the cut: the limit is the reason, not an absence a refresh would fix.
        let out = reg
            .invoke("extensions.resolveLinks", inspect("late"))
            .await
            .unwrap();
        let error = out["links"][0]["error"].as_str().expect("an error");
        assert!(
            error.contains("2,000") && error.contains("busy/late"),
            "{error}"
        );
        assert!(!error.contains("refresh"), "{error}");
    }
}
