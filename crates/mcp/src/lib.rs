//! Bridges the capability registry to the Model Context Protocol.

pub mod audit;
pub mod auth;
pub mod completeness;
pub mod http;
pub mod policy;
pub mod stdio;

use std::sync::Arc;

use srelens_capability::{CapabilityError, Registry};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct ToolDescriptor {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// Which transport a request arrived on. Recorded in the audit log and used to
/// tell an operator how an agent reached them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Stdio,
    Http,
}

impl Transport {
    pub fn as_str(self) -> &'static str {
        match self {
            Transport::Stdio => "stdio",
            Transport::Http => "http",
        }
    }
}

pub struct McpServer {
    registry: Arc<Registry>,
    confirm_policy: Arc<dyn crate::policy::ConfirmPolicy>,
    audit: Arc<dyn crate::audit::AuditSink>,
}

impl McpServer {
    pub fn new(registry: Arc<Registry>) -> Self {
        Self {
            registry,
            // Fail closed: a host that wires nothing permits nothing.
            confirm_policy: Arc::new(crate::policy::AlwaysDeny),
            audit: Arc::new(crate::audit::NoopAudit),
        }
    }

    pub fn with_policy(mut self, policy: Arc<dyn crate::policy::ConfirmPolicy>) -> Self {
        self.confirm_policy = policy;
        self
    }

    pub fn confirm_policy(&self) -> &Arc<dyn crate::policy::ConfirmPolicy> {
        &self.confirm_policy
    }

    pub fn with_audit(mut self, audit: Arc<dyn crate::audit::AuditSink>) -> Self {
        self.audit = audit;
        self
    }

    pub fn audit(&self) -> &Arc<dyn crate::audit::AuditSink> {
        &self.audit
    }

    /// Whether a tool reads sensitive material, so the audit log can redact
    /// its arguments wholesale.
    pub fn is_sensitive(&self, name: &str) -> bool {
        self.registry
            .get(name)
            .map(|c| c.annotations.sensitive)
            .unwrap_or(false)
    }

    pub fn list_tools(&self) -> Vec<ToolDescriptor> {
        self.registry
            .ids()
            .into_iter()
            .filter_map(|id| self.registry.get(id))
            .map(|cap| ToolDescriptor {
                name: cap.id.clone(),
                description: cap.summary.clone(),
                input_schema: cap.input_schema.clone(),
            })
            .collect()
    }

    pub async fn call_tool(&self, name: &str, args: Value) -> Result<Value, CapabilityError> {
        self.registry.invoke(name, args).await
    }

    /// Whether a tool mutates the cluster and should be consent-gated over
    /// remote transports (destructive or confirmation-requiring capabilities).
    pub fn requires_confirm(&self, name: &str) -> bool {
        self.registry
            .get(name)
            .map(|c| c.annotations.requires_confirm || c.annotations.destructive)
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use srelens_capability::Capability;
    use serde_json::json;

    fn registry_with_ping() -> Arc<Registry> {
        let mut reg = Registry::new();
        reg.register(Capability::read_only("ping", "health check", |v| async move {
            Ok(json!({ "echo": v }))
        }));
        Arc::new(reg)
    }

    #[test]
    fn list_tools_mirrors_registry() {
        let server = McpServer::new(registry_with_ping());
        let tools = server.list_tools();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "ping");
        assert_eq!(tools[0].description, "health check");
    }

    #[tokio::test]
    async fn call_tool_invokes_capability() {
        let server = McpServer::new(registry_with_ping());
        let out = server.call_tool("ping", json!("hi")).await.unwrap();
        assert_eq!(out, json!({ "echo": "hi" }));
    }
}
