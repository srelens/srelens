//! Caches authenticated kube-rs clients per context so capabilities don't
//! re-parse the kubeconfig and rebuild TLS config on every invocation.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use kube::Client;
use tokio::sync::{Mutex, RwLock};

use crate::connect::build_client;

pub struct ClientCache {
    paths: RwLock<Vec<PathBuf>>,
    clients: Mutex<HashMap<String, Client>>,
}

impl ClientCache {
    pub fn new(path: PathBuf) -> Arc<Self> {
        Self::new_many(vec![path])
    }

    pub fn new_many(paths: Vec<PathBuf>) -> Arc<Self> {
        Arc::new(Self {
            paths: RwLock::new(paths),
            clients: Mutex::new(HashMap::new()),
        })
    }

    pub async fn set_paths(&self, paths: Vec<PathBuf>) {
        let mut current = self.paths.write().await;
        if *current == paths {
            return;
        }
        *current = paths;
        drop(current);
        self.clients.lock().await.clear();
    }

    pub async fn paths(&self) -> Vec<PathBuf> {
        self.paths.read().await.clone()
    }

    /// Return a cached client for `context`, building and caching one on a miss.
    pub async fn get(&self, context: &str) -> Result<Client, String> {
        if let Some(client) = self.clients.lock().await.get(context).cloned() {
            return Ok(client);
        }
        let paths = self.paths().await;
        let client = build_client(&paths, context).await?;
        self.clients
            .lock()
            .await
            .insert(context.to_string(), client.clone());
        Ok(client)
    }

    /// Drop any cached client for a context (e.g. after a connection failure).
    pub async fn invalidate(&self, context: &str) {
        self.clients.lock().await.remove(context);
    }

    /// Clear all cached clients (e.g. after a kubeconfig change).
    pub async fn clear(&self) {
        self.clients.lock().await.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn get_errors_for_unknown_context() {
        let dir = std::env::temp_dir();
        let path = dir.join("srelens-cache-test-kubeconfig.yaml");
        tokio::fs::write(
            &path,
            "clusters:\n  - name: a\n    cluster: { server: https://127.0.0.1:1 }\ncontexts:\n  - name: ctx-a\n    context: { cluster: a }\n",
        )
        .await
        .unwrap();

        let cache = ClientCache::new(path.clone());
        assert!(cache.get("does-not-exist").await.is_err());
        let _ = tokio::fs::remove_file(&path).await;
    }

    #[tokio::test]
    async fn invalidate_is_safe_on_empty_cache() {
        let cache = ClientCache::new(PathBuf::from("/x"));
        cache.invalidate("nope").await; // must not panic
    }
}
