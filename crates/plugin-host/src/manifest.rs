use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

/// Extension API versions this host implements, oldest first. A manifest is accepted when
/// its `srelensApiVersion` range matches any of them. How versions are added and retired
/// is specified in docs/extensions/specification.md.
pub const SUPPORTED_API_VERSIONS: &[&str] = &["0.1.0"];

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

fn unsupported_api(range: &str) -> String {
    format!(
        "extension requires API {range}; host supports {}",
        SUPPORTED_API_VERSIONS.join(", ")
    )
}
/// Manifest fields added after API 0.1, each with the API version that introduced it. A
/// manifest may use a field only when its range negotiates to that version or later.
/// Paths are dot-separated from the manifest root; `[]` steps into every element of an
/// array, and the last segment always names a field. Empty while 0.1 is the only version.
pub const API_FIELDS: &[(&str, &str)] = &[];

/// Rejects a field in `raw` that `fields` says arrived after `negotiated`.
pub fn check_api_fields_in(
    raw: &Value,
    negotiated: &semver::Version,
    fields: &[(&str, &str)],
) -> Result<(), String> {
    for (path, introduced) in fields {
        let introduced = semver::Version::parse(introduced)
            .map_err(|e| format!("invalid API version for {path}: {e}"))?;
        if *negotiated < introduced && field_present(raw, path) {
            return Err(format!(
                "`{path}` requires API {introduced}; this manifest's srelensApiVersion is served as API {negotiated}"
            ));
        }
    }
    Ok(())
}

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
    true
}

pub const MAX_MANIFEST_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
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
    #[serde(rename = "rowActions")]
    pub row_actions: Vec<RowAction>,
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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RowAction {
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
fn label(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 120 && !value.chars().any(char::is_control)
}
fn keys(values: impl Iterator<Item = String>) -> Result<BTreeSet<String>, String> {
    let mut seen = BTreeSet::new();
    for key in values {
        if !seen.insert(key.clone()) {
            return Err(format!("duplicate identifier: {key}"));
        }
    }
    Ok(seen)
}
fn kinds(values: &[String]) -> Result<(), String> {
    if values.is_empty() || values.len() > 32 {
        return Err("forKinds must contain 1–32 qualified kinds".into());
    }
    for value in values {
        let Some((group, kind)) = value.split_once('/') else {
            return Err(format!("qualify kind with its API group: {value}"));
        };
        if !identifier(kind) || (!group.is_empty() && !group.split('.').all(identifier)) {
            return Err(format!("invalid qualified kind: {value}"));
        }
    }
    keys(values.iter().cloned())?;
    Ok(())
}

impl Manifest {
    pub fn parse(source: &str) -> Result<Self, String> {
        if source.len() > MAX_MANIFEST_BYTES {
            return Err("extension manifest exceeds 256 KiB".into());
        }
        // Check the API range before the strict schema, so a manifest written for a newer API
        // is told which version it needs rather than which field this host does not know,
        // and so it uses only the fields of the version its range negotiates to.
        let raw = serde_json::from_str::<Value>(source).ok();
        if let Some(raw) = &raw {
            if let Some(range) = raw.get("srelensApiVersion").and_then(Value::as_str) {
                if let Ok(req) = semver::VersionReq::parse(range) {
                    let negotiated =
                        negotiate_api_version(&req).ok_or_else(|| unsupported_api(range))?;
                    check_api_fields_in(raw, &negotiated, API_FIELDS)?;
                }
            }
        }
        let manifest: Self =
            serde_json::from_str(source).map_err(|e| format!("invalid extension manifest: {e}"))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn schema() -> Value {
        serde_json::to_value(schemars::schema_for!(Self)).expect("manifest schema serializes")
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.id.len() > 128 || !self.id.contains('.') || !self.id.split('.').all(identifier) {
            return Err("extension id must be a reverse-domain identifier".into());
        }
        if !label(&self.name) {
            return Err("invalid extension name".into());
        }
        semver::Version::parse(&self.version)
            .map_err(|e| format!("invalid extension version: {e}"))?;
        let range = semver::VersionReq::parse(&self.api_version)
            .map_err(|e| format!("invalid srelensApiVersion: {e}"))?;
        if negotiate_api_version(&range).is_none() {
            return Err(unsupported_api(&self.api_version));
        }
        if self.capabilities.is_empty() || self.capabilities.len() > 32 {
            return Err("declare 1–32 capabilities".into());
        }
        let names = keys(self.capabilities.iter().map(|c| c.name.clone()))?;
        let permissions = keys(self.permissions.iter().cloned())?;
        let mut targets = BTreeSet::new();
        for binding in &self.capabilities {
            if !identifier(&binding.name) || !label(&binding.title) {
                return Err("invalid capability name or title".into());
            }
            if binding.target.starts_with("plugin/") {
                return Err("extension-to-extension forwarding is unsupported".into());
            }
            targets.insert(binding.target.clone());
            let inputs = keys(binding.inputs.iter().cloned())?;
            if inputs.iter().any(|k| binding.arguments.contains_key(k)) {
                return Err("a bound argument cannot also be an input".into());
            }
        }
        if targets != permissions {
            return Err("permissions must exactly name the bound host capabilities".into());
        }
        let mut ids = BTreeSet::new();
        let entries = self
            .contributions
            .pages
            .iter()
            .map(|p| (&p.id, &p.title, &p.capability))
            .chain(
                self.contributions
                    .detail_tabs
                    .iter()
                    .map(|p| (&p.id, &p.title, &p.capability)),
            )
            .chain(
                self.contributions
                    .row_actions
                    .iter()
                    .map(|p| (&p.id, &p.title, &p.capability)),
            );
        for (id, title, capability) in entries {
            if ids.len() >= 64
                || !identifier(id)
                || !label(title)
                || !names.contains(capability)
                || !ids.insert(id)
            {
                return Err(format!(
                    "invalid, duplicate or unresolved contribution: {id}"
                ));
            }
        }
        for page in &self.contributions.pages {
            if page.group.as_ref().is_some_and(|group| !label(group)) {
                return Err("invalid page group".into());
            }
            if let Some(status) = &page.status_columns {
                if [Some(status.ready), status.suspended, status.progressing]
                    .into_iter()
                    .flatten()
                    .any(|i| i >= 64)
                {
                    return Err("status column index must be below 64".into());
                }
            }
            if let Some(dashboard) = &page.dashboard {
                if dashboard.pages.is_empty() || dashboard.pages.len() > 12 {
                    return Err("dashboard must reference 1–12 resource pages".into());
                }
                keys(dashboard.pages.iter().cloned())?;
                for id in &dashboard.pages {
                    if !self
                        .contributions
                        .pages
                        .iter()
                        .any(|p| &p.id == id && p.dashboard.is_none() && p.status_columns.is_some())
                    {
                        return Err(
                            "dashboard must reference a resource page with status columns".into(),
                        );
                    }
                }
                if let Some(events) = &dashboard.events {
                    if !self
                        .capabilities
                        .iter()
                        .any(|c| c.name == events.capability && c.target == "k8s.listEvents")
                        || events.api_groups.is_empty()
                        || events.api_groups.len() > 32
                        || events
                            .api_groups
                            .iter()
                            .any(|g| !g.contains('.') || !g.split('.').all(identifier))
                    {
                        return Err(
                            "dashboard events require an event reader and explicit API groups"
                                .into(),
                        );
                    }
                    keys(events.api_groups.iter().cloned())?;
                }
            }
        }
        for tab in &self.contributions.detail_tabs {
            kinds(&tab.for_kinds)?;
        }
        for action in &self.contributions.row_actions {
            kinds(&action.for_kinds)?;
        }
        Ok(())
    }
}
