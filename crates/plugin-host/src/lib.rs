//! Native extension contract and declarative capability broker.
//!
//! No package code is loaded into the host. A trusted installer supplies
//! explicit grants for native srelens manifests. Executable apps (#574) run
//! out of process, sandboxed, under [`sidecar::Supervisor`] (#572).
pub mod app_log;
#[cfg(any(test, feature = "fuzzing"))]
#[doc(hidden)]
pub mod fuzzing;
mod manifest;
mod secrets;
pub mod sidecar;
mod validation;
pub use manifest::*;
pub use secrets::*;
pub use validation::*;

use serde_json::{json, Map, Value};
use srelens_capability::{Annotations, BoxFuture, Capability, CapabilityError, Registry};
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

/// `binding` with `version` bound to its first listed one, when it lists `versions` and
/// fixes none: how the host checks such a reader against its target (#547).
fn at_first_version(binding: &Binding) -> Binding {
    let mut bound = binding.clone();
    if let Some(first) = binding.versions.first() {
        bound
            .arguments
            .entry("version")
            .or_insert_with(|| Value::String(first.clone()));
    }
    bound
}

/// `manifest` read at the first version of `reader`, when that reader lists versions:
/// how an action on it is checked before any cluster has chosen one (#547).
fn first_version_of(manifest: &Manifest, reader: &str) -> Option<Manifest> {
    unresolved_reader(manifest, reader)
        .and_then(|binding| binding.versions.first())
        .and_then(|first| manifest.at_version(reader, first).ok())
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
    /// The ids this registration added, `plugin/<id>/<name>`.
    pub fn ids(&self) -> &[String] {
        &self.ids
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
    /// [`Registration::unregister`] for every snapshot at once, including ones
    /// that are shared and cannot be changed: each keeps its handlers, and none
    /// of them can be invoked again.
    pub fn revoke(&self) {
        self.active.store(false, Ordering::SeqCst);
    }
}

/// How the host runs one of an app's tools: given the operation's name and
/// the checked input, the broker's own path for it (#574), with every check a
/// call through that path makes.
pub type ToolRoute =
    Arc<dyn Fn(String, Map<String, Value>) -> BoxFuture<Result<Value, CapabilityError>> + Send + Sync>;

/// The longest a string a caller passes to a binding's or an action's tool may
/// be, in bytes (#610): longer than any context, namespace, name, UID or
/// resourceVersion Kubernetes gives out. A sidecar operation's inputs carry
/// their own limits.
pub const MAX_TOOL_STRING_BYTES: usize = 1024;

/// What a reader's tool takes: the cluster it is read in, and the namespace
/// when the binding takes one. What the broker reads a reader with, and
/// nothing a caller could use to widen it.
const READER_TOOL_INPUTS: &[&str] = &["context", "namespace"];

/// One of an app's tools, before it is registered.
struct Tool {
    name: String,
    title: String,
    schema: Value,
    output: Value,
    annotations: Annotations,
    inputs: ToolInputs,
}

/// What one registered tool takes.
#[derive(Clone)]
enum ToolInputs {
    /// Strings, each at most [`MAX_TOOL_STRING_BYTES`]: a reader's or an action's.
    Strings { names: Vec<String>, required: Vec<String> },
    /// A sidecar operation's declared inputs.
    Operation(Operation),
}

impl ToolInputs {
    fn check(&self, name: &str, input: Value) -> Result<Map<String, Value>, CapabilityError> {
        let invalid = CapabilityError::InvalidInput;
        match self {
            ToolInputs::Operation(operation) => operation.check_input(&input).map_err(invalid),
            ToolInputs::Strings { names, required } => {
                let Value::Object(fields) = input else {
                    return Err(invalid(format!("{name} takes an object")));
                };
                for (key, value) in &fields {
                    if !names.contains(key) {
                        return Err(invalid(format!("{name} takes no input `{key}`")));
                    }
                    let Some(text) = value.as_str() else {
                        return Err(invalid(format!("`{key}` must be a string")));
                    };
                    if text.len() > MAX_TOOL_STRING_BYTES {
                        return Err(invalid(format!(
                            "`{key}` is at most {MAX_TOOL_STRING_BYTES} bytes"
                        )));
                    }
                }
                if let Some(missing) = required.iter().find(|key| !fields.contains_key(*key)) {
                    return Err(invalid(format!("{name} requires `{missing}`")));
                }
                Ok(fields)
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
    pub fn binding_problems(
        &self,
        index: usize,
        manifest: &Manifest,
        binding: &Binding,
    ) -> Vec<ValidationError> {
        let bound = at_first_version(binding);
        self.problems_at(&format!("capabilities[{index}]"), manifest, &bound, None)
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
                &at_first_version(binding),
                Some(values),
            ));
        }
        for (index, action) in manifest.actions.iter().enumerate() {
            // An action on a reader that lists versions is checked as install checks
            // it, at the first; its settable arguments do not depend on the version.
            let resolved = first_version_of(manifest, &action.resource);
            if let Ok(binding) = resolved.as_ref().unwrap_or(manifest).action_binding(action) {
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
        // An action on a reader that lists versions acts at whichever one resolved;
        // checked, like the reader, at the first.
        let resolved = first_version_of(manifest, &action.resource);
        match resolved.as_ref().unwrap_or(manifest).action_binding(action) {
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

    /// `target`'s input schema narrowed to `inputs`, and which of them it
    /// requires: the host's schema, never one an app supplies.
    fn exposed_schema(
        target: &Capability,
        inputs: &[String],
    ) -> Result<(Value, Vec<String>), String> {
        let mut schema = target.input_schema.clone();
        let properties = schema
            .get("properties")
            .and_then(Value::as_object)
            .ok_or("host capability needs an object input schema")?;
        let exposed: Map<String, Value> = inputs
            .iter()
            .map(|k| {
                let property = properties.get(k).cloned().unwrap_or(json!({"type": "string"}));
                (k.clone(), property)
            })
            .collect();
        let required: Vec<String> = Self::required_inputs(&schema)
            .into_iter()
            .filter(|k| inputs.contains(k))
            .collect();
        schema["properties"] = Value::Object(exposed);
        schema["required"] = json!(required);
        schema["additionalProperties"] = Value::Bool(false);
        Ok((schema, required))
    }

    /// Registers each of the manifest's operations as the tool
    /// `plugin/<id>/<name>` (#574): its readers, its declared actions, and the
    /// operations its sidecar answers. A pod binding is left out: it is a
    /// session a view opens as a stream, not a call.
    ///
    /// Everything about a tool is the host's. A reader takes the cluster and,
    /// when it takes one, the namespace, described by its target's own schema;
    /// an action takes [`ACTION_INPUTS`]; both run under their target's
    /// annotations through [`Annotations::for_binding`], so a manifest cannot
    /// make a write look like a read. A sidecar operation's schema is built
    /// from its declared inputs, and it runs under
    /// [`sidecar_operation_annotations`] of the app's declared actions, which
    /// its sidecar may ask the broker to run.
    ///
    /// A call is checked here first — only the inputs the tool takes, each of
    /// the right type and within its limit, the required ones present — and
    /// then handed to `route` with the operation's name: the broker's own path
    /// for it, which checks the inventory again on every call.
    ///
    /// Nothing is registered unless every tool is: the app's grants must cover
    /// its permissions, and each binding and action must fit its target, as
    /// installing it checked. [`Registration::revoke`] then withdraws every
    /// tool at once, from every snapshot holding them.
    pub fn register_tools(
        &self,
        registry: &mut Registry,
        manifest: &Manifest,
        grants: &[String],
        route: ToolRoute,
    ) -> Result<Registration, String> {
        manifest.validate()?;
        if manifest
            .permissions
            .iter()
            .any(|p| !grants.iter().any(|grant| p == grant.as_str()))
        {
            return Err("extension permissions have not been granted".into());
        }
        let host = |id: &str| {
            self.core
                .get(id)
                .ok_or_else(|| format!("unknown host capability: {id}"))
        };
        // Every tool, checked before any is registered.
        let mut tools: Vec<Tool> = Vec::new();
        // What the app's sidecar may ask the broker to run: its declared actions.
        let mut writes = Vec::new();
        for (index, binding) in manifest.capabilities.iter().enumerate() {
            if is_pod_target(&binding.target) || binding.target == "k8s.runJob" {
                continue;
            }
            if let Some(problem) = self.binding_problems(index, manifest, binding).first() {
                return Err(problem.to_string());
            }
            let target = host(&binding.target)?;
            let names: Vec<String> = READER_TOOL_INPUTS
                .iter()
                .filter(|input| **input == "context" || binding.inputs.iter().any(|i| i == *input))
                .map(|input| (*input).to_owned())
                .collect();
            let (mut schema, _) = Self::exposed_schema(target, &names)?;
            // Every read is in one explicit cluster; the broker refuses one without it.
            schema["required"] = json!(["context"]);
            let annotations = Annotations::for_binding(target.annotations, Annotations::WEAKEST);
            let inputs = ToolInputs::Strings {
                names,
                required: vec!["context".into()],
            };
            tools.push(Tool {
                name: binding.name.clone(),
                title: binding.title.clone(),
                schema,
                output: target.output_schema.clone(),
                annotations,
                inputs,
            });
        }
        for (index, action) in manifest.actions.iter().enumerate() {
            if let Some(problem) = self.action_problems(index, manifest, action).first() {
                return Err(problem.to_string());
            }
            // An action on a reader that lists versions writes whichever one a cluster
            // resolves to; its inputs and its primitive are the same at every one.
            let resolved = first_version_of(manifest, &action.resource);
            let binding = resolved
                .as_ref()
                .unwrap_or(manifest)
                .action_binding(action)
                .map_err(|why| format!("actions[{index}]: {why}"))?;
            let target = host(&binding.target)?;
            let (schema, _) = Self::exposed_schema(target, &binding.inputs)?;
            let mut schema = schema;
            // What the broker needs to name the object and the version reviewed; the
            // namespace is empty for a cluster-scoped kind.
            let required: Vec<String> = ["context", "name", "uid", "resourceVersion"]
                .iter()
                .map(|key| (*key).to_owned())
                .collect();
            schema["required"] = json!(required);
            let annotations = Annotations::for_binding(target.annotations, Annotations::WEAKEST);
            writes.push(target.annotations);
            let inputs = ToolInputs::Strings {
                names: binding.inputs.clone(),
                required,
            };
            tools.push(Tool {
                name: action.name.clone(),
                title: action.title.clone(),
                schema,
                output: target.output_schema.clone(),
                annotations,
                inputs,
            });
        }
        let operation_row = sidecar_operation_annotations(writes);
        for operation in manifest.sidecar.iter().flat_map(|sidecar| &sidecar.operations)
            .filter(|operation| !operation.view.as_ref().is_some_and(|view| view.stream))
        {
            tools.push(Tool {
                name: operation.name.clone(),
                title: operation.title.clone(),
                schema: operation.input_schema(),
                // What a sidecar answers is its own.
                output: Value::Null,
                annotations: operation_row,
                inputs: ToolInputs::Operation(operation.clone()),
            });
        }
        for Tool { name, .. } in &tools {
            let id = format!("plugin/{}/{name}", manifest.id);
            if registry.get(&id).is_some() {
                return Err(format!("capability already registered: {id}"));
            }
        }
        let active = Arc::new(AtomicBool::new(true));
        let mut ids = Vec::new();
        for Tool {
            name,
            title,
            schema,
            output,
            annotations,
            inputs,
        } in tools
        {
            let id = format!("plugin/{}/{name}", manifest.id);
            let enabled = active.clone();
            let route = route.clone();
            ids.push(id.clone());
            registry.register(Capability {
                id,
                summary: format!("{}: {title}", manifest.name),
                annotations,
                input_schema: schema,
                output_schema: output,
                bound_arguments: None,
                settable: Vec::new(),
                secret_slots: Vec::new(),
                // An app's tool is an MCP tool by definition.
                ui_only: false,
                handler: Arc::new(move |input| {
                    let enabled = enabled.clone();
                    let route = route.clone();
                    let inputs = inputs.clone();
                    let name = name.clone();
                    Box::pin(async move {
                        if !enabled.load(Ordering::SeqCst) {
                            return Err(CapabilityError::Handler(
                                "This tool was withdrawn when its app was disabled, updated or removed; list the tools again".into(),
                            ));
                        }
                        let input = inputs.check(&name, input)?;
                        route(name, input).await
                    })
                }),
            });
        }
        Ok(Registration {
            manifest: manifest.clone(),
            active,
            ids,
        })
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
        if manifest
            .permissions
            .iter()
            .any(|p| !grants.iter().any(|grant| p == grant.as_str()))
        {
            return Err("extension permissions have not been granted".into());
        }
        let active = Arc::new(AtomicBool::new(true));
        let declared = Arc::new(manifest.clone());
        let values = Arc::new(settings.clone());
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
            .map(|(index, binding)| {
                (
                    self.binding_problems(index, &manifest, binding),
                    binding.clone(),
                )
            })
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
            let (schema, required) = Self::exposed_schema(target, &binding.inputs)?;
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
                // A registered binding is the app's; the host injects a
                // secret into the target it calls, never into this facade.
                secret_slots: Vec::new(),
                // An app's own reader is a tool like its target.
                ui_only: false,
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
