use crate::{ValidationCode as Code, ValidationError, ValidationErrors};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

/// Extension API versions this host implements, oldest first. A manifest is accepted when
/// its `srelensApiVersion` range matches any of them. How versions are added and retired
/// is specified in docs/extensions/specification.md.
pub const SUPPORTED_API_VERSIONS: &[&str] = &["0.1.0"];

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
}

/// Manifest fields added or removed after API 0.1. A manifest may use a field only when
/// its range negotiates to a version inside the field's availability. A rename is a
/// removal plus an addition. Empty while 0.1 is the only version.
pub const API_FIELDS: &[ApiField] = &[];

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
        if !field_present(raw, field.path) {
            continue;
        }
        let introduced = parse(field, field.introduced)?;
        let removed = field
            .removed
            .map(|removed| parse(field, removed))
            .transpose()?;
        for version in versions {
            if *version < introduced {
                return Err(format!(
                    "`{}` requires API {introduced}, but this manifest's srelensApiVersion admits API {version}",
                    field.path
                ));
            }
            if let Some(removed) = &removed {
                if version >= removed {
                    return Err(format!(
                        "`{}` was removed in API {removed}, but this manifest's srelensApiVersion admits API {version}",
                        field.path
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Whether `path` holds a value in `value`. Null, an empty array and an empty object count
/// as absent: they contribute nothing, and a manifest loaded from storage serializes its
/// unused collection fields as empty.
fn field_present(value: &Value, path: &str) -> bool {
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
        if next.is_empty() {
            return false;
        }
        nodes = next;
    }
    nodes.iter().any(|node| match node {
        Value::Null => false,
        Value::Array(items) => !items.is_empty(),
        Value::Object(fields) => !fields.is_empty(),
        _ => true,
    })
}

pub const MAX_MANIFEST_BYTES: usize = 256 * 1024;

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
    pub contributions: Contributions,
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

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
}
/// Unicode's format characters, general category Cf, as of Unicode 17.0: the soft hyphen,
/// bidirectional marks, embeddings, overrides and isolates, zero-width spaces and joiners,
/// invisible operators, the byte order mark, tags, and a few script-specific marks. Each
/// changes how text displays without being seen itself. `char::is_control` covers only
/// category Cc. Listed here rather than taken from a Unicode crate; the ranges are those
/// of Unicode's `DerivedGeneralCategory.txt`.
const FORMAT_CHARACTERS: &[(char, char)] = &[
    ('\u{00AD}', '\u{00AD}'),
    ('\u{0600}', '\u{0605}'),
    ('\u{061C}', '\u{061C}'),
    ('\u{06DD}', '\u{06DD}'),
    ('\u{070F}', '\u{070F}'),
    ('\u{0890}', '\u{0891}'),
    ('\u{08E2}', '\u{08E2}'),
    ('\u{180E}', '\u{180E}'),
    ('\u{200B}', '\u{200F}'),
    ('\u{202A}', '\u{202E}'),
    ('\u{2060}', '\u{2064}'),
    ('\u{2066}', '\u{206F}'),
    ('\u{FEFF}', '\u{FEFF}'),
    ('\u{FFF9}', '\u{FFFB}'),
    ('\u{110BD}', '\u{110BD}'),
    ('\u{110CD}', '\u{110CD}'),
    ('\u{13430}', '\u{1343F}'),
    ('\u{1BCA0}', '\u{1BCA3}'),
    ('\u{1D173}', '\u{1D17A}'),
    ('\u{E0001}', '\u{E0001}'),
    ('\u{E0020}', '\u{E007F}'),
];

/// Whether `c` is a Unicode format character (category Cf), such as a right-to-left
/// override or a zero-width space. Text shown as an app's identity refuses them, because
/// they can make it display differently from what it holds.
pub fn is_format_character(c: char) -> bool {
    FORMAT_CHARACTERS
        .iter()
        .any(|&(first, last)| (first..=last).contains(&c))
}

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
                format!("\"{}\" is listed more than once", shown(value)),
            );
        }
    }
    seen
}
/// A rejected value as a problem message may quote it: control and format characters are
/// written as `\u{…}` escapes, so a value carrying a bidirectional override cannot reorder
/// or reshape the host's own problem row when the message is shown.
fn shown(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_control() || is_format_character(c) {
                format!("\\u{{{:x}}}", c as u32)
            } else {
                c.to_string()
            }
        })
        .collect()
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
        let at = format!("{path}[{index}]");
        match value.split_once('/') {
            None => problems.push(
                Code::InvalidKind,
                at,
                format!("Qualify \"{}\" with its API group, for example apps/Deployment, or /Pod for the core group", shown(value)),
            ),
            Some((group, kind))
                if !identifier(kind) || (!group.is_empty() && !group.split('.').all(identifier)) =>
            {
                problems.push(Code::InvalidKind, at, format!("\"{}\" is not a qualified Kubernetes kind", shown(value)))
            }
            Some(_) => {}
        }
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

    /// Checks the manifest's rules, reporting every violation with the path at fault.
    pub fn validate(&self) -> Result<(), ValidationErrors> {
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
        if self.capabilities.is_empty() || self.capabilities.len() > 32 {
            problems.push(
                Code::InvalidValue,
                "capabilities",
                "Declare 1–32 capabilities",
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
                        if target.dashboard.is_some() || target.status_columns.is_none() =>
                    {
                        problems.push(
                            Code::InvalidValue,
                            path,
                            format!("Page \"{id}\" must be a resource page with statusColumns"),
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
        problems.into_result()
    }
}
