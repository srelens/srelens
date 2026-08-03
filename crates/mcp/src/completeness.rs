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

/// A destructive capability that is not confirm-gated would be executable by
/// any client with no consent step. Registering one must fail the build, not
/// wait for a review to catch it.
pub fn assert_destructive_capabilities_are_gated(reg: &srelens_capability::Registry) {
    let ungated: Vec<String> = reg
        .ids()
        .into_iter()
        .filter_map(|id| reg.get(id))
        .filter(|c| c.annotations.destructive && !c.annotations.requires_confirm)
        .map(|c| c.id.clone())
        .collect();
    assert!(
        ungated.is_empty(),
        "destructive capabilities are not confirm-gated: {ungated:?}"
    );
}

#[cfg(test)]
mod gate_tests {
    use super::*;
    use serde_json::json;
    use srelens_capability::{Annotations, Capability, Registry};

    #[test]
    fn flags_a_destructive_capability_that_is_not_gated() {
        let mut reg = Registry::new();
        let mut cap = Capability::read_only("oops", "destroys without asking", |_| async {
            Ok(json!({}))
        });
        cap.annotations = Annotations {
            read_only: false,
            destructive: true,
            requires_confirm: false,
            sensitive: false,
        };
        reg.register(cap);
        let reg = std::panic::AssertUnwindSafe(reg);
        let caught = std::panic::catch_unwind(|| assert_destructive_capabilities_are_gated(&reg));
        assert!(
            caught.is_err(),
            "an ungated destructive capability must fail the assertion"
        );
    }

    #[test]
    fn passes_when_destructive_capability_requires_confirm() {
        let mut reg = Registry::new();
        let mut cap = Capability::read_only("safe-destroy", "destroys, but asks first", |_| {
            async { Ok(json!({})) }
        });
        cap.annotations = Annotations::DESTRUCTIVE;
        reg.register(cap);
        assert_destructive_capabilities_are_gated(&reg);
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
