//! Native extension contract and declarative capability broker.
//!
//! No package code is loaded here. A trusted installer supplies explicit
//! grants; the future Freelens adapter targets this same host contract.
pub mod freelens;
mod manifest;
pub use manifest::*;

use serde_json::{Map, Value};
use srelens_capability::{Capability, CapabilityError, Registry};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

pub struct PluginHost {
    core: Arc<Registry>,
}
pub struct Registration {
    manifest: Manifest,
    active: Arc<AtomicBool>,
    ids: Vec<String>,
}
impl Registration {
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    /// Existing registry snapshots retain a handler but can no longer invoke it.
    /// Calls already executing may finish; no new calls are admitted afterward.
    pub fn unregister(&self, registry: &mut Registry) {
        if self.active.swap(false, Ordering::SeqCst) {
            for id in &self.ids {
                registry.unregister(id);
            }
        }
    }
}

impl PluginHost {
    /// Capture only the trusted host registry, before registering extensions.
    pub fn new(core: Arc<Registry>) -> Self {
        Self { core }
    }

    pub fn register(
        &self,
        registry: &mut Registry,
        manifest: Manifest,
        grants: &[String],
    ) -> Result<Registration, String> {
        manifest.validate()?;
        let prefix = format!("plugin/{}/", manifest.id);
        if registry.ids().iter().any(|id| id.starts_with(&prefix)) {
            return Err(format!("extension already registered: {}", manifest.id));
        }
        if manifest.permissions.iter().any(|p| !grants.contains(p)) {
            return Err("extension permissions have not been granted".into());
        }
        let active = Arc::new(AtomicBool::new(true));
        let mut capabilities = Vec::new();
        for binding in &manifest.capabilities {
            let id = format!("plugin/{}/{}", manifest.id, binding.name);
            if registry.get(&id).is_some() {
                return Err(format!("capability already registered: {id}"));
            }
            let target = self
                .core
                .get(&binding.target)
                .ok_or_else(|| format!("unknown host capability: {}", binding.target))?;
            // Use the host schema, never plugin-supplied annotations or schemas.
            let mut schema = target.input_schema.clone();
            let properties = schema
                .get("properties")
                .and_then(Value::as_object)
                .ok_or("host capability needs an object input schema")?;
            if binding
                .inputs
                .iter()
                .chain(binding.arguments.keys())
                .any(|k| !properties.contains_key(k))
            {
                return Err(format!("unknown argument for {}", binding.target));
            }
            let required: Vec<String> = schema
                .get("required")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect();
            if required
                .iter()
                .any(|k| !binding.arguments.contains_key(k) && !binding.inputs.contains(k))
            {
                return Err(format!(
                    "binding omits a required argument for {}",
                    binding.target
                ));
            }
            let exposed: Map<String, Value> = binding
                .inputs
                .iter()
                .map(|k| (k.clone(), properties[k].clone()))
                .collect();
            schema["properties"] = Value::Object(exposed);
            schema["required"] = serde_json::to_value(
                required
                    .iter()
                    .filter(|k| binding.inputs.contains(k))
                    .collect::<Vec<_>>(),
            )
            .unwrap();
            schema["additionalProperties"] = Value::Bool(false);
            let required: Vec<String> = required
                .into_iter()
                .filter(|k| binding.inputs.contains(k))
                .collect();
            let binding = binding.clone();
            let handler = target.handler.clone();
            let enabled = active.clone();
            let mut annotations = target.annotations;
            // Fail closed even if a host annotation accidentally omitted the gate.
            if !annotations.read_only || annotations.sensitive || annotations.destructive {
                annotations.requires_confirm = true;
            }
            capabilities.push(Capability {
                id, summary: format!("{}: {}", manifest.name, binding.title), annotations,
                input_schema: schema, output_schema: target.output_schema.clone(),
                handler: Arc::new(move |input| {
                    let handler = handler.clone();
                    let enabled = enabled.clone();
                    let binding = binding.clone();
                    let required = required.clone();
                    Box::pin(async move {
                        if !enabled.load(Ordering::SeqCst) { return Err(CapabilityError::Handler("extension is disabled".into())); }
                        let input = input.as_object().ok_or_else(|| CapabilityError::InvalidInput("extension input must be an object".into()))?;
                        if input.keys().any(|k| !binding.inputs.contains(k)) || required.iter().any(|k| !input.contains_key(k)) {
                            return Err(CapabilityError::InvalidInput("unexpected or missing extension input; bound arguments cannot be overridden".into()));
                        }
                        let mut args = binding.arguments.clone();
                        args.extend(input.clone());
                        handler(Value::Object(args)).await
                    })
                }),
            });
        }
        let ids = capabilities.iter().map(|c| c.id.clone()).collect();
        // Commit only after every binding has passed validation.
        for cap in capabilities {
            registry.register(cap);
        }
        Ok(Registration {
            manifest,
            active,
            ids,
        })
    }
}
