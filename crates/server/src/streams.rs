//! Per-user streaming managers: the six `srelens_streams` cores bound to one
//! user's client cache. Each user's `UserEnv` owns one bundle so watches,
//! logs, exec, port-forwards, terminals, and helm ops run against only that
//! user's clusters.

use std::sync::Arc;

use srelens_kube::client_cache::ClientCache;
use srelens_streams::exec::ExecManager;
use srelens_streams::forward::ForwardManager;
use srelens_streams::helm::HelmManager;
use srelens_streams::logs::LogStreamManager;
use srelens_streams::terminal::TerminalManager;
use srelens_streams::watch::WatchManager;

pub struct UserStreams {
    pub watch: WatchManager,
    pub logs: LogStreamManager,
    pub exec: ExecManager,
    pub forward: ForwardManager,
    pub terminal: TerminalManager,
    pub helm: HelmManager,
}

impl UserStreams {
    pub fn new(cache: Arc<ClientCache>) -> Self {
        Self {
            watch: WatchManager::new(cache.clone()),
            logs: LogStreamManager::new(cache.clone()),
            exec: ExecManager::new(cache.clone()),
            forward: ForwardManager::new(cache),
            terminal: TerminalManager::new(),
            helm: HelmManager::new(),
        }
    }
}

/// Abort every running stream/session across all six managers when a user's
/// environment is torn down (e.g. logout, eviction from the LRU cache of
/// active users), so no background task or child process outlives it.
impl Drop for UserStreams {
    fn drop(&mut self) {
        self.watch.shutdown_all();
        self.logs.shutdown_all();
        self.exec.shutdown_all();
        self.forward.shutdown_all();
        self.terminal.shutdown_all();
        self.helm.shutdown_all();
    }
}
