//! The policy an administrator holds a host's apps to (#578): which apps may be
//! installed and used, from which publishers, with which host capabilities, and which
//! hosts their `network.http` requests may reach.
//!
//! The desktop has none: its apps are one person's, and that person decides. The web
//! server holds every user's apps to one, read from its deployment config, and the
//! broker applies it on every call, not only at install:
//!
//! - Every inventory load ([`super::read_under`]) marks each app the policy refuses as
//!   `policyBlocked`, and disabled. Every reader, resource read, action, resolver and
//!   stream refuses such an app, so one installed before the policy changed is refused
//!   from its next call. The mark is never saved: lifting the policy brings the app
//!   back as its user left it.
//! - Install, update, rollback and enable refuse such an app, and `extensions.validate`
//!   reports why (`EXTENSION_POLICY_REFUSED`), so the review says so first.
//! - Every `network.http` request and redirect must go to a host the policy's ceiling
//!   allows as well as one the app's own hosts allow, and over HTTPS: under a policy,
//!   plain HTTP to loopback would reach the host itself, not a person's computer.
//! - A required app cannot be removed or disabled.
//!
//! A policy is checked whole whenever it is read, by [`AppPolicy::parse`] and by any
//! other deserialization, which goes through the same checks. An entry that could never
//! match, such as an unknown publisher or capability, a wildcard app ID or a host that is
//! not one, is refused rather than quietly allowing or blocking nothing.
use super::{Installed, Inventory, InventoryKey, InventoryLock, InventoryStore, TrustRoot};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use srelens_plugin_host::{HostRule, Manifest, ManifestKind, NETWORK_HTTP};
use std::collections::BTreeSet;
use std::sync::{Arc, RwLock};

/// The largest policy [`AppPolicy::parse`] reads.
pub const MAX_POLICY_BYTES: usize = 1024 * 1024;

/// What an administrator allows a host's apps to be and do. [`AppPolicy::default`]
/// allows what a host with no policy does, except that `network.http` reaches no host,
/// and no policy lets an app use plain HTTP to loopback.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "Unchecked")]
pub struct AppPolicy {
    /// Only these app IDs may be installed and used; `null` allows any.
    #[serde(rename = "allowedApps")]
    allowed_apps: Option<BTreeSet<String>>,
    /// These app IDs may not be installed or used, whatever else allows them.
    #[serde(rename = "blockedApps")]
    blocked_apps: BTreeSet<String>,
    /// Signed apps only from these publishers, each named by its delegation's ID (#559);
    /// `null` allows every publisher the host trusts. An unsigned app is governed by
    /// `allowUnsignedApps` instead.
    #[serde(rename = "allowedPublishers")]
    allowed_publishers: Option<BTreeSet<String>>,
    /// Whether apps with no publisher signature may be installed and used at all. Each
    /// user's own "Allow unsigned apps to modify clusters" still applies on top.
    #[serde(rename = "allowUnsignedApps")]
    allow_unsigned_apps: bool,
    /// The host capabilities an app may be granted; `null` allows every one the host
    /// offers apps.
    #[serde(rename = "allowedCapabilities")]
    allowed_capabilities: Option<BTreeSet<String>>,
    /// Whether an app may declare write actions (the host action primitives). Running
    /// commands in pods is `k8s.exec`, allowed or not by `allowedCapabilities`.
    #[serde(rename = "allowWriteActions")]
    allow_write_actions: bool,
    /// The most `network.http` may reach, written as an app's hosts are: `name`,
    /// `name:port`, `*.example.com` or an IP address. A request goes only to a host
    /// both this and the app allow. Empty: `network.http` reaches no host.
    #[serde(rename = "networkCeiling")]
    network_ceiling: Vec<String>,
    /// Always `false`. A policy is a shared host's, and executable apps (#574) do not run
    /// on one until they have a per-user sidecar identity (#521). The desktop, which has
    /// no policy, runs them.
    #[serde(rename = "allowExecutableApps")]
    allow_executable_apps: bool,
    /// Apps every user keeps: one they have installed cannot be removed or disabled. The
    /// policy never installs an app for anyone.
    #[serde(rename = "requiredApps")]
    required_apps: BTreeSet<String>,
}

impl Default for AppPolicy {
    fn default() -> Self {
        Self {
            allowed_apps: None,
            blocked_apps: BTreeSet::new(),
            allowed_publishers: None,
            allow_unsigned_apps: true,
            allowed_capabilities: None,
            allow_write_actions: true,
            network_ceiling: Vec::new(),
            allow_executable_apps: false,
            required_apps: BTreeSet::new(),
        }
    }
}

/// A policy as written, before [`AppPolicy`]'s checks. Every field may be left out.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Unchecked {
    #[serde(default, rename = "allowedApps")]
    allowed_apps: Option<BTreeSet<String>>,
    #[serde(default, rename = "blockedApps")]
    blocked_apps: BTreeSet<String>,
    #[serde(default, rename = "allowedPublishers")]
    allowed_publishers: Option<BTreeSet<String>>,
    #[serde(default = "allowed", rename = "allowUnsignedApps")]
    allow_unsigned_apps: bool,
    #[serde(default, rename = "allowedCapabilities")]
    allowed_capabilities: Option<BTreeSet<String>>,
    #[serde(default = "allowed", rename = "allowWriteActions")]
    allow_write_actions: bool,
    #[serde(default, rename = "networkCeiling")]
    network_ceiling: Vec<String>,
    #[serde(default, rename = "allowExecutableApps")]
    allow_executable_apps: bool,
    #[serde(default, rename = "requiredApps")]
    required_apps: BTreeSet<String>,
}

fn allowed() -> bool {
    true
}

/// The publishers a policy may name: those this build ships a delegation for, by ID
/// (#559). Checked against the build, not the catalog, which a policy is read before and
/// which may change after: a publisher only a catalog delegates to is not one yet.
pub(super) fn publisher_ids() -> Vec<String> {
    TrustRoot::pinned()
        .shipped()
        .publishers()
        .iter()
        .map(|publisher| publisher.id.clone())
        .collect()
}

/// Every host capability an app can be granted on a host, as `validate_app` accepts
/// them as targets (the pod bindings' since #567), and the secret store an app with
/// secret settings asks for. A new target there has to be added here before a policy
/// can name it.
fn grantable() -> impl Iterator<Item = &'static str> {
    [
        "k8s.listCustomResource",
        "k8s.listEvents",
        NETWORK_HTTP,
        srelens_plugin_host::SECRET_STORE_PERMISSION,
    ]
    .into_iter()
    .chain(srelens_plugin_host::BUILTIN_READERS.iter().copied())
    .chain(srelens_plugin_host::POD_TARGETS.iter().copied())
    .chain(srelens_kube::action_primitives::PRIMITIVES.iter().copied())
}

impl TryFrom<Unchecked> for AppPolicy {
    type Error = String;

    fn try_from(policy: Unchecked) -> Result<Self, String> {
        let mut problems = Vec::new();
        for (field, ids) in [
            (
                "allowedApps",
                policy.allowed_apps.iter().flatten().collect::<Vec<_>>(),
            ),
            ("blockedApps", policy.blocked_apps.iter().collect()),
            ("requiredApps", policy.required_apps.iter().collect()),
        ] {
            for id in ids
                .into_iter()
                .filter(|id| !srelens_plugin_host::is_app_id(id))
            {
                problems.push(format!(
                    "{field}: {id:?} is not an app ID; name each app by its exact reverse-domain ID"
                ));
            }
        }
        let publishers = publisher_ids();
        for name in policy.allowed_publishers.iter().flatten() {
            if !publishers.contains(name) {
                problems.push(format!(
                    "allowedPublishers: {name:?} is not a publisher this host trusts ({})",
                    publishers.join(", ")
                ));
            }
        }
        let capabilities: BTreeSet<_> = grantable().collect();
        for id in policy.allowed_capabilities.iter().flatten() {
            if !capabilities.contains(id.as_str()) {
                problems.push(format!(
                    "allowedCapabilities: {id:?} is not a capability an app can be granted ({})",
                    capabilities.iter().copied().collect::<Vec<_>>().join(", ")
                ));
            }
        }
        for host in &policy.network_ceiling {
            if HostRule::parse(host).is_err() {
                problems.push(format!(
                    "networkCeiling: {host:?} is not a host; write `name`, `name:port`, `*.example.com` (one subdomain label) or an IP address (`[::1]` for IPv6), in lowercase ASCII with no scheme or path"
                ));
            }
        }
        if policy.allow_executable_apps {
            problems.push(
                "allowExecutableApps: executable apps stay off until they run with a per-user sidecar identity (#521); leave it false"
                    .into(),
            );
        }
        for id in &policy.required_apps {
            if policy.blocked_apps.contains(id) {
                problems.push(format!("requiredApps: {id} is also in blockedApps"));
            }
            if policy
                .allowed_apps
                .as_ref()
                .is_some_and(|apps| !apps.contains(id))
            {
                problems.push(format!("requiredApps: {id} is not in allowedApps"));
            }
        }
        if !problems.is_empty() {
            return Err(problems.join("\n"));
        }
        Ok(Self {
            allowed_apps: policy.allowed_apps,
            blocked_apps: policy.blocked_apps,
            allowed_publishers: policy.allowed_publishers,
            allow_unsigned_apps: policy.allow_unsigned_apps,
            allowed_capabilities: policy.allowed_capabilities,
            allow_write_actions: policy.allow_write_actions,
            network_ceiling: policy.network_ceiling,
            allow_executable_apps: false,
            required_apps: policy.required_apps,
        })
    }
}

/// Why a policy refuses an app, and the manifest field that answers for it.
pub(super) struct Refusal {
    pub path: &'static str,
    pub reason: String,
}

impl AppPolicy {
    /// A policy document, or every reason it is not one, one per line.
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.len() > MAX_POLICY_BYTES {
            return Err("an app policy is at most 1 MiB".into());
        }
        serde_json::from_str(text).map_err(|e| e.to_string())
    }

    /// Why this policy refuses the app `manifest` describes, signed by the publisher
    /// named, or unsigned when `None`. Checked in the order an administrator would ask.
    pub(super) fn refusal(&self, manifest: &Manifest, publisher: Option<&str>) -> Option<Refusal> {
        let id = manifest.id.as_str();
        let refuse = |path, reason: String| Some(Refusal { path, reason });
        // Exhaustive on purpose: an executable app (#574) is refused here until it runs
        // with a per-user sidecar identity (#521), whatever the rest of the policy says.
        match manifest.kind {
            ManifestKind::Declarative => {}
            ManifestKind::Executable => {
                return refuse(
                    "kind",
                    "The administrator's policy does not allow executable apps".into(),
                )
            }
        }
        if self.blocked_apps.contains(id) {
            return refuse("id", format!("The administrator's policy blocks {id}"));
        }
        if self
            .allowed_apps
            .as_ref()
            .is_some_and(|apps| !apps.contains(id))
        {
            return refuse(
                "id",
                format!("The administrator's policy does not allow {id}"),
            );
        }
        match publisher {
            None if !self.allow_unsigned_apps => {
                return refuse(
                    "",
                    "The administrator's policy allows only signed apps".into(),
                );
            }
            Some(name)
                if self
                    .allowed_publishers
                    .as_ref()
                    .is_some_and(|names| !names.contains(name)) =>
            {
                return refuse(
                    "",
                    format!("The administrator's policy does not allow apps signed by {name}"),
                );
            }
            _ => {}
        }
        // Write actions only: a command in a pod (`k8s.exec`, #567) is its own
        // capability, which `allowedCapabilities` allows or refuses.
        if !manifest.actions.is_empty() && !self.allow_write_actions {
            return refuse(
                "actions",
                "The administrator's policy does not allow apps that write to clusters".into(),
            );
        }
        let permissions = manifest.permission_names();
        let refused: Vec<_> = permissions
            .iter()
            .filter(|id| {
                self.allowed_capabilities
                    .as_ref()
                    .is_some_and(|allowed| !allowed.contains(id.as_str()))
            })
            .map(String::as_str)
            .collect();
        if !refused.is_empty() {
            return refuse(
                "permissions",
                format!(
                    "The administrator's policy does not allow {}",
                    refused.join(", ")
                ),
            );
        }
        if permissions.iter().any(|id| id == NETWORK_HTTP) && self.network_ceiling.is_empty() {
            return refuse(
                "permissions",
                format!("The administrator's policy lets {NETWORK_HTTP} reach no host"),
            );
        }
        None
    }

    /// Why this policy refuses the stored app `plugin`, taking its publisher from its
    /// signature only when that verified on this load.
    pub(super) fn refuses(&self, plugin: &Installed) -> Option<Refusal> {
        let publisher = plugin.signed_by.as_ref().map(|signer| signer.id.as_str());
        self.refusal(&plugin.manifest, publisher)
    }

    /// Whether every user must keep the app `id`.
    pub(super) fn requires(&self, id: &str) -> bool {
        self.required_apps.contains(id)
    }

    /// The most a `network.http` request may reach under this policy.
    pub(super) fn ceiling(&self) -> Vec<HostRule> {
        self.network_ceiling
            .iter()
            .filter_map(|host| HostRule::parse(host).ok())
            .collect()
    }

    /// Whether `network.http` may reach any host at all under this policy.
    pub(super) fn reaches_network(&self) -> bool {
        !self.network_ceiling.is_empty()
    }
}

/// Holds `state` to `policy`, as every read of a governed inventory does: each app the
/// policy refuses is marked with why, and disabled, and none may use plain HTTP to
/// loopback. `state` is then what a caller sees, never what is saved: `configure`
/// changes the inventory as saved, and governs only the copy it answers with.
pub(super) fn govern(state: &mut Inventory, policy: Option<&AppPolicy>) {
    let Some(policy) = policy else {
        return;
    };
    for plugin in &mut state.plugins {
        if let Some(refusal) = policy.refuses(plugin) {
            plugin.policy_blocked = Some(refusal.reason);
            plugin.enabled = false;
        }
        plugin.allow_loopback_http = false;
    }
    state.policy = Some(policy.clone());
}

/// One policy, shared by every registry a host builds and read again by each call, so a
/// replaced policy governs every user's next call without rebuilding anything.
#[derive(Clone)]
pub struct SharedPolicy(Arc<RwLock<Arc<AppPolicy>>>);

/// The default policy, shared.
impl Default for SharedPolicy {
    fn default() -> Self {
        Self::new(AppPolicy::default())
    }
}

impl SharedPolicy {
    pub fn new(policy: AppPolicy) -> Self {
        Self(Arc::new(RwLock::new(Arc::new(policy))))
    }

    /// The policy in force now.
    pub fn current(&self) -> Arc<AppPolicy> {
        // A policy is plain data, whole before and after every write, so a panic
        // elsewhere while the lock was held leaves nothing half-written to refuse.
        self.0
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Put `policy` in force for every registry that shares this one, from its next call
    /// on. The one way a policy changes: a write path (#739) checks its caller first.
    ///
    /// Two things are settled when a registry is built, not per call, so a write path
    /// rebuilds its users' registries after this: whether it has `network.http` at all
    /// (a ceiling that names no host refuses every request either way), and which app
    /// streams are open, which are checked again only at their next read, inventory
    /// write or reconnect.
    pub fn replace(&self, policy: AppPolicy) {
        *self
            .0
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Arc::new(policy);
    }
}

/// An inventory held to a policy: every read and save goes to the store it wraps, and
/// [`super::read_under`] holds what it loads to the policy in force.
pub(super) struct Governed {
    pub inventory: Arc<dyn InventoryStore>,
    pub policy: SharedPolicy,
}

impl InventoryStore for Governed {
    fn key(&self) -> InventoryKey {
        self.inventory.key()
    }
    fn load(&self, limit: usize) -> Result<Option<Vec<u8>>, String> {
        self.inventory.load(limit)
    }
    fn save(&self, raw: &[u8]) -> Result<(), String> {
        self.inventory.save(raw)
    }
    fn lock(&self) -> Result<InventoryLock, String> {
        self.inventory.lock()
    }
    fn policy(&self) -> Option<Arc<AppPolicy>> {
        Some(self.policy.current())
    }
}
