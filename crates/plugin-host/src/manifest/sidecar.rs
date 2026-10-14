//! Executable apps (#574): the sidecar a manifest of kind `executable` runs,
//! and the operations it answers.
//!
//! An executable manifest names one binary for each platform it ships for, a
//! file under `bin/<platform>/` in its package, and declares every operation
//! the sidecar serves: a name, a title and typed inputs. The host builds each
//! operation's input schema from those declarations, never from a schema the
//! app supplies, and holds every call to it before the sidecar sees the call.
//! An operation is reached as `plugin/<id>/<operation>`, like a binding.
//!
//! The sidecar runs under the supervisor (#572) in the OS sandbox, with no
//! kubeconfig, no network and one writable directory. It reaches the host only
//! through the broker (#573): its app's readers, and its app's declared actions,
//! each confirmed by a person. What an operation runs under follows from that:
//! see [`sidecar_operation_annotations`].
use super::{identifier, label, unique, Manifest, ManifestKind};
use crate::{ValidationCode as Code, ValidationErrors};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use srelens_capability::Annotations;
use std::collections::BTreeMap;

const LABEL: &str =
    "Must be 1–120 characters with no control characters and no bidirectional or invisible format characters";
const IDENTIFIER: &str = "Must be 1–64 letters, digits and -";

/// The platforms a sidecar may ship a binary for: the `bin/<platform>/`
/// directories a package may hold.
pub const SIDECAR_PLATFORMS: [&str; 5] = [
    "darwin-arm64",
    "darwin-amd64",
    "linux-amd64",
    "linux-arm64",
    "windows-amd64",
];
/// Most operations one sidecar may declare.
pub const MAX_OPERATIONS: usize = 32;
/// Most inputs one operation may declare.
pub const MAX_OPERATION_INPUTS: usize = 16;
/// The longest a string input may be, in bytes, when its declaration names no
/// `maxLength`.
pub const DEFAULT_INPUT_BYTES: u32 = 1024;
/// The largest `maxLength` a string input may declare, in bytes.
pub const MAX_INPUT_BYTES: u32 = 64 * 1024;
/// The largest input one operation call may carry, as compact JSON. Every
/// field has its own limit too; this bounds them together.
pub const MAX_OPERATION_CALL_BYTES: usize = 256 * 1024;

/// What a sidecar operation of an app that declares no actions runs under:
/// host metadata, which nothing in a manifest raises or lowers.
///
/// - **Read-only.** A sidecar has no kubeconfig, no network and no path but
///   its own data directory. Through the broker (#573) it can read what its
///   app's readers read and run its app's declared actions, and an app with
///   none can change nothing outside the sandbox. So its operations are not
///   consent-gated, like a declarative reader.
/// - **Sensitive.** Its inputs are named by the app, so the audit log cannot
///   tell which of them hold a credential; it redacts every argument instead.
pub const SIDECAR_OPERATION: Annotations = Annotations {
    sensitive: true,
    ..Annotations::READ_ONLY
};

/// The host's sentence for an operation of an app that declares actions: what
/// is asked, and that every change it leads to is asked about again.
pub const SIDECAR_OPERATION_CONFIRM: &str =
    "Let this app's sidecar run its operation? It may ask to run the app's declared actions, and each one is confirmed on its own.";

/// What a sidecar operation runs under, given the host rows of the actions its
/// app declares (`actions`).
///
/// With none, [`SIDECAR_OPERATION`]. With any, the sidecar may ask the broker
/// to run them, so the operation is not a read: it inherits the strongest of
/// their rows through [`Annotations::for_binding`] — gated, at least their
/// impact, destructive if one is — and the host's own
/// [`SIDECAR_OPERATION_CONFIRM`] in place of any one primitive's sentence,
/// which would describe a write the operation may never make. Each write it
/// then asks for is confirmed again by the broker's consent, with the app
/// named.
pub fn sidecar_operation_annotations(
    actions: impl IntoIterator<Item = Annotations>,
) -> Annotations {
    let mut actions = actions.into_iter().peekable();
    if actions.peek().is_none() {
        return SIDECAR_OPERATION;
    }
    let reach = actions.fold(Annotations::WEAKEST, Annotations::at_least);
    let mut row = Annotations::for_binding(reach, SIDECAR_OPERATION);
    row.confirm = Some(SIDECAR_OPERATION_CONFIRM);
    row
}

/// The sidecar an executable app runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Sidecar {
    /// The binary the host runs on each platform, by platform: a file directly
    /// under `bin/<platform>/` in the app's package, such as
    /// `"linux-amd64": "bin/linux-amd64/scanner"`. A platform left out does not
    /// run the app.
    pub binaries: BTreeMap<String, String>,
    /// Every operation the sidecar answers. Each is a request of that name,
    /// with the checked input as its params.
    pub operations: Vec<Operation>,
}

/// One request a sidecar answers, and the inputs it takes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Operation {
    pub name: String,
    pub title: String,
    /// What a caller passes, each checked by the host before the sidecar is
    /// asked. An operation that takes nothing leaves this out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<OperationInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<OperationView>,
}

/// Native host rendering choices. Apps supply data, never executable frontend code.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationView {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub auto_run: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stream: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
}

/// One input an operation takes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OperationInput {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(rename = "type")]
    pub input_type: InputType,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub required: bool,
    /// For a string: the most bytes it may hold, 1–65536. Default 1024.
    #[serde(default, rename = "maxLength", skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum InputType {
    String,
    Integer,
    Number,
    Boolean,
}

impl InputType {
    fn as_str(self) -> &'static str {
        match self {
            InputType::String => "string",
            InputType::Integer => "integer",
            InputType::Number => "number",
            InputType::Boolean => "boolean",
        }
    }
}

/// The platform this host runs on, as `bin/` names it, or `None` for one no
/// sidecar can ship for.
pub fn host_platform() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("darwin-arm64"),
        ("macos", "x86_64") => Some("darwin-amd64"),
        ("linux", "x86_64") => Some("linux-amd64"),
        ("linux", "aarch64") => Some("linux-arm64"),
        ("windows", "x86_64") => Some("windows-amd64"),
        _ => None,
    }
}

impl OperationInput {
    /// The most bytes a string input may hold.
    fn limit(&self) -> usize {
        self.max_length.unwrap_or(DEFAULT_INPUT_BYTES) as usize
    }
}

impl Operation {
    /// The input schema the host publishes for this operation, built from its
    /// declared inputs and nothing else.
    pub fn input_schema(&self) -> Value {
        let properties: Map<String, Value> = self
            .inputs
            .iter()
            .map(|input| {
                let mut property = json!({ "type": input.input_type.as_str() });
                if input.input_type == InputType::String {
                    property["maxLength"] = json!(input.limit());
                }
                if let Some(title) = &input.title {
                    property["title"] = json!(title);
                }
                (input.name.clone(), property)
            })
            .collect();
        let required: Vec<&str> = self
            .inputs
            .iter()
            .filter(|input| input.required)
            .map(|input| input.name.as_str())
            .collect();
        json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false,
        })
    }

    /// `input` as the sidecar is sent it, or why it is refused: not an object,
    /// a field the operation does not take, a required one missing, a value of
    /// the wrong type, or a string or the whole call over its limit. Each
    /// refusal names the field and its limit, and quotes no value.
    pub fn check_input(&self, input: &Value) -> Result<Map<String, Value>, String> {
        let Value::Object(fields) = input else {
            return Err(format!("{} takes an object", self.name));
        };
        let size = serde_json::to_vec(input).map_err(|e| e.to_string())?.len();
        if size > MAX_OPERATION_CALL_BYTES {
            return Err(format!(
                "The input to {} exceeds {} KiB ({MAX_OPERATION_CALL_BYTES} bytes)",
                self.name,
                MAX_OPERATION_CALL_BYTES / 1024
            ));
        }
        for key in fields.keys() {
            if !self.inputs.iter().any(|input| &input.name == key) {
                return Err(format!("{} takes no input `{key}`", self.name));
            }
        }
        for input in &self.inputs {
            let name = &input.name;
            let Some(value) = fields.get(name) else {
                if input.required {
                    return Err(format!("{} requires `{name}`", self.name));
                }
                continue;
            };
            let fits = match input.input_type {
                InputType::String => value.is_string(),
                InputType::Integer => value.is_i64() || value.is_u64(),
                InputType::Number => value.is_number(),
                InputType::Boolean => value.is_boolean(),
            };
            if !fits {
                return Err(format!("`{name}` must be a {}", input.input_type.as_str()));
            }
            if let Some(text) = value.as_str() {
                if text.len() > input.limit() {
                    return Err(format!("`{name}` is at most {} bytes", input.limit()));
                }
            }
        }
        Ok(fields.clone())
    }
}

impl Manifest {
    /// The operation `name` this app's sidecar declares, if it runs one.
    pub fn operation(&self, name: &str) -> Option<&Operation> {
        self.sidecar
            .as_ref()?
            .operations
            .iter()
            .find(|operation| operation.name == name)
    }

    /// The package path of the binary this app runs on `platform`, if it ships one.
    pub fn sidecar_binary(&self, platform: &str) -> Option<&str> {
        self.sidecar
            .as_ref()?
            .binaries
            .get(platform)
            .map(String::as_str)
    }
}

/// Whether `path` is a file directly under `bin/<platform>/`: one more
/// segment of letters, digits, `.`, `_` and `-`, not starting or ending with a
/// dot. The package reader holds the same path to its own, stricter rules.
fn binary_path(platform: &str, path: &str) -> bool {
    let Some(file) = path
        .strip_prefix("bin/")
        .and_then(|rest| rest.strip_prefix(platform))
        .and_then(|rest| rest.strip_prefix('/'))
    else {
        return false;
    };
    !file.is_empty()
        && file.len() <= 64
        && !file.starts_with('.')
        && !file.ends_with('.')
        && file
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// `sidecar` (#574): present exactly when the kind is `executable`, a binary
/// for each platform it names, and operations whose names share the one name
/// space `plugin/<id>/<name>` gives capabilities and actions.
pub(super) fn sidecar_problems(manifest: &Manifest, problems: &mut ValidationErrors) {
    let sidecar = match (&manifest.kind, &manifest.sidecar) {
        (ManifestKind::Declarative, None) => return,
        (ManifestKind::Declarative, Some(_)) => {
            problems.push(
                Code::InvalidKind,
                "sidecar",
                "Only an executable app runs a sidecar: set kind to \"executable\"",
            );
            return;
        }
        (ManifestKind::Executable, None) => {
            problems.push(
                Code::InvalidKind,
                "kind",
                "An executable app declares the sidecar it runs in `sidecar`",
            );
            return;
        }
        (ManifestKind::Executable, Some(sidecar)) => sidecar,
    };
    if sidecar.binaries.is_empty() {
        problems.push(
            Code::InvalidValue,
            "sidecar.binaries",
            "Name a binary for at least one platform",
        );
    }
    for (platform, path) in &sidecar.binaries {
        let at = format!("sidecar.binaries.{platform}");
        if !SIDECAR_PLATFORMS.contains(&platform.as_str()) {
            problems.push(
                Code::InvalidValue,
                at,
                format!(
                    "\"{platform}\" is not a platform; use one of {}",
                    SIDECAR_PLATFORMS.join(", ")
                ),
            );
        } else if !binary_path(platform, path) {
            problems.push(
                Code::InvalidValue,
                at,
                format!(
                    "A binary is a file directly under bin/{platform}/: letters, digits, '.', '_' and '-', not starting or ending with '.'"
                ),
            );
        }
    }
    if sidecar.operations.is_empty() || sidecar.operations.len() > MAX_OPERATIONS {
        problems.push(
            Code::InvalidValue,
            "sidecar.operations",
            format!("Declare 1–{MAX_OPERATIONS} operations"),
        );
    }
    // An operation's name becomes a tool id beside the readers' and actions'.
    let mut declared: std::collections::BTreeSet<&str> = manifest
        .capabilities
        .iter()
        .map(|binding| binding.name.as_str())
        .chain(manifest.actions.iter().map(|action| action.name.as_str()))
        .collect();
    for (index, operation) in sidecar.operations.iter().enumerate() {
        let at = format!("sidecar.operations[{index}]");
        if operation.view.as_ref().is_some_and(|view| view.auto_run && view.stream) {
            problems.push(Code::InvalidValue, format!("{at}.view"), "A streaming operation requires an explicit Run");
        }
        if !identifier(&operation.name) {
            problems.push(Code::InvalidValue, format!("{at}.name"), IDENTIFIER);
        } else if crate::sidecar::protocol::is_reserved(&operation.name) {
            problems.push(
                Code::InvalidValue,
                format!("{at}.name"),
                format!(
                    "\"{}\" is one of srelens's own sidecar methods",
                    operation.name
                ),
            );
        } else if !declared.insert(operation.name.as_str()) {
            problems.push(
                Code::DuplicateIdentifier,
                format!("{at}.name"),
                format!(
                    "\"{}\" is already used by another capability",
                    operation.name
                ),
            );
        }
        if !label(&operation.title) {
            problems.push(Code::InvalidValue, format!("{at}.title"), LABEL);
        }
        if operation.inputs.len() > MAX_OPERATION_INPUTS {
            problems.push(
                Code::InvalidValue,
                format!("{at}.inputs"),
                format!("Declare at most {MAX_OPERATION_INPUTS} inputs"),
            );
        }
        unique(
            problems,
            operation
                .inputs
                .iter()
                .enumerate()
                .map(|(position, input)| {
                    (format!("{at}.inputs[{position}].name"), input.name.as_str())
                }),
        );
        for (position, input) in operation.inputs.iter().enumerate() {
            let at = format!("{at}.inputs[{position}]");
            if !identifier(&input.name) {
                problems.push(Code::InvalidValue, format!("{at}.name"), IDENTIFIER);
            }
            if input.title.as_deref().is_some_and(|title| !label(title)) {
                problems.push(Code::InvalidValue, format!("{at}.title"), LABEL);
            }
            match (input.input_type, input.max_length) {
                (InputType::String, Some(0)) => problems.push(
                    Code::InvalidValue,
                    format!("{at}.maxLength"),
                    format!("maxLength is 1–{MAX_INPUT_BYTES} bytes"),
                ),
                (InputType::String, Some(length)) if length > MAX_INPUT_BYTES => problems.push(
                    Code::InvalidValue,
                    format!("{at}.maxLength"),
                    format!("maxLength is 1–{MAX_INPUT_BYTES} bytes"),
                ),
                (InputType::String, _) | (_, None) => {}
                (_, Some(_)) => problems.push(
                    Code::InvalidValue,
                    format!("{at}.maxLength"),
                    "Only a string input has a maxLength",
                ),
            }
        }
    }
}
