use crate::commands::{resolve_command, CommandTarget};

/// Supported SRElens Deep Links
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeepLink {
    /// Deep link to a specific Kubernetes resource
    /// `srelens://resource/<context>/<namespace>/<kind>/<name>`
    /// Cluster-scoped resources (Node, Namespace, ClusterRole, etc.) can have `_`, `_all`, or `-` as namespace.
    Resource {
        context: String,
        namespace: Option<String>,
        kind: String,
        name: String,
    },
    /// Deep link to switch to a cluster context
    /// `srelens://cluster/<context>`
    Cluster {
        context: String,
    },
    /// Deep link to a specific view
    /// `srelens://view/<context>/<namespace>/<view>`
    View {
        context: Option<String>,
        namespace: Option<String>,
        target: CommandTarget,
    },
}

impl DeepLink {
    /// Formats the canonical URL string for this deep link
    pub fn to_url(&self) -> String {
        match self {
            Self::Resource { context, namespace, kind, name } => {
                let ctx = if context.is_empty() { "_" } else { context };
                let ns = namespace.as_deref().unwrap_or("_");
                let ns_part = if ns.is_empty() { "_" } else { ns };
                format!("srelens://resource/{}/{}/{}/{}", ctx, ns_part, kind, name)
            }
            Self::Cluster { context } => {
                format!("srelens://cluster/{}", context)
            }
            Self::View { context, namespace, target } => {
                let ctx = context.as_deref().unwrap_or("_");
                let ns = namespace.as_deref().unwrap_or("_");
                let target_name = match target {
                    CommandTarget::Resource(k) => match k {
                        crate::commands::ResourceKind::Assistant => "ai".to_string(),
                        crate::commands::ResourceKind::Overview => "overview".to_string(),
                        crate::commands::ResourceKind::Toolbox => "toolbox".to_string(),
                        crate::commands::ResourceKind::Settings => "settings".to_string(),
                        other => other.to_string().to_lowercase(),
                    },
                    CommandTarget::CustomResource(crd) => crd.plural.to_lowercase(),
                    CommandTarget::Help => "help".to_string(),
                    CommandTarget::Contexts => "contexts".to_string(),
                    CommandTarget::Namespaces => "namespaces".to_string(),
                    CommandTarget::Quit => "quit".to_string(),
                    CommandTarget::OpenUrl(u) => format!("open/{}", u),
                };
                format!("srelens://view/{}/{}/{}", ctx, ns, target_name)
            }
        }
    }

    /// Parses a URL or CLI target into a DeepLink
    pub fn parse(input: &str) -> Result<Self, String> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err("Empty deep link URL".to_string());
        }

        // 1. Check if standard srelens:// schema
        if let Some(rest) = trimmed.strip_prefix("srelens://") {
            let parts: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
            if parts.is_empty() {
                return Err("Missing target in srelens:// URL".to_string());
            }

            match parts[0] {
                "resource" => {
                    // srelens://resource/<context>/<namespace>/<kind>/<name>
                    if parts.len() < 5 {
                        return Err("Invalid resource deep link. Expected format: srelens://resource/<context>/<namespace>/<kind>/<name>".to_string());
                    }
                    let context = parts[1].to_string();
                    let raw_ns = parts[2];
                    let namespace = if raw_ns == "_" || raw_ns == "_all" || raw_ns == "-" || raw_ns.is_empty() {
                        None
                    } else {
                        Some(raw_ns.to_string())
                    };
                    let kind = parts[3].to_string();
                    let name = parts[4].to_string();

                    Ok(Self::Resource { context, namespace, kind, name })
                }
                "cluster" | "context" => {
                    // srelens://cluster/<context>
                    if parts.len() < 2 {
                        return Err("Invalid cluster deep link. Expected format: srelens://cluster/<context>".to_string());
                    }
                    Ok(Self::Cluster { context: parts[1].to_string() })
                }
                "view" => {
                    // srelens://view/<context>/<namespace>/<target>
                    if parts.len() < 4 {
                        return Err("Invalid view deep link. Expected format: srelens://view/<context>/<namespace>/<target>".to_string());
                    }
                    let raw_ctx = parts[1];
                    let context = if raw_ctx == "_" || raw_ctx.is_empty() { None } else { Some(raw_ctx.to_string()) };
                    let raw_ns = parts[2];
                    let namespace = if raw_ns == "_" || raw_ns == "_all" || raw_ns == "-" || raw_ns.is_empty() {
                        None
                    } else {
                        Some(raw_ns.to_string())
                    };
                    let view_str = parts[3];
                    let target = resolve_command(format!(":{}", view_str).as_str())
                        .or_else(|| resolve_command(view_str))
                        .ok_or_else(|| format!("Unknown view target: '{}'", view_str))?;

                    Ok(Self::View { context, namespace, target })
                }
                other => Err(format!("Unknown srelens URL route '{}'. Supported routes: 'resource', 'cluster', 'view'", other)),
            }
        } else if trimmed.contains('/') {
            // Shorthand format: <kind>/<name> or <namespace>/<kind>/<name> or <context>/<namespace>/<kind>/<name>
            let parts: Vec<&str> = trimmed.split('/').filter(|s| !s.is_empty()).collect();
            match parts.len() {
                2 => {
                    // <kind>/<name>
                    let kind = parts[0].to_string();
                    let name = parts[1].to_string();
                    Ok(Self::Resource {
                        context: String::new(),
                        namespace: None,
                        kind,
                        name,
                    })
                }
                3 => {
                    // <namespace>/<kind>/<name>
                    let namespace = if parts[0] == "_" || parts[0] == "_all" || parts[0] == "-" {
                        None
                    } else {
                        Some(parts[0].to_string())
                    };
                    let kind = parts[1].to_string();
                    let name = parts[2].to_string();
                    Ok(Self::Resource {
                        context: String::new(),
                        namespace,
                        kind,
                        name,
                    })
                }
                4 => {
                    // <context>/<namespace>/<kind>/<name>
                    let context = parts[0].to_string();
                    let namespace = if parts[1] == "_" || parts[1] == "_all" || parts[1] == "-" {
                        None
                    } else {
                        Some(parts[1].to_string())
                    };
                    let kind = parts[2].to_string();
                    let name = parts[3].to_string();
                    Ok(Self::Resource {
                        context,
                        namespace,
                        kind,
                        name,
                    })
                }
                _ => Err(format!("Unrecognized shorthand format '{}'. Expected [context/][namespace/]kind/name", trimmed)),
            }
        } else {
            // Direct command target (e.g. "pods", "nodes")
            if let Some(target) = resolve_command(format!(":{}", trimmed).as_str()).or_else(|| resolve_command(trimmed)) {
                Ok(Self::View {
                    context: None,
                    namespace: None,
                    target,
                })
            } else {
                Err(format!("Unrecognized target or URL: '{}'", trimmed))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_resource_deep_link() {
        let url = "srelens://resource/prod-eu/default/Pod/backend-api-789";
        let link = DeepLink::parse(url).expect("should parse");
        assert_eq!(
            link,
            DeepLink::Resource {
                context: "prod-eu".to_string(),
                namespace: Some("default".to_string()),
                kind: "Pod".to_string(),
                name: "backend-api-789".to_string(),
            }
        );
        assert_eq!(link.to_url(), url);
    }

    #[test]
    fn test_parse_cluster_scoped_resource_deep_link() {
        let url = "srelens://resource/prod-eu/_/Node/metal64";
        let link = DeepLink::parse(url).expect("should parse");
        assert_eq!(
            link,
            DeepLink::Resource {
                context: "prod-eu".to_string(),
                namespace: None,
                kind: "Node".to_string(),
                name: "metal64".to_string(),
            }
        );
        assert_eq!(link.to_url(), url);
    }

    #[test]
    fn test_parse_cluster_deep_link() {
        let url = "srelens://cluster/harvester-amd-eu-dus1";
        let link = DeepLink::parse(url).expect("should parse");
        assert_eq!(
            link,
            DeepLink::Cluster {
                context: "harvester-amd-eu-dus1".to_string(),
            }
        );
        assert_eq!(link.to_url(), url);
    }

    #[test]
    fn test_parse_shorthand_resource() {
        let shorthand = "pods/my-service-pod";
        let link = DeepLink::parse(shorthand).expect("should parse");
        assert_eq!(
            link,
            DeepLink::Resource {
                context: String::new(),
                namespace: None,
                kind: "pods".to_string(),
                name: "my-service-pod".to_string(),
            }
        );
    }
}
