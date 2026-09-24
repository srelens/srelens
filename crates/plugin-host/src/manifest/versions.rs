//! Custom-resource readers that accept several served API versions (#547).
//!
//! A `k8s.listCustomResource` binding either fixes one version in `arguments.version` or
//! lists `versions` in preference order; never both. The host reads the first listed
//! version the cluster serves and nothing else: a cluster serving none of them is refused
//! exactly as a missing version was before.
//!
//! Where a field moved between versions, `jsonPathOverrides` names, per version, the path
//! to read instead of one the manifest writes. [`Manifest::at_version`] is the one place
//! that applies them: it returns the manifest as it reads a binding at one version, with
//! `arguments.version` bound and every path that reads the binding's objects rewritten,
//! so every consumer downstream — the read, a declared action, a column, a badge, a
//! panel, a status resolver, a card — sees an ordinary single-version manifest.
use super::*;

/// Most versions one binding may list.
pub const MAX_BINDING_VERSIONS: usize = 8;

/// Most paths one version of a binding may override.
pub const MAX_PATH_OVERRIDES: usize = 32;

/// A place a manifest declares a path that reads one binding's objects.
struct Site {
    /// The manifest path of the declaration, for messages.
    at: String,
    /// Why this declaration also reads objects of another binding or kind, when it does.
    /// Rewriting it for one binding would rewrite it for the others, so an override may
    /// not reach it.
    shared: Option<String>,
}

/// Whether `value` can name an API version: letters, digits, `.` and `-`, as the host
/// requires of every bound custom-resource identity.
fn version_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 63
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b".-".contains(&c))
}

impl Binding {
    /// The API versions this binding reads, most preferred first: its `versions`, or the
    /// one it fixes in `arguments.version`. Empty for one that names no version.
    pub fn accepted_versions(&self) -> Vec<String> {
        if !self.versions.is_empty() {
            return self.versions.clone();
        }
        self.arguments
            .get("version")
            .and_then(Value::as_str)
            .map(|version| vec![version.to_owned()])
            .unwrap_or_default()
    }
}

impl Manifest {
    /// [`Binding::accepted_versions`] of the named binding; empty when it is not declared.
    pub fn accepted_versions(&self, binding: &str) -> Vec<String> {
        self.capabilities
            .iter()
            .find(|b| b.name == binding)
            .map(Binding::accepted_versions)
            .unwrap_or_default()
    }

    /// This manifest as it reads `binding` at `version`: the version bound in
    /// `arguments.version`, and that version's `jsonPathOverrides` applied to every path
    /// that reads the binding's objects. Refuses a version the binding does not list, so
    /// nothing can read it at a version its author did not name.
    pub fn at_version(&self, binding: &str, version: &str) -> Result<Manifest, String> {
        let mut resolved = self.clone();
        resolved.apply_version(binding, version, true)?;
        Ok(resolved)
    }

    /// [`Manifest::at_version`] in place, with that version's overrides applied or, to
    /// find what an override itself breaks, left out.
    fn apply_version(&mut self, binding: &str, version: &str, apply: bool) -> Result<(), String> {
        let accepted = self.accepted_versions(binding);
        if !accepted.iter().any(|v| v == version) {
            return Err(if accepted.is_empty() {
                format!("\"{binding}\" is not a declared reader of a versioned API")
            } else {
                format!("\"{binding}\" reads {}, not {version}", accepted.join(", "))
            });
        }
        let Some(reader) = self.capabilities.iter_mut().find(|b| b.name == binding) else {
            return Err(format!("\"{binding}\" is not declared"));
        };
        let overrides = reader
            .json_path_overrides
            .remove(version)
            .filter(|_| apply)
            .unwrap_or_default();
        reader.versions.clear();
        reader.json_path_overrides.clear();
        reader
            .arguments
            .insert("version".to_owned(), Value::String(version.to_owned()));
        if !overrides.is_empty() {
            self.visit_binding_paths(binding, &mut |path, site| {
                if site.shared.is_none() {
                    if let Some(replacement) = overrides.get(path.as_str()) {
                        *path = replacement.clone();
                    }
                }
            });
        }
        Ok(())
    }

    /// Calls `visit` with every path that reads `binding`'s objects, and where it is.
    fn visit_binding_paths(&mut self, binding: &str, visit: &mut dyn FnMut(&mut String, &Site)) {
        let Some(index) = self.capabilities.iter().position(|b| b.name == binding) else {
            return;
        };
        let kind = Self::reader_kind(&self.capabilities[index]);
        // Another reader of the same kind reads everything declared for the kind too.
        let kind_readers = kind.as_ref().map_or(0, |kind| {
            self.capabilities
                .iter()
                .filter(|b| Self::reader_kind(b).as_ref() == Some(kind))
                .count()
        });
        let exclusive = |at: String| Site { at, shared: None };
        // Shared when the declaration lists other kinds, or another reader lists this one.
        let by_kind = |at: String, kinds: &[String]| {
            let others: Vec<&String> = kinds.iter().filter(|k| Some(*k) != kind.as_ref()).collect();
            let shared = if !others.is_empty() {
                Some(format!(
                    "{at} is also read for {}",
                    others
                        .iter()
                        .map(|k| k.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            } else if kind_readers > 1 {
                Some(format!(
                    "{at} is also read through another reader of the same kind"
                ))
            } else {
                None
            };
            Site { at, shared }
        };

        // The reader's own printer columns.
        if let Some(Value::Array(columns)) =
            self.capabilities[index].arguments.get_mut("printerColumns")
        {
            for (position, column) in columns.iter_mut().enumerate() {
                if let Some(Value::String(path)) = column.get_mut("jsonPath") {
                    visit(
                        path,
                        &exclusive(format!(
                            "capabilities[{index}].arguments.printerColumns[{position}]"
                        )),
                    );
                }
            }
        }
        // Declared actions on it: what the host checks before the write, and what
        // decides whether the control is offered.
        for (position, action) in self.actions.iter_mut().enumerate() {
            if action.resource != binding {
                continue;
            }
            for (field, list) in [
                ("preconditions", &mut action.preconditions),
                ("availableWhen", &mut action.available_when),
            ] {
                for (item, predicate) in list.iter_mut().enumerate() {
                    visit(
                        &mut predicate.json_path,
                        &exclusive(format!("actions[{position}].{field}[{item}]")),
                    );
                }
            }
        }
        let contributions = &mut self.contributions;
        // Joins over it, and what reads the resource a join matches.
        let joins: BTreeSet<String> = contributions
            .joins
            .iter()
            .filter(|join| join.capability == binding)
            .map(|join| join.id.clone())
            .collect();
        let joined = |join: &Option<String>| join.as_ref().is_some_and(|id| joins.contains(id));
        for (position, column) in contributions.table_columns.iter_mut().enumerate() {
            if joined(&column.source.join) {
                visit(
                    &mut column.source.json_path,
                    &exclusive(format!("contributions.tableColumns[{position}]")),
                );
            }
        }
        for (position, badge) in contributions.badges.iter_mut().enumerate() {
            if joined(&badge.join) {
                visit_rules(
                    &mut badge.rules,
                    &format!("contributions.badges[{position}]"),
                    &mut |path, at| visit(path, &exclusive(at)),
                );
            }
        }
        // Panels read a joined resource, or, without a join, the resource they are shown
        // for, which is this reader's kind when they list it.
        for (position, panel) in contributions.detail_panels.iter_mut().enumerate() {
            let for_kind = kind
                .as_ref()
                .is_some_and(|kind| panel.for_kinds.contains(kind));
            let for_kinds = panel.for_kinds.clone();
            for (number, section) in panel.sections.iter_mut().enumerate() {
                let at = format!("contributions.detailPanels[{position}].sections[{number}]");
                let mut sources: Vec<(&mut String, &Option<String>)> = match section {
                    DetailSection::Fields { fields } => fields
                        .iter_mut()
                        .map(|field| (&mut field.json_path, &field.join))
                        .collect(),
                    DetailSection::Conditions { json_path, join } => vec![(json_path, &*join)],
                };
                for (path, join) in sources.iter_mut() {
                    if joined(join) {
                        visit(path, &exclusive(at.clone()));
                    } else if join.is_none() && for_kind {
                        visit(
                            path,
                            &by_kind(
                                format!("contributions.detailPanels[{position}]"),
                                &for_kinds,
                            ),
                        );
                    }
                }
            }
        }
        // The status the app resolves for its kind, which countByStatus cards count by too.
        if let Some(kind) = &kind {
            for (position, resolver) in contributions.status_resolvers.iter_mut().enumerate() {
                if !resolver.for_kinds.contains(kind) {
                    continue;
                }
                let at = format!("contributions.statusResolvers[{position}]");
                let site = by_kind(at.clone(), &resolver.for_kinds);
                visit_rules(&mut resolver.rules, &at, &mut |path, _| visit(path, &site));
            }
        }
        // Cards over it.
        for (position, card) in contributions.dashboard_cards.iter_mut().enumerate() {
            if card.source != binding {
                continue;
            }
            let site = exclusive(format!("contributions.dashboardCards[{position}]"));
            if let Some(predicate) = &mut card.predicate {
                visit(&mut predicate.json_path, &site);
            }
            if let Some(metric) = &mut card.metric {
                visit(&mut metric.json_path, &site);
            }
            if let Some(path) = card.list.as_mut().and_then(|list| list.json_path.as_mut()) {
                visit(path, &site);
            }
        }
    }

    /// Whether a binding's overrides are within every limit, so checking them is bounded
    /// work. Past any limit the manifest is already refused, and its overrides are not
    /// looked at: each one costs a walk of the manifest and, in
    /// [`Manifest::override_path_problems`], two resolutions and two rule checks
    /// (CWE-400, PR #692 review).
    fn overrides_in_bounds(&self, binding: &Binding) -> bool {
        self.capabilities.len() <= MAX_CAPABILITIES
            && binding.versions.len() <= MAX_BINDING_VERSIONS
            && binding.json_path_overrides.len() <= MAX_BINDING_VERSIONS
    }

    /// Every path the manifest reads `binding`'s objects through, with each place it is
    /// declared: one walk, however many versions override it.
    fn binding_sites(&self, binding: &str) -> BTreeMap<String, Vec<Site>> {
        let mut sites: BTreeMap<String, Vec<Site>> = BTreeMap::new();
        // The walk takes the manifest mutably, since `at_version` rewrites through it;
        // this one clone per binding reads it.
        self.clone()
            .visit_binding_paths(binding, &mut |read, site| {
                sites.entry(read.clone()).or_default().push(Site {
                    at: site.at.clone(),
                    shared: site.shared.clone(),
                });
            });
        sites
    }

    /// `versions` and `jsonPathOverrides` problems that need no other version's rules.
    pub(super) fn version_problems(&self, problems: &mut ValidationErrors) {
        for (index, binding) in self.capabilities.iter().enumerate() {
            let at = format!("capabilities[{index}]");
            if binding.json_path_overrides.len() > MAX_BINDING_VERSIONS {
                problems.push(
                    Code::InvalidValue,
                    format!("{at}.jsonPathOverrides"),
                    format!("Override at most {MAX_BINDING_VERSIONS} versions"),
                );
            }
            if binding.versions.len() > MAX_BINDING_VERSIONS {
                // Refused whole: neither its entries nor its overrides are checked.
                problems.push(
                    Code::InvalidValue,
                    format!("{at}.versions"),
                    format!("List at most {MAX_BINDING_VERSIONS} versions"),
                );
            }
            if !binding.versions.is_empty() {
                if binding.target != "k8s.listCustomResource" {
                    problems.push(
                        Code::InvalidBinding,
                        format!("{at}.versions"),
                        "Only a k8s.listCustomResource reader chooses among API versions",
                    );
                } else if binding.arguments.contains_key("version") {
                    problems.push(
                        Code::InvalidBinding,
                        format!("{at}.versions"),
                        "Bind one version in arguments.version or list several in versions, not both",
                    );
                }
            }
            if !binding.versions.is_empty() && binding.versions.len() <= MAX_BINDING_VERSIONS {
                for (position, version) in binding.versions.iter().enumerate() {
                    if !version_name(version) {
                        problems.push(
                            Code::InvalidBinding,
                            format!("{at}.versions[{position}]"),
                            "An API version is 1–63 letters, digits, . and -",
                        );
                    }
                }
                unique(
                    problems,
                    binding
                        .versions
                        .iter()
                        .enumerate()
                        .map(|(position, version)| {
                            (format!("{at}.versions[{position}]"), version.as_str())
                        }),
                );
            }
            if !self.overrides_in_bounds(binding) {
                continue;
            }
            let sites = if binding.json_path_overrides.is_empty() {
                BTreeMap::new()
            } else {
                self.binding_sites(&binding.name)
            };
            for (version, overrides) in &binding.json_path_overrides {
                let path = format!("{at}.jsonPathOverrides.{version}");
                if !binding.versions.contains(version) {
                    problems.push(
                        Code::InvalidBinding,
                        path,
                        format!("\"{version}\" is not one of this binding's versions"),
                    );
                    continue;
                }
                if overrides.len() > MAX_PATH_OVERRIDES {
                    problems.push(
                        Code::InvalidValue,
                        path,
                        format!("Override at most {MAX_PATH_OVERRIDES} paths per version"),
                    );
                    continue;
                }
                for from in overrides.keys() {
                    match sites.get(from) {
                        None => problems.push(
                            Code::InvalidBinding,
                            path.clone(),
                            format!(
                                "\"{from}\" is not a path this binding's objects are read through"
                            ),
                        ),
                        Some(found) => {
                            if let Some(shared) = found.iter().find_map(|site| site.shared.as_ref())
                            {
                                problems.push(
                                    Code::InvalidBinding,
                                    path.clone(),
                                    format!(
                                        "\"{from}\" cannot be overridden for one version of this binding: {shared}. Declare it separately for this kind."
                                    ),
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    /// Whether each override is a valid path everywhere it replaces one: the manifest
    /// read at that version must break no rule the manifest without the override keeps.
    /// Reported at the override, since that is what has to change.
    ///
    /// Bounded work (CWE-400, PR #692 review): overrides of different bindings rewrite
    /// disjoint declarations — one another binding or kind also reads is refused — so
    /// round `k` checks every binding's `k`th override together, in one manifest read at
    /// all of them, and a new problem belongs to the binding whose declaration it is
    /// reported under. That is one clone and one rule check per round, at most
    /// [`MAX_BINDING_VERSIONS`] rounds, plus one for the baseline, whatever the number
    /// of bindings.
    pub(super) fn override_path_problems(&self, problems: &mut ValidationErrors) {
        // Per binding: the versions whose overrides are to be checked, and where each
        // declaration it may rewrite sits in the manifest.
        let mut pending: Vec<(usize, Vec<&String>, Vec<String>)> = Vec::new();
        for (index, binding) in self.capabilities.iter().enumerate() {
            if !self.overrides_in_bounds(binding) {
                continue;
            }
            let versions: Vec<&String> = binding
                .json_path_overrides
                .iter()
                // Nothing to check in an empty override, and anything already wrong
                // with one is reported as that.
                .filter(|(version, overrides)| {
                    let at = format!("capabilities[{index}].jsonPathOverrides.{version}");
                    !overrides.is_empty() && !problems.0.iter().any(|problem| problem.path == at)
                })
                .map(|(version, _)| version)
                .collect();
            if versions.is_empty() {
                continue;
            }
            let owned: Vec<String> = self
                .binding_sites(&binding.name)
                .into_values()
                .flatten()
                .filter(|site| site.shared.is_none())
                .map(|site| site.at)
                .collect();
            pending.push((index, versions, owned));
        }
        let rounds = pending.iter().map(|(_, versions, _)| versions.len()).max();
        let Some(rounds) = rounds else {
            return;
        };
        // Every binding read at a version without its overrides: what the rules say
        // before any override is applied.
        let read = |round: Option<usize>| {
            let mut manifest = self.clone();
            for (index, versions, _) in &pending {
                let name = &self.capabilities[*index].name;
                let (version, apply) = match round.and_then(|round| versions.get(round)) {
                    Some(version) => (*version, true),
                    None => (versions[0], false),
                };
                // Each version was checked to be listed; a failure leaves it unresolved.
                let _ = manifest.apply_version(name, version, apply);
            }
            manifest.rule_problems().0
        };
        let before = read(None);
        for round in 0..rounds {
            let owners: Vec<(String, &Vec<String>)> = pending
                .iter()
                .filter_map(|(index, versions, owned)| {
                    versions.get(round).map(|version| {
                        (
                            format!("capabilities[{index}].jsonPathOverrides.{version}"),
                            owned,
                        )
                    })
                })
                .collect();
            for problem in read(Some(round)) {
                if before.contains(&problem) {
                    continue;
                }
                let within = |site: &String| {
                    problem
                        .path
                        .strip_prefix(site.as_str())
                        .is_some_and(|rest| {
                            rest.is_empty() || rest.starts_with('.') || rest.starts_with('[')
                        })
                };
                let found: Vec<&String> = owners
                    .iter()
                    .filter(|(_, owned)| owned.iter().any(within))
                    .map(|(at, _)| at)
                    .collect();
                // A problem under no rewritten declaration came of this round's
                // overrides all the same: every one of them is told, rather than none.
                let blamed = if found.is_empty() {
                    owners.iter().map(|(at, _)| at).collect()
                } else {
                    found
                };
                for at in blamed {
                    problems.push(
                        Code::InvalidValue,
                        at.clone(),
                        format!("With this override, {}: {}", problem.path, problem.message),
                    );
                }
            }
        }
    }
}

/// Calls `visit` with each path a status rule list reads, and where.
fn visit_rules(
    rules: &mut [status::StatusRule],
    at: &str,
    visit: &mut dyn FnMut(&mut String, String),
) {
    for (position, rule) in rules.iter_mut().enumerate() {
        for (item, condition) in rule.when.iter_mut().enumerate() {
            visit(
                &mut condition.json_path,
                format!("{at}.rules[{position}].when[{item}]"),
            );
        }
        if let Some(reason) = &mut rule.reason {
            visit(reason, format!("{at}.rules[{position}].reason"));
        }
    }
}
