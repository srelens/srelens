//! The notification center: a bounded history behind the one-slot toast, and a
//! watcher that turns pod transitions into alerts.
//!
//! The toast says "something just happened" for a few seconds and is then gone,
//! and a second toast overwrites the first. This keeps what the toast loses, and
//! raises alerts nobody asked for: a pod that starts crash-looping, a pod that
//! restarts, a pod that recovers.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Instant;

use ratatui::style::Style;
use serde_json::Value;

use crate::theme::Theme;

/// How many notifications are kept; the oldest fall off.
const CAPACITY: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Success,
    Warn,
    Error,
}

impl Level {
    /// Warnings and errors are the ones that wait for you.
    fn needs_attention(self) -> bool {
        matches!(self, Level::Warn | Level::Error)
    }

    /// The level a toast's style stands for. Styles are all the toast carries.
    pub fn from_style(style: Style) -> Level {
        if style == Theme::status_error() {
            Level::Error
        } else if style == Theme::status_warn() {
            Level::Warn
        } else if style == Theme::status_ok() {
            Level::Success
        } else {
            Level::Info
        }
    }

    pub fn style(self) -> Style {
        match self {
            Level::Error => Theme::status_error(),
            Level::Warn => Theme::status_warn(),
            Level::Success => Theme::status_ok(),
            Level::Info => Theme::status_dim(),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Level::Error => "ERROR",
            Level::Warn => "WARN ",
            Level::Success => "OK   ",
            Level::Info => "INFO ",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Notification {
    pub level: Level,
    pub source: String,
    pub message: String,
    /// When it last happened; a repeat refreshes this.
    pub at: Instant,
    /// How many times in a row this exact message was raised.
    pub count: u32,
    /// How many of those times have not been seen on screen yet (warnings and
    /// errors only).
    unseen: u32,
    /// The value of `NotificationCenter::raised()` when it last happened.
    seq: u64,
}

/// What the watcher remembers about one pod between refreshes.
#[derive(Debug, Clone, PartialEq)]
struct PodState {
    problem: Option<String>,
    restarts: i64,
}

#[derive(Debug, Default)]
pub struct NotificationCenter {
    items: VecDeque<Notification>,
    unread: usize,
    /// How many of the unread ones are errors, for the badge colour.
    unread_errors: usize,
    pods: HashMap<(String, String, String), PodState>,
    /// (context, namespace) pairs seen at least once. The first look only sets
    /// a baseline, so opening a cluster that is already on fire is not 40 alerts.
    baselined: HashSet<(String, String)>,
    /// Every `push` ever, repeats included, so a caller can tell how many a
    /// call raised even when a repeat only bumped a count.
    total_raised: u64,
}

impl NotificationCenter {
    /// Newest first.
    pub fn items(&self) -> impl Iterator<Item = &Notification> {
        self.items.iter()
    }

    /// Warnings and errors raised since the history was last opened.
    pub fn unread(&self) -> usize {
        self.unread
    }

    /// An unread alert is an error: the badge turns red rather than yellow.
    pub fn unread_has_error(&self) -> bool {
        self.unread_errors > 0
    }

    pub fn mark_read(&mut self) {
        self.mark_seen(usize::MAX, u64::MAX);
    }

    /// The newest `depth` entries are on screen: they have been seen. Older
    /// unread ones stay unread until the reader scrolls to them.
    ///
    /// Only entries raised at or before `upto` (a value of `raised()`) count:
    /// one that lands while the overlay is already open was not there to be
    /// read when it opened, and stays unread.
    pub fn mark_seen(&mut self, depth: usize, upto: u64) {
        for n in self.items.iter_mut().take(depth) {
            if n.seq <= upto {
                n.unseen = 0;
            }
        }
        self.recount();
    }

    /// How many alerts have ever been raised; a bookmark for `mark_seen`.
    pub fn raised(&self) -> u64 {
        self.total_raised
    }

    fn recount(&mut self) {
        self.unread = self.items.iter().map(|n| n.unseen as usize).sum();
        self.unread_errors = self
            .items
            .iter()
            .filter(|n| n.level == Level::Error)
            .map(|n| n.unseen as usize)
            .sum();
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.unread = 0;
        self.unread_errors = 0;
    }

    pub fn push(&mut self, level: Level, source: &str, message: impl Into<String>) {
        let message = message.into();
        self.total_raised += 1;
        let unseen = u32::from(level.needs_attention());
        if let Some(latest) = self.items.front_mut() {
            if latest.level == level && latest.source == source && latest.message == message {
                latest.count += 1;
                latest.unseen += unseen;
                latest.seq = self.total_raised;
                latest.at = Instant::now();
                self.recount();
                return;
            }
        }
        self.items.push_front(Notification {
            level,
            source: source.to_string(),
            message,
            at: Instant::now(),
            count: 1,
            unseen,
            seq: self.total_raised,
        });
        self.items.truncate(CAPACITY);
        self.recount();
    }

    /// Compare this refresh of a namespace's pods with the last one and raise
    /// an alert for each pod that got worse, restarted, or recovered.
    ///
    /// Returns the alerts it raised this time, oldest first, so the caller can
    /// pop the newest up as a toast. They are already in the history.
    pub fn observe_pods(
        &mut self,
        context: &str,
        namespace: &str,
        items: &[Value],
    ) -> Vec<(Level, String)> {
        let before = self.total_raised;
        let scope = (context.to_string(), namespace.to_string());
        let first_look = self.baselined.insert(scope);
        let mut present: std::collections::HashSet<(String, String, String)> =
            std::collections::HashSet::with_capacity(items.len());
        for item in items {
            let Some((ns, name, state)) = pod_state(item, namespace) else {
                continue;
            };
            let key = (context.to_string(), ns.clone(), name.clone());
            present.insert(key.clone());
            let previous = self.pods.insert(key, state.clone());
            if first_look {
                continue;
            }
            let who = format!("{ns}/{name}");
            match (previous.as_ref().and_then(|p| p.problem.as_ref()), &state.problem) {
                // Healthy (or just born) -> broken. Not broken -> a different
                // kind of broken: Kubernetes flips a failing pod between Error
                // and CrashLoopBackOff (ErrImagePull and ImagePullBackOff) on
                // every back-off, and that is one problem, not many.
                (None, Some(now)) => {
                    self.push(Level::Error, context, format!("pod {who}: {now}"));
                }
                (Some(_), None) => {
                    self.push(Level::Success, context, format!("pod {who} recovered"));
                }
                _ => {
                    if let Some(prev) = &previous {
                        if state.problem.is_none() && state.restarts > prev.restarts {
                            self.push(
                                Level::Warn,
                                context,
                                format!("pod {who} restarted ({} total)", state.restarts),
                            );
                        }
                    }
                }
            }
        }
        // A snapshot omits deleted pods. Forget the ones this snapshot covers
        // that are no longer in it, so a pod recreated under the same name
        // (a StatefulSet's) starts from nothing instead of inheriting the old
        // one's failure, and the map does not grow as pods come and go.
        // An empty namespace is the all-namespaces watch: it covers them all.
        self.pods.retain(|key, _| {
            let covered = key.0 == context && (namespace.is_empty() || key.1 == namespace);
            !covered || present.contains(key)
        });
        let fresh = (self.total_raised - before) as usize;
        let mut raised: Vec<(Level, String)> = self
            .items
            .iter()
            .take(fresh.min(self.items.len()))
            .map(|n| (n.level, n.message.clone()))
            .collect();
        raised.reverse();
        raised
    }
}

/// Reasons that mean a pod is not going to get better on its own.
fn is_problem(reason: &str) -> bool {
    let reason = reason.strip_prefix("Init:").unwrap_or(reason);
    matches!(
        reason,
        "CrashLoopBackOff"
            | "ImagePullBackOff"
            | "ErrImagePull"
            | "InvalidImageName"
            | "CreateContainerConfigError"
            | "CreateContainerError"
            | "RunContainerError"
            | "OOMKilled"
            | "Error"
            | "Failed"
            | "Evicted"
    )
}

/// `(namespace, name, state)` from either a flat watch row
/// (`name`/`namespace`/`status`/`restarts`) or a raw Pod object.
fn pod_state(item: &Value, default_ns: &str) -> Option<(String, String, PodState)> {
    let name = item
        .get("name")
        .or_else(|| item.pointer("/metadata/name"))?
        .as_str()?
        .to_string();
    let ns = item
        .get("namespace")
        .or_else(|| item.pointer("/metadata/namespace"))
        .and_then(Value::as_str)
        .unwrap_or(default_ns)
        .to_string();

    let mut problem = item
        .get("status")
        .and_then(Value::as_str)
        .filter(|s| is_problem(s))
        .map(String::from);
    let mut restarts = item.get("restarts").and_then(Value::as_i64).unwrap_or(0);

    if let Some(statuses) = item
        .pointer("/status/containerStatuses")
        .and_then(Value::as_array)
    {
        restarts = statuses
            .iter()
            .filter_map(|c| c.get("restartCount").and_then(Value::as_i64))
            .sum();
        for c in statuses {
            let reason = c
                .pointer("/state/waiting/reason")
                .or_else(|| c.pointer("/state/terminated/reason"))
                .and_then(Value::as_str);
            if let Some(r) = reason.filter(|r| is_problem(r)) {
                problem = Some(r.to_string());
                break;
            }
        }
    }
    if problem.is_none() && item.pointer("/status/phase").and_then(Value::as_str) == Some("Failed")
    {
        problem = Some("Failed".to_string());
    }
    Some((ns, name, PodState { problem, restarts }))
}
