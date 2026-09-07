use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use srelens_kube::lineage::{LineageNode, LineageRelation};

use crate::theme::Theme;

#[derive(Debug, Clone)]
pub struct FlattenedTreeNode {
    pub kind: String,
    pub name: String,
    pub namespace: Option<String>,
    pub status: Option<String>,
    pub details: Option<String>,
    pub relationship: LineageRelation,
    pub depth: usize,
    pub is_last_child: bool,
    pub prefix: String,
}

pub struct TreeViewState {
    pub root_kind: String,
    pub root_name: String,
    pub namespace: Option<String>,
    pub nodes: Vec<FlattenedTreeNode>,
    pub raw_tree: Option<LineageNode>,
    pub selected_idx: usize,
    pub scroll_offset: usize,
    pub is_loading: bool,
    pub error: Option<String>,
}

impl TreeViewState {
    pub fn new(kind: String, name: String, namespace: Option<String>) -> Self {
        Self {
            root_kind: kind,
            root_name: name,
            namespace,
            nodes: Vec::new(),
            raw_tree: None,
            selected_idx: 0,
            scroll_offset: 0,
            is_loading: true,
            error: None,
        }
    }

    pub fn set_tree(&mut self, root: LineageNode) {
        let mut flattened = Vec::new();
        flatten_lineage(&root, 0, "", true, &mut flattened);

        // Find index of target node to select by default
        let target_idx = flattened
            .iter()
            .position(|n| n.relationship == LineageRelation::Target)
            .unwrap_or(0);

        self.nodes = flattened;
        self.raw_tree = Some(root);
        self.selected_idx = target_idx;
        self.scroll_offset = 0;
        self.is_loading = false;
        self.error = None;
    }

    pub fn set_error(&mut self, err: String) {
        self.is_loading = false;
        self.error = Some(err);
    }

    pub fn select_next(&mut self) {
        if !self.nodes.is_empty() && self.selected_idx + 1 < self.nodes.len() {
            self.selected_idx += 1;
        }
    }

    pub fn select_prev(&mut self) {
        self.selected_idx = self.selected_idx.saturating_sub(1);
    }

    pub fn select_first(&mut self) {
        self.selected_idx = 0;
    }

    pub fn select_last(&mut self) {
        if !self.nodes.is_empty() {
            self.selected_idx = self.nodes.len() - 1;
        }
    }

    pub fn selected_node(&self) -> Option<&FlattenedTreeNode> {
        self.nodes.get(self.selected_idx)
    }

    pub fn tree_as_text(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "Resource Relationship Tree for {}/{}\n",
            self.root_kind, self.root_name
        ));
        out.push_str(&"=".repeat(50));
        out.push('\n');

        for node in &self.nodes {
            let st = node.status.as_deref().unwrap_or("-");
            let dt = node.details.as_deref().unwrap_or("");
            out.push_str(&format!(
                "{}{}/{} {} [status: {}] {}\n",
                node.prefix,
                node.kind,
                node.name,
                node.relationship.badge(),
                st,
                dt
            ));
        }
        out
    }
}

fn flatten_lineage(
    node: &LineageNode,
    depth: usize,
    prefix: &str,
    is_last: bool,
    out: &mut Vec<FlattenedTreeNode>,
) {
    let connector = if depth == 0 {
        ""
    } else if is_last {
        "└── "
    } else {
        "├── "
    };
    let full_prefix = format!("{}{}", prefix, connector);

    out.push(FlattenedTreeNode {
        kind: node.kind.clone(),
        name: node.name.clone(),
        namespace: node.namespace.clone(),
        status: node.status.clone(),
        details: node.details.clone(),
        relationship: node.relationship,
        depth,
        is_last_child: is_last,
        prefix: full_prefix,
    });

    let child_prefix = if depth == 0 {
        ""
    } else if is_last {
        "    "
    } else {
        "│   "
    };
    let next_prefix = format!("{}{}", prefix, child_prefix);

    let count = node.children.len();
    for (i, child) in node.children.iter().enumerate() {
        flatten_lineage(child, depth + 1, &next_prefix, i == count - 1, out);
    }
}

pub fn render_tree_view(f: &mut Frame, area: Rect, state: &TreeViewState) {
    let title = format!(
        " 🌳 Resource Relationship Tree: {}/{} {} [↑/↓: Move, Enter: Jump, l: Logs, y: YAML, d: Describe, x: Actions, c: Copy, Esc: Back] ",
        state.root_kind,
        state.root_name,
        state.namespace.as_deref().unwrap_or(""),
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::BORDER))
        .title(Span::styled(title, Theme::title()));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if state.is_loading {
        let loading_line = Line::from(vec![
            Span::styled("⚡ Resolving resource lineage... ", Style::default().fg(Theme::CYAN).add_modifier(Modifier::BOLD)),
            Span::styled(format!("Tracing ownerReferences, dependents, and linked resources for {}/{}...", state.root_kind, state.root_name), Style::default().fg(Theme::DIM)),
        ]);
        f.render_widget(Paragraph::new(loading_line), inner);
        return;
    }

    if let Some(err) = &state.error {
        let err_line = Line::from(vec![
            Span::styled("⚠ Failed to resolve lineage: ", Style::default().fg(Theme::RED).add_modifier(Modifier::BOLD)),
            Span::styled(err.as_str(), Style::default().fg(Theme::FG)),
        ]);
        f.render_widget(Paragraph::new(err_line), inner);
        return;
    }

    if state.nodes.is_empty() {
        let empty_line = Line::from(vec![
            Span::styled("No relationships or lineage found for this resource.", Style::default().fg(Theme::DIM)),
        ]);
        f.render_widget(Paragraph::new(empty_line), inner);
        return;
    }

    let visible_rows = inner.height as usize;
    let start_idx = if state.selected_idx >= visible_rows {
        state.selected_idx.saturating_sub(visible_rows / 2)
    } else {
        0
    };
    let end_idx = (start_idx + visible_rows).min(state.nodes.len());

    let mut lines = Vec::new();
    for idx in start_idx..end_idx {
        let node = &state.nodes[idx];
        let is_selected = idx == state.selected_idx;

        let mut spans = Vec::new();

        // 1. Branch glyphs (prefix)
        spans.push(Span::styled(node.prefix.as_str(), Style::default().fg(Color::DarkGray)));

        // 2. Kind tag
        let kind_style = match node.kind.to_lowercase().as_str() {
            "pod" => Style::default().fg(Theme::CYAN).add_modifier(Modifier::BOLD),
            "deployment" | "replicaset" | "statefulset" | "daemonset" => Style::default().fg(Theme::ACCENT).add_modifier(Modifier::BOLD),
            "service" | "ingress" => Style::default().fg(Color::Rgb(100, 180, 255)).add_modifier(Modifier::BOLD),
            "configmap" | "secret" => Style::default().fg(Color::Yellow),
            "persistentvolumeclaim" | "persistentvolume" | "storageclass" => Style::default().fg(Color::Magenta),
            "node" => Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
            "container" => Style::default().fg(Color::Rgb(160, 160, 200)),
            _ => Style::default().fg(Theme::FG).add_modifier(Modifier::BOLD),
        };
        spans.push(Span::styled(format!("{}/", node.kind), kind_style));

        // 3. Name
        let name_style = if is_selected {
            Style::default().fg(Theme::SEL_FG).add_modifier(Modifier::BOLD)
        } else if node.relationship == LineageRelation::Target {
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Theme::FG)
        };
        spans.push(Span::styled(node.name.as_str(), name_style));

        // 4. Relationship badge
        let rel_style = match node.relationship {
            LineageRelation::Target => Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            LineageRelation::Owner => Style::default().fg(Color::Rgb(255, 140, 60)),
            LineageRelation::Child => Style::default().fg(Color::Rgb(100, 200, 255)),
            LineageRelation::Service | LineageRelation::Ingress => Style::default().fg(Color::Rgb(120, 180, 255)),
            LineageRelation::Config | LineageRelation::Secret => Style::default().fg(Color::Yellow),
            LineageRelation::Storage => Style::default().fg(Color::Magenta),
            LineageRelation::Node => Style::default().fg(Color::Green),
        };
        spans.push(Span::raw(" "));
        spans.push(Span::styled(node.relationship.badge(), rel_style));

        // 5. Status badge
        if let Some(status) = &node.status {
            let st_style = match status.to_lowercase().as_str() {
                s if s.contains("running") || s.contains("ready") || s.contains("bound") || s.contains("completed") || s.contains("active") => {
                    Theme::status_ok()
                }
                s if s.contains("crashloop") || s.contains("error") || s.contains("failed") || s.contains("unreachable") => {
                    Theme::status_error()
                }
                _ => Theme::status_warn(),
            };
            spans.push(Span::raw(" "));
            spans.push(Span::styled(format!("[● {}]", status), st_style));
        }

        // 6. Details
        if let Some(details) = &node.details {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(format!("({})", details), Style::default().fg(Theme::DIM)));
        }

        let row_style = if is_selected {
            Theme::selected_row()
        } else {
            Style::default()
        };

        lines.push(Line::from(spans).style(row_style));
    }

    f.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tree_flattening_and_target_selection() {
        let mut root = LineageNode::new("Deployment", "auth-api", Some("prod".into()), LineageRelation::Owner);
        let mut rs = LineageNode::new("ReplicaSet", "auth-api-987", Some("prod".into()), LineageRelation::Owner);
        let pod = LineageNode::new("Pod", "auth-api-987-abc", Some("prod".into()), LineageRelation::Target);

        rs.children.push(pod);
        root.children.push(rs);

        let mut state = TreeViewState::new("Pod".into(), "auth-api-987-abc".into(), Some("prod".into()));
        state.set_tree(root);

        assert_eq!(state.nodes.len(), 3);
        assert_eq!(state.nodes[0].kind, "Deployment");
        assert_eq!(state.nodes[1].kind, "ReplicaSet");
        assert_eq!(state.nodes[2].kind, "Pod");
        // Target Pod should be selected by default!
        assert_eq!(state.selected_idx, 2);
        assert_eq!(state.selected_node().unwrap().name, "auth-api-987-abc");

        // Navigate up
        state.select_prev();
        assert_eq!(state.selected_idx, 1);
        assert_eq!(state.selected_node().unwrap().name, "auth-api-987");
    }
}
