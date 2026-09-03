use ratatui::{
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
    Frame,
};
use serde_json::Value;
use std::collections::HashSet;

use crate::commands::ResourceKind;
use crate::theme::{status_style, Theme};

#[derive(Debug, Clone)]
pub struct ColumnDef {
    pub name: &'static str,
    pub key: &'static str,
    pub width: Constraint,
}

#[derive(Debug, Clone)]
pub struct ResourceTableState {
    pub kind: ResourceKind,
    pub columns: Vec<ColumnDef>,
    pub raw_items: Vec<Value>,
    pub filtered_indices: Vec<usize>,
    pub selected_idx: usize,
    pub scroll_offset: usize,
    pub marked_indices: HashSet<usize>,
    pub sort_col_idx: Option<usize>,
    pub sort_ascending: bool,
    pub is_loading: bool,
    pub warning_triage: bool,
}

impl ResourceTableState {
    pub fn new(kind: ResourceKind) -> Self {
        let columns = default_columns_for_kind(&kind);
        Self {
            kind,
            columns,
            raw_items: Vec::new(),
            filtered_indices: Vec::new(),
            selected_idx: 0,
            scroll_offset: 0,
            marked_indices: HashSet::new(),
            sort_col_idx: None,
            sort_ascending: true,
            is_loading: true,
            warning_triage: false,
        }
    }

    pub fn toggle_warning_triage(&mut self, filter_query: &str) -> bool {
        self.warning_triage = !self.warning_triage;
        self.apply_filter(filter_query);
        self.warning_triage
    }

    pub fn set_items(&mut self, items: Vec<Value>, filter_query: &str) {
        self.raw_items = items;
        self.is_loading = false;
        self.apply_filter(filter_query);
    }

    pub fn apply_filter(&mut self, filter_query: &str) {
        let q = filter_query.trim();
        let re = if !q.is_empty() {
            regex::RegexBuilder::new(q)
                .case_insensitive(true)
                .build()
                .ok()
        } else {
            None
        };
        let q_lower = q.to_lowercase();

        self.filtered_indices = self.raw_items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                // Warning triage filter for Events
                if self.kind == ResourceKind::Events && self.warning_triage {
                    if !is_event_warning_or_failure(item) {
                        return false;
                    }
                }

                if q.is_empty() {
                    return true;
                }

                let name = item.get("name")
                    .and_then(|v| v.as_str())
                    .or_else(|| item.get("metadata").and_then(|m| m.get("name")).and_then(|v| v.as_str()))
                    .unwrap_or("");
                let ns = item.get("namespace")
                    .and_then(|v| v.as_str())
                    .or_else(|| item.get("metadata").and_then(|m| m.get("namespace")).and_then(|v| v.as_str()))
                    .unwrap_or("");
                let full_str = item.to_string();

                if let Some(ref regex) = re {
                    if regex.is_match(name) || regex.is_match(ns) || regex.is_match(&full_str) {
                        return true;
                    }
                }

                name.to_lowercase().contains(&q_lower)
                    || ns.to_lowercase().contains(&q_lower)
                    || full_str.to_lowercase().contains(&q_lower)
            })
            .map(|(i, _)| i)
            .collect();

        if self.selected_idx >= self.filtered_indices.len() {
            self.selected_idx = self.filtered_indices.len().saturating_sub(1);
        }
    }

    pub fn select_next(&mut self) {
        if !self.filtered_indices.is_empty() && self.selected_idx + 1 < self.filtered_indices.len() {
            self.selected_idx += 1;
        }
    }

    pub fn select_prev(&mut self) {
        if self.selected_idx > 0 {
            self.selected_idx -= 1;
        }
    }

    pub fn select_top(&mut self) {
        self.selected_idx = 0;
    }

    pub fn select_bottom(&mut self) {
        if !self.filtered_indices.is_empty() {
            self.selected_idx = self.filtered_indices.len().saturating_sub(1);
        }
    }

    pub fn page_down(&mut self, page_size: usize) {
        if !self.filtered_indices.is_empty() {
            self.selected_idx = (self.selected_idx + page_size).min(self.filtered_indices.len() - 1);
        }
    }

    pub fn page_up(&mut self, page_size: usize) {
        self.selected_idx = self.selected_idx.saturating_sub(page_size);
    }

    pub fn toggle_mark_selected(&mut self) {
        if let Some(&raw_idx) = self.filtered_indices.get(self.selected_idx) {
            if self.marked_indices.contains(&raw_idx) {
                self.marked_indices.remove(&raw_idx);
            } else {
                self.marked_indices.insert(raw_idx);
            }
        }
    }

    pub fn selected_item(&self) -> Option<&Value> {
        self.filtered_indices
            .get(self.selected_idx)
            .and_then(|&idx| self.raw_items.get(idx))
    }

    pub fn selected_resource_name(&self) -> Option<String> {
        let item = self.selected_item()?;
        item.get("name")
            .and_then(|v| v.as_str())
            .or_else(|| item.get("metadata").and_then(|m| m.get("name")).and_then(|v| v.as_str()))
            .map(String::from)
    }

    pub fn selected_namespace(&self) -> Option<String> {
        let item = self.selected_item()?;
        item.get("namespace")
            .and_then(|v| v.as_str())
            .or_else(|| item.get("metadata").and_then(|m| m.get("namespace")).and_then(|v| v.as_str()))
            .map(String::from)
    }
}

pub fn default_columns_for_kind(kind: &ResourceKind) -> Vec<ColumnDef> {
    match kind {
        ResourceKind::Pods => vec![
            ColumnDef { name: "NAMESPACE", key: "namespace", width: Constraint::Length(18) },
            ColumnDef { name: "NAME", key: "name", width: Constraint::Min(25) },
            ColumnDef { name: "READY", key: "ready", width: Constraint::Length(8) },
            ColumnDef { name: "STATUS", key: "status", width: Constraint::Length(14) },
            ColumnDef { name: "RESTARTS", key: "restarts", width: Constraint::Length(10) },
            ColumnDef { name: "CPU", key: "cpu", width: Constraint::Length(8) },
            ColumnDef { name: "MEM", key: "memory", width: Constraint::Length(8) },
            ColumnDef { name: "IP", key: "podIp", width: Constraint::Length(16) },
            ColumnDef { name: "NODE", key: "nodeName", width: Constraint::Length(20) },
            ColumnDef { name: "AGE", key: "age", width: Constraint::Length(8) },
        ],
        ResourceKind::Deployments => vec![
            ColumnDef { name: "NAMESPACE", key: "namespace", width: Constraint::Length(18) },
            ColumnDef { name: "NAME", key: "name", width: Constraint::Min(25) },
            ColumnDef { name: "READY", key: "ready", width: Constraint::Length(10) },
            ColumnDef { name: "UP-TO-DATE", key: "upToDate", width: Constraint::Length(12) },
            ColumnDef { name: "AVAILABLE", key: "available", width: Constraint::Length(12) },
            ColumnDef { name: "AGE", key: "age", width: Constraint::Length(8) },
        ],
        ResourceKind::StatefulSets => vec![
            ColumnDef { name: "NAMESPACE", key: "namespace", width: Constraint::Length(18) },
            ColumnDef { name: "NAME", key: "name", width: Constraint::Min(25) },
            ColumnDef { name: "READY", key: "ready", width: Constraint::Length(10) },
            ColumnDef { name: "AGE", key: "age", width: Constraint::Length(8) },
        ],
        ResourceKind::DaemonSets => vec![
            ColumnDef { name: "NAMESPACE", key: "namespace", width: Constraint::Length(18) },
            ColumnDef { name: "NAME", key: "name", width: Constraint::Min(25) },
            ColumnDef { name: "DESIRED", key: "desired", width: Constraint::Length(10) },
            ColumnDef { name: "CURRENT", key: "current", width: Constraint::Length(10) },
            ColumnDef { name: "READY", key: "ready", width: Constraint::Length(10) },
            ColumnDef { name: "UP-TO-DATE", key: "upToDate", width: Constraint::Length(12) },
            ColumnDef { name: "AVAILABLE", key: "available", width: Constraint::Length(12) },
            ColumnDef { name: "AGE", key: "age", width: Constraint::Length(8) },
        ],
        ResourceKind::Jobs => vec![
            ColumnDef { name: "NAMESPACE", key: "namespace", width: Constraint::Length(18) },
            ColumnDef { name: "NAME", key: "name", width: Constraint::Min(25) },
            ColumnDef { name: "COMPLETIONS", key: "completions", width: Constraint::Length(14) },
            ColumnDef { name: "DURATION", key: "duration", width: Constraint::Length(12) },
            ColumnDef { name: "AGE", key: "age", width: Constraint::Length(8) },
        ],
        ResourceKind::CronJobs => vec![
            ColumnDef { name: "NAMESPACE", key: "namespace", width: Constraint::Length(18) },
            ColumnDef { name: "NAME", key: "name", width: Constraint::Min(25) },
            ColumnDef { name: "SCHEDULE", key: "schedule", width: Constraint::Length(18) },
            ColumnDef { name: "SUSPEND", key: "suspend", width: Constraint::Length(10) },
            ColumnDef { name: "ACTIVE", key: "active", width: Constraint::Length(8) },
            ColumnDef { name: "LAST SCHEDULE", key: "lastSchedule", width: Constraint::Length(16) },
            ColumnDef { name: "AGE", key: "age", width: Constraint::Length(8) },
        ],
        ResourceKind::Services => vec![
            ColumnDef { name: "NAMESPACE", key: "namespace", width: Constraint::Length(18) },
            ColumnDef { name: "NAME", key: "name", width: Constraint::Min(25) },
            ColumnDef { name: "TYPE", key: "type", width: Constraint::Length(14) },
            ColumnDef { name: "CLUSTER-IP", key: "clusterIP", width: Constraint::Length(16) },
            ColumnDef { name: "EXTERNAL-IP", key: "externalIP", width: Constraint::Length(18) },
            ColumnDef { name: "PORTS", key: "ports", width: Constraint::Min(20) },
            ColumnDef { name: "AGE", key: "age", width: Constraint::Length(8) },
        ],
        ResourceKind::Ingresses => vec![
            ColumnDef { name: "NAMESPACE", key: "namespace", width: Constraint::Length(18) },
            ColumnDef { name: "NAME", key: "name", width: Constraint::Min(25) },
            ColumnDef { name: "CLASS", key: "class", width: Constraint::Length(14) },
            ColumnDef { name: "HOSTS", key: "hosts", width: Constraint::Min(25) },
            ColumnDef { name: "ADDRESS", key: "address", width: Constraint::Length(18) },
            ColumnDef { name: "PORTS", key: "ports", width: Constraint::Length(12) },
            ColumnDef { name: "AGE", key: "age", width: Constraint::Length(8) },
        ],
        ResourceKind::EndpointSlices => vec![
            ColumnDef { name: "NAMESPACE", key: "namespace", width: Constraint::Length(18) },
            ColumnDef { name: "NAME", key: "name", width: Constraint::Min(25) },
            ColumnDef { name: "ADDRESS-TYPE", key: "addressType", width: Constraint::Length(14) },
            ColumnDef { name: "PORTS", key: "ports", width: Constraint::Min(20) },
            ColumnDef { name: "ENDPOINTS", key: "endpoints", width: Constraint::Min(20) },
            ColumnDef { name: "AGE", key: "age", width: Constraint::Length(8) },
        ],
        ResourceKind::ConfigMaps => vec![
            ColumnDef { name: "NAMESPACE", key: "namespace", width: Constraint::Length(18) },
            ColumnDef { name: "NAME", key: "name", width: Constraint::Min(30) },
            ColumnDef { name: "DATA", key: "dataCount", width: Constraint::Length(10) },
            ColumnDef { name: "AGE", key: "age", width: Constraint::Length(8) },
        ],
        ResourceKind::Secrets => vec![
            ColumnDef { name: "NAMESPACE", key: "namespace", width: Constraint::Length(18) },
            ColumnDef { name: "NAME", key: "name", width: Constraint::Min(30) },
            ColumnDef { name: "TYPE", key: "type", width: Constraint::Length(25) },
            ColumnDef { name: "DATA", key: "dataCount", width: Constraint::Length(10) },
            ColumnDef { name: "AGE", key: "age", width: Constraint::Length(8) },
        ],
        ResourceKind::PersistentVolumeClaims => vec![
            ColumnDef { name: "NAMESPACE", key: "namespace", width: Constraint::Length(18) },
            ColumnDef { name: "NAME", key: "name", width: Constraint::Min(25) },
            ColumnDef { name: "STATUS", key: "status", width: Constraint::Length(12) },
            ColumnDef { name: "VOLUME", key: "volume", width: Constraint::Min(20) },
            ColumnDef { name: "CAPACITY", key: "capacity", width: Constraint::Length(12) },
            ColumnDef { name: "ACCESS MODES", key: "accessModes", width: Constraint::Length(14) },
            ColumnDef { name: "STORAGECLASS", key: "storageClass", width: Constraint::Length(16) },
            ColumnDef { name: "AGE", key: "age", width: Constraint::Length(8) },
        ],
        ResourceKind::PersistentVolumes => vec![
            ColumnDef { name: "NAME", key: "name", width: Constraint::Min(30) },
            ColumnDef { name: "CAPACITY", key: "capacity", width: Constraint::Length(12) },
            ColumnDef { name: "ACCESS MODES", key: "accessModes", width: Constraint::Length(14) },
            ColumnDef { name: "RECLAIM POLICY", key: "reclaimPolicy", width: Constraint::Length(16) },
            ColumnDef { name: "STATUS", key: "status", width: Constraint::Length(12) },
            ColumnDef { name: "CLAIM", key: "claim", width: Constraint::Min(25) },
            ColumnDef { name: "STORAGECLASS", key: "storageClass", width: Constraint::Length(16) },
            ColumnDef { name: "AGE", key: "age", width: Constraint::Length(8) },
        ],
        ResourceKind::StorageClasses => vec![
            ColumnDef { name: "NAME", key: "name", width: Constraint::Min(30) },
            ColumnDef { name: "PROVISIONER", key: "provisioner", width: Constraint::Min(30) },
            ColumnDef { name: "RECLAIMPOLICY", key: "reclaimPolicy", width: Constraint::Length(16) },
            ColumnDef { name: "VOLUMEBINDINGMODE", key: "volumeBindingMode", width: Constraint::Length(22) },
            ColumnDef { name: "ALLOWEXPANSION", key: "allowVolumeExpansion", width: Constraint::Length(16) },
            ColumnDef { name: "AGE", key: "age", width: Constraint::Length(8) },
        ],
        ResourceKind::Nodes => vec![
            ColumnDef { name: "NAME", key: "name", width: Constraint::Min(30) },
            ColumnDef { name: "STATUS", key: "status", width: Constraint::Length(14) },
            ColumnDef { name: "ROLES", key: "roles", width: Constraint::Length(16) },
            ColumnDef { name: "VERSION", key: "version", width: Constraint::Length(14) },
            ColumnDef { name: "CPU", key: "allocatableCpuMillicores", width: Constraint::Length(12) },
            ColumnDef { name: "MEMORY", key: "allocatableMemoryMiB", width: Constraint::Length(12) },
            ColumnDef { name: "PODS", key: "allocatablePods", width: Constraint::Length(10) },
            ColumnDef { name: "AGE", key: "age", width: Constraint::Length(8) },
        ],
        ResourceKind::Namespaces => vec![
            ColumnDef { name: "NAME", key: "name", width: Constraint::Min(35) },
            ColumnDef { name: "STATUS", key: "status", width: Constraint::Length(14) },
            ColumnDef { name: "AGE", key: "age", width: Constraint::Length(8) },
        ],
        ResourceKind::Events => vec![
            ColumnDef { name: "NAMESPACE", key: "namespace", width: Constraint::Length(18) },
            ColumnDef { name: "LAST SEEN", key: "age", width: Constraint::Length(12) },
            ColumnDef { name: "TYPE", key: "type", width: Constraint::Length(10) },
            ColumnDef { name: "REASON", key: "reason", width: Constraint::Length(18) },
            ColumnDef { name: "OBJECT", key: "involvedObject", width: Constraint::Length(25) },
            ColumnDef { name: "MESSAGE", key: "message", width: Constraint::Min(40) },
        ],
        ResourceKind::CustomResourceDefinitions => vec![
            ColumnDef { name: "NAME", key: "name", width: Constraint::Min(35) },
            ColumnDef { name: "GROUP", key: "group", width: Constraint::Length(25) },
            ColumnDef { name: "VERSION", key: "version", width: Constraint::Length(12) },
            ColumnDef { name: "SCOPE", key: "scope", width: Constraint::Length(14) },
            ColumnDef { name: "AGE", key: "age", width: Constraint::Length(8) },
        ],
        ResourceKind::CustomResource(crd) => {
            let mut cols = Vec::new();
            if crd.namespaced {
                cols.push(ColumnDef { name: "NAMESPACE", key: "namespace", width: Constraint::Length(18) });
            }
            cols.push(ColumnDef { name: "NAME", key: "name", width: Constraint::Min(25) });

            if crd.printer_columns.is_empty() {
                cols.push(ColumnDef { name: "AGE", key: "age", width: Constraint::Length(8) });
            } else {
                let mut has_age = false;
                for pc in &crd.printer_columns {
                    if pc.priority > 0 {
                        continue;
                    }
                    let col_name_upper = pc.name.to_uppercase();
                    if col_name_upper == "AGE" {
                        has_age = true;
                    }
                    let width = match col_name_upper.as_str() {
                        "READY" => Constraint::Length(8),
                        "STATUS" => Constraint::Length(18),
                        "STORETYPE" => Constraint::Length(14),
                        "STORE" => Constraint::Min(22),
                        "REFRESH INTERVAL" => Constraint::Length(18),
                        "LAST SYNC" | "LASTSYNC" => Constraint::Length(12),
                        "AGE" => Constraint::Length(8),
                        _ => Constraint::Length(16),
                    };
                    let name_str: &'static str = Box::leak(pc.name.to_uppercase().into_boxed_str());
                    let key_str: &'static str = Box::leak(format!("printer:{}", pc.json_path).into_boxed_str());
                    cols.push(ColumnDef {
                        name: name_str,
                        key: key_str,
                        width,
                    });
                }
                if !has_age {
                    cols.push(ColumnDef { name: "AGE", key: "age", width: Constraint::Length(8) });
                }
            }
            cols
        }
        _ => vec![
            ColumnDef { name: "NAMESPACE", key: "namespace", width: Constraint::Length(18) },
            ColumnDef { name: "NAME", key: "name", width: Constraint::Min(30) },
            ColumnDef { name: "AGE", key: "age", width: Constraint::Length(8) },
        ],
    }
}

fn raw_value_to_string(v: &Value) -> Option<String> {
    if let Some(s) = v.as_str() {
        Some(s.to_string())
    } else if let Some(n) = v.as_i64() {
        Some(n.to_string())
    } else if let Some(b) = v.as_bool() {
        Some(b.to_string())
    } else if let Some(arr) = v.as_array() {
        let items: Vec<_> = arr.iter().filter_map(|item| item.as_str()).collect();
        if !items.is_empty() {
            Some(items.join(", "))
        } else {
            None
        }
    } else {
        None
    }
}

pub fn is_event_warning_or_failure(item: &Value) -> bool {
    let type_str = item.get("type")
        .or_else(|| item.get("type_"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if type_str.eq_ignore_ascii_case("Warning") {
        return true;
    }

    let reason = item.get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_lowercase();
    let critical_reasons = [
        "fail", "backoff", "crashloop", "oom", "evict", "unhealthy", 
        "killing", "notready", "error", "warn", "invalidspec", "pressure"
    ];
    for cr in critical_reasons {
        if reason.contains(cr) {
            return true;
        }
    }

    let msg = item.get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_lowercase();
    if msg.contains("oomkilled") || msg.contains("crashloopbackoff") || msg.contains("failed") || msg.contains("evicted") {
        return true;
    }

    false
}

#[derive(Debug, PartialEq, Eq)]
enum PathSegment {
    Field(String),
    Index(usize),
    Filter {
        field: String,
        match_key: String,
        match_val: String,
    },
}

fn parse_json_path_segments(path: &str) -> Vec<PathSegment> {
    let mut segments = Vec::new();
    let mut parts = Vec::new();
    let mut in_bracket = false;
    let mut start = 0;

    for (i, c) in path.char_indices() {
        if c == '[' {
            in_bracket = true;
        } else if c == ']' {
            in_bracket = false;
        } else if c == '.' && !in_bracket {
            if i > start {
                let segment = &path[start..i];
                if !segment.is_empty() {
                    parts.push(segment);
                }
            }
            start = i + 1;
        }
    }
    if start < path.len() {
        let segment = &path[start..];
        if !segment.is_empty() {
            parts.push(segment);
        }
    }

    for part in parts {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }

        if let Some(bracket_start) = part.find('[') {
            let field = &part[..bracket_start];
            let rest = &part[bracket_start..];
            if rest.starts_with("[?(@.") {
                // e.g. conditions[?(@.type=="Ready")]
                let inner = rest.strip_prefix("[?(@.").unwrap_or("");
                let inner = inner.trim_end_matches(|c| c == ']' || c == ')' || c == ' ');
                if let Some((k, v)) = inner.split_once("==") {
                    let match_key = k.trim().to_string();
                    let match_val = v.trim().trim_matches(|c| c == '"' || c == '\'' || c == ')' || c == ']').to_string();
                    segments.push(PathSegment::Filter {
                        field: field.to_string(),
                        match_key,
                        match_val,
                    });
                } else if !field.is_empty() {
                    segments.push(PathSegment::Field(field.to_string()));
                }
            } else if let Some(idx_str) = rest.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                if !field.is_empty() {
                    segments.push(PathSegment::Field(field.to_string()));
                }
                if let Ok(idx) = idx_str.parse::<usize>() {
                    segments.push(PathSegment::Index(idx));
                } else {
                    let key = idx_str.trim_matches('"').trim_matches('\'');
                    segments.push(PathSegment::Field(key.to_string()));
                }
            } else if !field.is_empty() {
                segments.push(PathSegment::Field(field.to_string()));
            }
        } else {
            segments.push(PathSegment::Field(part.to_string()));
        }
    }

    segments
}

pub fn eval_crd_json_path(val: &Value, raw_path: &str) -> String {
    let path = raw_path.trim().trim_start_matches('.');
    if path.is_empty() {
        return "-".to_string();
    }

    let mut current = val;
    let segments = parse_json_path_segments(path);

    for seg in &segments {
        match seg {
            PathSegment::Field(name) => {
                if let Some(next) = current.get(name) {
                    current = next;
                } else {
                    return "-".to_string();
                }
            }
            PathSegment::Index(idx) => {
                if let Some(next) = current.get(idx) {
                    current = next;
                } else {
                    return "-".to_string();
                }
            }
            PathSegment::Filter { field, match_key, match_val } => {
                let target = if field.is_empty() {
                    Some(current)
                } else {
                    current.get(field)
                };
                if let Some(arr) = target.and_then(|v| v.as_array()) {
                    let matched = arr.iter().find(|item| {
                        item.get(match_key)
                            .and_then(|v| v.as_str())
                            .map(|s| s == match_val)
                            .unwrap_or(false)
                    });
                    if let Some(next) = matched {
                        current = next;
                    } else {
                        return "-".to_string();
                    }
                } else {
                    return "-".to_string();
                }
            }
        }
    }

    format_crd_cell_value(current)
}

fn format_crd_cell_value(val: &Value) -> String {
    match val {
        Value::String(s) => {
            if s.is_empty() {
                return "-".to_string();
            }
            // Check if RFC3339 timestamp (e.g. 2026-09-03T09:04:38Z)
            if (s.contains('T') && s.ends_with('Z')) || (s.len() >= 20 && s.contains('-') && s.contains(':')) {
                if let Ok(ts) = s.parse::<srelens_kube::k8s_openapi::jiff::Timestamp>() {
                    let k8s_time = srelens_kube::k8s_openapi::apimachinery::pkg::apis::meta::v1::Time(ts);
                    let age = srelens_kube::humanize_age(Some(&k8s_time));
                    if !age.is_empty() {
                        return age;
                    }
                }
            }
            s.clone()
        }
        Value::Bool(b) => {
            if *b { "True".to_string() } else { "False".to_string() }
        }
        Value::Number(n) => n.to_string(),
        Value::Array(arr) => {
            let strs: Vec<_> = arr.iter().map(format_crd_cell_value).filter(|s| s != "-").collect();
            if strs.is_empty() {
                "-".to_string()
            } else {
                strs.join(", ")
            }
        }
        Value::Object(_) | Value::Null => "-".to_string(),
    }
}

pub fn extract_field_str<'a>(val: &'a Value, key: &str) -> String {
    // CRD printer column lookup (starts with printer:)
    if let Some(path) = key.strip_prefix("printer:") {
        return eval_crd_json_path(val, path);
    }

    // Dynamic age: when the summary carries a raw ISO 8601 creation timestamp
    // in `createdAt`, recompute the human-readable age from it so the column
    // never goes stale between watch re-lists.
    if key == "age" {
        if let Some(ts_str) = val.get("createdAt").and_then(|v| v.as_str()) {
            if !ts_str.is_empty() {
                if let Ok(ts) = ts_str.parse::<srelens_kube::k8s_openapi::jiff::Timestamp>() {
                    let now = srelens_kube::k8s_openapi::jiff::Timestamp::now();
                    let secs = now.duration_since(ts).as_secs();
                    return srelens_kube::format_age(secs);
                }
            }
        }
    }

    let key_lower = key.to_lowercase();

    // 0. Event specific field aliases
    if key_lower == "involvedobject" || key_lower == "object" {
        if let Some(obj) = val.get("object").and_then(|v| v.as_str()) {
            if !obj.is_empty() {
                return obj.to_string();
            }
        }
        if let Some(inv) = val.get("involvedObject") {
            let kind = inv.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            let name = inv.get("name").and_then(|v| v.as_str()).unwrap_or("");
            if !kind.is_empty() || !name.is_empty() {
                return format!("{}/{}", kind, name);
            }
        }
    }
    if key_lower == "type" || key_lower == "type_" {
        if let Some(t) = val.get("type").or_else(|| val.get("type_")).and_then(|v| v.as_str()) {
            if !t.is_empty() {
                return t.to_string();
            }
        }
    }

    // 1. Direct key lookup
    if let Some(v) = val.get(key) {
        if let Some(s) = raw_value_to_string(v) {
            if !s.is_empty() {
                return s;
            }
        }
    }

    // 2. Case-insensitive top-level lookup (e.g. clusterIP vs clusterIp, externalIP vs externalIp)
    if let Some(obj) = val.as_object() {
        for (k, v) in obj {
            if k.to_lowercase() == key_lower {
                if let Some(s) = raw_value_to_string(v) {
                    if !s.is_empty() {
                        return s;
                    }
                }
            }
        }
    }

    // 3. Fallback: spec, status, metadata
    for container in &["spec", "status", "metadata"] {
        if let Some(sub) = val.get(*container) {
            if let Some(v) = sub.get(key) {
                if let Some(s) = raw_value_to_string(v) {
                    if !s.is_empty() {
                        return s;
                    }
                }
            }
            if let Some(obj) = sub.as_object() {
                for (k, v) in obj {
                    if k.to_lowercase() == key_lower {
                        if let Some(s) = raw_value_to_string(v) {
                            if !s.is_empty() {
                                return s;
                            }
                        }
                    }
                }
            }
        }
    }

    // 4. Kubernetes Service specific lookups
    if key_lower == "clusterip" {
        if let Some(cip) = val.pointer("/spec/clusterIP").and_then(|v| v.as_str()) {
            return cip.to_string();
        }
    }

    if key_lower == "externalip" {
        // Check load-balancer ingress (IP or Hostname)
        if let Some(ingresses) = val.pointer("/status/loadBalancer/ingress").and_then(|v| v.as_array()) {
            let ips: Vec<_> = ingresses
                .iter()
                .filter_map(|ing| ing.get("ip").or_else(|| ing.get("hostname")).and_then(|v| v.as_str()))
                .collect();
            if !ips.is_empty() {
                return ips.join(", ");
            }
        }
        if let Some(ext_ips) = val.pointer("/spec/externalIPs").and_then(|v| v.as_array()) {
            let ips: Vec<_> = ext_ips.iter().filter_map(|v| v.as_str()).collect();
            if !ips.is_empty() {
                return ips.join(", ");
            }
        }
        // If it's a Service with type LoadBalancer but no ingress yet:
        let svc_type = val.get("type").or_else(|| val.pointer("/spec/type")).and_then(|v| v.as_str());
        if svc_type == Some("LoadBalancer") {
            return "<pending>".to_string();
        }
        if svc_type.is_some() {
            return "<none>".to_string();
        }
    }

    // 5. Pod specific lookups (status -> phase, nodeName -> node, podIP)
    if key_lower == "status" {
        if let Some(phase) = val.get("phase").or_else(|| val.pointer("/status/phase")).and_then(|v| v.as_str()) {
            return phase.to_string();
        }
    }
    if key_lower == "nodename" || key_lower == "node" {
        if let Some(node) = val.get("node").or_else(|| val.pointer("/spec/nodeName")).and_then(|v| v.as_str()) {
            return node.to_string();
        }
    }
    if key_lower == "podip" || key_lower == "ip" {
        if let Some(ip) = val.get("podIp").or_else(|| val.get("podIP")).or_else(|| val.pointer("/status/podIP")).and_then(|v| v.as_str()) {
            return ip.to_string();
        }
    }
    if key_lower == "cpu" {
        if let Some(cpu) = val.get("cpu").or_else(|| val.get("cpuUsage")).and_then(|v| v.as_str()) {
            if !cpu.is_empty() {
                return cpu.to_string();
            }
        }
        if let Some(req_cpu) = val.pointer("/spec/containers/0/resources/requests/cpu").and_then(|v| v.as_str()) {
            return req_cpu.to_string();
        }
    }
    if key_lower == "memory" || key_lower == "mem" {
        if let Some(mem) = val.get("memory").or_else(|| val.get("memUsage")).and_then(|v| v.as_str()) {
            if !mem.is_empty() {
                return mem.to_string();
            }
        }
        if let Some(req_mem) = val.pointer("/spec/containers/0/resources/requests/memory").and_then(|v| v.as_str()) {
            return req_mem.to_string();
        }
    }

    "-".to_string()
}

pub fn render_resource_table(f: &mut Frame, area: Rect, state: &ResourceTableState) {
    let triage_badge = if state.kind == ResourceKind::Events && state.warning_triage {
        " [WARNING TRIAGE: ON]"
    } else {
        ""
    };

    let count_badge = if !state.is_loading && state.filtered_indices.len() != state.raw_items.len() {
        format!(" [{}/{}]", state.filtered_indices.len(), state.raw_items.len())
    } else if state.is_loading {
        " [Loading...]".to_string()
    } else {
        format!(" [{}]", state.filtered_indices.len())
    };

    let title = format!(" {}{}{} ", state.kind.display_name(), count_badge, triage_badge);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(if state.kind == ResourceKind::Events && state.warning_triage {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Theme::BORDER)
        })
        .title(Span::styled(title, if state.kind == ResourceKind::Events && state.warning_triage {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Theme::title()
        }));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if state.is_loading {
        let loading_msg = Paragraph::new(Line::from(vec![
            Span::styled("⚡ Loading ", Style::default().fg(Theme::CYAN).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{} from cluster API...", state.kind.display_name()), Style::default().fg(Theme::DIM)),
        ]));
        f.render_widget(loading_msg, inner);
        return;
    }

    if state.filtered_indices.is_empty() {
        let empty_msg = Paragraph::new(Line::from(vec![
            Span::styled(format!("No {} found in this scope.", state.kind.display_name()), Style::default().fg(Theme::DIM)),
        ]));
        f.render_widget(empty_msg, inner);
        return;
    }

    // Prepare table headers
    let header_cells = state.columns.iter().map(|col| {
        Cell::from(col.name).style(Theme::table_header())
    });
    let header = Row::new(header_cells).height(1).bottom_margin(1);

    // Prepare rows
    let visible_rows_count = inner.height.saturating_sub(2) as usize;
    let start_idx = if state.selected_idx >= visible_rows_count {
        state.selected_idx.saturating_sub(visible_rows_count / 2)
    } else {
        0
    };
    let end_idx = (start_idx + visible_rows_count).min(state.filtered_indices.len());

    let rows: Vec<Row> = (start_idx..end_idx)
        .map(|display_idx| {
            let raw_idx = state.filtered_indices[display_idx];
            let item = &state.raw_items[raw_idx];
            let is_selected = display_idx == state.selected_idx;
            let is_marked = state.marked_indices.contains(&raw_idx);

            let cells = state.columns.iter().map(|col| {
                // Cluster-controlled text (event messages, annotations, CRD
                // fields) can carry tabs/escapes that desync the terminal.
                let text = super::sanitize_span_text(&extract_field_str(item, col.key));
                let is_crd = matches!(state.kind, ResourceKind::CustomResource(_));
                let is_status_col = col.key == "status"
                    || (state.kind == ResourceKind::Events && (col.key == "type" || col.key == "reason"))
                    || (is_crd && (col.name == "STATUS" || col.name == "READY" || col.name == "HEALTH" || col.name == "SYNC"));

                let cell_style = if is_status_col {
                    status_style(&text)
                } else if is_selected {
                    Style::default().fg(Theme::SEL_FG).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Theme::FG)
                };

                let prefix = if col.key == "name" && is_marked { "✔ " } else { "" };
                Cell::from(format!("{}{}", prefix, text)).style(cell_style)
            });

            let row_style = if is_selected {
                Theme::selected_row()
            } else if is_marked {
                Theme::marked_row()
            } else if display_idx % 2 == 0 {
                Style::default().bg(Color::Rgb(22, 24, 30))
            } else {
                Style::default()
            };

            Row::new(cells).style(row_style)
        })
        .collect();

    let widths: Vec<Constraint> = state.columns.iter().map(|c| c.width).collect();
    let table = Table::new(rows, widths).header(header);
    f.render_widget(table, inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_service_summary_fields() {
        let svc = json!({
            "name": "istio-ingress-internal",
            "namespace": "istio-system",
            "type": "LoadBalancer",
            "clusterIP": "10.255.45.10",
            "externalIP": "10.240.0.12",
            "ports": "80/TCP,443/TCP",
            "age": "12d"
        });

        // Exact keys
        assert_eq!(extract_field_str(&svc, "clusterIP"), "10.255.45.10");
        assert_eq!(extract_field_str(&svc, "externalIP"), "10.240.0.12");

        // Case-insensitive keys
        assert_eq!(extract_field_str(&svc, "clusterIp"), "10.255.45.10");
        assert_eq!(extract_field_str(&svc, "externalIp"), "10.240.0.12");
        assert_eq!(extract_field_str(&svc, "name"), "istio-ingress-internal");
        assert_eq!(extract_field_str(&svc, "namespace"), "istio-system");
        assert_eq!(extract_field_str(&svc, "type"), "LoadBalancer");
        assert_eq!(extract_field_str(&svc, "ports"), "80/TCP,443/TCP");
    }

    #[test]
    fn handles_cluster_ip_service_with_no_external_ip() {
        let svc = json!({
            "name": "kubernetes",
            "namespace": "default",
            "type": "ClusterIP",
            "clusterIP": "10.96.0.1",
            "externalIP": "",
            "ports": "443/TCP",
            "age": "30d"
        });

        assert_eq!(extract_field_str(&svc, "clusterIP"), "10.96.0.1");
        assert_eq!(extract_field_str(&svc, "clusterIp"), "10.96.0.1");
        assert_eq!(extract_field_str(&svc, "externalIP"), "<none>");
        assert_eq!(extract_field_str(&svc, "externalIp"), "<none>");
    }

    #[test]
    fn handles_load_balancer_pending_external_ip() {
        let svc = json!({
            "name": "my-lb",
            "namespace": "default",
            "type": "LoadBalancer",
            "clusterIP": "10.96.0.50",
            "externalIP": "<pending>",
            "ports": "80/TCP",
            "age": "1m"
        });

        assert_eq!(extract_field_str(&svc, "clusterIP"), "10.96.0.50");
        assert_eq!(extract_field_str(&svc, "externalIP"), "<pending>");
    }

    #[test]
    fn handles_raw_kubernetes_service_json() {
        let raw_svc = json!({
            "metadata": {
                "name": "raw-service",
                "namespace": "prod"
            },
            "spec": {
                "type": "LoadBalancer",
                "clusterIP": "10.100.1.20",
                "ports": [{"port": 80, "protocol": "TCP"}]
            },
            "status": {
                "loadBalancer": {
                    "ingress": [
                        {"ip": "35.200.10.5"}
                    ]
                }
            }
        });

        assert_eq!(extract_field_str(&raw_svc, "name"), "raw-service");
        assert_eq!(extract_field_str(&raw_svc, "namespace"), "prod");
        assert_eq!(extract_field_str(&raw_svc, "clusterIP"), "10.100.1.20");
        assert_eq!(extract_field_str(&raw_svc, "clusterIp"), "10.100.1.20");
        assert_eq!(extract_field_str(&raw_svc, "externalIP"), "35.200.10.5");
        assert_eq!(extract_field_str(&raw_svc, "externalIp"), "35.200.10.5");
    }

    #[test]
    fn handles_pod_summary_and_raw_pod() {
        let pod_summary = json!({
            "name": "web-pod-1",
            "namespace": "default",
            "phase": "Running",
            "ready": "1/1",
            "restarts": 0,
            "node": "node-1",
            "age": "5m"
        });

        assert_eq!(extract_field_str(&pod_summary, "status"), "Running");
        assert_eq!(extract_field_str(&pod_summary, "node"), "node-1");
        assert_eq!(extract_field_str(&pod_summary, "nodeName"), "node-1");

        let raw_pod = json!({
            "metadata": {"name": "raw-pod"},
            "status": {
                "phase": "Running",
                "podIP": "10.244.0.5"
            },
            "spec": {
                "nodeName": "worker-pool-1"
            }
        });

        assert_eq!(extract_field_str(&raw_pod, "status"), "Running");
        assert_eq!(extract_field_str(&raw_pod, "podIP"), "10.244.0.5");
        assert_eq!(extract_field_str(&raw_pod, "podIp"), "10.244.0.5");
        assert_eq!(extract_field_str(&raw_pod, "node"), "worker-pool-1");
    }

    #[test]
    fn test_events_field_extraction_and_warning_triage() {
        let ev1 = json!({
            "name": "default/pod-normal.17b",
            "namespace": "default",
            "type": "Normal",
            "reason": "Scheduled",
            "object": "Pod/frontend-web",
            "message": "Successfully assigned default/frontend-web to node-1",
            "age": "3m"
        });

        let ev2 = json!({
            "name": "prod/pod-crash.17c",
            "namespace": "prod",
            "type": "Warning",
            "reason": "CrashLoopBackOff",
            "object": "Pod/api-backend-xyz",
            "message": "Back-off restarting failed container",
            "age": "30s"
        });

        let ev3 = json!({
            "name": "prod/pod-oom.17d",
            "namespace": "prod",
            "type": "Warning",
            "reason": "OOMKilled",
            "involvedObject": {
                "kind": "Pod",
                "name": "ml-worker-gpu-0"
            },
            "message": "Container exceeded 32Gi memory limit and was killed",
            "age": "10s"
        });

        // Field extraction
        assert_eq!(extract_field_str(&ev1, "object"), "Pod/frontend-web");
        assert_eq!(extract_field_str(&ev1, "involvedObject"), "Pod/frontend-web");
        assert_eq!(extract_field_str(&ev3, "object"), "Pod/ml-worker-gpu-0");
        assert_eq!(extract_field_str(&ev3, "involvedObject"), "Pod/ml-worker-gpu-0");

        // Warning triage check
        assert!(!is_event_warning_or_failure(&ev1));
        assert!(is_event_warning_or_failure(&ev2));
        assert!(is_event_warning_or_failure(&ev3));

        // Table triage mode
        let mut table = ResourceTableState::new(ResourceKind::Events);
        table.set_items(vec![ev1, ev2, ev3], "");
        assert_eq!(table.filtered_indices.len(), 3);

        // Toggle triage ON
        let active = table.toggle_warning_triage("");
        assert!(active);
        assert_eq!(table.filtered_indices.len(), 2);

        // Toggle triage OFF
        let active = table.toggle_warning_triage("");
        assert!(!active);
        assert_eq!(table.filtered_indices.len(), 3);
    }

    #[test]
    fn test_crd_printer_columns_and_json_path_evaluator() {
        use crate::commands::{CrdMeta, PrinterColumn};

        let es_meta = CrdMeta {
            crd_name: "externalsecrets.external-secrets.io".to_string(),
            group: "external-secrets.io".to_string(),
            version: "v1".to_string(),
            kind: "ExternalSecret".to_string(),
            plural: "externalsecrets".to_string(),
            singular: "externalsecret".to_string(),
            namespaced: true,
            short_names: vec!["es".to_string()],
            printer_columns: vec![
                PrinterColumn {
                    name: "StoreType".to_string(),
                    json_path: ".spec.secretStoreRef.kind".to_string(),
                    col_type: "string".to_string(),
                    priority: 0,
                    description: None,
                },
                PrinterColumn {
                    name: "Store".to_string(),
                    json_path: ".spec.secretStoreRef.name".to_string(),
                    col_type: "string".to_string(),
                    priority: 0,
                    description: None,
                },
                PrinterColumn {
                    name: "Refresh Interval".to_string(),
                    json_path: ".spec.refreshInterval".to_string(),
                    col_type: "string".to_string(),
                    priority: 0,
                    description: None,
                },
                PrinterColumn {
                    name: "Status".to_string(),
                    json_path: ".status.conditions[?(@.type==\"Ready\")].reason".to_string(),
                    col_type: "string".to_string(),
                    priority: 0,
                    description: None,
                },
                PrinterColumn {
                    name: "Ready".to_string(),
                    json_path: ".status.conditions[?(@.type==\"Ready\")].status".to_string(),
                    col_type: "string".to_string(),
                    priority: 0,
                    description: None,
                },
                PrinterColumn {
                    name: "Last Sync".to_string(),
                    json_path: ".status.refreshTime".to_string(),
                    col_type: "date".to_string(),
                    priority: 0,
                    description: None,
                },
            ],
        };

        // Check generated columns
        let cols = default_columns_for_kind(&ResourceKind::CustomResource(es_meta.clone()));
        let col_names: Vec<&str> = cols.iter().map(|c| c.name).collect();
        assert_eq!(
            col_names,
            vec![
                "NAMESPACE",
                "NAME",
                "STORETYPE",
                "STORE",
                "REFRESH INTERVAL",
                "STATUS",
                "READY",
                "LAST SYNC",
                "AGE"
            ]
        );

        let es_json = json!({
            "metadata": {
                "name": "aip-secrets-binding",
                "namespace": "accommodation-identification-pipeline",
                "creationTimestamp": "2025-07-04T08:52:51Z"
            },
            "name": "aip-secrets-binding",
            "namespace": "accommodation-identification-pipeline",
            "age": "1y",
            "spec": {
                "refreshInterval": "1h",
                "secretStoreRef": {
                    "kind": "SecretStore",
                    "name": "trv-acc-ident-pipeline-prod"
                }
            },
            "status": {
                "conditions": [
                    {
                        "type": "Ready",
                        "status": "False",
                        "reason": "SecretSyncedError",
                        "message": "could not get secret data from provider"
                    }
                ]
            }
        });

        // Test field extraction
        assert_eq!(extract_field_str(&es_json, "printer:.spec.secretStoreRef.kind"), "SecretStore");
        assert_eq!(extract_field_str(&es_json, "printer:.spec.secretStoreRef.name"), "trv-acc-ident-pipeline-prod");
        assert_eq!(extract_field_str(&es_json, "printer:.spec.refreshInterval"), "1h");
        assert_eq!(extract_field_str(&es_json, "printer:.status.conditions[?(@.type==\"Ready\")].reason"), "SecretSyncedError");
        assert_eq!(extract_field_str(&es_json, "printer:.status.conditions[?(@.type==\"Ready\")].status"), "False");
        assert_eq!(extract_field_str(&es_json, "printer:.status.refreshTime"), "-");

        // Test with healthy item having refreshTime
        let healthy_es = json!({
            "name": "harvester-token-binding",
            "spec": {
                "refreshInterval": "1h",
                "secretStoreRef": {
                    "kind": "SecretStore",
                    "name": "harvester-token"
                }
            },
            "status": {
                "conditions": [
                    {
                        "type": "Ready",
                        "status": "True",
                        "reason": "SecretSynced"
                    }
                ],
                "refreshTime": "2026-09-03T09:04:38Z"
            }
        });

        assert_eq!(extract_field_str(&healthy_es, "printer:.status.conditions[?(@.type==\"Ready\")].reason"), "SecretSynced");
        assert_eq!(extract_field_str(&healthy_es, "printer:.status.conditions[?(@.type==\"Ready\")].status"), "True");
        assert_ne!(extract_field_str(&healthy_es, "printer:.status.refreshTime"), "-");
    }
}
