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
//! there is no way to name a second object, call a function or write a filter
//! that selects more than one element: a predicate is one question about one
//! value, asked the same way every time. That is what makes it checkable at
//! install rather than only when it runs.
//!
//! Status rules (#541) ask their questions with [`Condition`], which is a
//! predicate without the refusal sentence, evaluated by this same code.

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
    /// module documents: no wildcard or recursive descent, and one filter
    /// form, `[?(@.type=="Ready")]`, that selects the first matching element.
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

/// One question about one value, without a sentence to say when it fails.
///
/// A [`Predicate`] is a condition plus the reason an operator is told when it
/// does not hold. A status rule (#541) asks which of its rules describes an
/// object; nobody is refused, so there is no reason to give — but the
/// operators, the paths and the evaluation are these, and are this code:
///
/// ```json
/// {"jsonPath": ".spec.suspend", "equals": true}
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Condition {
    /// The value this condition asks about, in the grammar [`resolve`] documents.
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
}

impl Condition {
    /// Every rule a declared condition must satisfy: a path this host
    /// evaluates, exactly one operator, and a literal comparand.
    pub fn check(&self) -> Result<(), String> {
        test_of(
            &self.json_path,
            &self.equals,
            &self.not_equals,
            self.present,
            self.absent,
        )
        .map(|_| ())
    }

    /// Whether this condition holds for `object`. One the host would refuse
    /// never does.
    pub fn holds(&self, object: &Value) -> bool {
        test_of(
            &self.json_path,
            &self.equals,
            &self.not_equals,
            self.present,
            self.absent,
        )
        .is_ok_and(|(operator, path)| evaluate(operator, &path, object))
    }
}

/// Whether `path` is one this host evaluates, and if not, why.
pub fn check_path(path: &str) -> Result<(), String> {
    segments(path).map(|_| ())
}

/// A declared test's operator and parsed path, or why it is not one.
fn test_of<'a>(
    path: &str,
    equals: &'a Option<Value>,
    not_equals: &'a Option<Value>,
    present: Option<bool>,
    absent: Option<bool>,
) -> Result<(Operator<'a>, Vec<Segment>), String> {
    let parsed = segments(path)?;
    let operator = operator_of(path, equals, not_equals, present, absent)?;
    match operator {
        Operator::Equals(value) | Operator::NotEquals(value) => literal(value)?,
        Operator::Present | Operator::Absent => {}
    }
    Ok((operator, parsed))
}

fn evaluate(operator: Operator<'_>, path: &[Segment], object: &Value) -> bool {
    // A null is how the API server spells a field nobody set, so the two
    // are one answer here rather than a distinction an app must know.
    let found = walk(object, path).filter(|value| !value.is_null());
    match operator {
        Operator::Equals(want) => found == Some(want),
        Operator::NotEquals(want) => found != Some(want),
        Operator::Present => found.is_some(),
        Operator::Absent => found.is_none(),
    }
}

/// The one operator a predicate carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operator<'a> {
    Equals(&'a Value),
    NotEquals(&'a Value),
    Present,
    Absent,
}

/// One step of a predicate's path.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Key(String),
    Index(usize),
    /// `[?(@.key=="text")]`: the first element of a list that is an object
    /// whose `key` holds exactly the string `text`. The first, and only one,
    /// as a Kubernetes printer column reads the same path: a condition list
    /// is keyed by `type`, so there is one answer to "the Ready condition".
    First { key: String, equals: String },
}

impl Predicate {
    /// Every rule a declared predicate must satisfy.
    ///
    /// Run where a manifest is accepted and again on the way to the cluster,
    /// so there is one statement of each rule and no way to reach a cluster
    /// past it — the precedent `Capability::bound_arguments` sets.
    pub fn check(&self) -> Result<(), String> {
        self.test()?;
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
        self.test()
            .is_ok_and(|(operator, path)| evaluate(operator, &path, object))
    }

    /// The declared reason, safe to show: it came from a manifest, so control
    /// and format characters are written as escapes rather than drawn. The
    /// surface that shows it still frames it as the app's words, never as the
    /// host's.
    pub fn reason(&self) -> String {
        escape_invisible(self.reason.trim())
    }

    fn test(&self) -> Result<(Operator<'_>, Vec<Segment>), String> {
        test_of(
            &self.json_path,
            &self.equals,
            &self.not_equals,
            self.present,
            self.absent,
        )
    }
}

fn operator_of<'a>(
    path: &str,
    equals: &'a Option<Value>,
    not_equals: &'a Option<Value>,
    present: Option<bool>,
    absent: Option<bool>,
) -> Result<Operator<'a>, String> {
    let declared: Vec<Operator<'_>> = [
        equals.as_ref().map(Operator::Equals),
        not_equals.as_ref().map(Operator::NotEquals),
        present.map(|_| Operator::Present),
        absent.map(|_| Operator::Absent),
    ]
    .into_iter()
    .flatten()
    .collect();
    let [operator] = declared[..] else {
        return Err(format!(
            "`{path}` must declare exactly one of `equals`, `notEquals`, `present` and `absent`; `null` is not a comparand, so write `absent: true` for a field nobody set"
        ));
    };
    // `present: false` would be a second spelling of `absent`, and a
    // reader would have to hold both in mind to know what a manifest asks.
    if matches!(operator, Operator::Present) && present != Some(true) {
        return Err(
            "`present` is written `true`; to require a field to be unset, write `absent: true`"
                .into(),
        );
    }
    if matches!(operator, Operator::Absent) && absent != Some(true) {
        return Err(
            "`absent` is written `true`; to require a field to be set, write `present: true`"
                .into(),
        );
    }
    Ok(operator)
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
/// `.key`, `['key']`, `["key"]`, `[0]`, and one filter form,
/// `[?(@.key=="text")]` (either quote), which selects the FIRST element of a
/// list whose plain `key` holds exactly the string `text` (#541). No wildcard,
/// no other filter, no recursive descent, no function — each of those
/// addresses a *set* of values, and "does this hold" over a set is a second
/// question with its own quantifier. The filter is admitted because it does
/// not: it names one element, the one a Kubernetes printer column shows for
/// the same path, and a filter that matches nothing is an unset field, read
/// exactly as `.status.missing` is.
///
/// `crates/kube`'s CRD printer-column walker is not reused here although it
/// reads the same-looking paths: it renders display text and yields an empty
/// string for anything it cannot follow, which for a guard would turn a typo
/// into a check that passes.
pub fn resolve<'a>(object: &'a Value, path: &str) -> Option<&'a Value> {
    walk(object, &segments(path).ok()?)
}

fn walk<'a>(object: &'a Value, path: &[Segment]) -> Option<&'a Value> {
    let mut node = object;
    for segment in path {
        node = match segment {
            Segment::Key(key) => node.as_object()?.get(key)?,
            Segment::Index(index) => node.as_array()?.get(*index)?,
            Segment::First { key, equals } => node
                .as_array()?
                .iter()
                .find(|item| item.get(key).and_then(Value::as_str) == Some(equals.as_str()))?,
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
        rest = if let Some(open) = rest.strip_prefix("[?(") {
            let Some((segment, after)) = filter(open) else {
                return not_a_path(
                    "the one filter this host evaluates is `[?(@.key==\"text\")]`: the first element whose plain key equals a quoted string",
                );
            };
            parsed.push(segment);
            after
        } else if let Some(open) = rest.strip_prefix('[') {
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

/// The rest of a `[?(` filter: `@.key==` then a quoted string and `)]`,
/// returning the segment and what follows it. Nothing else is a filter here.
fn filter(open: &str) -> Option<(Segment, &str)> {
    let body = open.strip_prefix("@.")?;
    let (key, comparand) = body.split_once("==")?;
    if key.is_empty()
        || !key
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c))
    {
        return None;
    }
    let quote = comparand.chars().next().filter(|c| matches!(c, '\'' | '"'))?;
    let text = &comparand[1..];
    let end = text.find(quote)?;
    let (value, after) = (&text[..end], &text[end + 1..]);
    let after = after.strip_prefix(")]")?;
    (!value.is_empty() && value.chars().count() <= MAX_REASON_CHARS).then(|| {
        (
            Segment::First {
                key: key.to_owned(),
                equals: value.to_owned(),
            },
            after,
        )
    })
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
