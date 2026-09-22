//! Host action primitives: the bounded writes an extension manifest may bind.
//!
//! An extension never sends a Kubernetes request. It declares an action that
//! names one of the primitives here, and the host builds the request: the
//! object's identity comes from a reader binding the same manifest already
//! holds a grant for, the surface supplies the object the operator reviewed,
//! and everything else is fixed in the manifest at install time.
//!
//! Every primitive does the same three things, because each one is a rule that
//! was written once for `k8s.gitOpsAction` (#511) and must not be re-derived
//! per primitive:
//!
//! - a fresh GET, then a PATCH pinned to the reviewed `uid` and
//!   `resourceVersion` through [`pin_to_reviewed`], so a write cannot land on a
//!   replacement created under the same name nor on a version nobody saw;
//! - a refusal to write to an object that is being deleted;
//! - `{"requested": true}` — the API server accepted the request. Whether the
//!   controller does what the request asks is not something this host saw, and
//!   it does not say it did.

use crate::{
    client_cache::ClientCache,
    connect::request_timeout,
    gitops::{pin_to_reviewed, ResourceIn},
};
use chrono::{SecondsFormat, Utc};
use kube::{
    api::{Patch, PatchParams},
    Client,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use srelens_capability::{
    check_predicates, unmet, Annotations, Capability, CapabilityError, Impact, Predicate,
};
use std::sync::Arc;

/// Writes one fixed annotation key. Covers Flux's reconcile, force and reset
/// requests and Argo CD's refresh.
pub const ANNOTATE: &str = "k8s.annotate";

/// Sets fixed fields under `spec`. Covers suspend and resume.
pub const SET_FIELDS: &str = "k8s.setFields";

/// Writes one condition through the status subresource. Covers cert-manager's
/// Renew, which sets `Issuing=True` with reason `ManuallyTriggered`.
pub const SET_STATUS_CONDITION: &str = "k8s.setStatusCondition";

/// Sends one fixed merge-patch template, past a host deny-list. Covers Argo
/// CD's `operation.sync`, which no narrower primitive can express.
pub const MERGE_PATCH: &str = "k8s.mergePatch";

/// Every host action primitive, in the order they are documented.
pub const PRIMITIVES: &[&str] = &[ANNOTATE, SET_FIELDS, SET_STATUS_CONDITION, MERGE_PATCH];

/// The value tokens a binding may write instead of a literal.
const NOW: &str = "$now";
const UUID: &str = "$uuid";

/// The longest literal a binding may write. Kubernetes bounds an object's
/// annotations to 256 KiB in total; a fixed value in a manifest that needs
/// more than this is not a value, it is a payload.
const MAX_VALUE_CHARS: usize = 1024;

macro_rules! action_input {
    ($(#[$outer:meta])* $name:ident { $($(#[$inner:meta])* $field:ident : $ty:ty),* $(,)? }) => {
        $(#[$outer])*
        ///
        /// The identity fields are bound by the host from the manifest's
        /// reader binding, never written by the app; `context`, `namespace`,
        /// `name`, `uid` and `resourceVersion` are the inputs the reviewing
        /// surface supplies.
        #[derive(Debug, Deserialize, JsonSchema)]
        #[serde(deny_unknown_fields)]
        pub struct $name {
            pub context: String,
            pub group: String,
            pub version: String,
            pub plural: String,
            pub kind: String,
            pub namespaced: bool,
            /// Empty for a cluster-scoped kind.
            #[serde(default)]
            pub namespace: String,
            pub name: String,
            /// The UID of the object the operator reviewed.
            pub uid: String,
            /// The `resourceVersion` of the object the operator reviewed.
            #[serde(rename = "resourceVersion")]
            pub resource_version: String,
            /// What the manifest declared must be true of the object before
            /// this write is sent (#550). Bound by the host out of the
            /// action's own `preconditions`, never supplied by a caller.
            #[serde(default)]
            pub preconditions: Vec<Predicate>,
            $($(#[$inner])* pub $field: $ty,)*
        }
        impl $name {
            fn reviewed(&self) -> Reviewed<'_> {
                Reviewed {
                    resource: ResourceIn {
                        context: self.context.clone(),
                        group: self.group.clone(),
                        version: self.version.clone(),
                        plural: self.plural.clone(),
                        kind: self.kind.clone(),
                        namespaced: self.namespaced,
                        namespace: self.namespace.clone(),
                        name: self.name.clone(),
                    },
                    uid: &self.uid,
                    resource_version: &self.resource_version,
                }
            }
        }
    };
}

action_input!(
    /// `k8s.annotate` — write one fixed annotation key.
    AnnotateIn {
        /// The annotation key, fixed by the manifest.
        key: String,
        /// `$now`, `$uuid`, or a literal.
        value: String,
    }
);

action_input!(
    /// `k8s.setFields` — set fixed JSON pointers under `spec` to fixed values.
    SetFieldsIn {
        /// RFC 6901 pointers under `/spec`, each mapped to the value to write.
        fields: Map<String, Value>,
    }
);

action_input!(
    /// `k8s.setStatusCondition` — write one condition, through the status
    /// subresource.
    SetStatusConditionIn {
        /// The condition's `type`, e.g. `Issuing`.
        #[serde(rename = "conditionType")]
        condition_type: String,
        /// `True`, `False` or `Unknown`.
        #[serde(rename = "conditionStatus")]
        condition_status: String,
        /// The condition's `reason`, in CamelCase.
        reason: String,
        /// The condition's `message`. Optional; a condition carries one even
        /// when it is empty, because `metav1.Condition` requires the field.
        #[serde(default)]
        message: String,
    }
);

action_input!(
    /// `k8s.mergePatch` — send one fixed merge-patch template.
    MergePatchIn {
        /// The template, as a JSON merge patch.
        patch: Map<String, Value>,
    }
);

/// The object a request is pinned to: its identity, and the version the
/// operator reviewed.
struct Reviewed<'a> {
    resource: ResourceIn,
    uid: &'a str,
    resource_version: &'a str,
}

/// Refuses a request the operator's review no longer describes.
///
/// The two refusals every write in this host makes, wherever it comes from:
/// `gitops::guard_action` calls this before its action-specific checks. A UID
/// or `resourceVersion` that has moved means the object on the server is not
/// the object that was reviewed, and an object with a `deletionTimestamp` is
/// on its way out — a write to it either does nothing or overwrites what a
/// finalizer is in the middle of.
pub(crate) fn guard_reviewed(
    current: &Value,
    uid: &str,
    resource_version: &str,
) -> Result<(), String> {
    if uid.is_empty()
        || resource_version.is_empty()
        || current["metadata"]["uid"] != uid
        || current["metadata"]["resourceVersion"] != resource_version
    {
        return Err("Resource changed or was replaced; refresh and review the action again".into());
    }
    if !current["metadata"]["deletionTimestamp"].is_null() {
        return Err("Resource is being deleted".into());
    }
    Ok(())
}

/// What the operator is told when a declared precondition does not hold.
///
/// The host's words first, the app's after: `reason` is a manifest's text,
/// escaped by [`Predicate::reason`], and a refusal has to read as a refusal
/// whatever sentence an app put in it.
fn unmet_reason(predicate: &Predicate) -> String {
    format!("This action is not available: {}", predicate.reason())
}

/// GET the object, refuse anything the review no longer covers or the declared
/// preconditions do not admit, then send the patch `build` makes from what the
/// server actually holds.
///
/// The order is the point. The host's guards run first and unconditionally, so
/// no manifest can reach past them; the declared preconditions run next and
/// can only add refusals; the patch is built last, from the same fresh read
/// every check was made against.
///
/// `status` sends it to the status subresource instead of the object itself.
async fn request(
    client: Client,
    reviewed: Reviewed<'_>,
    preconditions: &[Predicate],
    status: bool,
    build: impl FnOnce(&Value) -> Result<Value, String>,
) -> Result<Value, String> {
    reviewed.resource.validate()?;
    let api = reviewed.resource.api(client);
    let current = serde_json::to_value(
        api.get(&reviewed.resource.name)
            .await
            .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    guard_reviewed(&current, reviewed.uid, reviewed.resource_version)?;
    if let Some(unheld) = unmet(preconditions, &current) {
        return Err(unmet_reason(unheld));
    }
    let mut patch = build(&current)?;
    pin_to_reviewed(&mut patch, reviewed.uid, reviewed.resource_version);
    let params = PatchParams::default();
    let name = &reviewed.resource.name;
    if status {
        api.patch_status(name, &params, &Patch::Merge(&patch)).await
    } else {
        api.patch(name, &params, &Patch::Merge(&patch)).await
    }
    .map_err(|e| e.to_string())?;
    // Accepted, not done. See the module docs.
    Ok(json!({"requested": true}))
}

/// A DNS subdomain as Kubernetes spells one: `example.io`, `a-b.c`.
fn subdomain(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 253
        && value.split('.').all(|part| {
            !part.is_empty()
                && part.len() <= 63
                && part
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
                && !part.starts_with('-')
                && !part.ends_with('-')
        })
}

/// Rejects anything that is not a Kubernetes annotation key: an optional DNS
/// subdomain prefix, a `/`, and a name of at most 63 characters.
fn check_annotation_key(key: &str) -> Result<(), String> {
    let name = match key.split_once('/') {
        Some((prefix, name)) if subdomain(prefix) => name,
        Some(_) => {
            return Err(format!(
                "`{key}` does not start with a DNS subdomain prefix"
            ))
        }
        None => key,
    };
    let valid = !name.is_empty()
        && name.len() <= 63
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
        && name.starts_with(|c: char| c.is_ascii_alphanumeric())
        && name.ends_with(|c: char| c.is_ascii_alphanumeric());
    if valid {
        Ok(())
    } else {
        Err(format!("`{key}` is not a Kubernetes annotation key"))
    }
}

/// The value a bound `value` writes for this request.
///
/// The vocabulary is closed on purpose. A `$`-prefixed string this host does
/// not know is refused rather than written through as a literal: an app that
/// asked for `$timestamp` meant a timestamp, and writing the seven characters
/// would be the host quietly doing something else.
fn resolve_value(value: &str) -> Result<String, String> {
    match value {
        NOW => Ok(Utc::now().to_rfc3339_opts(SecondsFormat::Nanos, true)),
        UUID => Ok(uuid::Uuid::new_v4().to_string()),
        literal if literal.starts_with('$') => Err(format!(
            "`{literal}` is not a value this host substitutes; bind `{NOW}`, `{UUID}` or a literal"
        )),
        literal if literal.chars().count() > MAX_VALUE_CHARS => Err(format!(
            "A bound value is at most {MAX_VALUE_CHARS} characters"
        )),
        literal => Ok(literal.to_owned()),
    }
}

/// Most pointers one `k8s.setFields` action may write, and the deepest any
/// one of them may reach. A bound template is written once by an app author;
/// anything past these is a payload rather than a declaration.
const MAX_FIELDS: usize = 16;
const MAX_POINTER_SEGMENTS: usize = 8;
/// The largest a bound template may serialize to.
const MAX_TEMPLATE_BYTES: usize = 8 * 1024;

/// `value` with every `$` token resolved, at every depth.
///
/// Recursive over the template an app bound, which is why the template is
/// size-bounded before it gets here: depth is bounded by length.
fn resolve_tokens(value: &Value) -> Result<Value, String> {
    Ok(match value {
        Value::String(text) => Value::String(resolve_value(text)?),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(resolve_tokens)
                .collect::<Result<Vec<_>, _>>()?,
        ),
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(key, value)| Ok((key.clone(), resolve_tokens(value)?)))
                .collect::<Result<Map<_, _>, String>>()?,
        ),
        other => other.clone(),
    })
}

fn bounded(template: &Map<String, Value>) -> Result<(), String> {
    let size = serde_json::to_string(template).map_or(usize::MAX, |t| t.len());
    if size > MAX_TEMPLATE_BYTES {
        return Err(format!(
            "A bound template is at most {MAX_TEMPLATE_BYTES} bytes"
        ));
    }
    Ok(())
}

/// One RFC 6901 pointer's segments, unescaped.
fn pointer_segments(pointer: &str) -> Result<Vec<String>, String> {
    let not_a_pointer = |why: &str| Err(format!("`{pointer}` is not a JSON pointer: {why}"));
    let Some(rest) = pointer.strip_prefix('/') else {
        return not_a_pointer("it must start with `/`");
    };
    let mut segments = Vec::new();
    for raw in rest.split('/') {
        if raw.is_empty() {
            return not_a_pointer("it has an empty segment");
        }
        if raw
            .split('~')
            .skip(1)
            .any(|tail| !tail.starts_with(['0', '1']))
        {
            return not_a_pointer("`~` escapes only `~0` and `~1`");
        }
        segments.push(raw.replace("~1", "/").replace("~0", "~"));
    }
    if segments.len() > MAX_POINTER_SEGMENTS {
        return not_a_pointer(&format!(
            "it is deeper than {MAX_POINTER_SEGMENTS} segments"
        ));
    }
    Ok(segments)
}

/// The merge patch one `k8s.setFields` binding sends, or why the binding is
/// not one this host will accept.
///
/// Building the patch IS the validation: the host checks a binding at install
/// time by building the patch it would send, so there is no rule here that the
/// request path could skip.
fn fields_patch(fields: &Map<String, Value>) -> Result<Value, String> {
    if fields.is_empty() {
        return Err("A setFields action must set at least one field".into());
    }
    if fields.len() > MAX_FIELDS {
        return Err(format!(
            "A setFields action sets at most {MAX_FIELDS} fields"
        ));
    }
    bounded(fields)?;
    let mut parsed = Vec::with_capacity(fields.len());
    for (pointer, value) in fields {
        let segments = pointer_segments(pointer)?;
        if segments.len() < 2 || segments[0] != "spec" {
            return Err(format!(
                "`{pointer}` is not a field under `/spec`; a setFields action writes only spec fields"
            ));
        }
        parsed.push((segments, resolve_tokens(value)?));
    }
    // Two pointers where one contains the other would make the result depend
    // on the order this host happened to apply them in.
    for (i, (a, _)) in parsed.iter().enumerate() {
        for (b, _) in parsed.iter().skip(i + 1) {
            let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
            if long.starts_with(short.as_slice()) {
                return Err(format!(
                    "`/{}` and `/{}` overlap; a setFields action sets fields side by side",
                    short.join("/"),
                    long.join("/")
                ));
            }
        }
    }
    let mut patch = Value::Object(Map::new());
    for (segments, value) in parsed {
        let (last, parents) = segments.split_last().expect("a pointer has a last segment");
        let mut node = &mut patch;
        for segment in parents {
            node = node
                .as_object_mut()
                .ok_or("a pointer reaches through a value")?
                .entry(segment.clone())
                .or_insert_with(|| Value::Object(Map::new()));
        }
        node.as_object_mut()
            .ok_or("a pointer reaches through a value")?
            .insert(last.clone(), value);
    }
    Ok(patch)
}

/// The longest `message` a bound condition may carry.
const MAX_MESSAGE_CHARS: usize = 1024;

/// A `metav1.Condition` `type`: an optional DNS-subdomain prefix and a name of
/// letters, digits, `-`, `_` and `.`, starting and ending alphanumeric.
fn condition_type(value: &str) -> Result<(), String> {
    let name = match value.split_once('/') {
        Some((prefix, name)) if subdomain(prefix) => name,
        Some(_) => return Err(format!("`{value}` is not a condition type")),
        None => value,
    };
    let valid = !name.is_empty()
        && name.len() <= 316
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
        && name.starts_with(|c: char| c.is_ascii_alphanumeric())
        && name.ends_with(|c: char| c.is_ascii_alphanumeric());
    if valid {
        Ok(())
    } else {
        Err(format!("`{value}` is not a condition type"))
    }
}

/// Every rule a bound condition must satisfy, as `metav1.Condition` states
/// them. Shared by the binding check and the request, so a condition that
/// would be rejected at install cannot be reached at call time either.
fn check_condition(
    condition_type_value: &str,
    status: &str,
    reason: &str,
    message: &str,
) -> Result<(), String> {
    condition_type(condition_type_value)?;
    if !matches!(status, "True" | "False" | "Unknown") {
        return Err(format!(
            "`{status}` is not a condition status; bind `True`, `False` or `Unknown`"
        ));
    }
    let reason_ok = !reason.is_empty()
        && reason.len() <= 1024
        && reason.starts_with(|c: char| c.is_ascii_alphabetic())
        && reason
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_,:".contains(&c))
        && reason.ends_with(|c: char| c.is_ascii_alphanumeric() || c == '_');
    if !reason_ok {
        return Err(format!(
            "`{reason}` is not a condition reason; use CamelCase, for example ManuallyTriggered"
        ));
    }
    if message.chars().count() > MAX_MESSAGE_CHARS {
        return Err(format!(
            "A condition message is at most {MAX_MESSAGE_CHARS} characters"
        ));
    }
    Ok(())
}

/// The status patch one `k8s.setStatusCondition` request sends, built from the
/// conditions the object actually holds.
///
/// A merge patch replaces a list whole, so the conditions this action does not
/// own are carried over from the fresh GET rather than dropped. The pinned
/// `resourceVersion` is what makes that safe: if anything changed the list
/// after the read, the API server rejects the patch.
fn condition_patch(current: &Value, input: &SetStatusConditionIn) -> Value {
    let mut conditions: Vec<Value> = current["status"]["conditions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let at = conditions
        .iter()
        .position(|c| c["type"] == input.condition_type.as_str());
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
    // A condition transitions when its status changes, and only then.
    let transitioned = at
        .and_then(|at| {
            (conditions[at]["status"] == input.condition_status.as_str())
                .then(|| {
                    conditions[at]["lastTransitionTime"]
                        .as_str()
                        .map(str::to_owned)
                })
                .flatten()
        })
        .unwrap_or(now);
    let condition = json!({
        "type": input.condition_type,
        "status": input.condition_status,
        "reason": input.reason,
        "message": input.message,
        "lastTransitionTime": transitioned,
    });
    match at {
        Some(at) => conditions[at] = condition,
        None => conditions.push(condition),
    }
    json!({"status": {"conditions": conditions}})
}

/// Paths a bound merge patch may never write, whatever it targets.
///
/// `finalizers` and `ownerReferences` decide what happens when the object is
/// deleted and what deletes it — a write there turns a declared action into a
/// garbage-collection change nobody reviewed. `uid` and `resourceVersion` are
/// the host's pin, `managedFields` is the API server's bookkeeping, and
/// `status` belongs to the controller and to `k8s.setStatusCondition`, which
/// writes it through the subresource that exists for it.
const DENIED_PATHS: &[&[&str]] = &[
    &["metadata", "finalizers"],
    &["metadata", "ownerReferences"],
    &["metadata", "managedFields"],
    &["metadata", "uid"],
    &["metadata", "resourceVersion"],
    &["status"],
];

/// A Secret's values, on top of [`DENIED_PATHS`]. Reading them is the
/// consent-gated `k8s.getSecret`; writing them is not something a declared
/// action does at all.
const DENIED_SECRET_PATHS: &[&[&str]] = &[&["data"], &["stringData"]];

const RBAC_GROUP: &str = "rbac.authorization.k8s.io";
const RBAC_KINDS: &[&str] = &["Role", "ClusterRole", "RoleBinding", "ClusterRoleBinding"];

/// Whether `patch` holds anything at `path`.
fn holds(patch: &Map<String, Value>, path: &[&str]) -> bool {
    let Some((first, rest)) = path.split_first() else {
        return false;
    };
    let Some(mut node) = patch.get(*first) else {
        return false;
    };
    for segment in rest {
        match node.get(segment) {
            Some(next) => node = next,
            None => return false,
        }
    }
    true
}

/// The patch one `k8s.mergePatch` binding sends, or why the host will not send
/// it. As with [`fields_patch`], resolving the template IS the validation.
///
/// The kind rules refuse by kind name as well as by API group, which is the
/// opposite of how a kind is *resolved* (a CRD may reuse a built-in name, so
/// resolution needs the group). Here the direction that fails closed is the
/// broad one: an `acme.io/Role` that cannot be merge-patched is an app author
/// choosing another primitive, while an `acme.io/Role` that can be is a
/// permission model with a name-shaped hole in it.
fn check_merge_patch(group: &str, kind: &str, patch: &Map<String, Value>) -> Result<Value, String> {
    if group == RBAC_GROUP || RBAC_KINDS.contains(&kind) {
        return Err(
            "RBAC roles and bindings cannot be changed by an action an app declares".into(),
        );
    }
    if patch.is_empty() {
        return Err("A mergePatch action must change something".into());
    }
    bounded(patch)?;
    let secret = kind == "Secret";
    let denied = DENIED_PATHS
        .iter()
        .chain(DENIED_SECRET_PATHS.iter().filter(|_| secret));
    for path in denied {
        if holds(patch, path) {
            return Err(format!(
                "`{}` cannot be written by an action an app declares",
                path.join(".")
            ));
        }
    }
    resolve_tokens(&Value::Object(patch.clone()))
}

/// Refuses a `k8s.setFields` pointer whose parent is not an object on the
/// object that was just read.
///
/// [`fields_patch`] nests one object per segment, and a JSON merge patch
/// *replaces* a value whose shape differs rather than merging into it
/// (RFC 7386). So `/spec/containers/0/image` over a list sends
/// `{"containers": {"0": …}}`: a schema with a type refuses it, but a CRD
/// field under `x-kubernetes-preserve-unknown-fields` has no type, and the
/// whole list would be replaced by an object — a field the action never
/// named, silently rewritten.
///
/// This runs on the fresh GET rather than at install, because that is the
/// only place the shape is known: a numeric or `-` segment is a legal object
/// key, so [`pointer_segments`] cannot classify one from the manifest alone
/// (CodeRabbit's point on PR #665). Addressing an *element* of a list would
/// need RFC 6902 operations and a second way to pin the review, which #549
/// does not ask for; a declared action writes a list by naming the list.
fn check_object_parents(current: &Value, fields: &Map<String, Value>) -> Result<(), String> {
    const ABSENT: &Value = &Value::Null;
    for pointer in fields.keys() {
        let segments = pointer_segments(pointer)?;
        let (_, parents) = segments.split_last().expect("a pointer has a last segment");
        let mut node = current;
        let mut walked = String::new();
        for segment in parents {
            object_or_absent(node, &walked, pointer)?;
            walked.push('/');
            walked.push_str(segment);
            node = node.get(segment.as_str()).unwrap_or(ABSENT);
        }
        // The last parent as well: the loop descends into it and would
        // otherwise never look at it, which is the whole of `/spec/containers/0`
        // versus `/spec/containers/0/image`.
        object_or_absent(node, &walked, pointer)?;
    }
    Ok(())
}

/// One parent of a `k8s.setFields` pointer. Absent is fine — the patch creates
/// an object there — and anything that is not an object is not.
fn object_or_absent(node: &Value, walked: &str, pointer: &str) -> Result<(), String> {
    if node.is_object() || node.is_null() {
        return Ok(());
    }
    let at = if walked.is_empty() {
        "this resource".to_owned()
    } else {
        format!("`{walked}`")
    };
    Err(format!(
        "{at} is not an object on this resource, so `{pointer}` cannot reach through it; a setFields action writes object fields and writes a list whole"
    ))
}

/// The argument `name` as a string, or why the binding cannot be accepted.
fn text<'a>(arguments: &'a Map<String, Value>, name: &str) -> Result<&'a str, String> {
    match arguments.get(name) {
        Some(Value::String(value)) => Ok(value),
        Some(_) => Err(format!("`{name}` must be a string")),
        None => Err(format!("`{name}` must be bound in the manifest")),
    }
}

/// The declared preconditions in a binding's arguments, held to the same rules
/// the handler holds them to.
///
/// The host binds these out of the action's own `preconditions` field, so a
/// value here that is not a list of predicates is a binding built by something
/// other than this host — refused rather than ignored.
fn check_bound_preconditions(arguments: &Map<String, Value>) -> Result<(), String> {
    let Some(declared) = arguments.get("preconditions") else {
        return Ok(());
    };
    let predicates: Vec<Predicate> = serde_json::from_value(declared.clone())
        .map_err(|e| format!("`preconditions` must be a list of predicates: {e}"))?;
    check_predicates(&predicates)
}

/// Every rule a manifest's bound arguments must satisfy that the input schema
/// cannot express.
///
/// The host runs this where the binding is accepted (install time, and again
/// when a stored app is reverified) and the handler runs the same rules on the
/// way to the cluster, so there is one statement of each rule and no way to
/// reach the API server past it.
pub fn check_bound_arguments(
    capability: &str,
    arguments: &Map<String, Value>,
) -> Result<(), String> {
    // Every primitive takes the same declared preconditions (#550), so they
    // are checked once here rather than in each arm — and by the same
    // `check_predicates` each handler runs before it reads the object, so a
    // predicate this refuses cannot be reached from the cluster side either.
    check_bound_preconditions(arguments)?;
    match capability {
        ANNOTATE => {
            check_annotation_key(text(arguments, "key")?)?;
            resolve_value(text(arguments, "value")?)?;
            Ok(())
        }
        SET_FIELDS => {
            let Some(fields) = arguments.get("fields").and_then(Value::as_object) else {
                return Err("`fields` must be bound in the manifest as an object".into());
            };
            fields_patch(fields).map(|_| ())
        }
        SET_STATUS_CONDITION => check_condition(
            text(arguments, "conditionType")?,
            text(arguments, "conditionStatus")?,
            text(arguments, "reason")?,
            match arguments.get("message") {
                None => "",
                Some(Value::String(message)) => message,
                Some(_) => return Err("`message` must be a string".into()),
            },
        ),
        MERGE_PATCH => {
            let Some(patch) = arguments.get("patch").and_then(Value::as_object) else {
                return Err("`patch` must be bound in the manifest as an object".into());
            };
            let identity = |field: &str| {
                arguments
                    .get(field)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
            };
            check_merge_patch(identity("group"), identity("kind"), patch).map(|_| ())
        }
        other => Err(format!("{other} is not a host action primitive")),
    }
}

async fn annotate(client: Client, input: AnnotateIn) -> Result<Value, String> {
    check_annotation_key(&input.key)?;
    check_predicates(&input.preconditions)?;
    let value = resolve_value(&input.value)?;
    request(
        client,
        input.reviewed(),
        &input.preconditions,
        false,
        |_| Ok(json!({"metadata": {"annotations": {&input.key: value}}})),
    )
    .await
}

async fn set_fields(client: Client, input: SetFieldsIn) -> Result<Value, String> {
    let patch = fields_patch(&input.fields)?;
    check_predicates(&input.preconditions)?;
    request(
        client,
        input.reviewed(),
        &input.preconditions,
        false,
        |current| {
            check_object_parents(current, &input.fields)?;
            Ok(patch)
        },
    )
    .await
}

async fn set_merge_patch(client: Client, input: MergePatchIn) -> Result<Value, String> {
    let patch = check_merge_patch(&input.group, &input.kind, &input.patch)?;
    check_predicates(&input.preconditions)?;
    request(
        client,
        input.reviewed(),
        &input.preconditions,
        false,
        |_| Ok(patch),
    )
    .await
}

async fn set_status_condition(
    client: Client,
    input: SetStatusConditionIn,
) -> Result<Value, String> {
    check_condition(
        &input.condition_type,
        &input.condition_status,
        &input.reason,
        &input.message,
    )?;
    check_predicates(&input.preconditions)?;
    request(
        client,
        input.reviewed(),
        &input.preconditions,
        true,
        |current| Ok(condition_patch(current, &input)),
    )
    .await
}

/// What a request to one primitive can disturb, and the host's own words for
/// the question a person is asked before it runs.
///
/// The level is the ceiling of what *that primitive's shape* can do, which is
/// not the same as the ceiling of what a controller may do afterwards. An
/// annotation sets one key: it changes no spec, stops nothing, and what the
/// controller then does is the controller's — so `k8s.annotate` is `medium`
/// even though the Flux `forceAt` key it can write ends in a Helm upgrade.
/// `k8s.mergePatch` is the one that can express an Argo CD sync, which applies
/// manifests and runs hooks, so it is `high`.
///
/// This is the floor for every binding, never a ceiling on one:
/// [`Annotations::for_binding`] only ever raises. #550's preconditions narrow
/// *when* a primitive runs, not what it can disturb when it does, so none of
/// these rows move for them; when #551's migrated Flux and Argo CD actions
/// give the host more to go on per action, a `force` binding can be published
/// above its primitive's row without any of them moving either.
fn metadata(primitive: &str) -> (Impact, &'static str) {
    match primitive {
        ANNOTATE => (
            Impact::Medium,
            "Set the action's annotation[ on {resource}][ in cluster {cluster}]? What the controller does next is up to it.",
        ),
        SET_FIELDS => (
            Impact::Medium,
            "Change the fields this action sets[ on {resource}][ in cluster {cluster}]? Workloads already running are not stopped.",
        ),
        SET_STATUS_CONDITION => (
            Impact::Medium,
            "Write the action's status condition[ on {resource}][ in cluster {cluster}]? The controller acts on it.",
        ),
        MERGE_PATCH => (
            Impact::High,
            "Apply the change this action declares[ to {resource}][ in cluster {cluster}]? It can start work the controller runs straight away, including replacing what is running.",
        ),
        // Unreachable: every id in `PRIMITIVES` is matched above, and
        // `every_primitive_is_a_gated_capability_carrying_the_hosts_own_level_and_words`
        // holds that true.
        _ => (Impact::High, Annotations::DESTRUCTIVE.confirm.unwrap_or("")),
    }
}

/// Builds one primitive's capability: the host row, the host's words, the
/// rules for what a manifest may bind, and the handler.
macro_rules! primitive {
    ($id:expr, $summary:expr, $input:ty, $run:ident, $cache:expr) => {{
        let cache: Arc<ClientCache> = $cache;
        let (impact, confirm) = metadata($id);
        Capability::typed::<$input, Value, _, _>(
            $id,
            $summary,
            Annotations::MUTATING.with_impact(impact).with_confirm(confirm),
            move |input: $input| {
                let cache = cache.clone();
                async move {
                    let client = cache
                        .get(&input.context)
                        .await
                        .map_err(CapabilityError::Handler)?;
                    tokio::time::timeout(request_timeout(), $run(client, input))
                        .await
                        .map_err(|_| {
                            CapabilityError::Handler(
                                "Action request timed out; refresh the resource to check whether it was accepted"
                                    .into(),
                            )
                        })?
                        .map_err(CapabilityError::Handler)
                }
            },
        )
        .checking_bound_arguments(|arguments| check_bound_arguments($id, arguments))
    }};
}

/// Every host action primitive, in [`PRIMITIVES`] order.
pub fn capabilities(cache: Arc<ClientCache>) -> Vec<Capability> {
    vec![
        primitive!(
            ANNOTATE,
            "Write one fixed annotation on the reviewed resource, as an app's action declares it; requires confirmation",
            AnnotateIn,
            annotate,
            cache.clone()
        ),
        primitive!(
            SET_FIELDS,
            "Set fixed spec fields on the reviewed resource, as an app's action declares them; requires confirmation",
            SetFieldsIn,
            set_fields,
            cache.clone()
        ),
        primitive!(
            SET_STATUS_CONDITION,
            "Write one status condition on the reviewed resource through the status subresource, as an app's action declares it; requires confirmation",
            SetStatusConditionIn,
            set_status_condition,
            cache.clone()
        ),
        primitive!(
            MERGE_PATCH,
            "Send the fixed merge patch an app's action declares, past the host deny-list, to the reviewed resource; requires confirmation",
            MergePatchIn,
            set_merge_patch,
            cache
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use kube::Client;
    use serde_json::{json, Value};
    use std::sync::{Arc, Mutex};

    type Requests = Arc<Mutex<Vec<(String, Value)>>>;

    /// A HelmRelease as the API server would return it, with whatever extra
    /// metadata or spec the case needs merged over the fixture.
    fn object(extra: Value) -> Value {
        let mut base = json!({
            "apiVersion": "helm.toolkit.fluxcd.io/v2",
            "kind": "HelmRelease",
            "metadata": {"name": "api", "namespace": "team", "uid": "u", "resourceVersion": "2"},
            "spec": {},
            "status": {}
        });
        merge(&mut base, &extra);
        base
    }

    fn merge(into: &mut Value, from: &Value) {
        match (into, from) {
            (Value::Object(into), Value::Object(from)) => {
                for (key, value) in from {
                    merge(into.entry(key.clone()).or_insert(Value::Null), value);
                }
            }
            (into, from) => *into = from.clone(),
        }
    }

    /// Serves `current` for every GET and records every request.
    fn mock(current: Value) -> (Client, Requests) {
        let requests: Requests = Arc::new(Mutex::new(vec![]));
        let captured = requests.clone();
        let service = tower::service_fn(move |request: http::Request<kube::client::Body>| {
            let captured = captured.clone();
            let current = current.clone();
            async move {
                let method = request.method().to_string();
                let uri = request.uri().to_string();
                let body = request.into_body().collect_bytes().await.unwrap();
                let value = if body.is_empty() {
                    Value::Null
                } else {
                    serde_json::from_slice(&body).unwrap()
                };
                captured
                    .lock()
                    .unwrap()
                    .push((format!("{method} {uri}"), value));
                Ok::<_, std::convert::Infallible>(
                    http::Response::builder()
                        .status(200)
                        .header("content-type", "application/json")
                        .body(kube::client::Body::from(current.to_string().into_bytes()))
                        .unwrap(),
                )
            }
        });
        (Client::new(service, "default"), requests)
    }

    /// The identity of the fixture object, as a manifest's reader binding and
    /// the surface's inputs together supply it.
    fn helmrelease(extra: Value) -> Value {
        let mut input = json!({
            "context": "cluster/a",
            "group": "helm.toolkit.fluxcd.io",
            "version": "v2",
            "plural": "helmreleases",
            "kind": "HelmRelease",
            "namespaced": true,
            "namespace": "team",
            "name": "api",
            "uid": "u",
            "resourceVersion": "2"
        });
        merge(&mut input, &extra);
        input
    }

    fn annotate_in(extra: Value) -> AnnotateIn {
        serde_json::from_value(helmrelease(extra)).expect("input deserializes")
    }

    #[tokio::test]
    async fn annotate_reads_then_patches_the_reviewed_object_with_the_bound_key() {
        let (client, requests) = mock(object(json!({})));
        let out = annotate(
            client,
            annotate_in(json!({"key": "reconcile.fluxcd.io/requestedAt", "value": "$now"})),
        )
        .await
        .expect("annotate");
        assert_eq!(out, json!({"requested": true}));

        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2, "a fresh GET and one PATCH: {requests:?}");
        assert_eq!(
            requests[0].0,
            "GET /apis/helm.toolkit.fluxcd.io/v2/namespaces/team/helmreleases/api"
        );
        let (request, body) = &requests[1];
        assert!(
            request.starts_with(
                "PATCH /apis/helm.toolkit.fluxcd.io/v2/namespaces/team/helmreleases/api"
            ),
            "{request}"
        );
        assert!(
            !request.contains("/status"),
            "an annotation is not a status write: {request}"
        );
        // Pinned to what the operator reviewed, and touching nothing else.
        assert_eq!(body["metadata"]["uid"], "u");
        assert_eq!(body["metadata"]["resourceVersion"], "2");
        assert_eq!(
            body.as_object().map(|b| b.len()),
            Some(1),
            "only metadata is written: {body}"
        );
        let written = body["metadata"]["annotations"]["reconcile.fluxcd.io/requestedAt"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let parsed = DateTime::parse_from_rfc3339(&written)
            .unwrap_or_else(|e| panic!("`{written}` is not RFC 3339: {e}"));
        assert!(
            written.contains('.') && written.ends_with('Z'),
            "$now is RFC 3339 nanoseconds in UTC, got `{written}`"
        );
        assert!(
            (Utc::now() - parsed.with_timezone(&Utc))
                .num_seconds()
                .abs()
                < 60,
            "`{written}` is not now"
        );
    }

    #[tokio::test]
    async fn a_literal_value_is_written_verbatim_and_uuid_is_fresh_for_each_request() {
        let (client, requests) = mock(object(json!({})));
        annotate(
            client,
            annotate_in(json!({"key": "argocd.argoproj.io/refresh", "value": "normal"})),
        )
        .await
        .expect("annotate");
        assert_eq!(
            requests.lock().unwrap()[1].1["metadata"]["annotations"]["argocd.argoproj.io/refresh"],
            "normal"
        );

        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..2 {
            let (client, requests) = mock(object(json!({})));
            annotate(
                client,
                annotate_in(json!({"key": "example.io/token", "value": "$uuid"})),
            )
            .await
            .expect("annotate");
            let value = requests.lock().unwrap()[1].1["metadata"]["annotations"]
                ["example.io/token"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            assert_eq!(value.len(), 36, "a hyphenated UUID, got `{value}`");
            assert_eq!(value.matches('-').count(), 4, "`{value}`");
            seen.insert(value);
        }
        assert_eq!(seen.len(), 2, "each request gets its own token: {seen:?}");
    }

    #[tokio::test]
    async fn a_review_of_another_object_or_another_version_never_reaches_a_patch() {
        for stale in [json!({"uid": "other"}), json!({"resourceVersion": "1"})] {
            let (client, requests) = mock(object(json!({})));
            let mut input = json!({"key": "example.io/k", "value": "x"});
            merge(&mut input, &stale);
            let err = annotate(client, annotate_in(input))
                .await
                .expect_err("a stale review is refused");
            assert!(err.contains("Resource changed or was replaced"), "{err}");
            assert_eq!(
                requests.lock().unwrap().len(),
                1,
                "the GET happened, the PATCH must not: {stale}"
            );
        }
    }

    #[tokio::test]
    async fn an_object_being_deleted_is_not_written_to() {
        let (client, requests) = mock(object(
            json!({"metadata": {"deletionTimestamp": "2026-01-01T00:00:00Z"}}),
        ));
        let err = annotate(
            client,
            annotate_in(json!({"key": "example.io/k", "value": "x"})),
        )
        .await
        .expect_err("a terminating object is refused");
        assert!(err.contains("being deleted"), "{err}");
        assert_eq!(requests.lock().unwrap().len(), 1);
    }

    fn set_fields_in(extra: Value) -> SetFieldsIn {
        serde_json::from_value(helmrelease(extra)).expect("input deserializes")
    }

    #[tokio::test]
    async fn set_fields_writes_the_bound_pointers_and_nothing_else() {
        let (client, requests) = mock(object(json!({"spec": {"suspend": false}})));
        let out = set_fields(
            client,
            set_fields_in(json!({"fields": {
                "/spec/suspend": true,
                "/spec/chart/spec/version": "1.2.3"
            }})),
        )
        .await
        .expect("set fields");
        assert_eq!(out, json!({"requested": true}));

        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        let (request, body) = &requests[1];
        assert!(
            request.starts_with(
                "PATCH /apis/helm.toolkit.fluxcd.io/v2/namespaces/team/helmreleases/api"
            ),
            "{request}"
        );
        assert!(!request.contains("/status"), "{request}");
        assert_eq!(
            *body,
            json!({
                "metadata": {"uid": "u", "resourceVersion": "2"},
                "spec": {"suspend": true, "chart": {"spec": {"version": "1.2.3"}}}
            })
        );
    }

    /// A merge patch REPLACES a value whose shape differs rather than merging
    /// into it (RFC 7386), and `fields_patch` nests one object per segment. So
    /// `/spec/containers/0/image` over a list would rewrite the whole list as
    /// `{"0": …}` wherever the CRD's schema does not refuse it — a field the
    /// action never named, silently replaced.
    #[tokio::test]
    async fn a_pointer_that_reaches_through_a_list_is_refused_before_the_patch() {
        let (client, requests) = mock(object(
            json!({"spec": {"containers": [{"image": "registry.example/app:1"}]}}),
        ));
        let err = set_fields(
            client,
            set_fields_in(
                json!({"fields": {"/spec/containers/0/image": "registry.example/app:2"}}),
            ),
        )
        .await
        .expect_err("a pointer into a list is refused");
        assert!(err.contains("/spec/containers"), "{err}");
        assert_eq!(
            requests.lock().unwrap().len(),
            1,
            "the GET happened, the PATCH must not"
        );
    }

    /// And the shallow case: the value's *immediate* parent. `/spec/containers/0`
    /// nests `{"containers": {"0": …}}` just the same, so it replaces the list
    /// just the same — the pointer is one segment shorter, and nothing else
    /// about it is different.
    #[tokio::test]
    async fn an_immediate_list_or_scalar_parent_is_refused_too() {
        for (spec, pointer, parent) in [
            (
                json!({"containers": [{"image": "a"}]}),
                "/spec/containers/0",
                "/spec/containers",
            ),
            (
                json!({"suspend": true}),
                "/spec/suspend/paused",
                "/spec/suspend",
            ),
        ] {
            let (client, requests) = mock(object(json!({"spec": spec})));
            let mut input = helmrelease(json!({}));
            input["fields"] = json!({});
            input["fields"][pointer] = json!("x");
            let input = serde_json::from_value(input).expect("input deserializes");
            let err = match set_fields(client, input).await {
                Err(err) => err,
                Ok(out) => panic!("{pointer} was written: {out}"),
            };
            assert!(err.contains(parent), "{pointer}: {err}");
            assert_eq!(
                requests.lock().unwrap().len(),
                1,
                "{pointer}: the GET happened, the PATCH must not"
            );
        }
    }

    /// What a declared action does instead: name the list itself and write it
    /// whole, which is an ordinary merge patch.
    #[tokio::test]
    async fn a_list_is_written_whole_at_its_own_pointer() {
        let (client, requests) = mock(object(json!({"spec": {"ignore": ["a"]}})));
        set_fields(
            client,
            set_fields_in(json!({"fields": {"/spec/ignore": ["b", "c"]}})),
        )
        .await
        .expect("set fields");
        assert_eq!(
            requests.lock().unwrap()[1].1["spec"]["ignore"],
            json!(["b", "c"])
        );
    }

    /// A numeric segment is a legal object key, and the host has no schema at
    /// install time to tell one from a list index — so the pointer rules keep
    /// accepting it and the object on the server decides.
    #[tokio::test]
    async fn a_numeric_segment_over_an_object_is_an_ordinary_field() {
        let (client, requests) = mock(object(json!({"spec": {"weights": {"0": 1}}})));
        set_fields(
            client,
            set_fields_in(json!({"fields": {"/spec/weights/0": 2}})),
        )
        .await
        .expect("set fields");
        assert_eq!(requests.lock().unwrap()[1].1["spec"]["weights"]["0"], 2);
        assert_eq!(
            check_bound_arguments(
                SET_FIELDS,
                json!({"fields": {"/spec/weights/0": 2}})
                    .as_object()
                    .expect("object")
            ),
            Ok(()),
            "a numeric segment is not rejected at install"
        );
    }

    #[test]
    fn set_fields_reaches_only_spec_and_only_through_well_formed_pointers() {
        let check = |fields: Value| {
            check_bound_arguments(
                SET_FIELDS,
                json!({"fields": fields}).as_object().expect("object"),
            )
            .err()
        };
        assert_eq!(check(json!({"/spec/suspend": false})), None);
        assert_eq!(check(json!({"/spec/values/image~1tag": "v2"})), None);
        for outside in [
            json!({"/metadata/finalizers": []}),
            json!({"/status/conditions": []}),
            json!({"/metadata/annotations/x": "y"}),
        ] {
            let refused = check(outside.clone()).unwrap_or_else(|| panic!("allowed {outside}"));
            assert!(refused.contains("spec"), "{refused}");
        }
        // The whole of `spec` is not a field, and neither is a malformed pointer.
        for malformed in [
            json!({"/spec": {"suspend": true}}),
            json!({"spec/suspend": true}),
            json!({"/spec/": true}),
            json!({"": true}),
        ] {
            assert!(check(malformed.clone()).is_some(), "allowed {malformed}");
        }
        // One pointer inside another would make the result depend on the order
        // the host happened to apply them in.
        assert!(check(json!({"/spec/a": {}, "/spec/a/b": 1})).is_some());
        assert!(check(json!({})).is_some(), "an action that sets nothing");
        // A `$`-prefixed string is a token wherever it appears, and only two exist.
        assert_eq!(check(json!({"/spec/at": "$now"})), None);
        assert!(check(json!({"/spec/at": "$then"})).is_some());
    }

    fn condition_in(extra: Value) -> SetStatusConditionIn {
        serde_json::from_value(helmrelease(extra)).expect("input deserializes")
    }

    /// cert-manager's Renew: `Issuing=True`, reason `ManuallyTriggered`.
    fn renew() -> Value {
        json!({
            "conditionType": "Issuing",
            "conditionStatus": "True",
            "reason": "ManuallyTriggered",
            "message": "Renewal triggered from srelens"
        })
    }

    #[tokio::test]
    async fn a_status_condition_is_written_through_the_status_subresource() {
        let (client, requests) = mock(object(json!({"status": {"conditions": [
            {"type": "Ready", "status": "True", "reason": "Reconciled", "message": "",
             "lastTransitionTime": "2026-01-01T00:00:00Z"}
        ]}})));
        let out = set_status_condition(client, condition_in(renew()))
            .await
            .expect("set condition");
        assert_eq!(out, json!({"requested": true}));

        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        let (request, body) = &requests[1];
        let path = request.split(['?', ' ']).nth(1).unwrap_or_default();
        assert_eq!(
            path, "/apis/helm.toolkit.fluxcd.io/v2/namespaces/team/helmreleases/api/status",
            "a condition is written to the status subresource, not the object: {request}"
        );
        assert_eq!(
            body["metadata"],
            json!({"uid": "u", "resourceVersion": "2"})
        );
        let conditions = body["status"]["conditions"]
            .as_array()
            .unwrap_or_else(|| panic!("conditions: {body}"));
        // A merge patch replaces the whole list, so the ones this action does
        // not own have to be carried over from the object that was just read.
        assert_eq!(conditions.len(), 2, "{body}");
        assert_eq!(conditions[0]["type"], "Ready");
        assert_eq!(conditions[0]["lastTransitionTime"], "2026-01-01T00:00:00Z");
        assert_eq!(conditions[1]["type"], "Issuing");
        assert_eq!(conditions[1]["status"], "True");
        assert_eq!(conditions[1]["reason"], "ManuallyTriggered");
        assert_eq!(conditions[1]["message"], "Renewal triggered from srelens");
        let transitioned = conditions[1]["lastTransitionTime"]
            .as_str()
            .unwrap_or_default();
        DateTime::parse_from_rfc3339(transitioned)
            .unwrap_or_else(|e| panic!("`{transitioned}` is not RFC 3339: {e}"));
    }

    #[tokio::test]
    async fn a_condition_of_the_same_type_is_replaced_in_place_and_only_transitions_on_a_change() {
        let existing = |status: &str| {
            object(json!({"status": {"conditions": [
                {"type": "Issuing", "status": status, "reason": "Old", "message": "old",
                 "lastTransitionTime": "2026-01-01T00:00:00Z"},
                {"type": "Ready", "status": "True", "reason": "R", "message": "",
                 "lastTransitionTime": "2026-01-01T00:00:00Z"}
            ]}}))
        };

        let (client, requests) = mock(existing("True"));
        set_status_condition(client, condition_in(renew()))
            .await
            .expect("set condition");
        let conditions = requests.lock().unwrap()[1].1["status"]["conditions"].clone();
        assert_eq!(conditions.as_array().map(Vec::len), Some(2));
        assert_eq!(conditions[0]["type"], "Issuing", "replaced where it stood");
        assert_eq!(conditions[0]["reason"], "ManuallyTriggered");
        assert_eq!(
            conditions[0]["lastTransitionTime"], "2026-01-01T00:00:00Z",
            "the status did not change, so neither did the transition time"
        );
        assert_eq!(conditions[1]["type"], "Ready");

        let (client, requests) = mock(existing("False"));
        set_status_condition(client, condition_in(renew()))
            .await
            .expect("set condition");
        let conditions = requests.lock().unwrap()[1].1["status"]["conditions"].clone();
        assert_ne!(
            conditions[0]["lastTransitionTime"], "2026-01-01T00:00:00Z",
            "False to True is a transition"
        );
    }

    #[test]
    fn a_status_condition_binding_takes_a_type_a_tri_state_status_and_a_reason() {
        let check = |extra: Value| {
            let mut args = renew();
            merge(&mut args, &extra);
            check_bound_arguments(SET_STATUS_CONDITION, args.as_object().expect("object")).err()
        };
        assert_eq!(check(json!({})), None);
        for status in ["False", "Unknown"] {
            assert_eq!(check(json!({"conditionStatus": status})), None);
        }
        for refused in [
            json!({"conditionStatus": "true"}),
            json!({"conditionStatus": "yes"}),
            json!({"conditionStatus": "$now"}),
            json!({"conditionType": ""}),
            json!({"conditionType": "not a type"}),
            json!({"reason": ""}),
            json!({"reason": "not a reason"}),
        ] {
            assert!(check(refused.clone()).is_some(), "allowed {refused}");
        }
    }

    /// Argo CD's Sync: the operation field, with pruning off.
    fn sync() -> Value {
        json!({"patch": {"operation": {
            "initiatedBy": {"username": "srelens"},
            "sync": {"prune": false, "syncStrategy": {"hook": {}}}
        }}})
    }

    #[tokio::test]
    async fn a_merge_patch_sends_the_bound_template_and_the_pin_and_nothing_more() {
        let (client, requests) = mock(object(json!({})));
        let out = set_merge_patch(
            client,
            serde_json::from_value(helmrelease(sync())).expect("input deserializes"),
        )
        .await
        .expect("merge patch");
        assert_eq!(out, json!({"requested": true}));

        let requests = requests.lock().unwrap();
        let (request, body) = &requests[1];
        assert!(!request.contains("/status"), "{request}");
        assert_eq!(
            *body,
            json!({
                "metadata": {"uid": "u", "resourceVersion": "2"},
                "operation": {
                    "initiatedBy": {"username": "srelens"},
                    "sync": {"prune": false, "syncStrategy": {"hook": {}}}
                }
            })
        );
    }

    #[test]
    fn the_merge_patch_deny_list_keeps_the_host_owned_paths_out_of_a_template() {
        let check = |patch: Value| {
            check_bound_arguments(
                MERGE_PATCH,
                json!({"patch": patch}).as_object().expect("object"),
            )
            .err()
        };
        assert_eq!(
            check(json!({"operation": {"sync": {"prune": false}}})),
            None
        );
        assert_eq!(check(json!({"metadata": {"labels": {"a": "b"}}})), None);
        for denied in [
            json!({"metadata": {"finalizers": []}}),
            json!({"metadata": {"ownerReferences": []}}),
            json!({"metadata": {"managedFields": []}}),
            json!({"metadata": {"uid": "someone-elses"}}),
            json!({"metadata": {"resourceVersion": "1"}}),
            json!({"status": {"conditions": []}}),
        ] {
            let refused = check(denied.clone()).unwrap_or_else(|| panic!("allowed {denied}"));
            assert!(refused.contains("cannot be written"), "{refused}");
        }
        assert!(check(json!({})).is_some(), "a patch that changes nothing");
        assert!(
            check(json!({"spec": {"at": "$then"}})).is_some(),
            "unknown token"
        );
        assert_eq!(check(json!({"spec": {"at": "$now"}})), None);
    }

    #[test]
    fn rbac_kinds_and_secret_values_are_out_of_a_merge_patch_reach() {
        let check = |identity: Value, patch: Value| {
            let mut args = json!({"patch": patch});
            merge(&mut args, &identity);
            check_bound_arguments(MERGE_PATCH, args.as_object().expect("object")).err()
        };
        let rbac = json!({"group": "rbac.authorization.k8s.io", "kind": "Role"});
        let refused = check(rbac, json!({"rules": []})).expect("RBAC is refused");
        assert!(refused.contains("RBAC"), "{refused}");
        // By kind as well as by group: a CRD may reuse the name.
        assert!(check(
            json!({"group": "rbac.authorization.k8s.io", "kind": "ClusterRoleBinding"}),
            json!({"subjects": []})
        )
        .is_some());
        let secret = json!({"group": "", "kind": "Secret"});
        for values in [
            json!({"data": {"k": "dg=="}}),
            json!({"stringData": {"k": "v"}}),
        ] {
            let refused =
                check(secret.clone(), values.clone()).unwrap_or_else(|| panic!("allowed {values}"));
            assert!(refused.contains("cannot be written"), "{refused}");
        }
        assert_eq!(
            check(secret, json!({"metadata": {"labels": {"a": "b"}}})),
            None,
            "labelling a Secret is not reading one"
        );
    }

    #[test]
    fn every_primitive_is_a_gated_capability_carrying_the_hosts_own_level_and_words() {
        let caps = capabilities(ClientCache::new(std::path::PathBuf::from("/dev/null")));
        assert_eq!(
            caps.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            PRIMITIVES
        );
        for cap in &caps {
            let a = cap.annotations;
            assert!(!a.read_only && a.requires_confirm, "{} is ungated", cap.id);
            let template = a.confirm.unwrap_or_else(|| panic!("{}: no words", cap.id));
            srelens_capability::check_confirm_template(template)
                .unwrap_or_else(|e| panic!("{}: {e}", cap.id));
            assert!(
                srelens_capability::render_confirm(template, &Default::default()).is_some(),
                "{} must read as a sentence with no fields resolved",
                cap.id
            );
            assert!(
                cap.bound_arguments.is_some(),
                "{} must check what a manifest binds to it",
                cap.id
            );
        }
        let impact = |id: &str| {
            caps.iter()
                .find(|c| c.id == id)
                .expect("registered")
                .annotations
                .impact
        };
        // The escape hatch is the one that can reach an Argo CD sync.
        assert_eq!(impact(MERGE_PATCH), srelens_capability::Impact::High);
        for narrow in [ANNOTATE, SET_FIELDS, SET_STATUS_CONDITION] {
            assert_eq!(impact(narrow), srelens_capability::Impact::Medium);
        }
    }

    #[tokio::test]
    async fn a_primitive_refuses_a_binding_its_own_rules_reject_before_any_request() {
        let caps = capabilities(ClientCache::new(std::path::PathBuf::from("/dev/null")));
        let check = caps
            .iter()
            .find(|c| c.id == ANNOTATE)
            .expect("registered")
            .bound_arguments
            .clone()
            .expect("checks its bound arguments");
        assert!(check(
            json!({"key": "a.io/b", "value": "$now"})
                .as_object()
                .unwrap()
        )
        .is_ok());
        assert!(check(
            json!({"key": "a.io/b", "value": "$then"})
                .as_object()
                .unwrap()
        )
        .is_err());
    }

    #[test]
    fn an_annotate_binding_takes_a_fixed_key_and_a_value_from_the_closed_vocabulary() {
        let check =
            |args: Value| check_bound_arguments(ANNOTATE, args.as_object().expect("object")).err();
        assert_eq!(
            check(json!({"key": "a.example.io/b", "value": "$now"})),
            None
        );
        assert_eq!(
            check(json!({"key": "a.example.io/b", "value": "$uuid"})),
            None
        );
        assert_eq!(
            check(json!({"key": "a.example.io/b", "value": "normal"})),
            None
        );
        // Anything else that looks like a variable is refused rather than
        // written through as a literal an app could not have meant.
        let refused = check(json!({"key": "a.example.io/b", "value": "$requestedBy"}))
            .expect("an unknown token is refused");
        assert!(
            refused.contains("$now") && refused.contains("$uuid"),
            "{refused}"
        );
        assert!(
            check(json!({"key": "", "value": "x"})).is_some(),
            "an empty key"
        );
        assert!(
            check(json!({"value": "x"})).is_some(),
            "no key bound at all"
        );
        assert!(
            check(json!({"key": "a.example.io/b"})).is_some(),
            "no value bound at all"
        );
    }

    // -- declared preconditions (#550) -------------------------------------

    /// `.spec.suspend notEquals true`, with the reason #550 names.
    fn not_suspended() -> Value {
        json!([{
            "jsonPath": ".spec.suspend", "notEquals": true,
            "reason": "Resume this resource before requesting reconciliation"
        }])
    }

    fn reconcile() -> Value {
        json!({"key": "reconcile.fluxcd.io/requestedAt", "value": "$now"})
    }

    #[tokio::test]
    async fn a_precondition_that_does_not_hold_refuses_before_the_patch() {
        let (client, requests) = mock(object(json!({"spec": {"suspend": true}})));
        let refused = annotate(
            client,
            annotate_in(
                json!({"key": "reconcile.fluxcd.io/requestedAt", "value": "$now",
                "preconditions": not_suspended()}),
            ),
        )
        .await
        .expect_err("a suspended resource is not reconciled");
        assert!(
            refused.contains("Resume this resource before requesting reconciliation"),
            "the operator is told what the app declared: {refused}"
        );
        let requests = requests.lock().unwrap();
        assert_eq!(
            requests.len(),
            1,
            "the fresh GET, and nothing written: {requests:?}"
        );
        assert!(requests[0].0.starts_with("GET "), "{:?}", requests[0].0);
    }

    #[tokio::test]
    async fn a_precondition_that_holds_lets_the_write_through() {
        let (client, requests) = mock(object(json!({"spec": {"suspend": false}})));
        annotate(
            client,
            annotate_in(
                json!({"key": "reconcile.fluxcd.io/requestedAt", "value": "$now",
                "preconditions": not_suspended()}),
            ),
        )
        .await
        .expect("a live resource reconciles");
        assert_eq!(requests.lock().unwrap().len(), 2, "the GET and the PATCH");
    }

    /// The two refusals every write makes are the host's, so a manifest that
    /// declares nothing still gets them — and one that declares a predicate
    /// which *holds* on a deleted object does not get past them either.
    #[tokio::test]
    async fn a_host_guard_refuses_whatever_the_manifest_declares() {
        let deleting = object(json!({"metadata": {"deletionTimestamp": "2026-01-01T00:00:00Z"}}));
        let waved_through = json!([{
            "jsonPath": ".metadata.deletionTimestamp", "present": true,
            "reason": "This app says a deletion is fine"
        }]);
        for declared in [json!([]), waved_through] {
            let (client, requests) = mock(deleting.clone());
            let mut input = reconcile();
            input["preconditions"] = declared.clone();
            let refused = annotate(client, annotate_in(input))
                .await
                .expect_err("an object being deleted is not written");
            assert_eq!(
                refused, "Resource is being deleted",
                "the host's guard, in the host's words, with {declared}"
            );
            assert_eq!(requests.lock().unwrap().len(), 1, "nothing was written");
        }
    }

    #[tokio::test]
    async fn a_precondition_cannot_relax_the_review_pin() {
        let (client, requests) = mock(object(json!({"metadata": {"resourceVersion": "9"}})));
        let mut input = reconcile();
        input["preconditions"] = json!([{
            "jsonPath": ".metadata.resourceVersion", "present": true,
            "reason": "This app says any version will do"
        }]);
        let refused = annotate(client, annotate_in(input))
            .await
            .expect_err("a version nobody reviewed is not written");
        assert!(refused.contains("refresh and review"), "{refused}");
        assert_eq!(requests.lock().unwrap().len(), 1, "nothing was written");
    }

    #[tokio::test]
    async fn a_predicate_the_host_cannot_evaluate_never_reaches_the_cluster() {
        let (client, requests) = mock(object(json!({})));
        let mut input = reconcile();
        input["preconditions"] = json!([{
            "jsonPath": ".status.conditions[?(@.type=='Ready')].status",
            "equals": "True", "reason": "Wait for readiness"
        }]);
        let refused = annotate(client, annotate_in(input))
            .await
            .expect_err("a path the host does not evaluate is refused");
        assert!(refused.contains("not a resource path"), "{refused}");
        assert!(
            requests.lock().unwrap().is_empty(),
            "the object was not even read"
        );
    }

    #[tokio::test]
    async fn the_declared_reason_cannot_reorder_the_host_words_around_it() {
        let (client, _) = mock(object(json!({"spec": {"suspend": true}})));
        let mut input = reconcile();
        input["preconditions"] = json!([{
            "jsonPath": ".spec.suspend", "notEquals": true,
            "reason": "Resume first\u{202e}\u{200b}"
        }]);
        let refused = annotate(client, annotate_in(input))
            .await
            .expect_err("suspended");
        assert!(
            !refused.contains('\u{202e}') && refused.contains("\\u{202e}"),
            "{refused}"
        );
    }

    #[test]
    fn every_primitive_refuses_a_predicate_it_would_not_evaluate() {
        let bad = json!([{"jsonPath": "spec.suspend", "present": true, "reason": "r"}]);
        let good = json!([{"jsonPath": ".spec.suspend", "absent": true, "reason": "r"}]);
        let arguments = |primitive: &str, predicates: &Value| {
            let mut args = match primitive {
                ANNOTATE => json!({"key": "a.example.io/b", "value": "normal"}),
                SET_FIELDS => json!({"fields": {"/spec/suspend": true}}),
                SET_STATUS_CONDITION => json!({
                    "conditionType": "Issuing", "conditionStatus": "True",
                    "reason": "ManuallyTriggered", "message": ""
                }),
                _ => json!({"patch": {"spec": {"suspend": true}}}),
            };
            args["preconditions"] = predicates.clone();
            args
        };
        for primitive in PRIMITIVES {
            let refused = check_bound_arguments(
                primitive,
                arguments(primitive, &bad).as_object().expect("object"),
            )
            .expect_err("a path the host cannot evaluate is refused at install");
            assert!(
                refused.contains("not a resource path"),
                "{primitive}: {refused}"
            );
            check_bound_arguments(
                primitive,
                arguments(primitive, &good).as_object().expect("object"),
            )
            .unwrap_or_else(|why| panic!("{primitive}: {why}"));
        }
    }

    #[test]
    fn preconditions_are_bound_in_number() {
        let one = json!({"jsonPath": ".spec.x", "present": true, "reason": "r"});
        let many: Vec<Value> =
            std::iter::repeat_n(one, srelens_capability::MAX_PREDICATES + 1).collect();
        let refused = check_bound_arguments(
            ANNOTATE,
            json!({"key": "a.example.io/b", "value": "normal", "preconditions": many})
                .as_object()
                .expect("object"),
        )
        .expect_err("a bound list");
        assert!(refused.contains("at most"), "{refused}");
    }
}
