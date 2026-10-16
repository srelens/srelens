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
use std::time::Duration;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::jiff::{SignedDuration, Timestamp};
use regex::{Regex, RegexBuilder};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use srelens_capability::CapabilityError;

use crate::logs::{PodLogsIn, WorkloadRef};
use crate::workloads::LabelSelectorRequirement;

const CURSOR_VERSION: u32 = 1;
const AGAIN: &str = "call again without `cursor` to start a new read";

/// A page's `logs` by default, and the least and most it may be, in bytes of
/// JSON-escaped text.
pub const DEFAULT_PAGE_BYTES: u32 = 12_000;
pub const MIN_PAGE_BYTES: u32 = 1_000;
pub const MAX_PAGE_BYTES: u32 = 40_000;
/// The longest a page waits for new lines (`wait_seconds`).
pub const MAX_WAIT_SECONDS: u32 = 60;
/// The most lines kept before and after a `grep` match.
pub const MAX_CONTEXT_LINES: u32 = 10;
/// The most a `grep` pattern may compile to.
const GREP_SIZE_LIMIT: usize = 1 << 20;
const WORKLOAD_KINDS: [&str; 5] = ["Deployment", "StatefulSet", "DaemonSet", "ReplicaSet", "Job"];

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

/// Where a read without a cursor starts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Start {
    /// The oldest line the kubelet still holds.
    Beginning,
    /// The last `n` lines.
    Tail(i64),
    /// The first line stamped at or after this time.
    At(Timestamp),
}

/// A page-mode call, checked.
#[derive(Debug)]
pub(crate) struct PageRequest {
    pub(crate) target: Target,
    pub(crate) container: Option<String>,
    pub(crate) previous: bool,
    pub(crate) timestamps: bool,
    pub(crate) start: Start,
    pub(crate) cursor: Option<Cursor>,
    pub(crate) grep: Option<Regex>,
    pub(crate) context_lines: usize,
    pub(crate) max_bytes: usize,
    pub(crate) wait: Duration,
}

fn within(name: &str, value: u32, min: u32, max: u32) -> Result<u32, CapabilityError> {
    if (min..=max).contains(&value) {
        Ok(value)
    } else {
        Err(invalid(format!("`{name}` must be {min}–{max}, not {value}")))
    }
}

/// `s` on one line, for an error a transcript shows the first line of.
fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl PageRequest {
    pub(crate) fn parse(input: &PodLogsIn) -> Result<Self, CapabilityError> {
        let target = match (
            &input.pod,
            &input.selector,
            &input.match_expressions,
            &input.workload,
        ) {
            (Some(pod), None, None, None) => Target::Pod(pod.clone()),
            (None, labels, expressions, None) if labels.is_some() || expressions.is_some() => {
                Target::Selector {
                    labels: labels.clone().unwrap_or_default(),
                    expressions: expressions.clone().unwrap_or_default(),
                }
            }
            (None, None, None, Some(workload)) => {
                if !WORKLOAD_KINDS.contains(&workload.kind.as_str()) {
                    return Err(invalid(format!(
                        "`workload.kind` {:?} is not one srelens reads; use one of {} (for a CronJob, name one of its Jobs)",
                        workload.kind,
                        WORKLOAD_KINDS.join(", ")
                    )));
                }
                Target::Workload(workload.clone())
            }
            _ => {
                return Err(invalid(
                    "give exactly one of `pod`, `selector` (with `matchExpressions`) or `workload`",
                ))
            }
        };
        if input.since_time.is_some() && input.since_seconds.is_some() {
            return Err(invalid("give `since_time` or `since_seconds`, not both"));
        }
        if input.context_lines.is_some() && input.grep.is_none() {
            return Err(invalid("`context_lines` needs `grep`"));
        }
        let context_lines = within(
            "context_lines",
            input.context_lines.unwrap_or(0),
            0,
            MAX_CONTEXT_LINES,
        )?;
        let max_bytes = within(
            "max_bytes",
            input.max_bytes.unwrap_or(DEFAULT_PAGE_BYTES),
            MIN_PAGE_BYTES,
            MAX_PAGE_BYTES,
        )?;
        let wait = within(
            "wait_seconds",
            input.wait_seconds.unwrap_or(0),
            0,
            MAX_WAIT_SECONDS,
        )?;
        let grep = match &input.grep {
            Some(pattern) => Some(
                RegexBuilder::new(pattern)
                    .size_limit(GREP_SIZE_LIMIT)
                    .build()
                    .map_err(|e| {
                        invalid(format!(
                            "`grep` is not a regex srelens can use: {}",
                            one_line(&e.to_string())
                        ))
                    })?,
            ),
            None => None,
        };
        let start = match (
            &input.since_time,
            input.since_seconds,
            input.all_lines,
            input.tail_lines,
        ) {
            (Some(at), _, _, _) => Start::At(at.parse().map_err(|e| {
                invalid(format!(
                    "`since_time` is not an RFC 3339 time: {}",
                    one_line(&format!("{e}"))
                ))
            })?),
            (None, Some(seconds), _, _) if seconds < 0 => {
                return Err(invalid("`since_seconds` must not be negative"))
            }
            // On srelens' clock, so a cursor can carry the window.
            (None, Some(seconds), _, _) => Start::At(
                Timestamp::now()
                    .checked_sub(SignedDuration::from_secs(seconds))
                    .unwrap_or(Timestamp::MIN),
            ),
            (None, None, false, Some(lines)) => Start::Tail(lines),
            _ => Start::Beginning,
        };
        let cursor = match &input.cursor {
            Some(text) => Some(Cursor::decode(text, &fingerprint(input, &target))?),
            None => None,
        };
        Ok(Self {
            target,
            container: input.container.clone(),
            previous: input.previous,
            timestamps: input.timestamps,
            start,
            cursor,
            grep,
            context_lines: context_lines as usize,
            max_bytes: max_bytes as usize,
            wait: Duration::from_secs(wait.into()),
        })
    }
}

/// Where one pod's read picks up: past the first `k` lines stamped `at`.
#[derive(Debug)]
pub(crate) struct Resume {
    at: Timestamp,
    k: u32,
    seen: u32,
    open: bool,
}

/// What a read does with the next line past where its request began.
#[derive(Debug, PartialEq)]
pub(crate) enum Admit {
    /// The cursor already read it.
    Skip,
    Take,
    /// Take it — but the cursor's own line never came back: rotated away.
    Gap,
}

impl Resume {
    pub(crate) fn new(at: Timestamp, k: u32) -> Self {
        Self {
            at,
            k,
            seen: 0,
            open: true,
        }
    }

    pub(crate) fn admit(&mut self, ts: Timestamp) -> Admit {
        if !self.open {
            return Admit::Take;
        }
        if ts < self.at {
            return Admit::Skip;
        }
        if ts == self.at && self.seen < self.k {
            self.seen += 1;
            return Admit::Skip;
        }
        self.open = false;
        if self.seen < self.k {
            Admit::Gap
        } else {
            Admit::Take
        }
    }

    /// At the end of the stream: the cursor's line never came back, and
    /// nothing newer did either.
    pub(crate) fn lost_at_end(&self) -> bool {
        self.open && self.seen < self.k
    }
}

/// One container instance, by its restart count: the current one, or the
/// one before it (`previous`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Instance {
    pub(crate) previous: bool,
    pub(crate) restarts: i32,
}

/// What became of the instance a cursor was reading.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Moved {
    Stayed,
    /// It ended once: read its rest, then the new one from its first line.
    Restarted,
    /// It is gone (more than one restart): read the newest from its start.
    Lost,
}

/// The instances a read covers, oldest first, given the container's restart
/// count `now` and the instance the cursor read (`read`, a restart count).
pub(crate) fn plan_instances(
    now: i32,
    previous: bool,
    read: Option<i32>,
) -> (Vec<Instance>, Moved) {
    let current = Instance {
        previous: false,
        restarts: now,
    };
    let last = Instance {
        previous: true,
        restarts: now - 1,
    };
    match (previous, read) {
        (false, None) => (vec![current], Moved::Stayed),
        (false, Some(r)) if r == now => (vec![current], Moved::Stayed),
        (false, Some(r)) if r == now - 1 => (vec![last, current], Moved::Restarted),
        (false, Some(_)) => (vec![current], Moved::Lost),
        (true, None) => (vec![last], Moved::Stayed),
        (true, Some(r)) if r == now - 1 => (vec![last], Moved::Stayed),
        (true, Some(_)) => (vec![last], Moved::Lost),
    }
}

/// Why a pod has no container to read.
#[derive(Debug)]
pub(crate) enum NoContainer {
    /// The named container is not in this pod: a notice when the read covers
    /// several.
    Missing(String),
    /// Several containers and none named: the call is wrong for every pod
    /// alike.
    Ambiguous(String),
}

/// The container a read of `pod` covers: the named one, or its only one.
pub(crate) fn container_of(pod: &Pod, named: Option<&str>) -> Result<String, NoContainer> {
    let pod_name = pod.metadata.name.as_deref().unwrap_or_default();
    let (main, init): (Vec<&str>, Vec<&str>) = match pod.spec.as_ref() {
        Some(spec) => (
            spec.containers.iter().map(|c| c.name.as_str()).collect(),
            spec.init_containers
                .iter()
                .flatten()
                .map(|c| c.name.as_str())
                .collect(),
        ),
        None => (Vec::new(), Vec::new()),
    };
    match named {
        Some(name) if main.contains(&name) || init.contains(&name) => Ok(name.to_string()),
        Some(name) => Err(NoContainer::Missing(format!(
            "{pod_name}: has no container {name:?} (it has {})",
            main.join(", ")
        ))),
        None => match main.as_slice() {
            [only] => Ok((*only).to_string()),
            _ => Err(NoContainer::Ambiguous(format!(
                "pod {pod_name} has containers {}; name one with `container`",
                main.join(", ")
            ))),
        },
    }
}

/// `container`'s restart count in `pod`'s status, 0 when it reports none.
pub(crate) fn restarts_of(pod: &Pod, container: &str) -> i32 {
    let Some(status) = pod.status.as_ref() else {
        return 0;
    };
    status
        .container_statuses
        .iter()
        .flatten()
        .chain(status.init_container_statuses.iter().flatten())
        .find(|s| s.name == container)
        .map_or(0, |s| s.restart_count)
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

    fn parse(v: Value) -> Result<PageRequest, String> {
        PageRequest::parse(&input(v)).map_err(|e| e.to_string())
    }

    /// `v` with `extra`'s fields added.
    fn with(mut v: Value, extra: Value) -> Value {
        v.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        v
    }

    #[test]
    fn a_read_names_exactly_one_target() {
        for v in [
            json!({ "context": "c", "namespace": "n", "grep": "x" }),
            json!({ "context": "c", "namespace": "n", "pod": "a", "selector": { "app": "x" } }),
            json!({ "context": "c", "namespace": "n", "pod": "a", "workload": { "kind": "Deployment", "name": "x" } }),
            json!({ "context": "c", "namespace": "n", "selector": { "app": "x" }, "workload": { "kind": "Deployment", "name": "x" } }),
        ] {
            let err = parse(v.clone()).unwrap_err();
            assert!(err.contains("exactly one of `pod`"), "{v}: {err}");
        }
        let only_expressions = parse(json!({
            "context": "c", "namespace": "n",
            "matchExpressions": [{ "key": "app", "operator": "Exists" }]
        }))
        .unwrap();
        assert_eq!(
            only_expressions.target,
            Target::Selector {
                labels: BTreeMap::new(),
                expressions: vec![LabelSelectorRequirement {
                    key: "app".into(),
                    operator: "Exists".into(),
                    values: vec![]
                }]
            }
        );
    }

    #[test]
    fn page_inputs_out_of_range_or_in_conflict_are_refused_in_one_line() {
        let cases = [
            (json!({ "since_time": "2026-10-16T00:00:00Z", "since_seconds": 60 }), "not both"),
            (json!({ "context_lines": 2 }), "needs `grep`"),
            (json!({ "grep": "x", "context_lines": 11 }), "`context_lines` must be 0–10"),
            (json!({ "max_bytes": 999 }), "`max_bytes` must be 1000–40000"),
            (json!({ "max_bytes": 40001 }), "`max_bytes` must be 1000–40000"),
            (json!({ "wait_seconds": 61 }), "`wait_seconds` must be 0–60"),
            (json!({ "grep": "(unclosed" }), "`grep` is not a regex"),
            (json!({ "since_time": "yesterday" }), "`since_time` is not an RFC 3339 time"),
            (json!({ "max_bytes": 2000, "since_seconds": -5 }), "`since_seconds` must not be negative"),
        ];
        for (extra, want) in cases {
            let err = parse(with(json!({ "context": "c", "namespace": "n", "pod": "a" }), extra.clone()))
                .unwrap_err();
            assert!(err.contains(want), "{extra}: {err}");
            assert_eq!(err.lines().count(), 1, "{err}");
        }
    }

    #[test]
    fn a_workload_of_a_kind_srelens_does_not_read_is_refused_naming_the_kinds() {
        let err = parse(json!({
            "context": "c", "namespace": "n", "workload": { "kind": "CronJob", "name": "x" }
        }))
        .unwrap_err();
        assert!(err.contains("Deployment, StatefulSet, DaemonSet, ReplicaSet, Job"), "{err}");
    }

    #[test]
    fn where_a_read_without_a_cursor_starts() {
        let start = |extra: Value| {
            parse(with(
                json!({ "context": "c", "namespace": "n", "pod": "a", "max_bytes": 2000 }),
                extra,
            ))
            .unwrap()
            .start
        };
        assert_eq!(start(json!({})), Start::Beginning);
        assert_eq!(start(json!({ "tail_lines": 50 })), Start::Tail(50));
        assert_eq!(start(json!({ "tail_lines": 50, "all_lines": true })), Start::Beginning);
        assert_eq!(
            start(json!({ "since_time": "2026-10-16T10:00:00.25Z" })),
            Start::At("2026-10-16T10:00:00.25Z".parse().unwrap())
        );
        let Start::At(at) = start(json!({ "since_seconds": 60 })) else {
            panic!("since_seconds is a start time in page mode")
        };
        let ago = Timestamp::now().duration_since(at).as_secs();
        assert!((59..=61).contains(&ago), "{ago}");
    }

    #[test]
    fn a_cursor_is_checked_against_the_read_it_is_given_to() {
        let (read, target) = pod_read("a");
        let cursor = Cursor {
            v: CURSOR_VERSION,
            t: fingerprint(&read, &target),
            p: BTreeMap::new(),
        }
        .encode();
        assert!(
            parse(json!({ "context": "c", "namespace": "n", "pod": "a", "cursor": cursor }))
                .unwrap()
                .cursor
                .is_some()
        );
        let err = parse(json!({ "context": "c", "namespace": "n", "pod": "b", "cursor": cursor }))
            .unwrap_err();
        assert!(err.contains("another read"), "{err}");
    }

    /// Kubelet stamps for the tests: `s` seconds and `ms` milliseconds past
    /// 2026-09-21.
    const BASE: i64 = 1_790_000_000;
    fn at(s: i64, ms: i32) -> Timestamp {
        Timestamp::new(BASE + s, ms * 1_000_000).unwrap()
    }

    fn admitted(mut resume: Resume, stamps: &[Timestamp]) -> Vec<&'static str> {
        stamps
            .iter()
            .map(|&ts| match resume.admit(ts) {
                Admit::Skip => "skip",
                Admit::Take => "take",
                Admit::Gap => "gap",
            })
            .collect()
    }

    #[test]
    fn a_resume_drops_what_the_cursor_read_and_takes_the_rest() {
        // Read: the line at 0.0 and one of the two at 1.0.
        assert_eq!(
            admitted(
                Resume::new(at(1, 0), 1),
                &[at(0, 0), at(1, 0), at(1, 0), at(2, 0)]
            ),
            ["skip", "skip", "take", "take"]
        );
        // Both lines at 1.0 were read.
        assert_eq!(
            admitted(Resume::new(at(1, 0), 2), &[at(1, 0), at(1, 0), at(1, 500)]),
            ["skip", "skip", "take"]
        );
        // A start time with nothing read yet (`k: 0`) takes its own second's
        // lines from it on.
        assert_eq!(
            admitted(Resume::new(at(1, 500), 0), &[at(1, 0), at(1, 500), at(2, 0)]),
            ["skip", "take", "take"]
        );
    }

    #[test]
    fn a_resume_whose_line_was_rotated_away_reports_the_gap_once() {
        assert_eq!(
            admitted(Resume::new(at(1, 0), 1), &[at(5, 0), at(6, 0)]),
            ["gap", "take"]
        );
        // Only one of the two lines the cursor read is still there.
        assert_eq!(
            admitted(Resume::new(at(1, 0), 2), &[at(1, 0), at(3, 0)]),
            ["skip", "gap"]
        );
        let mut resume = Resume::new(at(1, 0), 1);
        assert!(resume.lost_at_end(), "nothing at all came back");
        resume.admit(at(1, 0));
        assert!(!resume.lost_at_end());
    }

    #[test]
    fn which_instances_a_read_covers_after_restarts() {
        let current = |r| Instance {
            previous: false,
            restarts: r,
        };
        let last = |r| Instance {
            previous: true,
            restarts: r,
        };
        assert_eq!(plan_instances(3, false, None), (vec![current(3)], Moved::Stayed));
        assert_eq!(plan_instances(3, false, Some(3)), (vec![current(3)], Moved::Stayed));
        assert_eq!(
            plan_instances(3, false, Some(2)),
            (vec![last(2), current(3)], Moved::Restarted)
        );
        assert_eq!(plan_instances(3, false, Some(1)), (vec![current(3)], Moved::Lost));
        assert_eq!(plan_instances(3, true, None), (vec![last(2)], Moved::Stayed));
        assert_eq!(plan_instances(3, true, Some(2)), (vec![last(2)], Moved::Stayed));
        assert_eq!(plan_instances(3, true, Some(1)), (vec![last(2)], Moved::Lost));
    }

    fn pod_object(containers: &[&str], init: &[&str], restarts: i32) -> Pod {
        serde_json::from_value(json!({
            "metadata": { "name": "cmi-0" },
            "spec": {
                "containers": containers.iter().map(|c| json!({ "name": c })).collect::<Vec<_>>(),
                "initContainers": init.iter().map(|c| json!({ "name": c })).collect::<Vec<_>>(),
            },
            "status": { "containerStatuses": containers.iter().map(|c| json!({
                "name": c, "restartCount": restarts, "ready": true, "image": "i", "imageID": "i"
            })).collect::<Vec<_>>() }
        }))
        .unwrap()
    }

    #[test]
    fn the_container_a_read_covers() {
        let one = pod_object(&["app"], &["setup"], 0);
        assert_eq!(container_of(&one, None).ok().as_deref(), Some("app"));
        assert_eq!(container_of(&one, Some("setup")).ok().as_deref(), Some("setup"));
        let Err(NoContainer::Missing(why)) = container_of(&one, Some("sidecar")) else {
            panic!("a container the pod lacks is missing")
        };
        assert!(why.contains("cmi-0") && why.contains("\"sidecar\""), "{why}");
        let two = pod_object(&["app", "proxy"], &[], 0);
        let Err(NoContainer::Ambiguous(why)) = container_of(&two, None) else {
            panic!("two containers and none named is ambiguous")
        };
        assert!(why.contains("app, proxy") && why.contains("`container`"), "{why}");
        assert_eq!(restarts_of(&pod_object(&["app"], &[], 4), "app"), 4);
        assert_eq!(restarts_of(&pod_object(&["app"], &[], 4), "missing"), 0);
    }
}
