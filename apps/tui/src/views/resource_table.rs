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
        }
    }

    pub fn set_items(&mut self, items: Vec<Value>, filter_query: &str) {
        self.raw_items = items;
        self.is_loading = false;
        self.apply_filter(filter_query);
    }

    pub fn apply_filter(&mut self, filter_query: &str) {
        let q = filter_query.trim();
        self.filtered_indices = if q.is_empty() {
            (0..self.raw_items.len()).collect()
        } else {
            let re = regex::RegexBuilder::new(q)
                .case_insensitive(true)
                .build()
                .ok();

            let q_lower = q.to_lowercase();
            self.raw_items
                .iter()
                .enumerate()
                .filter(|(_, item)| {
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
                .collect()
        };

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
            cols.push(ColumnDef { name: "NAME", key: "name", width: Constraint::Min(30) });
            cols.push(ColumnDef { name: "AGE", key: "age", width: Constraint::Length(8) });
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

pub fn extract_field_str<'a>(val: &'a Value, key: &str) -> String {
    let key_lower = key.to_lowercase();

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

    "-".to_string()
}

pub fn render_resource_table(f: &mut Frame, area: Rect, state: &ResourceTableState) {
    let title = if state.is_loading {
        format!(" {} [Loading...] ", state.kind.display_name())
    } else {
        format!(" {} [{}] ", state.kind.display_name(), state.filtered_indices.len())
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::BORDER))
        .title(Span::styled(title, Theme::title()));

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
                let text = extract_field_str(item, col.key);
                let cell_style = if col.key == "status" || col.key == "type" && state.kind == ResourceKind::Events {
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
}
