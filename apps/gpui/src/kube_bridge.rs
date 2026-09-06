//! The one place this binary touches the cluster.
//!
//! Two facts shape it. **The service layer is already Rust**: `crates/kube`
//! owns the kube client, kubeconfig discovery and every summary type, and it
//! now exposes typed doors — `list_contexts`, `list_namespaces`, `list_pods`
//! — beside the JSON capabilities the registry serves. This shell calls the
//! doors. There is no `packages/core` here and no `Value` in the middle,
//! which is the claim the feasibility made and the thing this file exists to
//! test.
//!
//! **kube-rs needs a tokio reactor and GPUI has its own executor.** The two
//! do not share a runtime. So one tokio runtime is built at startup and every
//! kube future is spawned onto it; what comes back to GPUI is the
//! `JoinHandle`, which is an ordinary `Future` that any executor can await.
//! State is then mutated on GPUI's foreground through the normal `cx.spawn` /
//! `this.update` path — never from a tokio thread.
//!
//! Held as a GPUI global because it is an application service with one owner
//! and one lifetime, which is what the coding guide reserves globals for.

use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::Global;
use srelens_capability::CapabilityError;
use srelens_kube::client_cache::ClientCache;
use srelens_kube::contexts::{self, ContextDto};
use srelens_kube::workloads::{self, PodSummary};

pub struct KubeBridge {
    runtime: tokio::runtime::Runtime,
    cache: Arc<ClientCache>,
    default_paths: Vec<PathBuf>,
}

impl Global for KubeBridge {}

impl KubeBridge {
    /// Build the runtime and the client cache over the host's default
    /// kubeconfig paths — the same discovery the desktop and MCP surfaces use.
    pub fn new() -> std::io::Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(2)
            .thread_name("srelens-kube")
            .build()?;
        let default_paths = srelens_registry::default_kubeconfig_paths();
        let cache = ClientCache::new_many(default_paths.clone());
        Ok(Self {
            runtime,
            cache,
            default_paths,
        })
    }

    /// Every context across the active kubeconfig files.
    pub fn contexts(&self) -> impl Future<Output = Result<Vec<ContextDto>, String>> + Send {
        let cache = self.cache.clone();
        let paths = self.default_paths.clone();
        self.run(async move {
            contexts::list_contexts(&cache, paths, None, None)
                .await
                .map(|out| out.contexts)
        })
    }

    /// Namespace names in one context.
    pub fn namespaces(
        &self,
        context: String,
    ) -> impl Future<Output = Result<Vec<String>, String>> + Send {
        let cache = self.cache.clone();
        self.run(async move {
            workloads::list_namespaces(&cache, &context)
                .await
                .map(|out| out.namespaces)
        })
    }

    /// Pods in one namespace of one context; `""` for every namespace.
    pub fn pods(
        &self,
        context: String,
        namespace: String,
    ) -> impl Future<Output = Result<Vec<PodSummary>, String>> + Send {
        let cache = self.cache.clone();
        self.run(async move {
            workloads::list_pods(&cache, &context, &namespace)
                .await
                .map(|out| out.pods)
        })
    }

    /// Run a kube future on the tokio runtime and hand its result back as a
    /// future any executor can await. Both failures — the call's own, and the
    /// task being cancelled or panicking — arrive as one string the screen can
    /// show; a screen has nothing different to do about the two.
    fn run<T>(
        &self,
        work: impl Future<Output = Result<T, CapabilityError>> + Send + 'static,
    ) -> impl Future<Output = Result<T, String>> + Send
    where
        T: Send + 'static,
    {
        let join = self.runtime.spawn(work);
        async move {
            match join.await {
                Ok(Ok(value)) => Ok(value),
                Ok(Err(error)) => Err(describe(error)),
                Err(join_error) => Err(format!("the request was interrupted: {join_error}")),
            }
        }
    }
}

/// The message a reader sees for a capability failure: the error's own
/// `Display`, which `crates/capability` writes to carry the backend's words.
fn describe(error: CapabilityError) -> String {
    error.to_string()
}
