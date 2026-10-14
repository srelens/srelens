//! Typed app settings (#542): what a setting can be, and where one may go.
//!
//! An app declares settings in its manifest (`srelens-plugin-host`), a person
//! fills them in on a host form, and the host may interpolate one into a
//! binding argument — but only an argument the capability behind the binding
//! marks as settable, and only a setting whose type that argument accepts.
//! The mark lives here, on [`crate::Capability`], beside the handler that
//! reads the argument, for the reason `bound_arguments` does: the rule for
//! what a position takes belongs to the code that uses it, and the broker
//! reads the same declaration at install and on every request.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The type of one declared setting, as a manifest spells it.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum SettingType {
    /// Free text on one line.
    String,
    /// A JSON number, optionally an integer and bounded.
    Number,
    Boolean,
    /// One of the options the manifest lists.
    Select,
    /// Any of the options the manifest lists, each at most once.
    MultiSelect,
    /// An `http` or `https` URL with no credentials in it.
    Url,
    /// A Kubernetes namespace name.
    NamespaceSelector,
    /// A kubeconfig context, by its key (`ResolvedContext::key`).
    ClusterSelector,
    /// A secret held by the host's secret store (#543). The inventory keeps a
    /// reference, never the value, and no binding argument can take it.
    SecretReference,
}

/// A binding argument a host capability lets a setting fill.
#[derive(Debug, Clone, PartialEq)]
pub struct Settable {
    /// The top-level argument name, as the binding writes it.
    pub argument: String,
    /// The setting types this argument takes.
    pub accepts: Vec<SettingType>,
    /// A value this capability's own `bound_arguments` rule accepts in this
    /// position. When a manifest is checked there is no setting value yet, so
    /// the rest of the binding is checked around this one; the real value is
    /// checked by the same rule when it is saved and on every request.
    pub stand_in: Value,
}

const PREFIX: &str = "${settings.";

/// The setting a binding argument interpolates, when the argument is written
/// as a reference.
///
/// `None` for a value that is not an attempt at one — it is the capability's
/// to judge. A reference is the whole value, `${settings.<id>}` with an id of
/// letters, digits and `-`; anything else that mentions `${settings.` is
/// refused rather than handed on as literal text that reads like a template.
pub fn reference(value: &Value) -> Option<Result<&str, String>> {
    let text = value.as_str()?;
    if !mentions_setting(text) {
        return None;
    }
    let id = text
        .strip_prefix(PREFIX)
        .and_then(|rest| rest.strip_suffix('}'))
        .filter(|id| {
            !id.is_empty()
                && id.len() <= 64
                && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        });
    Some(id.ok_or_else(|| {
        "A setting is interpolated as the whole value, written `${settings.<id>}`".to_owned()
    }))
}

/// Whether `text` holds anything that looks like a setting reference: `${`,
/// then `settings` after any whitespace. Loose on purpose, so a near-miss
/// spelling is refused as a broken reference instead of passing as text.
pub fn mentions_setting(text: &str) -> bool {
    text.match_indices("${")
        .any(|(at, open)| text[at + open.len()..].trim_start().starts_with("settings"))
}
