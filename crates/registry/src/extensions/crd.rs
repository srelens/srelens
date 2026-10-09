//! A `k8s.listCustomResource` binding reads custom resources, never built-in ones (#601).
//!
//! Two checks keep it there. At install, rollback and load, the bound `group` must have
//! the shape a CustomResourceDefinition group needs: dot-separated labels, which built-in
//! groups such as `apps` lack. At every broker call, a CustomResourceDefinition named
//! `{plural}.{group}` must serve the bound version on the cluster. That also refuses a
//! dotted built-in group such as `networking.k8s.io`, an aggregated API, and a version of
//! a CRD's group and plural that something other than the CRD serves.
use super::*;
use srelens_plugin_host::Binding;

/// The broker's check that a CustomResourceDefinition serves a bound group and plural, and
/// which of the binding's versions it serves first (#547). Registered only in the registry
/// the broker dispatches through (`build_registry_*`), so it is not in the capability
/// catalog and no MCP client or app binding can call it.
pub(crate) const CHECK: &str = "extensions.customResourceServes";

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CheckIn {
    context: String,
    group: String,
    /// Most preferred first; at most `MAX_BINDING_VERSIONS`.
    versions: Vec<String>,
    plural: String,
}

pub(crate) fn check_capability(cache: Arc<srelens_kube::client_cache::ClientCache>) -> Capability {
    Capability::typed::<CheckIn, Option<String>, _, _>(
        CHECK,
        "The first of a group's and plural's versions a CustomResourceDefinition on the cluster serves",
        Annotations::READ_ONLY,
        move |input: CheckIn| {
            let cache = cache.clone();
            async move {
                if input.versions.is_empty()
                    || input.versions.len() > srelens_plugin_host::MAX_BINDING_VERSIONS
                {
                    return Err(CapabilityError::InvalidInput(format!(
                        "Name 1–{} versions",
                        srelens_plugin_host::MAX_BINDING_VERSIONS
                    )));
                }
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                srelens_kube::crds::custom_resource_first_served(
                    client,
                    &input.group,
                    &input.versions,
                    &input.plural,
                )
                .await
                .map_err(CapabilityError::Handler)
            }
        },
    )
}

/// Why `group` cannot be a CustomResourceDefinition's group, if it cannot. Kubernetes
/// requires a CRD group to be a DNS subdomain with at least one dot, which excludes the
/// built-in groups `apps`, `batch`, `policy` and the like. Dotted groups under `k8s.io`
/// are left to the broker's per-cluster check: some are built in, others, such as
/// `gateway.networking.k8s.io`, are CRDs.
fn group_problem(group: &str) -> Option<String> {
    if group.split('.').count() < 2 || group.split('.').any(str::is_empty) {
        return Some(format!(
            "\"{group}\" cannot be a CustomResourceDefinition group: one has dot-separated labels, such as argoproj.io, and built-in groups such as apps and batch do not"
        ));
    }
    None
}

/// Every custom-resource binding whose group no CustomResourceDefinition can declare, at
/// `capabilities[i].arguments.group`. Needs no host registry, so the inventory applies it
/// when it loads.
pub(super) fn group_problems(manifest: &Manifest) -> ValidationErrors {
    let mut problems = ValidationErrors::default();
    for (index, binding) in manifest.capabilities.iter().enumerate() {
        if binding.target != "k8s.listCustomResource" {
            continue;
        }
        let Some(group) = binding.arguments.get("group").and_then(Value::as_str) else {
            continue; // A missing or non-string group is refused by the syntax rule.
        };
        if let Some(reason) = group_problem(group) {
            problems.push(
                Code::InvalidBinding,
                format!("capabilities[{index}].arguments.group"),
                reason,
            );
        }
    }
    problems
}

/// The version the binding reads on the cluster `context` names: the first it accepts
/// that a CustomResourceDefinition named `{plural}.{group}` serves (#547). Refuses the
/// call when the CRD serves none of them — never falls back to a version the binding does
/// not list — and reports a failed lookup as one, not as an absence.
///
/// Not cached: every broker call already looks the CRD up (#601), so resolving costs no
/// extra request, follows a discovery change on the next call, and is per cluster by
/// construction.
pub(super) async fn resolve(
    core: &Registry,
    context: &str,
    binding: &Binding,
) -> Result<String, CapabilityError> {
    match serves(core, context, binding).await {
        Served::Yes(version) => Ok(version),
        Served::No(why) | Served::Unknown(why) => Err(CapabilityError::Handler(why)),
    }
}

/// What the cluster answered about a binding's kind: served, with what (the
/// resolved version, or a watch's target), or not.
pub(super) enum Served<T = String> {
    Yes(T),
    /// The cluster answered: no such CRD serves any version the binding accepts. With why.
    No(String),
    /// The lookup itself failed, so it is not known either way. With why.
    Unknown(String),
}

/// [`resolve`]'s answer with its two refusals kept apart, for a caller that
/// treats a failed lookup (a watch reconnecting) unlike an absence (#566).
pub(super) async fn serves(core: &Registry, context: &str, binding: &Binding) -> Served {
    let field = |key: &str| {
        binding
            .arguments
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
    };
    let (group, plural) = (field("group"), field("plural"));
    let versions = binding.accepted_versions();
    let name = format!("{plural}.{group}");
    let listed = match versions.as_slice() {
        [one] => one.clone(),
        several => format!("any of {}", several.join(", ")),
    };
    match core
        .invoke(
            CHECK,
            json!({ "context": context, "group": group, "versions": versions, "plural": plural }),
        )
        .await
    {
        Ok(Value::String(version)) if versions.contains(&version) => Served::Yes(version),
        Ok(_) => Served::No(format!(
            "No CustomResourceDefinition {name} serving {listed} on this cluster; an app reads only custom resources"
        )),
        Err(error) => Served::Unknown(format!(
            "Could not confirm that a CustomResourceDefinition {name} serves {listed}: {error}"
        )),
    }
}

/// `manifest` as it reads the named custom-resource binding on this cluster: at the
/// version [`resolve`] finds, with that version's path overrides applied.
pub(super) async fn resolved(
    core: &Registry,
    context: &str,
    manifest: &Manifest,
    binding: &str,
) -> Result<(Manifest, String), CapabilityError> {
    let reader = manifest
        .capabilities
        .iter()
        .find(|b| b.name == binding && b.target == "k8s.listCustomResource")
        .ok_or_else(|| CapabilityError::Handler("Declared reader is unavailable".into()))?;
    let version = resolve(core, context, reader).await?;
    let manifest = manifest
        .at_version(binding, &version)
        .map_err(CapabilityError::Handler)?;
    Ok((manifest, version))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_groups_shaped_like_a_crd_group_are_accepted() {
        for group in [
            "argoproj.io",
            "kustomize.toolkit.fluxcd.io",
            "cluster.x-k8s.io",
            "gateway.networking.k8s.io",
        ] {
            assert_eq!(group_problem(group), None, "{group}");
        }
        for group in ["apps", "batch", "policy", "apps.", ".apps", "a..b"] {
            assert!(group_problem(group).is_some(), "{group}");
        }
    }

    #[test]
    fn the_check_is_offered_to_the_broker_only() {
        let dir = tempfile::tempdir().unwrap();
        let reg = crate::build_registry_with_paths_and_settings(
            srelens_kube::client_cache::ClientCache::new_many(vec![]),
            vec![],
            Some(dir.path().join("settings.json")),
        );
        assert!(reg.get("extensions.read").is_some());
        assert!(reg.get(CHECK).is_none());
    }

    #[test]
    fn the_flux_and_argo_cd_examples_bind_only_crd_groups() {
        for source in [
            include_str!("../../../../examples/extensions/argocd.json"),
            include_str!("../../../../examples/extensions/flux.json"),
        ] {
            let manifest = Manifest::parse(source).unwrap();
            assert!(group_problems(&manifest).0.is_empty());
        }
        let historic = include_str!("../../tests/fixtures/argocd-manifest.json");
        assert!(Manifest::parse(historic).unwrap_err().to_string().contains("requires API ^0.1"));
        assert!(group_problems(&Manifest::decode(historic).unwrap()).0.is_empty());
    }
}
