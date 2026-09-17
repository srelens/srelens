//! A `k8s.listCustomResource` binding reads custom resources, never built-in ones (#601).
//!
//! Two checks keep it there. At install, rollback and load, the bound `group` must be one a
//! CustomResourceDefinition can declare. At every broker call, `{plural}.{group}` must be a
//! CustomResourceDefinition on the cluster, which also refuses a dotted group served by an
//! aggregated API.
use super::*;
use srelens_plugin_host::Binding;

/// The broker's check that a bound group and plural name a CustomResourceDefinition.
/// Registered only in the registry the broker dispatches through (`build_registry_*`), so
/// it is not in the capability catalog and no MCP client or app binding can call it.
pub(crate) const CHECK: &str = "extensions.customResourceDefinitionExists";

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CheckIn {
    context: String,
    group: String,
    plural: String,
}

pub(crate) fn check_capability(cache: Arc<srelens_kube::client_cache::ClientCache>) -> Capability {
    Capability::typed::<CheckIn, bool, _, _>(
        CHECK,
        "Whether a group and plural name a CustomResourceDefinition on the cluster",
        Annotations::READ_ONLY,
        move |input: CheckIn| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                srelens_kube::crds::custom_resource_definition_exists(
                    client,
                    &input.group,
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
/// built-in groups `apps`, `batch`, `policy` and the like, and reserves `k8s.io` and its
/// subdomains for Kubernetes.
fn group_problem(group: &str) -> Option<String> {
    if group.split('.').count() < 2 || group.split('.').any(str::is_empty) {
        return Some(format!(
            "\"{group}\" cannot be a CustomResourceDefinition group: one has dot-separated labels, such as argoproj.io, and built-in groups such as apps and batch do not"
        ));
    }
    let lower = group.to_ascii_lowercase();
    if lower == "k8s.io" || lower.ends_with(".k8s.io") {
        return Some(format!(
            "\"{group}\" is reserved for Kubernetes API groups; bind a custom resource group outside k8s.io"
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

/// Refuses a call unless the binding's `{plural}.{group}` is a CustomResourceDefinition on
/// the cluster `context` names. A failed lookup is reported as one, not as an absence.
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
    let (group, plural) = (field("group"), field("plural"));
    let name = format!("{plural}.{group}");
    match core
        .invoke(
            CHECK,
            json!({ "context": context, "group": group, "plural": plural }),
        )
        .await
    {
        Ok(Value::Bool(true)) => Ok(()),
        Ok(_) => Err(CapabilityError::Handler(format!(
            "{name} is not a CustomResourceDefinition on this cluster; an app reads only custom resources"
        ))),
        Err(error) => Err(CapabilityError::Handler(format!(
            "Could not confirm that {name} is a CustomResourceDefinition: {error}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_groups_a_crd_can_declare_are_accepted() {
        for group in [
            "argoproj.io",
            "kustomize.toolkit.fluxcd.io",
            "cluster.x-k8s.io",
            "k8s.io.example.com",
        ] {
            assert_eq!(group_problem(group), None, "{group}");
        }
        for group in [
            "apps",
            "batch",
            "policy",
            "apps.",
            ".apps",
            "a..b",
            "k8s.io",
            "networking.k8s.io",
            "rbac.authorization.k8s.io",
            "Metrics.K8S.io",
        ] {
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
