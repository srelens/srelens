//! The built-in kinds a resource link may point at (#728).
//!
//! A link's target is looked up in a list the host reads. For a custom resource that list
//! is the app's own declared reader; for a built-in kind it is this table: the Kubernetes
//! kinds the Inspector opens, by group and version, and whether they are namespaced. The host reads only their metadata for a link, and of a Secret only its
//! identity.

/// A Kubernetes built-in kind, as the host lists it for a resource link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinKind {
    /// The API group, `""` for the core group.
    pub group: &'static str,
    pub version: &'static str,
    pub kind: &'static str,
    pub namespaced: bool,
}

const fn kind(
    group: &'static str,
    version: &'static str,
    kind: &'static str,
    namespaced: bool,
) -> BuiltinKind {
    BuiltinKind {
        group,
        version,
        kind,
        namespaced,
    }
}

/// Every built-in kind a link may name as its `to`: the kinds the Inspector opens, less
/// `Event`, which is a record about a resource rather than one another resource points at.
pub const BUILTIN_LINK_KINDS: &[BuiltinKind] = &[
    kind("", "v1", "Pod", true),
    kind("", "v1", "Service", true),
    kind("", "v1", "Endpoints", true),
    kind("", "v1", "ConfigMap", true),
    kind("", "v1", "Secret", true),
    kind("", "v1", "ServiceAccount", true),
    kind("", "v1", "PersistentVolumeClaim", true),
    kind("", "v1", "PersistentVolume", false),
    kind("", "v1", "ResourceQuota", true),
    kind("", "v1", "LimitRange", true),
    kind("", "v1", "Namespace", false),
    kind("", "v1", "Node", false),
    kind("apps", "v1", "Deployment", true),
    kind("apps", "v1", "StatefulSet", true),
    kind("apps", "v1", "DaemonSet", true),
    kind("apps", "v1", "ReplicaSet", true),
    kind("batch", "v1", "Job", true),
    kind("batch", "v1", "CronJob", true),
    kind("autoscaling", "v2", "HorizontalPodAutoscaler", true),
    kind("policy", "v1", "PodDisruptionBudget", true),
    kind("networking.k8s.io", "v1", "Ingress", true),
    kind("networking.k8s.io", "v1", "IngressClass", false),
    kind("networking.k8s.io", "v1", "NetworkPolicy", true),
    kind("discovery.k8s.io", "v1", "EndpointSlice", true),
    kind("rbac.authorization.k8s.io", "v1", "Role", true),
    kind("rbac.authorization.k8s.io", "v1", "RoleBinding", true),
    kind("rbac.authorization.k8s.io", "v1", "ClusterRole", false),
    kind(
        "rbac.authorization.k8s.io",
        "v1",
        "ClusterRoleBinding",
        false,
    ),
    kind("storage.k8s.io", "v1", "StorageClass", false),
    kind("scheduling.k8s.io", "v1", "PriorityClass", false),
    kind("node.k8s.io", "v1", "RuntimeClass", false),
    kind("coordination.k8s.io", "v1", "Lease", true),
    kind(
        "admissionregistration.k8s.io",
        "v1",
        "MutatingWebhookConfiguration",
        false,
    ),
    kind(
        "admissionregistration.k8s.io",
        "v1",
        "ValidatingWebhookConfiguration",
        false,
    ),
];

/// The built-in kind a qualified kind (`group/Kind`, `/Pod` for the core group) names,
/// when a link may point at it. Exact: `/Deployment` and `acme.io/Service` are not the
/// built-in ones.
pub fn builtin_link_kind(qualified: &str) -> Option<&'static BuiltinKind> {
    let (group, name) = qualified.split_once('/')?;
    BUILTIN_LINK_KINDS
        .iter()
        .find(|builtin| builtin.group == group && builtin.kind == name)
}
