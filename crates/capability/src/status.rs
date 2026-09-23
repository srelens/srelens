//! Status rules (#541): what an app says an object's status is.
//!
//! An app declares ordered rules; the first whose conditions all hold names
//! one of six normalized statuses, the word shown for it, and optionally where
//! in the object the reason is. The same rules back a custom resource's status
//! (`statusResolvers`) and a badge on a built-in row (`badges`).
//!
//! In the lowest crate beside [`crate::Predicate`], for the same reason: the
//! rule is checked where a manifest is accepted (`srelens-plugin-host`) and
//! evaluated where the objects are (`srelens-kube`'s custom-resource list, the
//! registry's badge resolver), and those must not drift. The conditions ARE
//! predicates' — [`Condition`] — so there is one path grammar and one
//! evaluator, not a second one for status.

use crate::predicate::{check_path, resolve, Condition};
use crate::text::escape_invisible;
use crate::MAX_PREDICATES;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Most rules one resolver or badge may declare.
pub const MAX_RULES: usize = 16;

/// The longest word a rule may show. A badge sits in a table cell.
pub const MAX_LABEL_CHARS: usize = 40;

/// The longest reason a resolved status carries, in characters.
pub const MAX_RESOLVED_REASON_CHARS: usize = 200;

/// The six statuses every surface knows how to draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum NormalizedStatus {
    Healthy,
    Warning,
    Error,
    Progressing,
    Suspended,
    Unknown,
}

/// One rule: when every condition holds, the object has this status.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StatusRule {
    /// Every condition must hold. An empty list always holds, which is how a
    /// last, catch-all rule is written.
    pub when: Vec<Condition>,
    pub status: NormalizedStatus,
    /// The word shown. Required: colour is never the only signal.
    pub label: String,
    /// Where in the object the reason is, e.g.
    /// `.status.conditions[?(@.type=="Ready")].message`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// What one object resolved to. The label and reason are escaped: they come
/// from a manifest and from the cluster, and are drawn as text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ResolvedStatus {
    pub status: NormalizedStatus,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Every problem with a declared rule list, each at its path relative to the
/// list's owner (`rules`, `rules[1].label`, `rules[1].when[0]`, ...).
///
/// The single statement of the rules: the manifest validator reports these at
/// the fields a person fixes, and [`first_match`] refuses to match a list with
/// any, so a list that could not be installed cannot describe an object either.
pub fn rule_problems(rules: &[StatusRule]) -> Vec<(String, String)> {
    let mut problems = Vec::new();
    if rules.is_empty() || rules.len() > MAX_RULES {
        problems.push((
            "rules".to_owned(),
            format!("Declare 1–{MAX_RULES} rules"),
        ));
        if rules.len() > MAX_RULES {
            return problems;
        }
    }
    for (index, rule) in rules.iter().enumerate() {
        let at = format!("rules[{index}]");
        if rule.when.len() > MAX_PREDICATES {
            problems.push((
                format!("{at}.when"),
                format!("A rule has at most {MAX_PREDICATES} conditions"),
            ));
        } else {
            for (position, condition) in rule.when.iter().enumerate() {
                if let Err(why) = condition.check() {
                    problems.push((format!("{at}.when[{position}]"), why));
                }
            }
        }
        if !word(&rule.label) {
            problems.push((
                format!("{at}.label"),
                format!(
                    "A rule shows a word: 1–{MAX_LABEL_CHARS} characters, with no control or invisible format characters"
                ),
            ));
        }
        if let Some(path) = &rule.reason {
            if let Err(why) = check_path(path) {
                problems.push((format!("{at}.reason"), why));
            }
        }
    }
    problems
}

/// Whether `label` is a word a surface can show: not blank, short, and with
/// nothing that draws differently from what it holds.
fn word(label: &str) -> bool {
    let trimmed = label.trim();
    !trimmed.is_empty()
        && label.chars().count() <= MAX_LABEL_CHARS
        && !label
            .chars()
            .any(|c| c.is_control() || crate::is_format_character(c))
}

/// The first rule that holds for `object`, resolved; `None` when none does.
///
/// Fails closed: a rule list the host would refuse at install matches
/// nothing, so a typo in a path never becomes a status.
pub fn first_match(rules: &[StatusRule], object: &Value) -> Option<ResolvedStatus> {
    if !rule_problems(rules).is_empty() {
        return None;
    }
    let rule = rules
        .iter()
        .find(|rule| rule.when.iter().all(|condition| condition.holds(object)))?;
    Some(ResolvedStatus {
        status: rule.status,
        label: escape_invisible(rule.label.trim()),
        reason: rule
            .reason
            .as_deref()
            .and_then(|path| reason_at(object, path)),
    })
}

/// The reason at `path`, as one bounded, escaped line; `None` for a value
/// that is not a scalar or says nothing.
fn reason_at(object: &Value, path: &str) -> Option<String> {
    let text = match resolve(object, path)? {
        Value::String(text) => text.trim().to_owned(),
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        _ => return None,
    };
    if text.is_empty() {
        return None;
    }
    let mut shown: String = text.chars().take(MAX_RESOLVED_REASON_CHARS).collect();
    if shown.len() < text.len() {
        shown.push('…');
    }
    Some(escape_invisible(&shown))
}

/// The status of `object`: the first rule that holds, or `unknown`.
pub fn resolve_status(rules: &[StatusRule], object: &Value) -> ResolvedStatus {
    first_match(rules, object).unwrap_or_else(|| ResolvedStatus {
        status: NormalizedStatus::Unknown,
        label: "Unknown".into(),
        reason: None,
    })
}

/// [`resolve_status`] for each object, in order.
pub fn resolve_statuses<'a>(
    rules: &[StatusRule],
    objects: impl IntoIterator<Item = &'a Value>,
) -> Vec<ResolvedStatus> {
    objects
        .into_iter()
        .map(|object| resolve_status(rules, object))
        .collect()
}
