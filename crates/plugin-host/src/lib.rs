//! Native extension contract and declarative capability broker.
//!
//! No package code is loaded here. A trusted installer supplies explicit
//! grants for native srelens manifests.
#[cfg(any(test, feature = "fuzzing"))]
#[doc(hidden)]
pub mod fuzzing;
mod manifest;
mod validation;
pub use manifest::*;
pub use validation::*;

use serde_json::{Map, Value};
use srelens_capability::{Annotations, Capability, CapabilityError, Registry};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

/// The reader binding `name`, when it lists versions and none has been chosen yet.
fn unresolved_reader<'a>(manifest: &'a Manifest, name: &str) -> Option<&'a Binding> {
    manifest
        .capabilities
        .iter()
        .find(|binding| binding.name == name && !binding.versions.is_empty())
}

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

    /// Every way `binding`, the manifest's `index`th capability, does not fit its target:
    /// a target the host lacks, an argument or input the target does not take, or a
    /// required one left unbound.
    ///
    /// A reader that lists `versions` binds `version` per cluster (#547). Every listed
    /// version fills the same argument, so it is checked as bound at its first one.
    pub fn binding_problems(&self, index: usize, binding: &Binding) -> Vec<ValidationError> {
        let at = format!("capabilities[{index}]");
        match binding.versions.first() {
            Some(first) if !binding.arguments.contains_key("version") => {
                let mut bound = binding.clone();
                bound
                    .arguments
                    .insert("version".to_owned(), Value::String(first.clone()));
                self.problems_at(&at, &bound)
            }
            _ => self.problems_at(&at, binding),
        }
    }

    /// The same, for the `index`th declared action: the binding the host would
    /// build for it, checked against the primitive it names.
    pub fn action_problems(
        &self,
        index: usize,
        manifest: &Manifest,
        action: &ActionBinding,
    ) -> Vec<ValidationError> {
        let at = format!("actions[{index}]");
        // An action on a reader that lists versions acts at whichever one resolved;
        // checked, like the reader, at the first.
        let resolved = unresolved_reader(manifest, &action.resource)
            .and_then(|reader| reader.versions.first())
            .and_then(|first| manifest.at_version(&action.resource, first).ok());
        match resolved.as_ref().unwrap_or(manifest).action_binding(action) {
            // The reader this action names cannot scope it. Reported at
            // `resource`, which is the field that would have to change.
            Err(why) => vec![ValidationError::new(
                ValidationCode::InvalidBinding,
                format!("{at}.resource"),
                why,
            )],
            Ok(binding) => self.problems_at(&at, &binding),
        }
    }

    fn problems_at(&self, at: &str, binding: &Binding) -> Vec<ValidationError> {
        let mut problems = ValidationErrors::default();
        let Some(target) = self.core.get(&binding.target) else {
            problems.push(
                ValidationCode::UnsupportedTarget,
                format!("{at}.target"),
                format!("{} is not a host capability", binding.target),
            );
            return problems.0;
        };
        let Some(properties) = target
            .input_schema
            .get("properties")
            .and_then(Value::as_object)
        else {
            problems.push(
                ValidationCode::UnsupportedTarget,
                format!("{at}.target"),
                format!("{} does not take an object input", binding.target),
            );
            return problems.0;
        };
        for (position, input) in binding.inputs.iter().enumerate() {
            if !properties.contains_key(input) {
                problems.push(
                    ValidationCode::InvalidBinding,
                    format!("{at}.inputs[{position}]"),
                    format!("{} has no input \"{input}\"", binding.target),
                );
            }
        }
        for key in binding.arguments.keys() {
            if !properties.contains_key(key) {
                problems.push(
                    ValidationCode::InvalidBinding,
                    format!("{at}.arguments.{key}"),
                    format!("{} has no argument \"{key}\"", binding.target),
                );
            }
        }
        for key in Self::required_inputs(&target.input_schema) {
            if !binding.arguments.contains_key(&key) && !binding.inputs.contains(&key) {
                problems.push(
                    ValidationCode::InvalidBinding,
                    format!("{at}.arguments.{key}"),
                    format!(
                        "{} requires \"{key}\"; bind it or accept it as an input",
                        binding.target
                    ),
                );
            }
        }
        // The target's own rules for what may be bound to it: the ones a JSON
        // schema cannot state, such as an action primitive's token vocabulary
        // and its deny-list. The handler enforces the same closure, so an app
        // gains nothing by getting a manifest past this.
        if let Some(check) = &target.bound_arguments {
            if let Err(why) = check(&binding.arguments) {
                problems.push(
                    ValidationCode::InvalidBinding,
                    format!("{at}.arguments"),
                    why,
                );
            }
        }
        problems.0
    }

    fn required_inputs(schema: &Value) -> Vec<String> {
        schema
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect()
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
        // Readers as written, then one binding per declared action, which the
        // manifest builds from the reader it names (#549). From here on the
        // two are the same thing: a bound host capability.
        //
        // A reader that lists `versions` and has not been resolved to one of them for a
        // cluster (`Manifest::at_version`) is not registered, nor is an action on it: no
        // version has been chosen, so nothing may be read or written through it.
        let mut bindings: Vec<(Vec<ValidationError>, Binding)> = manifest
            .capabilities
            .iter()
            .enumerate()
            .filter(|(_, binding)| binding.versions.is_empty())
            .map(|(index, binding)| (self.binding_problems(index, binding), binding.clone()))
            .collect();
        for (index, action) in manifest.actions.iter().enumerate() {
            if unresolved_reader(&manifest, &action.resource).is_some() {
                continue;
            }
            let problems = self.action_problems(index, &manifest, action);
            // A reader that cannot scope the action leaves no binding to
            // check; `action_problems` has already said why.
            let binding = manifest
                .action_binding(action)
                .map_err(|why| format!("actions[{index}]: {why}"))?;
            bindings.push((problems, binding));
        }
        for (problems, binding) in &bindings {
            let id = format!("plugin/{}/{}", manifest.id, binding.name);
            if registry.get(&id).is_some() {
                return Err(format!("capability already registered: {id}"));
            }
            if let Some(problem) = problems.first() {
                return Err(problem.to_string());
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
            let required = Self::required_inputs(&schema);
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
            // Host metadata, raised by nothing a manifest says and lowered by
            // nothing either — including `impact` and the confirmation wording,
            // which a binding could otherwise soften into a shrug. A manifest
            // declares no annotations today, so the binding brings `WEAKEST`;
            // the rule lives in `for_binding` rather than in the absence of a
            // field, so #549 cannot reopen the hole by adding one.
            let annotations =
                Annotations::for_binding(target.annotations, Annotations::WEAKEST);
            capabilities.push(Capability {
                id, summary: format!("{}: {}", manifest.name, binding.title), annotations,
                input_schema: schema, output_schema: target.output_schema.clone(),
                // The binding's arguments were checked against the target's
                // own rule above; a registered binding takes no further
                // arguments, so it carries no rule of its own.
                bound_arguments: None,
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
