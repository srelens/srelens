use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

pub const API_VERSION: &str = "0.1.0";
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
        if !range.matches(&semver::Version::parse(API_VERSION).unwrap()) {
            return Err(format!(
                "extension requires API {}; host supports {API_VERSION}",
                self.api_version
            ));
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
        for tab in &self.contributions.detail_tabs {
            kinds(&tab.for_kinds)?;
        }
        for action in &self.contributions.row_actions {
            kinds(&action.for_kinds)?;
        }
        Ok(())
    }
}
