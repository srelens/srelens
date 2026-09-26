//! Logs, exec and port-forwards for apps (#567): which pods an app's pod
//! binding may reach, and the host capabilities those bindings target.
//!
//! A pod binding's scope is the manifest's (`Manifest::pod_scope`): the pods
//! an object of a kind the app reads selects, or the pods of a namespace its
//! permission grants. This module turns that into a [`Scope`] on one cluster
//! — reading the object and its own label selector with the user's
//! credentials — and holds every pod a view names to it, on every open. The
//! host never asks the cluster which pods a selector matches and trusts the
//! answer: it reads the pod and matches its labels itself.
//!
//! Cluster access goes through [`PodCluster`], so the tests can script a
//! cluster; the host's is [`KubePods`].

use super::crd;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use srelens_capability::{Annotations, Capability, CapabilityError, Impact, Registry};
use srelens_kube::app_pods::{Output, PodFacts, ServiceFacts, Upstream};
use srelens_kube::client_cache::ClientCache;
use srelens_plugin_host::{Binding, Manifest, PodScope, POD_EXEC, POD_FORWARD, POD_LOGS};
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;

/// A kind the host reads one object of, as a dynamic resource.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KindRef {
    pub group: String,
    pub version: String,
    pub kind: String,
    pub plural: String,
}

/// What one follow of a container's logs is asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogsAsk {
    pub context: String,
    pub namespace: String,
    pub pod: String,
    pub container: String,
    /// Lines of history to start with; 0 on a reconnect, so nothing is repeated.
    pub tail_lines: i64,
    pub since_seconds: Option<i64>,
    pub timestamps: bool,
}

/// What a follow reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LogEvent {
    /// The cluster opened the log stream.
    Connected,
    Line(String),
}

/// One run of an exec binding's command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecAsk {
    pub context: String,
    pub namespace: String,
    pub pod: String,
    pub container: String,
    pub command: Vec<String>,
}

/// The cluster, as the pod sources need it. Every call is made with the
/// user's own credentials and is held to the cluster's RBAC.
#[async_trait::async_trait]
pub trait PodCluster: Send + Sync {
    /// One namespaced object, as the API serves it; `None` when there is none.
    async fn object(
        &self,
        context: &str,
        kind: &KindRef,
        namespace: &str,
        name: &str,
    ) -> Result<Option<Value>, String>;
    /// The pods in `namespace` a label selector query selects, and whether the
    /// list stopped at the host's cap. The host matches every one again.
    async fn pods(
        &self,
        context: &str,
        namespace: &str,
        labels: &str,
    ) -> Result<(Vec<PodFacts>, bool), String>;
    /// One pod; `None` when there is none.
    async fn pod(
        &self,
        context: &str,
        namespace: &str,
        name: &str,
    ) -> Result<Option<PodFacts>, String>;
    async fn services(&self, context: &str, namespace: &str) -> Result<Vec<ServiceFacts>, String>;
    /// Follow one container's logs until the cluster ends the stream.
    async fn logs(&self, ask: &LogsAsk, events: UnboundedSender<LogEvent>) -> Result<(), String>;
    /// Run a command once; its exit code, or why it did not run to one.
    async fn exec(
        &self,
        ask: &ExecAsk,
        output: UnboundedSender<(Output, String)>,
    ) -> Result<i32, String>;
    /// Open one port-forward connection to `port` of `pod`.
    async fn connect(
        &self,
        context: &str,
        namespace: &str,
        pod: &str,
        port: u16,
    ) -> Result<Box<dyn Upstream>, String>;
}

/// The host's [`PodCluster`], over the shared client cache.
pub struct KubePods(pub Arc<ClientCache>);

#[async_trait::async_trait]
impl PodCluster for KubePods {
    async fn object(
        &self,
        context: &str,
        kind: &KindRef,
        namespace: &str,
        name: &str,
    ) -> Result<Option<Value>, String> {
        srelens_kube::app_pods::get_object(
            &self.0,
            context,
            &kind.group,
            &kind.version,
            &kind.kind,
            &kind.plural,
            namespace,
            name,
        )
        .await
    }
    async fn pods(
        &self,
        context: &str,
        namespace: &str,
        labels: &str,
    ) -> Result<(Vec<PodFacts>, bool), String> {
        srelens_kube::app_pods::list_pods(&self.0, context, namespace, labels).await
    }
    async fn pod(
        &self,
        context: &str,
        namespace: &str,
        name: &str,
    ) -> Result<Option<PodFacts>, String> {
        srelens_kube::app_pods::get_pod(&self.0, context, namespace, name).await
    }
    async fn services(&self, context: &str, namespace: &str) -> Result<Vec<ServiceFacts>, String> {
        srelens_kube::app_pods::list_services(&self.0, context, namespace).await
    }
    async fn logs(&self, ask: &LogsAsk, events: UnboundedSender<LogEvent>) -> Result<(), String> {
        let lines = events.clone();
        srelens_kube::logs::stream_pod_logs(
            self.0.clone(),
            ask.context.clone(),
            ask.namespace.clone(),
            ask.pod.clone(),
            Some(ask.container.clone()),
            srelens_kube::logs::StreamOpts {
                tail_lines: ask.tail_lines,
                since_seconds: ask.since_seconds,
                timestamps: ask.timestamps,
            },
            move |line| {
                let _ = lines.send(LogEvent::Line(line));
            },
            move || {
                let _ = events.send(LogEvent::Connected);
            },
        )
        .await
    }
    async fn exec(
        &self,
        ask: &ExecAsk,
        output: UnboundedSender<(Output, String)>,
    ) -> Result<i32, String> {
        srelens_kube::app_pods::exec_once(
            &self.0,
            &ask.context,
            &ask.namespace,
            &ask.pod,
            &ask.container,
            ask.command.clone(),
            move |stream, text| {
                let _ = output.send((stream, text));
            },
        )
        .await
    }
    async fn connect(
        &self,
        context: &str,
        namespace: &str,
        pod: &str,
        port: u16,
    ) -> Result<Box<dyn Upstream>, String> {
        srelens_kube::app_pods::connect_port(self.0.clone(), context, namespace, pod, port).await
    }
}

// ---- Label selectors ----

/// A label selector, as an object keeps one: `matchLabels` and
/// `matchExpressions`, or a plain map of labels as a Service writes it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selector {
    labels: BTreeMap<String, String>,
    expressions: Vec<Requirement>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Requirement {
    key: String,
    operator: Operator,
    values: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operator {
    In,
    NotIn,
    Exists,
    DoesNotExist,
}

/// A label key: an optional DNS-subdomain prefix and `/`, then a name of 1–63
/// letters, digits, `-`, `_` and `.`, starting and ending alphanumeric.
fn label_key(key: &str) -> bool {
    let (prefix, name) = match key.split_once('/') {
        Some((prefix, name)) => (Some(prefix), name),
        None => (None, key),
    };
    prefix.is_none_or(|prefix| {
        !prefix.is_empty()
            && prefix.len() <= 253
            && prefix.split('.').all(|label| {
                !label.is_empty()
                    && label
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            })
    }) && !name.is_empty()
        && label_value(name)
}

/// A label value: empty, or 1–63 letters, digits, `-`, `_` and `.`, starting
/// and ending alphanumeric.
fn label_value(value: &str) -> bool {
    value.is_empty()
        || (value.len() <= 63
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
            && value
                .bytes()
                .next()
                .is_some_and(|b| b.is_ascii_alphanumeric())
            && value
                .bytes()
                .last()
                .is_some_and(|b| b.is_ascii_alphanumeric()))
}

impl Selector {
    /// The selector an object keeps at a path, or why it is not one this host
    /// reads. Every key and value is held to Kubernetes' own label syntax, so
    /// nothing in it can change the meaning of the query the host sends.
    pub fn parse(value: &Value) -> Result<Self, String> {
        let Some(fields) = value.as_object() else {
            return Err("is not a label selector".into());
        };
        let text_map = |value: &Value| -> Result<BTreeMap<String, String>, String> {
            let Some(map) = value.as_object() else {
                return Err("matchLabels is not a map of labels".into());
            };
            map.iter()
                .map(|(key, value)| match value.as_str() {
                    Some(text) if label_key(key) && label_value(text) => {
                        Ok((key.clone(), text.to_owned()))
                    }
                    _ => Err(format!("the label {key} is not one Kubernetes accepts")),
                })
                .collect()
        };
        let structured =
            fields.contains_key("matchLabels") || fields.contains_key("matchExpressions");
        if !structured {
            // A plain map, as a Service or a Deployment's pod template writes one.
            return Ok(Self {
                labels: text_map(value)?,
                expressions: Vec::new(),
            });
        }
        if let Some(extra) = fields
            .keys()
            .find(|key| *key != "matchLabels" && *key != "matchExpressions")
        {
            return Err(format!("has a field {extra} a label selector does not"));
        }
        let labels = match fields.get("matchLabels") {
            None | Some(Value::Null) => BTreeMap::new(),
            Some(value) => text_map(value)?,
        };
        let mut expressions = Vec::new();
        for expression in fields
            .get("matchExpressions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let key = expression["key"].as_str().filter(|key| label_key(key));
            let operator = match expression["operator"].as_str() {
                Some("In") => Some(Operator::In),
                Some("NotIn") => Some(Operator::NotIn),
                Some("Exists") => Some(Operator::Exists),
                Some("DoesNotExist") => Some(Operator::DoesNotExist),
                _ => None,
            };
            let values: Option<Vec<String>> = match &expression["values"] {
                Value::Null => Some(Vec::new()),
                Value::Array(values) => values
                    .iter()
                    .map(|v| v.as_str().filter(|v| label_value(v)).map(str::to_owned))
                    .collect(),
                _ => None,
            };
            let (Some(key), Some(operator), Some(values)) = (key, operator, values) else {
                return Err("has a matchExpressions entry Kubernetes does not accept".into());
            };
            let set = matches!(operator, Operator::In | Operator::NotIn);
            if set == values.is_empty() {
                return Err(format!(
                    "has a matchExpressions entry for {key} whose values do not fit its operator"
                ));
            }
            expressions.push(Requirement {
                key: key.to_owned(),
                operator,
                values,
            });
        }
        Ok(Self {
            labels,
            expressions,
        })
    }

    /// Selects every pod: no labels and no expressions.
    pub fn is_empty(&self) -> bool {
        self.labels.is_empty() && self.expressions.is_empty()
    }

    /// Whether a pod with `labels` is selected.
    pub fn matches(&self, labels: &BTreeMap<String, String>) -> bool {
        self.labels
            .iter()
            .all(|(key, value)| labels.get(key) == Some(value))
            && self.expressions.iter().all(|r| {
                let found = labels.get(&r.key);
                match r.operator {
                    Operator::In => found.is_some_and(|v| r.values.contains(v)),
                    Operator::NotIn => found.is_none_or(|v| !r.values.contains(v)),
                    Operator::Exists => found.is_some(),
                    Operator::DoesNotExist => found.is_none(),
                }
            })
    }

    /// The selector as a label selector query, for a list the host then
    /// matches again itself.
    pub fn query(&self) -> String {
        let mut parts: Vec<String> = self
            .labels
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect();
        for r in &self.expressions {
            parts.push(match r.operator {
                Operator::In => format!("{} in ({})", r.key, r.values.join(",")),
                Operator::NotIn => format!("{} notin ({})", r.key, r.values.join(",")),
                Operator::Exists => r.key.clone(),
                Operator::DoesNotExist => format!("!{}", r.key),
            });
        }
        parts.join(",")
    }
}

// ---- Scopes ----

/// The pods one pod binding may reach on one cluster, now.
#[derive(Clone, Debug)]
pub struct Scope {
    /// The only namespace its pods are in.
    pub namespace: String,
    pub by: By,
}

#[derive(Clone, Debug)]
pub enum By {
    /// Selected by one object's own selector.
    Selector {
        selector: Selector,
        /// `Deployment web`, for a person reading why a pod was refused.
        object: String,
    },
    /// Any pod in a namespace the permission grants.
    Namespace,
}

impl Scope {
    /// Whether `pod` is in this scope: in its namespace and, for an object's
    /// scope, selected by that object's selector.
    pub fn admits(&self, pod: &PodFacts) -> bool {
        pod.namespace == self.namespace
            && match &self.by {
                By::Selector { selector, .. } => selector.matches(&pod.labels),
                By::Namespace => true,
            }
    }

    /// A label selector query for listing the pods in scope.
    pub fn query(&self) -> String {
        match &self.by {
            By::Selector { selector, .. } => selector.query(),
            By::Namespace => String::new(),
        }
    }

    /// Why `pod` is refused, in the words a person reads.
    pub fn refusal(&self, app: &str, pod: &str) -> String {
        match &self.by {
            By::Selector { object, .. } => format!(
                "Pod {pod} is not selected by {object} in {}, so app {app} may not reach it",
                self.namespace
            ),
            By::Namespace => format!(
                "Pod {pod} is not in namespace {}, so app {app} may not reach it",
                self.namespace
            ),
        }
    }
}

/// The kind a scope's reader lists, at the version a read of it resolves to
/// on this cluster. A built-in reader's kind is the host's, never the app's.
async fn reader_kind(core: &Registry, context: &str, reader: &Binding) -> Result<KindRef, String> {
    if let Some(identity) = srelens_plugin_host::builtin_reader_identity(&reader.target) {
        let field = |key: &str| identity[key].as_str().unwrap_or_default().to_owned();
        return Ok(KindRef {
            group: field("group"),
            version: field("version"),
            kind: field("kind"),
            plural: field("plural"),
        });
    }
    let version = crd::resolve(core, context, reader)
        .await
        .map_err(|e| e.to_string())?;
    let field = |key: &str| {
        reader
            .arguments
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    Ok(KindRef {
        group: field("group"),
        version,
        kind: field("kind"),
        plural: field("plural"),
    })
}

/// What `path` (plain dot-separated keys) reaches in `object`.
fn at_path<'a>(object: &'a Value, path: &str) -> Option<&'a Value> {
    path.strip_prefix('.')?
        .split('.')
        .try_fold(object, |node, key| node.get(key))
        .filter(|value| !value.is_null())
}

/// The scope of `binding` on this cluster, for a view in `namespace` naming
/// the object `name` — or why there is none. The object is read now, and its
/// selector with it, so a changed selector changes the scope.
pub async fn scope(
    cluster: &dyn PodCluster,
    core: &Registry,
    context: &str,
    manifest: &Manifest,
    binding: &Binding,
    namespace: &str,
    name: Option<&str>,
) -> Result<Scope, String> {
    if !super::cards::namespace_name(namespace) {
        return Err("Name the namespace the pods are in: a Kubernetes namespace name".into());
    }
    let name = name.filter(|name| !name.is_empty());
    match manifest.pod_scope(binding)? {
        PodScope::Namespaces(granted) => {
            if name.is_some() {
                return Err(format!(
                    "\"{}\" reaches pods by the namespaces its permission grants; it names no object",
                    binding.name
                ));
            }
            if !granted.iter().any(|granted| granted == namespace) {
                return Err(format!(
                    "App {} may reach pods through \"{}\" in {}, not in {namespace}",
                    manifest.id,
                    binding.name,
                    granted.join(", ")
                ));
            }
            Ok(Scope {
                namespace: namespace.to_owned(),
                by: By::Namespace,
            })
        }
        PodScope::Selected { reader, selector } => {
            let kind = reader_kind(core, context, reader).await?;
            let Some(name) = name else {
                return Err(format!(
                    "Name the {} whose pods \"{}\" reaches",
                    kind.kind, binding.name
                ));
            };
            let object = cluster
                .object(context, &kind, namespace, name)
                .await?
                .ok_or_else(|| format!("{} {namespace}/{name} does not exist", kind.kind))?;
            let found = at_path(&object, &selector).ok_or_else(|| {
                format!(
                    "{} {namespace}/{name} has no pod selector at {selector}",
                    kind.kind
                )
            })?;
            let parsed = Selector::parse(found)
                .map_err(|why| format!("The selector of {} {namespace}/{name} {why}", kind.kind))?;
            if parsed.is_empty() {
                return Err(format!(
                    "{} {namespace}/{name} selects every pod in its namespace; the host does not take that as a scope",
                    kind.kind
                ));
            }
            Ok(Scope {
                namespace: namespace.to_owned(),
                by: By::Selector {
                    selector: parsed,
                    object: format!("{} {name}", kind.kind),
                },
            })
        }
    }
}

/// The pod `name`, held to `scope`: it exists and the scope admits it.
pub async fn admitted_pod(
    cluster: &dyn PodCluster,
    context: &str,
    scope: &Scope,
    app: &str,
    name: &str,
) -> Result<PodFacts, String> {
    if !pod_name(name) {
        return Err("Name the pod: a Kubernetes pod name".into());
    }
    let pod = cluster
        .pod(context, &scope.namespace, name)
        .await?
        .ok_or_else(|| format!("Pod {}/{name} does not exist", scope.namespace))?;
    if !scope.admits(&pod) {
        return Err(scope.refusal(app, name));
    }
    Ok(pod)
}

/// A pod name: a DNS subdomain, lowercase, at most 253 characters.
pub fn pod_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 253
        && name.split('.').all(|label| {
            !label.is_empty()
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                && !label.starts_with('-')
                && !label.ends_with('-')
        })
}

/// The container a session runs in: the binding's own, else the one the view
/// asked for, else the pod's only one. It must be one of the pod's.
pub fn container(binding: &Binding, pod: &PodFacts, asked: Option<&str>) -> Result<String, String> {
    let fixed = srelens_plugin_host::pod_container(binding);
    let chosen = match (fixed, asked) {
        (Some(fixed), Some(asked)) if fixed != asked => {
            return Err(format!(
                "\"{}\" runs in container {fixed}, not {asked}",
                binding.name
            ))
        }
        (Some(fixed), _) => fixed.to_owned(),
        (None, Some(asked)) => asked.to_owned(),
        (None, None) => match pod.containers.as_slice() {
            [only] => only.clone(),
            many => {
                return Err(format!(
                    "Pick a container of pod {}: {}",
                    pod.name,
                    many.join(", ")
                ))
            }
        },
    };
    if !pod.containers.contains(&chosen) {
        return Err(format!("Pod {} has no container {chosen}", pod.name));
    }
    Ok(chosen)
}

// ---- The broker's declarations ----

/// The arguments a pod binding may bind. The rules for them are the
/// manifest's (`srelens_plugin_host::pod_problems`); this is what the broker's
/// schema check reads.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct LogsBinding {
    /// A reader binding whose objects select the pods.
    #[serde(default)]
    resource: Option<String>,
    /// Where a custom resource keeps its pod selector.
    #[serde(default)]
    selector: Option<String>,
    #[serde(default)]
    container: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct ExecBinding {
    #[serde(default)]
    resource: Option<String>,
    #[serde(default)]
    selector: Option<String>,
    #[serde(default)]
    container: Option<String>,
    /// The program and its arguments, run without a shell.
    command: Vec<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct ForwardBinding {
    #[serde(default)]
    resource: Option<String>,
    #[serde(default)]
    selector: Option<String>,
    /// The remote port: the pod's, or with `service`, the Service's.
    port: u16,
    #[serde(default)]
    service: bool,
}

#[derive(Serialize, JsonSchema)]
struct Refused {}

/// `k8s.exec`'s host metadata. Sensitive: a command can read anything the
/// container can, and change anything it may; the host cannot know which, so
/// every session is confirmed with its pod, container and exact command.
pub const EXEC_ANNOTATIONS: Annotations = Annotations {
    read_only: false,
    destructive: false,
    requires_confirm: true,
    sensitive: true,
    impact: Impact::High,
    confirm: Some("Run this app's command[ in {resource}][ in cluster {cluster}]?"),
};

/// `k8s.portForward`'s: it changes nothing in the cluster, but opens a port on
/// this computer that any local program may connect to while the view is open.
pub const FORWARD_ANNOTATIONS: Annotations = Annotations {
    impact: Impact::Medium,
    ..Annotations::READ_ONLY
};

fn declaration<In: JsonSchema + for<'de> Deserialize<'de> + Send + 'static>(
    id: &'static str,
    summary: &'static str,
    annotations: Annotations,
) -> Capability {
    Capability::typed::<In, Refused, _, _>(id, summary, annotations, move |_| async move {
        Err(CapabilityError::Handler(format!(
            "{id} runs only as an app stream the extension broker opens for an installed app"
        )))
    })
}

/// The three pod capabilities, as the broker declares them: what a binding is
/// checked against, and the host's metadata for it. Their handlers refuse —
/// only an app stream opened through `extension_stream_open` runs one — and
/// they are never in the catalog or MCP.
pub(crate) fn capabilities() -> Vec<Capability> {
    vec![
        declaration::<LogsBinding>(
            POD_LOGS,
            "Follow the logs of a pod an app's binding may reach; streamed by the extension broker",
            Annotations::READ_ONLY,
        ),
        declaration::<ExecBinding>(
            POD_EXEC,
            "Run an app's declared command in a pod its binding may reach, after the host confirmation; streamed by the extension broker",
            EXEC_ANNOTATIONS,
        ),
        declaration::<ForwardBinding>(
            POD_FORWARD,
            "Forward a local port the host picks to a pod an app's binding may reach; for as long as the view is open",
            FORWARD_ANNOTATIONS,
        ),
    ]
}

// ---- `extensions.pods` ----

/// What a view asks for, to offer the pods a binding may reach.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct PodsIn {
    /// The app's ID.
    pub id: String,
    pub revision: u64,
    /// The pod binding's name.
    pub capability: String,
    pub context: String,
    pub namespace: String,
    /// The object whose pods, for a binding scoped by `resource`.
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct PodOut {
    pub name: String,
    pub namespace: String,
    pub containers: Vec<String>,
    pub phase: Option<String>,
    pub ready: bool,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct ServiceOut {
    pub name: String,
    /// The Service port the binding forwards to.
    pub port: u16,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct PodsOut {
    pub pods: Vec<PodOut>,
    /// For a port-forward through a Service: the Services whose selected pods
    /// include one in scope, with the binding's port.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub services: Option<Vec<ServiceOut>>,
    /// The list stopped at the host's cap; there are more pods than shown.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
    /// What the scope is, in words: `pods selected by Deployment web`.
    pub scope: String,
}

/// The pods (and, for a forward through a Service, the Services) `binding`
/// may reach in `scope`.
pub(super) async fn targets(
    cluster: &dyn PodCluster,
    context: &str,
    binding: &Binding,
    scope: &Scope,
) -> Result<PodsOut, String> {
    let (pods, truncated) = cluster
        .pods(context, &scope.namespace, &scope.query())
        .await?;
    // Matched again here: the cluster's answer to a query is not the scope.
    let pods: Vec<PodFacts> = pods.into_iter().filter(|pod| scope.admits(pod)).collect();
    let services =
        if binding.target == POD_FORWARD && srelens_plugin_host::forward_via_service(binding) {
            let port = srelens_plugin_host::forward_port(binding).unwrap_or_default();
            let services = cluster.services(context, &scope.namespace).await?;
            Some(
                services
                    .into_iter()
                    .filter(|service| service.ports.iter().any(|p| p.port == port))
                    .filter(|service| {
                        let selector = Selector {
                            labels: service.selector.clone(),
                            expressions: Vec::new(),
                        };
                        !selector.is_empty() && pods.iter().any(|pod| selector.matches(&pod.labels))
                    })
                    .map(|service| ServiceOut {
                        name: service.name,
                        port,
                    })
                    .collect(),
            )
        } else {
            None
        };
    let described = match &scope.by {
        By::Selector { object, .. } => format!("pods selected by {object}"),
        By::Namespace => format!("pods in namespace {}", scope.namespace),
    };
    Ok(PodsOut {
        pods: pods
            .into_iter()
            .map(|pod| PodOut {
                name: pod.name,
                namespace: pod.namespace,
                containers: pod.containers,
                phase: pod.phase,
                ready: pod.ready,
            })
            .collect(),
        services,
        truncated,
        scope: described,
    })
}

/// The Service `name` resolved to a pod in `scope` and the port on it, for a
/// forward through a Service: the first running pod it selects that the scope
/// admits. A Service is only a way to name a pod here; it never widens the scope.
pub async fn service_target(
    cluster: &dyn PodCluster,
    context: &str,
    scope: &Scope,
    app: &str,
    name: &str,
    port: u16,
) -> Result<(PodFacts, u16), String> {
    let service = cluster
        .services(context, &scope.namespace)
        .await?
        .into_iter()
        .find(|service| service.name == name)
        .ok_or_else(|| format!("Service {}/{name} does not exist", scope.namespace))?;
    let service_port = service
        .ports
        .iter()
        .find(|p| p.port == port)
        .ok_or_else(|| format!("Service {name} has no port {port}"))?;
    let selector = Selector {
        labels: service.selector.clone(),
        expressions: Vec::new(),
    };
    if selector.is_empty() {
        return Err(format!(
            "Service {name} selects no pods, so there is no pod to forward to"
        ));
    }
    let (pods, _) = cluster
        .pods(context, &scope.namespace, &selector.query())
        .await?;
    let pod = pods
        .into_iter()
        .filter(|pod| selector.matches(&pod.labels) && scope.admits(pod) && pod.running())
        .find(|pod| service_port.on(pod).is_some())
        .ok_or_else(|| format!("Service {name} sends to no running pod app {app} may reach"))?;
    let target = service_port.on(&pod).expect("filtered above");
    Ok((pod, target))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn labels(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn a_label_selector_matches_as_kubernetes_does() {
        let selector = Selector::parse(&json!({
            "matchLabels": {"app": "web"},
            "matchExpressions": [
                {"key": "tier", "operator": "In", "values": ["api", "worker"]},
                {"key": "canary", "operator": "DoesNotExist"},
                {"key": "track", "operator": "NotIn", "values": ["legacy"]},
                {"key": "team", "operator": "Exists"}
            ]
        }))
        .unwrap();
        assert!(selector.matches(&labels(&[("app", "web"), ("tier", "api"), ("team", "a")])));
        assert!(!selector.matches(&labels(&[("app", "web"), ("tier", "db"), ("team", "a")])));
        assert!(!selector.matches(&labels(&[
            ("app", "web"),
            ("tier", "api"),
            ("team", "a"),
            ("canary", "yes")
        ])));
        assert!(!selector.matches(&labels(&[
            ("app", "web"),
            ("tier", "api"),
            ("team", "a"),
            ("track", "legacy")
        ])));
        assert!(!selector.matches(&labels(&[("app", "web"), ("tier", "api")])));
        assert_eq!(
            selector.query(),
            "app=web,tier in (api,worker),!canary,track notin (legacy),team"
        );
        // A Service writes a plain map.
        let plain = Selector::parse(&json!({"app": "web"})).unwrap();
        assert!(plain.matches(&labels(&[("app", "web"), ("x", "y")])));
        assert!(Selector::parse(&json!({})).unwrap().is_empty());
        assert!(Selector::parse(&json!({"matchLabels": {}}))
            .unwrap()
            .is_empty());
    }

    /// Nothing the object says can change what the host's query means: every
    /// key and value is a label Kubernetes accepts, or the selector is refused.
    #[test]
    fn a_selector_that_is_not_one_kubernetes_accepts_is_refused() {
        for bad in [
            json!("app=web"),
            json!({"app": "web,tier!=x"}),
            json!({"app": 3}),
            json!({"a b": "c"}),
            json!({"matchLabels": {"app": "web"}, "other": 1}),
            json!({"matchExpressions": [{"key": "tier", "operator": "In"}]}),
            json!({"matchExpressions": [{"key": "tier", "operator": "Exists", "values": ["a"]}]}),
            json!({"matchExpressions": [{"key": "tier", "operator": "Gt", "values": ["1"]}]}),
            json!({"matchExpressions": [{"key": "tier", "operator": "In", "values": ["a)"]}]}),
        ] {
            assert!(Selector::parse(&bad).is_err(), "{bad}");
        }
        assert!(Selector::parse(&json!({"app.kubernetes.io/name": "cert-manager"})).is_ok());
    }

    /// Settings → Apps states these capabilities' facts itself, since they are not
    /// in the catalog: it must say what the host declares, word for word.
    #[test]
    fn the_review_states_the_hosts_own_facts_for_the_pod_capabilities() {
        const REVIEW: &str =
            include_str!("../../../../packages/ui-next/src/extensions/ExtensionBindings.tsx");
        let template = EXEC_ANNOTATIONS.confirm.expect("exec is confirmed");
        assert!(
            REVIEW.contains(&format!("EXEC_CONFIRM = \"{template}\"")),
            "ExtensionBindings.tsx's EXEC_CONFIRM must be {template:?}"
        );
        for (target, annotations) in [
            ("POD_LOGS", Annotations::READ_ONLY),
            ("POD_EXEC", EXEC_ANNOTATIONS),
            ("POD_FORWARD", FORWARD_ANNOTATIONS),
        ] {
            let line = REVIEW
                .lines()
                .find(|line| line.trim_start().starts_with(&format!("[{target}]:")))
                .unwrap_or_else(|| panic!("no POD_FACTS line for {target}"));
            let kind = if annotations.sensitive {
                "Sensitive"
            } else {
                "Read-only"
            };
            assert!(
                line.contains(&format!(
                    "\"{kind} · {} impact",
                    annotations.impact.as_str()
                )),
                "{target}: {line}"
            );
        }
        let declared: Vec<_> = capabilities()
            .into_iter()
            .map(|c| (c.id, c.annotations))
            .collect();
        assert_eq!(declared[1], (POD_EXEC.to_owned(), EXEC_ANNOTATIONS));
        assert_eq!(declared[2], (POD_FORWARD.to_owned(), FORWARD_ANNOTATIONS));
        const { assert!(EXEC_ANNOTATIONS.requires_confirm && EXEC_ANNOTATIONS.sensitive) };
        srelens_capability::check_confirm_template(template).unwrap();
    }

    #[test]
    fn a_container_is_the_bindings_the_views_or_the_only_one() {
        let binding = |arguments: Value| Binding {
            name: "logs".into(),
            title: "Logs".into(),
            target: POD_LOGS.into(),
            versions: vec![],
            json_path_overrides: Default::default(),
            arguments: arguments.as_object().unwrap().clone(),
            inputs: vec![],
        };
        let pod = PodFacts {
            name: "web-1".into(),
            containers: vec!["app".into(), "sidecar".into()],
            ..PodFacts::default()
        };
        let fixed = binding(json!({"container": "app"}));
        assert_eq!(container(&fixed, &pod, None).unwrap(), "app");
        assert_eq!(container(&fixed, &pod, Some("app")).unwrap(), "app");
        assert!(container(&fixed, &pod, Some("sidecar")).is_err());
        let open = binding(json!({}));
        assert_eq!(container(&open, &pod, Some("sidecar")).unwrap(), "sidecar");
        assert!(container(&open, &pod, Some("nope")).is_err());
        let why = container(&open, &pod, None).unwrap_err();
        assert!(why.contains("app, sidecar"), "{why}");
        let single = PodFacts {
            containers: vec!["only".into()],
            ..pod.clone()
        };
        assert_eq!(container(&open, &single, None).unwrap(), "only");
    }
}
