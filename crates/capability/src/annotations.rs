//! Host-owned capability metadata: what a call does, how much it can disturb,
//! and the words the host uses when it asks a person to allow it.
//!
//! Every field here is authored in this repository, beside the handler it
//! describes. Nothing outside the host — no manifest, no binding, no MCP
//! client — supplies any of it, and [`Annotations::for_binding`] is the rule
//! that keeps that true rather than merely customary.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

/// How much a successful call can disturb, independent of whether it is gated.
///
/// Gating and impact answer different questions. `requires_confirm` says
/// *whether* a human is asked; `impact` says *how loudly*. An Argo CD status
/// refresh and an Argo CD sync are both mutations behind the same gate, and
/// one re-reads a status while the other applies manifests and runs hooks —
/// no boolean can hold that difference, which is why this exists.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
#[schemars(rename_all = "lowercase")]
pub enum Impact {
    /// Changes nothing a reader would notice: a read, or a write whose only
    /// effect is to make a controller look again.
    #[default]
    Low,
    /// Changes cluster or host state that a person would want to know about,
    /// but that leaves workloads running: a scale, a suspend, a tool install,
    /// a Secret revealed to a caller.
    Medium,
    /// Destroys, disrupts or replaces something running: a delete, a drain, a
    /// sync that applies manifests and runs hooks.
    High,
}

impl Impact {
    pub fn as_str(self) -> &'static str {
        match self {
            Impact::Low => "low",
            Impact::Medium => "medium",
            Impact::High => "high",
        }
    }
}

/// The field names a confirmation template may interpolate.
///
/// A fixed, host-owned vocabulary rather than "whatever is in the arguments":
/// a template that could name any input field would let a future capability
/// paste a token, a manifest or a Secret value into a dialog title.
/// [`check_confirm_template`] rejects anything not in this list, and
/// `every_registered_confirm_template_is_renderable`
/// (`crates/mcp/src/completeness.rs`) runs it over the whole registry.
pub const CONFIRM_FIELDS: &[&str] = &[
    "action",    // the named operation, for capabilities that take one
    "cluster",   // the kubeconfig context the call targets
    "kind",      // the object's kind
    "name",      // the object's name
    "namespace", // the object's namespace
    "resource",  // `kind namespace/name`, collapsed to whatever is known
];

/// A rendered set of [`CONFIRM_FIELDS`] values for one call.
pub type ConfirmFields = BTreeMap<&'static str, String>;

/// Reject a template a host author got wrong, with the reason.
///
/// Checked, not merely rendered: an unknown placeholder renders as an empty
/// segment and a missing brace renders as literal text, so without this a
/// typo in a confirmation prompt reaches a user as a sentence with a hole in
/// it — the one place in the app where a hole in a sentence costs the most.
pub fn check_confirm_template(template: &str) -> Result<(), String> {
    let mut depth = 0usize;
    let mut rest = template;
    while let Some(i) = rest.find(['{', '}', '[', ']']) {
        let marker = rest.as_bytes()[i];
        rest = &rest[i + 1..];
        match marker {
            b'[' => {
                if depth > 0 {
                    return Err("optional segments do not nest".into());
                }
                depth += 1;
            }
            b']' => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| "`]` with no `[`".to_string())?;
            }
            b'{' => {
                let end = rest.find('}').ok_or_else(|| "`{` with no `}`".to_string())?;
                let (field, tail) = rest.split_at(end);
                if !CONFIRM_FIELDS.contains(&field) {
                    return Err(format!("unknown confirmation field `{{{field}}}`"));
                }
                rest = &tail[1..];
            }
            _ => return Err("`}` with no `{`".into()),
        }
    }
    if depth != 0 {
        return Err("`[` with no `]`".into());
    }
    Ok(())
}

/// Render a confirmation template against one call's fields.
///
/// The scheme, in full:
///
/// - `{field}` is replaced by that field's value. `field` must be one of
///   [`CONFIRM_FIELDS`].
/// - `[ … ]` marks an **optional segment**: it is kept only if every `{field}`
///   inside it has a value, and dropped whole otherwise. Segments do not nest.
/// - A `{field}` **outside** a segment that has no value makes the whole
///   render fail (`None`), because the host would otherwise show a sentence
///   with a hole in it. Callers fall back to the capability summary.
///
/// So `"Suspend {resource}[ in {cluster}]?"` renders as
/// `Suspend HelmRelease team/api in cluster/prod?` when the call names a
/// cluster, `Suspend HelmRelease team/api?` when it does not, and nothing at
/// all if it somehow names no resource.
pub fn render_confirm(template: &str, fields: &ConfirmFields) -> Option<String> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    loop {
        let Some(open) = rest.find('[') else {
            out.push_str(&substitute(rest, fields)?);
            return Some(out);
        };
        out.push_str(&substitute(&rest[..open], fields)?);
        let close = rest[open..].find(']')? + open;
        if let Some(segment) = substitute(&rest[open + 1..close], fields) {
            out.push_str(&segment);
        }
        rest = &rest[close + 1..];
    }
}

/// One segment's `{field}` substitution. `None` if any named field is absent.
fn substitute(segment: &str, fields: &ConfirmFields) -> Option<String> {
    let mut out = String::with_capacity(segment.len());
    let mut rest = segment;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let close = rest[open..].find('}')? + open;
        out.push_str(fields.get(&rest[open + 1..close])?);
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    Some(out)
}

/// Read the [`CONFIRM_FIELDS`] a capability's arguments carry.
///
/// Looks at the top level and, one level down, at a nested `resource` object —
/// the shape `k8s.gitOpsAction` and `extensions.action` use, where the object
/// being acted on is a field rather than the whole input. Nothing deeper: this
/// reads arguments a caller controls, so it walks a fixed path rather than
/// searching for a key that looks right.
pub fn confirm_fields(args: &Value) -> ConfirmFields {
    let mut out = ConfirmFields::new();
    let mut read = |scope: &Value| {
        for (field, key) in [
            ("action", "action"),
            ("cluster", "context"),
            ("kind", "kind"),
            ("name", "name"),
            ("namespace", "namespace"),
        ] {
            if out.contains_key(field) {
                continue;
            }
            if let Some(value) = scope.get(key).and_then(Value::as_str) {
                if !value.is_empty() {
                    out.insert(field, value.to_string());
                }
            }
        }
    };
    read(args);
    if let Some(resource) = args.get("resource") {
        read(resource);
    }
    // `{resource}` is derived, never read: a caller cannot hand the host a
    // pre-formatted description of what it is about to change.
    let object = match (out.get("namespace"), out.get("name")) {
        (Some(ns), Some(name)) => Some(format!("{ns}/{name}")),
        (None, Some(name)) => Some(name.clone()),
        _ => None,
    };
    if let Some(object) = object {
        let resource = match out.get("kind") {
            Some(kind) => format!("{kind} {object}"),
            None => object,
        };
        out.insert("resource", resource);
    }
    out
}

/// What a capability does, how much it disturbs, and how the host asks.
///
/// `Copy` and constructible in a `const`, so the presets below are the normal
/// way to write one and a capability that needs a different level or different
/// words says so with [`Annotations::with_impact`] and
/// [`Annotations::with_confirm`] rather than respelling every field.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Annotations {
    pub read_only: bool,
    pub destructive: bool,
    pub requires_confirm: bool,
    /// Reads or exposes sensitive material (e.g. Secret values). Lets an MCP
    /// consent layer gate these separately from ordinary reads.
    pub sensitive: bool,
    /// How much a successful call can disturb. See [`Impact`].
    pub impact: Impact,
    /// The host's own words for the confirmation, as a template in the scheme
    /// [`render_confirm`] documents. `None` means the host has authored none
    /// and a confirming surface falls back to the capability summary.
    ///
    /// `&'static str` on purpose: the text is compiled into the host, so
    /// nothing a manifest, a catalog entry or an MCP client sends can become
    /// the sentence a user is asked to approve.
    pub confirm: Option<&'static str>,
}

impl Annotations {
    pub const READ_ONLY: Self = Self {
        read_only: true,
        destructive: false,
        requires_confirm: false,
        sensitive: false,
        impact: Impact::Low,
        confirm: None,
    };
    pub const DESTRUCTIVE: Self = Self {
        read_only: false,
        destructive: true,
        requires_confirm: true,
        sensitive: false,
        impact: Impact::High,
        confirm: Some("Allow this destructive action[ on {resource}][ in cluster {cluster}]?"),
    };
    /// A change that isn't destructive but still needs user consent (e.g.
    /// installing a tool): confirm-gated, not flagged destructive.
    pub const MUTATING: Self = Self {
        read_only: false,
        destructive: false,
        requires_confirm: true,
        sensitive: false,
        impact: Impact::Medium,
        confirm: Some("Allow this change[ to {resource}][ in cluster {cluster}]?"),
    };
    /// A read that returns sensitive material (e.g. Secret values): confirm-gated
    /// even though it's read-only and non-destructive, so an MCP consent layer
    /// doesn't hand raw secrets to a client without a human seeing it first.
    pub const SENSITIVE_READ: Self = Self {
        read_only: true,
        destructive: false,
        requires_confirm: true,
        sensitive: true,
        impact: Impact::Medium,
        confirm: Some("Reveal secret material[ from {resource}][ in cluster {cluster}] to the caller?"),
    };
    /// The weakest claim any row can make. The identity for [`at_least`], and
    /// what a binding that declares nothing brings to [`for_binding`].
    ///
    /// [`at_least`]: Annotations::at_least
    /// [`for_binding`]: Annotations::for_binding
    pub const WEAKEST: Self = Self {
        read_only: true,
        destructive: false,
        requires_confirm: false,
        sensitive: false,
        impact: Impact::Low,
        confirm: None,
    };

    /// The same row at a different impact level, for a capability whose
    /// preset understates (or overstates) what it can disturb.
    pub const fn with_impact(mut self, impact: Impact) -> Self {
        self.impact = impact;
        self
    }

    /// The same row with the host's own confirmation wording in place of the
    /// preset's. The template is in [`render_confirm`]'s scheme.
    pub const fn with_confirm(mut self, confirm: &'static str) -> Self {
        self.confirm = Some(confirm);
        self
    }

    /// `self`, raised to at least `floor` on every axis — never lowered on any.
    ///
    /// "Raised" is per-field and not always the larger value: `read_only` is a
    /// *permissive* hint (clients auto-approve read-only tools), so the safe
    /// direction there is `false`, and two rows agree on read-only only if both
    /// claim it.
    pub fn at_least(self, floor: Self) -> Self {
        Self {
            read_only: self.read_only && floor.read_only,
            destructive: self.destructive || floor.destructive,
            requires_confirm: self.requires_confirm || floor.requires_confirm,
            sensitive: self.sensitive || floor.sensitive,
            impact: self.impact.max(floor.impact),
            // The floor's wording wins: it is the host row's, and the host
            // owns the sentence a user approves.
            confirm: floor.confirm.or(self.confirm),
        }
    }

    /// The annotations an extension binding runs under.
    ///
    /// `host` is the row of the capability the binding targets; `declared` is
    /// whatever the binding claimed for itself. The result is never weaker
    /// than `host` on any axis, so an extension cannot turn a destructive host
    /// operation into a read-only-looking tool, drop its impact level, or
    /// replace the host's confirmation wording with its own.
    ///
    /// Today a manifest declares nothing and every caller passes
    /// [`Annotations::WEAKEST`] — but the rule lives here rather than in the
    /// absence of a manifest field, because #549's action primitives add
    /// per-action metadata and the hole would reopen silently.
    ///
    /// It also fails closed over the host row itself: a host capability that
    /// mutates, destroys or returns secrets is gated here even if its own
    /// annotation forgot to say so.
    pub fn for_binding(host: Self, declared: Self) -> Self {
        let mut out = declared.at_least(host);
        if !out.read_only || out.sensitive || out.destructive {
            out.requires_confirm = true;
        }
        if out.requires_confirm {
            out.impact = out.impact.max(Impact::Medium);
        }
        if out.destructive {
            out.impact = Impact::High;
        }
        out
    }

    /// This row's confirmation sentence for one call's arguments, or `None`
    /// when the host authored no template or the template names something this
    /// call has no value for. A caller that gets `None` shows the capability
    /// summary; it does not invent the missing half.
    pub fn confirm_text(&self, args: &Value) -> Option<String> {
        render_confirm(self.confirm?, &confirm_fields(args))
    }
}

#[cfg(test)]
mod impact_tests {
    use super::*;

    #[test]
    fn levels_order_low_to_high() {
        assert!(Impact::Low < Impact::Medium);
        assert!(Impact::Medium < Impact::High);
        assert_eq!(Impact::default(), Impact::Low);
    }

    #[test]
    fn max_takes_the_higher_level_either_way_round() {
        assert_eq!(Impact::Low.max(Impact::High), Impact::High);
        assert_eq!(Impact::High.max(Impact::Low), Impact::High);
        assert_eq!(Impact::Medium.max(Impact::Medium), Impact::Medium);
    }

    #[test]
    fn serializes_as_the_lowercase_word_the_catalog_publishes() {
        assert_eq!(serde_json::to_value(Impact::High).unwrap(), serde_json::json!("high"));
        assert_eq!(Impact::Medium.as_str(), "medium");
    }
}

#[cfg(test)]
mod template_tests {
    use super::*;
    use serde_json::json;

    fn fields(pairs: &[(&'static str, &str)]) -> ConfirmFields {
        pairs.iter().map(|(k, v)| (*k, (*v).to_string())).collect()
    }

    #[test]
    fn substitutes_named_fields() {
        let got = render_confirm("Suspend {resource} in {cluster}?", &fields(&[("resource", "HelmRelease team/api"), ("cluster", "prod")]));
        assert_eq!(got.as_deref(), Some("Suspend HelmRelease team/api in prod?"));
    }

    /// The whole point of the optional segment: one template serves a call
    /// that names a cluster and one that does not, and neither reads as a
    /// sentence with a hole in it.
    #[test]
    fn drops_an_optional_segment_whose_field_is_missing() {
        let template = "Suspend {resource}[ in {cluster}]?";
        assert_eq!(
            render_confirm(template, &fields(&[("resource", "r"), ("cluster", "c")])).as_deref(),
            Some("Suspend r in c?")
        );
        assert_eq!(
            render_confirm(template, &fields(&[("resource", "r")])).as_deref(),
            Some("Suspend r?")
        );
    }

    /// A missing field OUTSIDE a segment is not papered over. The caller falls
    /// back to the summary rather than showing "Suspend ?".
    #[test]
    fn a_missing_required_field_refuses_to_render() {
        assert_eq!(render_confirm("Suspend {resource}?", &fields(&[("cluster", "c")])), None);
    }

    #[test]
    fn a_template_with_no_placeholders_renders_as_itself() {
        assert_eq!(
            render_confirm("Allow this change?", &ConfirmFields::new()).as_deref(),
            Some("Allow this change?")
        );
    }

    #[test]
    fn every_preset_template_is_well_formed() {
        for (name, a) in [
            ("DESTRUCTIVE", Annotations::DESTRUCTIVE),
            ("MUTATING", Annotations::MUTATING),
            ("SENSITIVE_READ", Annotations::SENSITIVE_READ),
        ] {
            let template = a.confirm.unwrap_or_else(|| panic!("{name} has no template"));
            check_confirm_template(template).unwrap_or_else(|e| panic!("{name}: {e}"));
            // And each still reads as a sentence with nothing resolved.
            assert!(
                render_confirm(template, &ConfirmFields::new()).is_some(),
                "{name} must render with no fields at all"
            );
        }
        assert_eq!(Annotations::READ_ONLY.confirm, None, "an ordinary read asks nothing");
    }

    #[test]
    fn rejects_a_field_outside_the_vocabulary() {
        let err = check_confirm_template("Paste {token} here").unwrap_err();
        assert!(err.contains("token"), "got: {err}");
    }

    #[test]
    fn rejects_unbalanced_markers() {
        for bad in ["a {resource", "a resource}", "a [ {resource}", "a {resource} ]", "a [x[y]]"] {
            assert!(check_confirm_template(bad).is_err(), "accepted: {bad}");
        }
    }

    #[test]
    fn reads_fields_from_a_flat_argument_object() {
        let got = confirm_fields(&json!({"context":"cluster/prod","namespace":"team","name":"api","kind":"HelmRelease"}));
        assert_eq!(got["cluster"], "cluster/prod");
        assert_eq!(got["resource"], "HelmRelease team/api");
    }

    /// `k8s.gitOpsAction` nests the object under `resource`; the fields still
    /// have to be found, or every GitOps confirmation falls back to the summary.
    #[test]
    fn reads_fields_from_a_nested_resource_object() {
        let got = confirm_fields(&json!({
            "action": "sync",
            "resource": {"context":"cluster/prod","namespace":"team","name":"api","kind":"Application"},
        }));
        assert_eq!(got["action"], "sync");
        assert_eq!(got["cluster"], "cluster/prod");
        assert_eq!(got["resource"], "Application team/api");
    }

    /// `{resource}` is derived from kind/namespace/name, never read. A caller
    /// that sends its own `resource` string cannot choose the words in the
    /// dialog that authorizes it.
    #[test]
    fn a_caller_cannot_supply_the_resource_description() {
        let got = confirm_fields(&json!({"resource": "something harmless", "name": "api"}));
        assert_eq!(got["resource"], "api");
    }

    #[test]
    fn an_empty_string_field_counts_as_absent() {
        let got = confirm_fields(&json!({"context":"","name":"api"}));
        assert!(!got.contains_key("cluster"));
        assert_eq!(got["resource"], "api");
    }

    #[test]
    fn confirm_text_falls_back_to_none_without_a_template() {
        assert_eq!(Annotations::READ_ONLY.confirm_text(&json!({"name":"api"})), None);
        assert_eq!(
            Annotations::MUTATING.confirm_text(&json!({"name":"api","context":"prod"})).as_deref(),
            Some("Allow this change to api in cluster prod?")
        );
    }
}

#[cfg(test)]
mod binding_tests {
    use super::*;

    /// A binding declares nothing today, so the host row must come through
    /// whole — including the level and the wording, which are the two fields
    /// #548 adds and the two an extension would most like to soften.
    #[test]
    fn a_binding_that_declares_nothing_inherits_the_host_row() {
        let host = Annotations::DESTRUCTIVE.with_confirm("Drain {name}?");
        let got = Annotations::for_binding(host, Annotations::WEAKEST);
        assert_eq!(got, host);
    }

    /// The rule the epic names: an extension can never weaken host metadata.
    #[test]
    fn a_binding_cannot_lower_any_axis() {
        let host = Annotations::DESTRUCTIVE.with_confirm("Drain {name}?");
        let soft = Annotations {
            read_only: true,
            destructive: false,
            requires_confirm: false,
            sensitive: false,
            impact: Impact::Low,
            confirm: Some("Just a little refresh, nothing to see"),
        };
        let got = Annotations::for_binding(host, soft);
        assert!(!got.read_only, "a destructive host operation is not read-only");
        assert!(got.destructive);
        assert!(got.requires_confirm);
        assert_eq!(got.impact, Impact::High, "impact cannot be lowered");
        assert_eq!(got.confirm, Some("Drain {name}?"), "the host owns the wording");
    }

    #[test]
    fn a_binding_may_raise_what_it_cannot_lower() {
        let host = Annotations::READ_ONLY;
        let got = Annotations::for_binding(host, Annotations::DESTRUCTIVE);
        assert!(got.destructive && got.requires_confirm && !got.read_only);
        assert_eq!(got.impact, Impact::High);
    }

    /// Fails closed over the host row too: a host capability whose annotation
    /// forgot the gate is gated here, and carries a level to match.
    #[test]
    fn a_host_row_that_forgot_its_gate_is_gated_and_levelled() {
        let sloppy = Annotations { read_only: false, ..Annotations::READ_ONLY };
        let got = Annotations::for_binding(sloppy, Annotations::WEAKEST);
        assert!(got.requires_confirm);
        assert_eq!(got.impact, Impact::Medium);
    }

    #[test]
    fn weakest_is_the_identity_for_at_least() {
        for row in [Annotations::READ_ONLY, Annotations::MUTATING, Annotations::SENSITIVE_READ, Annotations::DESTRUCTIVE] {
            assert_eq!(row.at_least(Annotations::WEAKEST), row);
            assert_eq!(Annotations::WEAKEST.at_least(row), row);
        }
    }

    #[test]
    fn builders_change_only_what_they_name() {
        let a = Annotations::MUTATING.with_impact(Impact::High).with_confirm("Sync {name}?");
        assert_eq!(a.impact, Impact::High);
        assert_eq!(a.confirm, Some("Sync {name}?"));
        assert!(a.requires_confirm && !a.read_only && !a.destructive);
    }
}
