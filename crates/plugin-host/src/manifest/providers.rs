//! Metric, log and trace providers (#569): a query template an app declares,
//! sent through one of its `network.http` bindings, with the view's cluster,
//! namespace, workload or pod and a time range bound in by the host.
//!
//! An app never writes the request a view sends. It declares the template; the
//! host binds each variable, escapes it for the query language, adds the query
//! and its time range as the language's own HTTP parameters, and sends the
//! request through the binding with every rule `network.http` holds it to. What
//! comes back is data the host reads — a range query's series, a log stream's
//! lines, a search's traces — and draws itself. A provider supplies no markup.
//!
//! **Where a variable may stand.** A value the host binds is a name: a cluster,
//! a namespace, a workload, a pod. [`QueryTemplate::parse`] reads the template
//! the way each language reads its strings, and admits one of those variables
//! only inside a double-quoted string, where the host escapes `\` and `"` as it
//! inserts the value: with every backslash doubled, a value can neither end its
//! string nor begin any other escape sequence the languages read (`\n`, `\t`, octal,
//! hex and the rest). It refuses `/* */` and `//` comments outside strings, which LogQL's and
//! TraceQL's lexers skip, and a name in a regex matcher (`=~`, `!~`, `|~`) must be
//! `${name:regex}`. The values are held to what they can be: Kubernetes names, and
//! a context name of letters, digits and `._:/@+-`. One that is not is refused
//! rather than passed on, so a kubeconfig context named `prod" } || { true` never
//! reaches a query, and no value can end a string, open a comment or start a Go
//! template inside one. The escaping stays, so that holds whatever the value. The
//! durations `${range}` and `${step}` are the host's own numbers and stand
//! outside strings, where a duration is written.
use super::{identifier, is_format_character, label, namespace_name, unique, Manifest};
use crate::{ValidationCode as Code, ValidationErrors};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

/// Most providers in each of the three lists.
pub const MAX_PROVIDERS: usize = 16;
/// The longest query template.
pub const MAX_QUERY: usize = 2048;
/// The kinds a provider may be for: the resources a metric panel, a trace list
/// or the log view is drawn for.
pub const PROVIDER_KINDS: &[&str] = &[
    "apps/Deployment",
    "apps/StatefulSet",
    "apps/DaemonSet",
    "/Pod",
];
/// The longest cluster (kubeconfig context) name the host binds.
const MAX_CLUSTER: usize = 1024;

/// A metric provider: a PromQL range query drawn as a chart on the overview of
/// each kind in `forKinds`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MetricProvider {
    pub id: String,
    pub title: String,
    /// A `network.http` binding in `capabilities`, whose `path` is the range
    /// query endpoint, e.g. `/api/v1/query_range`.
    pub capability: String,
    pub language: MetricLanguage,
    #[serde(rename = "forKinds")]
    pub for_kinds: Vec<String>,
    /// The PromQL expression, with `${…}` variables the host binds.
    pub query: String,
    /// How the host formats the values: one unit, so one y axis.
    pub unit: MetricUnit,
}

/// A log provider: a LogQL stream selector the pod log view can follow as a
/// source beside Kubernetes, for each kind in `forKinds`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LogProvider {
    pub id: String,
    pub title: String,
    /// A `network.http` binding whose `path` is the range query endpoint, e.g.
    /// `/loki/api/v1/query_range`.
    pub capability: String,
    pub language: LogLanguage,
    #[serde(rename = "forKinds")]
    pub for_kinds: Vec<String>,
    /// The LogQL log query, with `${…}` variables the host binds.
    pub query: String,
}

/// A trace provider: a TraceQL search listed on the overview of each kind in
/// `forKinds`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TraceProvider {
    pub id: String,
    pub title: String,
    /// A `network.http` binding whose `path` is the search endpoint, e.g.
    /// `/api/search`.
    pub capability: String,
    pub language: TraceLanguage,
    #[serde(rename = "forKinds")]
    pub for_kinds: Vec<String>,
    /// The TraceQL query, with `${…}` variables the host binds.
    pub query: String,
}

/// The one language a metric provider queries in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum MetricLanguage {
    #[serde(rename = "promql")]
    Promql,
}

/// The one language a log provider queries in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum LogLanguage {
    #[serde(rename = "logql")]
    Logql,
}

/// The one language a trace provider queries in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum TraceLanguage {
    #[serde(rename = "traceql")]
    Traceql,
}

/// The units a chart formats, as the host's timeseries component (#570) names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum MetricUnit {
    Number,
    Percent,
    Ratio,
    Bytes,
    BytesPerSecond,
    Seconds,
    Cores,
    PerSecond,
}

/// Which list a provider is in, and so what the host does with its answer.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub enum ProviderKind {
    Metrics,
    Logs,
    Traces,
}

impl ProviderKind {
    /// The manifest list it is declared in.
    pub fn list(self) -> &'static str {
        match self {
            Self::Metrics => "metricProviders",
            Self::Logs => "logProviders",
            Self::Traces => "traceProviders",
        }
    }
}

/// A query language the host knows how to escape for, send and read.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum QueryLanguage {
    Promql,
    Logql,
    Traceql,
}

impl QueryLanguage {
    /// Its name, as a manifest writes it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Promql => "promql",
            Self::Logql => "logql",
            Self::Traceql => "traceql",
        }
    }

    /// How a person reads its name.
    pub fn title(self) -> &'static str {
        match self {
            Self::Promql => "PromQL",
            Self::Logql => "LogQL",
            Self::Traceql => "TraceQL",
        }
    }
}

/// The query parameters the host adds to a provider's request, in the order it
/// adds them: the query and its time range, in the language's HTTP API — a
/// Prometheus range query, a Loki range query, a Tempo search. A binding a
/// provider sends through may not set any of them itself.
pub fn host_parameters(language: QueryLanguage) -> &'static [&'static str] {
    match language {
        QueryLanguage::Promql => &["query", "start", "end", "step"],
        QueryLanguage::Logql => &["query", "start", "end", "limit", "direction"],
        QueryLanguage::Traceql => &["q", "start", "end", "limit"],
    }
}

/// One provider of any list, as the host reads it.
#[derive(Debug, Clone, Copy)]
pub struct ProviderRef<'a> {
    pub kind: ProviderKind,
    /// Its position in its list.
    pub index: usize,
    pub id: &'a str,
    pub title: &'a str,
    pub capability: &'a str,
    pub language: QueryLanguage,
    pub for_kinds: &'a [String],
    pub query: &'a str,
    /// A metric provider's unit; `None` for the others.
    pub unit: Option<MetricUnit>,
}

impl ProviderRef<'_> {
    /// Where it is declared, e.g. `contributions.metricProviders[0]`.
    pub fn path(&self) -> String {
        format!("contributions.{}[{}]", self.kind.list(), self.index)
    }
}

impl Manifest {
    /// Every provider the manifest declares: metrics, then logs, then traces.
    pub fn providers(&self) -> Vec<ProviderRef<'_>> {
        let contributions = &self.contributions;
        let metrics = contributions
            .metric_providers
            .iter()
            .enumerate()
            .map(|(index, p)| ProviderRef {
                kind: ProviderKind::Metrics,
                index,
                id: &p.id,
                title: &p.title,
                capability: &p.capability,
                language: QueryLanguage::Promql,
                for_kinds: &p.for_kinds,
                query: &p.query,
                unit: Some(p.unit),
            });
        let logs = contributions
            .log_providers
            .iter()
            .enumerate()
            .map(|(index, p)| ProviderRef {
                kind: ProviderKind::Logs,
                index,
                id: &p.id,
                title: &p.title,
                capability: &p.capability,
                language: QueryLanguage::Logql,
                for_kinds: &p.for_kinds,
                query: &p.query,
                unit: None,
            });
        let traces = contributions
            .trace_providers
            .iter()
            .enumerate()
            .map(|(index, p)| ProviderRef {
                kind: ProviderKind::Traces,
                index,
                id: &p.id,
                title: &p.title,
                capability: &p.capability,
                language: QueryLanguage::Traceql,
                for_kinds: &p.for_kinds,
                query: &p.query,
                unit: None,
            });
        metrics.chain(logs).chain(traces).collect()
    }

    /// The provider with this id, in whichever list.
    pub fn provider(&self, id: &str) -> Option<ProviderRef<'_>> {
        self.providers()
            .into_iter()
            .find(|provider| provider.id == id)
    }
}

/// A variable the host binds into a query template.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Variable {
    /// The kubeconfig context's name.
    Cluster,
    /// The view's namespace.
    Namespace,
    /// The Deployment, StatefulSet or DaemonSet the view shows.
    Workload,
    /// The Pod the view shows.
    Pod,
    /// The whole time range of a metric panel, as a duration: `3600s`.
    Range,
    /// A metric panel's resolution, as a duration: `15s`.
    Step,
}

impl Variable {
    fn named(name: &str) -> Option<Self> {
        Some(match name {
            "cluster" => Self::Cluster,
            "namespace" => Self::Namespace,
            "workload" => Self::Workload,
            "pod" => Self::Pod,
            "range" => Self::Range,
            "step" => Self::Step,
            _ => return None,
        })
    }

    /// Its name, as a template writes it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Cluster => "cluster",
            Self::Namespace => "namespace",
            Self::Workload => "workload",
            Self::Pod => "pod",
            Self::Range => "range",
            Self::Step => "step",
        }
    }

    /// A name the host binds as text inside a string, rather than a duration.
    fn is_name(self) -> bool {
        !matches!(self, Self::Range | Self::Step)
    }

    /// Whether a view of `kind` knows this variable's value.
    fn known_on(self, kind: &str) -> bool {
        match self {
            Self::Workload => kind != "/Pod",
            Self::Pod => kind == "/Pod",
            _ => true,
        }
    }
}

/// One part of a parsed template.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece {
    Text(String),
    /// A name, inside a double-quoted string: escaped, and regex-quoted first
    /// when the template says `${name:regex}`.
    Name {
        variable: Variable,
        regex: bool,
    },
    /// `${range}` or `${step}`, outside any string.
    Duration(Variable),
}

/// A query template, read the way its language reads strings, so the host
/// knows where each variable stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryTemplate {
    language: QueryLanguage,
    pieces: Vec<Piece>,
}

/// Where in the template the lexer is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum At {
    Outside,
    /// A double-quoted string, where `\` escapes the next character.
    Double,
    /// A PromQL single-quoted string, escaped the same way.
    Single,
    /// A raw string between backticks, with no escapes.
    Raw,
}

/// How far the lexer is into `or` after a regex matcher's string.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Alternative {
    None,
    /// A regex matcher's string just closed.
    AfterRegex,
    /// Then `o`.
    O,
    /// Then `or`: the next string is another pattern of the same matcher.
    Or,
}

impl Alternative {
    fn after(regex_string: bool) -> Self {
        if regex_string {
            Self::AfterRegex
        } else {
            Self::None
        }
    }
}

impl QueryTemplate {
    /// Reads `template` as a `language` query, or says why it cannot stand as one.
    ///
    /// Refused: a template longer than [`MAX_QUERY`] or empty; a control
    /// character; a comment (`#`), which would hide the rest of a line from
    /// the language but not from this reader; an unclosed string; a string
    /// delimiter the language lacks; an unknown variable, or `${settings.…}`;
    /// a name anywhere but a double-quoted string; and a duration anywhere but
    /// outside one.
    pub fn parse(language: QueryLanguage, template: &str) -> Result<Self, String> {
        if template.is_empty() || template.chars().count() > MAX_QUERY {
            return Err(format!("A query is 1–{MAX_QUERY} characters"));
        }
        if template.chars().any(|c| {
            c.is_control() || is_format_character(c) || matches!(c, '\u{2028}' | '\u{2029}')
        }) {
            return Err(
                "A query is one line with no control or invisible format characters".into(),
            );
        }
        let mut pieces = Vec::new();
        let mut text = String::new();
        let mut at = At::Outside;
        // The last two characters outside a string, spaces aside: `=~`, `!~` or `|~`
        // before a string makes it a regex.
        let mut before = [' ', ' '];
        let mut regex_string = false;
        // Where a LogQL line filter's `or` alternative stands: the string after
        // `|~ "a" or` is a pattern of the same filter, so a regex too.
        let mut alternative = Alternative::None;
        let mut chars = template.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '$' && chars.peek() == Some(&'{') {
                chars.next();
                let mut written = String::new();
                loop {
                    match chars.next() {
                        Some('}') => break,
                        Some(c) if c.is_ascii_alphanumeric() || c == ':' || c == '.' => {
                            written.push(c)
                        }
                        _ => {
                            return Err(
                                "`${` starts a variable: a name such as ${namespace}, then `}`"
                                    .into(),
                            )
                        }
                    }
                }
                let piece = variable_piece(&written, at, regex_string)?;
                if !text.is_empty() {
                    pieces.push(Piece::Text(std::mem::take(&mut text)));
                }
                pieces.push(piece);
                continue;
            }
            text.push(c);
            match at {
                At::Outside => match c {
                    '"' | '`' if c == '"' || language != QueryLanguage::Traceql => {
                        at = if c == '"' { At::Double } else { At::Raw };
                        regex_string = matches!(before, ['=' | '!' | '|', '~'])
                            || alternative == Alternative::Or;
                        alternative = Alternative::None;
                    }
                    '\'' if language == QueryLanguage::Promql => at = At::Single,
                    '`' | '\'' => {
                        return Err(format!(
                            "{} strings are written in {}",
                            language.title(),
                            match language {
                                QueryLanguage::Traceql => "double quotes",
                                _ => "double quotes or backticks",
                            }
                        ))
                    }
                    '#' => {
                        return Err(
                            "A query has no comments: `#` outside a string is refused".into()
                        )
                    }
                    // LogQL's and TraceQL's lexers skip these, quotes and all, so a
                    // string this reader sees there would be none to them.
                    '/' if matches!(chars.peek(), Some('*' | '/')) => {
                        return Err(
                            "A query has no comments: `/*` or `//` outside a string is refused"
                                .into(),
                        )
                    }
                    c if !c.is_whitespace() => {
                        before = [before[1], c];
                        alternative = match (alternative, c) {
                            (Alternative::AfterRegex, 'o') => Alternative::O,
                            (Alternative::O, 'r') => Alternative::Or,
                            _ => Alternative::None,
                        };
                    }
                    _ => {
                        if alternative == Alternative::O {
                            alternative = Alternative::None;
                        }
                    }
                },
                At::Double | At::Single => match c {
                    '\\' => match chars.next() {
                        Some(escaped) => text.push(escaped),
                        None => return Err("A string in the query is not closed".into()),
                    },
                    '"' if at == At::Double => {
                        at = At::Outside;
                        before = [before[1], '"'];
                        alternative = Alternative::after(regex_string);
                    }
                    '\'' if at == At::Single => at = At::Outside,
                    _ => {}
                },
                At::Raw => {
                    if c == '`' {
                        at = At::Outside;
                        before = [before[1], '`'];
                        alternative = Alternative::after(regex_string);
                    }
                }
            }
        }
        if at != At::Outside {
            return Err("A string in the query is not closed".into());
        }
        if !text.is_empty() {
            pieces.push(Piece::Text(text));
        }
        Ok(Self { language, pieces })
    }

    /// Every variable the template uses.
    pub fn variables(&self) -> BTreeSet<Variable> {
        self.pieces
            .iter()
            .filter_map(|piece| match piece {
                Piece::Name { variable, .. } | Piece::Duration(variable) => Some(*variable),
                Piece::Text(_) => None,
            })
            .collect()
    }

    /// The query with every variable bound: names escaped for the language's
    /// double-quoted strings, durations in whole seconds. Refused, naming the
    /// variable but never its value, when a value is missing or is not one a
    /// query can carry.
    pub fn render(&self, values: &QueryValues) -> Result<String, String> {
        let mut query = String::new();
        for piece in &self.pieces {
            match piece {
                Piece::Text(text) => query.push_str(text),
                Piece::Name { variable, regex } => {
                    let value = values.name(*variable)?;
                    let value = if *regex {
                        quote_meta(value)
                    } else {
                        value.to_owned()
                    };
                    for c in value.chars() {
                        if matches!(c, '\\' | '"') {
                            query.push('\\');
                        }
                        query.push(c);
                    }
                }
                Piece::Duration(variable) => {
                    let seconds = match variable {
                        Variable::Range => values.range_seconds,
                        _ => values.step_seconds,
                    }
                    .ok_or_else(|| {
                        format!("This view has no time range for ${{{}}}", variable.name())
                    })?;
                    query.push_str(&format!("{seconds}s"));
                }
            }
        }
        Ok(query)
    }
}

/// The piece `${written}` makes at `at`, or why it cannot stand there. A name in a
/// regex matcher's string is written `${name:regex}`, so its value matches only
/// itself, and only there.
fn variable_piece(written: &str, at: At, regex_string: bool) -> Result<Piece, String> {
    if written.starts_with("settings.") {
        return Err(format!(
            "A setting never reaches a query: ${{{written}}} is refused; a provider binds only the view's cluster, namespace, workload, pod and time range"
        ));
    }
    let (name, modifier) = match written.split_once(':') {
        Some((name, modifier)) => (name, Some(modifier)),
        None => (written, None),
    };
    let variable = Variable::named(name).ok_or_else(|| {
        format!(
            "${{{written}}} is not a variable: use ${{cluster}}, ${{namespace}}, ${{workload}}, ${{pod}}, ${{range}} or ${{step}}"
        )
    })?;
    let regex = match modifier {
        None => false,
        Some("regex") if variable.is_name() => true,
        Some(other) => {
            return Err(format!(
                "${{{written}}}: :{other} is not a format; a name may be written ${{{name}:regex}}"
            ))
        }
    };
    match (variable.is_name(), at) {
        (true, At::Double) if regex_string && !regex => Err(format!(
            "${{{written}}} stands in a regex matcher: write ${{{name}:regex}}, so the value matches only itself"
        )),
        (true, At::Double) if !regex_string && regex => Err(format!(
            "${{{written}}} quotes a value for a regex matcher (=~, !~ or |~); this string is matched as written, so write ${{{name}}}"
        )),
        (true, At::Double) => Ok(Piece::Name { variable, regex }),
        (true, _) => Err(format!(
            "${{{written}}} stands only inside a double-quoted string, where the host escapes it, e.g. namespace=\"${{{written}}}\""
        )),
        (false, At::Outside) => Ok(Piece::Duration(variable)),
        (false, _) => Err(format!(
            "${{{written}}} is a duration: write it outside a string, e.g. [${{{written}}}]"
        )),
    }
}

/// `value` with every RE2 metacharacter escaped, as Go's `regexp.QuoteMeta`
/// does, so it matches only itself.
fn quote_meta(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len());
    for c in value.chars() {
        if "\\.+*?()|[]{}^$".contains(c) {
            quoted.push('\\');
        }
        quoted.push(c);
    }
    quoted
}

/// The values a view binds into a template.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QueryValues {
    /// The kubeconfig context's name.
    pub cluster: String,
    pub namespace: String,
    /// The Deployment, StatefulSet or DaemonSet the view shows, if it shows one.
    pub workload: Option<String>,
    /// The Pod the view shows, if it shows one.
    pub pod: Option<String>,
    pub range_seconds: Option<u64>,
    pub step_seconds: Option<u64>,
}

impl QueryValues {
    /// The value of a name variable, held to what that name can be.
    fn name(&self, variable: Variable) -> Result<&str, String> {
        let missing = || {
            format!(
                "This view names no {} for ${{{}}}",
                variable.name(),
                variable.name()
            )
        };
        let (value, ok, what) = match variable {
            // A context name is free text. The characters real ones carry, and no
            // quote, backslash, brace, star or space, so it cannot end a string, open
            // a comment or a Go template (LogQL's `line_format`), whatever surrounds it.
            Variable::Cluster => (
                self.cluster.as_str(),
                !self.cluster.is_empty()
                    && self.cluster.len() <= MAX_CLUSTER
                    && self
                        .cluster
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "._:/@+-".contains(c)),
                "The cluster's name holds a character a query is not given (letters, digits and . _ : / @ + - only), or is empty or longer than 1024 characters",
            ),
            Variable::Namespace => (
                self.namespace.as_str(),
                namespace_name(&self.namespace),
                "The namespace is not a Kubernetes namespace name",
            ),
            Variable::Workload => {
                let value = self.workload.as_deref().ok_or_else(missing)?;
                (value, object_name(value), "The workload is not a Kubernetes name")
            }
            Variable::Pod => {
                let value = self.pod.as_deref().ok_or_else(missing)?;
                (value, object_name(value), "The pod is not a Kubernetes name")
            }
            Variable::Range | Variable::Step => unreachable!("a duration is not a name"),
        };
        if ok {
            Ok(value)
        } else {
            Err(what.into())
        }
    }
}

/// A Kubernetes object name: a DNS subdomain of at most 253 characters,
/// lowercase letters, digits, `-` and `.`, starting and ending with a letter or
/// digit.
pub fn object_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 253
        && value.split('.').all(|label| {
            !label.is_empty()
                && label
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
                && !label.starts_with('-')
                && !label.ends_with('-')
        })
}

/// The variables a provider in `kind`'s list may use.
fn allowed(kind: ProviderKind, variable: Variable) -> bool {
    kind == ProviderKind::Metrics || variable.is_name()
}

/// The rules for `metricProviders`, `logProviders` and `traceProviders`.
pub(super) fn provider_problems(manifest: &Manifest, problems: &mut ValidationErrors) {
    const LABEL: &str =
        "Must be 1–120 characters with no control characters and no bidirectional or invisible format characters";
    const IDENTIFIER: &str = "Must be 1–64 letters, digits and -";
    let providers = manifest.providers();
    let mut crowded = BTreeSet::new();
    for kind in [
        ProviderKind::Metrics,
        ProviderKind::Logs,
        ProviderKind::Traces,
    ] {
        if providers.iter().filter(|p| p.kind == kind).count() > MAX_PROVIDERS {
            crowded.insert(kind);
            problems.push(
                Code::InvalidValue,
                format!("contributions.{}", kind.list()),
                format!("Declare at most {MAX_PROVIDERS} {}", kind.list()),
            );
        }
    }
    let providers: Vec<_> = providers
        .into_iter()
        .filter(|provider| !crowded.contains(&provider.kind))
        .collect();
    unique(
        problems,
        providers
            .iter()
            .map(|provider| (format!("{}.id", provider.path()), provider.id)),
    );
    for provider in &providers {
        let at = provider.path();
        if !identifier(provider.id) {
            problems.push(Code::InvalidValue, format!("{at}.id"), IDENTIFIER);
        }
        if !label(provider.title) {
            problems.push(Code::InvalidValue, format!("{at}.title"), LABEL);
        }
        match manifest
            .capabilities
            .iter()
            .find(|binding| binding.name == provider.capability)
        {
            None => problems.push(
                Code::UnresolvedCapability,
                format!("{at}.capability"),
                format!("Capability \"{}\" is not declared", provider.capability),
            ),
            Some(binding) if binding.target != super::NETWORK_HTTP => problems.push(
                Code::InvalidBinding,
                format!("{at}.capability"),
                format!(
                    "A provider sends its query through a network.http binding, and \"{}\" is a {} one",
                    binding.name, binding.target
                ),
            ),
            Some(_) => {}
        }
        let kinds_at = format!("{at}.forKinds");
        if provider.for_kinds.is_empty() || provider.for_kinds.len() > PROVIDER_KINDS.len() {
            problems.push(
                Code::InvalidValue,
                kinds_at.clone(),
                format!(
                    "forKinds lists 1–{} of {}",
                    PROVIDER_KINDS.len(),
                    PROVIDER_KINDS.join(", ")
                ),
            );
        }
        for (position, kind) in provider.for_kinds.iter().enumerate() {
            if !PROVIDER_KINDS.contains(&kind.as_str()) {
                problems.push(
                    Code::InvalidKind,
                    format!("{kinds_at}[{position}]"),
                    format!("A provider is for one of {}", PROVIDER_KINDS.join(", ")),
                );
            }
        }
        unique(
            problems,
            provider
                .for_kinds
                .iter()
                .enumerate()
                .map(|(position, kind)| (format!("{kinds_at}[{position}]"), kind.as_str())),
        );
        let query_at = format!("{at}.query");
        let template = match QueryTemplate::parse(provider.language, provider.query) {
            Ok(template) => template,
            // `${settings.…}` outside a settable argument is the settings rule's to
            // report, at this same path.
            Err(_) if srelens_capability::settings::mentions_setting(provider.query) => continue,
            Err(why) => {
                problems.push(Code::InvalidBinding, query_at, why);
                continue;
            }
        };
        for variable in template.variables() {
            let name = variable.name();
            if !allowed(provider.kind, variable) {
                problems.push(
                    Code::InvalidBinding,
                    query_at.clone(),
                    format!(
                        "${{{name}}} is a metric panel's time range; a {} sets its own",
                        match provider.kind {
                            ProviderKind::Logs => "log follow",
                            _ => "trace search",
                        }
                    ),
                );
                continue;
            }
            let unknown: Vec<&str> = provider
                .for_kinds
                .iter()
                .map(String::as_str)
                .filter(|kind| PROVIDER_KINDS.contains(kind) && !variable.known_on(kind))
                .collect();
            if !unknown.is_empty() {
                problems.push(
                    Code::InvalidBinding,
                    query_at.clone(),
                    format!(
                        "${{{name}}} is not known on {}, which this provider is for",
                        unknown.join(", ")
                    ),
                );
            }
        }
    }
    // What the host sets on a request a provider sends, the binding may not.
    for (index, binding) in manifest.capabilities.iter().enumerate() {
        let languages: BTreeSet<QueryLanguage> = providers
            .iter()
            .filter(|provider| provider.capability == binding.name)
            .map(|provider| provider.language)
            .collect();
        let Some(Value::Object(query)) = binding.arguments.get("query") else {
            continue;
        };
        for key in query.keys() {
            if let Some(language) = languages
                .iter()
                .find(|language| host_parameters(**language).contains(&key.as_str()))
            {
                problems.push(
                    Code::InvalidBinding,
                    format!("capabilities[{index}].arguments.query.{key}"),
                    format!(
                        "The host sets `{key}` on a {} provider's request",
                        language.title()
                    ),
                );
            }
        }
    }
}
