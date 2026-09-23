//! Declarative predicates: the checks an extension's action declares about the
//! object it is about to write (#550).
//!
//! One statement of the rule, in the lowest crate, because the same predicate
//! is read in three places that must not drift: a manifest's `preconditions`
//! and `availableWhen` are checked where a binding is accepted — at install
//! and again at reverification (`srelens-plugin-host`) — the host evaluates
//! the preconditions against the fresh GET before it patches (`srelens-kube`'s
//! action primitives), and the surface evaluates `availableWhen` to decide
//! whether a control is offered at all.
//!
//! What a predicate can say is deliberately small. The comparand is a literal
//! written into the manifest, the path is a fixed address into the object, and
//! there is no way to name a second object, call a function or write a filter:
//! a predicate is one question about one value, asked the same way every time.
//! That is what makes it checkable at install rather than only when it runs.

use crate::text::escape_invisible;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Most predicates one declared list may hold. A declared action states its
/// conditions; a rule set longer than this is a controller's job.
pub const MAX_PREDICATES: usize = 8;

/// The longest path a predicate may address, and how deep it may reach.
const MAX_PATH_CHARS: usize = 256;
const MAX_PATH_SEGMENTS: usize = 8;

/// The longest reason — and string comparand — a predicate may carry. The
/// reason is shown as one line, in a refusal or as a disabled control's
/// tooltip, so it is a sentence rather than a document.
pub const MAX_REASON_CHARS: usize = 200;

/// One declared check about the object an action would write.
///
/// Exactly one operator per predicate:
///
/// ```json
/// {"jsonPath": ".spec.suspend", "notEquals": true,
///  "reason": "Resume this resource before requesting reconciliation"}
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Predicate {
    /// The value this predicate asks about, as a bounded JSONPath subset:
    /// `.spec.suspend`, `.metadata.annotations['acme.io/pinned']`,
    /// `.status.conditions[0].status`. The grammar is the bounded subset the
    /// module documents: no wildcard, filter or recursive descent.
    #[serde(rename = "jsonPath")]
    pub json_path: String,
    /// The value at `jsonPath` must equal this literal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equals: Option<Value>,
    /// The value at `jsonPath` must not equal this literal. An unset field is
    /// not equal to anything, so this holds when it is absent.
    #[serde(default, rename = "notEquals", skip_serializing_if = "Option::is_none")]
    pub not_equals: Option<Value>,
    /// `true`: the value at `jsonPath` must be set and not null.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub present: Option<bool>,
    /// `true`: the value at `jsonPath` must be unset or null.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub absent: Option<bool>,
    /// What the operator is told when this predicate does not hold. Untrusted
    /// display text: written by the app, and escaped by the host before
    /// anything draws it.
    pub reason: String,
}

/// The one operator a predicate carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operator<'a> {
    Equals(&'a Value),
    NotEquals(&'a Value),
    Present,
    Absent,
    /// A timestamp within this many seconds of now: ahead when positive, behind
    /// when negative. Only a card predicate declares it.
    Within(i64),
    /// A timestamp earlier than now plus this many seconds. Only a card
    /// predicate declares it.
    Before(i64),
}

/// Whether `operator` holds for the value found at a predicate's path, read
/// against `now` in seconds since the Unix epoch.
///
/// The one evaluator both kinds of predicate go through, so an action's
/// `equals` and a card's `equals` cannot come to mean different things.
fn evaluate(operator: Operator<'_>, found: Option<&Value>, now: i64) -> bool {
    // A null is how the API server spells a field nobody set, so the two
    // are one answer here rather than a distinction an app must know.
    let found = found.filter(|value| !value.is_null());
    match operator {
        Operator::Equals(want) => found == Some(want),
        Operator::NotEquals(want) => found != Some(want),
        Operator::Present => found.is_some(),
        Operator::Absent => found.is_none(),
        Operator::Within(window) => timestamp(found).is_some_and(|at| {
            let edge = now.saturating_add(window);
            (now.min(edge)..=now.max(edge)).contains(&at)
        }),
        Operator::Before(offset) => {
            timestamp(found).is_some_and(|at| at < now.saturating_add(offset))
        }
    }
}

/// The instant a value names, when it is an RFC 3339 timestamp — the form
/// Kubernetes writes every `metav1.Time` in. Anything else is no instant, and
/// a date operator over it does not hold.
fn timestamp(value: Option<&Value>) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value?.as_str()?)
        .ok()
        .map(|at| at.timestamp())
}

/// The longest duration a card predicate may name: ten years, either way.
const MAX_DURATION_SECONDS: i64 = 3_650 * 86_400;

/// A signed duration of one unit, such as `14d`, `-1h` or `30s`, in seconds.
/// One spelling per quantity, and no calendar units: a month or a year is not
/// a fixed number of seconds.
fn duration(text: &str) -> Result<i64, String> {
    let bad = || {
        Err(format!(
            "`{text}` is not a duration: write a whole number and one unit of s, m, h, d or w, such as `14d` or `-1h`, at most 3650d"
        ))
    };
    let (negative, magnitude) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let Some(unit) = magnitude.chars().last() else {
        return bad();
    };
    let digits = &magnitude[..magnitude.len() - unit.len_utf8()];
    // The limit is `MAX_DURATION_SECONDS`, below. This only stops an absurd
    // count before parsing: the smallest ten-digit count, 10^9 seconds, is
    // already about 11,574 days, so nine digits refuse nothing in range.
    if digits.is_empty() || digits.len() > 9 || !digits.bytes().all(|c| c.is_ascii_digit()) {
        return bad();
    }
    let scale = match unit {
        's' => 1,
        'm' => 60,
        'h' => 3_600,
        'd' => 86_400,
        'w' => 7 * 86_400,
        _ => return bad(),
    };
    let Ok(count) = digits.parse::<i64>() else {
        return bad();
    };
    let Some(seconds) = count
        .checked_mul(scale)
        .filter(|s| *s <= MAX_DURATION_SECONDS)
    else {
        return bad();
    };
    Ok(if negative { -seconds } else { seconds })
}

/// What a dashboard card counts (#540): one question about one value of each
/// object its source lists.
///
/// The action predicates' path grammar, literal rule and `equals` / `absent`,
/// plus two date operators. There is no `reason`: nothing is refused, so
/// there is nothing to tell an operator. Exactly one operator per predicate:
///
/// ```json
/// {"jsonPath": ".status.notAfter", "within": "14d"}
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CardPredicate {
    /// The value this predicate asks about, in the same bounded JSONPath
    /// subset as an action predicate's.
    #[serde(rename = "jsonPath")]
    pub json_path: String,
    /// The value at `jsonPath` must equal this string, number or boolean.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equals: Option<Value>,
    /// `true`: the value at `jsonPath` must be unset or null.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub absent: Option<bool>,
    /// The value at `jsonPath` is an RFC 3339 timestamp within this duration
    /// of now: `14d` is from now until fourteen days ahead, `-1h` the last
    /// hour. Units s, m, h, d and w.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub within: Option<String>,
    /// The value at `jsonPath` is an RFC 3339 timestamp earlier than now plus
    /// this duration: `14d` includes everything already past, `0d` is only the
    /// past, `-30d` is older than thirty days.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
}

impl CardPredicate {
    /// Every rule a declared card predicate must satisfy.
    pub fn check(&self) -> Result<(), String> {
        segments(&self.json_path)?;
        if let Operator::Equals(value) = self.operator()? {
            literal(value)?;
        }
        Ok(())
    }

    /// Whether this predicate holds for `object` at `now`, in seconds since
    /// the Unix epoch.
    ///
    /// A predicate this host would refuse at install does not hold, whatever
    /// it says — so a path typo counts nothing rather than everything, even
    /// under `absent`.
    pub fn holds_at(&self, object: &Value, now: i64) -> bool {
        let (Ok(operator), Ok(path)) = (self.operator(), segments(&self.json_path)) else {
            return false;
        };
        if self.check().is_err() {
            return false;
        }
        evaluate(operator, walk(object, &path), now)
    }

    fn operator(&self) -> Result<Operator<'_>, String> {
        let within = self.within.as_deref().map(duration).transpose()?;
        let before = self.before.as_deref().map(duration).transpose()?;
        let declared: Vec<Operator<'_>> = [
            self.equals.as_ref().map(Operator::Equals),
            self.absent.map(|_| Operator::Absent),
            within.map(Operator::Within),
            before.map(Operator::Before),
        ]
        .into_iter()
        .flatten()
        .collect();
        let [operator] = declared[..] else {
            return Err(format!(
                "`{}` must declare exactly one of `equals`, `absent`, `within` and `before`",
                self.json_path
            ));
        };
        if matches!(operator, Operator::Absent) && self.absent != Some(true) {
            return Err("`absent` is written `true`".into());
        }
        if operator == Operator::Within(0) {
            return Err("`within` needs a window of some length; `0d` matches nothing".into());
        }
        Ok(operator)
    }
}

/// One step of a predicate's path.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Key(String),
    Index(usize),
}

impl Predicate {
    /// Every rule a declared predicate must satisfy.
    ///
    /// Run where a manifest is accepted and again on the way to the cluster,
    /// so there is one statement of each rule and no way to reach a cluster
    /// past it — the precedent `Capability::bound_arguments` sets.
    pub fn check(&self) -> Result<(), String> {
        segments(&self.json_path)?;
        match self.operator()? {
            Operator::Equals(value) | Operator::NotEquals(value) => literal(value)?,
            // An action predicate declares no date operator.
            Operator::Present | Operator::Absent | Operator::Within(_) | Operator::Before(_) => {}
        }
        if self.reason.trim().is_empty() {
            return Err(format!(
                "`{}` must carry a `reason`: it is what the operator is told when the check does not hold",
                self.json_path
            ));
        }
        if self.reason.chars().count() > MAX_REASON_CHARS {
            return Err(format!(
                "A predicate's reason is at most {MAX_REASON_CHARS} characters"
            ));
        }
        Ok(())
    }

    /// Whether this predicate holds for `object`.
    ///
    /// A predicate this host would refuse at install does not hold, whatever
    /// it says: a caller that reaches here without checking refuses the action
    /// rather than waving it through, and a path typo cannot become a
    /// condition that silently passes.
    pub fn holds(&self, object: &Value) -> bool {
        if self.check().is_err() {
            return false;
        }
        let (Ok(operator), Ok(path)) = (self.operator(), segments(&self.json_path)) else {
            return false;
        };
        // No date operator reaches here, so the clock is never read.
        evaluate(operator, walk(object, &path), 0)
    }

    /// The declared reason, safe to show: it came from a manifest, so control
    /// and format characters are written as escapes rather than drawn. The
    /// surface that shows it still frames it as the app's words, never as the
    /// host's.
    pub fn reason(&self) -> String {
        escape_invisible(self.reason.trim())
    }

    fn operator(&self) -> Result<Operator<'_>, String> {
        let declared: Vec<Operator<'_>> = [
            self.equals.as_ref().map(Operator::Equals),
            self.not_equals.as_ref().map(Operator::NotEquals),
            self.present.map(|_| Operator::Present),
            self.absent.map(|_| Operator::Absent),
        ]
        .into_iter()
        .flatten()
        .collect();
        let [operator] = declared[..] else {
            return Err(format!(
                "`{}` must declare exactly one of `equals`, `notEquals`, `present` and `absent`; `null` is not a comparand, so write `absent: true` for a field nobody set",
                self.json_path
            ));
        };
        // `present: false` would be a second spelling of `absent`, and a
        // reader would have to hold both in mind to know what a manifest asks.
        if matches!(operator, Operator::Present) && self.present != Some(true) {
            return Err(
                "`present` is written `true`; to require a field to be unset, write `absent: true`"
                    .into(),
            );
        }
        if matches!(operator, Operator::Absent) && self.absent != Some(true) {
            return Err(
                "`absent` is written `true`; to require a field to be set, write `present: true`"
                    .into(),
            );
        }
        Ok(operator)
    }
}

/// Only a literal is a comparand. A predicate compares one value to one
/// constant; an object or a list on the right-hand side is a shape match — a
/// different question, with an order-dependent answer and no obvious wording
/// for why it failed.
fn literal(value: &Value) -> Result<(), String> {
    match value {
        Value::Object(_) | Value::Array(_) => Err(
            "A predicate compares against a string, number or boolean literal, not an object or a list"
                .into(),
        ),
        Value::String(text) if text.chars().count() > MAX_REASON_CHARS => Err(format!(
            "A predicate's comparand is at most {MAX_REASON_CHARS} characters"
        )),
        _ => Ok(()),
    }
}

/// Every rule one declared list of predicates must satisfy.
pub fn check_predicates(predicates: &[Predicate]) -> Result<(), String> {
    if predicates.len() > MAX_PREDICATES {
        return Err(format!("Declare at most {MAX_PREDICATES} predicates"));
    }
    predicates.iter().try_for_each(Predicate::check)
}

/// The first predicate that does not hold for `object`, or `None` when every
/// one does.
///
/// Per object on purpose: the host calls it once, on the object it just read,
/// and a list view maps it over the rows it already holds to say how much of a
/// selection an action applies to (#553). There is no variant that takes a
/// list, because the answer a list needs is per row.
pub fn unmet<'a>(predicates: &'a [Predicate], object: &Value) -> Option<&'a Predicate> {
    predicates.iter().find(|p| !p.holds(object))
}

/// The value at a predicate's path, if the object holds one.
///
/// A bounded JSONPath subset, not the grammar: an optional leading `$`, then
/// `.key`, `['key']`, `["key"]` and `[0]`. No wildcard, no filter, no
/// recursive descent, no function — each of those addresses a *set* of values,
/// and "does this hold" over a set is a second question with its own quantifier.
///
/// `crates/kube`'s CRD printer-column walker is not reused here although it
/// reads the same-looking paths: it renders display text and yields an empty
/// string for anything it cannot follow, which for a guard would turn a typo
/// into a check that passes.
pub fn resolve<'a>(object: &'a Value, path: &str) -> Option<&'a Value> {
    walk(object, &segments(path).ok()?)
}

/// Whether `path` is one this host evaluates, and why not when it is not. For
/// a declared path that is read without a predicate around it, such as the
/// value a dashboard card sums.
pub fn check_path(path: &str) -> Result<(), String> {
    segments(path).map(|_| ())
}

fn walk<'a>(object: &'a Value, path: &[Segment]) -> Option<&'a Value> {
    let mut node = object;
    for segment in path {
        node = match segment {
            Segment::Key(key) => node.as_object()?.get(key)?,
            Segment::Index(index) => node.as_array()?.get(*index)?,
        };
    }
    Some(node)
}

/// One path's segments, or why it is not a path this host evaluates.
fn segments(path: &str) -> Result<Vec<Segment>, String> {
    let not_a_path = |why: &str| Err(format!("`{path}` is not a resource path: {why}"));
    if path.chars().count() > MAX_PATH_CHARS {
        return not_a_path(&format!("it is longer than {MAX_PATH_CHARS} characters"));
    }
    // `$` is how JSONPath names the document root; both spellings address it.
    let mut rest = path.strip_prefix('$').unwrap_or(path);
    if !rest.starts_with(['.', '[']) {
        return not_a_path("it must start with `.`");
    }
    let mut parsed = Vec::new();
    while !rest.is_empty() {
        rest = if let Some(open) = rest.strip_prefix('[') {
            let Some(close) = open.find(']') else {
                return not_a_path("a `[` has no `]`");
            };
            let inner = &open[..close];
            let Some(segment) = bracket(inner) else {
                return not_a_path(&format!(
                    "`[{inner}]` is neither an index nor a quoted key; wildcards and filters address a set of values, which this host does not evaluate"
                ));
            };
            parsed.push(segment);
            &open[close + 1..]
        } else if let Some(after) = rest.strip_prefix('.') {
            let end = after.find(['.', '[']).unwrap_or(after.len());
            let key = &after[..end];
            if key.is_empty() {
                return not_a_path("it has an empty segment; quote a key that contains a `.`");
            }
            if !key
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c))
            {
                return not_a_path(&format!(
                    "`{key}` is not a plain key; write `['{key}']` for a key with other characters"
                ));
            }
            parsed.push(Segment::Key(key.to_owned()));
            &after[end..]
        } else {
            return not_a_path("a segment starts with `.` or `[`");
        };
    }
    if parsed.len() > MAX_PATH_SEGMENTS {
        return not_a_path(&format!("it is deeper than {MAX_PATH_SEGMENTS} segments"));
    }
    Ok(parsed)
}

/// The inside of one `[...]`: an index, or a quoted key.
fn bracket(inner: &str) -> Option<Segment> {
    if let Ok(index) = inner.parse::<usize>() {
        // `[01]` also parses as 1; one spelling per index, so two manifests
        // that address the same element read the same.
        return (inner == index.to_string()).then_some(Segment::Index(index));
    }
    for quote in ['\'', '"'] {
        if let Some(key) = inner
            .strip_prefix(quote)
            .and_then(|rest| rest.strip_suffix(quote))
        {
            // A quote inside a quoted key would need an escape, and an escape
            // is a second grammar; such a key is addressed with the other
            // quote or not at all.
            return (!key.is_empty() && !key.contains(quote)).then(|| Segment::Key(key.to_owned()));
        }
    }
    None
}
