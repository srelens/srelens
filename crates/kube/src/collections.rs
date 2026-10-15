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
    /// Set when the collection also narrows resource lists (#885): in the
    /// lists of these kinds, inside its namespaces, only resources whose name
    /// matches the expression are shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourceFilter>,
}

/// A collection's resource filter: which kinds' lists it narrows, and how.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourceFilter {
    /// The kinds whose lists it narrows, as `group/Kind`, or the bare `Kind`
    /// of the core group — as the cluster's API discovery names them.
    #[serde(default)]
    pub kinds: Vec<String>,
    /// A regular expression, searched for in each resource's name: `^payment-`
    /// for names starting so, `canary` for any name containing it.
    #[serde(default)]
    pub name_regex: String,
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

    if let Some(filter) = &collection.resources {
        if filter.kinds.is_empty() {
            problems.push(Problem::on("resources.kinds", "Choose at least one kind to filter."));
        } else if filter.kinds.len() > MAX_ENTRIES {
            problems.push(Problem::on("resources.kinds", format!("At most {MAX_ENTRIES} kinds.")));
        }
        for (i, kind) in filter.kinds.iter().enumerate() {
            if let Err(why) = check_kind(kind) {
                problems.push(Problem::at("resources.kinds", i, why));
            }
        }
        if let Err(why) = compile_name_regex(&filter.name_regex) {
            problems.push(Problem::on("resources.nameRegex", why));
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

// -- resource filters -----------------------------------------------------------

/// Longest name expression accepted.
const MAX_REGEX_CHARS: usize = 1024;
/// How large a compiled name expression may grow, so one expression cannot
/// cost the process memory it was never meant to (the `regex` crate's own
/// guard; its matching is linear in the name whatever the expression).
const REGEX_SIZE_LIMIT: usize = 1 << 20;

/// A name expression compiled, or why it cannot be.
pub fn compile_name_regex(text: &str) -> Result<regex::Regex, String> {
    if text.trim().is_empty() {
        return Err("Give a regular expression for the names to show, such as ^payment- or canary.".into());
    }
    if text.chars().count() > MAX_REGEX_CHARS {
        return Err(format!("A regular expression can be at most {MAX_REGEX_CHARS} characters."));
    }
    regex::RegexBuilder::new(text)
        .size_limit(REGEX_SIZE_LIMIT)
        .build()
        .map_err(|e| format!("\"{text}\" is not a regular expression this can use: {}", first_line(&e.to_string())))
}

fn first_line(text: &str) -> &str {
    text.lines().rev().find(|l| l.trim_start().starts_with("error:")).map(str::trim).unwrap_or(text)
}

/// Whether a filter's kind entry names `kind`: `group/Kind` with that group,
/// or the bare `Kind`. A list view knows a built-in kind by its name alone
/// (`group` is `None`), and the name is unambiguous among built-in kinds.
fn names_kind(entry: &str, kind: &str, group: Option<&str>) -> bool {
    let (entry_group, entry_kind) = entry.rsplit_once('/').unwrap_or(("", entry));
    entry_kind == kind && group.is_none_or(|g| g == entry_group)
}

/// Which of `names` a list of `kind` shows when scoped to `collections`.
///
/// Several collections are a union. One that does not filter this kind
/// allows every name, so a list is narrowed only when every collection in
/// force filters it; then a name is shown when any of their expressions is
/// found in it. `filtered` says whether the list was narrowed at all.
pub fn match_names(collections: &[Collection], kind: &str, group: Option<&str>, names: &[String]) -> NameMatches {
    let mut expressions = Vec::new();
    let mut problems = Vec::new();
    for collection in collections {
        let filter = collection
            .resources
            .as_ref()
            .filter(|f| f.kinds.iter().any(|k| names_kind(k, kind, group)));
        let Some(filter) = filter else {
            return NameMatches { filtered: false, matches: vec![true; names.len()], problems };
        };
        match compile_name_regex(&filter.name_regex) {
            Ok(re) => expressions.push(re),
            Err(why) => problems.push(format!("{}: {why}", collection.name)),
        }
    }
    if collections.is_empty() {
        return NameMatches { filtered: false, matches: vec![true; names.len()], problems };
    }
    let matches = names.iter().map(|name| expressions.iter().any(|re| re.is_match(name))).collect();
    NameMatches { filtered: true, matches, problems }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct NameMatches {
    /// Whether the list is narrowed at all.
    pub filtered: bool,
    /// For each name, in order, whether it is shown.
    pub matches: Vec<bool>,
    /// A collection whose expression could not be used, which then shows nothing.
    pub problems: Vec<String>,
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
pub struct MatchNamesIn {
    /// The collections the list's namespaces are scoped to.
    pub collections: Vec<Collection>,
    /// The kind the list shows, e.g. "Deployment".
    pub kind: String,
    /// Its group, when the list knows it — a custom kind. A built-in kind is
    /// known by its name alone.
    #[serde(default)]
    pub group: Option<String>,
    /// The names of the rows the list has.
    pub names: Vec<String>,
}

/// Most names one call may carry.
const MAX_NAMES: usize = 100_000;

/// `collections.matchNames` — which of a list's rows its collections' resource
/// filters keep.
pub fn match_names_capability() -> Capability {
    Capability::typed::<MatchNamesIn, NameMatches, _, _>(
        "collections.matchNames",
        "for a list of one kind scoped to collections: which resource names their resource filters keep \
         (each a regular expression for the kinds it names); reads nothing from the cluster",
        Annotations::READ_ONLY,
        |input: MatchNamesIn| async move {
            if input.names.len() > MAX_NAMES {
                return Err(CapabilityError::InvalidInput(format!("at most {MAX_NAMES} names")));
            }
            if input.collections.len() > MAX_COLLECTIONS {
                return Err(CapabilityError::InvalidInput(format!("at most {MAX_COLLECTIONS} collections")));
            }
            Ok(match_names(&input.collections, &input.kind, input.group.as_deref(), &input.names))
        },
    )
}

/// One listable kind the cluster serves, as its API discovery reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ApiKind {
    /// Empty for the core group.
    pub group: String,
    pub version: String,
    pub kind: String,
    pub plural: String,
    pub namespaced: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListApiKindsIn {
    pub context: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListApiKindsOut {
    pub kinds: Vec<ApiKind>,
}

/// The kinds a cluster serves that can be listed, one per group and kind at
/// its preferred version, sorted by kind then group.
pub fn api_kinds(discovery: &kube::Discovery) -> Vec<ApiKind> {
    use kube::discovery::{verbs, Scope};
    let mut kinds: Vec<ApiKind> = discovery
        .groups()
        .flat_map(|group| group.recommended_resources())
        .filter(|(_, caps)| caps.supports_operation(verbs::LIST))
        .map(|(ar, caps)| ApiKind {
            group: ar.group,
            version: ar.version,
            kind: ar.kind,
            plural: ar.plural,
            namespaced: caps.scope == Scope::Namespaced,
        })
        .collect();
    kinds.sort_by(|a, b| a.kind.cmp(&b.kind).then(a.group.cmp(&b.group)));
    kinds.dedup_by(|a, b| a.kind == b.kind && a.group == b.group);
    kinds
}

/// `k8s.listApiKinds` — the kinds a cluster serves that can be listed, for
/// choosing which lists a collection narrows.
pub fn list_api_kinds_capability(cache: std::sync::Arc<crate::client_cache::ClientCache>) -> Capability {
    Capability::typed::<ListApiKindsIn, ListApiKindsOut, _, _>(
        "k8s.listApiKinds",
        "list the kinds a cluster serves that can be listed (group, version, kind, plural, scope), from its API discovery",
        Annotations::READ_ONLY,
        move |input: ListApiKindsIn| {
            let cache = cache.clone();
            async move {
                let client = cache.get(&input.context).await.map_err(CapabilityError::Handler)?;
                let discovery = tokio::time::timeout(crate::connect::request_timeout(), kube::Discovery::new(client).run())
                    .await
                    .map_err(|_| CapabilityError::Handler("API discovery timed out".into()))?
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                Ok(ListApiKindsOut { kinds: api_kinds(&discovery) })
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
            resources: None,
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

    // -- resource filters (#885) -------------------------------------------------

    fn with_filter(name: &str, kinds: &[&str], regex: &str) -> Collection {
        let mut c = namespace_collection(name);
        c.patterns = strings(&["prod-*"]);
        c.resources = Some(ResourceFilter { kinds: strings(kinds), name_regex: regex.into() });
        c
    }

    fn shown(m: &NameMatches, names: &[&str]) -> Vec<String> {
        names.iter().zip(&m.matches).filter(|(_, keep)| **keep).map(|(n, _)| n.to_string()).collect()
    }

    const NAMES: [&str; 4] = ["payment-service", "order-service", "radar-app", "radar-app-canary"];

    #[test]
    fn a_filter_shows_the_names_its_expression_is_found_in() {
        let c = with_filter("prod", &["apps/Deployment"], "^(payment|order)-");
        let m = match_names(&[c], "Deployment", None, &strings(&NAMES));
        assert!(m.filtered);
        assert_eq!(shown(&m, &NAMES), ["payment-service", "order-service"]);
        let canary = with_filter("prod", &["Deployment"], "canary");
        assert_eq!(shown(&match_names(&[canary], "Deployment", None, &strings(&NAMES)), &NAMES), ["radar-app-canary"]);
    }

    #[test]
    fn a_kind_not_named_by_the_filter_is_not_narrowed() {
        let c = with_filter("prod", &["apps/Deployment"], "^payment-");
        let m = match_names(&[c], "Service", None, &strings(&NAMES));
        assert!(!m.filtered);
        assert_eq!(m.matches, [true; 4]);
    }

    /// A built-in kind is known to a list by name; a custom kind by its group
    /// as well, so `acme.io/Deployment` is not `apps/Deployment`.
    #[test]
    fn a_custom_kind_is_matched_by_its_group_too() {
        let c = with_filter("prod", &["apps/Deployment"], "^payment-");
        assert!(!match_names(&[c.clone()], "Deployment", Some("acme.io"), &strings(&NAMES)).filtered);
        let custom = with_filter("prod", &["acme.io/Deployment"], "^payment-");
        assert!(match_names(&[custom], "Deployment", Some("acme.io"), &strings(&NAMES)).filtered);
    }

    #[test]
    fn several_collections_are_a_union_and_one_without_a_filter_for_the_kind_allows_everything() {
        let a = with_filter("a", &["apps/Deployment"], "^payment-");
        let b = with_filter("b", &["apps/Deployment"], "canary$");
        assert_eq!(
            shown(&match_names(&[a.clone(), b], "Deployment", None, &strings(&NAMES)), &NAMES),
            ["payment-service", "radar-app-canary"]
        );
        let plain = namespace_collection("plain");
        let m = match_names(&[a, plain], "Deployment", None, &strings(&NAMES));
        assert!(!m.filtered);
        assert_eq!(m.matches, [true; 4]);
    }

    #[test]
    fn no_collection_narrows_nothing() {
        let m = match_names(&[], "Deployment", None, &strings(&NAMES));
        assert!(!m.filtered);
        assert_eq!(m.matches, [true; 4]);
    }

    #[test]
    fn a_filter_needs_kinds_and_a_regular_expression_this_can_use() {
        let mut c = with_filter("prod", &[], "");
        let problems = validate(&c, &[]);
        let fields: Vec<&str> = problems.iter().map(|p| p.field.as_str()).collect();
        assert_eq!(fields, ["resources.kinds", "resources.nameRegex"]);
        c.resources = Some(ResourceFilter { kinds: strings(&["apps/v1/Deployment"]), name_regex: "(unclosed".into() });
        let problems = validate(&c, &[]);
        assert_eq!(problems[0].field, "resources.kinds");
        assert_eq!(problems[0].index, Some(0));
        assert!(problems[1].message.contains("(unclosed"), "{}", problems[1].message);
        assert!(validate(&with_filter("ok", &["apps/Deployment", "Service"], "^payment-"), &[]).is_empty());
    }

    #[test]
    fn a_filter_whose_expression_cannot_be_used_shows_nothing_and_says_why() {
        let c = with_filter("prod", &["Deployment"], "(");
        let m = match_names(&[c], "Deployment", None, &strings(&NAMES));
        assert!(m.filtered);
        assert_eq!(m.matches, [false; 4]);
        assert!(m.problems[0].starts_with("prod:"));
    }

    #[test]
    fn reads_and_writes_the_filter_in_the_frontend_s_spelling() {
        let c: Collection = serde_json::from_value(serde_json::json!({
            "name": "prod", "type": "namespace", "patterns": ["prod-*"],
            "resources": {"kinds": ["apps/Deployment"], "nameRegex": "^payment-"},
        }))
        .unwrap();
        assert_eq!(c.resources, Some(ResourceFilter { kinds: strings(&["apps/Deployment"]), name_regex: "^payment-".into() }));
        assert_eq!(serde_json::to_value(&c).unwrap()["resources"]["nameRegex"], "^payment-");
        let plain: Collection = serde_json::from_value(serde_json::json!({"name": "a", "type": "namespace"})).unwrap();
        assert!(serde_json::to_value(&plain).unwrap().get("resources").is_none());
    }

    #[tokio::test]
    async fn the_match_capability_answers_in_order() {
        let cap = match_names_capability();
        assert!(cap.annotations.read_only);
        let out = (cap.handler)(serde_json::json!({
            "collections": [{"name": "prod", "type": "namespace", "resources": {"kinds": ["apps/Deployment"], "nameRegex": "^payment-"}}],
            "kind": "Deployment",
            "names": ["order-service", "payment-service"],
        }))
        .await
        .unwrap();
        assert_eq!(out["matches"], serde_json::json!([false, true]));
        assert_eq!(out["filtered"], true);
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
