//! Precision-first Kubernetes distribution classification for the cluster rail.
//!
//! This is presentation metadata, not an authority decision. False positives
//! are more damaging than an unclassified cluster: a context somebody happened
//! to call `my-eks-lab` must not be presented as AWS. We therefore accept only
//! provider-generated authentication or API endpoint signatures, and leave
//! everything else unclassified.

use crate::local_cluster::LocalProvider;

/// The distribution label sent to the UI, when the kubeconfig supplies enough
/// evidence to name one. Local providers already have precise classification,
/// so they use the same stable wire name here.
pub(crate) fn classify_distribution(
    local: Option<LocalProvider>,
    context_name: &str,
    cluster_name: &str,
    server: &str,
    exec_command: Option<&str>,
    auth_provider: Option<&str>,
) -> Option<String> {
    if let Some(provider) = local {
        return Some(provider.as_str().to_string());
    }

    let command = exec_command.map(command_basename).unwrap_or_default();
    let auth = auth_provider
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let host = host_of(server).to_ascii_lowercase();
    let context = context_name.trim().to_ascii_lowercase();
    let cluster = cluster_name.trim().to_ascii_lowercase();

    // `gke-gcloud-auth-plugin` exists specifically for GKE. The legacy `gcp`
    // auth-provider and plain `gcloud` are reusable credentials, so they only
    // count beside a name the GKE CLI itself emits. The current DNS suffix is
    // direct evidence.
    let generated_gke_name = context.starts_with("gke_") || cluster.starts_with("gke_");
    if command == "gke-gcloud-auth-plugin"
        || host == "gke.goog"
        || host.ends_with(".gke.goog")
        || ((auth == "gcp" || command == "gcloud") && generated_gke_name)
    {
        return Some("gke".to_string());
    }

    // EKS cluster endpoints and context ARNs are provider-generated. `aws`
    // alone is not: it is a general CLI and may back a custom exec credential.
    let aws_auth = matches!(
        command.as_str(),
        "aws" | "aws-iam-authenticator" | "heptio-authenticator-aws"
    );
    if amazon_eks_host(&host, aws_auth)
        || (aws_auth
            && (context.starts_with("arn:aws:eks:") || cluster.starts_with("arn:aws:eks:")))
    {
        return Some("eks".to_string());
    }

    // Public, private-link and sovereign AKS endpoints retain an Azure-owned
    // suffix. `kubelogin` alone is an OIDC client and therefore not proof.
    if azure_aks_host(&host) {
        return Some("aks".to_string());
    }

    None
}

fn command_basename(command: &str) -> String {
    let base = command
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(command)
        .trim()
        .to_ascii_lowercase();
    base.strip_suffix(".exe").unwrap_or(&base).to_string()
}

/// Extract the host from a kubeconfig server URL without accepting user-info,
/// paths or query text as provider evidence. `http::Uri` is already a direct
/// dependency of this crate and handles bracketed IPv6 and explicit ports.
fn host_of(server: &str) -> String {
    server
        .parse::<http::Uri>()
        .ok()
        .and_then(|uri| uri.host().map(str::to_string))
        .unwrap_or_default()
        .trim_end_matches('.')
        .to_string()
}

fn has_dns_suffix(host: &str, suffix: &str) -> bool {
    host == suffix
        || host
            .strip_suffix(suffix)
            .is_some_and(|prefix| prefix.ends_with('.'))
}

fn amazon_eks_host(host: &str, aws_auth: bool) -> bool {
    // The classic AWS/GovCloud endpoint has `eks` in the owned suffix and is
    // conclusive on its own. Dual-stack `api.aws` and China's
    // `amazonwebservices.com.cn` are shared-looking domains, so they count only
    // with the AWS exec authenticator the generated kubeconfig also carries.
    has_dns_suffix(host, "eks.amazonaws.com")
        || (aws_auth
            && ["api.aws", "amazonwebservices.com.cn"]
                .iter()
                .any(|suffix| has_dns_suffix(host, suffix)))
}

fn azure_aks_host(host: &str) -> bool {
    [
        "azmk8s.io",
        "cx.aks.containerservice.azure.us",
        "cx.prod.service.azk8s.cn",
    ]
    .iter()
    .any(|suffix| has_dns_suffix(host, suffix))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote(
        context: &str,
        cluster: &str,
        server: &str,
        command: Option<&str>,
        auth: Option<&str>,
    ) -> Option<String> {
        classify_distribution(None, context, cluster, server, command, auth)
    }

    #[test]
    fn exact_managed_provider_signatures_are_named() {
        assert_eq!(
            remote(
                "gke_acme_eu_prod",
                "prod",
                "https://34.1.2.3",
                Some("C:\\tools\\GKE-GCLOUD-AUTH-PLUGIN.EXE"),
                None
            )
            .as_deref(),
            Some("gke")
        );
        assert_eq!(
            remote(
                "prod",
                "prod",
                "https://abc.gr7.eu-west-1.eks.amazonaws.com",
                Some("aws"),
                None
            )
            .as_deref(),
            Some("eks")
        );
        assert_eq!(
            remote(
                "prod-v6",
                "prod-v6",
                "https://eks-cluster.us-west-2.api.aws",
                Some("aws"),
                None
            )
            .as_deref(),
            Some("eks")
        );
        assert_eq!(
            remote(
                "prod-cn",
                "prod-cn",
                "https://eks-cluster.cn-north-1.amazonwebservices.com.cn",
                Some("aws"),
                None
            )
            .as_deref(),
            Some("eks")
        );
        assert_eq!(
            remote(
                "prod",
                "prod",
                "https://private.prod.hcp.westeurope.azmk8s.io:443",
                Some("kubelogin"),
                None
            )
            .as_deref(),
            Some("aks")
        );
        assert_eq!(
            remote(
                "gov-prod",
                "gov-prod",
                "https://cluster.hcp.usgovvirginia.cx.aks.containerservice.azure.us",
                Some("kubelogin"),
                None
            )
            .as_deref(),
            Some("aks")
        );
        assert_eq!(
            remote(
                "china-prod",
                "china-prod",
                "https://cluster.hcp.chinanorth3.cx.prod.service.azk8s.cn",
                Some("kubelogin"),
                None
            )
            .as_deref(),
            Some("aks")
        );
    }

    #[test]
    fn loose_names_and_generic_auth_stay_unclassified() {
        assert_eq!(
            remote(
                "my-eks-lab",
                "cluster",
                "https://cluster.example",
                None,
                None
            ),
            None
        );
        assert_eq!(
            remote(
                "prod",
                "cluster",
                "https://fake.eks.amazonaws.com.attacker.example",
                Some("aws"),
                None
            ),
            None
        );
        assert_eq!(
            remote(
                "prod",
                "cluster",
                "https://packages.azk8s.cn",
                Some("kubelogin"),
                None
            ),
            None,
            "an Azure artifact host is not an AKS API endpoint"
        );
        assert_eq!(
            remote(
                "prod",
                "cluster",
                "https://something.us-west-2.api.aws",
                None,
                None
            ),
            None,
            "the shared dual-stack suffix needs AWS authentication evidence"
        );
        assert_eq!(
            remote(
                "prod",
                "cluster",
                "https://cluster.example",
                Some("kubelogin"),
                None
            ),
            None
        );
        assert_eq!(
            remote(
                "bare-metal",
                "bare-metal",
                "https://cluster.example",
                None,
                Some("gcp")
            ),
            None,
            "a reusable legacy credential is not proof that the API server is GKE"
        );
        assert_eq!(
            remote(
                "prod",
                "cluster",
                "https://cluster.example",
                None,
                Some("oidc")
            ),
            None
        );
        assert_eq!(
            remote(
                "prod",
                "cluster",
                "https://cluster.example",
                Some("aws"),
                None
            ),
            None
        );
    }

    #[test]
    fn a_known_local_provider_keeps_its_precise_name() {
        assert_eq!(
            classify_distribution(
                Some(LocalProvider::Kind),
                "anything",
                "anything",
                "https://example",
                None,
                None,
            )
            .as_deref(),
            Some("kind"),
        );
    }
}
