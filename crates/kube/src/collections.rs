//! Collections (#881, #885): saved groupings that act as filters.
//!
//! A *namespace* collection says where — `prod-*`, `team=platform`, or a
//! hand-picked list — and stands in for a namespace selection in every
//! resource view. A *resource* collection says what, within one or more kinds:
//! the services going live in next Tuesday's release. Both share one model and
//! one rule:
//!
//! ```text
//! matches = members ∪ (items matching the patterns AND the label selector)
//! ```
//!
//! Explicit members are always in and are not filtered by the selector. With
//! only patterns, an item matches any of them; with only a selector, every item
//! it selects; with both, an item must match a pattern and the selector. With
//! neither, the collection is its members and nothing else.
//!
//! The matching lives here rather than in the frontend so that every surface
//! that resolves a collection — the desktop and web UI today, MCP and the
//! terminal client later — answers the same question the same way. It is pure:
//! the caller hands over the items it already has (the namespace listing a
//! view loaded, labels included), and nothing here reads the cluster.
//!
//! The collections themselves are kept by the frontend in its settings
//! document (`srelens.collections`, `{ version: 1, collections: [...] }`);
//! they arrive here as input. That keeps one store per host — the desktop
//! settings file, a web user's settings rows — rather than a second one this
//! module would have to keep in step with it.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use kube::core::{Expression, Selector, SelectorExt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use srelens_capability::{Annotations, Capability, CapabilityError};

/// The version of the stored collections document.
pub const DOCUMENT_VERSION: u32 = 1;

/// Longest collection name accepted.
const MAX_NAME_CHARS: usize = 128;
/// Longest description accepted.
const MAX_DESCRIPTION_CHARS: usize = 1024;
/// Most members, patterns or kinds one collection may hold.
const MAX_ENTRIES: usize = 2000;
/// Longest label selector accepted, as `listResource` bounds its own.
const MAX_SELECTOR_CHARS: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum CollectionType {
    /// Groups namespaces: where to look.
    Namespace,
    /// Groups resources of one or more kinds: what to show.
    Resource,
}

/// One saved collection. Unknown fields are ignored, so a document written by
/// a later build still loads.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Collection {
    /// Unique among collections of its type.
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(rename = "type")]
    pub collection_type: CollectionType,
    /// Resource collections only: the kinds it applies to, as `group/Kind`,
    /// or the bare `Kind` for the core group. No version.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<String>,
    /// Explicit names. For a resource collection a plain name matches in
    /// every namespace in scope and `namespace/name` pins one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<String>,
    /// Name globs: `*` matches any run of characters, case-sensitively.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub patterns: Vec<String>,
    /// A Kubernetes label selector, as `kubectl get -l` takes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_selector: Option<String>,
}

/// Something wrong with a collection, against the field it is about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Problem {
    /// `name`, `description`, `kinds`, `members`, `patterns` or `labelSelector`.
    pub field: String,
    /// The entry in a list field the problem is about, when it is one entry's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<usize>,
    pub message: String,
}

impl Problem {
    fn on(field: &str, message: impl Into<String>) -> Self {
        Self { field: field.into(), index: None, message: message.into() }
    }

    fn at(field: &str, index: usize, message: impl Into<String>) -> Self {
        Self { field: field.into(), index: Some(index), message: message.into() }
    }
}

/// Everything wrong with `collection`, or nothing. `taken` holds the names of
/// the other collections of the same type, which this one may not reuse.
pub fn validate(collection: &Collection, taken: &[String]) -> Vec<Problem> {
    let mut problems = Vec::new();

    let name = collection.name.trim();
    if name.is_empty() {
        problems.push(Problem::on("name", "Give the collection a name."));
    } else if name.chars().count() > MAX_NAME_CHARS {
        problems.push(Problem::on("name", format!("A name can be at most {MAX_NAME_CHARS} characters.")));
    } else if taken.iter().any(|other| other.trim() == name) {
        problems.push(Problem::on("name", format!("There is already a collection called \"{name}\".")));
    }
    if collection.description.chars().count() > MAX_DESCRIPTION_CHARS {
        problems.push(Problem::on(
            "description",
            format!("A description can be at most {MAX_DESCRIPTION_CHARS} characters."),
        ));
    }

    for (field, list) in [
        ("kinds", &collection.kinds),
        ("members", &collection.members),
        ("patterns", &collection.patterns),
    ] {
        if list.len() > MAX_ENTRIES {
            problems.push(Problem::on(field, format!("At most {MAX_ENTRIES} entries.")));
        }
    }

    match collection.collection_type {
        CollectionType::Namespace => {
            if !collection.kinds.is_empty() {
                problems.push(Problem::on("kinds", "A namespace collection has no kinds."));
            }
            for (i, member) in collection.members.iter().enumerate() {
                if let Err(why) = check_name(member, NameRule::Label) {
                    problems.push(Problem::at("members", i, format!("\"{member}\" is not a namespace name: {why}")));
                }
            }
        }
        CollectionType::Resource => {
            if collection.kinds.is_empty() {
                problems.push(Problem::on("kinds", "Choose at least one kind."));
            }
            for (i, kind) in collection.kinds.iter().enumerate() {
                if let Err(why) = check_kind(kind) {
                    problems.push(Problem::at("kinds", i, why));
                }
            }
            for (i, member) in collection.members.iter().enumerate() {
                if let Err(why) = check_resource_member(member) {
                    problems.push(Problem::at("members", i, format!("\"{member}\" {why}")));
                }
            }
        }
    }

    for (i, pattern) in collection.patterns.iter().enumerate() {
        if let Err(why) = check_pattern(pattern) {
            problems.push(Problem::at("patterns", i, why));
        }
    }

    if let Some(selector) = collection.label_selector.as_deref() {
        if selector.chars().count() > MAX_SELECTOR_CHARS {
            problems.push(Problem::on(
                "labelSelector",
                format!("A label selector can be at most {MAX_SELECTOR_CHARS} characters."),
            ));
        } else if let Err(why) = parse_selector(selector) {
            problems.push(Problem::on("labelSelector", why));
        }
    }

    problems
}

// -- names, kinds and patterns -------------------------------------------------

#[derive(Clone, Copy)]
enum NameRule {
    /// RFC 1123 label: a namespace.
    Label,
    /// RFC 1123 subdomain: most object names.
    Subdomain,
}

fn check_name(name: &str, rule: NameRule) -> Result<(), String> {
    let max = match rule {
        NameRule::Label => 63,
        NameRule::Subdomain => 253,
    };
    if name.is_empty() {
        return Err("it is empty".into());
    }
    if name.len() > max {
        return Err(format!("it is longer than {max} characters"));
    }
    let allowed = |c: char| {
        c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || (matches!(rule, NameRule::Subdomain) && c == '.')
    };
    if let Some(bad) = name.chars().find(|c| !allowed(*c)) {
        return Err(if bad.is_ascii_uppercase() {
            "names are lowercase".into()
        } else {
            format!("it may not contain {bad:?}")
        });
    }
    let edge = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric());
    if !edge(name.chars().next()) || !edge(name.chars().last()) {
        return Err("it must start and end with a letter or digit".into());
    }
    Ok(())
}

/// `name`, or `namespace/name` pinning it to one namespace.
fn check_resource_member(member: &str) -> Result<(), String> {
    match member.split_once('/') {
        Some((namespace, name)) => {
            check_name(namespace, NameRule::Label).map_err(|why| format!("has a namespace that is not valid: {why}"))?;
            check_name(name, NameRule::Subdomain).map_err(|why| format!("is not a resource name: {why}"))
        }
        None => check_name(member, NameRule::Subdomain).map_err(|why| format!("is not a resource name: {why}")),
    }
}

/// `group/Kind`, or a bare `Kind` for the core group. No version.
fn check_kind(kind: &str) -> Result<(), String> {
    let (group, name) = kind.rsplit_once('/').unwrap_or(("", kind));
    if !group.is_empty() && check_name(group, NameRule::Subdomain).is_err() {
        return Err(format!("\"{kind}\" does not name a valid API group"));
    }
    if group.contains('/') || kind.matches('/').count() > 1 {
        return Err(format!("\"{kind}\" has a version in it; write the group and kind only, e.g. apps/Deployment"));
    }
    let first_upper = name.chars().next().is_some_and(|c| c.is_ascii_uppercase());
    if !first_upper || !name.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(format!("\"{kind}\" is not a kind; kinds are written like Deployment"));
    }
    Ok(())
}

fn check_pattern(pattern: &str) -> Result<(), String> {
    if pattern.is_empty() {
        return Err("A pattern cannot be empty.".into());
    }
    if pattern.starts_with('!') {
        return Err(format!("\"{pattern}\": patterns starting with ! are reserved for exclusions, which are not supported yet."));
    }
    if let Some(bad) = pattern
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '*')))
    {
        return Err(format!("\"{pattern}\" may not contain {bad:?}; only letters, digits, -, ., _ and * are used."));
    }
    Ok(())
}

/// Whether `text` matches `pattern`, where `*` stands for any run of
/// characters, the empty run included. Case-sensitive; nothing else is
/// special.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let (p, t) = (pattern.as_bytes(), text.as_bytes());
    let (mut pi, mut ti) = (0, 0);
    // Where the last `*` was, and where in the text it was last tried from.
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && p[pi] == b'*' {
            star = Some((pi, ti));
            pi += 1;
        } else if pi < p.len() && p[pi] == t[ti] {
            pi += 1;
            ti += 1;
        } else if let Some((sp, st)) = star {
            pi = sp + 1;
            ti = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == b'*')
}

// -- label selectors -------------------------------------------------------------

/// Parse a label selector in the string syntax `kubectl get -l` and the API
/// server take: comma-separated requirements, each `key=value`, `key==value`,
/// `key!=value`, `key in (a,b)`, `key notin (a,b)`, `key` or `!key`.
///
/// `kube` matches a parsed selector but has no parser for this syntax; the
/// match itself is its `SelectorExt`, so it is the same rule the API server's
/// own client library applies. An empty string selects everything.
pub fn parse_selector(text: &str) -> Result<Selector, String> {
    let mut expressions = Vec::new();
    for term in split_terms(text)? {
        let term = term.trim();
        if term.is_empty() {
            if text.trim().is_empty() {
                continue;
            }
            return Err("The selector has an empty requirement; check for a stray comma.".into());
        }
        expressions.push(parse_term(term)?);
    }
    Ok(Selector::from_iter(expressions))
}

/// Split on the commas between requirements, not those inside `( … )`.
fn split_terms(text: &str) -> Result<Vec<&str>, String> {
    let mut terms = Vec::new();
    let (mut depth, mut start) = (0usize, 0usize);
    for (i, c) in text.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| "The selector closes a parenthesis it never opened.".to_string())?
            }
            ',' if depth == 0 => {
                terms.push(&text[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if depth != 0 {
        return Err("The selector opens a parenthesis it never closes.".into());
    }
    terms.push(&text[start..]);
    Ok(terms)
}

fn parse_term(term: &str) -> Result<Expression, String> {
    if let Some(key) = term.strip_prefix('!') {
        let key = key.trim();
        check_label_key(key)?;
        return Ok(Expression::DoesNotExist(key.into()));
    }
    for (word, negated) in [(" notin ", true), (" in ", false)] {
        if let Some((key, rest)) = term.split_once(word) {
            let key = key.trim();
            check_label_key(key)?;
            let rest = rest.trim();
            let inner = rest
                .strip_prefix('(')
                .and_then(|r| r.strip_suffix(')'))
                .ok_or_else(|| format!("\"{term}\": the values after `{}` go in parentheses, e.g. (a,b).", word.trim()))?;
            let values: BTreeSet<String> = inner
                .split(',')
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .collect();
            if values.is_empty() {
                return Err(format!("\"{term}\" lists no values."));
            }
            for value in &values {
                check_label_value(value)?;
            }
            return Ok(if negated { Expression::NotIn(key.into(), values) } else { Expression::In(key.into(), values) });
        }
    }
    for (op, negated) in [("!=", true), ("==", false), ("=", false)] {
        if let Some((key, value)) = term.split_once(op) {
            let (key, value) = (key.trim(), value.trim());
            check_label_key(key)?;
            check_label_value(value)?;
            return Ok(if negated {
                Expression::NotEqual(key.into(), value.into())
            } else {
                Expression::Equal(key.into(), value.into())
            });
        }
    }
    check_label_key(term)?;
    Ok(Expression::Exists(term.into()))
}

/// `[prefix/]name`: a DNS subdomain prefix, and a name of at most 63
/// characters from letters, digits, `-`, `_` and `.`, starting and ending
/// with a letter or digit.
fn check_label_key(key: &str) -> Result<(), String> {
    let (prefix, name) = match key.rsplit_once('/') {
        Some((prefix, name)) => (Some(prefix), name),
        None => (None, key),
    };
    if let Some(prefix) = prefix {
        if prefix.is_empty() || check_name(prefix, NameRule::Subdomain).is_err() {
            return Err(format!("\"{key}\" has a prefix that is not a valid DNS subdomain."));
        }
    }
    if name.is_empty() {
        return Err(if key.is_empty() {
            "A requirement is missing its label key.".into()
        } else {
            format!("\"{key}\" has no name after its prefix.")
        });
    }
    check_label_text(name).map_err(|why| format!("\"{key}\" is not a label key: {why}"))
}

/// Empty, or the same characters a key's name takes.
fn check_label_value(value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Ok(());
    }
    check_label_text(value).map_err(|why| format!("\"{value}\" is not a label value: {why}"))
}

fn check_label_text(text: &str) -> Result<(), String> {
    if text.len() > 63 {
        return Err("it is longer than 63 characters".into());
    }
    if let Some(bad) = text.chars().find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))) {
        return Err(format!("it may not contain {bad:?}"));
    }
    let edge = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric());
    if !edge(text.chars().next()) || !edge(text.chars().last()) {
        return Err("it must start and end with a letter or digit".into());
    }
    Ok(())
}

// -- resolving namespace collections ------------------------------------------

/// One namespace the caller can see, with its labels.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct NamespaceItem {
    pub name: String,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum MemberStatus {
    /// In the listing.
    Found,
    /// Not in a complete listing: a typo, or not on this cluster.
    Missing,
    /// The listing was partial (restricted credentials), so it cannot say.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct MemberCheck {
    pub name: String,
    pub status: MemberStatus,
}

/// What one collection resolved to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct CollectionMatch {
    pub name: String,
    /// The namespaces it covers, in listing order, members the listing could
    /// not see last.
    pub matched: Vec<String>,
    /// Each explicit member, and whether the listing has it.
    pub members: Vec<MemberCheck>,
    /// Set when the collection could not be resolved — it is invalid, or not
    /// a namespace collection — in which case it matched nothing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct NamespaceResolution {
    /// Every namespace any of the collections covers: their union.
    pub namespaces: Vec<String>,
    pub collections: Vec<CollectionMatch>,
    /// True when the listing was partial, so patterns and selectors saw only
    /// the namespaces these credentials can list.
    pub partial: bool,
}

/// Resolve namespace collections against the namespaces the caller can see.
///
/// `complete` is false when the listing is not every namespace on the cluster
/// — restricted credentials, scoped to the namespaces they are known to have.
/// Patterns and selectors then match only what was listed, and explicit
/// members the listing lacks are kept rather than dropped: the view asks for
/// them by name, which such credentials may well be allowed to do.
pub fn resolve_namespaces(collections: &[Collection], listing: &[NamespaceItem], complete: bool) -> NamespaceResolution {
    let listed: HashSet<&str> = listing.iter().map(|n| n.name.as_str()).collect();
    let mut covered: HashSet<String> = HashSet::new();
    let mut unlisted: Vec<String> = Vec::new();

    let results: Vec<CollectionMatch> = collections
        .iter()
        .map(|collection| {
            let refused = |problem: String| CollectionMatch {
                name: collection.name.clone(),
                matched: Vec::new(),
                members: Vec::new(),
                problem: Some(problem),
            };
            if collection.collection_type != CollectionType::Namespace {
                return refused("This is not a namespace collection.".into());
            }
            if let Some(problem) = validate(collection, &[]).into_iter().next() {
                return refused(problem.message);
            }
            let selector = match collection.label_selector.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                Some(text) => match parse_selector(text) {
                    Ok(selector) => Some(selector),
                    Err(why) => return refused(why),
                },
                None => None,
            };
            let has_rule = !collection.patterns.is_empty() || selector.is_some();
            let members: BTreeSet<&str> = collection.members.iter().map(String::as_str).collect();

            let mut matched: Vec<String> = listing
                .iter()
                .filter(|item| {
                    members.contains(item.name.as_str())
                        || (has_rule
                            && (collection.patterns.is_empty()
                                || collection.patterns.iter().any(|p| glob_match(p, &item.name)))
                            && selector.as_ref().is_none_or(|s| s.matches(&item.labels)))
                })
                .map(|item| item.name.clone())
                .collect();

            let mut seen = HashSet::new();
            let checks: Vec<MemberCheck> = collection
                .members
                .iter()
                .filter(|m| seen.insert(m.as_str()))
                .map(|m| {
                    let status = if listed.contains(m.as_str()) {
                        MemberStatus::Found
                    } else if complete {
                        MemberStatus::Missing
                    } else {
                        MemberStatus::Unknown
                    };
                    if status == MemberStatus::Unknown {
                        matched.push(m.clone());
                    }
                    MemberCheck { name: m.clone(), status }
                })
                .collect();

            for name in &matched {
                if covered.insert(name.clone()) && !listed.contains(name.as_str()) {
                    unlisted.push(name.clone());
                }
            }
            CollectionMatch { name: collection.name.clone(), matched, members: checks, problem: None }
        })
        .collect();

    let mut namespaces: Vec<String> = listing
        .iter()
        .filter(|item| covered.contains(&item.name))
        .map(|item| item.name.clone())
        .collect();
    namespaces.extend(unlisted);

    NamespaceResolution { namespaces, collections: results, partial: !complete }
}

// -- resolving resource collections ------------------------------------------

/** One object of a kind, as a metadata-only list returns it. */
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceItem {
    /// Empty for a cluster-scoped kind.
    pub namespace: String,
    pub name: String,
    pub labels: BTreeMap<String, String>,
}

impl ResourceItem {
    /// `namespace/name`, the frontend's row key, with the empty namespace for
    /// a cluster-scoped object.
    pub fn key(&self) -> String {
        format!("{}/{}", self.namespace, self.name)
    }
}

/// What one resource collection came to on one kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ResourceMatch {
    pub name: String,
    /// How many objects it covers here.
    pub matched: usize,
    /// Each explicit member, and whether anything in scope has that name.
    pub members: Vec<MemberCheck>,
    /// Set when the collection could not be applied — invalid, a namespace
    /// collection, or not one of this kind's — in which case it matched nothing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ResourceResolution {
    /// The objects any of the collections covers, as `namespace/name`.
    pub keys: Vec<String>,
    pub collections: Vec<ResourceMatch>,
    /// True when some namespace in scope could not be listed.
    pub partial: bool,
}

/// `group/Kind` as a collection's `kinds` writes it: the bare `Kind` for the
/// core group.
pub fn kind_key(group: &str, kind: &str) -> String {
    if group.is_empty() {
        kind.to_string()
    } else {
        format!("{group}/{kind}")
    }
}

/// Resolve resource collections against the objects of one kind in scope.
///
/// The rule is the namespace collections' rule: members ∪ (patterns AND
/// selector). A member written `name` matches that name in every namespace in
/// scope — one release collection works in dev, staging and prod — and
/// `namespace/name` pins it to one. Patterns match the name only; the
/// namespace is the view's scope. A collection that does not list `kind` is
/// not applied to it.
///
/// `builtin` is the kind's bare name when it is a built-in kind, which a
/// collection may also name without its group — `Deployment` for
/// `apps/Deployment`: a list view knows a built-in kind by name, and the name
/// is unambiguous there. A custom kind is only ever named with its group.
pub fn resolve_resources(
    collections: &[Collection],
    kind: &str,
    builtin: Option<&str>,
    items: &[ResourceItem],
    complete: bool,
) -> ResourceResolution {
    let mut covered: HashSet<String> = HashSet::new();
    let results = collections
        .iter()
        .map(|collection| {
            let refused = |problem: String| ResourceMatch {
                name: collection.name.clone(),
                matched: 0,
                members: Vec::new(),
                problem: Some(problem),
            };
            if collection.collection_type != CollectionType::Resource {
                return refused("This is not a resource collection.".into());
            }
            if !collection.kinds.iter().any(|k| k == kind || builtin == Some(k.as_str())) {
                return refused(format!("This collection does not cover {kind}."));
            }
            if let Some(problem) = validate(collection, &[]).into_iter().next() {
                return refused(problem.message);
            }
            let selector = match collection.label_selector.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                Some(text) => match parse_selector(text) {
                    Ok(selector) => Some(selector),
                    Err(why) => return refused(why),
                },
                None => None,
            };
            let has_rule = !collection.patterns.is_empty() || selector.is_some();
            let is_member = |item: &ResourceItem| {
                collection.members.iter().any(|m| match m.split_once('/') {
                    Some((namespace, name)) => item.namespace == namespace && item.name == name,
                    None => item.name == *m,
                })
            };
            let mut matched = 0;
            for item in items {
                let hit = is_member(item)
                    || (has_rule
                        && (collection.patterns.is_empty() || collection.patterns.iter().any(|p| glob_match(p, &item.name)))
                        && selector.as_ref().is_none_or(|s| s.matches(&item.labels)));
                if hit {
                    matched += 1;
                    covered.insert(item.key());
                }
            }
            let mut seen = HashSet::new();
            let members = collection
                .members
                .iter()
                .filter(|m| seen.insert(m.as_str()))
                .map(|m| {
                    let found = items.iter().any(|item| match m.split_once('/') {
                        Some((namespace, name)) => item.namespace == namespace && item.name == name,
                        None => item.name == *m,
                    });
                    let status = if found {
                        MemberStatus::Found
                    } else if complete {
                        MemberStatus::Missing
                    } else {
                        MemberStatus::Unknown
                    };
                    MemberCheck { name: m.clone(), status }
                })
                .collect();
            ResourceMatch { name: collection.name.clone(), matched, members, problem: None }
        })
        .collect();
    // In the listing's order, which is the API server's.
    let keys = items.iter().map(ResourceItem::key).filter(|k| covered.contains(k)).collect::<Vec<_>>();
    let mut once = HashSet::new();
    let keys = keys.into_iter().filter(|k| once.insert(k.clone())).collect();
    ResourceResolution { keys, collections: results, partial: !complete }
}

/// The objects of `ar` in scope, names and labels only: one metadata-only
/// list per namespace in `namespaces`, or one across the cluster when there
/// are none — for a cluster-scoped kind, always. A namespace whose list is
/// refused costs that namespace, and the answer says it is partial; every
/// namespace refused is an error.
pub async fn list_items(
    client: kube::Client,
    ar: &kube::core::ApiResource,
    namespaced: bool,
    namespaces: &[String],
) -> Result<(Vec<ResourceItem>, bool), CapabilityError> {
    use kube::api::{Api, DynamicObject, ListParams};
    let scopes: Vec<Option<&str>> = if !namespaced || namespaces.is_empty() {
        vec![None]
    } else {
        namespaces.iter().map(|ns| Some(ns.as_str())).collect()
    };
    let mut items = Vec::new();
    let mut failures = Vec::new();
    for scope in &scopes {
        let api: Api<DynamicObject> = match scope {
            Some(ns) => Api::namespaced_with(client.clone(), ns, ar),
            None => Api::all_with(client.clone(), ar),
        };
        match tokio::time::timeout(crate::connect::request_timeout(), api.list_metadata(&ListParams::default())).await {
            Ok(Ok(list)) => items.extend(list.items.into_iter().map(|o| ResourceItem {
                namespace: o.metadata.namespace.unwrap_or_default(),
                name: o.metadata.name.unwrap_or_default(),
                labels: o.metadata.labels.unwrap_or_default(),
            })),
            Ok(Err(e)) => failures.push(e.to_string()),
            Err(_) => failures.push("listing timed out".into()),
        }
    }
    if failures.len() == scopes.len() {
        return Err(CapabilityError::Handler(failures.join("; ")));
    }
    Ok((items, failures.is_empty()))
}

// -- capabilities ------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ValidateIn {
    pub collection: Collection,
    /// The names of the other collections of the same type, which this one
    /// may not reuse.
    #[serde(default)]
    pub taken: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ValidateOut {
    /// Empty when the collection can be saved.
    pub problems: Vec<Problem>,
}

/// `collections.validate` — what is wrong with a collection before it is saved.
pub fn validate_capability() -> Capability {
    Capability::typed::<ValidateIn, ValidateOut, _, _>(
        "collections.validate",
        "check a namespace or resource collection (name, members, glob patterns, label selector, kinds) \
         before saving it; returns each problem against its field, empty when it can be saved",
        Annotations::READ_ONLY,
        |input: ValidateIn| async move { Ok(ValidateOut { problems: validate(&input.collection, &input.taken) }) },
    )
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ResolveNamespacesIn {
    /// The namespace collections to resolve; the result is their union.
    pub collections: Vec<Collection>,
    /// The namespaces the caller can see, with their labels.
    pub namespaces: Vec<NamespaceItem>,
    /// False when `namespaces` is not every namespace on the cluster, as for
    /// credentials that cannot list them all. Defaults to true.
    #[serde(default = "complete_by_default")]
    pub complete: bool,
}

fn complete_by_default() -> bool {
    true
}

/// Most namespaces a caller may hand over in one call.
const MAX_NAMESPACES: usize = 50_000;
/// Most collections resolved in one call.
const MAX_COLLECTIONS: usize = 200;

/// `collections.resolveNamespaces` — which namespaces a set of namespace
/// collections covers, out of a listing the caller already has.
pub fn resolve_namespaces_capability() -> Capability {
    Capability::typed::<ResolveNamespacesIn, NamespaceResolution, _, _>(
        "collections.resolveNamespaces",
        "resolve namespace collections against a namespace listing (name + labels) the caller already has: \
         members plus the namespaces matching the patterns and label selector, unioned across the \
         collections, with each member marked found, missing or unknown. Reads nothing from the cluster.",
        Annotations::READ_ONLY,
        |input: ResolveNamespacesIn| async move {
            if input.namespaces.len() > MAX_NAMESPACES {
                return Err(CapabilityError::InvalidInput(format!("at most {MAX_NAMESPACES} namespaces")));
            }
            if input.collections.len() > MAX_COLLECTIONS {
                return Err(CapabilityError::InvalidInput(format!("at most {MAX_COLLECTIONS} collections")));
            }
            Ok(resolve_namespaces(&input.collections, &input.namespaces, input.complete))
        },
    )
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ResolveResourcesIn {
    pub context: String,
    /// The kind the view lists, e.g. "Deployment".
    pub kind: String,
    /// A custom resource's group, version and plural, as getObject takes
    /// them; left out for a built-in kind.
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub plural: Option<String>,
    /// The view's namespace scope; empty for every namespace.
    #[serde(default)]
    pub namespaces: Vec<String>,
    /// The resource collections to apply; the result is their union.
    pub collections: Vec<Collection>,
}

/// `collections.resolveResources` — which objects of a kind a set of resource
/// collections covers, in the view's namespace scope.
pub fn resolve_resources_capability(cache: std::sync::Arc<crate::client_cache::ClientCache>) -> Capability {
    Capability::typed::<ResolveResourcesIn, ResourceResolution, _, _>(
        "collections.resolveResources",
        "resolve resource collections for one kind in a namespace scope: lists the kind's objects \
         (names and labels only) and returns the ones the collections cover, as namespace/name, \
         with each member marked found, missing or unknown",
        Annotations::READ_ONLY,
        move |input: ResolveResourcesIn| {
            let cache = cache.clone();
            async move {
                if input.collections.len() > MAX_COLLECTIONS {
                    return Err(CapabilityError::InvalidInput(format!("at most {MAX_COLLECTIONS} collections")));
                }
                let (ar, namespaced) = crate::manifest::resolve_kind(
                    &input.kind,
                    None,
                    input.group.as_deref(),
                    input.version.as_deref(),
                    input.plural.as_deref(),
                )?;
                // A custom resource's scope is not known from its name: one
                // asked for in namespaces is namespaced.
                let namespaced = namespaced || (input.plural.is_some() && !input.namespaces.is_empty());
                let client = cache.get(&input.context).await.map_err(CapabilityError::Handler)?;
                let (items, complete) = list_items(client, &ar, namespaced, &input.namespaces).await?;
                let builtin = input.plural.is_none().then_some(ar.kind.as_str());
                Ok(resolve_resources(&input.collections, &kind_key(&ar.group, &ar.kind), builtin, &items, complete))
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn namespace_collection(name: &str) -> Collection {
        Collection {
            name: name.into(),
            description: String::new(),
            collection_type: CollectionType::Namespace,
            kinds: vec![],
            members: vec![],
            patterns: vec![],
            label_selector: None,
        }
    }

    fn listing(items: &[(&str, &[(&str, &str)])]) -> Vec<NamespaceItem> {
        items
            .iter()
            .map(|(name, labels)| NamespaceItem {
                name: (*name).into(),
                labels: labels.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect(),
            })
            .collect()
    }

    fn strings(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| (*s).into()).collect()
    }

    // -- globs ----------------------------------------------------------------

    #[test]
    fn a_glob_matches_any_run_of_characters_at_each_star() {
        assert!(glob_match("prod-*", "prod-us-east"));
        assert!(glob_match("prod-*", "prod-"));
        assert!(!glob_match("prod-*", "staging-prod-a"));
        assert!(glob_match("*-canary", "api-canary"));
        assert!(!glob_match("*-canary", "api-canary-2"));
        assert!(glob_match("team-*-staging", "team-platform-staging"));
        assert!(glob_match("team-*-staging", "team--staging"));
        assert!(!glob_match("team-*-staging", "team-platform-prod"));
        assert!(glob_match("*", "anything"));
        assert!(glob_match("a*b*c", "aXbYbZc"));
        assert!(glob_match("exact", "exact"));
        assert!(!glob_match("exact", "exactly"));
    }

    #[test]
    fn a_glob_is_case_sensitive_and_treats_nothing_but_star_as_special() {
        assert!(!glob_match("Prod-*", "prod-a"));
        assert!(!glob_match("prod.?", "prod-a"));
        assert!(glob_match("prod.a", "prod.a"));
        assert!(!glob_match("prod.a", "prodXa"));
    }

    // -- selectors ------------------------------------------------------------

    fn selects(selector: &str, labels: &[(&str, &str)]) -> bool {
        let labels: BTreeMap<String, String> = labels.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect();
        parse_selector(selector).unwrap().matches(&labels)
    }

    #[test]
    fn parses_every_requirement_kubectl_takes() {
        assert!(selects("env=prod", &[("env", "prod")]));
        assert!(selects("env==prod", &[("env", "prod")]));
        assert!(!selects("env=prod", &[("env", "dev")]));
        assert!(selects("env!=prod", &[("env", "dev")]));
        assert!(selects("env!=prod", &[]));
        assert!(selects("env in (prod, staging)", &[("env", "staging")]));
        assert!(!selects("env in (prod,staging)", &[]));
        assert!(selects("env notin (prod)", &[("env", "dev")]));
        assert!(selects("env notin (prod)", &[]));
        assert!(selects("team", &[("team", "")]));
        assert!(!selects("team", &[]));
        assert!(selects("!legacy", &[]));
        assert!(!selects("!legacy", &[("legacy", "true")]));
        assert!(selects("team=platform,env in (prod,staging),!legacy", &[("team", "platform"), ("env", "prod")]));
        assert!(selects("app.kubernetes.io/part-of=shop", &[("app.kubernetes.io/part-of", "shop")]));
        assert!(selects("", &[]));
    }

    #[test]
    fn refuses_a_selector_the_api_server_would_refuse() {
        for bad in [
            "env=pr od",
            "=prod",
            "env in prod",
            "env in ()",
            "env in (prod",
            "env=prod)",
            "env=prod,,team=a",
            "-env=prod",
            "my_prefix/env=prod",
            &format!("{}=x", "a".repeat(64)),
        ] {
            assert!(parse_selector(bad).is_err(), "{bad:?} must be refused");
        }
    }

    // -- validation -----------------------------------------------------------

    fn fields(problems: &[Problem]) -> Vec<&str> {
        problems.iter().map(|p| p.field.as_str()).collect()
    }

    #[test]
    fn a_sound_namespace_collection_has_no_problems() {
        let mut c = namespace_collection("prod");
        c.patterns = strings(&["prod-*"]);
        c.members = strings(&["shared-ingress"]);
        c.label_selector = Some("env=prod".into());
        assert!(validate(&c, &strings(&["staging"])).is_empty());
    }

    #[test]
    fn refuses_a_missing_or_taken_name() {
        assert_eq!(fields(&validate(&namespace_collection("  "), &[])), ["name"]);
        let taken = validate(&namespace_collection("prod"), &strings(&["prod"]));
        assert_eq!(fields(&taken), ["name"]);
        assert!(taken[0].message.contains("already"));
    }

    /// `!` is kept for exclusions (#885 leaves them for later), so a pattern
    /// starting with one is refused now rather than meaning something else
    /// once exclusions arrive.
    #[test]
    fn refuses_a_pattern_starting_with_bang_and_one_with_stray_characters() {
        let mut c = namespace_collection("prod");
        c.patterns = strings(&["prod-*", "!prod-legacy", "prod east", ""]);
        let problems = validate(&c, &[]);
        let at: Vec<Option<usize>> = problems.iter().map(|p| p.index).collect();
        assert_eq!(fields(&problems), ["patterns", "patterns", "patterns"]);
        assert_eq!(at, [Some(1), Some(2), Some(3)]);
        assert!(problems[0].message.contains("reserved"));
    }

    #[test]
    fn refuses_a_member_that_cannot_be_a_namespace_and_a_bad_selector() {
        let mut c = namespace_collection("prod");
        c.members = strings(&["payments", "Payments", "pay/ments"]);
        c.label_selector = Some("env in prod".into());
        let problems = validate(&c, &[]);
        assert_eq!(fields(&problems), ["members", "members", "labelSelector"]);
        assert_eq!(problems[0].index, Some(1));
        assert!(problems[0].message.contains("lowercase"));
    }

    #[test]
    fn a_resource_collection_needs_kinds_written_without_a_version() {
        let mut c = namespace_collection("release");
        c.collection_type = CollectionType::Resource;
        assert_eq!(fields(&validate(&c, &[])), ["kinds"]);
        c.kinds = strings(&["apps/Deployment", "Service", "apps/v1/Deployment", "deployment"]);
        c.members = strings(&["payment-service", "prod/order-service", "a/b/c"]);
        let problems = validate(&c, &[]);
        assert_eq!(fields(&problems), ["kinds", "kinds", "members"]);
        assert_eq!(problems[0].index, Some(2));
        assert_eq!(problems[2].index, Some(2));
    }

    #[test]
    fn a_namespace_collection_has_no_kinds() {
        let mut c = namespace_collection("prod");
        c.kinds = strings(&["apps/Deployment"]);
        assert_eq!(fields(&validate(&c, &[])), ["kinds"]);
    }

    // -- resolution -----------------------------------------------------------

    fn names(resolution: &NamespaceResolution) -> Vec<&str> {
        resolution.namespaces.iter().map(String::as_str).collect()
    }

    fn cluster() -> Vec<NamespaceItem> {
        listing(&[
            ("prod-eu", &[("env", "prod"), ("team", "shop")]),
            ("prod-us", &[("env", "prod"), ("team", "data")]),
            ("staging-eu", &[("env", "staging"), ("team", "shop")]),
            ("kube-system", &[]),
            ("shared-ingress", &[("env", "shared")]),
        ])
    }

    #[test]
    fn patterns_alone_match_any_pattern() {
        let mut c = namespace_collection("envs");
        c.patterns = strings(&["prod-*", "staging-*"]);
        assert_eq!(names(&resolve_namespaces(&[c], &cluster(), true)), ["prod-eu", "prod-us", "staging-eu"]);
    }

    #[test]
    fn a_selector_alone_matches_everything_it_selects() {
        let mut c = namespace_collection("shop");
        c.label_selector = Some("team=shop".into());
        assert_eq!(names(&resolve_namespaces(&[c], &cluster(), true)), ["prod-eu", "staging-eu"]);
    }

    #[test]
    fn patterns_and_a_selector_together_both_have_to_match() {
        let mut c = namespace_collection("prod shop");
        c.patterns = strings(&["prod-*"]);
        c.label_selector = Some("team=shop".into());
        assert_eq!(names(&resolve_namespaces(&[c], &cluster(), true)), ["prod-eu"]);
    }

    /// The core rule: members are in whatever the patterns and selector say.
    #[test]
    fn members_are_always_in_and_are_not_filtered_by_the_selector() {
        let mut c = namespace_collection("prod shop");
        c.patterns = strings(&["prod-*"]);
        c.label_selector = Some("team=shop".into());
        c.members = strings(&["shared-ingress"]);
        assert_eq!(names(&resolve_namespaces(&[c], &cluster(), true)), ["prod-eu", "shared-ingress"]);
    }

    #[test]
    fn with_neither_rule_a_collection_is_its_members() {
        let mut c = namespace_collection("picked");
        c.members = strings(&["kube-system", "staging-eu"]);
        assert_eq!(names(&resolve_namespaces(&[c], &cluster(), true)), ["staging-eu", "kube-system"]);
    }

    #[test]
    fn several_collections_are_unioned_once_each_in_listing_order() {
        let mut a = namespace_collection("prod");
        a.patterns = strings(&["prod-*"]);
        let mut b = namespace_collection("shop");
        b.label_selector = Some("team=shop".into());
        let r = resolve_namespaces(&[a, b], &cluster(), true);
        assert_eq!(names(&r), ["prod-eu", "prod-us", "staging-eu"]);
        assert_eq!(r.collections[1].matched, ["prod-eu", "staging-eu"]);
    }

    #[test]
    fn marks_each_member_found_or_missing_and_ignores_a_missing_one() {
        let mut c = namespace_collection("picked");
        c.members = strings(&["prod-eu", "prod-ue", "prod-eu"]);
        let r = resolve_namespaces(&[c], &cluster(), true);
        assert_eq!(names(&r), ["prod-eu"]);
        let checks: Vec<(&str, MemberStatus)> =
            r.collections[0].members.iter().map(|m| (m.name.as_str(), m.status)).collect();
        assert_eq!(checks, [("prod-eu", MemberStatus::Found), ("prod-ue", MemberStatus::Missing)]);
    }

    /// Restricted credentials list only what they are known to see. A
    /// member that listing lacks is asked for by name rather than dropped,
    /// and the result says it is partial.
    #[test]
    fn keeps_members_a_partial_listing_cannot_see_and_says_it_is_partial() {
        let mut c = namespace_collection("picked");
        c.members = strings(&["team-a", "team-b"]);
        c.patterns = strings(&["team-*"]);
        let r = resolve_namespaces(&[c], &listing(&[("team-a", &[]), ("team-c", &[])]), false);
        assert!(r.partial);
        assert_eq!(names(&r), ["team-a", "team-c", "team-b"]);
        assert_eq!(r.collections[0].members[1].status, MemberStatus::Unknown);
    }

    #[test]
    fn matching_nothing_is_an_empty_answer_not_an_error() {
        let mut c = namespace_collection("none");
        c.patterns = strings(&["nothing-*"]);
        let r = resolve_namespaces(&[c], &cluster(), true);
        assert!(r.namespaces.is_empty());
        assert_eq!(r.collections[0].problem, None);
    }

    #[test]
    fn an_invalid_or_resource_collection_matches_nothing_and_says_why() {
        let mut bad = namespace_collection("bad");
        bad.patterns = strings(&["!prod-*"]);
        let mut resource = namespace_collection("release");
        resource.collection_type = CollectionType::Resource;
        resource.kinds = strings(&["apps/Deployment"]);
        let r = resolve_namespaces(&[bad, resource], &cluster(), true);
        assert!(r.namespaces.is_empty());
        assert!(r.collections[0].problem.as_deref().unwrap().contains("reserved"));
        assert!(r.collections[1].problem.as_deref().unwrap().contains("not a namespace collection"));
    }

    #[test]
    fn resolves_five_thousand_namespaces_without_trouble() {
        let items: Vec<NamespaceItem> = (0..5000)
            .map(|i| NamespaceItem {
                name: format!("team-{}-{}", i % 50, if i % 2 == 0 { "prod" } else { "staging" }),
                labels: BTreeMap::from([("env".to_string(), if i % 2 == 0 { "prod" } else { "staging" }.to_string())]),
            })
            .collect();
        let mut a = namespace_collection("prod");
        a.patterns = strings(&["team-*-prod"]);
        a.label_selector = Some("env in (prod)".into());
        let mut b = namespace_collection("picked");
        b.members = (0..200).map(|i| format!("team-{i}-staging")).collect();
        let started = std::time::Instant::now();
        let r = resolve_namespaces(&[a, b], &items, true);
        let took = started.elapsed();
        assert_eq!(r.collections[0].matched.len(), 2500);
        // Generous, so a loaded CI runner on a debug build does not flake;
        // a release build on a laptop is far under #885's 50 ms.
        assert!(took < std::time::Duration::from_secs(2), "{took:?}");
    }

    // -- resource collections -------------------------------------------------

    fn resource_collection(name: &str, kinds: &[&str]) -> Collection {
        Collection {
            name: name.into(),
            description: String::new(),
            collection_type: CollectionType::Resource,
            kinds: strings(kinds),
            members: vec![],
            patterns: vec![],
            label_selector: None,
        }
    }

    fn deployments() -> Vec<ResourceItem> {
        let item = |namespace: &str, name: &str, labels: &[(&str, &str)]| ResourceItem {
            namespace: namespace.into(),
            name: name.into(),
            labels: labels.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect(),
        };
        vec![
            item("dev", "payment-service", &[("team", "shop")]),
            item("dev", "order-service", &[("team", "shop")]),
            item("prod", "payment-service", &[("team", "shop")]),
            item("prod", "radar-app", &[("team", "platform")]),
            item("prod", "radar-app-canary", &[("team", "platform"), ("track", "canary")]),
        ]
    }

    fn keys(r: &ResourceResolution) -> Vec<&str> {
        r.keys.iter().map(String::as_str).collect()
    }

    /// #885's release group: names with nothing in common, the same
    /// collection across dev, staging and prod.
    #[test]
    fn a_plain_member_matches_its_name_in_every_namespace_in_scope() {
        let mut c = resource_collection("release", &["apps/Deployment"]);
        c.members = strings(&["payment-service", "shipment-tracker"]);
        let r = resolve_resources(&[c], "apps/Deployment", None, &deployments(), true);
        assert_eq!(keys(&r), ["dev/payment-service", "prod/payment-service"]);
        let checks: Vec<(&str, MemberStatus)> =
            r.collections[0].members.iter().map(|m| (m.name.as_str(), m.status)).collect();
        assert_eq!(checks, [("payment-service", MemberStatus::Found), ("shipment-tracker", MemberStatus::Missing)]);
        assert_eq!(r.collections[0].matched, 2);
    }

    #[test]
    fn a_namespace_slash_name_member_is_pinned_to_its_namespace() {
        let mut c = resource_collection("release", &["apps/Deployment"]);
        c.members = strings(&["prod/payment-service"]);
        assert_eq!(keys(&resolve_resources(&[c], "apps/Deployment", None, &deployments(), true)), ["prod/payment-service"]);
    }

    #[test]
    fn patterns_and_a_selector_match_as_they_do_for_namespaces_and_members_are_not_filtered() {
        let mut c = resource_collection("platform canaries", &["apps/Deployment"]);
        c.patterns = strings(&["*-canary", "order-*"]);
        c.label_selector = Some("team=platform".into());
        c.members = strings(&["dev/order-service"]);
        let r = resolve_resources(&[c], "apps/Deployment", None, &deployments(), true);
        assert_eq!(keys(&r), ["dev/order-service", "prod/radar-app-canary"]);
    }

    #[test]
    fn a_resource_selector_alone_matches_everything_it_selects() {
        let mut c = resource_collection("platform", &["apps/Deployment"]);
        c.label_selector = Some("team=platform".into());
        assert_eq!(
            keys(&resolve_resources(&[c], "apps/Deployment", None, &deployments(), true)),
            ["prod/radar-app", "prod/radar-app-canary"]
        );
    }

    #[test]
    fn several_collections_are_unioned_once_each() {
        let mut a = resource_collection("a", &["apps/Deployment"]);
        a.members = strings(&["payment-service"]);
        let mut b = resource_collection("b", &["apps/Deployment"]);
        b.patterns = strings(&["payment-*"]);
        let r = resolve_resources(&[a, b], "apps/Deployment", None, &deployments(), true);
        assert_eq!(keys(&r), ["dev/payment-service", "prod/payment-service"]);
    }

    #[test]
    fn a_collection_of_other_kinds_or_a_namespace_collection_is_not_applied() {
        let mut services = resource_collection("svc", &["Service"]);
        services.patterns = strings(&["*"]);
        let mut namespaces = namespace_collection("prod");
        namespaces.patterns = strings(&["*"]);
        let r = resolve_resources(&[services, namespaces], "apps/Deployment", Some("Deployment"), &deployments(), true);
        assert!(r.keys.is_empty());
        assert!(r.collections[0].problem.as_deref().unwrap().contains("does not cover apps/Deployment"));
        assert!(r.collections[1].problem.as_deref().unwrap().contains("not a resource collection"));
    }

    #[test]
    fn a_cluster_scoped_kind_is_matched_by_name_alone_and_says_unknown_when_partial() {
        let nodes = vec![ResourceItem { namespace: String::new(), name: "worker-1".into(), labels: BTreeMap::new() }];
        let mut c = resource_collection("workers", &["Node"]);
        c.members = strings(&["worker-1", "worker-2"]);
        let r = resolve_resources(&[c], "Node", Some("Node"), &nodes, false);
        assert_eq!(keys(&r), ["/worker-1"]);
        assert!(r.partial);
        assert_eq!(r.collections[0].members[1].status, MemberStatus::Unknown);
    }

    /// A client that records each request line and answers each namespace's
    /// list from `answers`, or with a 403 for one it does not name.
    fn metadata_client(answers: Vec<(&'static str, serde_json::Value)>) -> (kube::Client, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = seen.clone();
        let answers = std::sync::Arc::new(answers);
        let service = tower::service_fn(move |request: http::Request<kube::client::Body>| {
            let log = log.clone();
            let answers = answers.clone();
            async move {
                let uri = request.uri().to_string();
                log.lock().unwrap().push(format!("{} {} {}", request.method(), uri, request.headers().get("accept").and_then(|v| v.to_str().ok()).unwrap_or("")));
                let found = answers.iter().find(|(path, _)| uri.starts_with(path)).map(|(_, body)| body.clone());
                let (status, body) = match found {
                    Some(body) => (200, body),
                    None => (403, serde_json::json!({"apiVersion": "v1", "kind": "Status", "status": "Failure", "code": 403, "reason": "Forbidden", "message": "forbidden"})),
                };
                Ok::<_, std::convert::Infallible>(
                    http::Response::builder()
                        .status(status)
                        .header("content-type", "application/json")
                        .body(kube::client::Body::from(body.to_string().into_bytes()))
                        .unwrap(),
                )
            }
        });
        (kube::Client::new(service, "default"), seen)
    }

    fn metadata_list(items: &[(&str, &str, &[(&str, &str)])]) -> serde_json::Value {
        serde_json::json!({
            "apiVersion": "meta.k8s.io/v1",
            "kind": "PartialObjectMetadataList",
            "metadata": {},
            "items": items.iter().map(|(ns, name, labels)| serde_json::json!({
                "apiVersion": "meta.k8s.io/v1",
                "kind": "PartialObjectMetadata",
                "metadata": {"name": name, "namespace": ns, "labels": labels.iter().map(|(k, v)| (k.to_string(), serde_json::json!(v))).collect::<serde_json::Map<_, _>>()},
            })).collect::<Vec<_>>(),
        })
    }

    /// Names and labels only, one list per namespace in scope: a credential
    /// scoped to a few namespaces is refused a cluster-wide list (#688), and
    /// the objects themselves are not needed to match a collection.
    #[tokio::test]
    async fn lists_the_kind_metadata_only_one_namespace_at_a_time_and_says_when_one_was_refused() {
        let (client, seen) = metadata_client(vec![(
            "/apis/apps/v1/namespaces/dev/deployments",
            metadata_list(&[("dev", "payment-service", &[("team", "shop")])]),
        )]);
        let ar = kube::core::ApiResource::from_gvk(&kube::core::GroupVersionKind::gvk("apps", "v1", "Deployment"));
        let (items, complete) = list_items(client, &ar, true, &strings(&["dev", "prod"])).await.unwrap();
        assert_eq!(items, [ResourceItem {
            namespace: "dev".into(),
            name: "payment-service".into(),
            labels: BTreeMap::from([("team".to_string(), "shop".to_string())]),
        }]);
        assert!(!complete, "prod was refused");
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert!(seen[0].starts_with("GET /apis/apps/v1/namespaces/dev/deployments"), "{}", seen[0]);
        assert!(seen[0].contains("PartialObjectMetadataList"), "metadata only: {}", seen[0]);
    }

    #[tokio::test]
    async fn lists_across_the_cluster_with_no_scope_and_fails_when_every_list_is_refused() {
        let (client, seen) = metadata_client(vec![("/apis/apps/v1/deployments", metadata_list(&[("dev", "a", &[])]))]);
        let ar = kube::core::ApiResource::from_gvk(&kube::core::GroupVersionKind::gvk("apps", "v1", "Deployment"));
        let (items, complete) = list_items(client, &ar, true, &[]).await.unwrap();
        assert_eq!(items.len(), 1);
        assert!(complete);
        assert!(seen.lock().unwrap()[0].starts_with("GET /apis/apps/v1/deployments"));

        let (client, _) = metadata_client(vec![]);
        assert!(matches!(list_items(client, &ar, true, &strings(&["dev"])).await, Err(CapabilityError::Handler(_))));
    }

    /// A list view knows a built-in kind by name: a collection saved from the
    /// Deployments list says `Deployment`, and covers `apps/Deployment`. A
    /// custom kind of the same name in another group is not covered by it.
    #[test]
    fn a_built_in_kind_may_be_named_without_its_group_and_a_custom_kind_may_not() {
        let mut bare = resource_collection("release", &["Deployment"]);
        bare.members = strings(&["payment-service"]);
        let r = resolve_resources(&[bare.clone()], "apps/Deployment", Some("Deployment"), &deployments(), true);
        assert_eq!(keys(&r), ["dev/payment-service", "prod/payment-service"]);
        let custom = resolve_resources(&[bare], "acme.io/Deployment", None, &deployments(), true);
        assert!(custom.keys.is_empty());
        assert!(custom.collections[0].problem.is_some());
    }

    #[test]
    fn the_kind_key_drops_the_core_group() {
        assert_eq!(kind_key("", "Service"), "Service");
        assert_eq!(kind_key("apps", "Deployment"), "apps/Deployment");
    }

    #[test]
    fn resolves_ten_thousand_resources_without_trouble() {
        let items: Vec<ResourceItem> = (0..10_000)
            .map(|i| ResourceItem {
                namespace: format!("team-{}", i % 40),
                name: format!("svc-{i}"),
                labels: BTreeMap::from([("tier".to_string(), if i % 3 == 0 { "web" } else { "api" }.to_string())]),
            })
            .collect();
        let mut c = resource_collection("web", &["Service"]);
        c.patterns = strings(&["svc-*"]);
        c.label_selector = Some("tier=web".into());
        c.members = (0..300).map(|i| format!("svc-{i}")).collect();
        let started = std::time::Instant::now();
        let r = resolve_resources(&[c], "Service", Some("Service"), &items, true);
        let took = started.elapsed();
        assert!(r.keys.len() > 3000);
        assert!(took < std::time::Duration::from_secs(5), "{took:?}");
    }

    // -- the stored document ----------------------------------------------------

    /// #885 asks that the loader tolerate fields it does not know, so a
    /// document written by a later build still loads here.
    #[test]
    fn reads_a_collection_with_fields_it_does_not_know() {
        let c: Collection = serde_json::from_value(serde_json::json!({
            "name": "release-2026-10-13", "type": "resource",
            "description": "Go-live Tuesday", "kinds": ["apps/Deployment"],
            "members": ["payment-service"], "clusters": ["prod"], "expires": "2026-11-01",
        }))
        .unwrap();
        assert_eq!(c.collection_type, CollectionType::Resource);
        assert_eq!(c.members, ["payment-service"]);
        let ns: Collection =
            serde_json::from_value(serde_json::json!({"name": "prod", "type": "namespace", "patterns": ["prod-*"]}))
                .unwrap();
        assert_eq!(ns.label_selector, None);
        assert!(ns.members.is_empty());
    }

    #[tokio::test]
    async fn the_capabilities_answer_through_the_registry_shapes() {
        let resolve = resolve_namespaces_capability();
        assert!(resolve.annotations.read_only);
        let out = (resolve.handler)(serde_json::json!({
            "collections": [{"name": "prod", "type": "namespace", "patterns": ["prod-*"]}],
            "namespaces": [{"name": "prod-eu", "labels": {}}, {"name": "dev"}],
        }))
        .await
        .unwrap();
        assert_eq!(out["namespaces"], serde_json::json!(["prod-eu"]));
        assert_eq!(out["partial"], serde_json::json!(false));

        let validate = validate_capability();
        let out = (validate.handler)(serde_json::json!({
            "collection": {"name": "prod", "type": "namespace", "patterns": ["!x"]},
            "taken": [],
        }))
        .await
        .unwrap();
        assert_eq!(out["problems"][0]["field"], "patterns");
        assert_eq!(out["problems"][0]["index"], 0);
    }
}
