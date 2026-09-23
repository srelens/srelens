use super::columns::{join_objects, match_joined, JoinCache};
use super::*;
use srelens_plugin_host::{DetailPanel, DetailSection};
use std::collections::{HashMap, HashSet};

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ResolvePanels {
    id: String,
    revision: u64,
    context: String,
    namespace: String,
    kind: String,
    resource: Value,
}

#[derive(Serialize, JsonSchema)]
struct ResolvedPanels {
    panels: Vec<Value>,
}

fn check_input(input: &ResolvePanels) -> Result<(), CapabilityError> {
    if input.id.is_empty()
        || input.id.len() > 128
        || input.context.trim().is_empty()
        || input.context.len() > 4_096
        || input.kind.trim().is_empty()
        || input.kind.len() > 317
        || input.namespace.len() > 63
        || (!input.namespace.is_empty()
            && (input.namespace.starts_with('-')
                || input.namespace.ends_with('-')
                || !input
                    .namespace
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')))
    {
        return Err(CapabilityError::InvalidInput(
            "Panel scope exceeds the supported limits".into(),
        ));
    }
    let bytes = serde_json::to_vec(&input.resource)
        .map_err(|error| CapabilityError::InvalidInput(error.to_string()))?;
    if bytes.len() > 1_048_576 {
        return Err(CapabilityError::InvalidInput(
            "Panel resource exceeds 1 MiB".into(),
        ));
    }
    let metadata = &input.resource["metadata"];
    let name = metadata["name"].as_str().unwrap_or("");
    if name.is_empty()
        || name.len() > 253
        || metadata["uid"].as_str().is_some_and(|uid| uid.len() > 128)
        || metadata["namespace"].as_str().unwrap_or("") != input.namespace
    {
        return Err(CapabilityError::InvalidInput(
            "Panel resource identity does not match its scope".into(),
        ));
    }
    let kind = input.resource["kind"].as_str().unwrap_or("");
    let api_version = input.resource["apiVersion"].as_str().unwrap_or("");
    let group = api_version
        .split_once('/')
        .map(|(group, _)| group)
        .unwrap_or("");
    if format!("{group}/{kind}") != input.kind {
        return Err(CapabilityError::InvalidInput(
            "Panel resource kind does not match its scope".into(),
        ));
    }
    Ok(())
}

fn used_joins(panels: &[&DetailPanel]) -> HashSet<String> {
    let mut ids = HashSet::new();
    for panel in panels {
        for section in &panel.sections {
            match section {
                DetailSection::Fields { fields } => {
                    ids.extend(fields.iter().filter_map(|field| field.join.clone()));
                }
                DetailSection::Conditions { join, .. } => {
                    if let Some(id) = join {
                        ids.insert(id.clone());
                    }
                }
            }
        }
    }
    ids
}

pub(super) fn register(
    reg: &mut Registry,
    path: PathBuf,
    core: Arc<Registry>,
    client_cache: Arc<srelens_kube::client_cache::ClientCache>,
    cache: JoinCache,
) {
    reg.register(Capability::typed::<ResolvePanels, ResolvedPanels, _, _>(
        "extensions.resolvePanels",
        "Resolve declarative app panels for a resource Inspector",
        Annotations::READ_ONLY,
        move |input| {
            let path = path.clone();
            let core = core.clone();
            let client_cache = client_cache.clone();
            let cache = cache.clone();
            async move {
                check_input(&input)?;
                let (state, index, context) = resolver_app(
                    path,
                    &core,
                    &client_cache,
                    &input.id,
                    input.revision,
                    input.context,
                )
                .await?;
                let plugin = &state.plugins[index];
                let panels: Vec<_> = plugin
                    .manifest
                    .contributions
                    .detail_panels
                    .iter()
                    .filter(|panel| panel.for_kinds.contains(&input.kind))
                    .collect();
                let mut sources = PanelSources::new();
                for join_id in used_joins(&panels) {
                    let join = plugin
                        .manifest
                        .contributions
                        .joins
                        .iter()
                        .find(|join| join.id == join_id)
                        .ok_or_else(|| {
                            CapabilityError::Handler("Panel join is no longer declared".into())
                        })?;
                    let objects = match join_objects(
                        &cache,
                        &client_cache,
                        &core,
                        plugin,
                        join,
                        &context,
                        &input.namespace,
                    )
                    .await
                    {
                        Ok(objects) => objects,
                        Err(error) => {
                            sources.insert(join_id, Err(error.to_string()));
                            continue;
                        }
                    };
                    let matched = match_joined(
                        &join.match_by,
                        &objects,
                        input.resource["metadata"]["uid"].as_str(),
                        input.resource["metadata"]["name"].as_str().unwrap_or(""),
                        &input.namespace,
                        &input.kind,
                    );
                    sources.insert(
                        join_id,
                        matched
                            .map(|object| object.cloned())
                            .map_err(|()| "matched multiple joined resources".to_owned()),
                    );
                }
                Ok(ResolvedPanels {
                    panels: panels
                        .into_iter()
                        .map(|panel| resolve_panel(panel, &input.resource, &sources))
                        .collect(),
                })
            }
        },
    ));
}

type PanelSources = HashMap<String, Result<Option<Value>, String>>;

fn resolve_panel(panel: &DetailPanel, resource: &Value, sources: &PanelSources) -> Value {
    let sections: Vec<Value> = panel.sections.iter().map(|section| match section {
        DetailSection::Fields { fields } => json!({
            "type":"fields",
            "fields":fields.iter().map(|field| {
                let target = match field.join.as_ref().map(|id| sources.get(id)) {
                    None => Ok(Some(resource)),
                    Some(Some(Ok(object))) => Ok(object.as_ref()),
                    Some(Some(Err(reason))) => Err(reason.as_str()),
                    Some(None) => Err("Declared join is unavailable"),
                };
                let target = match target {
                    Ok(target) => target,
                    Err(reason) => return json!({"label":field.label,"value":null,"error":reason}),
                };
                let value = target.map(|target| srelens_kube::crds::resolve_json_path(target, &field.json_path)).unwrap_or_default();
                if value.len() > 1_024 {
                    json!({"label":field.label,"value":null,"error":"Value exceeds 1,024 bytes"})
                } else {
                    let mut resolved = json!({"label":field.label,"value":if value.is_empty() { None } else { Some(value) }});
                    if let Some(format) = &field.format {
                        resolved["format"] = json!(format);
                    }
                    resolved
                }
            }).collect::<Vec<_>>()
        }),
        DetailSection::Conditions { json_path, join } => {
            let target = match join.as_ref().map(|id| sources.get(id)) {
                None => Ok(Some(resource)),
                Some(Some(Ok(object))) => Ok(object.as_ref()),
                Some(Some(Err(reason))) => Err(reason.as_str()),
                Some(None) => Err("Declared join is unavailable"),
            };
            let target = match target {
                Ok(target) => target,
                Err(reason) => return json!({"type":"conditions","items":[],"error":reason}),
            };
            let value = target.and_then(|target| simple_path(target, json_path));
            match value {
                None | Some(Value::Null) => json!({"type":"conditions","items":[]}),
                Some(value) if !value.is_array() =>
                    json!({"type":"conditions","items":[],"error":"Conditions value is not an array"}),
                Some(Value::Array(items)) if items.len() > 1_000 =>
                    json!({"type":"conditions","items":[],"error":"Conditions exceed the 1,000-item limit"}),
                Some(Value::Array(items)) if items.iter().any(|item| !item.is_object()) =>
                    json!({"type":"conditions","items":[],"error":"Conditions contain an entry that is not an object"}),
                Some(Value::Array(items)) => json!({"type":"conditions","items":items.iter().map(|item| {
                    let mut condition = serde_json::Map::new();
                    for key in ["type","status","reason","message","lastTransitionTime","observedGeneration"] {
                        if let Some(value) = item.get(key) { condition.insert(key.into(), value.clone()); }
                    }
                    Value::Object(condition)
                }).collect::<Vec<_>>()}),
                Some(_) => unreachable!(),
            }
        }
    }).collect();
    json!({"id":panel.id,"title":panel.title,"sections":sections})
}

fn simple_path<'a>(resource: &'a Value, path: &str) -> Option<&'a Value> {
    path.strip_prefix('.')?
        .split('.')
        .try_fold(resource, |value, key| value.get(key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_namespace_and_oversized_uid() {
        let mut input = ResolvePanels {
            id: "org.example.cert".into(),
            revision: 1,
            context: "cluster/a".into(),
            namespace: "Team".into(),
            kind: "cert-manager.io/Certificate".into(),
            resource: json!({"apiVersion":"cert-manager.io/v1","kind":"Certificate",
                "metadata":{"name":"web","namespace":"Team"}}),
        };
        assert!(check_input(&input).is_err());
        input.namespace = "team".into();
        input.resource["metadata"]["namespace"] = json!("team");
        input.resource["metadata"]["uid"] = json!("u".repeat(129));
        assert!(check_input(&input).is_err());
    }

    #[test]
    fn declared_panel_resolves_fields_and_conditions_without_core_changes() {
        let panel: DetailPanel = serde_json::from_value(json!({
            "id":"certificate", "title":"Certificate", "forKinds":["cert-manager.io/Certificate"],
            "sections":[
                {"type":"fields", "fields":[
                    {"label":"Not after", "jsonPath":".status.notAfter", "format":"date"},
                    {"label":"Issuer", "jsonPath":".spec.issuerRef.name"},
                    {"label":"Absent", "jsonPath":".status.missing"}
                ]},
                {"type":"conditions", "jsonPath":".status.conditions"}
            ]
        }))
        .unwrap();
        let resource = json!({"spec":{"issuerRef":{"name":"letsencrypt"}},"status":{
            "notAfter":"2026-12-01T00:00:00Z", "conditions":[
                {"type":"Ready","status":"True","reason":"Issued","message":"Certificate is ready"}
            ]
        }});
        let resolved = resolve_panel(&panel, &resource, &HashMap::new());
        assert_eq!(
            resolved["sections"][0]["fields"][0]["value"],
            "2026-12-01T00:00:00Z"
        );
        assert_eq!(resolved["sections"][0]["fields"][1]["value"], "letsencrypt");
        assert!(resolved["sections"][0]["fields"][1].get("format").is_none());
        assert!(resolved["sections"][0]["fields"][2]["value"].is_null());
        assert_eq!(resolved["sections"][1]["items"][0]["type"], "Ready");
    }

    #[test]
    fn a_joined_field_uses_the_granted_related_resource_and_keeps_other_fields() {
        let panel: DetailPanel = serde_json::from_value(json!({
            "id":"security", "title":"Security", "forKinds":["apps/Deployment"],
            "sections":[{"type":"fields","fields":[
                {"label":"Workload", "jsonPath":".metadata.name"},
                {"label":"Findings", "join":"reports", "jsonPath":".report.count", "format":"number"}
            ]}]
        })).unwrap();
        let source = HashMap::from([("reports".into(), Ok(Some(json!({"report":{"count":7}}))))]);
        let resolved = resolve_panel(&panel, &json!({"metadata":{"name":"api"}}), &source);
        assert_eq!(resolved["sections"][0]["fields"][0]["value"], "api");
        assert_eq!(resolved["sections"][0]["fields"][1]["value"], "7");
    }

    #[test]
    fn malformed_conditions_report_a_read_error_instead_of_no_conditions() {
        let panel: DetailPanel = serde_json::from_value(json!({
            "id":"health", "title":"Health", "forKinds":["/Pod"],
            "sections":[{"type":"conditions","jsonPath":".status.conditions"}]
        }))
        .unwrap();
        let resolved = resolve_panel(
            &panel,
            &json!({"status":{"conditions":"bad data"}}),
            &HashMap::new(),
        );
        assert_eq!(
            resolved["sections"][0]["error"],
            "Conditions value is not an array"
        );
        let resolved = resolve_panel(
            &panel,
            &json!({"status":{"conditions":["bad entry"]}}),
            &HashMap::new(),
        );
        assert_eq!(
            resolved["sections"][0]["error"],
            "Conditions contain an entry that is not an object"
        );
    }

    #[tokio::test]
    async fn panel_capability_uses_the_installed_manifest_and_rechecks_revision() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("apps.json");
        let core = super::super::tests::fake_core();
        let mut source: Value = serde_json::from_str(&super::super::tests::manifest()).unwrap();
        source["contributions"]["detailPanels"] = json!([{
            "id":"health", "title":"Health", "forKinds":["argoproj.io/Application"],
            "sections":[{"type":"fields","fields":[{"label":"State","jsonPath":".status.health.status"}]}]
        }]);
        let revision = mutate(
            &path,
            core.clone(),
            Configure::Install {
                signature: None,
                manifest: source.to_string(),
                grants: vec!["k8s.listCustomResource".into()],
                reviewed_revision: None,
            },
        )
        .unwrap()
        .plugins[0]
            .revision;
        let cache = srelens_kube::client_cache::ClientCache::new_many(vec![]);
        let mut reg = Registry::new();
        super::super::register(&mut reg, path, core, cache);
        let input = json!({"id":"org.example.argocd","revision":revision,"context":"cluster/a",
            "namespace":"team","kind":"argoproj.io/Application",
            "resource":{"apiVersion":"argoproj.io/v1alpha1","kind":"Application",
                "metadata":{"name":"app","namespace":"team"},"status":{"health":{"status":"Healthy"}}}});
        let output = reg
            .invoke("extensions.resolvePanels", input.clone())
            .await
            .unwrap();
        assert_eq!(
            output["panels"][0]["sections"][0]["fields"][0]["value"],
            "Healthy"
        );
        let mut stale = input;
        stale["revision"] = json!(revision + 1);
        assert!(reg.invoke("extensions.resolvePanels", stale).await.is_err());
    }

    #[tokio::test]
    async fn failed_join_preserves_fields_from_the_selected_resource() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("apps.json");
        let core = super::super::tests::fake_core();
        let mut source: Value = serde_json::from_str(&super::super::tests::manifest()).unwrap();
        source["contributions"]["joins"] = json!([{
            "id":"related", "capability":"applications", "match":{"name":true}
        }]);
        source["contributions"]["detailPanels"] = json!([{
            "id":"application", "title":"Application", "forKinds":["argoproj.io/Application"],
            "sections":[{"type":"fields","fields":[
                {"label":"Name","jsonPath":".metadata.name"},
                {"label":"Related","join":"related","jsonPath":".metadata.name"}
            ]}]
        }]);
        let revision = mutate(
            &path,
            core.clone(),
            Configure::Install {
                signature: None,
                manifest: source.to_string(),
                grants: vec!["k8s.listCustomResource".into()],
                reviewed_revision: None,
            },
        )
        .unwrap()
        .plugins[0]
            .revision;
        let mut reg = Registry::new();
        super::super::register(
            &mut reg,
            path,
            core,
            srelens_kube::client_cache::ClientCache::new_many(vec![]),
        );
        let result = reg
            .invoke(
                "extensions.resolvePanels",
                json!({
                    "id":"org.example.argocd", "revision":revision, "context":"cluster/a",
                    "namespace":"team", "kind":"argoproj.io/Application",
                    "resource":{"apiVersion":"argoproj.io/v1alpha1","kind":"Application",
                        "metadata":{"name":"app","namespace":"team"}}
                }),
            )
            .await
            .unwrap();
        assert_eq!(
            result["panels"][0]["sections"][0]["fields"][0]["value"],
            "app"
        );
        assert!(result["panels"][0]["sections"][0]["fields"][1]["error"].is_string());
    }
}
