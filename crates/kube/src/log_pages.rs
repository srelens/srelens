//! Page mode of `k8s.podLogs`: a log far bigger than one tool answer, read in
//! pages that each fit under the MCP server's cap, filtered on the server,
//! followed for a bounded time, and merged across every pod of a selector or
//! workload.
//!
//! A page streams each log from the API server and stops reading as soon as it
//! is full, so no log is held whole. Where it stopped is the cursor: per pod,
//! the last line read, as the kubelet stamped it, and how many lines carried
//! that stamp. The API server hands `sinceTime` to the kubelet in whole
//! seconds, so a stamp alone cannot resume mid-second.

use schemars::JsonSchema;
use serde::Serialize;

/// Where a page-mode read stopped, and what happened to the logs on the way.
#[derive(Debug, Serialize, JsonSchema)]
pub struct Page {
    /// Pass back as `cursor` to read on, or with `wait_seconds` to follow.
    pub next_cursor: String,
    /// Every log was read to its end, as of this call. `false`: call again
    /// for more.
    pub end: bool,
    /// Lines in `logs`.
    pub lines: usize,
    /// Lines read to build this page: more than `lines` when `grep` skipped
    /// some.
    pub scanned: usize,
    /// What happened under the cursor, one sentence each, naming the pod: a
    /// rotation gap, a restart crossed, a pod replaced, gone or new, a read
    /// that failed.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notices: Vec<String>,
}
