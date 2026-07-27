//! Build a one-context kubeconfig from the srelens "Add cluster" UI form, so a
//! UI-defined cluster is stored and materialised exactly like an uploaded
//! kubeconfig.

use base64::Engine;

use crate::oidc_detect::OidcClusterConfig;

#[derive(Debug, Clone)]
pub struct ClusterForm {
    pub name: String,
    pub server: String,
    pub ca_cert_pem: Option<String>,
    pub insecure_skip_tls_verify: bool,
    pub oidc: Option<OidcClusterConfig>,
}

/// Produce a one-context kubeconfig YAML. Validates that the name is non-empty
/// and free of characters that would break YAML keys / path use.
pub fn synthesize_kubeconfig(form: &ClusterForm) -> Result<String, String> {
    let name = form.name.trim();
    if name.is_empty() {
        return Err("cluster name must not be empty".into());
    }
    if form.server.trim().is_empty() {
        return Err("server URL must not be empty".into());
    }

    let mut cluster = serde_yaml::Mapping::new();
    cluster.insert("server".into(), form.server.trim().into());
    if form.insecure_skip_tls_verify {
        cluster.insert("insecure-skip-tls-verify".into(), true.into());
    } else if let Some(pem) = &form.ca_cert_pem {
        let data = base64::engine::general_purpose::STANDARD.encode(pem.as_bytes());
        cluster.insert("certificate-authority-data".into(), data.into());
    }

    let mut user = serde_yaml::Mapping::new();
    if let Some(oidc) = &form.oidc {
        let mut cfg = serde_yaml::Mapping::new();
        cfg.insert("idp-issuer-url".into(), oidc.issuer.clone().into());
        cfg.insert("client-id".into(), oidc.client_id.clone().into());
        if let Some(secret) = &oidc.client_secret {
            cfg.insert("client-secret".into(), secret.clone().into());
        }
        if !oidc.extra_scopes.is_empty() {
            cfg.insert("extra-scopes".into(), oidc.extra_scopes.join(",").into());
        }
        let mut ap = serde_yaml::Mapping::new();
        ap.insert("name".into(), "oidc".into());
        ap.insert("config".into(), serde_yaml::Value::Mapping(cfg));
        user.insert("auth-provider".into(), serde_yaml::Value::Mapping(ap));
    }

    let named = |name: &str, inner_key: &str, inner: serde_yaml::Value| {
        let mut m = serde_yaml::Mapping::new();
        m.insert("name".into(), name.into());
        m.insert(inner_key.into(), inner);
        serde_yaml::Value::Sequence(vec![serde_yaml::Value::Mapping(m)])
    };
    let mut context_inner = serde_yaml::Mapping::new();
    context_inner.insert("cluster".into(), name.into());
    context_inner.insert("user".into(), name.into());

    let mut root = serde_yaml::Mapping::new();
    root.insert("apiVersion".into(), "v1".into());
    root.insert("kind".into(), "Config".into());
    root.insert("current-context".into(), name.into());
    root.insert(
        "clusters".into(),
        named(name, "cluster", serde_yaml::Value::Mapping(cluster)),
    );
    root.insert(
        "users".into(),
        named(name, "user", serde_yaml::Value::Mapping(user)),
    );
    root.insert(
        "contexts".into(),
        named(name, "context", serde_yaml::Value::Mapping(context_inner)),
    );
    let doc = serde_yaml::to_string(&serde_yaml::Value::Mapping(root)).map_err(|e| e.to_string())?;
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oidc_detect::{detect_oidc_user, OidcClusterConfig};
    use kube::config::Kubeconfig;

    #[test]
    fn synthesizes_a_parseable_oidc_kubeconfig() {
        let form = ClusterForm {
            name: "prod".into(),
            server: "https://api.prod:6443".into(),
            ca_cert_pem: Some(
                "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----".into(),
            ),
            insecure_skip_tls_verify: false,
            oidc: Some(OidcClusterConfig {
                issuer: "https://dex".into(),
                client_id: "k8s".into(),
                client_secret: None,
                extra_scopes: vec!["groups".into()],
            }),
        };
        let yaml = synthesize_kubeconfig(&form).unwrap();
        let kc = Kubeconfig::from_yaml(&yaml).expect("kube parses it");
        assert_eq!(kc.contexts[0].name, "prod");
        // The detector recovers the OIDC config we put in.
        let auth = kc.auth_infos[0].auth_info.clone().unwrap();
        let got = detect_oidc_user(&auth).unwrap();
        assert_eq!(got.issuer, "https://dex");
        assert_eq!(got.client_id, "k8s");
        assert_eq!(got.extra_scopes, vec!["groups".to_string()]);
    }

    #[test]
    fn skip_tls_and_name_validation() {
        let mut form = ClusterForm {
            name: "  ".into(),
            server: "https://x".into(),
            ca_cert_pem: None,
            insecure_skip_tls_verify: true,
            oidc: None,
        };
        assert!(synthesize_kubeconfig(&form).is_err()); // empty name
        form.name = "dev".into();
        let yaml = synthesize_kubeconfig(&form).unwrap();
        assert!(yaml.contains("insecure-skip-tls-verify: true"));
    }
}
