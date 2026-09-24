use crate::{ValidationCode as Code, ValidationError, ValidationErrors};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use srelens_capability::status::{self, StatusRule};
use srelens_capability::{Predicate, ReferenceFormat, MAX_PREDICATES};
use std::collections::{BTreeMap, BTreeSet};

mod cards;
mod settings;
mod versions;
pub use cards::*;
pub use settings::*;
pub use versions::{MAX_BINDING_VERSIONS, MAX_PATH_OVERRIDES};

/// Extension API versions this host implements, oldest first. A manifest is accepted when
/// its `srelensApiVersion` range matches any of them. How versions are added and retired
/// is specified in docs/extensions/specification.md.
pub const SUPPORTED_API_VERSIONS: &[&str] = &["0.3.0", "0.4.0"];

/// The `format` values JSON Schema draft-07 defines.
const STANDARD_FORMATS: &[&str] = &[
    "date-time",
    "date",
    "time",
    "email",
    "idn-email",
    "hostname",
    "idn-hostname",
    "ipv4",
    "ipv6",
    "uri",
    "uri-reference",
    "iri",
    "iri-reference",
    "uri-template",
    "json-pointer",
    "relative-json-pointer",
    "regex",
];

fn strip_nonstandard_formats(value: &mut Value) {
    match value {
        Value::Object(map) => {
            // A string `format` is the keyword; a property named `format` is an object.
            if map
                .get("format")
                .and_then(Value::as_str)
                .is_some_and(|format| !STANDARD_FORMATS.contains(&format))
            {
                map.remove("format");
            }
            map.values_mut().for_each(strip_nonstandard_formats);
        }
        Value::Array(items) => items.iter_mut().for_each(strip_nonstandard_formats),
        _ => {}
    }
}

/// The highest version in `supported` that `range` matches.
pub fn negotiate_api_version_in(
    range: &semver::VersionReq,
    supported: &[&str],
) -> Option<semver::Version> {
    supported
        .iter()
        .filter_map(|version| semver::Version::parse(version).ok())
        .filter(|version| range.matches(version))
        .max()
}

/// The API version this host serves a manifest under, if it supports the manifest's range.
pub fn negotiate_api_version(range: &semver::VersionReq) -> Option<semver::Version> {
    negotiate_api_version_in(range, SUPPORTED_API_VERSIONS)
}

/// Every version in `supported` that `range` matches, oldest first.
pub fn matching_api_versions_in(
    range: &semver::VersionReq,
    supported: &[&str],
) -> Vec<semver::Version> {
    let mut versions: Vec<semver::Version> = supported
        .iter()
        .filter_map(|version| semver::Version::parse(version).ok())
        .filter(|version| range.matches(version))
        .collect();
    versions.sort();
    versions
}

fn unsupported_api(range: &str) -> String {
    format!(
        "extension requires API {range}; host supports {}",
        SUPPORTED_API_VERSIONS.join(", ")
    )
}
/// A manifest field that is not part of every supported API version.
#[derive(Debug, Clone, Copy)]
pub struct ApiField {
    /// Dot-separated from the manifest root; `[]` steps into every element of an array,
    /// and the last segment always names a field.
    pub path: &'static str,
    /// The first API version with the field.
    pub introduced: &'static str,
    /// The first API version without it, when a later line removed or renamed it.
    pub removed: Option<&'static str>,
    /// When set, the entry is about one form of value in a field every line has, rather
    /// than the field itself: only a string at `path` written in this form counts as
    /// using it.
    pub form: Option<ApiForm>,
}

/// A form of value that only some API lines accept in a field they all have, such as a
/// path grammar a later line widened.
#[derive(Debug, Clone, Copy)]
pub struct ApiForm {
    /// What the refusal calls it, e.g. "the `[?(@.key==\"text\")]` filter".
    pub name: &'static str,
    /// Whether a string value is written in this form.
    pub matches: fn(&str) -> bool,
}

/// A field API 0.4 added (#709).
const fn api_0_4(path: &'static str) -> ApiField {
    ApiField {
        path,
        introduced: "0.4.0",
        removed: None,
        form: None,
    }
}

/// The predicate path filter API 0.4 added to a path field API 0.3 already had (#541).
const fn api_0_4_filter(path: &'static str) -> ApiField {
    ApiField {
        form: Some(ApiForm {
            name: "the `[?(@.key==\"text\")]` filter",
            matches: srelens_capability::path_uses_filter,
        }),
        ..api_0_4(path)
    }
}

/// Manifest fields, and forms of a field's value, added or removed after the oldest
/// supported API version. A manifest may use one only when every version its range
/// admits has it. A rename is a removal plus an addition.
pub const API_FIELDS: &[ApiField] = &[
    // Added to API 0.3 in place while the platform was being built, then moved to a line
    // of their own before a signed release used them (#709): a host that implements 0.3
    // without them is told "requires API 0.4" rather than meeting an unknown field.
    api_0_4("contributions.joins"),              // #538
    api_0_4("contributions.tableColumns"),       // #538
    api_0_4("contributions.detailPanels"),       // #539
    api_0_4("contributions.statusResolvers"),    // #541
    api_0_4("contributions.badges"),             // #541
    api_0_4("contributions.dashboardCards"),     // #540
    api_0_4("contributions.commands"),           // #544
    api_0_4("contributions.resourceLinks"),      // #545
    api_0_4("settings"),                         // #542
    api_0_4("capabilities[].versions"),          // #547
    api_0_4("capabilities[].jsonPathOverrides"), // #547
    // The statusResolvers, badges and dashboardCards that also take the filter are
    // already 0.4 fields; these are the predicate paths API 0.3 had.
    api_0_4_filter("actions[].preconditions[].jsonPath"), // #541
    api_0_4_filter("actions[].availableWhen[].jsonPath"), // #541
];

/// Rejects a field in `raw` that is missing from any of `versions`: every supported API
/// version the manifest's range admits. A range that also admits an older line claims
/// hosts on that line, so it may use only fields every admitted line has.
pub fn check_api_fields_in(
    raw: &Value,
    versions: &[semver::Version],
    fields: &[ApiField],
) -> Result<(), String> {
    let parse = |field: &ApiField, version: &str| {
        semver::Version::parse(version)
            .map_err(|e| format!("invalid API version for {}: {e}", field.path))
    };
    for field in fields {
        let values = field_values(raw, field.path);
        let used = match field.form {
            None => values.iter().any(|value| contributes(value)),
            Some(form) => values
                .iter()
                .any(|value| value.as_str().is_some_and(form.matches)),
        };
        if !used {
            continue;
        }
        let what = match field.form {
            None => format!("`{}`", field.path),
            Some(form) => format!("{} in `{}`", form.name, field.path),
        };
        let introduced = parse(field, field.introduced)?;
        let removed = field
            .removed
            .map(|removed| parse(field, removed))
            .transpose()?;
        for version in versions {
            if *version < introduced {
                return Err(format!(
                    "{what} requires API {introduced}, but this manifest's srelensApiVersion admits API {version}"
                ));
            }
            if let Some(removed) = &removed {
                if version >= removed {
                    return Err(format!(
                        "{what} was removed in API {removed}, but this manifest's srelensApiVersion admits API {version}"
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Whether a value counts as using its field. Null, an empty array and an empty object
/// do not: they contribute nothing, and a manifest loaded from storage serializes its
/// unused collection fields as empty.
fn contributes(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Array(items) => !items.is_empty(),
        Value::Object(fields) => !fields.is_empty(),
        _ => true,
    }
}

/// Every value `path` reaches in `value`.
fn field_values<'a>(value: &'a Value, path: &str) -> Vec<&'a Value> {
    let mut nodes = vec![value];
    for segment in path.split('.') {
        let (key, each) = match segment.strip_suffix("[]") {
            Some(key) => (key, true),
            None => (segment, false),
        };
        let mut next = Vec::new();
        for node in nodes {
            match node.get(key) {
                Some(Value::Array(items)) if each => next.extend(items.iter()),
                Some(child) if !each => next.push(child),
                _ => {}
            }
        }
        nodes = next;
    }
    nodes
}

pub const MAX_MANIFEST_BYTES: usize = 256 * 1024;

/// Most capability bindings one manifest may declare.
pub const MAX_CAPABILITIES: usize = 32;

/// Most printer columns a binding may declare (#609). Refused at
/// `capabilities[i].arguments.printerColumns` with `EXTENSION_INVALID_VALUE`.
pub const MAX_PRINTER_COLUMNS: usize = 32;
fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Editor metadata: the JSON Schema the manifest is written against. The host ignores
    /// it; it is the one key allowed that is not part of the contract.
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema_url: Option<String>,
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(rename = "srelensApiVersion")]
    pub api_version: String,
    pub kind: ManifestKind,
    pub permissions: Vec<String>,
    pub capabilities: Vec<Binding>,
    /// Declared mutations (#549). Absent in a manifest that only reads, and
    /// left out of the serialized form when empty so a manifest stored and
    /// signed without it still round-trips to its own bytes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<ActionBinding>,
    /// Typed settings (#542), drawn as a host form and held to these
    /// declarations on every save. Left out of the stored form when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub settings: Vec<Setting>,
    pub contributions: Contributions,
}

/// Most actions one manifest may declare.
pub const MAX_ACTIONS: usize = 32;

/// The inputs an action takes, fixed by the host.
///
/// Not the app's to choose, unlike a reader binding's `inputs`: everything a
/// caller may vary about a declared mutation is the object it names and the
/// version of that object the operator reviewed. An app that could expose its
/// own input would be back to sending a Kubernetes request the host did not
/// write.
pub const ACTION_INPUTS: &[&str] = &["context", "namespace", "name", "uid", "resourceVersion"];

/// The arguments the host fills in from the reader binding an action names,
/// which is what restricts an action to a kind the app already holds a granted
/// reader for. An action that bound any of these itself would choose its own
/// kind, so binding one is refused.
pub const ACTION_IDENTITY: &[&str] = &["group", "version", "plural", "kind", "namespaced"];

/// Built-in summary readers usable to scope the host's narrow operational actions.
/// Identity comes from the host, never from an app's bound arguments.
pub fn builtin_reader_identity(target: &str) -> Option<Map<String, Value>> {
    let (group, plural, kind, namespaced) = match target {
        "k8s.listDeployments" => ("apps", "deployments", "Deployment", true),
        "k8s.listStatefulSets" => ("apps", "statefulsets", "StatefulSet", true),
        "k8s.listDaemonSets" => ("apps", "daemonsets", "DaemonSet", true),
        "k8s.listNodes" => ("", "nodes", "Node", false),
        _ => return None,
    };
    Some(serde_json::json!({"group":group,"version":"v1","plural":plural,"kind":kind,"namespaced":namespaced})
        .as_object().expect("object").clone())
}

/// The predicate lists an action declares in fields of its own (#550), which
/// is why binding one as an argument is refused: two spellings of one
/// declaration would leave the host reading whichever it happened to look at.
pub const ACTION_PREDICATES: &[&str] = &["preconditions", "availableWhen"];

/// One declared mutation: a host action primitive, the reader binding whose
/// kind it acts on, the arguments that fix what it writes, and what must be
/// true of the object before it is written.
///
/// Flux and Argo CD declare their actions using this contract in API 0.3.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActionBinding {
    pub name: String,
    pub title: String,
    /// A host action primitive, e.g. `k8s.annotate`.
    pub target: String,
    /// The name of a reader binding in `capabilities`. The action acts on that
    /// binding's kind and on no other.
    pub resource: String,
    /// What this action writes, fixed at install time.
    pub arguments: Map<String, Value>,
    /// What must be true of the object for the write to be sent. Evaluated by
    /// the host against the fresh GET the primitive already performs, before
    /// the patch; a predicate that does not hold refuses the request and the
    /// operator is told this predicate's `reason`.
    ///
    /// These add refusals. The host's own guards — an object being deleted, a
    /// UID or `resourceVersion` that has moved — run first and are not
    /// something a manifest can reach.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preconditions: Vec<Predicate>,
    /// What must be true of the object for the control to be offered. The same
    /// predicates, asked by the surface rather than by the cluster request, so
    /// a person is not shown a button whose refusal is already known.
    ///
    /// Display only, and deliberately so: an app that needs a condition
    /// *enforced* declares it in `preconditions`, where the host is what
    /// checks it. A surface can be out of date; the fresh GET cannot.
    #[serde(
        default,
        rename = "availableWhen",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub available_when: Vec<Predicate>,
}

/// Only data is executable in this first host. Code-bearing manifests must go
/// through the future sandboxed runtime, never through a permissive fallback.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ManifestKind {
    Declarative,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub name: String,
    pub title: String,
    pub target: String,
    /// A `k8s.listCustomResource` reader's API versions, most preferred first, instead of
    /// one fixed `arguments.version` (#547). On each cluster the host reads the first one
    /// the CustomResourceDefinition serves, and refuses the read when it serves none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub versions: Vec<String>,
    /// For a listed version whose objects keep a field elsewhere: each JSONPath this
    /// manifest reads the binding's objects through, mapped to the path to read at that
    /// version instead. Applied everywhere the objects are read: printer columns, status
    /// resolvers, declared actions, joined columns, badges and panels, and cards.
    #[serde(
        default,
        rename = "jsonPathOverrides",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub json_path_overrides: BTreeMap<String, BTreeMap<String, String>>,
    pub arguments: Map<String, Value>,
    pub inputs: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Contributions {
    pub pages: Vec<Page>,
    #[serde(rename = "detailTabs")]
    pub detail_tabs: Vec<DetailTab>,
    #[serde(rename = "detailLinks")]
    pub detail_links: Vec<DetailLink>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub joins: Vec<Join>,
    #[serde(
        default,
        rename = "tableColumns",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub table_columns: Vec<TableColumn>,
    #[serde(
        default,
        rename = "dashboardCards",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub dashboard_cards: Vec<DashboardCard>,
    #[serde(
        default,
        rename = "detailPanels",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub detail_panels: Vec<DetailPanel>,
    /// What status an app's custom resources have (#541). Replaces
    /// `pages[].statusColumns`, which is deprecated but still accepted in 0.3.
    #[serde(
        default,
        rename = "statusResolvers",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub status_resolvers: Vec<StatusResolver>,
    /// Words an app puts on built-in rows, from the row's metadata or from a
    /// joined resource (#541).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub badges: Vec<Badge>,
    /// Entries in the command palette (#544): open one of this app's pages,
    /// or run one of its declared actions on the resource the reader has open.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<PaletteCommand>,
    /// Relationships from one kind to another an app knows how to find (#545):
    /// the Inspector's "Related" links, and the edges a topology can draw.
    #[serde(
        default,
        rename = "resourceLinks",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub resource_links: Vec<ResourceLink>,
}

/// Most palette commands one manifest may declare.
pub const MAX_COMMANDS: usize = 32;

/// One command palette entry. The host shows it under the app's name, so an
/// app cannot pass its command off as the host's own.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PaletteCommand {
    pub id: String,
    pub title: String,
    pub target: CommandTarget,
    /// For an action command: the qualified kinds it is offered on, which must
    /// be the kind of the reader binding its action acts on. A page command
    /// has no subject and takes none.
    #[serde(default, rename = "forKinds", skip_serializing_if = "Vec::is_empty")]
    pub for_kinds: Vec<String>,
}

/// What a palette command does: exactly one of opening a declared page or
/// running a declared action through the host's confirmation.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum CommandTarget {
    /// The `id` of a page in `contributions.pages`.
    Page(String),
    /// The `name` of a declared action in `actions`.
    Action(String),
}

/// Most resource links one manifest may declare.
pub const MAX_RESOURCE_LINKS: usize = 32;

/// A relationship from a resource of kind `from` to resources of kind `to`,
/// found by reading the `from` resource's own metadata (#545).
///
/// The `match` selectors are a join's (`joins[].match`), read the other way
/// round: a join indexes the *listed* resources by a key that names the row,
/// while a link reads the key on the resource being inspected, and the key
/// names the target. The target is then looked up by name in the list of the
/// declared reader for `to`, through the same index a join uses.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResourceLink {
    pub id: String,
    /// The qualified kind of the resource the link is read from, e.g. `apps/Deployment`.
    pub from: String,
    /// The qualified kind of the target. A declared `k8s.listCustomResource`
    /// reader must list it: that is how the host knows the target exists.
    pub to: String,
    pub relation: LinkRelation,
    #[serde(rename = "match")]
    pub match_by: LinkMatch,
}

/// What the `from` resource is to the `to` resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum LinkRelation {
    OwnedBy,
    ManagedBy,
    ExposedBy,
    References,
}

/// Where on the `from` resource the target's name is written. Exactly one of
/// `label`, `ownerReference`, `annotation` and `name`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LinkMatch {
    /// The label whose value is the target's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// With `label`: the label whose value is the target's namespace. Without
    /// it the target is in the `from` resource's namespace.
    #[serde(
        default,
        rename = "namespaceLabel",
        skip_serializing_if = "Option::is_none"
    )]
    pub namespace_label: Option<String>,
    /// The `from` resource's owner references of the `to` kind.
    #[serde(default, rename = "ownerReference", skip_serializing_if = "is_false")]
    pub owner_reference: bool,
    /// The annotation whose value is the target's name, or, with `parse`, a
    /// reference in a host-known format that names it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotation: Option<String>,
    /// With `annotation`: the format the value is written in. The host
    /// accepts a reference only when it names the `from` resource itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parse: Option<ReferenceFormat>,
    /// With `parse: "argocd-tracking-id"`: the namespace Argo CD runs in,
    /// where an application written as a bare name lives. Without it a bare
    /// name is shown unverified, never searched for across namespaces.
    #[serde(
        default,
        rename = "defaultNamespace",
        skip_serializing_if = "Option::is_none"
    )]
    pub default_namespace: Option<String>,
    /// The target has the `from` resource's own name and namespace.
    #[serde(default, skip_serializing_if = "is_false")]
    pub name: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DetailPanel {
    pub id: String,
    pub title: String,
    #[serde(rename = "forKinds")]
    pub for_kinds: Vec<String>,
    pub sections: Vec<DetailSection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum DetailSection {
    Fields {
        fields: Vec<DetailField>,
    },
    Conditions {
        #[serde(rename = "jsonPath")]
        json_path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        join: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DetailField {
    pub label: String,
    #[serde(rename = "jsonPath")]
    pub json_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub join: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<ColumnFormat>,
}

/// Most status resolvers, and most badges, one manifest may declare.
pub const MAX_STATUS_CONTRIBUTIONS: usize = 16;

/// Ordered rules that resolve the status of the custom resources of `forKinds`.
/// The first rule whose conditions all hold is the status; when none does the
/// host says `unknown`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StatusResolver {
    /// Qualified kinds of custom resources this app declares a reader for.
    #[serde(rename = "forKinds")]
    pub for_kinds: Vec<String>,
    pub rules: Vec<StatusRule>,
}

/// A word on a built-in row. Without a `join`, the rules read the row's own
/// `.metadata`; with one, they read the joined custom resource. The first rule
/// that holds is the badge; when none does there is no badge.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Badge {
    pub id: String,
    /// Qualified built-in kinds, e.g. `apps/Deployment`.
    #[serde(rename = "forKinds")]
    pub for_kinds: Vec<String>,
    /// A declared join whose matched resource the rules read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub join: Option<String>,
    pub rules: Vec<StatusRule>,
}

/// Whether a direct badge's path stays inside the row's metadata, which is
/// all of a built-in row the host reads on an app's behalf.
fn metadata_path(path: &str) -> bool {
    let path = path.strip_prefix('$').unwrap_or(path);
    path == ".metadata"
        || path.starts_with(".metadata.")
        || path.starts_with(".metadata[")
        || path.starts_with("['metadata']")
        || path.starts_with("[\"metadata\"]")
}

/// Reports a status rule list's problems at the manifest path that owns it.
fn rule_list_problems(problems: &mut ValidationErrors, at: &str, rules: &[StatusRule]) {
    for (path, why) in status::rule_problems(rules) {
        // A bad condition is a binding problem, as a bad predicate is; a
        // count, a word or an unreadable reason path is a value out of range.
        let code = if path.contains(".when[") {
            Code::InvalidBinding
        } else {
            Code::InvalidValue
        };
        problems.push(code, format!("{at}.{path}"), why);
    }
}

/// A single granted custom-resource list used to enrich native table rows.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Join {
    pub id: String,
    pub capability: String,
    #[serde(rename = "match")]
    pub match_by: JoinMatch,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JoinMatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, rename = "kindLabel", skip_serializing_if = "Option::is_none")]
    pub kind_label: Option<String>,
    #[serde(default, rename = "ownerReference", skip_serializing_if = "is_false")]
    pub owner_reference: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotation: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub name: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TableColumn {
    pub id: String,
    pub title: String,
    #[serde(rename = "forKinds")]
    pub for_kinds: Vec<String>,
    pub source: ColumnSource,
    pub format: ColumnFormat,
    #[serde(default, skip_serializing_if = "is_false")]
    pub sortable: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub filterable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ColumnSource {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub join: Option<String>,
    #[serde(rename = "jsonPath")]
    pub json_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ColumnFormat {
    Text,
    Number,
    Status,
    Badge,
    Date,
    Duration,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Page {
    pub id: String,
    pub title: String,
    pub capability: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(
        default,
        rename = "statusColumns",
        skip_serializing_if = "Option::is_none"
    )]
    pub status_columns: Option<StatusColumns>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dashboard: Option<Dashboard>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StatusColumns {
    pub ready: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suspended: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progressing: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Dashboard {
    pub pages: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub events: Option<DashboardEvents>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DashboardEvents {
    pub capability: String,
    #[serde(rename = "apiGroups")]
    pub api_groups: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DetailTab {
    pub id: String,
    pub title: String,
    pub capability: String,
    /// Qualified Kubernetes kinds, e.g. argoproj.io/Application.
    #[serde(rename = "forKinds")]
    pub for_kinds: Vec<String>,
}

/// An entry in a resource detail view's app links menu that opens a read-only results
/// panel. It never writes to the cluster; the name `rowActions` is reserved for declared
/// mutations.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DetailLink {
    pub id: String,
    pub title: String,
    pub capability: String,
    #[serde(rename = "forKinds")]
    pub for_kinds: Vec<String>,
}

/// Reports every declared predicate the host will not evaluate, at the field
/// that has to change.
///
/// The rule itself is `Predicate::check` in `srelens-capability`, which the
/// action primitives run again on the way to the cluster. This is the same
/// rule read at the place a person can fix it, not a second one: a list that
/// passes here cannot be refused there, and one refused there could not have
/// been installed.
fn predicate_problems(
    problems: &mut ValidationErrors,
    at: &str,
    field: &str,
    declared: &[Predicate],
) {
    if declared.len() > MAX_PREDICATES {
        problems.push(
            Code::InvalidValue,
            format!("{at}.{field}"),
            format!("Declare at most {MAX_PREDICATES} predicates"),
        );
        return;
    }
    for (index, predicate) in declared.iter().enumerate() {
        if let Err(why) = predicate.check() {
            problems.push(Code::InvalidBinding, format!("{at}.{field}[{index}]"), why);
        }
    }
}

/// A Kubernetes namespace name: an RFC 1123 label of 1–63 lowercase letters,
/// digits and `-`, starting and ending with a letter or digit.
pub fn namespace_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 63
        && value
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        && !value.starts_with('-')
        && !value.ends_with('-')
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
}
/// Whether `c` is a Unicode format character (category Cf), such as a right-to-left
/// override or a zero-width space. Text shown as an app's identity refuses them, because
/// they can make it display differently from what it holds.
///
/// Re-exported from `srelens-capability`, which holds the single copy of the ranges: the
/// confirmation sentences a capability renders (#661) need the same rule and this crate
/// sits above that one, so the table moved down rather than being written twice.
pub use srelens_capability::is_format_character;

fn label(value: &str) -> bool {
    !value.trim().is_empty()
        && value.len() <= 120
        && !value
            .chars()
            .any(|c| c.is_control() || is_format_character(c))
}
/// Records a problem for each value already seen, and returns the distinct values.
fn unique<'a>(
    problems: &mut ValidationErrors,
    values: impl IntoIterator<Item = (String, &'a str)>,
) -> BTreeSet<&'a str> {
    let mut seen = BTreeSet::new();
    for (path, value) in values {
        if !seen.insert(value) {
            problems.push(
                Code::DuplicateIdentifier,
                path,
                format!("\"{value}\" is listed more than once"),
            );
        }
    }
    seen
}
/// Reports `value` at `at` unless it is a qualified kind, `group/Kind`.
fn qualified_kind(problems: &mut ValidationErrors, at: String, value: &str) {
    match value.split_once('/') {
        None => problems.push(
            Code::InvalidKind,
            at,
            format!("Qualify \"{value}\" with its API group, for example apps/Deployment, or /Pod for the core group"),
        ),
        Some((group, kind))
            if !identifier(kind) || (!group.is_empty() && !group.split('.').all(identifier)) =>
        {
            problems.push(Code::InvalidKind, at, format!("\"{value}\" is not a qualified Kubernetes kind"))
        }
        Some(_) => {}
    }
}
fn kinds(problems: &mut ValidationErrors, path: &str, values: &[String]) {
    if values.is_empty() || values.len() > 32 {
        problems.push(
            Code::InvalidValue,
            path,
            "forKinds must list 1–32 qualified kinds",
        );
    }
    for (index, value) in values.iter().enumerate() {
        qualified_kind(problems, format!("{path}[{index}]"), value);
    }
    unique(
        problems,
        values
            .iter()
            .enumerate()
            .map(|(index, value)| (format!("{path}[{index}]"), value.as_str())),
    );
}

/// Maps a schema error to the field it names, without serde's line and column.
/// Check the structural subset accepted by the host's scalar JSONPath reader.
/// A typo must fail at install time instead of becoming an unexplained empty cell.
fn column_json_path(path: &str) -> bool {
    if path.len() > 256
        || !path.starts_with('.')
        || path.len() < 2
        || path
            .chars()
            .any(|c| c.is_control() || is_format_character(c))
    {
        return false;
    }
    let chars: Vec<char> = path.chars().collect();
    let mut index = 1;
    while index < chars.len() {
        if chars[index] == '[' {
            index += 1;
            let start = index;
            while index < chars.len() && chars[index] != ']' {
                index += 1;
            }
            if index == chars.len() || index == start {
                return false;
            }
            let inner = &chars[start..index];
            if (inner[0] == '\'' && inner.last() != Some(&'\''))
                || (inner[0] == '"' && inner.last() != Some(&'"'))
            {
                return false;
            }
            index += 1;
        } else {
            let start = index;
            while index < chars.len() && chars[index] != '.' && chars[index] != '[' {
                if chars[index] == ']' {
                    return false;
                }
                if chars[index] == '\\' {
                    index += 1;
                    if index == chars.len() {
                        return false;
                    }
                }
                index += 1;
            }
            if index == start {
                return false;
            }
        }
        if index < chars.len() && chars[index] == '.' {
            index += 1;
            if index == chars.len() || chars[index] == '.' {
                return false;
            }
        }
    }
    true
}

/// Conditions resolve to an array, so their path stays on plain object keys.
fn condition_json_path(path: &str) -> bool {
    column_json_path(path)
        && path.strip_prefix('.').is_some_and(|tail| {
            tail.split('.').all(|key| {
                !key.is_empty()
                    && key
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
            })
        })
}

fn schema_error(error: &serde_path_to_error::Error<serde_json::Error>) -> ValidationError {
    let inner = error.inner();
    let mut message = inner.to_string();
    let position = format!(" at line {} column {}", inner.line(), inner.column());
    if let Some(stripped) = message.strip_suffix(&position) {
        message = stripped.to_owned();
    }
    let mut path = error.path().to_string();
    if path == "." {
        path.clear();
    }
    let named = |prefix: &str| {
        message
            .strip_prefix(prefix)
            .and_then(|rest| rest.split('`').next())
            .map(str::to_owned)
    };
    let (code, field) = if let Some(field) = named("unknown field `") {
        (Code::UnknownField, Some(field))
    } else if let Some(field) = named("missing field `") {
        (Code::InvalidField, Some(field))
    } else if path == "kind" && message.starts_with("unknown variant") {
        (Code::InvalidKind, None)
    } else {
        (Code::InvalidField, None)
    };
    // A missing field is reported at the object that lacks it; point at the field.
    if let Some(field) = field {
        if path != field && !path.ends_with(&format!(".{field}")) {
            path = if path.is_empty() {
                field
            } else {
                format!("{path}.{field}")
            };
        }
    }
    ValidationError::new(code, path, message)
}

impl Manifest {
    /// Decodes and validates a manifest, reporting every problem found.
    pub fn parse(source: &str) -> Result<Self, ValidationErrors> {
        let manifest = Self::decode(source)?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Decodes a manifest against the schema without checking its rules. A schema problem
    /// stops decoding, so those are reported one at a time; [`Manifest::validate`] then
    /// reports every rule violation together.
    pub fn decode(source: &str) -> Result<Self, ValidationErrors> {
        if source.len() > MAX_MANIFEST_BYTES {
            return Err(
                ValidationError::new(Code::TooLarge, "", "Manifest exceeds 256 KiB").into(),
            );
        }
        let raw: Value = serde_json::from_str(source).map_err(|e| {
            ValidationError::new(
                Code::InvalidJson,
                "",
                format!("Manifest is not valid JSON: {e}"),
            )
        })?;
        let unsupported = raw
            .get("srelensApiVersion")
            .and_then(Value::as_str)
            .filter(|range| {
                semver::VersionReq::parse(range).is_ok_and(|req| {
                    matching_api_versions_in(&req, SUPPORTED_API_VERSIONS).is_empty()
                })
            });
        // Decode the source, not `raw`: a repeated key is an error rather than last-wins.
        let mut deserializer = serde_json::Deserializer::from_str(source);
        serde_path_to_error::deserialize(&mut deserializer).map_err(|error| {
            // A manifest written for an API this host lacks is told the version it needs,
            // not the field this host does not know. One that still decodes goes on to
            // `validate`, which reports the version together with every other problem.
            let problem = match unsupported {
                Some(range) => ValidationError::new(
                    Code::ApiIncompatible,
                    "srelensApiVersion",
                    unsupported_api(range),
                ),
                None => schema_error(&error),
            };
            ValidationErrors::from(problem)
        })
    }

    /// The published JSON Schema. schemars annotates Rust integers with formats such as
    /// `uint`, which JSON Schema does not define and strict validators reject; the
    /// `minimum` it emits beside them already carries the constraint.
    pub fn schema() -> Value {
        let mut schema =
            serde_json::to_value(schemars::schema_for!(Self)).expect("manifest schema serializes");
        strip_nonstandard_formats(&mut schema);
        schema
    }

    /// Rejects a field the manifest uses that is missing from any version in `supported`
    /// its range admits. It works on a manifest loaded from storage as well as a parsed one,
    /// so inventory reverification applies the same rules as installation.
    pub fn check_api_fields(&self, supported: &[&str], fields: &[ApiField]) -> Result<(), String> {
        let range = semver::VersionReq::parse(&self.api_version)
            .map_err(|e| format!("invalid srelensApiVersion: {e}"))?;
        let admitted = matching_api_versions_in(&range, supported);
        let value = serde_json::to_value(self).map_err(|e| e.to_string())?;
        check_api_fields_in(&value, &admitted, fields)
    }

    /// The qualified kind (`group/Kind`) a custom-resource reader binding
    /// lists, from its bound `group` and `kind`.
    pub fn reader_kind(binding: &Binding) -> Option<String> {
        if binding.target != "k8s.listCustomResource" {
            return None;
        }
        let group = binding.arguments.get("group")?.as_str()?;
        let kind = binding.arguments.get("kind")?.as_str()?;
        Some(format!("{group}/{kind}"))
    }

    /// The status rules declared for `kind` (`group/Kind`), if any.
    ///
    /// The per-kind lookup #540's `countByStatus` and every list read share;
    /// validation allows one resolver per kind, so there is one answer.
    pub fn status_rules_for(&self, kind: &str) -> Option<&[StatusRule]> {
        self.contributions
            .status_resolvers
            .iter()
            .find(|resolver| resolver.for_kinds.iter().any(|k| k == kind))
            .map(|resolver| resolver.rules.as_slice())
    }

    /// The status rules for the kind the named reader binding lists.
    pub fn status_rules_for_binding(&self, capability: &str) -> Option<&[StatusRule]> {
        let binding = self.capabilities.iter().find(|b| b.name == capability)?;
        self.status_rules_for(&Self::reader_kind(binding)?)
    }

    /// The binding the host registers for one declared action: the primitive
    /// it names, the identity of the reader binding it acts through, and its
    /// own arguments, with the inputs the host fixes.
    ///
    /// This is where "an action reaches only the kind of a granted reader
    /// binding" is true rather than merely intended: the kind is *copied* from
    /// that binding, so there is no field an app could write it in.
    pub fn action_binding(&self, action: &ActionBinding) -> Result<Binding, String> {
        let reader = self
            .capabilities
            .iter()
            .find(|binding| binding.name == action.resource)
            .ok_or_else(|| format!("\"{}\" is not a declared capability", action.resource))?;
        // Which version the action writes is chosen per cluster (#547); until then there
        // is no version to write, and no binding.
        if !reader.versions.is_empty() {
            return Err(format!(
                "\"{}\" reads one of {}; an action on it is bound once a cluster resolves one",
                action.resource,
                reader.versions.join(", ")
            ));
        }
        let mut arguments = Map::new();
        let builtin = builtin_reader_identity(&reader.target);
        if let Some(identity) = &builtin {
            let target = if identity["kind"] == "Node" {
                "k8s.requestCordonNode"
            } else {
                "k8s.requestRolloutRestart"
            };
            if action.target != target {
                return Err(format!("{} scopes only {target}", reader.target));
            }
        }
        let identity = builtin.unwrap_or_else(|| reader.arguments.clone());
        for key in ACTION_IDENTITY {
            let Some(value) = identity.get(*key) else {
                return Err(format!(
                    "\"{}\" does not fix `{key}`, so it cannot scope an action to one kind",
                    action.resource
                ));
            };
            arguments.insert((*key).to_owned(), value.clone());
        }
        for (key, value) in &action.arguments {
            if ACTION_IDENTITY.contains(&key.as_str())
                || ACTION_INPUTS.contains(&key.as_str())
                || ACTION_PREDICATES.contains(&key.as_str())
            {
                return Err(format!("`{key}` is filled in by the host"));
            }
            arguments.insert(key.clone(), value.clone());
        }
        // The checks the host makes before it patches, carried the same way
        // the kind is: copied out of the declaration into the binding, so the
        // primitive is handed them rather than trusting a caller to pass them.
        // `availableWhen` is not here — it decides whether a control is
        // offered, and no cluster request needs it.
        if !action.preconditions.is_empty() {
            arguments.insert(
                "preconditions".to_owned(),
                serde_json::to_value(&action.preconditions)
                    .map_err(|e| format!("preconditions: {e}"))?,
            );
        }
        Ok(Binding {
            name: action.name.clone(),
            title: action.title.clone(),
            target: action.target.clone(),
            versions: Vec::new(),
            json_path_overrides: BTreeMap::new(),
            arguments,
            inputs: ACTION_INPUTS.iter().map(|i| (*i).to_owned()).collect(),
        })
    }

    /// Checks the manifest's rules, reporting every violation with the path at fault.
    pub fn validate(&self) -> Result<(), ValidationErrors> {
        let mut problems = self.rule_problems();
        self.version_problems(&mut problems);
        self.override_path_problems(&mut problems);
        problems.into_result()
    }

    /// Every rule violation except those about a binding's `versions` and
    /// `jsonPathOverrides` (#547). Those are checked once, by `validate`; an override is
    /// checked by running these rules on the manifest read at its version, which must
    /// not check every other binding's overrides again for each one.
    fn rule_problems(&self) -> ValidationErrors {
        const LABEL: &str =
            "Must be 1–120 characters with no control characters and no bidirectional or invisible format characters";
        const IDENTIFIER: &str = "Must be 1–64 letters, digits and -";
        let mut problems = ValidationErrors::default();
        if self.id.len() > 128 || !self.id.contains('.') || !self.id.split('.').all(identifier) {
            problems.push(
                Code::InvalidId,
                "id",
                "App ID must be reverse-domain: at least two dot-separated segments of letters, digits and -, at most 128 characters",
            );
        }
        if !label(&self.name) {
            problems.push(Code::InvalidValue, "name", LABEL);
        }
        if let Err(e) = semver::Version::parse(&self.version) {
            problems.push(
                Code::InvalidVersion,
                "version",
                format!("Version must be SemVer: {e}"),
            );
        }
        match semver::VersionReq::parse(&self.api_version) {
            Err(e) => problems.push(
                Code::InvalidVersion,
                "srelensApiVersion",
                format!("srelensApiVersion must be a SemVer range: {e}"),
            ),
            Ok(range) if negotiate_api_version(&range).is_none() => problems.push(
                Code::ApiIncompatible,
                "srelensApiVersion",
                unsupported_api(&self.api_version),
            ),
            // Stored manifests are rechecked here as well, so an app using a field a newer
            // host no longer admits is quarantined rather than left enabled.
            Ok(_) => {
                if let Err(reason) = self.check_api_fields(SUPPORTED_API_VERSIONS, API_FIELDS) {
                    problems.push(Code::ApiIncompatible, "srelensApiVersion", reason);
                }
            }
        }
        if self.capabilities.is_empty() || self.capabilities.len() > MAX_CAPABILITIES {
            problems.push(
                Code::InvalidValue,
                "capabilities",
                format!("Declare 1–{MAX_CAPABILITIES} capabilities"),
            );
        }
        let names = unique(
            &mut problems,
            self.capabilities
                .iter()
                .enumerate()
                .map(|(index, c)| (format!("capabilities[{index}].name"), c.name.as_str())),
        );
        let permissions = unique(
            &mut problems,
            self.permissions
                .iter()
                .enumerate()
                .map(|(index, p)| (format!("permissions[{index}]"), p.as_str())),
        );
        let mut targets = BTreeSet::new();
        for (index, binding) in self.capabilities.iter().enumerate() {
            let at = format!("capabilities[{index}]");
            if !identifier(&binding.name) {
                problems.push(Code::InvalidValue, format!("{at}.name"), IDENTIFIER);
            }
            if !label(&binding.title) {
                problems.push(Code::InvalidValue, format!("{at}.title"), LABEL);
            }
            if binding.target.starts_with("plugin/") {
                problems.push(
                    Code::UnsupportedTarget,
                    format!("{at}.target"),
                    "An app cannot forward to another app's capability",
                );
            }
            targets.insert(binding.target.as_str());
            unique(
                &mut problems,
                binding
                    .inputs
                    .iter()
                    .enumerate()
                    .map(|(position, input)| (format!("{at}.inputs[{position}]"), input.as_str())),
            );
            for (position, input) in binding.inputs.iter().enumerate() {
                if binding.arguments.contains_key(input) {
                    problems.push(
                        Code::InvalidBinding,
                        format!("{at}.inputs[{position}]"),
                        format!("\"{input}\" is a bound argument, so it cannot also be an input"),
                    );
                }
            }
            if let Some(columns) = binding
                .arguments
                .get("printerColumns")
                .and_then(Value::as_array)
            {
                if columns.len() > MAX_PRINTER_COLUMNS {
                    problems.push(
                        Code::InvalidValue,
                        format!("{at}.arguments.printerColumns"),
                        format!("Declare at most {MAX_PRINTER_COLUMNS} printerColumns"),
                    );
                }
            }
        }
        if self.actions.len() > MAX_ACTIONS {
            problems.push(
                Code::InvalidValue,
                "actions",
                format!("Declare at most {MAX_ACTIONS} actions"),
            );
        }
        // An action's name becomes a capability id beside the readers', so the
        // two share one name space.
        let mut declared: BTreeSet<&str> = names.clone();
        for (index, action) in self.actions.iter().enumerate() {
            let at = format!("actions[{index}]");
            if !identifier(&action.name) {
                problems.push(Code::InvalidValue, format!("{at}.name"), IDENTIFIER);
            } else if !declared.insert(action.name.as_str()) {
                problems.push(
                    Code::DuplicateIdentifier,
                    format!("{at}.name"),
                    format!("\"{}\" is already used by another capability", action.name),
                );
            }
            if !label(&action.title) {
                problems.push(Code::InvalidValue, format!("{at}.title"), LABEL);
            }
            if action.target.starts_with("plugin/") {
                problems.push(
                    Code::UnsupportedTarget,
                    format!("{at}.target"),
                    "An app cannot forward to another app's capability",
                );
            }
            targets.insert(action.target.as_str());
            for key in action.arguments.keys() {
                if ACTION_IDENTITY.contains(&key.as_str()) || ACTION_INPUTS.contains(&key.as_str())
                {
                    problems.push(
                        Code::InvalidBinding,
                        format!("{at}.arguments.{key}"),
                        format!(
                            "`{key}` is filled in by the host, from the reader binding this action names"
                        ),
                    );
                } else if ACTION_PREDICATES.contains(&key.as_str()) {
                    problems.push(
                        Code::InvalidBinding,
                        format!("{at}.arguments.{key}"),
                        format!(
                            "`{key}` is declared in the action's own `{key}`, not as an argument"
                        ),
                    );
                }
            }
            predicate_problems(&mut problems, &at, "preconditions", &action.preconditions);
            predicate_problems(&mut problems, &at, "availableWhen", &action.available_when);
            if !names.contains(action.resource.as_str()) {
                problems.push(
                    Code::UnresolvedCapability,
                    format!("{at}.resource"),
                    format!("Capability \"{}\" is not declared", action.resource),
                );
            }
        }
        if targets != permissions {
            let targets: Vec<_> = targets.into_iter().collect();
            problems.push(
                Code::PermissionMismatch,
                "permissions",
                format!(
                    "permissions must name exactly the bound host capabilities: {}",
                    targets.join(", ")
                ),
            );
        }
        let contributions: [(&str, Vec<(&String, &String, &String)>); 3] = [
            (
                "pages",
                self.contributions
                    .pages
                    .iter()
                    .map(|p| (&p.id, &p.title, &p.capability))
                    .collect(),
            ),
            (
                "detailTabs",
                self.contributions
                    .detail_tabs
                    .iter()
                    .map(|p| (&p.id, &p.title, &p.capability))
                    .collect(),
            ),
            (
                "detailLinks",
                self.contributions
                    .detail_links
                    .iter()
                    .map(|p| (&p.id, &p.title, &p.capability))
                    .collect(),
            ),
        ];
        if contributions
            .iter()
            .map(|(_, entries)| entries.len())
            .sum::<usize>()
            > 64
        {
            problems.push(
                Code::InvalidValue,
                "contributions",
                "Declare at most 64 pages, detail tabs and detail links in total",
            );
        }
        let mut ids = BTreeSet::new();
        for (list, entries) in &contributions {
            for (index, (id, title, capability)) in entries.iter().enumerate() {
                let at = format!("contributions.{list}[{index}]");
                if !identifier(id) {
                    problems.push(Code::InvalidValue, format!("{at}.id"), IDENTIFIER);
                } else if !ids.insert(id.as_str()) {
                    problems.push(
                        Code::DuplicateIdentifier,
                        format!("{at}.id"),
                        format!(
                            "\"{id}\" is already used by another page, detail tab or detail link"
                        ),
                    );
                }
                if !label(title) {
                    problems.push(Code::InvalidValue, format!("{at}.title"), LABEL);
                }
                if !names.contains(capability.as_str()) {
                    problems.push(
                        Code::UnresolvedCapability,
                        format!("{at}.capability"),
                        format!("Capability \"{capability}\" is not declared"),
                    );
                }
            }
        }
        for (index, page) in self.contributions.pages.iter().enumerate() {
            let at = format!("contributions.pages[{index}]");
            if page.group.as_ref().is_some_and(|group| !label(group)) {
                problems.push(Code::InvalidValue, format!("{at}.group"), LABEL);
            }
            if let Some(status) = &page.status_columns {
                for (field, column) in [
                    ("ready", Some(status.ready)),
                    ("suspended", status.suspended),
                    ("progressing", status.progressing),
                ] {
                    if column.is_some_and(|column| column >= 64) {
                        problems.push(
                            Code::InvalidValue,
                            format!("{at}.statusColumns.{field}"),
                            "Status column index must be below 64",
                        );
                    }
                }
            }
            let Some(dashboard) = &page.dashboard else {
                continue;
            };
            if dashboard.pages.is_empty() || dashboard.pages.len() > 12 {
                problems.push(
                    Code::InvalidValue,
                    format!("{at}.dashboard.pages"),
                    "A dashboard references 1–12 resource pages",
                );
            }
            unique(
                &mut problems,
                dashboard.pages.iter().enumerate().map(|(position, id)| {
                    (format!("{at}.dashboard.pages[{position}]"), id.as_str())
                }),
            );
            for (position, id) in dashboard.pages.iter().enumerate() {
                let path = format!("{at}.dashboard.pages[{position}]");
                match self.contributions.pages.iter().find(|p| &p.id == id) {
                    None => problems.push(
                        Code::UnresolvedPage,
                        path,
                        format!("Page \"{id}\" is not declared"),
                    ),
                    Some(target)
                        if target.dashboard.is_some()
                            || (target.status_columns.is_none()
                                && self.status_rules_for_binding(&target.capability).is_none()) =>
                    {
                        problems.push(
                            Code::InvalidValue,
                            path,
                            format!("Page \"{id}\" must be a resource page whose kind has a status resolver"),
                        )
                    }
                    Some(_) => {}
                }
            }
            let Some(events) = &dashboard.events else {
                continue;
            };
            let at = format!("{at}.dashboard.events");
            if !self
                .capabilities
                .iter()
                .any(|c| c.name == events.capability && c.target == "k8s.listEvents")
            {
                problems.push(
                    Code::UnresolvedCapability,
                    format!("{at}.capability"),
                    format!(
                        "\"{}\" is not a declared k8s.listEvents capability",
                        events.capability
                    ),
                );
            }
            if events.api_groups.is_empty() || events.api_groups.len() > 32 {
                problems.push(
                    Code::InvalidValue,
                    format!("{at}.apiGroups"),
                    "List 1–32 API groups",
                );
            }
            for (position, group) in events.api_groups.iter().enumerate() {
                if !group.contains('.') || !group.split('.').all(identifier) {
                    problems.push(
                        Code::InvalidValue,
                        format!("{at}.apiGroups[{position}]"),
                        format!(
                            "\"{group}\" is not an API group, for example source.toolkit.fluxcd.io"
                        ),
                    );
                }
            }
            unique(
                &mut problems,
                events
                    .api_groups
                    .iter()
                    .enumerate()
                    .map(|(position, group)| {
                        (format!("{at}.apiGroups[{position}]"), group.as_str())
                    }),
            );
        }
        for (index, tab) in self.contributions.detail_tabs.iter().enumerate() {
            kinds(
                &mut problems,
                &format!("contributions.detailTabs[{index}].forKinds"),
                &tab.for_kinds,
            );
        }
        for (index, link) in self.contributions.detail_links.iter().enumerate() {
            kinds(
                &mut problems,
                &format!("contributions.detailLinks[{index}].forKinds"),
                &link.for_kinds,
            );
        }
        if self.contributions.joins.len() > 16 {
            problems.push(
                Code::InvalidValue,
                "contributions.joins",
                "Declare at most 16 joins",
            );
        }
        if self.contributions.table_columns.len() > 32 {
            problems.push(
                Code::InvalidValue,
                "contributions.tableColumns",
                "Declare at most 32 table columns",
            );
        }
        let join_ids = unique(
            &mut problems,
            self.contributions
                .joins
                .iter()
                .enumerate()
                .map(|(index, join)| {
                    (format!("contributions.joins[{index}].id"), join.id.as_str())
                }),
        );
        for (index, join) in self.contributions.joins.iter().enumerate() {
            let at = format!("contributions.joins[{index}]");
            if !identifier(&join.id) {
                problems.push(Code::InvalidValue, format!("{at}.id"), IDENTIFIER);
            }
            if !self.capabilities.iter().any(|binding| {
                binding.name == join.capability && binding.target == "k8s.listCustomResource"
            }) {
                problems.push(
                    Code::UnresolvedCapability,
                    format!("{at}.capability"),
                    "A join must name a declared custom-resource reader",
                );
            }
            let matching = &join.match_by;
            let selectors = usize::from(matching.label.is_some())
                + usize::from(matching.owner_reference)
                + usize::from(matching.annotation.is_some())
                + usize::from(matching.name);
            if selectors != 1 {
                problems.push(
                    Code::InvalidBinding,
                    format!("{at}.match"),
                    "Choose exactly one of label, ownerReference, annotation or name",
                );
            }
            if matching.kind_label.is_some() && matching.label.is_none() {
                problems.push(
                    Code::InvalidBinding,
                    format!("{at}.match.kindLabel"),
                    "kindLabel requires label",
                );
            }
            for (field, key) in [
                ("label", &matching.label),
                ("kindLabel", &matching.kind_label),
                ("annotation", &matching.annotation),
            ] {
                if key.as_ref().is_some_and(|key| {
                    key.is_empty()
                        || key.len() > 253
                        || key
                            .chars()
                            .any(|c| c.is_control() || is_format_character(c))
                }) {
                    problems.push(
                        Code::InvalidValue,
                        format!("{at}.match.{field}"),
                        "Metadata key must be 1–253 visible characters",
                    );
                }
            }
        }
        unique(
            &mut problems,
            self.contributions
                .table_columns
                .iter()
                .enumerate()
                .map(|(index, column)| {
                    (
                        format!("contributions.tableColumns[{index}].id"),
                        column.id.as_str(),
                    )
                }),
        );
        for (index, column) in self.contributions.table_columns.iter().enumerate() {
            let at = format!("contributions.tableColumns[{index}]");
            if !identifier(&column.id) {
                problems.push(Code::InvalidValue, format!("{at}.id"), IDENTIFIER);
            }
            if !label(&column.title) {
                problems.push(Code::InvalidValue, format!("{at}.title"), LABEL);
            }
            kinds(&mut problems, &format!("{at}.forKinds"), &column.for_kinds);
            if let Some(join) = &column.source.join {
                if !join_ids.contains(join.as_str()) {
                    problems.push(
                        Code::InvalidBinding,
                        format!("{at}.source.join"),
                        "Column source must name a declared join",
                    );
                }
            }
            let path = &column.source.json_path;
            if !column_json_path(path) {
                problems.push(
                    Code::InvalidValue,
                    format!("{at}.source.jsonPath"),
                    "jsonPath must be a valid absolute scalar path of at most 256 characters",
                );
            }
        }
        if self.contributions.detail_panels.len() > 16 {
            problems.push(
                Code::InvalidValue,
                "contributions.detailPanels",
                "Declare at most 16 detail panels",
            );
        }
        unique(
            &mut problems,
            self.contributions
                .detail_panels
                .iter()
                .enumerate()
                .map(|(index, panel)| {
                    (
                        format!("contributions.detailPanels[{index}].id"),
                        panel.id.as_str(),
                    )
                }),
        );
        for (index, panel) in self.contributions.detail_panels.iter().enumerate() {
            let at = format!("contributions.detailPanels[{index}]");
            if !identifier(&panel.id) {
                problems.push(Code::InvalidValue, format!("{at}.id"), IDENTIFIER);
            }
            if !label(&panel.title) {
                problems.push(Code::InvalidValue, format!("{at}.title"), LABEL);
            }
            kinds(&mut problems, &format!("{at}.forKinds"), &panel.for_kinds);
            if panel.sections.is_empty() || panel.sections.len() > 8 {
                problems.push(
                    Code::InvalidValue,
                    format!("{at}.sections"),
                    "Declare 1–8 sections",
                );
            }
            for (section_index, section) in panel.sections.iter().enumerate() {
                let section_at = format!("{at}.sections[{section_index}]");
                let check_source = |problems: &mut ValidationErrors,
                                    path: &str,
                                    join: &Option<String>,
                                    field_at: &str| {
                    if !column_json_path(path) {
                        problems.push(
                            Code::InvalidValue,
                            format!("{field_at}.jsonPath"),
                            "jsonPath must be a valid absolute path of at most 256 characters",
                        );
                    }
                    if join
                        .as_ref()
                        .is_some_and(|id| !join_ids.contains(id.as_str()))
                    {
                        problems.push(
                            Code::InvalidBinding,
                            format!("{field_at}.join"),
                            "Panel source must name a declared join",
                        );
                    }
                };
                match section {
                    DetailSection::Fields { fields } => {
                        if fields.is_empty() || fields.len() > 32 {
                            problems.push(
                                Code::InvalidValue,
                                format!("{section_at}.fields"),
                                "Declare 1–32 fields",
                            );
                        }
                        for (field_index, field) in fields.iter().enumerate() {
                            let field_at = format!("{section_at}.fields[{field_index}]");
                            if !label(&field.label) {
                                problems.push(
                                    Code::InvalidValue,
                                    format!("{field_at}.label"),
                                    LABEL,
                                );
                            }
                            check_source(&mut problems, &field.json_path, &field.join, &field_at);
                        }
                    }
                    DetailSection::Conditions { json_path, join } => {
                        check_source(&mut problems, json_path, join, &section_at);
                        if column_json_path(json_path) && !condition_json_path(json_path) {
                            problems.push(
                                Code::InvalidValue,
                                format!("{section_at}.jsonPath"),
                                "Conditions jsonPath must use plain dot-separated object keys",
                            );
                        }
                    }
                }
            }
        }
        self.status_problems(&mut problems, &join_ids);
        cards::card_problems(self, &mut problems);
        settings::setting_problems(self, &mut problems);
        self.command_problems(&mut problems);
        self.link_problems(&mut problems);
        problems
    }

    /// `commands` (#544).
    fn command_problems(&self, problems: &mut ValidationErrors) {
        const LABEL: &str =
            "Must be 1–120 characters with no control characters and no bidirectional or invisible format characters";
        const IDENTIFIER: &str = "Must be 1–64 letters, digits and -";
        let commands = &self.contributions.commands;
        if commands.len() > MAX_COMMANDS {
            problems.push(
                Code::InvalidValue,
                "contributions.commands",
                format!("Declare at most {MAX_COMMANDS} commands"),
            );
        }
        unique(
            problems,
            commands.iter().enumerate().map(|(index, command)| {
                (
                    format!("contributions.commands[{index}].id"),
                    command.id.as_str(),
                )
            }),
        );
        for (index, command) in commands.iter().enumerate() {
            let at = format!("contributions.commands[{index}]");
            if !identifier(&command.id) {
                problems.push(Code::InvalidValue, format!("{at}.id"), IDENTIFIER);
            }
            if !label(&command.title) {
                problems.push(Code::InvalidValue, format!("{at}.title"), LABEL);
            }
            match &command.target {
                CommandTarget::Page(page) => {
                    if !self.contributions.pages.iter().any(|p| &p.id == page) {
                        problems.push(
                            Code::UnresolvedPage,
                            format!("{at}.target.page"),
                            format!("Page \"{page}\" is not declared"),
                        );
                    }
                    if !command.for_kinds.is_empty() {
                        problems.push(
                            Code::InvalidBinding,
                            format!("{at}.forKinds"),
                            "A page command has no resource to be scoped to; forKinds is for action commands",
                        );
                    }
                }
                CommandTarget::Action(name) => {
                    kinds(problems, &format!("{at}.forKinds"), &command.for_kinds);
                    let path = format!("{at}.target.action");
                    let Some(action) = self.actions.iter().find(|a| &a.name == name) else {
                        problems.push(
                            Code::UnresolvedCapability,
                            path,
                            format!("Action \"{name}\" is not declared"),
                        );
                        continue;
                    };
                    // The palette runs an action where the host already asks
                    // for it: in the app's resource inspector, which reads a
                    // custom resource through one of the app's pages.
                    let Some(kind) = self
                        .capabilities
                        .iter()
                        .find(|b| b.name == action.resource)
                        .and_then(Self::reader_kind)
                    else {
                        // An undeclared reader is reported at the action itself.
                        if self.capabilities.iter().any(|b| b.name == action.resource) {
                            problems.push(
                                Code::InvalidBinding,
                                path,
                                "A command runs only an action on a k8s.listCustomResource reader that fixes its group and kind",
                            );
                        }
                        continue;
                    };
                    if !self
                        .contributions
                        .pages
                        .iter()
                        .any(|page| page.capability == action.resource)
                    {
                        problems.push(
                            Code::InvalidBinding,
                            path,
                            format!(
                                "No page lists \"{}\", so there is no resource view to run this action from",
                                action.resource
                            ),
                        );
                    }
                    for (position, candidate) in command.for_kinds.iter().enumerate() {
                        if candidate != &kind && candidate.contains('/') {
                            problems.push(
                                Code::InvalidBinding,
                                format!("{at}.forKinds[{position}]"),
                                format!("Action \"{name}\" acts on {kind}, not {candidate}"),
                            );
                        }
                    }
                }
            }
        }
    }

    /// `resourceLinks` (#545).
    fn link_problems(&self, problems: &mut ValidationErrors) {
        const IDENTIFIER: &str = "Must be 1–64 letters, digits and -";
        let links = &self.contributions.resource_links;
        if links.len() > MAX_RESOURCE_LINKS {
            problems.push(
                Code::InvalidValue,
                "contributions.resourceLinks",
                format!("Declare at most {MAX_RESOURCE_LINKS} resource links"),
            );
            return;
        }
        let readable: BTreeSet<String> = self
            .capabilities
            .iter()
            .filter_map(Self::reader_kind)
            .collect();
        unique(
            problems,
            links.iter().enumerate().map(|(index, link)| {
                (
                    format!("contributions.resourceLinks[{index}].id"),
                    link.id.as_str(),
                )
            }),
        );
        for (index, link) in links.iter().enumerate() {
            let at = format!("contributions.resourceLinks[{index}]");
            if !identifier(&link.id) {
                problems.push(Code::InvalidValue, format!("{at}.id"), IDENTIFIER);
            }
            let before = problems.0.len();
            qualified_kind(problems, format!("{at}.from"), &link.from);
            qualified_kind(problems, format!("{at}.to"), &link.to);
            let kinds_ok = problems.0.len() == before;
            if kinds_ok && !readable.contains(&link.to) {
                problems.push(
                    Code::UnresolvedCapability,
                    format!("{at}.to"),
                    format!(
                        "No declared k8s.listCustomResource reader lists {}",
                        link.to
                    ),
                );
            }
            let matching = &link.match_by;
            let selectors = usize::from(matching.label.is_some())
                + usize::from(matching.owner_reference)
                + usize::from(matching.annotation.is_some())
                + usize::from(matching.name);
            if selectors != 1 {
                problems.push(
                    Code::InvalidBinding,
                    format!("{at}.match"),
                    "Choose exactly one of label, ownerReference, annotation or name",
                );
            }
            if matching.namespace_label.is_some() && matching.label.is_none() {
                problems.push(
                    Code::InvalidBinding,
                    format!("{at}.match.namespaceLabel"),
                    "namespaceLabel requires label",
                );
            }
            if matching.parse.is_some() && matching.annotation.is_none() {
                problems.push(
                    Code::InvalidBinding,
                    format!("{at}.match.parse"),
                    "parse requires annotation",
                );
            }
            // The parser yields an Argo CD Application's name, and nothing
            // else's: toward another kind it would name a stranger.
            if kinds_ok
                && matching.parse == Some(ReferenceFormat::ArgocdTrackingId)
                && link.to != "argoproj.io/Application"
            {
                problems.push(
                    Code::InvalidBinding,
                    format!("{at}.match.parse"),
                    "An argocd-tracking-id names an Argo CD Application; `to` must be argoproj.io/Application",
                );
            }
            if let Some(namespace) = &matching.default_namespace {
                let path = format!("{at}.match.defaultNamespace");
                if matching.annotation.is_none()
                    || matching.parse != Some(ReferenceFormat::ArgocdTrackingId)
                {
                    problems.push(
                        Code::InvalidBinding,
                        path,
                        "defaultNamespace places a bare Argo CD application name; it requires annotation with parse: \"argocd-tracking-id\"",
                    );
                } else if !namespace_name(namespace) {
                    problems.push(
                        Code::InvalidValue,
                        path,
                        "defaultNamespace must be a namespace name: 1–63 lowercase letters, digits and -, starting and ending with a letter or digit",
                    );
                }
            }
            for (field, key) in [
                ("label", &matching.label),
                ("namespaceLabel", &matching.namespace_label),
                ("annotation", &matching.annotation),
            ] {
                if key.as_ref().is_some_and(|key| {
                    key.is_empty()
                        || key.len() > 253
                        || key
                            .chars()
                            .any(|c| c.is_control() || is_format_character(c))
                }) {
                    problems.push(
                        Code::InvalidValue,
                        format!("{at}.match.{field}"),
                        "Metadata key must be 1–253 visible characters",
                    );
                }
            }
            // The host blanks every annotation value of a Secret on every
            // ungated read: an annotation match could only read placeholders.
            if link.from == "/Secret" && matching.annotation.is_some() {
                problems.push(
                    Code::InvalidBinding,
                    format!("{at}.match.annotation"),
                    "A Secret's annotation values are redacted on every read; match it by label, ownerReference or name",
                );
            }
            if matching.name && link.from == link.to {
                problems.push(
                    Code::InvalidBinding,
                    format!("{at}.match.name"),
                    "A kind linked to itself by name is the resource itself",
                );
            }
        }
    }

    /// `statusResolvers` and `badges` (#541).
    fn status_problems(&self, problems: &mut ValidationErrors, join_ids: &BTreeSet<&str>) {
        const IDENTIFIER: &str = "Must be 1–64 letters, digits and -";
        let contributions = &self.contributions;
        if contributions.status_resolvers.len() > MAX_STATUS_CONTRIBUTIONS {
            problems.push(
                Code::InvalidValue,
                "contributions.statusResolvers",
                format!("Declare at most {MAX_STATUS_CONTRIBUTIONS} status resolvers"),
            );
        }
        let readable: BTreeSet<String> = self
            .capabilities
            .iter()
            .filter_map(Self::reader_kind)
            .collect();
        let mut resolved: BTreeSet<&str> = BTreeSet::new();
        for (index, resolver) in contributions.status_resolvers.iter().enumerate() {
            let at = format!("contributions.statusResolvers[{index}]");
            kinds(problems, &format!("{at}.forKinds"), &resolver.for_kinds);
            for (position, kind) in resolver.for_kinds.iter().enumerate() {
                let path = format!("{at}.forKinds[{position}]");
                if !readable.contains(kind) {
                    problems.push(
                        Code::UnresolvedCapability,
                        path,
                        format!("No declared k8s.listCustomResource reader lists {kind}"),
                    );
                } else if !resolved.insert(kind.as_str()) {
                    problems.push(
                        Code::DuplicateIdentifier,
                        path,
                        format!("{kind} already has a status resolver"),
                    );
                }
            }
            rule_list_problems(problems, &at, &resolver.rules);
        }
        if contributions.badges.len() > MAX_STATUS_CONTRIBUTIONS {
            problems.push(
                Code::InvalidValue,
                "contributions.badges",
                format!("Declare at most {MAX_STATUS_CONTRIBUTIONS} badges"),
            );
        }
        unique(
            problems,
            contributions
                .badges
                .iter()
                .enumerate()
                .map(|(index, badge)| {
                    (
                        format!("contributions.badges[{index}].id"),
                        badge.id.as_str(),
                    )
                }),
        );
        for (index, badge) in contributions.badges.iter().enumerate() {
            let at = format!("contributions.badges[{index}]");
            if !identifier(&badge.id) {
                problems.push(Code::InvalidValue, format!("{at}.id"), IDENTIFIER);
            }
            kinds(problems, &format!("{at}.forKinds"), &badge.for_kinds);
            match &badge.join {
                Some(join) if !join_ids.contains(join.as_str()) => problems.push(
                    Code::InvalidBinding,
                    format!("{at}.join"),
                    "A badge's join must name a declared join",
                ),
                Some(_) => {}
                None => {
                    const METADATA: &str = "A badge without a join reads only its row's metadata: start the path with .metadata";
                    for (position, rule) in badge.rules.iter().enumerate() {
                        let rule_at = format!("{at}.rules[{position}]");
                        for (item, condition) in rule.when.iter().enumerate() {
                            if !metadata_path(&condition.json_path) {
                                problems.push(
                                    Code::InvalidBinding,
                                    format!("{rule_at}.when[{item}]"),
                                    METADATA,
                                );
                            }
                        }
                        if rule
                            .reason
                            .as_deref()
                            .is_some_and(|path| !metadata_path(path))
                        {
                            problems.push(
                                Code::InvalidBinding,
                                format!("{rule_at}.reason"),
                                METADATA,
                            );
                        }
                    }
                }
            }
            rule_list_problems(problems, &at, &badge.rules);
        }
    }
}
