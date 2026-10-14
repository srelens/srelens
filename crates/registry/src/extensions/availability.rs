//! Discovery of declared bindings, with absence kept distinct from a failed read.
use super::{crd, resolver_app, Store};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use srelens_capability::{Annotations, Capability, CapabilityError, Registry};
use std::collections::HashSet;
use std::sync::Arc;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AvailabilityIn {
    id: String,
    revision: u64,
    context: String,
    #[serde(default)]
    namespace: Option<String>,
    bindings: Vec<String>,
}

#[derive(Serialize, JsonSchema)]
struct AvailabilityOut {
    bindings: Vec<BindingAvailability>,
}

#[derive(Serialize, JsonSchema)]
#[serde(tag = "state", rename_all = "lowercase")]
enum BindingAvailability {
    Served {
        binding: String,
        version: String,
        namespaced: bool,
    },
    Absent {
        binding: String,
    },
    Unknown {
        binding: String,
        reason: String,
    },
}

pub(super) fn register(
    reg: &mut Registry,
    store: Store,
    core: Arc<Registry>,
    cache: Arc<srelens_kube::client_cache::ClientCache>,
) {
    reg.register(Capability::typed::<AvailabilityIn, AvailabilityOut, _, _>(
        "extensions.bindingAvailability",
        "Check which declared app bindings this cluster serves",
        Annotations::READ_ONLY,
        move |input| {
            let (store, core, cache) = (store.clone(), core.clone(), cache.clone());
            async move {
                let mut names = HashSet::new();
                if input.context.trim().is_empty() || input.context.len() > 4096 {
                    return Err(CapabilityError::InvalidInput(
                        "A binding lookup needs an explicit cluster".into(),
                    ));
                }
                if input.bindings.is_empty()
                    || input.bindings.len() > 16
                    || input.bindings.iter().any(|name| !names.insert(name))
                {
                    return Err(CapabilityError::InvalidInput(
                        "Choose 1–16 unique declared binding names".into(),
                    ));
                }
                let (state, index, context) = resolver_app(
                    store,
                    &core,
                    &cache,
                    &input.id,
                    input.revision,
                    input.context,
                )
                .await?;
                let app = &state.plugins[index];
                let mut checked = Vec::new();
                // Validate every name and grant before making any discovery request.
                for name in &input.bindings {
                    let binding = app
                        .manifest
                        .capabilities
                        .iter()
                        .find(|binding| &binding.name == name)
                        .ok_or_else(|| {
                            CapabilityError::InvalidInput(format!(
                                "The app declares no binding {name}"
                            ))
                        })?;
                    if !app.grants.iter().any(|grant| grant == &binding.target) {
                        return Err(CapabilityError::InvalidInput(format!(
                            "The app has no grant for {}",
                            binding.target
                        )));
                    }
                    checked.push(binding);
                }
                let mut bindings = Vec::new();
                for binding in checked {
                    let name = binding.name.clone();
                    let availability = if binding.target == "k8s.listCustomResource" {
                        match crd::serves(&core, &context, binding).await {
                            crd::Served::Yes(version) => BindingAvailability::Served {
                                binding: name,
                                version,
                                namespaced: binding
                                    .arguments
                                    .get("namespaced")
                                    .and_then(serde_json::Value::as_bool)
                                    .unwrap_or(true),
                            },
                            crd::Served::No(_) => BindingAvailability::Absent { binding: name },
                            crd::Served::Unknown(reason) => BindingAvailability::Unknown {
                                binding: name,
                                reason: reason.chars().take(1024).collect(),
                            },
                        }
                    } else if let Some(identity) =
                        srelens_plugin_host::builtin_reader_identity(&binding.target)
                    {
                        BindingAvailability::Served {
                            binding: name,
                            version: "v1".into(),
                            namespaced: identity["namespaced"] == true,
                        }
                    } else {
                        return Err(CapabilityError::InvalidInput(format!(
                            "{} does not name a discoverable resource reader",
                            binding.name
                        )));
                    };
                    bindings.push(availability);
                }
                let _ = input.namespace; // namespace never narrows cluster API discovery.
                Ok(AvailabilityOut { bindings })
            }
        },
    ));
}
