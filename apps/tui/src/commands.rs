#![allow(dead_code)]

use std::fmt;

/// Supported Resource Views in the SRElens TUI
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ResourceKind {
    Pods,
    Deployments,
    StatefulSets,
    DaemonSets,
    Jobs,
    CronJobs,
    ConfigMaps,
    Secrets,
    ResourceQuotas,
    LimitRanges,
    Services,
    Endpoints,
    EndpointSlices,
    Ingresses,
    NetworkPolicies,
    PersistentVolumeClaims,
    PersistentVolumes,
    StorageClasses,
    ServiceAccounts,
    Roles,
    ClusterRoles,
    RoleBindings,
    ClusterRoleBindings,
    Nodes,
    Namespaces,
    Events,
    CustomResourceDefinitions,
    CustomResource(CrdMeta),
    HelmReleases,
    PortForwards,
    Overview,
    Toolbox,
    Assistant,
    Settings,
    TuiConfig,
    Workloads,
    Topology,
    GpuInfo,
    TopPods,
    TopNodes,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct PrinterColumn {
    pub name: String,
    pub json_path: String,
    pub col_type: String,
    pub priority: i32,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct CrdMeta {
    pub crd_name: String,
    pub group: String,
    pub version: String,
    pub kind: String,
    pub plural: String,
    pub singular: String,
    pub namespaced: bool,
    pub short_names: Vec<String>,
    #[serde(default)]
    pub printer_columns: Vec<PrinterColumn>,
}

impl ResourceKind {
    pub fn display_name(&self) -> &str {
        match self {
            Self::Pods => "Pods",
            Self::Deployments => "Deployments",
            Self::StatefulSets => "StatefulSets",
            Self::DaemonSets => "DaemonSets",
            Self::Jobs => "Jobs",
            Self::CronJobs => "CronJobs",
            Self::ConfigMaps => "ConfigMaps",
            Self::Secrets => "Secrets",
            Self::ResourceQuotas => "ResourceQuotas",
            Self::LimitRanges => "LimitRanges",
            Self::Services => "Services",
            Self::Endpoints => "Endpoints",
            Self::EndpointSlices => "EndpointSlices",
            Self::Ingresses => "Ingresses",
            Self::NetworkPolicies => "NetworkPolicies",
            Self::PersistentVolumeClaims => "PersistentVolumeClaims",
            Self::PersistentVolumes => "PersistentVolumes",
            Self::StorageClasses => "StorageClasses",
            Self::ServiceAccounts => "ServiceAccounts",
            Self::Roles => "Roles",
            Self::ClusterRoles => "ClusterRoles",
            Self::RoleBindings => "RoleBindings",
            Self::ClusterRoleBindings => "ClusterRoleBindings",
            Self::Nodes => "Nodes",
            Self::Namespaces => "Namespaces",
            Self::Events => "Events",
            Self::CustomResourceDefinitions => "CustomResourceDefinitions",
            Self::CustomResource(crd) => crd.kind.as_str(),
            Self::HelmReleases => "Helm Releases",
            Self::PortForwards => "Port Forwards",
            Self::Overview => "Cluster Overview",
            Self::Toolbox => "Toolbox Diagnostics",
            Self::Assistant => "SRElens Assistant",
            Self::Settings => "AI & Assistant Settings",
            Self::TuiConfig => "TUI Configuration",
            Self::Workloads => "Workloads",
            Self::Topology => "Workload & Traffic Topology",
            Self::GpuInfo => "GPU Info & VRAM Allocation",
            Self::TopPods => "Top Pods",
            Self::TopNodes => "Top Nodes",
        }
    }

    pub fn watch_kind(&self) -> Option<&str> {
        match self {
            Self::Pods => Some("pods"),
            Self::Deployments => Some("deployments"),
            Self::StatefulSets => Some("statefulsets"),
            Self::DaemonSets => Some("daemonsets"),
            Self::Jobs => Some("jobs"),
            Self::CronJobs => Some("cronjobs"),
            Self::ConfigMaps => Some("configmaps"),
            Self::Secrets => Some("secrets"),
            Self::ResourceQuotas => Some("resourcequotas"),
            Self::LimitRanges => Some("limitranges"),
            Self::Services => Some("services"),
            Self::EndpointSlices => Some("endpointslices"),
            Self::Ingresses => Some("ingresses"),
            Self::NetworkPolicies => Some("networkpolicies"),
            Self::PersistentVolumeClaims => Some("persistentvolumeclaims"),
            Self::PersistentVolumes => Some("persistentvolumes"),
            Self::StorageClasses => Some("storageclasses"),
            Self::ServiceAccounts => Some("serviceaccounts"),
            Self::Roles => Some("roles"),
            Self::ClusterRoles => Some("clusterroles"),
            Self::RoleBindings => Some("rolebindings"),
            Self::ClusterRoleBindings => Some("clusterrolebindings"),
            Self::Nodes => Some("nodes"),
            Self::Namespaces => Some("namespaces"),
            Self::Events => Some("events"),
            _ => None,
        }
    }

    pub fn k8s_kind(&self) -> Option<&str> {
        match self {
            Self::Pods => Some("Pod"),
            Self::Deployments => Some("Deployment"),
            Self::StatefulSets => Some("StatefulSet"),
            Self::DaemonSets => Some("DaemonSet"),
            Self::Jobs => Some("Job"),
            Self::CronJobs => Some("CronJob"),
            Self::ConfigMaps => Some("ConfigMap"),
            Self::Secrets => Some("Secret"),
            Self::ResourceQuotas => Some("ResourceQuota"),
            Self::LimitRanges => Some("LimitRange"),
            Self::Services => Some("Service"),
            Self::Endpoints => Some("Endpoints"),
            Self::EndpointSlices => Some("EndpointSlice"),
            Self::Ingresses => Some("Ingress"),
            Self::NetworkPolicies => Some("NetworkPolicy"),
            Self::PersistentVolumeClaims => Some("PersistentVolumeClaim"),
            Self::PersistentVolumes => Some("PersistentVolume"),
            Self::StorageClasses => Some("StorageClass"),
            Self::ServiceAccounts => Some("ServiceAccount"),
            Self::Roles => Some("Role"),
            Self::ClusterRoles => Some("ClusterRole"),
            Self::RoleBindings => Some("RoleBinding"),
            Self::ClusterRoleBindings => Some("ClusterRoleBinding"),
            Self::Nodes => Some("Node"),
            Self::Namespaces => Some("Namespace"),
            Self::Events => Some("Event"),
            Self::CustomResourceDefinitions => Some("CustomResourceDefinition"),
            Self::CustomResource(crd) => Some(crd.kind.as_str()),
            _ => None,
        }
    }

    pub fn is_namespaced(&self) -> bool {
        match self {
            Self::CustomResource(crd) => crd.namespaced,
            Self::Nodes
            | Self::Namespaces
            | Self::PersistentVolumes
            | Self::StorageClasses
            | Self::ClusterRoles
            | Self::ClusterRoleBindings
            | Self::CustomResourceDefinitions
            | Self::Overview
            | Self::Toolbox
            | Self::Assistant => false,
            _ => true,
        }
    }
}

impl fmt::Display for ResourceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

pub struct CommandDef {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub description: &'static str,
    pub target: CommandTarget,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandTarget {
    Resource(ResourceKind),
    CustomResource(CrdMeta),
    Contexts,
    Namespaces,
    Help,
    Quit,
    OpenUrl(String),
    ThemePicker,
    SetTheme(String),
}

pub const COMMAND_REGISTRY: &[CommandDef] = &[
    CommandDef {
        name: "themes",
        aliases: &["theme", "colors"],
        description: "Interactive theme selector (Catppuccin, Tokyo Night, Dracula, Nord, Gruvbox, etc.)",
        target: CommandTarget::ThemePicker,
    },
    CommandDef {
        name: "workloads",
        aliases: &["wl", "workload"],
        description: "Unified Workloads view (Deployments, StatefulSets, DaemonSets, Pods, CronJobs)",
        target: CommandTarget::Resource(ResourceKind::Workloads),
    },
    CommandDef {
        name: "topology",
        aliases: &["topo", "flow"],
        description: "Workload and traffic topology flow map (:topology, :topo, :flow)",
        target: CommandTarget::Resource(ResourceKind::Topology),
    },
    CommandDef {
        name: "gpuinfo",
        aliases: &["gpu", "gpus"],
        description: "GPU nodes, VRAM allocation & GPU workloads (:gpuinfo, :gpu, :gpus)",
        target: CommandTarget::Resource(ResourceKind::GpuInfo),
    },
    CommandDef {
        name: "top",
        aliases: &["hotspots", "ranking"],
        description: "Top Hotspots ranking (Pods & Nodes) (:top, :toppods, :topnodes)",
        target: CommandTarget::Resource(ResourceKind::TopPods),
    },
    CommandDef {
        name: "toppods",
        aliases: &["top pods", "top-pods", "tp"],
        description: "Top Pods ranking by CPU & Memory (:toppods, :top pods)",
        target: CommandTarget::Resource(ResourceKind::TopPods),
    },
    CommandDef {
        name: "topnodes",
        aliases: &["top nodes", "top-nodes", "tn"],
        description: "Top Nodes ranking by CPU & Memory (:topnodes, :top nodes)",
        target: CommandTarget::Resource(ResourceKind::TopNodes),
    },
    CommandDef {
        name: "pods",
        aliases: &["po", "pod"],
        description: "Pods view",
        target: CommandTarget::Resource(ResourceKind::Pods),
    },
    CommandDef {
        name: "deployments",
        aliases: &["dp", "deploy"],
        description: "Deployments view",
        target: CommandTarget::Resource(ResourceKind::Deployments),
    },
    CommandDef {
        name: "statefulsets",
        aliases: &["sts", "statefulset"],
        description: "StatefulSets view",
        target: CommandTarget::Resource(ResourceKind::StatefulSets),
    },
    CommandDef {
        name: "daemonsets",
        aliases: &["ds", "daemonset"],
        description: "DaemonSets view",
        target: CommandTarget::Resource(ResourceKind::DaemonSets),
    },
    CommandDef {
        name: "jobs",
        aliases: &["job"],
        description: "Jobs view",
        target: CommandTarget::Resource(ResourceKind::Jobs),
    },
    CommandDef {
        name: "cronjobs",
        aliases: &["cj", "cronjob"],
        description: "CronJobs view",
        target: CommandTarget::Resource(ResourceKind::CronJobs),
    },
    CommandDef {
        name: "services",
        aliases: &["svc", "service"],
        description: "Services view",
        target: CommandTarget::Resource(ResourceKind::Services),
    },
    CommandDef {
        name: "ingresses",
        aliases: &["ing", "ingress"],
        description: "Ingresses view",
        target: CommandTarget::Resource(ResourceKind::Ingresses),
    },
    CommandDef {
        name: "endpointslices",
        aliases: &["ep", "eps", "endpointslice"],
        description: "EndpointSlices view",
        target: CommandTarget::Resource(ResourceKind::EndpointSlices),
    },
    CommandDef {
        name: "networkpolicies",
        aliases: &["np", "netpol"],
        description: "NetworkPolicies view",
        target: CommandTarget::Resource(ResourceKind::NetworkPolicies),
    },
    CommandDef {
        name: "configmaps",
        aliases: &["cm", "configmap"],
        description: "ConfigMaps view",
        target: CommandTarget::Resource(ResourceKind::ConfigMaps),
    },
    CommandDef {
        name: "secrets",
        aliases: &["sec", "secret"],
        description: "Secrets view",
        target: CommandTarget::Resource(ResourceKind::Secrets),
    },
    CommandDef {
        name: "persistentvolumeclaims",
        aliases: &["pvc"],
        description: "PersistentVolumeClaims view",
        target: CommandTarget::Resource(ResourceKind::PersistentVolumeClaims),
    },
    CommandDef {
        name: "persistentvolumes",
        aliases: &["pv"],
        description: "PersistentVolumes view",
        target: CommandTarget::Resource(ResourceKind::PersistentVolumes),
    },
    CommandDef {
        name: "storageclasses",
        aliases: &["sc", "storageclass"],
        description: "StorageClasses view",
        target: CommandTarget::Resource(ResourceKind::StorageClasses),
    },
    CommandDef {
        name: "nodes",
        aliases: &["no", "node"],
        description: "Nodes view",
        target: CommandTarget::Resource(ResourceKind::Nodes),
    },
    CommandDef {
        name: "namespaces",
        aliases: &["ns", "namespace"],
        description: "Namespaces view / Switcher",
        target: CommandTarget::Namespaces,
    },
    CommandDef {
        name: "events",
        aliases: &["ev", "event"],
        description: "Cluster Events stream",
        target: CommandTarget::Resource(ResourceKind::Events),
    },
    CommandDef {
        name: "serviceaccounts",
        aliases: &["sa", "serviceaccount"],
        description: "ServiceAccounts view",
        target: CommandTarget::Resource(ResourceKind::ServiceAccounts),
    },
    CommandDef {
        name: "roles",
        aliases: &["role"],
        description: "Roles view",
        target: CommandTarget::Resource(ResourceKind::Roles),
    },
    CommandDef {
        name: "clusterroles",
        aliases: &["cr", "clusterrole"],
        description: "ClusterRoles view",
        target: CommandTarget::Resource(ResourceKind::ClusterRoles),
    },
    CommandDef {
        name: "rolebindings",
        aliases: &["rb", "rolebinding"],
        description: "RoleBindings view",
        target: CommandTarget::Resource(ResourceKind::RoleBindings),
    },
    CommandDef {
        name: "clusterrolebindings",
        aliases: &["crb", "clusterrolebinding"],
        description: "ClusterRoleBindings view",
        target: CommandTarget::Resource(ResourceKind::ClusterRoleBindings),
    },
    CommandDef {
        name: "crds",
        aliases: &["crd", "customresourcedefinitions"],
        description: "Custom Resource Definitions",
        target: CommandTarget::Resource(ResourceKind::CustomResourceDefinitions),
    },
    CommandDef {
        name: "helm",
        aliases: &["helmreleases", "releases"],
        description: "Helm 3 Releases",
        target: CommandTarget::Resource(ResourceKind::HelmReleases),
    },
    CommandDef {
        name: "portforwards",
        aliases: &["pf", "portforward"],
        description: "Active Port Forwards",
        target: CommandTarget::Resource(ResourceKind::PortForwards),
    },
    CommandDef {
        name: "contexts",
        aliases: &["ctx", "context"],
        description: "Cluster Contexts Switcher",
        target: CommandTarget::Contexts,
    },
    CommandDef {
        name: "overview",
        aliases: &["info", "cluster"],
        description: "Cluster Overview & Health",
        target: CommandTarget::Resource(ResourceKind::Overview),
    },
    CommandDef {
        name: "toolbox",
        aliases: &["tb", "tools"],
        description: "Toolbox diagnostics (kubectl, helm, krew)",
        target: CommandTarget::Resource(ResourceKind::Toolbox),
    },
    CommandDef {
        name: "assistant",
        aliases: &["ai", "chat"],
        description: "SRElens AI Assistant Chat",
        target: CommandTarget::Resource(ResourceKind::Assistant),
    },
    CommandDef {
        name: "ai-settings",
        aliases: &["settings", "ai-config"],
        description: "AI & Assistant Settings",
        target: CommandTarget::Resource(ResourceKind::Settings),
    },
    CommandDef {
        name: "config",
        aliases: &["tui-config", "tui"],
        description: "TUI Configuration & Popup Size",
        target: CommandTarget::Resource(ResourceKind::TuiConfig),
    },
    CommandDef {
        name: "help",
        aliases: &["?"],
        description: "Show keybindings and command help",
        target: CommandTarget::Help,
    },
    CommandDef {
        name: "quit",
        aliases: &["q", "exit"],
        description: "Quit srelens",
        target: CommandTarget::Quit,
    },
    CommandDef {
        name: "open",
        aliases: &["goto", "url"],
        description: "Open deep link URL or resource (:open <url>)",
        target: CommandTarget::OpenUrl(String::new()),
    },
];

#[derive(Debug, Clone, PartialEq)]
pub struct DynamicCommandDef {
    pub name: String,
    pub aliases: Vec<String>,
    pub description: String,
    pub target: CommandTarget,
}

impl From<&'static CommandDef> for DynamicCommandDef {
    fn from(cmd: &'static CommandDef) -> Self {
        Self {
            name: cmd.name.to_string(),
            aliases: cmd.aliases.iter().map(|a| a.to_string()).collect(),
            description: cmd.description.to_string(),
            target: cmd.target.clone(),
        }
    }
}

impl From<&CrdMeta> for DynamicCommandDef {
    fn from(crd: &CrdMeta) -> Self {
        let mut aliases = Vec::new();
        if !crd.singular.is_empty() && crd.singular != crd.plural {
            aliases.push(crd.singular.clone());
        }
        for short in &crd.short_names {
            if !aliases.contains(short) {
                aliases.push(short.clone());
            }
        }
        let norm_plural = crd.plural.to_lowercase().replace("loadbalancer", "lb");
        if norm_plural != crd.plural && !aliases.contains(&norm_plural) {
            aliases.push(norm_plural);
        }
        let norm_singular = crd.singular.to_lowercase().replace("loadbalancer", "lb");
        if norm_singular != crd.singular && !aliases.contains(&norm_singular) {
            aliases.push(norm_singular);
        }

        Self {
            name: crd.plural.clone(),
            aliases,
            description: format!("CRD: {} ({})", crd.kind, crd.group),
            target: CommandTarget::CustomResource(crd.clone()),
        }
    }
}

pub fn resolve_command(input: &str) -> Option<CommandTarget> {
    resolve_command_with_crds(input, &[])
}

pub fn resolve_command_with_crds(input: &str, crds: &[CrdMeta]) -> Option<CommandTarget> {
    let trimmed = input.trim().trim_start_matches(':').trim();
    if trimmed.is_empty() {
        return None;
    }

    // Direct deep link URL (e.g. ":srelens://...")
    if trimmed.starts_with("srelens://") {
        return Some(CommandTarget::OpenUrl(trimmed.to_string()));
    }

    // Open command prefix: "open <url>", "goto <url>", "open:<url>"
    for prefix in &["open ", "open:", "goto ", "goto:"] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            return Some(CommandTarget::OpenUrl(rest.trim().to_string()));
        }
    }

    // Direct theme setter command (e.g. ":theme tokyo-night" or ":colors nord")
    for prefix in &["theme ", "theme:", "colors ", "colors:"] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let name = rest.trim();
            if !name.is_empty() {
                return Some(CommandTarget::SetTheme(name.to_string()));
            }
        }
    }

    // 1. Exact match on static command primary name
    for cmd in COMMAND_REGISTRY {
        if cmd.name.eq_ignore_ascii_case(trimmed) {
            return Some(cmd.target.clone());
        }
    }

    // 2. Exact match on CRDs & aliases (cluster CRDs take precedence over static command aliases)
    let q = trimmed.to_lowercase();
    for crd in crds {
        if crd.plural.eq_ignore_ascii_case(&q)
            || crd.singular.eq_ignore_ascii_case(&q)
            || crd.kind.eq_ignore_ascii_case(&q)
            || crd.crd_name.eq_ignore_ascii_case(&q)
        {
            return Some(CommandTarget::CustomResource(crd.clone()));
        }
        for short in &crd.short_names {
            if short.eq_ignore_ascii_case(&q) {
                return Some(CommandTarget::CustomResource(crd.clone()));
            }
        }
        let norm_plural = crd.plural.to_lowercase().replace("loadbalancer", "lb");
        let norm_singular = crd.singular.to_lowercase().replace("loadbalancer", "lb");
        if norm_plural == q || norm_singular == q {
            return Some(CommandTarget::CustomResource(crd.clone()));
        }
    }

    // 3. Exact match on static command aliases (e.g. ":po" -> pods, ":settings" -> ai-settings if no CRD exists)
    for cmd in COMMAND_REGISTRY {
        for alias in cmd.aliases {
            if alias.eq_ignore_ascii_case(trimmed) {
                return Some(cmd.target.clone());
            }
        }
    }

    // 4. Prefix match on CRDs (e.g. if q is at least 3 chars)
    if q.len() >= 3 {
        for crd in crds {
            let norm_plural = crd.plural.to_lowercase().replace("loadbalancer", "lb");
            let norm_singular = crd.singular.to_lowercase().replace("loadbalancer", "lb");
            if crd.plural.to_lowercase().starts_with(&q)
                || crd.singular.to_lowercase().starts_with(&q)
                || norm_plural.starts_with(&q)
                || norm_singular.starts_with(&q)
            {
                return Some(CommandTarget::CustomResource(crd.clone()));
            }
        }
    }

    // 5. Prefix match on static commands & aliases (e.g. ":deplo" -> deployments)
    for cmd in COMMAND_REGISTRY {
        if cmd.name.starts_with(&q) {
            return Some(cmd.target.clone());
        }
        for alias in cmd.aliases {
            if alias.starts_with(&q) {
                return Some(cmd.target.clone());
            }
        }
    }

    if let Some(ns) = trimmed.strip_prefix("ns ") {
        let ns_clean = ns.trim();
        if !ns_clean.is_empty() {
            return Some(CommandTarget::Namespaces);
        }
    }

    if let Some(ctx) = trimmed.strip_prefix("ctx ") {
        let ctx_clean = ctx.trim();
        if !ctx_clean.is_empty() {
            return Some(CommandTarget::Contexts);
        }
    }

    None
}

pub fn command_suggestions(query: &str) -> Vec<(DynamicCommandDef, usize)> {
    command_suggestions_with_crds(query, &[])
}

pub fn command_suggestions_with_crds(query: &str, crds: &[CrdMeta]) -> Vec<(DynamicCommandDef, usize)> {
    let q = query.trim().trim_start_matches(':').to_lowercase();
    let mut matches = Vec::new();

    if q.is_empty() {
        for cmd in COMMAND_REGISTRY {
            matches.push((DynamicCommandDef::from(cmd), 0));
        }
        for crd in crds.iter().take(30) {
            matches.push((DynamicCommandDef::from(crd), 0));
        }
        return matches;
    }

    // Direct theme name suggestions when typing ":theme <subquery>" or ":colors <subquery>"
    if q.starts_with("theme ") || q.starts_with("colors ") {
        let prefix = if q.starts_with("theme ") { "theme " } else { "colors " };
        let sub = q.strip_prefix(prefix).unwrap().trim();
        for p in crate::theme::ALL_THEMES {
            if sub.is_empty() || p.name.starts_with(sub) || p.display_name.to_lowercase().starts_with(sub) {
                matches.push((
                    DynamicCommandDef {
                        name: format!("theme {}", p.name),
                        aliases: vec![],
                        description: format!("Theme: {} ({})", p.display_name, p.description),
                        target: CommandTarget::SetTheme(p.name.to_string()),
                    },
                    150,
                ));
            }
        }
        if !matches.is_empty() {
            matches.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.name.cmp(&b.0.name)));
            return matches;
        }
    }

    // Match static registry
    for cmd in COMMAND_REGISTRY {
        if cmd.name == q || cmd.aliases.iter().any(|a| *a == q) {
            matches.push((DynamicCommandDef::from(cmd), 120));
        } else if cmd.name.starts_with(&q) {
            matches.push((DynamicCommandDef::from(cmd), 100));
        } else if cmd.aliases.iter().any(|a| a.starts_with(&q)) {
            matches.push((DynamicCommandDef::from(cmd), 80));
        } else if cmd.name.contains(&q) || cmd.description.to_lowercase().contains(&q) {
            matches.push((DynamicCommandDef::from(cmd), 50));
        }
    }

    // Match dynamic CRD registry
    for crd in crds {
        let crd_def = DynamicCommandDef::from(crd);
        let norm_plural = crd.plural.to_lowercase().replace("loadbalancer", "lb");
        let norm_singular = crd.singular.to_lowercase().replace("loadbalancer", "lb");

        if crd.plural.to_lowercase() == q
            || crd.singular.to_lowercase() == q
            || crd.kind.to_lowercase() == q
            || norm_plural == q
            || norm_singular == q
            || crd_def.aliases.iter().any(|a| a.to_lowercase() == q)
        {
            matches.push((crd_def, 120));
        } else if crd.plural.to_lowercase().starts_with(&q)
            || crd.singular.to_lowercase().starts_with(&q)
            || crd.kind.to_lowercase().starts_with(&q)
            || norm_plural.starts_with(&q)
            || norm_singular.starts_with(&q)
        {
            matches.push((crd_def, 105));
        } else if crd_def.aliases.iter().any(|a| a.to_lowercase().starts_with(&q)) {
            matches.push((crd_def, 95));
        } else if crd.plural.to_lowercase().contains(&q)
            || crd.kind.to_lowercase().contains(&q)
            || crd.group.to_lowercase().contains(&q)
            || crd.crd_name.to_lowercase().contains(&q)
        {
            matches.push((crd_def, 55));
        }
    }

    matches.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.name.cmp(&b.0.name)));
    matches
}
