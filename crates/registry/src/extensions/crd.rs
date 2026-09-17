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

/// The broker's check that a CustomResourceDefinition serves a bound group, version and
/// plural. Registered only in the registry the broker dispatches through
/// (`build_registry_*`), so it is not in the capability catalog and no MCP client or app
/// binding can call it.
pub(crate) const CHECK: &str = "extensions.customResourceServes";

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CheckIn {
    context: String,
    group: String,
    version: String,
    plural: String,
}

pub(crate) fn check_capability(cache: Arc<srelens_kube::client_cache::ClientCache>) -> Capability {
    Capability::typed::<CheckIn, bool, _, _>(
        CHECK,
        "Whether a CustomResourceDefinition on the cluster serves a group, version and plural",
        Annotations::READ_ONLY,
        move |input: CheckIn| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                srelens_kube::crds::custom_resource_serves(
                    client,
                    &input.group,
                    &input.version,
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

/// Refuses a call unless a CustomResourceDefinition named `{plural}.{group}` serves the
/// binding's version on the cluster `context` names. A failed lookup is reported as one,
/// not as an absence.
pub(super) async fn require(
    core: &Registry,
    context: &str,
    binding: &Binding,
) -> Result<(), CapabilityError> {
    let field = |key: &str| {
        binding
            .arguments
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
    };
    let (group, version, plural) = (field("group"), field("version"), field("plural"));
    let name = format!("{plural}.{group}");
    match core
        .invoke(
            CHECK,
            json!({ "context": context, "group": group, "version": version, "plural": plural }),
        )
        .await
    {
        Ok(Value::Bool(true)) => Ok(()),
        Ok(_) => Err(CapabilityError::Handler(format!(
            "No CustomResourceDefinition {name} serving {version} on this cluster; an app reads only custom resources"
        ))),
        Err(error) => Err(CapabilityError::Handler(format!(
            "Could not confirm that a CustomResourceDefinition {name} serves {version}: {error}"
        ))),
    }
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
            include_str!("../../tests/fixtures/argocd-manifest.json"),
        ] {
            let manifest = Manifest::parse(source).unwrap();
            assert!(group_problems(&manifest).0.is_empty());
        }
    }
}
