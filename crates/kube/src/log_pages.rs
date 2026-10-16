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

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::pin::Pin;
use std::time::Duration;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use futures::AsyncBufRead;
use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, ReplicaSet, StatefulSet};
use k8s_openapi::api::batch::v1::Job;
use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;
use k8s_openapi::jiff::{SignedDuration, Timestamp};
use kube::api::{ListParams, LogParams};
use kube::{Api, Client};
use regex::{Regex, RegexBuilder};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use srelens_capability::CapabilityError;

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;
use crate::logs::{Line, LineReader, PodLogsIn, PodLogsOut, WorkloadRef};
use crate::workloads::{selector_query, LabelSelectorRequirement};

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

/// The most a page's answer may be, in bytes of compact JSON: under the MCP
/// server's 50,000-byte cap on one read-only answer
/// (`srelens_mcp::MAX_RESULT_BYTES`; a registry test pins the two), with
/// room to spare.
pub const MAX_ANSWER_BYTES: usize = 49_000;
/// The most notices a page carries, past which one more counts the rest.
const MAX_NOTICES: usize = 20;
/// The most of one notice kept, escaped.
const MAX_NOTICE_BYTES: usize = 400;

/// The longest the backlog of one page is read before it is handed back as
/// it stands.
const BACKLOG: Duration = Duration::from_secs(20);
/// Marks a line cut to fit the page.
pub(crate) const CUT: &str = " [line cut by srelens]";

type LogStream = Pin<Box<dyn AsyncBufRead + Send>>;

/// A line with the kubelet's stamp split off.
#[derive(Debug, Clone)]
struct Stamped {
    ts: Timestamp,
    raw: String,
    text: String,
    truncated: bool,
}

/// One pod a page reads, and where its read stands.
struct PodRead {
    name: String,
    container: String,
    /// The instances still to read, oldest first; the front one is being read.
    instances: VecDeque<Instance>,
    /// `pos.t`, parsed.
    last: Option<Timestamp>,
    pos: Pos,
    /// Lines from the end a fresh read starts with (`tail_lines`).
    tail: Option<i64>,
    /// Drops what the cursor already read, once per stream opened.
    resume: Option<Resume>,
    lines: Option<LineReader<LogStream>>,
    /// The next line, read but not yet taken.
    head: Option<Stamped>,
    /// Nothing more to read, as of now.
    ended: bool,
    /// The read failed; its position stays where it was.
    failed: bool,
    /// For `context_lines`: lines held before a match, and how many after one
    /// are owed.
    before: VecDeque<Stamped>,
    after: usize,
    notices: Vec<String>,
}

impl PodRead {
    fn new(
        name: String,
        uid: String,
        container: String,
        instances: VecDeque<Instance>,
        at: Option<Pos>,
        req: &PageRequest,
    ) -> Self {
        let restarts = instances.front().map_or(0, |i| i.restarts);
        let fresh = |t: Option<String>| Pos {
            u: uid.clone(),
            r: restarts,
            t,
            k: 0,
        };
        let (pos, tail) = match (at, req.cursor.is_some(), req.start) {
            (Some(pos), _, _) => (pos, None),
            // A pod a cursor has not seen yet is read from its first line.
            (None, true, _) | (None, false, Start::Beginning) => (fresh(None), None),
            (None, false, Start::Tail(n)) => (fresh(None), Some(n)),
            (None, false, Start::At(at)) => (fresh(Some(at.to_string())), None),
        };
        let last = pos.t.as_deref().and_then(|t| t.parse().ok());
        Self {
            name,
            container,
            instances,
            last,
            pos,
            tail,
            resume: None,
            lines: None,
            head: None,
            ended: false,
            failed: false,
            before: VecDeque::new(),
            after: 0,
            notices: Vec::new(),
        }
    }

    async fn open(
        &mut self,
        pods: &Api<Pod>,
        instance: Instance,
        req: &PageRequest,
        follow: bool,
    ) -> Result<LineReader<LogStream>, String> {
        let mut params = LogParams {
            container: Some(self.container.clone()),
            follow,
            previous: instance.previous,
            timestamps: true,
            ..LogParams::default()
        };
        match self.last {
            Some(at) => {
                // The kubelet is handed whole seconds; `Resume` drops the rest.
                params.since_time =
                    Some(Timestamp::from_second(at.as_second()).map_err(|e| e.to_string())?);
                self.resume = Some(Resume::new(at, self.pos.k));
            }
            None => params.tail_lines = self.tail,
        }
        let log = tokio::time::timeout(request_timeout(), pods.log_stream(&self.name, &params))
            .await
            .map_err(|_| "opening the log timed out".to_string())?
            .map_err(|e| e.to_string())?;
        let log: LogStream = Box::pin(log);
        // Room for the stamp in front of a line as long as the page.
        Ok(LineReader::new(log, req.max_bytes + 64))
    }

    /// The next line past the cursor, reading on into the next instance at
    /// the end of one; `None` once there is nothing more, as of now.
    async fn next(
        &mut self,
        pods: &Api<Pod>,
        req: &PageRequest,
        follow: bool,
    ) -> Result<Option<Stamped>, String> {
        loop {
            let Some(instance) = self.instances.front().copied() else {
                return Ok(None);
            };
            if self.lines.is_none() {
                self.lines = Some(self.open(pods, instance, req, follow).await?);
            }
            let read = self.lines.as_mut().expect("opened above").next().await;
            let Some(line) = read.map_err(|e| e.to_string())? else {
                self.lines = None;
                if self.resume.take().is_some_and(|r| r.lost_at_end()) {
                    self.notices.push(format!(
                        "{}: the line the cursor stopped at ({}) was rotated away by the kubelet, and nothing newer is readable yet",
                        self.name,
                        self.pos.t.as_deref().unwrap_or("?")
                    ));
                    // What is left of it is all there is to skip next time.
                    self.pos.k = 0;
                }
                if self.instances.len() == 1 {
                    return Ok(None);
                }
                // The instance that ended is read; the next starts at its first line.
                self.instances.pop_front();
                self.pos = Pos {
                    u: self.pos.u.clone(),
                    r: self.instances[0].restarts,
                    t: None,
                    k: 0,
                };
                self.last = None;
                self.tail = None;
                continue;
            };
            let line = self.stamp(line);
            match self.resume.as_mut().map(|r| r.admit(line.ts)) {
                Some(Admit::Skip) => continue,
                Some(Admit::Gap) => self.notices.push(format!(
                    "{}: lines written between {} and {} were rotated away by the kubelet before this read reached them",
                    self.name,
                    self.pos.t.as_deref().unwrap_or("?"),
                    line.raw
                )),
                _ => {}
            }
            return Ok(Some(line));
        }
    }

    /// `line` with the kubelet's stamp split off. A line without one (the
    /// kubelet stamps every line; this is defence) takes the stamp before it.
    fn stamp(&self, line: Line) -> Stamped {
        if let Some((raw, text)) = line.text.split_once(' ') {
            if let Ok(ts) = raw.parse::<Timestamp>() {
                return Stamped {
                    ts,
                    raw: raw.to_string(),
                    text: text.to_string(),
                    truncated: line.truncated,
                };
            }
        }
        let ts = self.last.unwrap_or(Timestamp::UNIX_EPOCH);
        Stamped {
            ts,
            raw: ts.to_string(),
            text: line.text,
            truncated: line.truncated,
        }
    }

    /// Move the read past `line`.
    fn took(&mut self, line: &Stamped) {
        if self.last == Some(line.ts) {
            self.pos.k += 1;
        } else {
            self.last = Some(line.ts);
            self.pos.t = Some(line.raw.clone());
            self.pos.k = 1;
        }
    }

    /// A read that cannot go on: the call's error for a read of one pod, a
    /// notice for one of several.
    fn fail(&mut self, why: String, req: &PageRequest) -> Result<(), CapabilityError> {
        if req.target.single() {
            return Err(CapabilityError::Handler(why));
        }
        self.notices.push(format!(
            "{}: could not read its log: {}",
            self.name,
            one_line(&why)
        ));
        self.ended = true;
        self.failed = true;
        Ok(())
    }

    /// Open the stream again with `follow`, from where the read stands.
    async fn reopen(&mut self, pods: &Api<Pod>, req: &PageRequest) -> Result<(), CapabilityError> {
        self.ended = false;
        let instance = *self
            .instances
            .front()
            .expect("a read keeps the instance it last read");
        match self.open(pods, instance, req, true).await {
            Ok(lines) => {
                self.lines = Some(lines);
                Ok(())
            }
            Err(why) => self.fail(why, req),
        }
    }

    /// Read the next line into `head`, unless one is there or the read ended.
    async fn fill(
        &mut self,
        pods: &Api<Pod>,
        req: &PageRequest,
        follow: bool,
    ) -> Result<(), CapabilityError> {
        if self.head.is_some() || self.ended {
            return Ok(());
        }
        match self.next(pods, req, follow).await {
            Ok(Some(line)) => self.head = Some(line),
            Ok(None) => self.ended = true,
            Err(why) => self.fail(why, req)?,
        }
        Ok(())
    }
}

/// What `c` costs inside a JSON string, as serde_json escapes it.
fn char_cost(c: char) -> usize {
    match c {
        '"' | '\\' | '\n' | '\r' | '\t' | '\u{08}' | '\u{0c}' => 2,
        c if (c as u32) < 0x20 => 6,
        c => c.len_utf8(),
    }
}

/// `s`'s size inside a JSON string.
fn escaped_len(s: &str) -> usize {
    s.chars().map(char_cost).sum()
}

/// The longest start of `s` whose escaped size is at most `room`, cut on a
/// character boundary.
fn fit(s: &str, room: usize) -> String {
    let mut used = 0;
    s.chars()
        .take_while(|&c| {
            used += char_cost(c);
            used <= room
        })
        .collect()
}

/// A page's `logs`, built a line at a time within its budget.
struct PageText {
    text: String,
    used: usize,
    budget: usize,
    lines: usize,
    scanned: usize,
    /// `[pod] ` in front of each line, for a read of several pods.
    prefix: bool,
    timestamps: bool,
}

impl PageText {
    fn new(budget: usize, prefix: bool, timestamps: bool) -> Self {
        Self {
            text: String::new(),
            used: 0,
            budget,
            lines: 0,
            scanned: 0,
            prefix,
            timestamps,
        }
    }

    /// Take `line`, the next one `read` has: shown, or skipped by `grep`.
    /// False when the page is full and the line is left for the next page.
    fn offer(&mut self, read: &mut PodRead, line: &Stamped, req: &PageRequest) -> bool {
        let Some(grep) = req.grep.as_ref() else {
            return self.keep(&read.name, line);
        };
        if grep.is_match(&line.text) {
            // ponytail: before-context a page edge falls inside is not shown
            // again on the next page; carry it in the cursor if it matters.
            while let Some(before) = read.before.pop_front() {
                if !self.push(&read.name, &before) {
                    return false;
                }
            }
            let kept = self.keep(&read.name, line);
            if kept {
                read.after = req.context_lines;
            }
            kept
        } else if read.after > 0 {
            let kept = self.keep(&read.name, line);
            if kept {
                read.after -= 1;
            }
            kept
        } else {
            if req.context_lines > 0 {
                if read.before.len() == req.context_lines {
                    read.before.pop_front();
                }
                read.before.push_back(line.clone());
            }
            self.scanned += 1;
            true
        }
    }

    fn keep(&mut self, pod: &str, line: &Stamped) -> bool {
        if !self.push(pod, line) {
            return false;
        }
        self.scanned += 1;
        true
    }

    /// Show `line`; false when it does not fit. The first line always does,
    /// cut if it must be, so a read always moves on.
    fn push(&mut self, pod: &str, line: &Stamped) -> bool {
        let mut shown = String::new();
        if self.prefix {
            shown.push('[');
            shown.push_str(pod);
            shown.push_str("] ");
        }
        if self.timestamps {
            shown.push_str(&line.raw);
            shown.push(' ');
        }
        shown.push_str(&line.text);
        if line.truncated {
            shown.push_str(CUT);
        }
        let room = self.budget.saturating_sub(self.used);
        // And its "\n", escaped.
        let mut cost = escaped_len(&shown) + 2;
        if cost > room {
            if self.lines > 0 {
                return false;
            }
            shown = fit(&shown, room.saturating_sub(2 + CUT.len()));
            shown.push_str(CUT);
            cost = escaped_len(&shown) + 2;
        }
        self.text.push_str(&shown);
        self.text.push('\n');
        self.used += cost;
        self.lines += 1;
        true
    }
}

/// Phase 1, the backlog: the reads' lines in time order until the page is
/// full (`false`) or every log has ended (`true`).
async fn merge(
    reads: &mut [PodRead],
    pods: &Api<Pod>,
    req: &PageRequest,
    page: &mut PageText,
) -> Result<bool, CapabilityError> {
    futures::future::try_join_all(reads.iter_mut().map(|read| read.fill(pods, req, false)))
        .await?;
    loop {
        // The oldest head; on a tie, the pod first by name.
        let Some(i) = (0..reads.len())
            .filter(|&i| reads[i].head.is_some())
            .min_by_key(|&i| reads[i].head.as_ref().map(|line| line.ts))
        else {
            return Ok(true);
        };
        let line = reads[i].head.take().expect("picked for its head");
        if !page.offer(&mut reads[i], &line, req) {
            reads[i].head = Some(line);
            return Ok(false);
        }
        reads[i].took(&line);
        reads[i].fill(pods, req, false).await?;
    }
}

/// Phase 2, the follow: lines as they arrive until the page is full
/// (`false`) or `req.wait` runs out (`true`: the read caught up).
async fn follow(
    reads: &mut [PodRead],
    pods: &Api<Pod>,
    req: &PageRequest,
    page: &mut PageText,
) -> Result<bool, CapabilityError> {
    let deadline = tokio::time::Instant::now() + req.wait;
    // Every stream reopened at once, so a line on one pod does not cancel
    // another's half-made request.
    futures::future::try_join_all(
        reads
            .iter_mut()
            .filter(|read| !read.failed)
            .map(|read| read.reopen(pods, req)),
    )
    .await?;
    loop {
        if reads.iter().all(|read| read.ended) {
            return Ok(true);
        }
        // `LineReader::next` is cancel-safe, so the reads still waiting lose
        // nothing.
        let arrived = futures::future::select_all(
            reads
                .iter_mut()
                .filter(|read| !read.ended)
                .map(|read| Box::pin(read.fill(pods, req, true))),
        );
        match tokio::time::timeout_at(deadline, arrived).await {
            Err(_) => return Ok(true),
            Ok((filled, _, _)) => filled?,
        }
        while let Some(i) = (0..reads.len())
            .filter(|&i| reads[i].head.is_some())
            .min_by_key(|&i| reads[i].head.as_ref().map(|line| line.ts))
        {
            let line = reads[i].head.take().expect("picked for its head");
            if !page.offer(&mut reads[i], &line, req) {
                reads[i].head = Some(line);
                return Ok(false);
            }
            reads[i].took(&line);
        }
    }
}

async fn timed<T>(
    what: &str,
    call: impl std::future::Future<Output = Result<T, kube::Error>>,
) -> Result<T, CapabilityError> {
    tokio::time::timeout(request_timeout(), call)
        .await
        .map_err(|_| CapabilityError::Handler(format!("{what} timed out")))?
        .map_err(|e| CapabilityError::Handler(e.to_string()))
}

/// The most pods one page reads.
pub const MAX_PODS: usize = 20;

/// The pod selector of `workload`, which `parse` checked is a kind srelens
/// reads.
async fn workload_selector(
    client: &Client,
    namespace: &str,
    workload: &WorkloadRef,
) -> Result<LabelSelector, CapabilityError> {
    let what = format!("reading {} {}", workload.kind, workload.name);
    let name = workload.name.as_str();
    let selector = match workload.kind.as_str() {
        "Deployment" => timed(&what, Api::<Deployment>::namespaced(client.clone(), namespace).get(name))
            .await?
            .spec
            .map(|s| s.selector),
        "StatefulSet" => timed(&what, Api::<StatefulSet>::namespaced(client.clone(), namespace).get(name))
            .await?
            .spec
            .map(|s| s.selector),
        "DaemonSet" => timed(&what, Api::<DaemonSet>::namespaced(client.clone(), namespace).get(name))
            .await?
            .spec
            .map(|s| s.selector),
        "ReplicaSet" => timed(&what, Api::<ReplicaSet>::namespaced(client.clone(), namespace).get(name))
            .await?
            .spec
            .map(|s| s.selector),
        "Job" => timed(&what, Api::<Job>::namespaced(client.clone(), namespace).get(name))
            .await?
            .spec
            .and_then(|s| s.selector),
        _ => None,
    };
    selector.ok_or_else(|| {
        CapabilityError::Handler(format!(
            "{} {} has no pod selector",
            workload.kind, workload.name
        ))
    })
}

/// The pods a read covers, by name.
async fn resolve_pods(
    client: &Client,
    namespace: &str,
    target: &Target,
) -> Result<Vec<Pod>, CapabilityError> {
    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let query = match target {
        Target::Pod(name) => return Ok(vec![timed("reading the pod", pods.get(name)).await?]),
        Target::Selector {
            labels,
            expressions,
        } => selector_query(labels, expressions)?,
        Target::Workload(workload) => {
            let selector = workload_selector(client, namespace, workload).await?;
            let expressions: Vec<LabelSelectorRequirement> = selector
                .match_expressions
                .unwrap_or_default()
                .into_iter()
                .map(|e| LabelSelectorRequirement {
                    key: e.key,
                    operator: e.operator,
                    values: e.values.unwrap_or_default(),
                })
                .collect();
            selector_query(&selector.match_labels.unwrap_or_default(), &expressions)?
        }
    };
    // A selector that constrains nothing, or can never match, reads no pods
    // rather than the whole namespace (`selector_query`).
    let Some(query) = query else {
        return Ok(Vec::new());
    };
    let mut found = timed(
        "listing the pods",
        pods.list(&ListParams::default().labels(&query)),
    )
    .await?
    .items;
    if found.len() > MAX_PODS {
        return Err(invalid(format!(
            "{} pods match; srelens reads at most {MAX_PODS} in one call — narrow the selector (a label such as pod-template-hash, or one pod)",
            found.len()
        )));
    }
    found.sort_by(|a, b| a.metadata.name.cmp(&b.metadata.name));
    Ok(found)
}

/// The most a page's answer holds besides `logs`: the cursor with every read
/// at its longest, the notices at their most, and the other fields.
fn envelope_bound(reads: &[PodRead]) -> usize {
    let longest = Cursor {
        v: CURSOR_VERSION,
        t: "0".repeat(16),
        // A kubelet stamp is at most 35 characters (nanoseconds and an offset).
        p: reads
            .iter()
            .map(|read| {
                (
                    read.name.clone(),
                    Pos {
                        u: read.pos.u.clone(),
                        r: i32::MIN,
                        t: Some("9".repeat(40)),
                        k: u32::MAX,
                    },
                )
            })
            .collect(),
    };
    longest.encode().len() + (MAX_NOTICES + 1) * (MAX_NOTICE_BYTES + 3) + 200
}

/// At most `MAX_NOTICES`, each cut to `MAX_NOTICE_BYTES`, and a last one
/// counting any dropped.
fn cap_notices(mut notices: Vec<String>) -> Vec<String> {
    if notices.len() > MAX_NOTICES {
        let dropped = notices.len() - MAX_NOTICES;
        notices.truncate(MAX_NOTICES);
        notices.push(format!("{dropped} more notices not shown"));
    }
    notices
        .iter()
        .map(|notice| fit(notice, MAX_NOTICE_BYTES))
        .collect()
}

/// One read per pod, from where the cursor left it, and what to tell the
/// agent about how the pods changed.
fn plan_reads(
    pods: Vec<Pod>,
    req: &PageRequest,
) -> Result<(Vec<PodRead>, Vec<String>), CapabilityError> {
    let mut reads = Vec::new();
    let mut notices = Vec::new();
    let mut seen = BTreeSet::new();
    for pod in pods {
        let name = pod.metadata.name.clone().unwrap_or_default();
        seen.insert(name.clone());
        let uid = pod.metadata.uid.clone().unwrap_or_default();
        let container = match container_of(&pod, req.container.as_deref()) {
            Ok(container) => container,
            Err(NoContainer::Ambiguous(why)) => return Err(invalid(why)),
            Err(NoContainer::Missing(why)) if req.target.single() => return Err(invalid(why)),
            Err(NoContainer::Missing(why)) => {
                notices.push(why);
                continue;
            }
        };
        let now = restarts_of(&pod, &container);
        let mut notices_here = Vec::new();
        let mut at = req.cursor.as_ref().and_then(|c| c.p.get(&name)).cloned();
        if at.as_ref().is_some_and(|pos| pos.u != uid) {
            notices_here.push(format!(
                "{name}: replaced by a new pod since the cursor; read from the new one's first line, and the old one's unread lines are gone"
            ));
            at = None;
        }
        if at.is_none() && req.cursor.is_some() && !req.target.single() && notices_here.is_empty()
        {
            notices_here.push(format!(
                "{name}: new since the cursor; read from its first line"
            ));
        }
        let (instances, moved) = plan_instances(now, req.previous, at.as_ref().map(|p| p.r));
        match moved {
            Moved::Stayed => {}
            Moved::Restarted => notices_here.push(format!(
                "{name}: the container restarted since the cursor; this reads the rest of the instance that ended, then the new one from its first line"
            )),
            Moved::Lost => {
                notices_here.push(format!(
                    "{name}: the container restarted more than once since the cursor; lines written after {} up to the newest instance are lost",
                    at.as_ref()
                        .and_then(|p| p.t.as_deref())
                        .unwrap_or("the cursor")
                ));
                at = Some(Pos {
                    u: uid.clone(),
                    r: instances[0].restarts,
                    t: None,
                    k: 0,
                });
            }
        }
        let mut read = PodRead::new(name, uid, container, instances.into(), at, req);
        read.notices = notices_here;
        reads.push(read);
    }
    if let Some(cursor) = &req.cursor {
        for gone in cursor.p.keys().filter(|name| !seen.contains(*name)) {
            notices.push(format!(
                "{gone}: gone since the cursor; its unread lines are lost"
            ));
        }
    }
    Ok((reads, notices))
}

/// `k8s.podLogs` in page mode.
pub(crate) async fn read_page(
    cache: &ClientCache,
    input: PodLogsIn,
) -> Result<PodLogsOut, CapabilityError> {
    let req = PageRequest::parse(&input)?;
    let client = cache
        .get(&input.context)
        .await
        .map_err(CapabilityError::Handler)?;
    let pods = resolve_pods(&client, &input.namespace, &req.target).await?;
    let (mut reads, mut notices) = plan_reads(pods, &req)?;
    let api: Api<Pod> = Api::namespaced(client, &input.namespace);
    let budget = req
        .max_bytes
        .min(MAX_ANSWER_BYTES.saturating_sub(envelope_bound(&reads)));
    let mut page = PageText::new(budget, !req.target.single(), req.timestamps);
    // Phase 1, the backlog. The deadline hands back the page as it stands.
    let mut end = tokio::time::timeout(BACKLOG, merge(&mut reads, &api, &req, &mut page))
        .await
        .unwrap_or(Ok(false))?;
    // Phase 2, the follow: only once every log is read to its end and the
    // page has room.
    if end && !req.wait.is_zero() {
        end = follow(&mut reads, &api, &req, &mut page).await?;
    }
    for read in &reads {
        notices.extend(read.notices.iter().cloned());
    }
    let next_cursor = Cursor {
        v: CURSOR_VERSION,
        t: fingerprint(&input, &req.target),
        p: reads
            .iter()
            .map(|read| (read.name.clone(), read.pos.clone()))
            .collect(),
    }
    .encode();
    Ok(PodLogsOut {
        logs: page.text,
        page: Some(Page {
            next_cursor,
            end,
            lines: page.lines,
            scanned: page.scanned,
            notices: cap_notices(notices),
        }),
    })
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

    use bytes::Bytes;
    use http_body_util::combinators::UnsyncBoxBody;
    use http_body_util::{BodyExt, Full, StreamBody};
    use std::sync::{Arc, Mutex};

    type Body = UnsyncBoxBody<Bytes, std::convert::Infallible>;

    /// A pod as the fake API server keeps it, with each instance's log as the
    /// kubelet would: `(stamp, text)`, oldest first.
    #[derive(Clone, Default)]
    struct FakePod {
        name: String,
        uid: String,
        labels: BTreeMap<String, String>,
        containers: Vec<String>,
        restarts: i32,
        current: Vec<(Timestamp, String)>,
        previous: Vec<(Timestamp, String)>,
        /// What a follow hands out after `current`; the stream then stays open.
        later: Vec<(Timestamp, String)>,
        /// Every log read of the pod fails with this message (a 400).
        fails: Option<String>,
    }

    fn pod(name: &str, lines: Vec<(Timestamp, String)>) -> FakePod {
        FakePod {
            name: name.into(),
            uid: format!("uid-{name}"),
            labels: [("app".to_string(), "cmi".to_string())].into(),
            containers: vec!["app".into()],
            current: lines,
            ..FakePod::default()
        }
    }

    /// The fake API server's state, which a test may change between pages.
    #[derive(Default)]
    struct Cluster {
        pods: Mutex<Vec<FakePod>>,
        /// Deployments by name, as their `matchLabels`.
        deployments: Mutex<BTreeMap<String, BTreeMap<String, String>>>,
    }

    async fn cache_for(cluster: Arc<Cluster>) -> Arc<ClientCache> {
        let service = tower::service_fn(move |request: http::Request<kube::client::Body>| {
            let response = answer(&cluster, request.uri());
            async move { Ok::<_, std::convert::Infallible>(response) }
        });
        let cache = ClientCache::new(std::path::PathBuf::from("/nonexistent"));
        cache.preload("test", Client::new(service, "default")).await;
        cache
    }

    fn json_reply(status: u16, value: Value) -> http::Response<Body> {
        http::Response::builder()
            .status(status)
            .header("content-type", "application/json")
            .body(Full::new(Bytes::from(value.to_string())).boxed_unsync())
            .unwrap()
    }

    fn status(code: u16, reason: &str, message: &str) -> http::Response<Body> {
        json_reply(
            code,
            json!({
                "kind": "Status", "apiVersion": "v1", "metadata": {}, "status": "Failure",
                "reason": reason, "code": code, "message": message
            }),
        )
    }

    fn pod_json(p: &FakePod) -> Value {
        json!({
            "apiVersion": "v1", "kind": "Pod",
            "metadata": { "name": p.name, "namespace": "team", "uid": p.uid, "labels": p.labels },
            "spec": { "containers": p.containers.iter().map(|c| json!({ "name": c })).collect::<Vec<_>>() },
            "status": { "containerStatuses": p.containers.iter().map(|c| json!({
                "name": c, "restartCount": p.restarts, "ready": true, "image": "i", "imageID": "i"
            })).collect::<Vec<_>>() }
        })
    }

    fn answer(cluster: &Cluster, uri: &http::Uri) -> http::Response<Body> {
        let query: BTreeMap<String, String> =
            form_urlencoded::parse(uri.query().unwrap_or("").as_bytes())
                .into_owned()
                .collect();
        let parts: Vec<&str> = uri.path().trim_matches('/').split('/').collect();
        let pods = cluster.pods.lock().unwrap().clone();
        match parts.as_slice() {
            ["api", "v1", "namespaces", _, "pods"] => {
                // Equality terms only: what the tests' selectors spell.
                let wanted = query.get("labelSelector").cloned().unwrap_or_default();
                let items: Vec<Value> = pods
                    .iter()
                    .filter(|p| {
                        wanted.split(',').filter(|t| !t.is_empty()).all(|term| {
                            term.split_once('=').is_some_and(|(k, v)| {
                                p.labels.get(k).map(String::as_str) == Some(v)
                            })
                        })
                    })
                    .map(pod_json)
                    .collect();
                json_reply(
                    200,
                    json!({ "apiVersion": "v1", "kind": "PodList", "metadata": {}, "items": items }),
                )
            }
            ["api", "v1", "namespaces", _, "pods", name] => {
                match pods.iter().find(|p| p.name == *name) {
                    Some(p) => json_reply(200, pod_json(p)),
                    None => status(404, "NotFound", &format!("pods \"{name}\" not found")),
                }
            }
            ["api", "v1", "namespaces", _, "pods", name, "log"] => {
                match pods.iter().find(|p| p.name == *name) {
                    None => status(404, "NotFound", &format!("pods \"{name}\" not found")),
                    Some(FakePod {
                        fails: Some(why), ..
                    }) => status(400, "BadRequest", why),
                    Some(p) => log_reply(p, &query),
                }
            }
            ["apis", "apps", "v1", "namespaces", _, "deployments", name] => {
                match cluster.deployments.lock().unwrap().get(*name) {
                    Some(labels) => json_reply(
                        200,
                        json!({
                            "apiVersion": "apps/v1", "kind": "Deployment", "metadata": { "name": name },
                            "spec": { "selector": { "matchLabels": labels }, "template": {} }
                        }),
                    ),
                    None => status(
                        404,
                        "NotFound",
                        &format!("deployments.apps \"{name}\" not found"),
                    ),
                }
            }
            _ => status(404, "NotFound", uri.path()),
        }
    }

    /// A log as the kubelet answers one: from `sinceTime` (whole seconds) or
    /// the last `tailLines`, stamped when asked. A follow then hands out
    /// `later` and stays open.
    fn log_reply(p: &FakePod, query: &BTreeMap<String, String>) -> http::Response<Body> {
        let flag = |k: &str| query.get(k).is_some_and(|v| v == "true");
        let mut lines = if flag("previous") {
            p.previous.clone()
        } else {
            p.current.clone()
        };
        let follow = flag("follow");
        if follow {
            lines.extend(p.later.iter().cloned());
        }
        if let Some(since) = query.get("sinceTime") {
            let since: Timestamp = since.parse().unwrap();
            assert_eq!(
                since.subsec_nanosecond(),
                0,
                "the API server hands the kubelet whole seconds"
            );
            lines.retain(|(ts, _)| *ts >= since);
        }
        if let Some(tail) = query.get("tailLines") {
            let n: usize = tail.parse().unwrap();
            lines = lines.split_off(lines.len().saturating_sub(n));
        }
        let stamped = flag("timestamps");
        let frames = lines.into_iter().map(move |(ts, text)| {
            let line = if stamped {
                format!("{ts} {text}\n")
            } else {
                format!("{text}\n")
            };
            Ok::<_, std::convert::Infallible>(http_body::Frame::data(Bytes::from(line)))
        });
        let stream = futures::stream::iter(frames);
        let body = if follow {
            StreamBody::new(futures::StreamExt::chain(stream, futures::stream::pending()))
                .boxed_unsync()
        } else {
            StreamBody::new(stream).boxed_unsync()
        };
        http::Response::builder()
            .status(200)
            .header("content-type", "text/plain")
            .body(body)
            .unwrap()
    }

    /// `n` lines of `bytes` each, about 50 a second, every 7th sharing the
    /// stamp of the one before it.
    fn log_of(n: usize, bytes: usize) -> Vec<(Timestamp, String)> {
        (0..n)
            .map(|i| {
                let j = i - usize::from(i % 7 == 6);
                let ts = Timestamp::new(BASE + (j / 50) as i64, ((j % 50) * 20_000_000) as i32)
                    .unwrap();
                let mut text = format!("line {i:06} ");
                text.push_str(&"x".repeat(bytes.saturating_sub(text.len())));
                (ts, text)
            })
            .collect()
    }

    fn joined(lines: &[(Timestamp, String)]) -> String {
        lines.iter().map(|(_, text)| format!("{text}\n")).collect()
    }

    async fn page(cache: &ClientCache, args: Value) -> Result<Value, CapabilityError> {
        read_page(cache, input(args))
            .await
            .map(|out| serde_json::to_value(out).unwrap())
    }

    /// Every page from `args` on, until one says `end`: what they held, put
    /// together, and the pages.
    async fn read_to_end(cache: &ClientCache, mut args: Value) -> (String, Vec<Value>) {
        let mut read = String::new();
        let mut pages = Vec::new();
        loop {
            let out = page(cache, args.clone()).await.unwrap();
            read.push_str(out["logs"].as_str().unwrap());
            args["cursor"] = out["next_cursor"].clone();
            let end = out["end"] == true;
            pages.push(out);
            if end {
                return (read, pages);
            }
            assert!(pages.len() < 500, "the read never ended");
        }
    }

    fn one_pod(lines: Vec<(Timestamp, String)>) -> Arc<Cluster> {
        let cluster = Arc::new(Cluster::default());
        cluster.pods.lock().unwrap().push(pod("cmi-0", lines));
        cluster
    }

    fn read_of(extra: Value) -> Value {
        with(
            json!({ "context": "test", "namespace": "team", "pod": "cmi-0" }),
            extra,
        )
    }

    #[tokio::test]
    async fn a_3_mb_log_pages_to_its_end_with_every_answer_under_the_cap() {
        let log = log_of(30_000, 100);
        let cache = cache_for(one_pod(log.clone())).await;
        let (read, pages) =
            read_to_end(&cache, read_of(json!({ "max_bytes": MAX_PAGE_BYTES }))).await;
        for (i, out) in pages.iter().enumerate() {
            let size = out.to_string().len();
            assert!(size <= 50_000, "page {i} was {size} bytes");
        }
        assert!(pages.len() > 60, "{} pages for 3 MB", pages.len());
        assert!(
            read == joined(&log),
            "the pages put together are not the log: {} of {} bytes",
            read.len(),
            joined(&log).len()
        );
    }

    /// Lines sharing one stamp split across page edges are each read once:
    /// the cursor counts how many lines with its stamp were read.
    #[tokio::test]
    async fn lines_sharing_one_stamp_split_across_pages_are_each_read_once() {
        let log: Vec<_> = (0..10)
            .map(|i| (at(0, 500), format!("{i} {}", "y".repeat(300))))
            .collect();
        let cache = cache_for(one_pod(log.clone())).await;
        let (read, pages) = read_to_end(&cache, read_of(json!({ "max_bytes": 1000 }))).await;
        assert!(pages.len() >= 4, "{} pages", pages.len());
        assert_eq!(read, joined(&log));
    }

    #[tokio::test]
    async fn stamps_are_shown_only_when_asked_for() {
        let log = vec![(at(0, 250), "one".to_string()), (at(1, 0), "two".to_string())];
        let cache = cache_for(one_pod(log)).await;
        let plain = page(&cache, read_of(json!({ "max_bytes": 1000 })))
            .await
            .unwrap();
        assert_eq!(plain["logs"], "one\ntwo\n");
        let stamped = page(&cache, read_of(json!({ "max_bytes": 1000, "timestamps": true })))
            .await
            .unwrap();
        assert_eq!(
            stamped["logs"],
            format!("{} one\n{} two\n", at(0, 250), at(1, 0))
        );
    }

    /// Review focus 1: a line longer than the page, of characters wider than
    /// a byte, is cut on a character boundary within the budget, marked, and
    /// the read moves on to the next line.
    #[tokio::test]
    async fn a_line_longer_than_the_page_is_cut_and_the_read_moves_on() {
        let log = vec![
            (at(0, 0), "日本語".repeat(1_000)),
            (at(1, 0), "after".to_string()),
        ];
        let cache = cache_for(one_pod(log)).await;
        let first = page(&cache, read_of(json!({ "max_bytes": 1000 })))
            .await
            .unwrap();
        let shown = first["logs"].as_str().unwrap();
        assert!(
            shown.starts_with("日本語") && shown.ends_with(&format!("{CUT}\n")),
            "{shown}"
        );
        assert!(escaped_len(shown) <= 1000, "{}", escaped_len(shown));
        assert_eq!(first["lines"], 1);
        let next = page(
            &cache,
            read_of(json!({ "max_bytes": 1000, "cursor": first["next_cursor"] })),
        )
        .await
        .unwrap();
        assert_eq!(next["logs"], "after\n");
    }

    #[tokio::test]
    async fn a_since_time_start_drops_earlier_lines_within_its_second() {
        let log = vec![
            (at(0, 100), "before".to_string()),
            (at(0, 600), "after".to_string()),
        ];
        let cache = cache_for(one_pod(log)).await;
        let out = page(
            &cache,
            read_of(json!({ "since_time": at(0, 500).to_string() })),
        )
        .await
        .unwrap();
        assert_eq!(out["logs"], "after\n");
    }

    /// Review focus 2: polling with the last cursor and no wait gives an
    /// empty page that says `end`, and repeats nothing.
    #[tokio::test]
    async fn polling_past_the_end_repeats_nothing() {
        let cache = cache_for(one_pod(log_of(20, 50))).await;
        let (_, pages) = read_to_end(&cache, read_of(json!({ "max_bytes": 1000 }))).await;
        let cursor = pages.last().unwrap()["next_cursor"].clone();
        for _ in 0..2 {
            let again = page(
                &cache,
                read_of(json!({ "max_bytes": 1000, "cursor": cursor })),
            )
            .await
            .unwrap();
            assert_eq!(again["logs"], "");
            assert_eq!(again["end"], true);
        }
    }

    /// Review focus 3: an agent resends its first call's start fields with
    /// the cursor; the cursor wins and the read goes on rather than
    /// restarting.
    #[tokio::test]
    async fn a_cursor_wins_over_the_start_fields_resent_with_it() {
        let log = log_of(40, 50);
        let cache = cache_for(one_pod(log.clone())).await;
        let first = page(&cache, read_of(json!({ "max_bytes": 1000, "tail_lines": 30 })))
            .await
            .unwrap();
        let shown = first["lines"].as_u64().unwrap() as usize;
        let next = page(
            &cache,
            read_of(json!({
                "max_bytes": 1000, "tail_lines": 30, "since_time": at(0, 0).to_string(),
                "cursor": first["next_cursor"]
            })),
        )
        .await
        .unwrap();
        assert!(
            next["logs"].as_str().unwrap().starts_with(&log[10 + shown].1),
            "{next}"
        );
    }

    /// Through the capability: page mode answers with a cursor.
    #[tokio::test]
    async fn the_capability_routes_page_mode_here() {
        let cache = cache_for(one_pod(log_of(5, 20))).await;
        let cap = crate::logs::pod_logs_capability(cache);
        let out = (cap.handler)(read_of(json!({ "max_bytes": 1000 })))
            .await
            .unwrap();
        assert!(out["next_cursor"].is_string(), "{out}");
        assert_eq!(out["end"], true);
    }

    #[tokio::test]
    async fn grep_returns_only_the_matching_lines_of_the_whole_log() {
        let mut log = log_of(5_000, 80);
        log[1234].1 = "POST /webhook 502 upstream reset".into();
        log[4321].1 = "POST /webhook 504 timeout".into();
        let cache = cache_for(one_pod(log)).await;
        let out = page(&cache, read_of(json!({ "grep": "webhook 50[0-9]" })))
            .await
            .unwrap();
        assert_eq!(
            out["logs"],
            "POST /webhook 502 upstream reset\nPOST /webhook 504 timeout\n"
        );
        assert_eq!(out["lines"], 2);
        assert_eq!(out["scanned"], 5_000);
        assert_eq!(out["end"], true);
    }

    #[tokio::test]
    async fn context_lines_keep_the_neighbours_of_a_match() {
        let log: Vec<_> = ["a", "b", "MATCH", "c", "d", "e", "f", "MATCH again", "g"]
            .iter()
            .enumerate()
            .map(|(i, text)| (at(i as i64, 0), text.to_string()))
            .collect();
        let cache = cache_for(one_pod(log)).await;
        let out = page(
            &cache,
            read_of(json!({ "grep": "(?i)match", "context_lines": 1 })),
        )
        .await
        .unwrap();
        assert_eq!(out["logs"], "b\nMATCH\nc\nf\nMATCH again\ng\n");
    }

    #[tokio::test]
    async fn a_cursor_whose_line_was_rotated_away_carries_on_and_says_so() {
        let log = log_of(100, 100);
        let cluster = one_pod(log.clone());
        let cache = cache_for(cluster.clone()).await;
        let first = page(&cache, read_of(json!({ "max_bytes": 1000 })))
            .await
            .unwrap();
        let read = first["lines"].as_u64().unwrap() as usize;
        // The kubelet rotates the file: what it serves now starts 40 lines on.
        cluster.pods.lock().unwrap()[0].current = log[40..].to_vec();
        let next = page(
            &cache,
            read_of(json!({ "max_bytes": 1000, "cursor": first["next_cursor"] })),
        )
        .await
        .unwrap();
        let notice = next["notices"][0].as_str().unwrap();
        assert!(
            notice.starts_with("cmi-0: ") && notice.contains("rotated away"),
            "{notice}"
        );
        assert!(
            notice.contains(&log[read - 1].0.to_string()),
            "names where the cursor stopped: {notice}"
        );
        assert!(
            notice.contains(&log[40].0.to_string()),
            "names the first line readable: {notice}"
        );
        assert!(
            next["logs"].as_str().unwrap().starts_with(&log[40].1),
            "{next}"
        );
    }

    #[tokio::test]
    async fn a_restart_under_the_cursor_reads_the_rest_of_the_old_instance_then_the_new_one() {
        let old = log_of(30, 60);
        let new: Vec<_> = log_of(5, 40)
            .into_iter()
            .map(|(ts, text)| (ts + SignedDuration::from_hours(1), format!("new {text}")))
            .collect();
        let cluster = one_pod(old[..10].to_vec());
        let cache = cache_for(cluster.clone()).await;
        let first = page(&cache, read_of(json!({ "max_bytes": MAX_PAGE_BYTES })))
            .await
            .unwrap();
        assert_eq!(first["logs"], joined(&old[..10]));
        {
            let mut pods = cluster.pods.lock().unwrap();
            pods[0].previous = old.clone();
            pods[0].current = new.clone();
            pods[0].restarts = 1;
        }
        let next = page(
            &cache,
            read_of(json!({ "max_bytes": MAX_PAGE_BYTES, "cursor": first["next_cursor"] })),
        )
        .await
        .unwrap();
        assert_eq!(
            next["logs"].as_str().unwrap(),
            joined(&old[10..]) + &joined(&new)
        );
        assert!(
            next["notices"][0].as_str().unwrap().contains("restarted"),
            "{next}"
        );
    }

    #[tokio::test]
    async fn two_restarts_under_the_cursor_lose_the_lines_between_and_say_so() {
        let cluster = one_pod(log_of(10, 60));
        let cache = cache_for(cluster.clone()).await;
        let first = page(&cache, read_of(json!({ "max_bytes": MAX_PAGE_BYTES })))
            .await
            .unwrap();
        let new: Vec<_> = log_of(3, 40)
            .into_iter()
            .map(|(ts, text)| (ts + SignedDuration::from_hours(1), text))
            .collect();
        {
            let mut pods = cluster.pods.lock().unwrap();
            pods[0].current = new.clone();
            pods[0].restarts = 2;
        }
        let next = page(
            &cache,
            read_of(json!({ "max_bytes": MAX_PAGE_BYTES, "cursor": first["next_cursor"] })),
        )
        .await
        .unwrap();
        assert_eq!(next["logs"], joined(&new));
        assert!(
            next["notices"][0]
                .as_str()
                .unwrap()
                .contains("more than once"),
            "{next}"
        );
    }

    #[tokio::test]
    async fn a_pod_replaced_under_the_cursor_is_read_from_its_first_line() {
        let cluster = one_pod(log_of(10, 60));
        let cache = cache_for(cluster.clone()).await;
        let first = page(&cache, read_of(json!({ "max_bytes": MAX_PAGE_BYTES })))
            .await
            .unwrap();
        let new = log_of(4, 40);
        {
            let mut pods = cluster.pods.lock().unwrap();
            pods[0].uid = "uid-cmi-0-again".into();
            pods[0].current = new.clone();
        }
        let next = page(
            &cache,
            read_of(json!({ "max_bytes": MAX_PAGE_BYTES, "cursor": first["next_cursor"] })),
        )
        .await
        .unwrap();
        assert_eq!(next["logs"], joined(&new));
        assert!(
            next["notices"][0].as_str().unwrap().contains("replaced"),
            "{next}"
        );
    }

    fn by_selector(extra: Value) -> Value {
        with(
            json!({ "context": "test", "namespace": "team", "selector": { "app": "cmi" } }),
            extra,
        )
    }

    fn two_pods() -> Arc<Cluster> {
        let t = |s: i64| at(s, 0);
        let cluster = Arc::new(Cluster::default());
        cluster.pods.lock().unwrap().extend([
            pod(
                "cmi-a",
                vec![(t(0), "a0".into()), (t(2), "a2".into()), (t(4), "a4".into())],
            ),
            pod("cmi-b", vec![(t(1), "b1".into()), (t(3), "b3".into())]),
        ]);
        cluster
    }

    #[tokio::test]
    async fn a_selectors_pods_are_merged_in_time_order_each_line_naming_its_pod() {
        let cache = cache_for(two_pods()).await;
        let out = page(&cache, by_selector(json!({}))).await.unwrap();
        assert_eq!(
            out["logs"],
            "[cmi-a] a0\n[cmi-b] b1\n[cmi-a] a2\n[cmi-b] b3\n[cmi-a] a4\n"
        );
        assert_eq!(out["end"], true);
    }

    #[tokio::test]
    async fn a_workloads_pods_are_read_through_its_selector() {
        let cluster = two_pods();
        cluster
            .deployments
            .lock()
            .unwrap()
            .insert("cmi".into(), [("app".to_string(), "cmi".to_string())].into());
        let cache = cache_for(cluster).await;
        let out = page(
            &cache,
            json!({
                "context": "test", "namespace": "team",
                "workload": { "kind": "Deployment", "name": "cmi" }, "grep": "[ab][24]"
            }),
        )
        .await
        .unwrap();
        assert_eq!(out["logs"], "[cmi-a] a2\n[cmi-a] a4\n");
    }

    #[tokio::test]
    async fn more_pods_than_one_call_reads_are_refused_with_the_count() {
        let cluster = Arc::new(Cluster::default());
        cluster
            .pods
            .lock()
            .unwrap()
            .extend((0..21).map(|i| pod(&format!("cmi-{i}"), vec![])));
        let err = page(&*cache_for(cluster).await, by_selector(json!({})))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("21 pods match") && err.contains("20"), "{err}");
    }

    #[tokio::test]
    async fn a_pod_of_several_that_cannot_be_read_is_a_notice_and_the_rest_are_read() {
        let cluster = two_pods();
        cluster.pods.lock().unwrap()[1].fails = Some(
            "container \"app\" in pod \"cmi-b\" is waiting to start: ContainerCreating".into(),
        );
        let out = page(&*cache_for(cluster).await, by_selector(json!({})))
            .await
            .unwrap();
        assert_eq!(out["logs"], "[cmi-a] a0\n[cmi-a] a2\n[cmi-a] a4\n");
        let notice = out["notices"][0].as_str().unwrap();
        assert!(
            notice.starts_with("cmi-b: could not read its log") && notice.contains("waiting to start"),
            "{notice}"
        );
    }

    /// Review focus 4: one named pod that cannot be read is the call's error,
    /// with the API's own reason, not an empty page.
    #[tokio::test]
    async fn a_single_pod_that_cannot_be_read_is_the_calls_error() {
        let cluster = one_pod(vec![]);
        cluster.pods.lock().unwrap()[0].fails = Some(
            "container \"app\" in pod \"cmi-0\" is waiting to start: ContainerCreating".into(),
        );
        let err = page(&*cache_for(cluster).await, read_of(json!({ "max_bytes": 1000 })))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("waiting to start"), "{err}");
    }

    #[tokio::test]
    async fn pods_gone_or_new_since_the_cursor_are_said_and_the_new_one_read_from_its_start() {
        let cluster = two_pods();
        let cache = cache_for(cluster.clone()).await;
        let first = page(&cache, by_selector(json!({}))).await.unwrap();
        {
            let mut pods = cluster.pods.lock().unwrap();
            pods.retain(|p| p.name != "cmi-b");
            pods.push(pod("cmi-c", vec![(at(10, 0), "c10".into())]));
        }
        let next = page(&cache, by_selector(json!({ "cursor": first["next_cursor"] })))
            .await
            .unwrap();
        assert_eq!(next["logs"], "[cmi-c] c10\n");
        let notices: Vec<&str> = next["notices"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_str().unwrap())
            .collect();
        assert!(notices.iter().any(|n| n.starts_with("cmi-b: gone")), "{notices:?}");
        assert!(notices.iter().any(|n| n.starts_with("cmi-c: new")), "{notices:?}");
    }

    #[tokio::test]
    async fn which_container_to_read_across_several_pods() {
        let cluster = two_pods();
        cluster.pods.lock().unwrap()[0].containers = vec!["app".into(), "sidecar".into()];
        let cache = cache_for(cluster).await;
        let err = page(&cache, by_selector(json!({})))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("app, sidecar") && err.contains("`container`"), "{err}");
        let out = page(&cache, by_selector(json!({ "container": "sidecar" })))
            .await
            .unwrap();
        assert_eq!(out["logs"], "[cmi-a] a0\n[cmi-a] a2\n[cmi-a] a4\n");
        assert!(
            out["notices"][0]
                .as_str()
                .unwrap()
                .starts_with("cmi-b: has no container \"sidecar\""),
            "{out}"
        );
    }

    #[tokio::test]
    async fn a_selector_matching_no_pod_reads_an_empty_page_to_its_end() {
        let cache = cache_for(two_pods()).await;
        let out = page(
            &cache,
            json!({ "context": "test", "namespace": "team", "selector": { "app": "nothing" } }),
        )
        .await
        .unwrap();
        assert_eq!(out["logs"], "");
        assert_eq!(out["end"], true);
    }

    #[tokio::test(start_paused = true)]
    async fn wait_seconds_returns_lines_written_after_the_backlog() {
        let cluster = one_pod(log_of(3, 30));
        cluster.pods.lock().unwrap()[0].later = vec![
            (at(100, 0), "later one".to_string()),
            (at(101, 0), "later two".to_string()),
        ];
        let cache = cache_for(cluster).await;
        let first = page(&cache, read_of(json!({ "max_bytes": 1000 })))
            .await
            .unwrap();
        assert_eq!(first["end"], true);
        let next = page(
            &cache,
            read_of(json!({ "max_bytes": 1000, "wait_seconds": 5, "cursor": first["next_cursor"] })),
        )
        .await
        .unwrap();
        assert_eq!(next["logs"], "later one\nlater two\n");
        assert_eq!(next["end"], true);
        // The follow's own cursor reads on past what it handed out.
        let after = page(
            &cache,
            read_of(json!({ "max_bytes": 1000, "cursor": next["next_cursor"] })),
        )
        .await
        .unwrap();
        assert_eq!(after["logs"], "");
    }

    #[tokio::test(start_paused = true)]
    async fn a_follow_of_several_pods_hands_on_each_ones_new_lines() {
        let cluster = two_pods();
        {
            let mut pods = cluster.pods.lock().unwrap();
            pods[0].later = vec![(at(50, 0), "a50".into())];
            pods[1].later = vec![(at(51, 0), "b51".into())];
        }
        let cache = cache_for(cluster).await;
        let first = page(&cache, by_selector(json!({}))).await.unwrap();
        let next = page(
            &cache,
            by_selector(json!({ "wait_seconds": 3, "cursor": first["next_cursor"] })),
        )
        .await
        .unwrap();
        let shown = next["logs"].as_str().unwrap();
        assert!(
            shown.contains("[cmi-a] a50\n") && shown.contains("[cmi-b] b51\n"),
            "{shown}"
        );
    }

    /// Review focus 5: 20 pods with names as long as a workload's get, lines
    /// of quotes and ANSI colour codes (2 and 6 bytes each once escaped), half
    /// the pods failing with long reasons, and a cursor naming 20 more pods
    /// now gone — still one answer under the cap, its notices capped and
    /// counted.
    #[tokio::test]
    async fn a_worst_case_page_stays_under_the_answer_cap() {
        let cluster = Arc::new(Cluster::default());
        let name = |i: usize| format!("{}-{i:02}", "c".repeat(60));
        let line = "\"\u{1b}[31m".repeat(40);
        for i in 0..20 {
            let mut p = pod(&name(i), (0..200).map(|j| (at(j, 0), line.clone())).collect());
            if i % 2 == 1 {
                p.fails = Some("x".repeat(1_500));
            }
            cluster.pods.lock().unwrap().push(p);
        }
        let cache = cache_for(cluster).await;
        let mut args = by_selector(json!({ "max_bytes": MAX_PAGE_BYTES }));
        let target = Target::Selector {
            labels: [("app".to_string(), "cmi".to_string())].into(),
            expressions: vec![],
        };
        let gone = Cursor {
            v: CURSOR_VERSION,
            t: fingerprint(&input(args.clone()), &target),
            p: (0..20)
                .map(|i| {
                    (
                        format!("{}-gone-{i:02}", "g".repeat(55)),
                        Pos {
                            u: "u".into(),
                            r: 0,
                            t: Some(at(0, 0).to_string()),
                            k: 1,
                        },
                    )
                })
                .collect(),
        };
        args["cursor"] = json!(gone.encode());
        let out = page(&cache, args).await.unwrap();
        let size = out.to_string().len();
        assert!(size <= MAX_ANSWER_BYTES, "{size} bytes");
        let notices = out["notices"].as_array().unwrap();
        assert_eq!(notices.len(), MAX_NOTICES + 1);
        assert!(notices[MAX_NOTICES]
            .as_str()
            .unwrap()
            .contains("more notices not shown"));
        assert!(notices
            .iter()
            .all(|n| escaped_len(n.as_str().unwrap()) <= MAX_NOTICE_BYTES));
        assert!(out["lines"].as_u64().unwrap() > 0);
    }
}
