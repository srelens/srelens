//! Detect a kubeconfig user's OIDC settings so srelens can run the flow itself
//! (Headlamp-style) instead of relying on a `kubelogin`/`oidc-login` exec
//! plugin that can't run headlessly. Recognises the legacy
//! `auth-provider: oidc` block and the `exec` kubelogin plugin's args.

use sha2::{Digest, Sha256};

/// The OIDC settings needed to run the authorization-code + PKCE flow for a
/// cluster. `client_secret` is optional (public clients use PKCE alone).
#[derive(Debug, Clone, PartialEq)]
pub struct OidcClusterConfig {
    pub issuer: String,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub extra_scopes: Vec<String>,
}

/// Stable key grouping every context that shares an issuer+client, so one
/// sign-in serves them all. `hex(sha256(issuer + "|" + client_id))`.
pub fn oidc_key(issuer: &str, client_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(issuer.as_bytes());
    hasher.update(b"|");
    hasher.update(client_id.as_bytes());
    hex::encode(hasher.finalize())
}

impl OidcClusterConfig {
    pub fn key(&self) -> String {
        oidc_key(&self.issuer, &self.client_id)
    }
}

/// The final path component of an exec command, lower-cased and with a trailing
/// `.exe` removed (Windows kubeconfigs). Handles both `/` and `\` separators.
fn command_base(command: &str) -> String {
    let base = command.rsplit(['/', '\\']).next().unwrap_or(command);
    base.strip_suffix(".exe")
        .unwrap_or(base)
        .to_ascii_lowercase()
}

/// True when a kubeconfig user's `exec` command is a known OIDC login helper
/// invoked directly (kubelogin / kubectl-oidc_login / oidc-login).
fn is_kubelogin(command: &str) -> bool {
    matches!(
        command_base(command).as_str(),
        "kubelogin" | "kubectl-oidc_login" | "oidc-login" | "kubectl-oidc-login"
    )
}

/// True when the exec command is `kubectl` (the OIDC helper may be invoked as
/// the `kubectl oidc-login` plugin subcommand).
fn is_kubectl(command: &str) -> bool {
    command_base(command) == "kubectl"
}

/// Pull `--flag value` or `--flag=value` for `flag` from an exec arg list.
fn exec_flag(args: &[String], flag: &str) -> Option<String> {
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if let Some(v) = a.strip_prefix(&format!("{flag}=")) {
            return Some(v.to_string());
        }
        if a == flag {
            return args.get(i + 1).cloned();
        }
        i += 1;
    }
    None
}

/// Detect OIDC settings from a kubeconfig user, or None if it isn't OIDC.
pub fn detect_oidc_user(auth: &kube::config::AuthInfo) -> Option<OidcClusterConfig> {
    // 1) Legacy auth-provider: oidc
    if let Some(ap) = &auth.auth_provider {
        if ap.name == "oidc" {
            let issuer = ap.config.get("idp-issuer-url")?.clone();
            let client_id = ap.config.get("client-id")?.clone();
            let client_secret = ap
                .config
                .get("client-secret")
                .filter(|s| !s.is_empty())
                .cloned();
            let extra_scopes = ap
                .config
                .get("extra-scopes")
                .map(|s| {
                    s.split(',')
                        .map(|x| x.trim().to_string())
                        .filter(|x| !x.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            return Some(OidcClusterConfig {
                issuer,
                client_id,
                client_secret,
                extra_scopes,
            });
        }
    }
    // 2) exec kubelogin/oidc-login, either as a direct binary or invoked as the
    //    `kubectl oidc-login` plugin subcommand.
    if let Some(exec) = &auth.exec {
        let command = exec.command.as_deref()?;
        let args = exec.args.clone().unwrap_or_default();
        let is_oidc_login =
            is_kubelogin(command) || (is_kubectl(command) && args.iter().any(|a| a == "oidc-login"));
        if is_oidc_login {
            let issuer = exec_flag(&args, "--oidc-issuer-url")?;
            let client_id = exec_flag(&args, "--oidc-client-id")?;
            let client_secret =
                exec_flag(&args, "--oidc-client-secret").filter(|s| !s.is_empty());
            let extra_scopes: Vec<String> = args
                .iter()
                .enumerate()
                .filter_map(|(i, a)| {
                    a.strip_prefix("--oidc-extra-scope=")
                        .map(str::to_string)
                        .or_else(|| {
                            (a == "--oidc-extra-scope")
                                .then(|| args.get(i + 1).cloned())
                                .flatten()
                        })
                })
                .collect();
            return Some(OidcClusterConfig {
                issuer,
                client_id,
                client_secret,
                extra_scopes,
            });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use kube::config::Kubeconfig;

    fn auth_of(yaml: &str) -> kube::config::AuthInfo {
        let kc = Kubeconfig::from_yaml(yaml).unwrap();
        kc.auth_infos.into_iter().next().unwrap().auth_info.unwrap()
    }

    #[test]
    fn oidc_key_is_stable_and_issuer_client_specific() {
        assert_eq!(oidc_key("https://idp", "cid"), oidc_key("https://idp", "cid"));
        assert_ne!(oidc_key("https://idp", "cid"), oidc_key("https://idp", "other"));
    }

    #[test]
    fn detects_legacy_auth_provider_oidc() {
        let yaml = r#"apiVersion: v1
kind: Config
users:
- name: u
  user:
    auth-provider:
      name: oidc
      config:
        idp-issuer-url: https://dex.example.com
        client-id: k8s
        client-secret: shh
        extra-scopes: groups,email
"#;
        let got = detect_oidc_user(&auth_of(yaml)).unwrap();
        assert_eq!(
            got,
            OidcClusterConfig {
                issuer: "https://dex.example.com".into(),
                client_id: "k8s".into(),
                client_secret: Some("shh".into()),
                extra_scopes: vec!["groups".into(), "email".into()],
            }
        );
    }

    #[test]
    fn detects_exec_kubelogin_public_client() {
        let yaml = r#"apiVersion: v1
kind: Config
users:
- name: u
  user:
    exec:
      apiVersion: client.authentication.k8s.io/v1beta1
      command: kubelogin
      args:
      - get-token
      - --oidc-issuer-url=https://keycloak.example/realms/k8s
      - --oidc-client-id
      - kubernetes
      - --oidc-extra-scope=groups
"#;
        let got = detect_oidc_user(&auth_of(yaml)).unwrap();
        assert_eq!(got.issuer, "https://keycloak.example/realms/k8s");
        assert_eq!(got.client_id, "kubernetes");
        assert_eq!(got.client_secret, None);
        assert_eq!(got.extra_scopes, vec!["groups".to_string()]);
    }

    #[test]
    fn detects_kubectl_oidc_login_plugin_form() {
        // `kubectl oidc-login` — the plugin invoked via kubectl (command is
        // `kubectl`, first arg `oidc-login`).
        let yaml = r#"apiVersion: v1
kind: Config
users:
- name: u
  user:
    exec:
      apiVersion: client.authentication.k8s.io/v1beta1
      command: kubectl
      args:
      - oidc-login
      - get-token
      - --oidc-issuer-url=https://dex.example.com
      - --oidc-client-id=k8s
      - --oidc-extra-scope=groups
"#;
        let got = detect_oidc_user(&auth_of(yaml)).unwrap();
        assert_eq!(got.issuer, "https://dex.example.com");
        assert_eq!(got.client_id, "k8s");
        assert_eq!(got.extra_scopes, vec!["groups".to_string()]);
    }

    #[test]
    fn detects_windows_kubelogin_exe_and_path() {
        let yaml = r#"apiVersion: v1
kind: Config
users:
- name: u
  user:
    exec:
      apiVersion: client.authentication.k8s.io/v1beta1
      command: C:\tools\kubelogin.exe
      args:
      - get-token
      - --oidc-issuer-url=https://idp
      - --oidc-client-id=cid
"#;
        let got = detect_oidc_user(&auth_of(yaml)).unwrap();
        assert_eq!(got.issuer, "https://idp");
        assert_eq!(got.client_id, "cid");
    }

    #[test]
    fn plain_kubectl_exec_without_oidc_login_is_none() {
        // `kubectl` used as an exec plugin for something else must NOT match.
        let yaml = r#"apiVersion: v1
kind: Config
users:
- name: u
  user:
    exec:
      apiVersion: client.authentication.k8s.io/v1beta1
      command: kubectl
      args:
      - config
      - view
"#;
        assert!(detect_oidc_user(&auth_of(yaml)).is_none());
    }

    #[test]
    fn azure_kubelogin_without_oidc_flags_is_none() {
        // Azure kubelogin (same binary name, different flags) is a different
        // auth flow srelens can't manage — must not be mis-detected as OIDC.
        let yaml = r#"apiVersion: v1
kind: Config
users:
- name: u
  user:
    exec:
      apiVersion: client.authentication.k8s.io/v1beta1
      command: kubelogin
      args:
      - get-token
      - --server-id=abc
      - --client-id=def
      - --tenant-id=ghi
      - --login=azurecli
"#;
        assert!(detect_oidc_user(&auth_of(yaml)).is_none());
    }

    #[test]
    fn non_oidc_auth_is_none() {
        let yaml = r#"apiVersion: v1
kind: Config
users:
- name: u
  user:
    token: static-bearer
"#;
        assert!(detect_oidc_user(&auth_of(yaml)).is_none());
    }
}
