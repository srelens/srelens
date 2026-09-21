use std::collections::BTreeSet;

use srelens_capability::Registry;

use crate::McpServer;

/// Returns Err(missing_ids) if any registered capability has no MCP tool.
pub fn assert_every_capability_has_a_tool(
    registry: &Registry,
    server: &McpServer,
) -> Result<(), Vec<String>> {
    // Bind the owned Vec so the &str borrows outlive the expression.
    let tools = server.list_tools();
    let tool_names: BTreeSet<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    let missing: Vec<String> = registry
        .ids()
        .into_iter()
        .filter(|id| !tool_names.contains(id))
        .map(str::to_string)
        .collect();
    if missing.is_empty() { Ok(()) } else { Err(missing) }
}

/// A capability that mutates something (i.e. is not `read_only`) but is not
/// confirm-gated would be executable by any client with no consent step —
/// whether or not it also happens to be flagged `destructive`. A capability
/// can be non-destructive and still need consent (e.g. refreshing local
/// cache state, installing a tool); the only annotation that actually gates
/// execution is `requires_confirm` (see `McpServer::requires_confirm`), so
/// that is what this checks. Registering an ungated mutating capability must
/// fail the build, not wait for a review to catch it.
///
/// This subsumes the narrower "destructive but not confirm-gated" check:
/// every destructive capability is also non-read-only, so any capability the
/// old, narrower predicate would have flagged is still flagged here.
pub fn assert_mutating_capabilities_are_gated(reg: &srelens_capability::Registry) {
    let ungated: Vec<String> = reg
        .ids()
        .into_iter()
        .filter_map(|id| reg.get(id))
        .filter(|c| !c.annotations.read_only && !c.annotations.requires_confirm)
        .map(|c| c.id.clone())
        .collect();
    assert!(
        ungated.is_empty(),
        "mutating capabilities are not confirm-gated: {ungated:?}"
    );
}

/// The level and the gate must agree, or the catalog says two things at once.
///
/// Three rules, each one a way the metadata could lie about a call:
///
/// - anything `destructive` is `High` — that is what the word means;
/// - anything confirm-gated is at least `Medium`, because a call worth stopping
///   a human for is not a call that "changes nothing a reader would notice";
/// - an ungated read is `Low`, so `Medium` on a read stays meaningful (it is
///   how `k8s.getSecret` says the secret leaves the host).
///
/// Registering a row that breaks one must fail the build. An impact level is
/// only worth showing if it cannot disagree with the flag beside it.
pub fn assert_impact_matches_the_gate(reg: &srelens_capability::Registry) {
    use srelens_capability::Impact;
    let wrong: Vec<String> = reg
        .ids()
        .into_iter()
        .filter_map(|id| reg.get(id))
        .filter_map(|c| {
            let a = c.annotations;
            let complaint = if a.destructive && a.impact != Impact::High {
                "destructive but not high impact"
            } else if a.requires_confirm && a.impact < Impact::Medium {
                "confirm-gated but low impact"
            } else if a.read_only && !a.requires_confirm && a.impact != Impact::Low {
                "an ungated read above low impact"
            } else {
                return None;
            };
            Some(format!("{} ({complaint})", c.id))
        })
        .collect();
    assert!(wrong.is_empty(), "impact disagrees with the gate: {wrong:?}");
}

/// Every host confirmation template must be well-formed and must render with
/// no fields resolved at all.
///
/// The second half is the one that bites: a template is rendered against a
/// call's arguments, and a capability whose input happens not to carry a
/// namespace would silently fall back to the summary forever. Requiring the
/// bare render means every template reads as a sentence on its own and every
/// field it names is inside an optional segment.
pub fn assert_confirm_templates_are_renderable(reg: &srelens_capability::Registry) {
    let bad: Vec<String> = reg
        .ids()
        .into_iter()
        .filter_map(|id| reg.get(id))
        .filter_map(|c| {
            let template = c.annotations.confirm?;
            if let Err(why) = srelens_capability::check_confirm_template(template) {
                return Some(format!("{}: {why}", c.id));
            }
            if srelens_capability::render_confirm(template, &Default::default()).is_none() {
                return Some(format!(
                    "{}: names a field outside an optional segment, so a call \
                     without that field would show no confirmation at all",
                    c.id
                ));
            }
            None
        })
        .collect();
    assert!(bad.is_empty(), "unusable confirmation templates: {bad:?}");
}

#[cfg(test)]
mod metadata_tests {
    use super::*;
    use serde_json::json;
    use srelens_capability::{Annotations, Capability, Impact, Registry};

    fn with_annotations(id: &str, annotations: Annotations) -> Registry {
        let mut reg = Registry::new();
        let mut cap = Capability::read_only(id, "does a thing", |_| async { Ok(json!({})) });
        cap.annotations = annotations;
        reg.register(cap);
        reg
    }

    fn panics(reg: Registry, f: fn(&Registry)) -> bool {
        let reg = std::panic::AssertUnwindSafe(reg);
        std::panic::catch_unwind(|| f(&reg)).is_err()
    }

    #[test]
    fn every_preset_agrees_with_its_own_gate() {
        for a in [
            Annotations::READ_ONLY,
            Annotations::MUTATING,
            Annotations::SENSITIVE_READ,
            Annotations::DESTRUCTIVE,
        ] {
            assert_impact_matches_the_gate(&with_annotations("t.x", a));
        }
    }

    #[test]
    fn flags_a_destructive_capability_below_high() {
        let reg = with_annotations("t.x", Annotations::DESTRUCTIVE.with_impact(Impact::Medium));
        assert!(panics(reg, assert_impact_matches_the_gate));
    }

    #[test]
    fn flags_a_gated_capability_at_low() {
        let reg = with_annotations("t.x", Annotations::MUTATING.with_impact(Impact::Low));
        assert!(panics(reg, assert_impact_matches_the_gate));
    }

    #[test]
    fn flags_an_ungated_read_above_low() {
        let reg = with_annotations("t.x", Annotations::READ_ONLY.with_impact(Impact::High));
        assert!(panics(reg, assert_impact_matches_the_gate));
    }

    #[test]
    fn accepts_every_preset_template() {
        for a in [Annotations::MUTATING, Annotations::SENSITIVE_READ, Annotations::DESTRUCTIVE] {
            assert_confirm_templates_are_renderable(&with_annotations("t.x", a));
        }
    }

    #[test]
    fn flags_a_template_naming_a_field_outside_the_vocabulary() {
        let reg = with_annotations("t.x", Annotations::MUTATING.with_confirm("Paste {token}?"));
        assert!(panics(reg, assert_confirm_templates_are_renderable));
    }

    /// The trap this check exists for: a template that reads perfectly when
    /// every field resolves and vanishes entirely when one does not.
    #[test]
    fn flags_a_template_that_cannot_render_without_its_fields() {
        let reg = with_annotations("t.x", Annotations::MUTATING.with_confirm("Scale {name}?"));
        assert!(panics(reg, assert_confirm_templates_are_renderable));
    }
}

#[cfg(test)]
mod gate_tests {
    use super::*;
    use serde_json::json;
    use srelens_capability::{Annotations, Capability, Registry};

    #[test]
    fn flags_a_mutating_capability_that_is_not_gated() {
        let mut reg = Registry::new();
        let mut cap = Capability::read_only("oops", "mutates without asking", |_| async {
            Ok(json!({}))
        });
        cap.annotations = Annotations::default(); // read_only: false, requires_confirm: false
        reg.register(cap);
        let reg = std::panic::AssertUnwindSafe(reg);
        let caught = std::panic::catch_unwind(|| assert_mutating_capabilities_are_gated(&reg));
        assert!(
            caught.is_err(),
            "an ungated mutating capability must fail the assertion"
        );
    }

    #[test]
    fn does_not_flag_a_read_only_capability() {
        let mut reg = Registry::new();
        let cap = Capability::read_only("readit", "reads only", |_| async { Ok(json!({})) });
        reg.register(cap);
        assert_mutating_capabilities_are_gated(&reg);
    }

    #[test]
    fn does_not_flag_a_confirm_gated_mutating_capability() {
        let mut reg = Registry::new();
        let mut cap = Capability::read_only("safe-mutate", "mutates, but asks first", |_| {
            async { Ok(json!({})) }
        });
        cap.annotations = Annotations::MUTATING;
        reg.register(cap);
        assert_mutating_capabilities_are_gated(&reg);
    }

    #[test]
    fn still_flags_a_destructive_capability_that_is_not_gated() {
        // The narrower "destructive but ungated" case must still be caught
        // by the widened predicate.
        let mut reg = Registry::new();
        let mut cap = Capability::read_only("oops-destroy", "destroys without asking", |_| {
            async { Ok(json!({})) }
        });
        cap.annotations = Annotations { requires_confirm: false, ..Annotations::DESTRUCTIVE };
        reg.register(cap);
        let reg = std::panic::AssertUnwindSafe(reg);
        let caught = std::panic::catch_unwind(|| assert_mutating_capabilities_are_gated(&reg));
        assert!(
            caught.is_err(),
            "an ungated destructive capability must still fail the assertion"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use srelens_capability::{Capability, Registry};
    use serde_json::json;
    use std::sync::Arc;

    #[test]
    fn complete_registry_passes() {
        let mut reg = Registry::new();
        reg.register(Capability::read_only("a", "", |_| async { Ok(json!(null)) }));
        let server = McpServer::new(Arc::new(reg.clone()));
        assert_eq!(assert_every_capability_has_a_tool(&reg, &server), Ok(()));
    }

    #[test]
    fn detects_capability_with_no_tool() {
        // server built from an empty registry => "a" is missing a tool
        let mut reg = Registry::new();
        reg.register(Capability::read_only("a", "", |_| async { Ok(json!(null)) }));
        let empty = McpServer::new(Arc::new(Registry::new()));
        assert_eq!(
            assert_every_capability_has_a_tool(&reg, &empty),
            Err(vec!["a".to_string()])
        );
    }
}
