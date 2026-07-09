//! Kubernetes integration for the srelens core: kubeconfig discovery, the
//! cluster model, and kube-related capabilities.

use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
use k8s_openapi::chrono::Utc;
use kube::core::NamespaceResourceScope;
use kube::{Api, Client, Resource};

/// Format a duration (seconds) as a compact srelens age string (5s, 3m,
/// 2h, 4d, 1y).
pub(crate) fn format_age(secs: i64) -> String {
    let s = secs.max(0);
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m", s / 60)
    } else if s < 86_400 {
        format!("{}h", s / 3600)
    } else if s < 86_400 * 365 {
        format!("{}d", s / 86_400)
    } else {
        format!("{}y", s / (86_400 * 365))
    }
}

/// Compact age of a resource from its `creationTimestamp` ("-" if unset).
pub(crate) fn humanize_age(creation: Option<&Time>) -> String {
    match creation {
        Some(t) => format_age((Utc::now() - t.0).num_seconds()),
        None => "-".to_string(),
    }
}

/// Abbreviate PV/PVC access modes ("ReadWriteOnce" → "RWO"), comma-joined.
pub(crate) fn abbreviate_access_modes(modes: Option<&Vec<String>>) -> String {
    modes
        .map(|m| {
            m.iter()
                .map(|mode| match mode.as_str() {
                    "ReadWriteOnce" => "RWO",
                    "ReadOnlyMany" => "ROX",
                    "ReadWriteMany" => "RWX",
                    "ReadWriteOncePod" => "RWOP",
                    other => other,
                })
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod access_mode_tests {
    use super::abbreviate_access_modes;

    #[test]
    fn abbreviates_known_modes() {
        let modes = vec!["ReadWriteOnce".to_string(), "ReadOnlyMany".to_string()];
        assert_eq!(abbreviate_access_modes(Some(&modes)), "RWO, ROX");
        assert_eq!(abbreviate_access_modes(None), "");
    }
}

/// Build a namespaced API, or an all-namespaces API when `namespace` is empty.
/// An empty namespace is how the UI requests "All namespaces".
pub(crate) fn scoped_api<K>(client: Client, namespace: &str) -> Api<K>
where
    K: Resource<Scope = NamespaceResourceScope>,
    <K as Resource>::DynamicType: Default,
{
    if namespace.is_empty() {
        Api::all(client)
    } else {
        Api::namespaced(client, namespace)
    }
}

#[cfg(test)]
mod age_tests {
    use super::format_age;

    #[test]
    fn formats_compact_age() {
        assert_eq!(format_age(-5), "0s");
        assert_eq!(format_age(42), "42s");
        assert_eq!(format_age(120), "2m");
        assert_eq!(format_age(3 * 3600), "3h");
        assert_eq!(format_age(5 * 86_400), "5d");
        assert_eq!(format_age(400 * 86_400), "1y");
    }
}

pub mod actions;
pub mod client_cache;
pub mod cluster;
pub mod crds;
pub mod connect;
pub mod contexts;
pub mod configmaps;
pub mod cronjobs;
pub mod daemonsets;
pub mod deployments;
pub mod endpointslices;
pub mod events;
pub mod exec;
pub mod forward;
pub mod helm;
pub mod ingresses;
pub mod jobs;
pub mod kubeconfig;
pub mod limitranges;
pub mod local_cluster;
pub mod logs;
pub mod manifest;
pub mod metrics;
pub mod networkpolicies;
pub mod nodes;
pub mod persistentvolumes;
pub mod pvcs;
pub mod resourcequotas;
pub mod rolebindings;
pub mod roles;
pub mod schema;
pub mod secrets;
pub mod serviceaccounts;
pub mod services;
pub mod statefulsets;
pub mod storageclasses;
pub mod watch;
pub mod workloads;
