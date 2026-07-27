//! Per-user index of OIDC clusters: which contexts are OIDC, their oidc_key,
//! and the OidcClusterConfig (issuer/client/scopes) needed to run/refresh the
//! flow. Built from the user's decrypted kubeconfigs.

use std::collections::HashMap;

use srelens_kube::oidc_detect::{detect_oidc_user, OidcClusterConfig};

#[derive(Default)]
pub struct ClusterOidcRegistry {
    by_key: HashMap<String, OidcClusterConfig>,
    key_by_context: HashMap<String, String>,
}

impl ClusterOidcRegistry {
    /// Build from a user's kubeconfig YAML documents. Contexts whose user is
    /// OIDC are indexed; non-OIDC contexts are ignored. Malformed YAML
    /// documents are skipped rather than aborting the whole build.
    pub fn from_kubeconfig_yamls(yamls: &[String]) -> Self {
        let mut reg = ClusterOidcRegistry::default();
        for yaml in yamls {
            let Ok(kc) = kube::config::Kubeconfig::from_yaml(yaml) else {
                continue;
            };
            // Map user-name -> detected OIDC config.
            let mut oidc_users: HashMap<String, OidcClusterConfig> = HashMap::new();
            for named in &kc.auth_infos {
                if let Some(auth) = &named.auth_info {
                    if let Some(cfg) = detect_oidc_user(auth) {
                        oidc_users.insert(named.name.clone(), cfg);
                    }
                }
            }
            // A context is OIDC if its user is OIDC.
            for ctx in &kc.contexts {
                if let Some(c) = &ctx.context {
                    if let Some(cfg) = oidc_users.get(&c.user) {
                        let key = cfg.key();
                        reg.by_key.entry(key.clone()).or_insert_with(|| cfg.clone());
                        reg.key_by_context.insert(ctx.name.clone(), key);
                    }
                }
            }
        }
        reg
    }

    pub fn config_for_key(&self, oidc_key: &str) -> Option<OidcClusterConfig> {
        self.by_key.get(oidc_key).cloned()
    }

    pub fn key_for_context(&self, context: &str) -> Option<String> {
        self.key_by_context.get(context).cloned()
    }

    pub fn all_keys(&self) -> Vec<String> {
        self.by_key.keys().cloned().collect()
    }

    /// Every OIDC cluster: its key, config, and the contexts that use it.
    pub fn oidc_clusters(&self) -> Vec<(String, OidcClusterConfig, Vec<String>)> {
        let mut out: Vec<(String, OidcClusterConfig, Vec<String>)> = self
            .by_key
            .iter()
            .map(|(key, cfg)| {
                let mut contexts: Vec<String> = self
                    .key_by_context
                    .iter()
                    .filter(|(_, k)| *k == key)
                    .map(|(ctx, _)| ctx.clone())
                    .collect();
                contexts.sort();
                (key.clone(), cfg.clone(), contexts)
            })
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0)); // stable order for the UI
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oidc_kc(ctx: &str, issuer: &str, client: &str) -> String {
        format!(
            "apiVersion: v1\nkind: Config\nclusters:\n- name: c\n  cluster: {{server: https://x}}\nusers:\n- name: u\n  user:\n    auth-provider:\n      name: oidc\n      config:\n        idp-issuer-url: {issuer}\n        client-id: {client}\ncontexts:\n- name: {ctx}\n  context: {{cluster: c, user: u}}\n"
        )
    }

    #[test]
    fn indexes_oidc_contexts_and_dedups_by_key() {
        let a = oidc_kc("ctx-a", "https://idp", "k8s");
        let b = oidc_kc("ctx-b", "https://idp", "k8s"); // same issuer+client → same key
        let reg = ClusterOidcRegistry::from_kubeconfig_yamls(&[a, b]);
        let ka = reg.key_for_context("ctx-a").unwrap();
        let kb = reg.key_for_context("ctx-b").unwrap();
        assert_eq!(ka, kb); // shared key
        assert_eq!(reg.all_keys().len(), 1);
        let cfg = reg.config_for_key(&ka).unwrap();
        assert_eq!(cfg.issuer, "https://idp");
        assert_eq!(cfg.client_id, "k8s");
    }

    #[test]
    fn oidc_clusters_dedups_by_key_and_lists_all_contexts() {
        let a = oidc_kc("ctx-a", "https://idp", "k8s");
        let b = oidc_kc("ctx-b", "https://idp", "k8s"); // same issuer+client → same key
        let reg = ClusterOidcRegistry::from_kubeconfig_yamls(&[a, b]);
        let entries = reg.oidc_clusters();
        assert_eq!(entries.len(), 1);
        let (key, cfg, contexts) = &entries[0];
        assert_eq!(key, &reg.key_for_context("ctx-a").unwrap());
        assert_eq!(cfg.issuer, "https://idp");
        assert_eq!(cfg.client_id, "k8s");
        assert_eq!(contexts, &vec!["ctx-a".to_string(), "ctx-b".to_string()]);
    }

    #[test]
    fn non_oidc_context_is_absent() {
        let yaml = "apiVersion: v1\nkind: Config\nclusters:\n- name: c\n  cluster: {server: https://x}\nusers:\n- name: u\n  user: {token: static}\ncontexts:\n- name: ctx\n  context: {cluster: c, user: u}\n".to_string();
        let reg = ClusterOidcRegistry::from_kubeconfig_yamls(&[yaml]);
        assert!(reg.key_for_context("ctx").is_none());
        assert!(reg.all_keys().is_empty());
    }
}
