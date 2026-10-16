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

use std::collections::BTreeMap;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use srelens_capability::CapabilityError;

use crate::logs::{PodLogsIn, WorkloadRef};
use crate::workloads::LabelSelectorRequirement;

const CURSOR_VERSION: u32 = 1;
const AGAIN: &str = "call again without `cursor` to start a new read";

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

fn invalid(msg: impl Into<String>) -> CapabilityError {
    CapabilityError::InvalidInput(msg.into())
}

/// What a page-mode read covers.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Target {
    Pod(String),
    Selector {
        labels: BTreeMap<String, String>,
        expressions: Vec<LabelSelectorRequirement>,
    },
    Workload(WorkloadRef),
}

impl Target {
    /// One named pod: a failure to read it is the call's, not a notice.
    pub(crate) fn single(&self) -> bool {
        matches!(self, Target::Pod(_))
    }
}

/// Where a read stands, as a page hands it back in `next_cursor`. Short keys
/// keep a 20-pod cursor near 4 KB.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Cursor {
    v: u32,
    /// The read this cursor belongs to, as [`fingerprint`] spells it.
    t: String,
    /// Each pod's read, by pod name.
    p: BTreeMap<String, Pos>,
}

/// Where one pod's read stands.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Pos {
    /// The pod's uid: a pod of the same name with another uid was replaced.
    u: String,
    /// The container instance read, as its restart count.
    r: i32,
    /// The last line read, as the kubelet stamped it, or the read's start
    /// time before any line; absent for a read from the first line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    t: Option<String>,
    /// How many lines stamped `t` were read.
    #[serde(default, skip_serializing_if = "is_zero")]
    k: u32,
}

fn is_zero(k: &u32) -> bool {
    *k == 0
}

impl Cursor {
    pub(crate) fn encode(&self) -> String {
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(self).expect("a cursor serializes"))
    }

    /// The cursor `text` spells, if it belongs to the read `fingerprint` names.
    pub(crate) fn decode(text: &str, fingerprint: &str) -> Result<Self, CapabilityError> {
        #[derive(Deserialize)]
        struct Version {
            v: u32,
        }
        let garbled = || invalid(format!("`cursor` is not one srelens wrote; {AGAIN}"));
        let bytes = URL_SAFE_NO_PAD
            .decode(text.trim())
            .map_err(|_| garbled())?;
        let Version { v } = serde_json::from_slice(&bytes).map_err(|_| garbled())?;
        if v != CURSOR_VERSION {
            return Err(invalid(format!(
                "`cursor` is from another version of srelens; {AGAIN}"
            )));
        }
        let cursor: Cursor = serde_json::from_slice(&bytes).map_err(|_| garbled())?;
        if cursor.t != fingerprint {
            return Err(invalid(format!(
                "`cursor` belongs to another read (another pod, selector, workload, container or `previous`); {AGAIN}"
            )));
        }
        Ok(cursor)
    }
}

/// What a cursor belongs to: the context, namespace, target, container and
/// `previous` — not `grep` or the page fields, which may change mid-walk.
pub(crate) fn fingerprint(input: &PodLogsIn, target: &Target) -> String {
    let target = match target {
        Target::Pod(pod) => serde_json::json!({ "pod": pod }),
        Target::Selector {
            labels,
            expressions,
        } => serde_json::json!({
            "selector": labels,
            "matchExpressions": expressions
                .iter()
                .map(|e| serde_json::json!([e.key, e.operator, e.values]))
                .collect::<Vec<_>>(),
        }),
        Target::Workload(w) => serde_json::json!({ "workload": [w.kind, w.name] }),
    };
    let canonical = serde_json::json!([
        input.context,
        input.namespace,
        target,
        input.container,
        input.previous
    ])
    .to_string();
    hex::encode(&Sha256::digest(canonical.as_bytes())[..8])
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use serde_json::{json, Value};

    fn input(v: Value) -> PodLogsIn {
        serde_json::from_value(v).unwrap()
    }

    fn pod_read(pod: &str) -> (PodLogsIn, Target) {
        (
            input(json!({ "context": "c", "namespace": "n", "pod": pod })),
            Target::Pod(pod.into()),
        )
    }

    #[test]
    fn a_cursor_reads_back_as_it_was_written() {
        let (read, target) = pod_read("a");
        let fp = fingerprint(&read, &target);
        let cursor = Cursor {
            v: CURSOR_VERSION,
            t: fp.clone(),
            p: [(
                "a".to_string(),
                Pos {
                    u: "uid".into(),
                    r: 2,
                    t: Some("2026-10-16T10:00:00.5Z".into()),
                    k: 3,
                },
            )]
            .into(),
        };
        assert_eq!(Cursor::decode(&cursor.encode(), &fp).unwrap(), cursor);
    }

    /// A cursor taken from one read and handed to another would skip or
    /// repeat lines without a word, so it is refused, in one line saying what
    /// to do.
    #[test]
    fn a_cursor_from_another_read_is_refused() {
        let (read, target) = pod_read("a");
        let encoded = Cursor {
            v: CURSOR_VERSION,
            t: fingerprint(&read, &target),
            p: BTreeMap::new(),
        }
        .encode();
        for other in [
            json!({ "context": "c", "namespace": "n", "pod": "b" }),
            json!({ "context": "c", "namespace": "n", "pod": "a", "container": "sidecar" }),
            json!({ "context": "c", "namespace": "n", "pod": "a", "previous": true }),
            json!({ "context": "c", "namespace": "other", "pod": "a" }),
            json!({ "context": "d", "namespace": "n", "pod": "a" }),
        ] {
            let target = Target::Pod(other["pod"].as_str().unwrap().into());
            let err = Cursor::decode(&encoded, &fingerprint(&input(other.clone()), &target))
                .unwrap_err()
                .to_string();
            assert!(err.contains("another read"), "{other}: {err}");
            assert!(err.contains("without `cursor`"), "{err}");
            assert_eq!(err.lines().count(), 1, "{err}");
        }
    }

    #[test]
    fn a_garbled_cursor_and_one_from_another_version_are_refused() {
        let (read, target) = pod_read("a");
        let fp = fingerprint(&read, &target);
        for garbled in [
            String::new(),
            "not base64!".into(),
            URL_SAFE_NO_PAD.encode(b"{\"v\":1}"),
            URL_SAFE_NO_PAD.encode(b"[1,2]"),
        ] {
            let err = Cursor::decode(&garbled, &fp).unwrap_err().to_string();
            assert!(err.contains("not one srelens wrote"), "{garbled:?}: {err}");
        }
        let later = URL_SAFE_NO_PAD.encode(format!("{{\"v\":2,\"t\":\"{fp}\",\"p\":{{}}}}"));
        let err = Cursor::decode(&later, &fp).unwrap_err().to_string();
        assert!(err.contains("another version"), "{err}");
    }

    /// An agent may change its filter mid-walk: `grep` and the page fields
    /// are not part of what a cursor belongs to.
    #[test]
    fn a_new_grep_keeps_the_cursor_usable() {
        let (plain, target) = pod_read("a");
        let filtered = input(json!({
            "context": "c", "namespace": "n", "pod": "a",
            "grep": "x", "max_bytes": 2000, "wait_seconds": 5
        }));
        assert_eq!(fingerprint(&plain, &target), fingerprint(&filtered, &target));
    }
}
