//! Precision-first classification of kube contexts into local vs remote.
//!
//! The guiding rule: **never tag a production cluster as local.** Detection is
//! therefore biased toward precision, not recall:
//!
//! 1. A cluster earns `is_local` **only** from a tool-generated context/cluster
//!    name signature. Its network address alone never promotes it.
//! 2. Cloud authentication (an `aws`/`gcloud`/`az`/`kubelogin` exec plugin, or a
//!    `gcp`/`azure`/`oidc` auth provider) is a hard gate: it forces `remote`
//!    even when the server is a loopback address (e.g. a port-forwarded EKS).
//! 3. The server address is only allowed to *demote*: a public host cancels a
//!    local-looking name. A private/loopback host never promotes on its own.
//!
//! Anything we are unsure about stays remote. The heuristic is always
//! overridable by the user in the UI.

use serde::Serialize;

/// A recognised local-cluster provider. Serialised as its wire string (see
/// [`LocalProvider::as_str`]) for the `provider` field of a context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(into = "String")]
pub enum LocalProvider {
    Kind,
    K3d,
    Minikube,
    Microk8s,
    DockerDesktop,
    RancherDesktop,
    Colima,
    Orbstack,
    /// Kubernetes in Apple Containers (github.com/saiyam1814/kiac).
    Kiac,
    /// vCluster in Docker — `vcluster create <name> --driver docker` (vind).
    Vind,
}

impl LocalProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            LocalProvider::Kind => "kind",
            LocalProvider::K3d => "k3d",
            LocalProvider::Minikube => "minikube",
            LocalProvider::Microk8s => "microk8s",
            LocalProvider::DockerDesktop => "docker-desktop",
            LocalProvider::RancherDesktop => "rancher-desktop",
            LocalProvider::Colima => "colima",
            LocalProvider::Orbstack => "orbstack",
            LocalProvider::Kiac => "kiac",
            LocalProvider::Vind => "vind",
        }
    }
}

impl From<LocalProvider> for String {
    fn from(value: LocalProvider) -> Self {
        value.as_str().to_string()
    }
}

/// The outcome of classifying a single context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classification {
    pub is_local: bool,
    /// The detected local provider, present only when `is_local` is true.
    pub provider: Option<LocalProvider>,
}

impl Classification {
    fn remote() -> Self {
        Classification {
            is_local: false,
            provider: None,
        }
    }
}

/// Match a single name against the known tool-generated signatures.
///
/// Both the context name and the cluster name are checked by the caller, since
/// tools such as kind, kiac and vind name every entry identically.
fn provider_from_name(name: &str) -> Option<LocalProvider> {
    let n = name.trim();
    // Prefixed, per-cluster names.
    if n.starts_with("kind-") {
        return Some(LocalProvider::Kind);
    }
    if n.starts_with("k3d-") {
        return Some(LocalProvider::K3d);
    }
    if n.starts_with("kiac-") {
        return Some(LocalProvider::Kiac);
    }
    // vind: `vcluster create <name> --driver docker` writes `vcluster-docker_<name>`.
    // NB: the helm/platform drivers write `vcluster_<name>_<ns>_<host-context>`,
    // which is deliberately NOT matched — that tenant cluster may live in a
    // production host cluster and must stay remote.
    if n.starts_with("vcluster-docker_") {
        return Some(LocalProvider::Vind);
    }
    if n == "colima" || n.starts_with("colima-") {
        return Some(LocalProvider::Colima);
    }
    // Fixed, single-cluster names.
    match n {
        "minikube" => Some(LocalProvider::Minikube),
        "microk8s" => Some(LocalProvider::Microk8s),
        "docker-desktop" | "docker-for-desktop" => Some(LocalProvider::DockerDesktop),
        "rancher-desktop" => Some(LocalProvider::RancherDesktop),
        "orbstack" => Some(LocalProvider::Orbstack),
        _ => None,
    }
}

/// Exec-plugin command basenames that indicate a managed/cloud cluster.
const CLOUD_EXEC_COMMANDS: &[&str] = &[
    "aws",
    "aws-iam-authenticator",
    "gcloud",
    "gke-gcloud-auth-plugin",
    "az",
    "kubelogin",
    "kubectl-oidc_login",
    "oidc-login",
    "doctl",
    "linode-cli",
    "tsh",
];

/// Auth-provider names (the legacy `auth-provider` field) that indicate cloud.
const CLOUD_AUTH_PROVIDERS: &[&str] = &["gcp", "azure", "oidc"];

/// Whether the context's user authenticates via a cloud/managed mechanism.
/// Local tools (kind, k3d, minikube, docker-desktop, rancher-desktop, kiac,
/// vind) all use embedded client certs or tokens, never these.
fn is_cloud_auth(exec_command: Option<&str>, auth_provider: Option<&str>) -> bool {
    if let Some(cmd) = exec_command {
        // The command may be an absolute path and, on Windows, carry a `.exe`
        // suffix or mixed case; compare on its lowercased basename.
        let base = cmd.rsplit(['/', '\\']).next().unwrap_or(cmd).trim().to_ascii_lowercase();
        let base = base.strip_suffix(".exe").unwrap_or(&base);
        if CLOUD_EXEC_COMMANDS.contains(&base) {
            return true;
        }
    }
    if let Some(provider) = auth_provider {
        let provider = provider.trim().to_ascii_lowercase();
        if CLOUD_AUTH_PROVIDERS.contains(&provider.as_str()) {
            return true;
        }
    }
    false
}

/// Extract the host portion of a `scheme://host:port/...` server URL.
fn host_of(server: &str) -> &str {
    let after_scheme = match server.find("://") {
        Some(i) => &server[i + 3..],
        None => server,
    };
    // Strip path/query, then the port. IPv6 hosts are bracketed: [::1]:6443.
    let authority = after_scheme
        .split(['/', '?'])
        .next()
        .unwrap_or(after_scheme);
    if let Some(rest) = authority.strip_prefix('[') {
        // [ipv6]:port -> ipv6
        return rest.split(']').next().unwrap_or(rest);
    }
    authority.split(':').next().unwrap_or(authority)
}

/// Whether the server host is a routable public address. Empty/unparseable
/// hosts return false (we do not demote on missing information). Loopback,
/// RFC1918 private ranges, link-local, and `*.internal`/`*.local` names are
/// treated as non-public (consistent with a local cluster).
fn host_is_public(server: &str) -> bool {
    let host = host_of(server).trim().to_ascii_lowercase();
    if host.is_empty() {
        return false;
    }
    if host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".internal")
        || host.ends_with(".local")
    {
        return false;
    }
    // IPv4 ranges.
    if let Some(v4) = parse_ipv4(&host) {
        let [a, b, ..] = v4;
        let is_private = a == 127 // loopback 127.0.0.0/8
            || a == 10 // 10.0.0.0/8
            || (a == 192 && b == 168) // 192.168.0.0/16
            || (a == 172 && (16..=31).contains(&b)) // 172.16.0.0/12
            || (a == 169 && b == 254) // 169.254.0.0/16 link-local
            || v4 == [0, 0, 0, 0]; // 0.0.0.0
        return !is_private;
    }
    // IPv6 ranges, mirroring the IPv4 private handling: loopback (::1),
    // unspecified (::), unique-local (fc00::/7), and link-local unicast
    // (fe80::/10) are non-public. (std's is_unique_local/is_unicast_link_local
    // are still unstable, so match the prefixes on the leading segment.)
    if let Ok(v6) = host.parse::<std::net::Ipv6Addr>() {
        if v6.is_loopback() || v6.is_unspecified() {
            return false;
        }
        let first = v6.segments()[0];
        let is_unique_local = (first & 0xfe00) == 0xfc00; // fc00::/7
        let is_link_local = (first & 0xffc0) == 0xfe80; // fe80::/10
        return !(is_unique_local || is_link_local);
    }
    // A non-IPv4, non-IPv6, non-local hostname (e.g. *.eks.amazonaws.com) is
    // public.
    true
}

/// Parse a dotted IPv4 string into four octets, or `None` if it is not one.
fn parse_ipv4(host: &str) -> Option<[u8; 4]> {
    let mut octets = [0u8; 4];
    let mut count = 0;
    for part in host.split('.') {
        if count == 4 {
            return None;
        }
        octets[count] = part.parse::<u8>().ok()?;
        count += 1;
    }
    if count == 4 {
        Some(octets)
    } else {
        None
    }
}

/// Classify a context. Pure and dependency-free so it can be exhaustively
/// unit-tested; the capability layer supplies the strings from the kubeconfig.
///
/// - `context_name` / `cluster_name`: the two names to match signatures against.
/// - `server`: the cluster's API server URL (used only to demote).
/// - `exec_command`: the user's `exec` auth plugin command, if any.
/// - `auth_provider`: the user's legacy `auth-provider` name, if any.
pub fn classify(
    context_name: &str,
    cluster_name: &str,
    server: &str,
    exec_command: Option<&str>,
    auth_provider: Option<&str>,
) -> Classification {
    let provider = provider_from_name(context_name).or_else(|| provider_from_name(cluster_name));
    let Some(provider) = provider else {
        // No local signature -> remote. We never promote from address alone.
        return Classification::remote();
    };
    // Hard gate: cloud auth wins over any name.
    if is_cloud_auth(exec_command, auth_provider) {
        return Classification::remote();
    }
    // The address may only demote: a public host cancels the local guess.
    if host_is_public(server) {
        return Classification::remote();
    }
    Classification {
        is_local: true,
        provider: Some(provider),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(name: &str, server: &str) -> Classification {
        classify(name, name, server, None, None)
    }

    #[test]
    fn recognises_each_local_provider_by_name() {
        let cases: &[(&str, &str, LocalProvider)] = &[
            ("kind-dev", "https://127.0.0.1:6443", LocalProvider::Kind),
            ("k3d-demo", "https://0.0.0.0:52000", LocalProvider::K3d),
            (
                "minikube",
                "https://192.168.49.2:8443",
                LocalProvider::Minikube,
            ),
            (
                "microk8s",
                "https://127.0.0.1:16443",
                LocalProvider::Microk8s,
            ),
            (
                "docker-desktop",
                "https://kubernetes.docker.internal:6443",
                LocalProvider::DockerDesktop,
            ),
            (
                "docker-for-desktop",
                "https://localhost:6443",
                LocalProvider::DockerDesktop,
            ),
            (
                "rancher-desktop",
                "https://127.0.0.1:6443",
                LocalProvider::RancherDesktop,
            ),
            ("colima", "https://127.0.0.1:6443", LocalProvider::Colima),
            (
                "colima-work",
                "https://127.0.0.1:6443",
                LocalProvider::Colima,
            ),
            (
                "orbstack",
                "https://127.0.0.1:26443",
                LocalProvider::Orbstack,
            ),
            (
                "kiac-demo",
                "https://192.168.64.5:6443",
                LocalProvider::Kiac,
            ),
            (
                "vcluster-docker_demo",
                "https://localhost:13755",
                LocalProvider::Vind,
            ),
        ];
        for (name, server, want) in cases {
            let got = local(name, server);
            assert!(got.is_local, "{name} should be local");
            assert_eq!(got.provider, Some(*want), "{name} provider");
        }
    }

    #[test]
    fn unknown_names_default_to_remote() {
        // On-prem prod on a private IP, but no local signature -> remote.
        assert_eq!(
            local("production", "https://10.0.0.5:6443"),
            Classification::remote()
        );
        assert_eq!(
            local("prod-eu", "https://127.0.0.1:6443"),
            Classification::remote()
        );
        assert_eq!(local("", ""), Classification::remote());
    }

    #[test]
    fn cloud_auth_forces_remote_even_with_local_name() {
        // A port-forwarded EKS reachable on localhost, but auth is `aws`.
        let got = classify(
            "kind-tunnel",
            "kind-tunnel",
            "https://127.0.0.1:6443",
            Some("aws"),
            None,
        );
        assert_eq!(got, Classification::remote());
        // Absolute exec path is compared on its basename.
        let got = classify(
            "kind-tunnel",
            "kind-tunnel",
            "https://127.0.0.1:6443",
            Some("/opt/homebrew/bin/gke-gcloud-auth-plugin"),
            None,
        );
        assert_eq!(got, Classification::remote());
        // Legacy auth-provider (gcp/oidc) also gates.
        let got = classify(
            "kind-x",
            "kind-x",
            "https://localhost:6443",
            None,
            Some("gcp"),
        );
        assert_eq!(got, Classification::remote());
        // Windows kubeconfigs: `.exe` suffix and mixed case still gate.
        for cmd in ["aws.exe", "AWS", "C:\\bin\\Az.EXE", "gke-gcloud-auth-plugin.exe"] {
            let got = classify("kind-win", "kind-win", "https://127.0.0.1:6443", Some(cmd), None);
            assert_eq!(got, Classification::remote(), "exec {cmd} should gate to remote");
        }
        let got = classify("kind-x", "kind-x", "https://localhost:6443", None, Some("GCP"));
        assert_eq!(got, Classification::remote());
    }

    #[test]
    fn public_host_demotes_a_local_looking_name() {
        // A local-looking name pointed at a public address is cancelled.
        let got = local("kind-weird", "https://34.120.1.2:443");
        assert_eq!(got, Classification::remote());
        let got = local("kind-weird", "https://my-cluster.eks.amazonaws.com");
        assert_eq!(got, Classification::remote());
    }

    #[test]
    fn helm_or_platform_vcluster_stays_remote() {
        // Only the docker driver (`vcluster-docker_`) is local. A tenant cluster
        // in a host cluster is `vcluster_<name>_<ns>_<host-context>`.
        assert_eq!(
            local("vcluster_demo_default_prod-eks", "https://localhost:8443"),
            Classification::remote()
        );
    }

    #[test]
    fn matches_on_cluster_name_when_context_is_renamed() {
        // Context renamed, but the cluster still carries the signature.
        let got = classify("my-alias", "kind-dev", "https://127.0.0.1:6443", None, None);
        assert!(got.is_local);
        assert_eq!(got.provider, Some(LocalProvider::Kind));
    }

    #[test]
    fn ipv6_loopback_is_local_public_ipv6_is_not() {
        assert!(local("kind-v6", "https://[::1]:6443").is_local);
        assert_eq!(
            local("kind-v6", "https://[2001:db8::1]:6443"),
            Classification::remote()
        );
    }

    #[test]
    fn ipv6_private_ranges_are_not_public() {
        // Unique-local (fc00::/7) and link-local (fe80::/10) mirror the IPv4
        // private ranges: a local-looking name over them stays local.
        assert!(local("kind-ula", "https://[fd12:3456::1]:6443").is_local);
        assert!(local("kind-ll", "https://[fe80::1]:6443").is_local);
        // A global-unicast v6 address still demotes a local-looking name.
        assert_eq!(
            local("kind-gua", "https://[2606:4700::1111]:6443"),
            Classification::remote()
        );
    }

    #[test]
    fn host_parsing_handles_ports_paths_and_schemes() {
        assert_eq!(host_of("https://127.0.0.1:6443"), "127.0.0.1");
        assert_eq!(
            host_of("https://kubernetes.docker.internal:6443/foo"),
            "kubernetes.docker.internal"
        );
        assert_eq!(host_of("[::1]:6443"), "::1");
        assert_eq!(host_of("localhost"), "localhost");
    }
}
