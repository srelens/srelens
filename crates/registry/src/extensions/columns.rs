use super::*;
use serde_json::Map;
use srelens_plugin_host::{Join, JoinMatch, TableColumn};
use std::{
    collections::HashMap,
    future::Future,
    sync::Mutex,
    time::{Duration, Instant},
};

const CACHE_TTL: Duration = Duration::from_secs(5);
const CACHE_LIMIT: usize = 32;
type Snapshot = (Instant, Arc<Vec<Value>>);
#[derive(Clone, Hash, PartialEq, Eq)]
struct CacheKey {
    app: String,
    revision: u64,
    context: String,
    namespace: String,
    reader: String,
}
type JoinCache = Arc<Mutex<HashMap<CacheKey, Arc<tokio::sync::Mutex<Option<Snapshot>>>>>>;

#[derive(Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct ColumnRow {
    #[serde(default)]
    uid: Option<String>,
    name: String,
    #[serde(default)]
    namespace: String,
    #[serde(default)]
    row: Value,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct ResolveColumns {
    id: String,
    revision: u64,
    context: String,
    #[serde(default)]
    namespace: String,
    kind: String,
    uids: Vec<ColumnRow>,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ResolvedCell {
    uid: Option<String>,
    name: String,
    namespace: String,
    values: Map<String, Value>,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ResolvedColumns {
    columns: Vec<TableColumn>,
    cells: Vec<ResolvedCell>,
}

fn check_input(input: &ResolveColumns) -> Result<(), CapabilityError> {
    if input.context.trim().is_empty() || input.kind.trim().is_empty() {
        return Err(CapabilityError::InvalidInput(
            "Explicit context and qualified kind are required".into(),
        ));
    }
    if input.uids.len() > 1_000 {
        return Err(CapabilityError::InvalidInput(
            "Resolve at most 1,000 table rows per call".into(),
        ));
    }
    if !input.namespace.is_empty()
        && (input.namespace.len() > 63
            || !input
                .namespace
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
            || input.namespace.starts_with('-')
            || input.namespace.ends_with('-'))
    {
        return Err(CapabilityError::InvalidInput(
            "Namespace must be a Kubernetes namespace name".into(),
        ));
    }
    for row in &input.uids {
        if row.name.is_empty()
            || row.name.len() > 253
            || row.uid.as_ref().is_some_and(|uid| uid.len() > 128)
            || (!input.namespace.is_empty() && row.namespace != input.namespace)
            || serde_json::to_vec(&row.row).is_ok_and(|bytes| bytes.len() > 4_096)
        {
            return Err(CapabilityError::InvalidInput(
                "Row identity, namespace or summary exceeds the column resolver limit".into(),
            ));
        }
    }
    Ok(())
}

fn cache_slot(cache: &JoinCache, key: CacheKey) -> Arc<tokio::sync::Mutex<Option<Snapshot>>> {
    let mut map = cache.lock().expect("join cache lock poisoned");
    if !map.contains_key(&key) && map.len() >= CACHE_LIMIT {
        // An active caller holds another Arc; evicting its slot would let a second
        // caller start the same list before the first finished.
        if let Some(oldest) = map
            .iter()
            .find(|(_, slot)| Arc::strong_count(slot) == 1)
            .map(|(key, _)| key.clone())
        {
            map.remove(&oldest);
        }
    }
    map.entry(key)
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(None)))
        .clone()
}

fn key_for_join(app: &str, revision: u64, context: &str, namespace: &str, join: &Join) -> CacheKey {
    CacheKey {
        app: app.to_owned(),
        revision,
        context: context.to_owned(),
        namespace: namespace.to_owned(),
        // Two match rules over one granted reader need one Kubernetes list.
        reader: join.capability.clone(),
    }
}

async fn cached_objects<F, Fut>(
    cache: &JoinCache,
    key: CacheKey,
    load: F,
) -> Result<Arc<Vec<Value>>, CapabilityError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<Vec<Value>, CapabilityError>>,
{
    let slot = cache_slot(cache, key);
    let mut snapshot = slot.lock().await;
    if let Some((time, objects)) = &*snapshot {
        if time.elapsed() < CACHE_TTL {
            return Ok(Arc::clone(objects));
        }
    }
    // Release an expired large list even if its replacement fails.
    *snapshot = None;
    let objects = Arc::new(load().await?);
    let loaded_at = Instant::now();
    *snapshot = Some((loaded_at, Arc::clone(&objects)));
    drop(snapshot);
    // Idle cache entries must release their raw CRs after the TTL as well.
    let weak = Arc::downgrade(&slot);
    tokio::spawn(async move {
        tokio::time::sleep(CACHE_TTL).await;
        if let Some(slot) = weak.upgrade() {
            let mut snapshot = slot.lock().await;
            if snapshot
                .as_ref()
                .is_some_and(|(time, _)| *time == loaded_at)
            {
                *snapshot = None;
            }
        }
    });
    Ok(objects)
}

async fn join_objects(
    cache: &JoinCache,
    client_cache: &srelens_kube::client_cache::ClientCache,
    core: &Registry,
    plugin: &Installed,
    join: &Join,
    context: &str,
    namespace: &str,
) -> Result<Arc<Vec<Value>>, CapabilityError> {
    let binding = plugin
        .manifest
        .capabilities
        .iter()
        .find(|binding| binding.name == join.capability)
        .ok_or_else(|| CapabilityError::Handler("Declared join reader is unavailable".into()))?;
    crd::require(core, context, binding).await?;
    let key = key_for_join(
        &plugin.manifest.id,
        plugin.revision,
        context,
        namespace,
        join,
    );
    let argument = |key: &str| {
        binding
            .arguments
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
    };
    let namespaced = binding
        .arguments
        .get("namespaced")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    cached_objects(cache, key, || async {
        let (objects, truncated) = srelens_kube::crds::list_custom_resource_join_objects(
            client_cache, context, namespace, argument("group"), argument("version"),
            argument("kind"), argument("plural"), namespaced,
        ).await?;
        if truncated {
            return Err(CapabilityError::Handler(
                "Joined resource list reached its 2,000-object limit; column values would be incomplete".into(),
            ));
        }
        Ok(objects)
    }).await
}

fn resolved_cells(
    columns: &[TableColumn],
    joins: &HashMap<String, (JoinMatch, Arc<Vec<Value>>)>,
    rows: &[ColumnRow],
    kind: &str,
) -> Result<Vec<ResolvedCell>, CapabilityError> {
    let indexed: HashMap<_, _> = joins
        .iter()
        .map(|(id, (rule, objects))| (id, index_join(rule, objects)))
        .collect();
    rows.iter()
        .map(|row| {
            let mut values = Map::new();
            for column in columns {
                let source = &column.source;
                let value = if let Some(join_id) = &source.join {
                    let object = match (joins.get(join_id), indexed.get(join_id)) {
                        (Some((rule, objects)), Some(index)) => {
                            joined(rule, objects, index, row, kind).map_err(|()| {
                                CapabilityError::Handler(format!(
                                    "Column {} for {}/{} matched multiple joined resources",
                                    column.id, row.namespace, row.name,
                                ))
                            })?
                        }
                        _ => None,
                    };
                    object.map(|object| {
                        srelens_kube::crds::resolve_json_path(object, &source.json_path)
                    })
                } else {
                    Some(srelens_kube::crds::resolve_json_path(
                        &row.row,
                        &source.json_path,
                    ))
                };
                if value.as_ref().is_some_and(|value| value.len() > 1_024) {
                    return Err(CapabilityError::Handler(format!(
                        "Column {} value exceeds 1,024 bytes",
                        column.id
                    )));
                }
                values.insert(
                    column.id.clone(),
                    value
                        .filter(|value| !value.is_empty())
                        .map(Value::String)
                        .unwrap_or(Value::Null),
                );
            }
            Ok(ResolvedCell {
                uid: row.uid.clone(),
                name: row.name.clone(),
                namespace: row.namespace.clone(),
                values,
            })
        })
        .collect()
}

pub(super) fn register(
    reg: &mut Registry,
    path: PathBuf,
    core: Arc<Registry>,
    client_cache: Arc<srelens_kube::client_cache::ClientCache>,
) {
    let cache: JoinCache = Arc::new(Mutex::new(HashMap::new()));
    reg.register(Capability::typed::<ResolveColumns, ResolvedColumns, _, _>(
        "extensions.resolveColumns",
        "Resolve native extension table columns in one batch",
        Annotations::READ_ONLY,
        move |input| {
            let path = path.clone();
            let core = core.clone();
            let client_cache = client_cache.clone();
            let cache = cache.clone();
            async move {
                check_input(&input)?;
                let resolved = request_context(&client_cache, &input.context).await;
                let state = tokio::task::spawn_blocking(move || read(&path))
                    .await
                    .map_err(|error| CapabilityError::Handler(error.to_string()))?
                    .map_err(CapabilityError::Handler)?;
                let plugin = state
                    .plugins
                    .iter()
                    .find(|plugin| plugin.manifest.id == input.id)
                    .ok_or_else(|| {
                        CapabilityError::Handler("Extension was removed; refresh the view".into())
                    })?;
                if let Some(reason) = &plugin.policy_blocked {
                    return Err(CapabilityError::Handler(reason.clone()));
                }
                if !plugin.enabled || plugin.revision != input.revision {
                    return Err(CapabilityError::Handler(
                        "Extension was disabled or updated; refresh the view".into(),
                    ));
                }
                plugin.check_scope(&resolved)?;
                validate_app(&plugin.manifest, &plugin.grants, core.clone())
                    .map_err(|errors| CapabilityError::Handler(errors.to_string()))?;
                let context = resolved
                    .ok()
                    .and_then(|context| context.pinned_id())
                    .unwrap_or(input.context);
                let columns: Vec<_> = plugin
                    .manifest
                    .contributions
                    .table_columns
                    .iter()
                    .filter(|column| column.for_kinds.contains(&input.kind))
                    .cloned()
                    .collect();
                let mut joins = HashMap::new();
                for join_id in columns
                    .iter()
                    .filter_map(|column| column.source.join.as_ref())
                {
                    if joins.contains_key(join_id) {
                        continue;
                    }
                    let join = plugin
                        .manifest
                        .contributions
                        .joins
                        .iter()
                        .find(|join| &join.id == join_id)
                        .ok_or_else(|| {
                            CapabilityError::Handler("Column join is no longer declared".into())
                        })?;
                    let objects = join_objects(
                        &cache,
                        &client_cache,
                        &core,
                        plugin,
                        join,
                        &context,
                        &input.namespace,
                    )
                    .await?;
                    joins.insert(join_id.clone(), (join.match_by.clone(), objects));
                }
                let cells = resolved_cells(&columns, &joins, &input.uids, &input.kind)?;
                Ok(ResolvedColumns { columns, cells })
            }
        },
    ));
}

fn join_key(namespace: &str, kind: &str, name: &str) -> String {
    format!("{namespace}\u{0}{kind}\u{0}{name}")
}

/// Each joined list is indexed once, then every row/column lookup is constant time.
fn record_match(index: &mut HashMap<String, Option<usize>>, key: String, position: usize) {
    use std::collections::hash_map::Entry;
    match index.entry(key) {
        Entry::Vacant(entry) => {
            entry.insert(Some(position));
        }
        Entry::Occupied(mut entry) if *entry.get() != Some(position) => {
            entry.insert(None);
        }
        _ => {}
    }
}

fn index_join(rule: &JoinMatch, objects: &[Value]) -> HashMap<String, Option<usize>> {
    let mut index = HashMap::new();
    for (position, object) in objects.iter().enumerate() {
        let metadata = &object["metadata"];
        let namespace = metadata["namespace"].as_str().unwrap_or("");
        if let Some(label) = &rule.label {
            if let Some(name) = metadata["labels"][label].as_str() {
                let kind = rule
                    .kind_label
                    .as_ref()
                    .and_then(|key| metadata["labels"][key].as_str())
                    .unwrap_or("");
                if rule.kind_label.is_none() || !kind.is_empty() {
                    record_match(&mut index, join_key(namespace, kind, name), position);
                }
            }
        } else if let Some(annotation) = &rule.annotation {
            if let Some(name) = metadata["annotations"][annotation].as_str() {
                record_match(&mut index, join_key(namespace, "", name), position);
            }
        } else if rule.name {
            if let Some(name) = metadata["name"].as_str() {
                record_match(&mut index, join_key(namespace, "", name), position);
            }
        } else if rule.owner_reference {
            if let Some(owners) = metadata["ownerReferences"].as_array() {
                for owner in owners {
                    let kind = owner["kind"].as_str().unwrap_or("");
                    if let Some(uid) = owner["uid"].as_str() {
                        record_match(
                            &mut index,
                            join_key(namespace, &format!("uid:{kind}"), uid),
                            position,
                        );
                    }
                    if let Some(name) = owner["name"].as_str() {
                        record_match(
                            &mut index,
                            join_key(namespace, &format!("name:{kind}"), name),
                            position,
                        );
                    }
                }
            }
        }
    }
    index
}

fn joined<'a>(
    rule: &JoinMatch,
    objects: &'a [Value],
    index: &HashMap<String, Option<usize>>,
    row: &ColumnRow,
    kind: &str,
) -> Result<Option<&'a Value>, ()> {
    let kind = kind.rsplit('/').next().unwrap_or(kind);
    let (kind_key, identity) = if rule.owner_reference {
        match row.uid.as_deref() {
            Some(uid) => (format!("uid:{kind}"), uid),
            None => (format!("name:{kind}"), row.name.as_str()),
        }
    } else if rule.kind_label.is_some() {
        (kind.to_owned(), row.name.as_str())
    } else {
        (String::new(), row.name.as_str())
    };
    let key = join_key(&row.namespace, &kind_key, identity);
    let exact = index.get(&key);
    let fallback = if row.namespace.is_empty() {
        None
    } else {
        index.get(&join_key("", &kind_key, identity))
    };
    match (exact, fallback) {
        (Some(_), Some(_)) | (Some(None), _) | (_, Some(None)) => Err(()),
        (Some(Some(position)), None) | (None, Some(Some(position))) => Ok(objects.get(*position)),
        (None, None) => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn two_join_rules_share_one_granted_reader_snapshot() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let cache: JoinCache = Arc::new(Mutex::new(HashMap::new()));
        let calls = AtomicUsize::new(0);
        let mut first_snapshot = None;
        for id in ["by-name", "by-label"] {
            let rule = JoinMatch {
                label: (id == "by-label").then(|| "target".into()),
                kind_label: None,
                owner_reference: false,
                annotation: None,
                name: id == "by-name",
            };
            let join = Join {
                id: id.into(),
                capability: "reports".into(),
                match_by: rule,
            };
            let result = cached_objects(
                &cache,
                key_for_join("app", 2, "prod", "team", &join),
                || async {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(vec![json!({"metadata":{"name":"report","namespace":"team","labels":{"target":"report"}}})])
                },
            )
            .await
            .unwrap();
            assert_eq!(result.len(), 1);
            if let Some(first) = &first_snapshot {
                assert!(Arc::ptr_eq(first, &result));
            } else {
                first_snapshot = Some(Arc::clone(&result));
            }
            let row = ColumnRow {
                uid: None,
                name: "report".into(),
                namespace: "team".into(),
                row: Value::Null,
            };
            let index = index_join(&join.match_by, &result);
            assert!(joined(&join.match_by, &result, &index, &row, "/Pod")
                .unwrap()
                .is_some());
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn expired_snapshot_is_released_even_when_reloading_fails() {
        let cache: JoinCache = Arc::new(Mutex::new(HashMap::new()));
        let join = Join {
            id: "by-name".into(),
            capability: "reports".into(),
            match_by: JoinMatch {
                label: None,
                kind_label: None,
                owner_reference: false,
                annotation: None,
                name: true,
            },
        };
        let key = key_for_join("app", 2, "prod", "team", &join);
        let slot = cache_slot(&cache, key.clone());
        *slot.lock().await = Some((
            Instant::now() - CACHE_TTL - Duration::from_secs(1),
            Arc::new(vec![json!({"large":"old"})]),
        ));
        assert!(cached_objects(&cache, key, || async {
            Err(CapabilityError::Handler("offline".into()))
        })
        .await
        .is_err());
        assert!(slot.lock().await.is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn idle_snapshot_releases_raw_objects_at_ttl() {
        let cache: JoinCache = Arc::new(Mutex::new(HashMap::new()));
        let join = Join {
            id: "by-name".into(),
            capability: "reports".into(),
            match_by: JoinMatch {
                label: None,
                kind_label: None,
                owner_reference: false,
                annotation: None,
                name: true,
            },
        };
        let key = key_for_join("app", 2, "prod", "team", &join);
        let slot = cache_slot(&cache, key.clone());
        let objects = cached_objects(&cache, key, || async {
            Ok(vec![json!({"large":"raw CR"})])
        })
        .await
        .unwrap();
        assert!(slot.lock().await.is_some());
        drop(objects);
        tokio::task::yield_now().await;
        tokio::time::advance(CACHE_TTL).await;
        tokio::task::yield_now().await;
        assert!(slot.lock().await.is_none());
    }

    #[test]
    fn join_keys_use_metadata_and_row_identity_without_crossing_namespaces() {
        let object = json!({"metadata":{"name":"report-api","namespace":"team","labels":{
            "trivy-operator.resource.name":"api","trivy-operator.resource.kind":"Deployment"
        },"ownerReferences":[{"uid":"uid-api","name":"api","kind":"Deployment"}]}});
        let row = ColumnRow {
            uid: Some("uid-api".into()),
            name: "api".into(),
            namespace: "team".into(),
            row: Value::Null,
        };
        let label = JoinMatch {
            label: Some("trivy-operator.resource.name".into()),
            kind_label: Some("trivy-operator.resource.kind".into()),
            owner_reference: false,
            annotation: None,
            name: false,
        };
        let objects = vec![object.clone()];
        let label_index = index_join(&label, &objects);
        assert_eq!(
            joined(&label, &objects, &label_index, &row, "apps/Deployment"),
            Ok(Some(&object))
        );
        assert_eq!(
            joined(&label, &objects, &label_index, &row, "apps/StatefulSet"),
            Ok(None)
        );
        let mut other = row.clone();
        other.namespace = "prod".into();
        assert_eq!(
            joined(&label, &objects, &label_index, &other, "apps/Deployment"),
            Ok(None)
        );
        let owner = JoinMatch {
            label: None,
            kind_label: None,
            owner_reference: true,
            annotation: None,
            name: false,
        };
        let owner_index = index_join(&owner, &objects);
        assert_eq!(
            joined(&owner, &objects, &owner_index, &row, "apps/Deployment"),
            Ok(Some(&object))
        );
        assert_eq!(
            joined(&owner, &objects, &owner_index, &row, "apps/StatefulSet"),
            Ok(None)
        );
        other.namespace = "team".into();
        other.uid = Some("different".into());
        assert_eq!(
            joined(&owner, &objects, &owner_index, &other, "apps/Deployment"),
            Ok(None)
        );
    }

    #[test]
    fn caller_payload_uses_uids_and_camel_case_row_fields() {
        let payload = json!({"id":"org.example.app","revision":2,"context":"prod","namespace":"team","kind":"apps/Deployment",
            "uids":[{"uid":"uid-1","name":"api","namespace":"team","row":{"name":"api"}}]});
        let input: ResolveColumns = serde_json::from_value(payload).unwrap();
        assert_eq!(input.uids[0].uid.as_deref(), Some("uid-1"));
        assert!(check_input(&input).is_ok());
        let mut old = json!({"id":"org.example.app","revision":2,"context":"prod","namespace":"team","kind":"apps/Deployment",
            "rows":[{"name":"api","namespace":"team"}]});
        assert!(serde_json::from_value::<ResolveColumns>(old.take()).is_err());
    }

    #[test]
    fn annotation_and_name_indexes_are_namespace_scoped() {
        let objects = vec![
            json!({"metadata":{"name":"api","namespace":"team","annotations":{"target":"web"}}}),
            json!({"metadata":{"name":"api","namespace":"prod","annotations":{"target":"web"}}}),
        ];
        let row = ColumnRow {
            uid: None,
            name: "web".into(),
            namespace: "prod".into(),
            row: Value::Null,
        };
        let annotation = JoinMatch {
            label: None,
            kind_label: None,
            owner_reference: false,
            annotation: Some("target".into()),
            name: false,
        };
        let index = index_join(&annotation, &objects);
        assert_eq!(
            joined(&annotation, &objects, &index, &row, "/Pod"),
            Ok(Some(&objects[1]))
        );
        let name = JoinMatch {
            label: None,
            kind_label: None,
            owner_reference: false,
            annotation: None,
            name: true,
        };
        let row = ColumnRow {
            name: "api".into(),
            ..row
        };
        let index = index_join(&name, &objects);
        assert_eq!(
            joined(&name, &objects, &index, &row, "/Pod"),
            Ok(Some(&objects[1]))
        );
    }

    #[test]
    fn duplicate_join_matches_report_ambiguity_instead_of_arbitrary_first_value() {
        let rule = JoinMatch {
            label: Some("target".into()),
            kind_label: None,
            owner_reference: false,
            annotation: None,
            name: false,
        };
        let objects = Arc::new(vec![
            json!({"metadata":{"name":"report-a","namespace":"team","labels":{"target":"api"}},"report":{"count":2}}),
            json!({"metadata":{"name":"report-b","namespace":"team","labels":{"target":"api"}},"report":{"count":5}}),
        ]);
        let joins = HashMap::from([("reports".into(), (rule, objects))]);
        let column = TableColumn {
            id: "count".into(),
            title: "Count".into(),
            for_kinds: vec!["/Pod".into()],
            source: srelens_plugin_host::ColumnSource {
                join: Some("reports".into()),
                json_path: ".report.count".into(),
            },
            format: srelens_plugin_host::ColumnFormat::Number,
            sortable: false,
            filterable: false,
        };
        let row = ColumnRow {
            uid: None,
            name: "api".into(),
            namespace: "team".into(),
            row: Value::Null,
        };
        let error = resolved_cells(&[column], &joins, &[row], "/Pod")
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("multiple joined resources"), "{error}");
    }

    #[test]
    fn a_long_scalar_is_an_explicit_read_error_not_an_unbounded_cell() {
        let column = TableColumn {
            id: "note".into(),
            title: "Note".into(),
            for_kinds: vec!["/Pod".into()],
            source: srelens_plugin_host::ColumnSource {
                join: None,
                json_path: ".note".into(),
            },
            format: srelens_plugin_host::ColumnFormat::Text,
            sortable: false,
            filterable: false,
        };
        let row = ColumnRow {
            uid: None,
            name: "api".into(),
            namespace: "team".into(),
            row: json!({"note":"x".repeat(4_097)}),
        };
        assert!(resolved_cells(&[column], &HashMap::new(), &[row], "/Pod").is_err());
    }

    #[tokio::test]
    async fn one_batch_resolves_a_thousand_rows_and_disable_revokes_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("apps.json");
        let core = super::super::tests::fake_core();
        let revision = super::super::tests::install(&path, core.clone());
        let mut state = read(&path).unwrap();
        state.plugins[0]
            .manifest
            .contributions
            .table_columns
            .push(TableColumn {
                id: "name".into(),
                title: "Name".into(),
                for_kinds: vec!["apps/Deployment".into()],
                source: srelens_plugin_host::ColumnSource {
                    join: None,
                    json_path: ".name".into(),
                },
                format: srelens_plugin_host::ColumnFormat::Text,
                sortable: true,
                filterable: true,
            });
        write(&path, &state).unwrap();
        let mut registry = Registry::new();
        register(
            &mut registry,
            path.clone(),
            core.clone(),
            srelens_kube::client_cache::ClientCache::new_many(vec![]),
        );
        let uids: Vec<Value> = (0..1_000)
            .map(|index| {
                json!({"name":format!("api-{index}"),"namespace":"team",
            "row":{"name":format!("api-{index}")}})
            })
            .collect();
        let payload = json!({"id":"org.example.argocd","revision":revision,"context":"cluster/a",
            "namespace":"team","kind":"apps/Deployment","uids":uids});
        let out = registry
            .invoke("extensions.resolveColumns", payload.clone())
            .await
            .unwrap();
        assert_eq!(out["cells"].as_array().unwrap().len(), 1_000);
        assert_eq!(out["cells"][999]["values"]["name"], json!("api-999"));
        mutate(
            &path,
            core,
            Configure::Enable {
                id: "org.example.argocd".into(),
                enabled: false,
            },
        )
        .unwrap();
        assert!(registry
            .invoke("extensions.resolveColumns", payload)
            .await
            .is_err());
    }
}
