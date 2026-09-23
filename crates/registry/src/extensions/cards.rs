//! `extensions.resolveCards`: the cluster dashboard's app cards (#540).
//!
//! One call per app answers every card it declares. Each card reads its
//! source through the same short reader snapshot table columns use, so a
//! dashboard with three cards over one reader lists it once, and a card that
//! cannot be read says why on that card alone.
use super::*;
use srelens_plugin_host::{Binding, CardAggregate, CardOrder, CardType, DashboardCard};

/// Most namespaces one call may narrow to. The dashboard's selection is a
/// person's pick from a list, not an export of the cluster.
pub(super) const MAX_NAMESPACES: usize = 256;

/// The longest value a list row carries. A row is one line on a card.
const MAX_ROW_VALUE_CHARS: usize = 256;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct ResolveCards {
    id: String,
    revision: u64,
    context: String,
    /// The dashboard's namespace selection; empty is every namespace.
    #[serde(default)]
    namespaces: Vec<String>,
}

/// Whether `namespace` is a Kubernetes namespace name.
pub(super) fn namespace_name(namespace: &str) -> bool {
    !namespace.is_empty()
        && namespace.len() <= 63
        && namespace
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        && !namespace.starts_with('-')
        && !namespace.ends_with('-')
}

pub(super) fn check_input(input: &ResolveCards) -> Result<(), CapabilityError> {
    if input.context.trim().is_empty() {
        return Err(CapabilityError::InvalidInput(
            "An explicit cluster context is required".into(),
        ));
    }
    if input.context.len() > 4_096 {
        return Err(CapabilityError::InvalidInput(
            "Context exceeds 4,096 bytes".into(),
        ));
    }
    if input.namespaces.len() > MAX_NAMESPACES {
        return Err(CapabilityError::InvalidInput(format!(
            "Narrow to at most {MAX_NAMESPACES} namespaces"
        )));
    }
    if !input
        .namespaces
        .iter()
        .all(|namespace| namespace_name(namespace))
    {
        return Err(CapabilityError::InvalidInput(
            "Each namespace must be a Kubernetes namespace name".into(),
        ));
    }
    Ok(())
}

/// The namespace to read a source in, and the selection to narrow what it
/// returns to — the rule the resource lists follow (`watchNamespaceForSelection`
/// in @srelens/core): one namespace is read directly, none or several read
/// every namespace and keep the selected ones. A cluster-scoped source has no
/// namespace to narrow by, so no selection applies to it.
pub(super) fn read_scope<'a>(
    binding: &Binding,
    namespaces: &'a [String],
) -> (String, Option<&'a [String]>) {
    let namespaced = binding
        .arguments
        .get("namespaced")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !namespaced {
        return (String::new(), None);
    }
    match namespaces {
        [one] => (one.clone(), Some(namespaces)),
        _ => (String::new(), Some(namespaces)),
    }
}

/// What one card says: a figure of its type, or why it has none.
#[derive(Debug, Serialize, JsonSchema, PartialEq)]
#[serde(tag = "state", rename_all = "camelCase")]
pub(super) enum Resolved {
    Count {
        count: usize,
    },
    CountByStatus {
        total: usize,
        statuses: Vec<StatusCount>,
    },
    Metric {
        /// `null` when no matching object carried a number to take a minimum
        /// or maximum of. A sum of nothing is `0`.
        value: Option<f64>,
        /// How many matching objects carried a number.
        counted: usize,
    },
    List {
        /// How many objects matched, of which `rows` are the first.
        total: usize,
        rows: Vec<ListRow>,
    },
    /// The card's source could not be read, or its figure could not be made.
    Error {
        reason: String,
    },
}

#[derive(Debug, Serialize, JsonSchema, PartialEq)]
pub(super) struct StatusCount {
    status: String,
    count: usize,
}

#[derive(Debug, Serialize, JsonSchema, PartialEq)]
pub(super) struct ListRow {
    namespace: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<String>,
}

/// One call's reads, by reader and namespace: the objects, or why they could not be listed.
type ReadsThisCall = std::collections::HashMap<(String, String), Result<Arc<Vec<Value>>, String>>;

/// How an app's objects map to a status word.
pub(super) type StatusResolver = dyn Fn(&Value) -> Option<String> + Send + Sync;

/// The status an app's own rules (#541) give each object its `source` lists:
/// the first rule that holds, or Unknown — the word the same object shows as
/// a badge on the app's list, so the card and the page count alike.
///
/// `None` when the source's kind has no `statusResolvers`. Installation
/// refuses a `countByStatus` card over such a source, and every read
/// revalidates the app first, so a card never reaches here without rules.
pub(super) fn status_resolver(manifest: &Manifest, source: &str) -> Option<Box<StatusResolver>> {
    let rules = manifest.status_rules_for_binding(source)?.to_vec();
    Some(Box::new(move |object| {
        Some(srelens_capability::status::resolve_status(&rules, object).label)
    }))
}

fn namespace_of(object: &Value) -> &str {
    object["metadata"]["namespace"].as_str().unwrap_or("")
}

fn name_of(object: &Value) -> &str {
    object["metadata"]["name"].as_str().unwrap_or("")
}

/// The objects a card counts: those in the selection whose predicate holds.
pub(super) fn matching<'a>(
    card: &'a DashboardCard,
    objects: &'a [Value],
    selection: Option<&'a [String]>,
    now: i64,
) -> impl Iterator<Item = &'a Value> + 'a {
    objects.iter().filter(move |object| {
        selection.is_none_or(|selected| {
            selected.is_empty() || selected.iter().any(|ns| ns == namespace_of(object))
        }) && card
            .predicate
            .as_ref()
            .is_none_or(|predicate| predicate.holds_at(object, now))
    })
}

/// A scalar drawn as one line, or nothing for a value that is not one.
fn scalar_text(value: &Value) -> Option<String> {
    let text = match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        _ => return None,
    };
    Some(text.chars().take(MAX_ROW_VALUE_CHARS).collect())
}

/// Orders two list values: numbers numerically, then text, then nothing.
fn compare_values(a: Option<&Value>, b: Option<&Value>) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let a = a.filter(|value| !value.is_null());
    let b = b.filter(|value| !value.is_null());
    match (a, b) {
        (Some(Value::Number(x)), Some(Value::Number(y))) => x
            .as_f64()
            .partial_cmp(&y.as_f64())
            .unwrap_or(Ordering::Equal),
        (Some(Value::Number(_)), Some(_)) => Ordering::Less,
        (Some(_), Some(Value::Number(_))) => Ordering::Greater,
        (Some(x), Some(y)) => scalar_text(x).cmp(&scalar_text(y)),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// One card's figure over the objects its source listed.
pub(super) fn resolve_card(
    card: &DashboardCard,
    objects: &[Value],
    selection: Option<&[String]>,
    now: i64,
    status: Option<&StatusResolver>,
) -> Resolved {
    let matched = matching(card, objects, selection, now);
    match card.card_type {
        CardType::Count => Resolved::Count {
            count: matched.count(),
        },
        CardType::CountByStatus => {
            let Some(status) = status else {
                // Installation refuses this card without rules, so this is a
                // manifest the host should not be holding: fail closed, never 0.
                return Resolved::Error {
                    reason: "This card's source has no status resolver to count by".into(),
                };
            };
            let mut counts = std::collections::BTreeMap::<String, usize>::new();
            let mut total = 0;
            for object in matched {
                total += 1;
                *counts
                    .entry(status(object).unwrap_or_else(|| "Unknown".into()))
                    .or_default() += 1;
            }
            let mut statuses: Vec<StatusCount> = counts
                .into_iter()
                .map(|(status, count)| StatusCount { status, count })
                .collect();
            statuses.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.status.cmp(&b.status)));
            Resolved::CountByStatus { total, statuses }
        }
        CardType::Metric => {
            let Some(metric) = &card.metric else {
                return Resolved::Error {
                    reason: "This metric card declares no value to read".into(),
                };
            };
            let mut numbers = Vec::new();
            for object in matched {
                match srelens_capability::resolve(object, &metric.json_path) {
                    None | Some(Value::Null) => {}
                    Some(Value::Number(number)) => match number.as_f64() {
                        Some(number) => numbers.push(number),
                        None => {
                            return Resolved::Error {
                                reason: format!(
                                    "{} on {}/{} is not a number this host can add",
                                    metric.json_path,
                                    namespace_of(object),
                                    name_of(object)
                                ),
                            }
                        }
                    },
                    Some(_) => {
                        return Resolved::Error {
                            reason: format!(
                                "{} on {}/{} is not a number",
                                metric.json_path,
                                namespace_of(object),
                                name_of(object)
                            ),
                        }
                    }
                }
            }
            let value = match metric.aggregate {
                CardAggregate::Sum => Some(numbers.iter().sum::<f64>()),
                CardAggregate::Min => numbers.iter().copied().reduce(f64::min),
                CardAggregate::Max => numbers.iter().copied().reduce(f64::max),
            };
            if value.is_some_and(|value| !value.is_finite()) {
                return Resolved::Error {
                    reason: format!(
                        "The {} of {} is too large to show",
                        match metric.aggregate {
                            CardAggregate::Sum => "sum",
                            CardAggregate::Min => "minimum",
                            CardAggregate::Max => "maximum",
                        },
                        metric.json_path
                    ),
                };
            }
            Resolved::Metric {
                value,
                counted: numbers.len(),
            }
        }
        CardType::List => {
            let path = card
                .list
                .as_ref()
                .and_then(|list| list.json_path.as_deref());
            let descending = card
                .list
                .as_ref()
                .and_then(|list| list.order)
                .is_some_and(|order| order == CardOrder::Desc);
            let limit = card
                .list
                .as_ref()
                .and_then(|list| list.limit)
                .unwrap_or_else(|| card.size.default_rows())
                .min(srelens_plugin_host::MAX_CARD_LIST_ROWS);
            let mut rows: Vec<(&Value, Option<&Value>)> = matched
                .map(|object| {
                    (
                        object,
                        path.and_then(|path| srelens_capability::resolve(object, path)),
                    )
                })
                .collect();
            let total = rows.len();
            let identity =
                |object: &Value| (namespace_of(object).to_owned(), name_of(object).to_owned());
            rows.sort_by(|(a, a_value), (b, b_value)| {
                let by_value = if descending {
                    // Missing values stay last either way round.
                    match (
                        a_value.filter(|v| !v.is_null()),
                        b_value.filter(|v| !v.is_null()),
                    ) {
                        (Some(_), Some(_)) => compare_values(*b_value, *a_value),
                        _ => compare_values(*a_value, *b_value),
                    }
                } else {
                    compare_values(*a_value, *b_value)
                };
                by_value.then_with(|| identity(a).cmp(&identity(b)))
            });
            Resolved::List {
                total,
                rows: rows
                    .into_iter()
                    .take(limit)
                    .map(|(object, value)| ListRow {
                        namespace: namespace_of(object).to_owned(),
                        name: name_of(object).to_owned(),
                        value: value.and_then(scalar_text),
                    })
                    .collect(),
            }
        }
    }
}

/// One card's answer, under the id the manifest gave it.
#[derive(Debug, Serialize, JsonSchema)]
struct ResolvedCard {
    id: String,
    #[serde(flatten)]
    resolved: Resolved,
}

#[derive(Debug, Serialize, JsonSchema)]
struct ResolvedCards {
    cards: Vec<ResolvedCard>,
}

/// The message a failed read carries, without the error kind's prefix: it is
/// shown on a card, where "handler error:" says nothing a person can use.
fn reason(error: &CapabilityError) -> String {
    match error {
        CapabilityError::NotFound(message)
        | CapabilityError::InvalidInput(message)
        | CapabilityError::Handler(message) => message.clone(),
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
        })
}

/// The installed app a request names, when it may be used for it: enabled, at
/// the revision the view was drawn from, allowed on this cluster and still
/// valid against this host.
fn usable_app<'a>(
    state: &'a Inventory,
    id: &str,
    revision: u64,
    resolved: &Result<srelens_kube::context_resolve::ResolvedContext, String>,
    core: Arc<Registry>,
) -> Result<&'a Installed, CapabilityError> {
    let plugin = state
        .plugins
        .iter()
        .find(|plugin| plugin.manifest.id == id)
        .ok_or_else(|| {
            CapabilityError::Handler("Extension was removed; refresh the view".into())
        })?;
    if let Some(reason) = &plugin.policy_blocked {
        return Err(CapabilityError::Handler(reason.clone()));
    }
    if !plugin.enabled || plugin.revision != revision {
        return Err(CapabilityError::Handler(
            "Extension was disabled or updated; refresh the view".into(),
        ));
    }
    plugin.check_scope(resolved)?;
    validate_app(&plugin.manifest, &plugin.grants, core)
        .map_err(|errors| CapabilityError::Handler(errors.to_string()))?;
    Ok(plugin)
}

fn source_binding<'a>(plugin: &'a Installed, source: &str) -> Result<&'a Binding, CapabilityError> {
    plugin
        .manifest
        .capabilities
        .iter()
        .find(|binding| binding.name == source && binding.target == "k8s.listCustomResource")
        .ok_or_else(|| CapabilityError::Handler("The card's reader is no longer declared".into()))
}

/// The rules for the namespace list a page read may carry: only with a card,
/// only instead of a single namespace, and as bounded as the card's own read.
pub(super) fn check_card_namespaces(
    card: bool,
    namespace: &str,
    namespaces: &[String],
) -> Result<(), CapabilityError> {
    if namespaces.is_empty() {
        return Ok(());
    }
    if !card {
        return Err(CapabilityError::InvalidInput(
            "A namespace list narrows a dashboard card's rows; name the card".into(),
        ));
    }
    if !namespace.is_empty() {
        return Err(CapabilityError::InvalidInput(
            "Name one namespace or a list of them, not both".into(),
        ));
    }
    if namespaces.len() > MAX_NAMESPACES {
        return Err(CapabilityError::InvalidInput(format!(
            "Narrow to at most {MAX_NAMESPACES} namespaces"
        )));
    }
    if !namespaces.iter().all(|namespace| namespace_name(namespace)) {
        return Err(CapabilityError::InvalidInput(
            "Each namespace must be a Kubernetes namespace name".into(),
        ));
    }
    Ok(())
}

/// Which of a page's rows a card counted, by namespace and name: what its
/// target route shows (#540). The same snapshot and predicate the card read,
/// so the page and the card agree on what "matching" meant.
#[allow(clippy::too_many_arguments)]
pub(super) async fn card_rows(
    snapshots: &columns::JoinCache,
    client_cache: &srelens_kube::client_cache::ClientCache,
    core: &Registry,
    plugin: &Installed,
    card_id: &str,
    capability: &str,
    context: &str,
    namespace: &str,
    namespaces: &[String],
) -> Result<std::collections::HashSet<(String, String)>, CapabilityError> {
    let card = plugin
        .manifest
        .contributions
        .dashboard_cards
        .iter()
        .find(|card| card.id == card_id)
        .ok_or_else(|| {
            CapabilityError::Handler(format!(
                "Dashboard card \"{card_id}\" is no longer declared; refresh the view"
            ))
        })?;
    if card.source != capability {
        return Err(CapabilityError::InvalidInput(format!(
            "Dashboard card \"{card_id}\" does not count this page's resources"
        )));
    }
    let binding = source_binding(plugin, &card.source)?;
    // The same selection the card counted in: one namespace, several, or all.
    let selected: Vec<String> = if namespaces.is_empty() {
        [namespace]
            .into_iter()
            .filter(|namespace| !namespace.is_empty())
            .map(str::to_owned)
            .collect()
    } else {
        namespaces.to_vec()
    };
    let (read_in, selection) = read_scope(binding, &selected);
    let objects = columns::reader_objects(
        snapshots,
        client_cache,
        core,
        plugin,
        &card.source,
        context,
        &read_in,
    )
    .await?;
    Ok(matching(card, &objects, selection, now())
        .map(|object| (namespace_of(object).to_owned(), name_of(object).to_owned()))
        .collect())
}

pub(super) fn register(
    reg: &mut Registry,
    path: PathBuf,
    core: Arc<Registry>,
    client_cache: Arc<srelens_kube::client_cache::ClientCache>,
    snapshots: columns::JoinCache,
) {
    reg.register(Capability::typed::<ResolveCards, ResolvedCards, _, _>(
        "extensions.resolveCards",
        "Resolve the cluster dashboard cards an enabled extension declares, each to a figure or the reason it has none",
        Annotations::READ_ONLY,
        move |input| {
            let path = path.clone();
            let core = core.clone();
            let client_cache = client_cache.clone();
            let snapshots = snapshots.clone();
            async move {
                check_input(&input)?;
                let resolved = request_context(&client_cache, &input.context).await;
                let state = tokio::task::spawn_blocking(move || read(&path))
                    .await
                    .map_err(|error| CapabilityError::Handler(error.to_string()))?
                    .map_err(CapabilityError::Handler)?;
                let plugin = usable_app(&state, &input.id, input.revision, &resolved, core.clone())?;
                let context = resolved
                    .ok()
                    .and_then(|context| context.pinned_id())
                    .unwrap_or(input.context);
                let now = now();
                // Each reader is read once per call, whatever it answers: the shared
                // snapshot keeps a success for five seconds but lets a later caller
                // retry a failure, which would otherwise be one failed list per card.
                let mut reads: ReadsThisCall = std::collections::HashMap::new();
                let mut cards = Vec::new();
                for card in &plugin.manifest.contributions.dashboard_cards {
                    let status = status_resolver(&plugin.manifest, &card.source);
                    let resolved = match source_binding(plugin, &card.source) {
                        Err(error) => Resolved::Error { reason: reason(&error) },
                        Ok(binding) => {
                            let (namespace, selection) = read_scope(binding, &input.namespaces);
                            let key = (card.source.clone(), namespace);
                            if !reads.contains_key(&key) {
                                let read = columns::reader_objects(
                                    &snapshots, &client_cache, &core, plugin, &card.source,
                                    &context, &key.1,
                                )
                                .await
                                .map_err(|error| reason(&error));
                                reads.insert(key.clone(), read);
                            }
                            match &reads[&key] {
                                Ok(objects) => resolve_card(card, objects, selection, now, status.as_deref()),
                                Err(reason) => Resolved::Error { reason: reason.clone() },
                            }
                        }
                    };
                    cards.push(ResolvedCard { id: card.id.clone(), resolved });
                }
                Ok(ResolvedCards { cards })
            }
        },
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use srelens_plugin_host::DashboardCard;

    /// 2026-09-23T00:00:00Z.
    const NOW: i64 = 1_790_121_600;

    fn card(value: Value) -> DashboardCard {
        let mut base =
            json!({"id":"card","title":"Card","size":"m","type":"count","source":"certificates"});
        base.as_object_mut()
            .unwrap()
            .extend(value.as_object().unwrap().clone());
        serde_json::from_value(base).unwrap()
    }

    fn cert(namespace: &str, name: &str, not_after: &str, critical: Value) -> Value {
        json!({"metadata":{"name":name,"namespace":namespace},
            "status":{"notAfter":not_after},"report":{"critical":critical}})
    }

    fn certificates() -> Vec<Value> {
        vec![
            cert("team", "web", "2026-09-25T00:00:00Z", json!(3)),
            cert("team", "api", "2026-12-01T00:00:00Z", json!(0)),
            cert("prod", "shop", "2026-09-30T00:00:00Z", json!(7)),
            cert("prod", "old", "2026-09-01T00:00:00Z", Value::Null),
        ]
    }

    fn resolved(card: &DashboardCard, objects: &[Value], filter: Option<&[String]>) -> Value {
        serde_json::to_value(resolve_card(card, objects, filter, NOW, None)).unwrap()
    }

    #[test]
    fn count_is_the_objects_the_predicate_holds_for() {
        let soon = card(json!({"predicate":{"jsonPath":".status.notAfter","within":"14d"}}));
        assert_eq!(
            resolved(&soon, &certificates(), None),
            json!({"state":"count","count":2})
        );
        // No predicate counts every object.
        assert_eq!(
            resolved(&card(json!({})), &certificates(), None)["count"],
            4
        );
    }

    #[test]
    fn zero_is_an_answer_not_an_absence() {
        let never = card(json!({"predicate":{"jsonPath":".status.notAfter","before":"-3650d"}}));
        assert_eq!(
            resolved(&never, &certificates(), None),
            json!({"state":"count","count":0})
        );
        assert_eq!(
            resolved(&card(json!({})), &[], None),
            json!({"state":"count","count":0})
        );
    }

    #[test]
    fn a_namespace_selection_narrows_namespaced_objects_only() {
        let all = card(json!({}));
        let prod = ["prod".to_owned()];
        assert_eq!(resolved(&all, &certificates(), Some(&prod))["count"], 2);
        let both = ["prod".to_owned(), "team".to_owned()];
        assert_eq!(resolved(&all, &certificates(), Some(&both))["count"], 4);
        // An empty selection is every namespace.
        assert_eq!(resolved(&all, &certificates(), Some(&[]))["count"], 4);
        // A cluster-scoped source carries no namespace; the filter does not apply.
        let issuers = vec![json!({"metadata":{"name":"letsencrypt"}})];
        assert_eq!(resolved(&all, &issuers, None)["count"], 1);
    }

    #[test]
    fn metric_reduces_the_numbers_of_matching_objects() {
        let metric = |aggregate: &str| {
            card(
                json!({"type":"metric","metric":{"jsonPath":".report.critical","aggregate":aggregate}}),
            )
        };
        assert_eq!(
            resolved(&metric("sum"), &certificates(), None),
            json!({"state":"metric","value":10.0,"counted":3})
        );
        assert_eq!(
            resolved(&metric("min"), &certificates(), None)["value"],
            0.0
        );
        assert_eq!(
            resolved(&metric("max"), &certificates(), None)["value"],
            7.0
        );
        // Nothing to reduce: a sum of nothing is zero, a minimum of nothing is no value.
        assert_eq!(
            resolved(&metric("sum"), &[], None),
            json!({"state":"metric","value":0.0,"counted":0})
        );
        assert_eq!(
            resolved(&metric("min"), &[], None),
            json!({"state":"metric","value":null,"counted":0})
        );
    }

    #[test]
    fn a_metric_over_a_value_that_is_not_a_number_is_an_error_naming_the_object() {
        let sum = card(
            json!({"type":"metric","metric":{"jsonPath":".report.critical","aggregate":"sum"}}),
        );
        let mut objects = certificates();
        objects.push(cert("team", "odd", "2026-09-25T00:00:00Z", json!("three")));
        let out = resolved(&sum, &objects, None);
        assert_eq!(out["state"], "error", "{out}");
        assert!(
            out["reason"].as_str().unwrap().contains("team/odd"),
            "{out}"
        );
        assert!(
            out.get("value").is_none(),
            "a failed metric carries no figure: {out}"
        );
    }

    #[test]
    fn list_orders_by_its_value_and_stops_at_its_limit() {
        let soonest = card(json!({"type":"list","size":"s",
            "predicate":{"jsonPath":".status.notAfter","before":"30d"},
            "list":{"jsonPath":".status.notAfter"}}));
        let out = resolved(&soonest, &certificates(), None);
        assert_eq!(out["state"], "list");
        assert_eq!(out["total"], 3);
        assert_eq!(
            out["rows"],
            json!([
                {"namespace":"prod","name":"old","value":"2026-09-01T00:00:00Z"},
                {"namespace":"team","name":"web","value":"2026-09-25T00:00:00Z"},
                {"namespace":"prod","name":"shop","value":"2026-09-30T00:00:00Z"},
            ])
        );
        let latest = card(json!({"type":"list",
            "list":{"jsonPath":".report.critical","order":"desc","limit":2}}));
        let out = resolved(&latest, &certificates(), None);
        assert_eq!(out["total"], 4);
        assert_eq!(
            out["rows"],
            json!([
                {"namespace":"prod","name":"shop","value":"7"},
                {"namespace":"team","name":"web","value":"3"},
            ])
        );
        // No value path: by namespace, then name.
        let plain = card(json!({"type":"list","size":"s"}));
        let names: Vec<_> = resolved(&plain, &certificates(), None)["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["name"].clone())
            .collect();
        assert_eq!(names, [json!("old"), json!("shop"), json!("api")]);
    }

    #[test]
    fn count_by_status_without_rules_is_an_error_never_a_count() {
        // Installation refuses this card, so only a manifest the host should not
        // hold gets here; it fails closed rather than guessing a status.
        let by_status = card(json!({"type":"countByStatus"}));
        let out = resolved(&by_status, &certificates(), None);
        assert_eq!(out["state"], "error", "{out}");
        assert!(
            out["reason"].as_str().unwrap().contains("status resolver"),
            "{out}"
        );
        assert!(
            out.get("count").is_none() && out.get("total").is_none(),
            "{out}"
        );
    }

    #[test]
    fn count_by_status_counts_through_the_status_seam_when_a_resolver_exists() {
        let by_status = card(json!({"type":"countByStatus"}));
        let resolver = |object: &Value| -> Option<String> {
            match object["report"]["critical"].as_i64() {
                Some(0) => Some("Healthy".into()),
                Some(_) => Some("Degraded".into()),
                None => None,
            }
        };
        let out = serde_json::to_value(resolve_card(
            &by_status,
            &certificates(),
            None,
            NOW,
            Some(&resolver),
        ))
        .unwrap();
        assert_eq!(
            out,
            json!({"state":"countByStatus","total":4,"statuses":[
                {"status":"Degraded","count":2},{"status":"Healthy","count":1},{"status":"Unknown","count":1}
            ]})
        );
    }

    #[test]
    fn the_status_resolver_is_the_apps_own_rules_for_the_sources_kind() {
        let mut value: Value = serde_json::from_str(&super::super::tests::manifest()).unwrap();
        let bare: Manifest = serde_json::from_value(value.clone()).unwrap();
        assert!(
            status_resolver(&bare, "applications").is_none(),
            "no rules, no resolver"
        );
        value["contributions"]["statusResolvers"] = json!([{
            "forKinds":["argoproj.io/Application"],
            "rules":[{"when":[{"jsonPath":".spec.suspend","equals":true}],"status":"suspended","label":"Paused"}]
        }]);
        let manifest: Manifest = serde_json::from_value(value).unwrap();
        let resolve =
            status_resolver(&manifest, "applications").expect("the reader's kind has rules");
        assert_eq!(
            resolve(&json!({"spec":{"suspend":true}})).as_deref(),
            Some("Paused")
        );
        // No rule holds: Unknown, as the same object's badge on the app's list says.
        assert_eq!(resolve(&json!({"spec":{}})).as_deref(), Some("Unknown"));
    }

    #[test]
    fn caller_payload_uses_the_wrappers_spelling_and_is_bounded() {
        let input: ResolveCards = serde_json::from_value(json!({
            "id":"org.example.argocd","revision":2,"context":"prod","namespaces":["team","prod"]}))
        .unwrap();
        assert!(check_input(&input).is_ok());
        assert!(serde_json::from_value::<ResolveCards>(json!({
            "id":"org.example.argocd","revision":2,"context":"prod","namespace":"team"}))
        .is_err());
        let too_many = ResolveCards {
            namespaces: (0..=MAX_NAMESPACES).map(|i| format!("ns-{i}")).collect(),
            ..input
        };
        assert!(check_input(&too_many)
            .unwrap_err()
            .to_string()
            .contains("namespaces"));
        for bad in ["Team", "-team", &"x".repeat(64)] {
            let input = ResolveCards {
                namespaces: vec![bad.to_owned()],
                ..serde_json::from_value(json!({"id":"a.b","revision":1,"context":"prod"})).unwrap()
            };
            assert!(check_input(&input).is_err(), "{bad}");
        }
        let blank = ResolveCards {
            context: " ".into(),
            ..serde_json::from_value(json!({"id":"a.b","revision":1,"context":"prod"})).unwrap()
        };
        assert!(check_input(&blank).is_err());
    }

    /// A Kubernetes API server on loopback that answers every request with
    /// `status` and `body`, recording each request's path and query.
    fn api_server(status: u16, body: Value) -> (u16, Arc<std::sync::Mutex<Vec<String>>>) {
        paged_api_server(status, move |_| body.clone())
    }

    /// The same, with the body for the nth request (from 1).
    fn paged_api_server(
        status: u16,
        body: impl Fn(usize) -> Value + Send + 'static,
    ) -> (u16, Arc<std::sync::Mutex<Vec<String>>>) {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let record = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    continue;
                }
                record
                    .lock()
                    .unwrap()
                    .push(line.split(' ').nth(1).unwrap_or("").to_owned());
                // Drain the headers; these requests carry no body.
                loop {
                    let mut header = String::new();
                    if reader.read_line(&mut header).is_err()
                        || header == "\r\n"
                        || header.is_empty()
                    {
                        break;
                    }
                }
                let text = body(record.lock().unwrap().len()).to_string();
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}",
                    text.len()
                );
            }
        });
        (port, seen)
    }

    fn application(namespace: &str, name: &str, health: &str) -> Value {
        json!({"apiVersion":"argoproj.io/v1alpha1","kind":"Application",
            "metadata":{"name":name,"namespace":namespace},
            "status":{"health":{"status":health}}})
    }

    fn application_list(items: Vec<Value>) -> Value {
        json!({"apiVersion":"argoproj.io/v1alpha1","kind":"ApplicationList","metadata":{},"items":items})
    }

    /// The example app with a count, a list and a status card over its reader,
    /// installed against a kubeconfig whose one context `mock` is `port`.
    fn installed(port: u16, core: Arc<Registry>) -> (tempfile::TempDir, Registry, u64) {
        let dir = tempfile::tempdir().unwrap();
        let kubeconfig = dir.path().join("config");
        std::fs::write(
            &kubeconfig,
            format!("apiVersion: v1\nkind: Config\nclusters:\n- name: c\n  cluster:\n    server: http://127.0.0.1:{port}\nusers:\n- name: u\n  user:\n    token: t\ncontexts:\n- name: mock\n  context:\n    cluster: c\n    user: u\ncurrent-context: mock\n"),
        )
        .unwrap();
        let mut manifest: Value = serde_json::from_str(&super::super::tests::manifest()).unwrap();
        manifest["contributions"]["dashboardCards"] = json!([
            {"id":"degraded","title":"Degraded applications","size":"s","type":"count","source":"applications",
             "predicate":{"jsonPath":".status.health.status","equals":"Degraded"},
             "target":{"page":"applications"}},
            {"id":"all","title":"Applications","size":"l","type":"list","source":"applications"},
            {"id":"by-status","title":"By status","size":"m","type":"countByStatus","source":"applications"}
        ]);
        // The app's own status rules (#541) for the kind its reader lists.
        manifest["contributions"]["statusResolvers"] = json!([{
            "forKinds":["argoproj.io/Application"],
            "rules":[
                {"when":[{"jsonPath":".status.health.status","equals":"Healthy"}],"status":"healthy","label":"Healthy"},
                {"when":[{"jsonPath":".status.health.status","equals":"Degraded"}],"status":"error","label":"Degraded"},
                {"when":[],"status":"unknown","label":"Unknown"}
            ]
        }]);
        let path = dir.path().join("extensions.json");
        let revision = mutate(
            &path,
            core.clone(),
            Configure::Install {
                signature: None,
                manifest: manifest.to_string(),
                grants: vec!["k8s.listCustomResource".into()],
                reviewed_revision: None,
            },
        )
        .unwrap()
        .plugins[0]
            .revision;
        let mut registry = Registry::new();
        super::super::register(
            &mut registry,
            path,
            core,
            srelens_kube::client_cache::ClientCache::new_many(vec![kubeconfig]),
        );
        (dir, registry, revision)
    }

    fn cards_payload(revision: u64, namespaces: Value) -> Value {
        json!({"id":"org.example.argocd","revision":revision,"context":"mock","namespaces":namespaces})
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cards_read_their_source_over_http_once_and_follow_one_selected_namespace() {
        let (port, paths) = api_server(
            200,
            application_list(vec![
                application("team", "web", "Degraded"),
                application("team", "api", "Healthy"),
                application("team", "db", "Degraded"),
            ]),
        );
        let (_dir, registry, revision) = installed(port, super::super::tests::fake_core());
        let out = registry
            .invoke(
                "extensions.resolveCards",
                cards_payload(revision, json!(["team"])),
            )
            .await
            .unwrap();
        let cards = out["cards"].as_array().unwrap();
        assert_eq!(cards[0], json!({"id":"degraded","state":"count","count":2}));
        assert_eq!(cards[1]["state"], "list");
        assert_eq!(cards[1]["total"], 3);
        assert_eq!(
            cards[1]["rows"][0],
            json!({"namespace":"team","name":"api"})
        );
        // Counted by the app's own status rules, from the same one list.
        assert_eq!(
            cards[2],
            json!({"id":"by-status","state":"countByStatus","total":3,"statuses":[
                {"status":"Degraded","count":2},{"status":"Healthy","count":1}
            ]})
        );
        let paths = paths.lock().unwrap().clone();
        // Three cards over one reader in one namespace: one list request.
        assert_eq!(paths.len(), 1, "{paths:?}");
        assert!(
            paths[0].starts_with("/apis/argoproj.io/v1alpha1/namespaces/team/applications"),
            "{paths:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn several_selected_namespaces_read_every_namespace_and_keep_the_selected() {
        let (port, paths) = api_server(
            200,
            application_list(vec![
                application("team", "web", "Degraded"),
                application("prod", "shop", "Degraded"),
                application("other", "noise", "Degraded"),
            ]),
        );
        let (_dir, registry, revision) = installed(port, super::super::tests::fake_core());
        let out = registry
            .invoke(
                "extensions.resolveCards",
                cards_payload(revision, json!(["team", "prod"])),
            )
            .await
            .unwrap();
        assert_eq!(out["cards"][0]["count"], 2, "{out}");
        assert!(paths.lock().unwrap()[0].starts_with("/apis/argoproj.io/v1alpha1/applications"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_refused_read_is_an_error_on_each_card_with_the_reason_never_a_zero() {
        let (port, _paths) = api_server(
            403,
            json!({"apiVersion":"v1","kind":"Status","status":"Failure",
            "code":403,"reason":"Forbidden","message":"applications.argoproj.io is forbidden"}),
        );
        let (_dir, registry, revision) = installed(port, super::super::tests::fake_core());
        let out = registry
            .invoke(
                "extensions.resolveCards",
                cards_payload(revision, json!([])),
            )
            .await
            .unwrap();
        for card in out["cards"].as_array().unwrap().iter().take(2) {
            assert_eq!(card["state"], "error", "{card}");
            assert!(
                card["reason"].as_str().unwrap().contains("forbidden"),
                "{card}"
            );
            assert!(
                card.get("count").is_none() && card.get("rows").is_none(),
                "{card}"
            );
        }
        // A status card reads the cluster too, and a refused read is its error, not a zero.
        assert_eq!(out["cards"][2]["state"], "error", "{out}");
        assert!(out["cards"][2].get("total").is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_source_past_the_read_limit_is_an_error_not_a_smaller_count() {
        // Every page says more remain, so the host stops at its 2,000-object cap.
        let (port, paths) = paged_api_server(200, |n| {
            let mut page = application_list(
                (0..500)
                    .map(|i| application("team", &format!("app-{n}-{i}"), "Degraded"))
                    .collect(),
            );
            page["metadata"]["continue"] = json!(format!("page-{n}"));
            page
        });
        let (_dir, registry, revision) = installed(port, super::super::tests::fake_core());
        let out = registry
            .invoke(
                "extensions.resolveCards",
                cards_payload(revision, json!(["team"])),
            )
            .await
            .unwrap();
        assert_eq!(out["cards"][0]["state"], "error", "{out}");
        assert!(
            out["cards"][0]["reason"]
                .as_str()
                .unwrap()
                .contains("2,000"),
            "{out}"
        );
        assert!(out["cards"][0].get("count").is_none());
        assert_eq!(
            paths.lock().unwrap().len(),
            4,
            "bounded to the cap, not the cluster"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_disabled_app_answers_no_cards() {
        let (port, _paths) = api_server(200, application_list(vec![]));
        let core = super::super::tests::fake_core();
        let (dir, registry, revision) = installed(port, core.clone());
        mutate(
            &dir.path().join("extensions.json"),
            core,
            Configure::Enable {
                id: "org.example.argocd".into(),
                enabled: false,
            },
        )
        .unwrap();
        let error = registry
            .invoke(
                "extensions.resolveCards",
                cards_payload(revision, json!([])),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("disabled"), "{error}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_target_page_read_keeps_only_what_the_card_counted() {
        let (port, _paths) = api_server(
            200,
            application_list(vec![
                application("team", "web", "Degraded"),
                application("team", "api", "Healthy"),
            ]),
        );
        // The page's own summary read, as `k8s.listCustomResource` answers it.
        let mut core = (*super::super::tests::fake_core()).clone();
        let mut list = core.get("k8s.listCustomResource").unwrap().clone();
        list.handler = Arc::new(|_| {
            Box::pin(async {
                Ok(json!({"items":[
                    {"name":"web","namespace":"team","age":"1d","columns":["Degraded"]},
                    {"name":"api","namespace":"team","age":"1d","columns":["Healthy"]},
                    {"name":"new","namespace":"team","age":"1s","columns":["Degraded"]}
                ]}))
            })
        });
        core.register(list);
        let (_dir, registry, revision) = installed(port, Arc::new(core));
        let read = |card: Option<&str>| {
            let mut payload = json!({"id":"org.example.argocd","revision":revision,
                "capability":"applications","context":"mock","namespace":"team"});
            if let Some(card) = card {
                payload["card"] = json!(card);
            }
            registry.invoke("extensions.read", payload)
        };
        let names = |out: &Value| -> Vec<String> {
            out["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|i| i["name"].as_str().unwrap().to_owned())
                .collect()
        };
        assert_eq!(names(&read(None).await.unwrap()), ["web", "api", "new"]);
        // `new` is not in the card's snapshot, so it is not shown as something the card counted.
        assert_eq!(names(&read(Some("degraded")).await.unwrap()), ["web"]);
        let missing = read(Some("gone")).await.unwrap_err();
        assert!(missing.to_string().contains("gone"), "{missing}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_target_read_over_several_namespaces_keeps_exactly_what_the_card_counted() {
        let (port, _paths) = api_server(
            200,
            application_list(vec![
                application("team", "web", "Degraded"),
                application("prod", "shop", "Degraded"),
                application("other", "noise", "Degraded"),
            ]),
        );
        let mut core = (*super::super::tests::fake_core()).clone();
        let mut list = core.get("k8s.listCustomResource").unwrap().clone();
        list.handler = Arc::new(|_| {
            Box::pin(async {
                Ok(json!({"items":[
                    {"name":"web","namespace":"team","age":"1d","columns":[]},
                    {"name":"shop","namespace":"prod","age":"1d","columns":[]},
                    {"name":"noise","namespace":"other","age":"1d","columns":[]}
                ]}))
            })
        });
        core.register(list);
        let (_dir, registry, revision) = installed(port, Arc::new(core));
        let payload = json!({"id":"org.example.argocd","revision":revision,"capability":"applications",
            "context":"mock","namespace":"","card":"degraded","namespaces":["team","prod"]});
        let cards = registry
            .invoke(
                "extensions.resolveCards",
                cards_payload(revision, json!(["team", "prod"])),
            )
            .await
            .unwrap();
        let out = registry.invoke("extensions.read", payload).await.unwrap();
        let mut names: Vec<_> = out["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["name"].as_str().unwrap().to_owned())
            .collect();
        names.sort();
        // `noise` matches the predicate too, but in a namespace the card did not count.
        assert_eq!(names, ["shop", "web"]);
        assert_eq!(cards["cards"][0]["count"], names.len());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_namespace_list_narrows_only_a_cards_read_and_is_bounded() {
        let (port, _paths) = api_server(200, application_list(vec![]));
        let (_dir, registry, revision) = installed(port, super::super::tests::fake_core());
        let read = |extra: Value| {
            let mut payload = json!({"id":"org.example.argocd","revision":revision,
                "capability":"applications","context":"mock"});
            payload
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            registry.invoke("extensions.read", payload)
        };
        for (extra, why) in [
            (
                json!({"namespaces":["team","prod"]}),
                "a namespace list without a card",
            ),
            (
                json!({"card":"degraded","namespace":"team","namespaces":["team","prod"]}),
                "one namespace and a list",
            ),
            (
                json!({"card":"degraded","namespaces":["Team"]}),
                "a name that is not a namespace",
            ),
            (
                json!({"card":"degraded","namespaces":(0..=MAX_NAMESPACES).map(|i| format!("n{i}")).collect::<Vec<_>>()}),
                "too many",
            ),
        ] {
            let error = read(extra).await.unwrap_err();
            assert!(
                matches!(error, CapabilityError::InvalidInput(_)),
                "{why}: {error}"
            );
            assert!(
                !error.to_string().contains("unknown field"),
                "{why}: refused for its own reason, not the schema's: {error}"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_target_read_whose_card_cannot_be_evaluated_fails_rather_than_showing_everything() {
        let (port, _paths) = api_server(
            403,
            json!({"apiVersion":"v1","kind":"Status","status":"Failure",
            "code":403,"reason":"Forbidden","message":"forbidden"}),
        );
        let (_dir, registry, revision) = installed(port, super::super::tests::fake_core());
        let error = registry
            .invoke(
                "extensions.read",
                json!({"id":"org.example.argocd","revision":revision,
                "capability":"applications","context":"mock","namespace":"team","card":"degraded"}),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("forbidden"), "{error}");
    }

    #[test]
    fn a_read_namespace_follows_the_selection_rule_of_the_resource_lists() {
        let binding = |namespaced: bool| srelens_plugin_host::Binding {
            name: "r".into(),
            title: "R".into(),
            target: "k8s.listCustomResource".into(),
            arguments: json!({"namespaced":namespaced})
                .as_object()
                .unwrap()
                .clone(),
            inputs: vec!["context".into(), "namespace".into()],
        };
        let one = ["team".to_owned()];
        let two = ["team".to_owned(), "prod".to_owned()];
        assert_eq!(
            read_scope(&binding(true), &one),
            ("team".to_owned(), Some(&one[..]))
        );
        assert_eq!(
            read_scope(&binding(true), &two),
            (String::new(), Some(&two[..]))
        );
        assert_eq!(
            read_scope(&binding(true), &[]),
            (String::new(), Some(&[][..]))
        );
        assert_eq!(read_scope(&binding(false), &one), (String::new(), None));
    }
}
