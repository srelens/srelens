//! Metric, log and trace providers (#569): the broker's half.
//!
//! A provider is a query template an app declares (`srelens_plugin_host`'s
//! `metricProviders`, `logProviders`, `traceProviders`). A view names a
//! resource; the host checks the app may answer for it (every check an app
//! read makes), binds the template's variables, escaped for the language, and
//! sends the query through the provider's `network.http` binding —
//! [`network::request`], so the allowlist, the loopback switch, the redirect
//! rules, the size limit and secrets by reference all hold as for any request.
//! What comes back is read here, into data the host draws itself:
//!
//! | Language | Endpoint | Parameters the host sets | Read as |
//! |---|---|---|---|
//! | PromQL | a Prometheus range query | `query`, `start`, `end`, `step` (seconds) | a timeseries chart (#570): at most [`MAX_SERIES`] series |
//! | LogQL | a Loki range query | `query`, `start`, `end` (nanoseconds), `limit`, `direction` | log lines, oldest first |
//! | TraceQL | a Tempo search | `q`, `start`, `end` (seconds), `limit` | a list of traces, newest first, at most [`MAX_TRACES`] |
//!
//! `extensions.queryProvider` answers one query. A log provider is also a
//! stream source, `logProvider`, which the log view follows
//! (`streams/providers.rs`): its history, then a query every
//! [`ProviderTiming::poll`] for what is new, for as long as the view is open.
use super::network::{self, Limits, RequestError, Scrub};
use super::{resolve_app, AppPolicy, Installed, Store};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use srelens_capability::{Annotations, Capability, CapabilityError, Registry};
use srelens_plugin_host::{
    namespace_name, object_name, MetricUnit, ProviderKind, QueryLanguage, QueryTemplate,
    QueryValues, SecretStore, Variable, PROVIDER_KINDS,
};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The shortest and longest time range a query asks for, and the default.
pub const MIN_RANGE: u64 = 5 * 60;
pub const MAX_RANGE: u64 = 7 * 24 * 3600;
const DEFAULT_RANGE: u64 = 3600;
/// A range query's finest step, and the most points it asks for: well inside
/// the chart's 1,000 samples, and near what a panel's width can show.
const MIN_STEP: u64 = 15;
const MAX_POINTS: u64 = 250;
/// Most series one chart draws (the timeseries component's own limit).
pub const MAX_SERIES: usize = 8;
/// Most traces one search lists.
pub const MAX_TRACES: usize = 50;
/// Most log lines one query answers.
pub const MAX_LOG_LINES: usize = 1000;
/// The fewest lines a log query is asked for when answers are too large.
pub const MIN_LOG_PAGE: usize = 10;
/// Most lines of history a log follow starts with.
pub const MAX_HISTORY: i64 = 5000;
/// Lines of history a log follow starts with when the view names none.
pub const DEFAULT_HISTORY: i64 = 200;
/// The longest name, service or operation the host keeps, in characters.
const MAX_TEXT: usize = 256;
/// The longest error text a query backend's answer is quoted with.
const MAX_QUOTED: usize = 300;

/// The longest context name a query is asked for, as the longest a template binds.
const MAX_CONTEXT: usize = 1024;

/// How often a log follow asks for what is new.
#[derive(Clone, Copy, Debug)]
pub struct ProviderTiming {
    /// Between two queries, while the last one answered less than a page.
    pub poll: Duration,
    /// After a query that answered a whole page: there is more to catch up on.
    pub full_page: Duration,
    /// How far behind now a query stops: log agents push in batches, and Loki takes
    /// a line written with an older timestamp, so the newest instants are still
    /// filling in. Loki's own tail API waits the same way (`delay_for`).
    pub lag: Duration,
}

impl Default for ProviderTiming {
    fn default() -> Self {
        Self {
            poll: Duration::from_secs(5),
            full_page: Duration::from_secs(1),
            lag: Duration::from_secs(2),
        }
    }
}

/// `extensions.queryProvider`: one query of a provider, for the resource a view
/// shows. Multi-word fields are the wrapper's camelCase.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct QueryIn {
    pub id: String,
    pub revision: u64,
    /// The provider's `id` in the app's manifest.
    pub provider: String,
    pub context: String,
    pub namespace: String,
    /// The qualified kind of the resource the view shows, e.g. `apps/Deployment`.
    #[serde(rename = "resourceKind")]
    pub resource_kind: String,
    /// Its name.
    pub name: String,
    /// How far back from now: 300–604800 seconds, default 3600.
    #[serde(default, rename = "rangeSeconds")]
    pub range_seconds: Option<u64>,
}

/// What a provider answered, read by the host.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(super) enum QueryOut {
    /// A metric provider's range query, as the chart it draws (#570).
    Metrics { chart: Chart },
    /// A log provider's lines, oldest first. `truncated` when there were more.
    Logs {
        lines: Vec<LogLine>,
        truncated: bool,
    },
    /// A trace provider's traces, newest first. `truncated` when there were more.
    Traces { traces: Vec<Trace>, truncated: bool },
}

/// The timeseries component's data (`schemas/native-component.v1.json`).
#[derive(Debug, Serialize, JsonSchema)]
pub(super) struct Chart {
    label: String,
    unit: MetricUnit,
    range: ChartRange,
    /// Epoch milliseconds, one per step of the range.
    times: Vec<i64>,
    series: Vec<Series>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct ChartRange {
    start: i64,
    end: i64,
}

#[derive(Debug, Serialize, JsonSchema)]
struct Series {
    name: String,
    /// One per time; `null` where the query had no sample, or one that is not a
    /// finite number.
    values: Vec<Option<f64>>,
}

/// One log line, as a query answered it.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub(super) struct LogLine {
    /// When it was logged, RFC 3339 with nanoseconds.
    time: String,
    /// `pod/container` when its stream says both, else the pod, else the provider.
    source: String,
    line: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    truncated: bool,
}

/// One trace a search found.
#[derive(Debug, PartialEq, Serialize, JsonSchema)]
pub(super) struct Trace {
    #[serde(rename = "traceId")]
    trace_id: String,
    #[serde(rename = "rootService", skip_serializing_if = "Option::is_none")]
    root_service: Option<String>,
    #[serde(rename = "rootName", skip_serializing_if = "Option::is_none")]
    root_name: Option<String>,
    /// When it started, in epoch milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    start: Option<i64>,
    #[serde(rename = "durationMs", skip_serializing_if = "Option::is_none")]
    duration_ms: Option<u64>,
}

/// The resource a view shows, which the provider answers for.
#[derive(Debug, Clone)]
pub(super) struct Subject {
    pub namespace: String,
    pub resource_kind: String,
    pub name: String,
}

/// Who asks: which app at which revision, on which cluster, for which provider.
#[derive(Clone)]
pub(super) struct Ask {
    pub path: Store,
    pub core: Arc<Registry>,
    pub cache: Arc<srelens_kube::client_cache::ClientCache>,
    pub secrets: Arc<dyn SecretStore>,
    pub id: String,
    pub revision: u64,
    pub context: String,
    pub provider: String,
    pub subject: Subject,
}

/// A provider bound for one query: the app as installed now, the binding its
/// query goes through, and the query with every variable in place.
pub(super) struct Bound {
    pub plugin: Installed,
    /// The administrator's policy the app was read under (#578), whose network
    /// ceiling holds its requests on the web host.
    pub policy: Option<AppPolicy>,
    pub kind: ProviderKind,
    pub id: String,
    pub title: String,
    pub capability: String,
    pub language: QueryLanguage,
    pub unit: Option<MetricUnit>,
    pub query: String,
}

/// A metric panel's time grid: whole seconds, the end aligned to the step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Grid {
    pub start: i64,
    pub end: i64,
    pub step: i64,
}

impl Grid {
    /// The range ending at the last whole step before `now`, and rounded up to
    /// whole steps, so both ends are on the step: some query frontends snap the
    /// start to it, and the grid is then the one they answer on.
    fn ending(now: i64, range: u64) -> Self {
        let step = MIN_STEP.max(range.div_ceil(MAX_POINTS)) as i64;
        let end = now - now.rem_euclid(step);
        Self {
            start: end - range.div_ceil(step as u64) as i64 * step,
            end,
            step,
        }
    }
}

/// Seconds since the Unix epoch, now.
fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64)
}

/// Nanoseconds since the Unix epoch, now.
pub(super) fn now_nanos() -> i128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos() as i128)
}

fn check_range(range: u64) -> Result<(), String> {
    if (MIN_RANGE..=MAX_RANGE).contains(&range) {
        Ok(())
    } else {
        Err(format!(
            "A provider's time range is {MIN_RANGE}–{MAX_RANGE} seconds, not {range}"
        ))
    }
}

impl Ask {
    /// The provider, bound for this view: the app may answer on this cluster
    /// (installed, enabled, this revision, enabled here, valid against its
    /// grants), the provider is declared, of `expect` when given, and for the
    /// view's kind, and every variable its template uses has a value it can
    /// carry. Read again for every query, so a changed setting or a disabled
    /// app is seen by the next one.
    pub async fn bind(
        &self,
        expect: Option<ProviderKind>,
        grid: Option<Grid>,
    ) -> Result<Bound, String> {
        let subject = &self.subject;
        // Held to what each can be before any is looked up or repeated: a caller,
        // MCP among them, may send anything.
        if !srelens_plugin_host::is_app_id(&self.id) {
            return Err("The app ID is not one: reverse-domain, at most 128 characters".into());
        }
        if self.provider.is_empty()
            || self.provider.len() > 64
            || !self
                .provider
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        {
            return Err("A provider ID is 1–64 letters, digits and -".into());
        }
        if self.context.len() > MAX_CONTEXT {
            return Err(format!(
                "A cluster context's name is at most {MAX_CONTEXT} characters"
            ));
        }
        if !PROVIDER_KINDS.contains(&subject.resource_kind.as_str()) {
            return Err(format!(
                "A provider answers for one of {}",
                PROVIDER_KINDS.join(", ")
            ));
        }
        if !namespace_name(&subject.namespace) {
            return Err("The namespace is not a Kubernetes namespace name".into());
        }
        if !object_name(&subject.name) {
            return Err("The resource's name is not a Kubernetes name".into());
        }
        if self.context.trim().is_empty() {
            return Err("An explicit cluster context is required".into());
        }
        let app = resolve_app(
            self.path.clone(),
            &self.core,
            &self.cache,
            &self.id,
            self.revision,
            self.context.clone(),
        )
        .await
        .map_err(|e| e.to_string())?;
        let plugin = app.state.plugins[app.index].clone();
        let policy = app.state.policy.clone();
        let manifest = &plugin.manifest;
        let provider = manifest
            .provider(&self.provider)
            .ok_or_else(|| format!("App {} declares no provider \"{}\"", self.id, self.provider))?;
        if let Some(expect) = expect.filter(|expect| *expect != provider.kind) {
            return Err(format!(
                "\"{}\" is not a {} provider",
                provider.id,
                match expect {
                    ProviderKind::Metrics => "metric",
                    ProviderKind::Logs => "log",
                    ProviderKind::Traces => "trace",
                }
            ));
        }
        if !provider.for_kinds.contains(&subject.resource_kind) {
            return Err(format!(
                "Provider \"{}\" is not for {}; it is for {}",
                provider.id,
                subject.resource_kind,
                provider.for_kinds.join(", ")
            ));
        }
        let template = QueryTemplate::parse(provider.language, provider.query)?;
        let pod = subject.resource_kind == "/Pod";
        let cluster = match &app.resolved {
            Ok(context) => context.original_name.clone(),
            Err(why) if template.variables().contains(&Variable::Cluster) => {
                return Err(format!(
                    "Could not tell which cluster ${{cluster}} names: {why}"
                ))
            }
            Err(_) => String::new(),
        };
        let values = QueryValues {
            cluster,
            namespace: subject.namespace.clone(),
            workload: (!pod).then(|| subject.name.clone()),
            pod: pod.then(|| subject.name.clone()),
            range_seconds: grid.map(|grid| (grid.end - grid.start) as u64),
            step_seconds: grid.map(|grid| grid.step as u64),
        };
        let query = template.render(&values)?;
        Ok(Bound {
            kind: provider.kind,
            id: provider.id.to_owned(),
            title: provider.title.to_owned(),
            capability: provider.capability.to_owned(),
            language: provider.language,
            unit: provider.unit,
            query,
            plugin,
            policy,
        })
    }

    /// Sends `bound`'s query with the host's `parameters` after it, through its
    /// `network.http` binding, and answers the body, and what any of its words the
    /// host repeats are scrubbed of.
    pub async fn send(
        &self,
        bound: &Bound,
        parameters: Vec<(&'static str, String)>,
    ) -> Result<(Value, Scrub), RequestError> {
        let query_key = match bound.language {
            QueryLanguage::Traceql => "q",
            _ => "query",
        };
        let mut extra = vec![(query_key, bound.query.clone())];
        extra.extend(parameters);
        let (answer, scrub) = network::request(
            &self.core,
            self.secrets.as_ref(),
            &bound.plugin,
            &bound.capability,
            &extra,
            bound.policy.as_ref(),
            Limits::default(),
        )
        .await?;
        Ok((answer.body, scrub))
    }

    /// Log lines from `start` (nanoseconds, inclusive) to `end`, at most `limit`,
    /// the newest when `backward`, the oldest otherwise: oldest first, and whether
    /// the query answered a whole page. An answer too large is asked again for half
    /// as many lines, down to [`MIN_LOG_PAGE`].
    pub async fn logs(
        &self,
        bound: &Bound,
        start: i128,
        end: i128,
        limit: usize,
        backward: bool,
    ) -> Result<(Vec<Entry>, bool), RequestError> {
        let mut limit = limit;
        let (body, scrub) = loop {
            let asked = self
                .send(
                    bound,
                    vec![
                        ("start", start.to_string()),
                        ("end", end.to_string()),
                        ("limit", limit.to_string()),
                        (
                            "direction",
                            if backward { "backward" } else { "forward" }.into(),
                        ),
                    ],
                )
                .await;
            match asked {
                Err(RequestError::TooLarge(_)) if limit > MIN_LOG_PAGE => {
                    limit = (limit / 2).max(MIN_LOG_PAGE);
                }
                asked => break asked?,
            }
        };
        let mut entries = read_streams(&body, &bound.id)
            .map_err(|why| RequestError::Refused(scrubbed(&scrub, &why)))?;
        let full = entries.len() >= limit;
        entries.sort_by_key(|entry| entry.nanos);
        if entries.len() > limit {
            // Loki keeps to its limit; one that does not is held to it here, on
            // the side the query asked for.
            let excess = entries.len() - limit;
            if backward {
                entries.drain(..excess);
            } else {
                entries.truncate(limit);
            }
        }
        Ok((entries, full))
    }
}

/// One log line as a query answered it, before it is framed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Entry {
    pub nanos: i128,
    pub source: String,
    pub line: String,
}

impl Entry {
    /// The line as a stream frame or an answer carries it: prefixed with its
    /// time when the view asks for timestamps, as `kubectl logs --timestamps`
    /// writes it.
    pub fn text(&self, timestamps: bool) -> String {
        if timestamps {
            format!("{} {}", rfc3339(self.nanos), self.line)
        } else {
            self.line.clone()
        }
    }
}

/// `extensions.queryProvider`'s handler.
async fn query(ask: Ask, range: Option<u64>) -> Result<QueryOut, String> {
    let range = range.unwrap_or(DEFAULT_RANGE);
    check_range(range)?;
    let now = now_seconds();
    let grid = Grid::ending(now, range);
    // Only a metric template binds `${range}` and `${step}`; binding the grid for
    // every provider is harmless and keeps one path.
    let bound = ask.bind(None, Some(grid)).await?;
    let failed = |error: RequestError| error.message().to_owned();
    match bound.kind {
        ProviderKind::Metrics => {
            let (body, scrub) = ask
                .send(
                    &bound,
                    vec![
                        ("start", grid.start.to_string()),
                        ("end", grid.end.to_string()),
                        ("step", grid.step.to_string()),
                    ],
                )
                .await
                .map_err(failed)?;
            Ok(QueryOut::Metrics {
                chart: read_matrix(&body, &bound, grid).map_err(|why| scrubbed(&scrub, &why))?,
            })
        }
        ProviderKind::Logs => {
            let end = i128::from(now) * 1_000_000_000;
            let start = end - i128::from(range) * 1_000_000_000;
            let (entries, full) = ask
                .logs(&bound, start, end, MAX_LOG_LINES, true)
                .await
                .map_err(failed)?;
            let lines = entries
                .iter()
                .map(|entry| {
                    let (line, truncated) = cut_line(entry.line.clone());
                    LogLine {
                        time: rfc3339(entry.nanos),
                        source: entry.source.clone(),
                        line,
                        truncated,
                    }
                })
                .collect();
            Ok(QueryOut::Logs {
                lines,
                truncated: full,
            })
        }
        ProviderKind::Traces => {
            let (body, scrub) = ask
                .send(
                    &bound,
                    vec![
                        ("start", (now - range as i64).to_string()),
                        ("end", now.to_string()),
                        // One more than is listed, so a search with more says so.
                        ("limit", (MAX_TRACES + 1).to_string()),
                    ],
                )
                .await
                .map_err(failed)?;
            let (traces, truncated) = read_traces(&body).map_err(|why| scrubbed(&scrub, &why))?;
            Ok(QueryOut::Traces { traces, truncated })
        }
    }
}

/// Why an answer could not be read, scrubbed of what the request carried: the
/// reasons quote the backend, which may echo a credential or the URL it was sent
/// with a 2xx as it may with a refusal.
fn scrubbed(scrub: &Scrub, why: &str) -> String {
    scrub.text(why).unwrap_or_else(|| {
        "The server's answer could not be read, and its reason is not repeated: it may hold the credential the request carried".into()
    })
}

/// A query backend's own words for why it refused, cut and with invisible
/// characters shown.
fn quoted(text: &str) -> String {
    let cut: String = text.chars().take(MAX_QUOTED).collect();
    srelens_capability::escape_invisible(&cut)
}

/// `text` cut to [`MAX_TEXT`] characters.
fn short(text: &str) -> String {
    text.chars().take(MAX_TEXT).collect()
}

/// The `data` of a successful Prometheus or Loki answer, of `result_type`.
fn success_data<'a>(
    body: &'a Value,
    backend: &str,
    result_type: &str,
    what: &str,
) -> Result<&'a Map<String, Value>, String> {
    let Some(object) = body.as_object() else {
        return Err(format!(
            "The server's answer is not a {backend} query result"
        ));
    };
    match object.get("status").and_then(Value::as_str) {
        Some("success") => {}
        Some("error") => {
            let why = object
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("no reason given");
            return Err(format!("{backend} refused the query: {}", quoted(why)));
        }
        _ => {
            return Err(format!(
                "The server's answer is not a {backend} query result"
            ))
        }
    }
    let data = object
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("The server's answer is not a {backend} query result"))?;
    match data.get("resultType").and_then(Value::as_str) {
        Some(found) if found == result_type => Ok(data),
        Some(found) => Err(format!("{what}; this query answered a {}", quoted(found))),
        None => Err(format!(
            "The server's answer is not a {backend} query result"
        )),
    }
}

/// A Prometheus range query's matrix, as a chart on `grid`.
fn read_matrix(body: &Value, bound: &Bound, grid: Grid) -> Result<Chart, String> {
    let data = success_data(
        body,
        "Prometheus",
        "matrix",
        "A metric provider's query answers a range of samples (a matrix)",
    )?;
    let result = data
        .get("result")
        .and_then(Value::as_array)
        .ok_or("The server's answer is not a Prometheus query result")?;
    if result.len() > MAX_SERIES {
        return Err(format!(
            "The query returned {} series; a chart draws at most {MAX_SERIES}. Aggregate them in the query, for example with sum by (…)",
            result.len()
        ));
    }
    let times: Vec<i64> = (0..)
        .map(|step| grid.start + step * grid.step)
        .take_while(|time| *time <= grid.end)
        .map(|time| time * 1000)
        .collect();
    // A sample lands on the step nearest it, within half a step: a frontend that
    // evaluates a little off the asked start still answers every step.
    let step_ms = grid.step * 1000;
    let place = |at: i64| {
        let offset = at - grid.start * 1000;
        let position = (offset + step_ms / 2).div_euclid(step_ms);
        usize::try_from(position)
            .ok()
            .filter(|position| *position < times.len())
            .filter(|position| (at - times[*position]).abs() <= step_ms / 2)
    };
    let mut names: Vec<String> = Vec::new();
    let mut series = Vec::new();
    for entry in result {
        let labels: BTreeMap<&str, &str> = entry
            .get("metric")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .filter_map(|(key, value)| Some((key.as_str(), value.as_str()?)))
            .collect();
        let mut name = labels
            .iter()
            .filter(|(key, _)| **key != "__name__")
            .map(|(key, value)| format!("{key}=\"{value}\""))
            .collect::<Vec<_>>()
            .join(", ");
        if name.is_empty() {
            name = labels
                .get("__name__")
                .map_or_else(|| bound.title.clone(), |metric| (*metric).to_owned());
        }
        // A label value is the backend's text: shown with invisible characters as escapes,
        // so a name cannot display as another series'.
        let mut name = short(&srelens_capability::escape_invisible(&name));
        // One name per series: two label sets cut to the same text are told apart.
        let base = name.clone();
        let mut ordinal = 2;
        while names.contains(&name) {
            name = format!(
                "{} ({ordinal})",
                base.chars().take(MAX_TEXT - 8).collect::<String>()
            );
            ordinal += 1;
        }
        names.push(name.clone());
        let mut values = vec![None; times.len()];
        for sample in entry
            .get("values")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let (Some(time), Some(value)) = (
                sample.get(0).and_then(Value::as_f64),
                sample.get(1).and_then(Value::as_str),
            ) else {
                return Err(
                    "The server's answer holds a sample that is not [time, \"value\"]".into(),
                );
            };
            if let Some(position) = place((time * 1000.0).round() as i64) {
                values[position] = value.parse::<f64>().ok().filter(|value| value.is_finite());
            }
        }
        series.push(Series { name, values });
    }
    // A query that matched nothing is a chart with no data, which says so.
    if series.is_empty() {
        series.push(Series {
            name: bound.title.clone(),
            values: vec![None; times.len()],
        });
    }
    Ok(Chart {
        label: bound.title.clone(),
        unit: bound.unit.unwrap_or(MetricUnit::Number),
        range: ChartRange {
            start: grid.start * 1000,
            end: grid.end * 1000,
        },
        times,
        series,
    })
}

/// A Loki range query's streams, as entries in the order the answer gave them.
fn read_streams(body: &Value, provider: &str) -> Result<Vec<Entry>, String> {
    let data = success_data(
        body,
        "Loki",
        "streams",
        "A log provider's query answers log lines (streams), not a metric query",
    )?;
    let result = data
        .get("result")
        .and_then(Value::as_array)
        .ok_or("The server's answer is not a Loki query result")?;
    let mut entries = Vec::new();
    for stream in result {
        let labels = stream.get("stream").and_then(Value::as_object);
        let label = |key: &str| {
            labels
                .and_then(|labels| labels.get(key))
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
        };
        let source = match (label("pod"), label("container")) {
            (Some(pod), Some(container)) => format!("{pod}/{container}"),
            (Some(pod), None) => pod.to_owned(),
            _ => provider.to_owned(),
        };
        let source = short(&source);
        for value in stream
            .get("values")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let (Some(nanos), Some(line)) = (
                value
                    .get(0)
                    .and_then(Value::as_str)
                    .and_then(|nanos| nanos.parse::<i128>().ok()),
                value.get(1).and_then(Value::as_str),
            ) else {
                return Err(
                    "The server's answer holds a log entry that is not [\"nanoseconds\", \"line\"]"
                        .into(),
                );
            };
            entries.push(Entry {
                nanos,
                source: source.clone(),
                line: line.to_owned(),
            });
        }
    }
    Ok(entries)
}

/// A Tempo search's traces, newest first, and whether there were more than
/// [`MAX_TRACES`].
fn read_traces(body: &Value) -> Result<(Vec<Trace>, bool), String> {
    const NOT_A_SEARCH: &str = "The server's answer is not a Tempo search result";
    if let Some(why) = body.get("error").and_then(Value::as_str) {
        return Err(format!("Tempo refused the search: {}", quoted(why)));
    }
    let traces = match body.get("traces") {
        Some(Value::Array(traces)) => traces,
        // An empty list may be left out of a search's answer, which still carries
        // its `metrics`; any other object without `traces` is not an answer, and is
        // never read as an empty one.
        None if body.get("metrics").is_some_and(Value::is_object) => {
            return Ok((Vec::new(), false))
        }
        _ => return Err(NOT_A_SEARCH.into()),
    };
    let text = |trace: &Value, key: &str| {
        trace
            .get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(short)
    };
    let mut found = Vec::new();
    for trace in traces {
        let Some(trace_id) = text(trace, "traceID") else {
            return Err(NOT_A_SEARCH.into());
        };
        found.push(Trace {
            trace_id,
            root_service: text(trace, "rootServiceName"),
            root_name: text(trace, "rootTraceName"),
            start: trace
                .get("startTimeUnixNano")
                .and_then(Value::as_str)
                .and_then(|nanos| nanos.parse::<i128>().ok())
                .map(|nanos| (nanos / 1_000_000) as i64),
            duration_ms: trace.get("durationMs").and_then(Value::as_u64),
        });
    }
    found.sort_by_key(|trace| std::cmp::Reverse(trace.start));
    let truncated = found.len() > MAX_TRACES;
    found.truncate(MAX_TRACES);
    Ok((found, truncated))
}

/// A log line cut to the pod sources' limit, on a character boundary.
fn cut_line(mut line: String) -> (String, bool) {
    let limit = super::streams::MAX_LINE_BYTES;
    if line.len() <= limit {
        return (line, false);
    }
    let mut end = limit;
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    line.truncate(end);
    (line, true)
}

/// Nanoseconds since the Unix epoch as RFC 3339 in UTC, with all nine digits,
/// as Kubernetes writes a log line's timestamp.
pub(super) fn rfc3339(nanos: i128) -> String {
    let nanos = nanos.max(0);
    let seconds = (nanos / 1_000_000_000) as i64;
    let fraction = (nanos % 1_000_000_000) as i64;
    let (days, of_day) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{fraction:09}Z",
        of_day / 3600,
        of_day % 3600 / 60,
        of_day % 60
    )
}

/// Registers `extensions.queryProvider`.
pub(super) fn register(
    reg: &mut Registry,
    path: Store,
    core: Arc<Registry>,
    cache: Arc<srelens_kube::client_cache::ClientCache>,
    secrets: Arc<dyn SecretStore>,
) {
    reg.register(Capability::typed::<QueryIn, QueryOut, _, _>(
        "extensions.queryProvider",
        "Run one of an enabled app's metric, log or trace provider queries for a workload or pod, through its network.http binding, and answer the data the host draws",
        Annotations::READ_ONLY,
        move |input: QueryIn| {
            let ask = Ask {
                path: path.clone(),
                core: core.clone(),
                cache: cache.clone(),
                secrets: secrets.clone(),
                id: input.id,
                revision: input.revision,
                context: input.context,
                provider: input.provider,
                subject: Subject {
                    namespace: input.namespace,
                    resource_kind: input.resource_kind,
                    name: input.name,
                },
            };
            async move {
                query(ask, input.range_seconds)
                    .await
                    .map_err(CapabilityError::Handler)
            }
        },
    ));
}

#[cfg(test)]
#[path = "providers_tests.rs"]
mod tests;
