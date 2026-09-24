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
    pub fn binding_problems(
        &self,
        index: usize,
        manifest: &Manifest,
        binding: &Binding,
    ) -> Vec<ValidationError> {
        self.problems_at(&format!("capabilities[{index}]"), manifest, binding, None)
    }

    /// Every way saving `values` as `manifest`'s settings would break a
    /// binding that interpolates them: each binding and action, with these
    /// values in place, checked by its target's own rule. The same check as a
    /// request makes, run when the value is chosen, so a person hears about a
    /// value a capability refuses in the form rather than on the next click.
    pub fn settings_problems(
        &self,
        manifest: &Manifest,
        values: &Map<String, Value>,
    ) -> Vec<ValidationError> {
        let mut problems = Vec::new();
        for (index, binding) in manifest.capabilities.iter().enumerate() {
            problems.extend(self.problems_at(
                &format!("capabilities[{index}]"),
                manifest,
                binding,
                Some(values),
            ));
        }
        for (index, action) in manifest.actions.iter().enumerate() {
            if let Ok(binding) = manifest.action_binding(action) {
                problems.extend(self.problems_at(
                    &format!("actions[{index}]"),
                    manifest,
                    &binding,
                    Some(values),
                ));
            }
        }
        problems
    }

    /// `why` with every setting value interpolated into `checked` scrubbed out.
    ///
    /// A target's rule tends to quote the value it refuses (`k8s.annotate`
    /// does), and the value there is what a person saved. The scrub is the
    /// audit log's (#555, #660): the values that differ from the binding as
    /// written are the hidden ones, and `redact_error` removes them in every
    /// spelling serde would echo.
    fn scrub_settings(
        why: &str,
        written: &Map<String, Value>,
        checked: &Map<String, Value>,
    ) -> String {
        let used: Map<String, Value> = checked
            .iter()
            .filter(|(key, value)| written.get(*key) != Some(*value))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        if used.is_empty() {
            return why.to_owned();
        }
        let used = Value::Object(used);
        let redacted = srelens_capability::audit::redact(&used, true);
        srelens_capability::audit::redact_error(why, &used, &redacted)
    }

    /// The arguments `binding` sends to `target`, with each
    /// `${settings.<id>}` replaced; or every reason it cannot be, as
    /// `(argument, why)`.
    ///
    /// **The one path a setting takes into a request.** Install, save and
    /// request all come here, so the rules cannot drift apart:
    ///
    /// - the argument must be one `target` marks settable, and the setting
    ///   declared with a type that position accepts;
    /// - with no `values` (install) the position's stand-in is used, so the
    ///   target's own rule can check the rest of the binding around it;
    /// - with `values` (save and request) the saved value, else the default,
    ///   is checked against its declaration first — a stored value is
    ///   re-validated on every request, so an inventory edited by hand gains
    ///   nothing.
    ///
    /// No reason repeats a value.
    pub fn interpolate(
        target: &Capability,
        manifest: &Manifest,
        binding: &Binding,
        values: Option<&Map<String, Value>>,
    ) -> Result<Map<String, Value>, Vec<(String, String)>> {
        let mut arguments = binding.arguments.clone();
        let mut problems = Vec::new();
        for (key, value) in &binding.arguments {
            let id = match srelens_capability::settings::reference(value) {
                None => continue,
                Some(Err(why)) => {
                    problems.push((key.clone(), why));
                    continue;
                }
                Some(Ok(id)) => id,
            };
            let Some(position) = target.settable_argument(key) else {
                problems.push((
                    key.clone(),
                    format!("{} does not let a setting fill `{key}`", target.id),
                ));
                continue;
            };
            let Some(setting) = manifest.setting(id) else {
                problems.push((key.clone(), format!("No setting \"{id}\" is declared")));
                continue;
            };
            if !position.accepts.contains(&setting.setting_type) {
                let accepts: Vec<String> = position
                    .accepts
                    .iter()
                    .filter_map(|kind| serde_json::to_value(kind).ok())
                    .filter_map(|kind| kind.as_str().map(str::to_owned))
                    .collect();
                problems.push((
                    key.clone(),
                    format!(
                        "`{key}` takes a setting of type {}, not setting \"{id}\"",
                        accepts.join(" or ")
                    ),
                ));
                continue;
            }
            let Some(values) = values else {
                arguments.insert(key.clone(), position.stand_in.clone());
                continue;
            };
            match setting.effective(values.get(id)) {
                None => problems.push((
                    key.clone(),
                    format!(
                        "Setting \"{}\" ({id}) needs a value; set it in Settings → Apps",
                        setting.title
                    ),
                )),
                Some(value) => match setting.check_value(value) {
                    Err(why) => problems.push((
                        key.clone(),
                        format!("Setting \"{}\" ({id}): {why}", setting.title),
                    )),
                    Ok(()) => {
                        arguments.insert(key.clone(), value.clone());
                    }
                },
            }
        }
        if problems.is_empty() {
            Ok(arguments)
        } else {
            Err(problems)
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
        match manifest.action_binding(action) {
            // The reader this action names cannot scope it. Reported at
            // `resource`, which is the field that would have to change.
            Err(why) => vec![ValidationError::new(
                ValidationCode::InvalidBinding,
                format!("{at}.resource"),
                why,
            )],
            Ok(binding) => self.problems_at(&at, manifest, &binding, None),
        }
    }

    fn problems_at(
        &self,
        at: &str,
        manifest: &Manifest,
        binding: &Binding,
        values: Option<&Map<String, Value>>,
    ) -> Vec<ValidationError> {
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
        // Settings first: where one may go, of which type, and — when there
        // are values — whether each still fits its declaration (#542).
        let arguments = match Self::interpolate(target, manifest, binding, values) {
            Ok(arguments) => arguments,
            Err(found) => {
                for (key, why) in found {
                    problems.push(
                        ValidationCode::InvalidBinding,
                        format!("{at}.arguments.{key}"),
                        why,
                    );
                }
                return problems.0;
            }
        };
        // The target's own rules for what may be bound to it: the ones a JSON
        // schema cannot state, such as an action primitive's token vocabulary
        // and its deny-list. The handler enforces the same closure, so an app
        // gains nothing by getting a manifest past this. A setting is checked
        // here in its place: as the stand-in at install, as its value on save.
        if let Some(check) = &target.bound_arguments {
            if let Err(why) = check(&arguments) {
                // On save the value in place is the person's, and the rule
                // may quote it back.
                let why = match values {
                    Some(_) => Self::scrub_settings(&why, &binding.arguments, &arguments),
                    None => why,
                };
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

    /// [`PluginHost::register_with_settings`] with no saved settings: every
    /// interpolated setting takes its default, and a required one refuses.
    pub fn register(
        &self,
        registry: &mut Registry,
        manifest: Manifest,
        grants: &[String],
    ) -> Result<Registration, String> {
        self.register_with_settings(registry, manifest, grants, &Map::new())
    }

    /// Registers the manifest's bindings, each filling its `${settings.<id>}`
    /// arguments from `settings` — the app's saved values, read by the caller
    /// with the request — through [`PluginHost::interpolate`] and the
    /// target's own rule, on every call.
    pub fn register_with_settings(
        &self,
        registry: &mut Registry,
        manifest: Manifest,
        grants: &[String],
        settings: &Map<String, Value>,
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
        let declared = Arc::new(manifest.clone());
        let values = Arc::new(settings.clone());
        let mut capabilities = Vec::new();
        // Readers as written, then one binding per declared action, which the
        // manifest builds from the reader it names (#549). From here on the
        // two are the same thing: a bound host capability.
        let mut bindings: Vec<(Vec<ValidationError>, Binding)> = manifest
            .capabilities
            .iter()
            .enumerate()
            .map(|(index, binding)| {
                (
                    self.binding_problems(index, &manifest, binding),
                    binding.clone(),
                )
            })
            .collect();
        for (index, action) in manifest.actions.iter().enumerate() {
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
            let target = Arc::new(target.clone());
            let declared = declared.clone();
            let values = values.clone();
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
                // Its settings are already interpolated, below; nothing more
                // may be.
                settable: Vec::new(),
                handler: Arc::new(move |input| {
                    let handler = handler.clone();
                    let enabled = enabled.clone();
                    let binding = binding.clone();
                    let required = required.clone();
                    let target = target.clone();
                    let declared = declared.clone();
                    let values = values.clone();
                    Box::pin(async move {
                        if !enabled.load(Ordering::SeqCst) { return Err(CapabilityError::Handler("extension is disabled".into())); }
                        let input = input.as_object().ok_or_else(|| CapabilityError::InvalidInput("extension input must be an object".into()))?;
                        if input.keys().any(|k| !binding.inputs.contains(k)) || required.iter().any(|k| !input.contains_key(k)) {
                            return Err(CapabilityError::InvalidInput("unexpected or missing extension input; bound arguments cannot be overridden".into()));
                        }
                        // The app's settings as saved when this request was
                        // made, through the checks install and save ran (#542).
                        let mut args = Self::interpolate(&target, &declared, &binding, Some(&values))
                            .map_err(|problems| CapabilityError::Handler(
                                problems.into_iter().map(|(key, why)| format!("{key}: {why}")).collect::<Vec<_>>().join("; "),
                            ))?;
                        if args != binding.arguments {
                            if let Some(check) = &target.bound_arguments {
                                check(&args).map_err(|why| CapabilityError::Handler(
                                    Self::scrub_settings(&why, &binding.arguments, &args),
                                ))?;
                            }
                        }
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
