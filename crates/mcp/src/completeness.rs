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
        cap.annotations = Annotations {
            read_only: false,
            destructive: true,
            requires_confirm: false,
            sensitive: false,
        };
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
