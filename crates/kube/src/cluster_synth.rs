//! Build a one-context kubeconfig from the srelens "Add cluster" UI form, so a
//! UI-defined cluster is stored and materialised exactly like an uploaded
//! kubeconfig.

use base64::Engine;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use srelens_capability::{Annotations, Capability, CapabilityError};

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
        // Emit the `exec` kubelogin form (not the legacy `auth-provider: oidc`
        // block). Desktop runs kubelogin natively to authenticate; web detects
        // this same form, strips the exec block, and injects a srelens-managed
        // token — so one synthesized kubeconfig works on both surfaces.
        let mut args: Vec<serde_yaml::Value> = vec![
            "get-token".into(),
            format!("--oidc-issuer-url={}", oidc.issuer).into(),
            format!("--oidc-client-id={}", oidc.client_id).into(),
        ];
        if let Some(secret) = &oidc.client_secret {
            args.push(format!("--oidc-client-secret={secret}").into());
        }
        for scope in &oidc.extra_scopes {
            args.push(format!("--oidc-extra-scope={scope}").into());
        }
        let mut exec = serde_yaml::Mapping::new();
        exec.insert(
            "apiVersion".into(),
            "client.authentication.k8s.io/v1beta1".into(),
        );
        exec.insert("command".into(), "kubelogin".into());
        exec.insert("args".into(), serde_yaml::Value::Sequence(args));
        exec.insert("interactiveMode".into(), "IfAvailable".into());
        user.insert("exec".into(), serde_yaml::Value::Mapping(exec));
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

/// Add-cluster form fields (camelCase to match the frontend). Deserializable so
/// this is usable directly as a capability input on both desktop and web.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SynthesizeClusterIn {
    pub name: String,
    pub server: String,
    #[serde(default)]
    pub ca_cert_pem: Option<String>,
    #[serde(default)]
    pub insecure_skip_tls_verify: bool,
    #[serde(default)]
    pub oidc: Option<OidcFormIn>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OidcFormIn {
    pub issuer: String,
    pub client_id: String,
    #[serde(default)]
    pub client_secret: Option<String>,
    #[serde(default)]
    pub extra_scopes: Option<Vec<String>>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SynthesizeClusterOut {
    pub yaml: String,
}

/// `k8s.synthesizeClusterKubeconfig` — turn the Add-cluster form fields into a
/// one-context kubeconfig YAML. Pure (no cluster contact); the caller then
/// stores the YAML like any other kubeconfig. Shared by desktop and web so both
/// produce byte-identical output.
pub fn synthesize_cluster_capability() -> Capability {
    Capability::typed::<SynthesizeClusterIn, SynthesizeClusterOut, _, _>(
        "k8s.synthesizeClusterKubeconfig",
        "synthesize a one-context kubeconfig from Add-cluster form fields",
        Annotations::READ_ONLY,
        move |input: SynthesizeClusterIn| async move {
            let form = ClusterForm {
                name: input.name,
                server: input.server,
                ca_cert_pem: input.ca_cert_pem,
                insecure_skip_tls_verify: input.insecure_skip_tls_verify,
                oidc: input.oidc.map(|o| OidcClusterConfig {
                    issuer: o.issuer,
                    client_id: o.client_id,
                    client_secret: o.client_secret,
                    extra_scopes: o.extra_scopes.unwrap_or_default(),
                }),
            };
            let yaml = synthesize_kubeconfig(&form).map_err(CapabilityError::Handler)?;
            Ok(SynthesizeClusterOut { yaml })
        },
    )
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
        // Uses the exec kubelogin form so native desktop auth works.
        assert!(yaml.contains("command: kubelogin"));
        let kc = Kubeconfig::from_yaml(&yaml).expect("kube parses it");
        assert_eq!(kc.contexts[0].name, "prod");
        // The detector recovers the OIDC config from the exec block.
        let auth = kc.auth_infos[0].auth_info.clone().unwrap();
        let got = detect_oidc_user(&auth).unwrap();
        assert_eq!(got.issuer, "https://dex");
        assert_eq!(got.client_id, "k8s");
        assert_eq!(got.extra_scopes, vec!["groups".to_string()]);
    }

    #[tokio::test]
    async fn synthesize_capability_emits_exec_form() {
        let cap = synthesize_cluster_capability();
        let out = (cap.handler)(serde_json::json!({
            "name": "prod",
            "server": "https://api:6443",
            "insecureSkipTlsVerify": true,
            "oidc": { "issuer": "https://dex", "clientId": "k8s", "extraScopes": ["groups"] }
        }))
        .await
        .unwrap();
        let yaml = out["yaml"].as_str().unwrap();
        assert!(yaml.contains("command: kubelogin"));
        assert!(yaml.contains("--oidc-issuer-url=https://dex"));
        assert!(yaml.contains("--oidc-extra-scope=groups"));
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
