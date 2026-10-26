//! Whether the AI Assistant may make a call (#968).
//!
//! The person decides, in srelens Settings, that the assistant is off — for
//! everything, or for some clusters. The UI hides its entry points; this is
//! what makes that true for every agent that reaches a cluster through srelens,
//! including an outside client pointed at this server. The setting itself is
//! read by the host (`AssistantSource`), because this crate knows neither the
//! settings file nor the kubeconfig contexts.
//!
//! The agent is treated as working against the setting: besides naming a
//! switched-off cluster, it could re-enable itself, load the same kubeconfig
//! under another path, or reach a node by address. Each of those is refused
//! here. What remains — an agent with a shell of its own — is outside srelens.

use serde_json::Value;

use crate::APP_TOOL_PREFIX;

/// The settings key the switches live under, as the frontend writes it.
pub const SETTING_KEY: &str = "srelens.assistant";
const KUBECONFIG_FILES_KEY: &str = "srelens.kubeconfigFiles";

/// Tools that can reach a cluster other than the one they name: the node-SSH
/// tools take a node by IP or hostname with the cluster optional, and the two
/// kubeconfig tools take one the caller writes — which can point at the
/// person's own credential files, or at any server.
///
/// ponytail: a hand-kept list. A new capability that picks its cluster any way
/// but `context` belongs here (or in `CLUSTER_KEYS`); an allowlist of tools
/// usable while some clusters are off is the upgrade if this list ever drifts.
const REACHES_ANY: [&str; 6] = [
    "k8s.nodeServiceStatus",
    "k8s.nodeJournalLogs",
    "k8s.nodeRuntimeDiagnostics",
    "k8s.nodeServiceRestart",
    "k8s.testClusterConnection",
    "k8s.synthesizeClusterKubeconfig",
];

/// The argument keys a call names a cluster under, in any scope `named_clusters` reads.
const CLUSTER_KEYS: [&str; 5] = ["context", "cluster", "clusterId", "hubContext", "contexts"];

/// What the setting says right now, as the host read it for one call.
#[derive(Debug, Clone, PartialEq)]
pub struct AssistantSnapshot {
    pub enabled: bool,
    pub disabled: Vec<DisabledCluster>,
}

/// One switched-off cluster: what to call it, and every name a call could use
/// for it (pinned ID, stable ID, display name, the kubeconfig's own name).
#[derive(Debug, Clone, PartialEq)]
pub struct DisabledCluster {
    pub label: String,
    pub names: Vec<String>,
}

/// Where the setting comes from. Read per call, so a switch takes effect on the
/// next one. An `Err` refuses the call with its text: a setting that cannot be
/// read must not read as "on".
#[async_trait::async_trait]
pub trait AssistantSource: Send + Sync {
    async fn snapshot(&self) -> Result<AssistantSnapshot, String>;
}

/// The default: a host that wires no source keeps today's behaviour.
pub struct AlwaysOn;

#[async_trait::async_trait]
impl AssistantSource for AlwaysOn {
    async fn snapshot(&self) -> Result<AssistantSnapshot, String> {
        Ok(AssistantSnapshot {
            enabled: true,
            disabled: Vec::new(),
        })
    }
}

/// Whether a `settings.set` call writes or removes `key`.
fn sets(tool: &str, args: &Value, key: &str) -> bool {
    tool == "settings.set"
        && (args["values"].get(key).is_some()
            || args["remove"]
                .as_array()
                .is_some_and(|keys| keys.iter().any(|k| k == key)))
}

/// The one refusal that holds whether the assistant is on or off: an agent may
/// never change the switch that governs it.
pub fn guarded_setting(tool: &str, args: &Value) -> Option<String> {
    sets(tool, args, SETTING_KEY)
        .then(|| "The AI Assistant setting can only be changed in srelens Settings.".to_string())
}

/// Every cluster a call names, under any of `CLUSTER_KEYS` — a string, or a
/// list of strings — at the top level, under `resource` (where
/// `srelens_capability::audit::describe_target` looks) and under `params` (an
/// app operation's `clusterId`). A value that is not a context name can only
/// refuse more.
pub fn named_clusters(args: &Value) -> Vec<String> {
    let scopes = [Some(args), args.get("resource"), args.get("params")];
    let mut named = Vec::new();
    for scope in scopes.into_iter().flatten().filter(|v| v.is_object()) {
        for value in CLUSTER_KEYS.iter().filter_map(|key| scope.get(*key)) {
            let items = value
                .as_array()
                .map_or_else(|| vec![value], |list| list.iter().collect());
            named.extend(
                items
                    .into_iter()
                    .filter_map(Value::as_str)
                    .filter(|n| !n.is_empty())
                    .map(str::to_owned),
            );
        }
    }
    named
}

/// Rules 2–6 of the design, in order. `schema_has_context`: whether the tool's
/// input schema has a `context` property, so a call without one could still
/// reach a cluster (the current context).
pub fn decide(
    snapshot: &AssistantSnapshot,
    tool: &str,
    args: &Value,
    schema_has_context: bool,
) -> Result<(), String> {
    if !snapshot.enabled {
        return Err("The srelens AI Assistant is turned off in Settings.".into());
    }
    let named = named_clusters(args);
    for name in &named {
        if let Some(cluster) = snapshot.disabled.iter().find(|c| c.names.contains(name)) {
            return Err(format!(
                "The srelens AI Assistant is turned off for cluster {}.",
                cluster.label
            ));
        }
    }
    if snapshot.disabled.is_empty() {
        return Ok(());
    }
    let new_paths =
        tool == "k8s.listContexts" && args["paths"].as_array().is_some_and(|p| !p.is_empty());
    if new_paths || sets(tool, args, KUBECONFIG_FILES_KEY) {
        return Err(
            "Kubeconfig files can't be changed by the AI Assistant while it is turned off for some clusters.".into(),
        );
    }
    if REACHES_ANY.contains(&tool) || tool.starts_with(APP_TOOL_PREFIX) {
        return Err(
            "This tool can reach any cluster, so it is unavailable while the AI Assistant is turned off for some clusters.".into(),
        );
    }
    if named.is_empty() && schema_has_context {
        return Err("Name the cluster: the AI Assistant is turned off for some clusters.".into());
    }
    Ok(())
}

/// A source that answers a script of snapshots, repeating the last — for the
/// request-path tests in this crate.
#[cfg(test)]
pub(crate) struct Scripted(std::sync::Mutex<std::collections::VecDeque<Result<AssistantSnapshot, String>>>);

#[cfg(test)]
impl Scripted {
    pub(crate) fn new(script: Vec<Result<AssistantSnapshot, String>>) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self(std::sync::Mutex::new(script.into())))
    }

    /// Off for one cluster called `label`, known by `names`.
    pub(crate) fn off_for(label: &str, names: &[&str]) -> std::sync::Arc<Self> {
        Self::new(vec![Ok(off_for(label, names))])
    }
}

#[cfg(test)]
pub(crate) fn off_for(label: &str, names: &[&str]) -> AssistantSnapshot {
    AssistantSnapshot {
        enabled: true,
        disabled: vec![DisabledCluster {
            label: label.into(),
            names: names.iter().map(|n| n.to_string()).collect(),
        }],
    }
}

#[cfg(test)]
#[async_trait::async_trait]
impl AssistantSource for Scripted {
    async fn snapshot(&self) -> Result<AssistantSnapshot, String> {
        let mut script = self.0.lock().unwrap();
        if script.len() > 1 {
            script.pop_front().unwrap()
        } else {
            script.front().cloned().expect("a scripted source needs one answer")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const OFF: &str = "The srelens AI Assistant is turned off in Settings.";
    const KUBECONFIG: &str =
        "Kubeconfig files can't be changed by the AI Assistant while it is turned off for some clusters.";
    const REACH: &str =
        "This tool can reach any cluster, so it is unavailable while the AI Assistant is turned off for some clusters.";
    const NAME: &str = "Name the cluster: the AI Assistant is turned off for some clusters.";

    fn on() -> AssistantSnapshot {
        AssistantSnapshot {
            enabled: true,
            disabled: vec![],
        }
    }

    fn off_for_prod() -> AssistantSnapshot {
        AssistantSnapshot {
            enabled: true,
            disabled: vec![DisabledCluster {
                label: "kube_a/prod".into(),
                names: vec![
                    "srelens-context:/k/a#prod".into(),
                    "/k/a#prod".into(),
                    "kube_a/prod".into(),
                    "prod".into(),
                ],
            }],
        }
    }

    #[test]
    fn settings_set_of_the_assistant_key_is_refused_even_when_on() {
        let refusal =
            Some("The AI Assistant setting can only be changed in srelens Settings.".to_string());
        let write = json!({"values": {"srelens.assistant": {"enabled": true}}});
        let remove = json!({"remove": ["srelens.assistant"]});
        assert_eq!(guarded_setting("settings.set", &write), refusal);
        assert_eq!(guarded_setting("settings.set", &remove), refusal);
        assert_eq!(
            guarded_setting(
                "settings.set",
                &json!({"values": {"srelens.design": "next"}})
            ),
            None
        );
        assert_eq!(
            guarded_setting("settings.get", &json!({"key": "srelens.assistant"})),
            None
        );
    }

    #[test]
    fn global_off_refuses_everything() {
        let off = AssistantSnapshot {
            enabled: false,
            disabled: vec![],
        };
        assert_eq!(decide(&off, "ping", &json!({}), false), Err(OFF.into()));
        assert_eq!(
            decide(&off, "k8s.listPods", &json!({"context": "dev"}), true),
            Err(OFF.into())
        );
    }

    #[test]
    fn a_call_naming_a_disabled_cluster_by_any_of_its_names_is_refused() {
        let refused =
            Err("The srelens AI Assistant is turned off for cluster kube_a/prod.".to_string());
        for name in [
            "srelens-context:/k/a#prod",
            "/k/a#prod",
            "kube_a/prod",
            "prod",
        ] {
            assert_eq!(
                decide(
                    &off_for_prod(),
                    "k8s.listPods",
                    &json!({"context": name}),
                    true
                ),
                refused
            );
        }
        assert_eq!(
            decide(&off_for_prod(), "x", &json!({"cluster": "prod"}), false),
            refused
        );
        assert_eq!(
            decide(
                &off_for_prod(),
                "extensions.action",
                &json!({"action": "a", "resource": {"context": "prod"}}),
                false
            ),
            refused
        );
    }

    #[test]
    fn a_call_naming_an_allowed_cluster_passes() {
        assert_eq!(
            decide(
                &off_for_prod(),
                "k8s.listPods",
                &json!({"context": "dev"}),
                true
            ),
            Ok(())
        );
        assert_eq!(
            decide(&on(), "k8s.listPods", &json!({"context": "prod"}), true),
            Ok(())
        );
    }

    #[test]
    fn kubeconfig_changes_are_refused_while_any_cluster_is_off() {
        let paths = json!({"paths": ["/elsewhere/config"]});
        let files = json!({"values": {"srelens.kubeconfigFiles": ["/x"]}});
        let unset = json!({"remove": ["srelens.kubeconfigFiles"]});
        assert_eq!(
            decide(&off_for_prod(), "k8s.listContexts", &paths, false),
            Err(KUBECONFIG.into())
        );
        assert_eq!(
            decide(&off_for_prod(), "settings.set", &files, false),
            Err(KUBECONFIG.into())
        );
        assert_eq!(
            decide(&off_for_prod(), "settings.set", &unset, false),
            Err(KUBECONFIG.into())
        );
    }

    #[test]
    fn kubeconfig_changes_pass_when_no_cluster_is_off() {
        assert_eq!(
            decide(
                &on(),
                "k8s.listContexts",
                &json!({"paths": ["/elsewhere/config"]}),
                false
            ),
            Ok(())
        );
        assert_eq!(
            decide(
                &on(),
                "settings.set",
                &json!({"values": {"srelens.kubeconfigFiles": ["/x"]}}),
                false
            ),
            Ok(())
        );
        // Listing what is already loaded changes nothing.
        assert_eq!(
            decide(&off_for_prod(), "k8s.listContexts", &json!({}), false),
            Ok(())
        );
        assert_eq!(
            decide(
                &off_for_prod(),
                "k8s.listContexts",
                &json!({"paths": []}),
                false
            ),
            Ok(())
        );
    }

    #[test]
    fn node_ssh_and_app_tools_are_refused_while_any_cluster_is_off() {
        for tool in [
            "k8s.nodeServiceStatus",
            "k8s.nodeJournalLogs",
            "k8s.nodeRuntimeDiagnostics",
            "k8s.nodeServiceRestart",
        ] {
            assert_eq!(
                decide(
                    &off_for_prod(),
                    tool,
                    &json!({"context": "dev", "node": "10.0.0.5"}),
                    true
                ),
                Err(REACH.into())
            );
            assert_eq!(
                decide(&on(), tool, &json!({"node": "10.0.0.5"}), true),
                Ok(())
            );
        }
        assert_eq!(
            decide(&off_for_prod(), "plugin/acme.scan/run", &json!({}), false),
            Err(REACH.into())
        );
        assert_eq!(
            decide(&on(), "plugin/acme.scan/run", &json!({}), false),
            Ok(())
        );
    }

    #[test]
    fn a_tool_with_a_context_property_called_without_one_is_refused_while_any_cluster_is_off() {
        assert_eq!(
            decide(&off_for_prod(), "k8s.listPods", &json!({}), true),
            Err(NAME.into())
        );
        assert_eq!(
            decide(
                &off_for_prod(),
                "k8s.listPods",
                &json!({"context": ""}),
                true
            ),
            Err(NAME.into())
        );
        assert_eq!(decide(&on(), "k8s.listPods", &json!({}), true), Ok(()));
    }

    /// Security review: a kubeconfig the caller writes can point at the
    /// person's own credential files, and a synthesised one at any server.
    #[test]
    fn tools_that_bring_their_own_kubeconfig_are_refused_while_any_cluster_is_off() {
        for tool in [
            "k8s.testClusterConnection",
            "k8s.synthesizeClusterKubeconfig",
        ] {
            assert_eq!(
                decide(
                    &off_for_prod(),
                    tool,
                    &json!({"context": "dev", "yaml": "x"}),
                    true
                ),
                Err(REACH.into())
            );
            assert_eq!(
                decide(&on(), tool, &json!({"context": "dev", "yaml": "x"}), true),
                Ok(())
            );
        }
    }

    /// Security review: a cluster named under another key, or in a list, is
    /// still named.
    #[test]
    fn a_disabled_cluster_named_under_any_cluster_key_is_refused() {
        let refused =
            Err("The srelens AI Assistant is turned off for cluster kube_a/prod.".to_string());
        for args in [
            json!({"context": "dev", "hubContext": "prod"}),
            json!({"context": "dev", "params": {"clusterId": "srelens-context:/k/a#prod"}}),
            json!({"contexts": ["dev", "prod"]}),
            json!({"resource": {"context": "dev", "clusterId": "prod"}}),
        ] {
            assert_eq!(
                decide(&off_for_prod(), "x", &args, false),
                refused,
                "{args}"
            );
        }
    }

    #[test]
    fn a_tool_with_no_context_property_passes() {
        assert_eq!(
            decide(&off_for_prod(), "toolbox.status", &json!({}), false),
            Ok(())
        );
    }
}
