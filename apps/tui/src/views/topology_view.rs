use std::collections::{HashMap, HashSet};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use srelens_kube::topology::{
    EdgeKind, Health, Lane, Provenance, TopologyEdge, TopologyGraphOut, TopologyNode,
};

use crate::theme::Theme;

#[derive(Debug, Clone)]
pub struct LaneColumn {
    pub lane: Lane,
    pub title: &'static str,
    pub node_indices: Vec<usize>,
}

#[derive(Debug, Clone)]
pub struct TopologyViewState {
    pub namespaces: Vec<String>,
    pub graph: Option<TopologyGraphOut>,
    pub lanes: Vec<LaneColumn>,
    pub selected_lane: usize,
    pub selected_in_lane: usize,
    pub lane_scroll: Vec<usize>,
    pub is_loading: bool,
    pub error: Option<String>,
}

impl TopologyViewState {
    pub fn new(namespaces: Vec<String>) -> Self {
        Self {
            namespaces,
            graph: None,
            lanes: Vec::new(),
            selected_lane: 0,
            selected_in_lane: 0,
            lane_scroll: Vec::new(),
            is_loading: true,
            error: None,
        }
    }

    pub fn set_graph(&mut self, graph: TopologyGraphOut) {
        let mut lane_map: HashMap<Lane, Vec<usize>> = HashMap::new();
        for (idx, node) in graph.nodes.iter().enumerate() {
            lane_map.entry(node.lane).or_default().push(idx);
        }

        // We organize lanes into logical flow order:
        // 1. Routes (Ingresses)
        // 2. Services
        // 3. Workloads (Deployments, StatefulSets, DaemonSets)
        // 4. Revisions (ReplicaSets - only if present)
        // 5. External Dependencies (Databases, Third-party APIs)
        let mut lanes = Vec::new();
        lanes.push(LaneColumn {
            lane: Lane::Route,
            title: "ROUTES (INGRESS)",
            node_indices: lane_map.remove(&Lane::Route).unwrap_or_default(),
        });
        lanes.push(LaneColumn {
            lane: Lane::Service,
            title: "SERVICES",
            node_indices: lane_map.remove(&Lane::Service).unwrap_or_default(),
        });
        lanes.push(LaneColumn {
            lane: Lane::Workload,
            title: "WORKLOADS",
            node_indices: lane_map.remove(&Lane::Workload).unwrap_or_default(),
        });

        if let Some(replicas) = lane_map.remove(&Lane::ReplicaSet) {
            if !replicas.is_empty() {
                lanes.push(LaneColumn {
                    lane: Lane::ReplicaSet,
                    title: "REVISIONS",
                    node_indices: replicas,
                });
            }
        }

        lanes.push(LaneColumn {
            lane: Lane::External,
            title: "DEPENDENCIES / EGRESS",
            node_indices: lane_map.remove(&Lane::External).unwrap_or_default(),
        });

        self.lane_scroll = vec![0; lanes.len()];
        self.lanes = lanes;
        self.graph = Some(graph);
        self.is_loading = false;
        self.error = None;

        // Auto-select the first non-empty lane (preferring Workloads or Services)
        if let Some(pos) = self.lanes.iter().position(|l| l.lane == Lane::Workload && !l.node_indices.is_empty()) {
            self.selected_lane = pos;
            self.selected_in_lane = 0;
        } else if let Some(pos) = self.lanes.iter().position(|l| !l.node_indices.is_empty()) {
            self.selected_lane = pos;
            self.selected_in_lane = 0;
        } else {
            self.selected_lane = 0;
            self.selected_in_lane = 0;
        }
    }

    pub fn set_error(&mut self, err: String) {
        self.is_loading = false;
        self.error = Some(err);
    }

    pub fn selected_node(&self) -> Option<&TopologyNode> {
        let graph = self.graph.as_ref()?;
        let lane = self.lanes.get(self.selected_lane)?;
        let &node_idx = lane.node_indices.get(self.selected_in_lane)?;
        graph.nodes.get(node_idx)
    }

    pub fn select_next_lane(&mut self) {
        if self.lanes.is_empty() {
            return;
        }
        let start = self.selected_lane;
        for i in 1..=self.lanes.len() {
            let next = (start + i) % self.lanes.len();
            if !self.lanes[next].node_indices.is_empty() {
                self.selected_lane = next;
                let max_in_lane = self.lanes[next].node_indices.len().saturating_sub(1);
                self.selected_in_lane = self.selected_in_lane.min(max_in_lane);
                return;
            }
        }
    }

    pub fn select_prev_lane(&mut self) {
        if self.lanes.is_empty() {
            return;
        }
        let start = self.selected_lane;
        for i in 1..=self.lanes.len() {
            let prev = (start + self.lanes.len() - (i % self.lanes.len())) % self.lanes.len();
            if !self.lanes[prev].node_indices.is_empty() {
                self.selected_lane = prev;
                let max_in_lane = self.lanes[prev].node_indices.len().saturating_sub(1);
                self.selected_in_lane = self.selected_in_lane.min(max_in_lane);
                return;
            }
        }
    }

    pub fn select_next_node(&mut self) {
        if let Some(lane) = self.lanes.get(self.selected_lane) {
            if !lane.node_indices.is_empty() && self.selected_in_lane + 1 < lane.node_indices.len() {
                self.selected_in_lane += 1;
            }
        }
    }

    pub fn select_prev_node(&mut self) {
        self.selected_in_lane = self.selected_in_lane.saturating_sub(1);
    }

    pub fn select_first_node(&mut self) {
        self.selected_in_lane = 0;
    }

    pub fn select_last_node(&mut self) {
        if let Some(lane) = self.lanes.get(self.selected_lane) {
            if !lane.node_indices.is_empty() {
                self.selected_in_lane = lane.node_indices.len() - 1;
            }
        }
    }

    /// Trace the full connected path for the currently selected node:
    /// returns set of all node IDs that are upstream ancestors or downstream targets.
    pub fn connected_path_node_ids(&self) -> (HashSet<String>, Vec<&TopologyEdge>, Vec<&TopologyEdge>) {
        let mut connected = HashSet::new();
        let mut incoming_edges = Vec::new();
        let mut outgoing_edges = Vec::new();

        let Some(sel_node) = self.selected_node() else {
            return (connected, incoming_edges, outgoing_edges);
        };
        let target_id = &sel_node.id;
        connected.insert(target_id.clone());

        let Some(graph) = &self.graph else {
            return (connected, incoming_edges, outgoing_edges);
        };

        // 1. Direct incoming edges to target
        for edge in &graph.edges {
            if edge.to == *target_id {
                incoming_edges.push(edge);
                connected.insert(edge.from.clone());
            }
            if edge.from == *target_id {
                outgoing_edges.push(edge);
                connected.insert(edge.to.clone());
            }
        }

        // 2. Upstream transitive search (e.g. Ingress -> Service -> Workload)
        let mut frontier: Vec<String> = incoming_edges.iter().map(|e| e.from.clone()).collect();
        while let Some(current) = frontier.pop() {
            for edge in &graph.edges {
                if edge.to == current && !connected.contains(&edge.from) {
                    connected.insert(edge.from.clone());
                    frontier.push(edge.from.clone());
                }
            }
        }

        // 3. Downstream transitive search (e.g. Workload -> Dependency)
        let mut down_frontier: Vec<String> = outgoing_edges.iter().map(|e| e.to.clone()).collect();
        while let Some(current) = down_frontier.pop() {
            for edge in &graph.edges {
                if edge.from == current && !connected.contains(&edge.to) {
                    connected.insert(edge.to.clone());
                    down_frontier.push(edge.to.clone());
                }
            }
        }

        (connected, incoming_edges, outgoing_edges)
    }
}

pub fn render_topology_view(f: &mut Frame, area: Rect, state: &TopologyViewState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::border()))
        .title(Span::styled(
            format!(
                " Workload & Traffic Topology Flow [{}] ",
                if state.namespaces.is_empty() {
                    "all namespaces".to_string()
                } else {
                    state.namespaces.join(", ")
                }
            ),
            Theme::title(),
        ));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if state.is_loading {
        let msg = Paragraph::new(Line::from(vec![
            Span::styled("⚡ Building topology graph ", Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD)),
            Span::styled("resolving ingresses, services, selectors & dependencies...", Style::default().fg(Theme::dim())),
        ]));
        f.render_widget(msg, inner);
        return;
    }

    if let Some(err) = &state.error {
        let msg = Paragraph::new(vec![
            Line::from(Span::styled("✖ Failed to build topology graph:", Theme::status_error().add_modifier(Modifier::BOLD))),
            Line::from(Span::styled(err.clone(), Style::default().fg(Theme::fg()))),
            Line::from(""),
            Line::from(Span::styled("Press <r> to retry or <Esc> to return.", Style::default().fg(Theme::dim()))),
        ]);
        f.render_widget(msg, inner);
        return;
    }

    let Some(graph) = &state.graph else {
        let msg = Paragraph::new("No topology graph available.");
        f.render_widget(msg, inner);
        return;
    };

    if graph.nodes.is_empty() {
        let msg = Paragraph::new(Line::from(vec![
            Span::styled("No workloads, services or ingresses found in this namespace scope.", Style::default().fg(Theme::dim())),
        ]));
        f.render_widget(msg, inner);
        return;
    }

    // Split inner area into:
    // 1. Lanes columns (main flow)
    // 2. Bottom detail & path inspector (6-8 rows)
    let has_detail = inner.height >= 14;
    let chunks = if has_detail {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(8), Constraint::Length(7)])
            .split(inner)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(4)])
            .split(inner)
    };

    let (connected_ids, in_edges, out_edges) = state.connected_path_node_ids();
    let sel_node_opt = state.selected_node();

    // Render lane columns
    let lane_count = state.lanes.len();
    if lane_count > 0 {
        let lane_constraints: Vec<Constraint> = vec![Constraint::Ratio(1, lane_count as u32); lane_count];
        let lane_areas = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(lane_constraints)
            .split(chunks[0]);

        for (lane_idx, lane_col) in state.lanes.iter().enumerate() {
            let lane_area = lane_areas[lane_idx];
            let is_lane_focused = lane_idx == state.selected_lane;

            let lane_border_color = if is_lane_focused {
                Theme::accent()
            } else {
                Theme::border()
            };

            let lane_title = format!(" {} [{}] ", lane_col.title, lane_col.node_indices.len());
            let lane_block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(lane_border_color))
                .title(Span::styled(
                    lane_title,
                    if is_lane_focused {
                        Style::default().fg(Theme::accent()).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Theme::dim())
                    },
                ));

            let lane_inner = lane_block.inner(lane_area);
            f.render_widget(lane_block, lane_area);

            if lane_col.node_indices.is_empty() {
                let empty_msg = Paragraph::new(Span::styled(" (none)", Style::default().fg(Theme::dim())));
                f.render_widget(empty_msg, lane_inner);
                continue;
            }

            // Each node card takes 3 rows:
            // ┌ Kind: Name ──┐
            // │ ● 3/3 ready  │
            // └──────────────┘
            let card_height = 3usize;
            let visible_cards = (lane_inner.height as usize) / card_height;
            let sel_in_lane = if is_lane_focused { state.selected_in_lane } else { 0 };

            // Compute windowed scroll for this lane
            let scroll_offset = if sel_in_lane >= visible_cards {
                sel_in_lane.saturating_sub(visible_cards.saturating_sub(1))
            } else {
                0
            };

            for (slot_idx, &node_idx) in lane_col.node_indices.iter().skip(scroll_offset).take(visible_cards).enumerate() {
                let node = &graph.nodes[node_idx];
                let is_node_selected = is_lane_focused && (scroll_offset + slot_idx == state.selected_in_lane);
                let is_in_path = connected_ids.contains(&node.id);

                let card_y = lane_inner.y + (slot_idx * card_height) as u16;
                let card_area = Rect {
                    x: lane_inner.x,
                    y: card_y,
                    width: lane_inner.width,
                    height: 3,
                };

                let (health_dot, health_style) = match node.health {
                    Health::Ok => ("●", Theme::status_ok()),
                    Health::Degraded => ("▲", Theme::status_warn()),
                    Health::Failing => ("✖", Theme::status_error()),
                    Health::Unknown => ("○", Theme::status_dim()),
                };

                let card_border_style = if is_node_selected {
                    Style::default().fg(Theme::sel_fg()).add_modifier(Modifier::BOLD)
                } else if is_in_path {
                    Style::default().fg(Theme::cyan())
                } else {
                    Style::default().fg(Theme::dim())
                };

                let card_bg = if is_node_selected {
                    Theme::selected_row().bg.unwrap_or(Color::Reset)
                } else if is_in_path {
                    if Theme::active_palette().is_light {
                        Color::Rgb(235, 243, 250)
                    } else {
                        Color::Rgb(28, 35, 45)
                    }
                } else {
                    Color::Reset
                };

                let card_title = format!(" {} ", node.kind);
                let card_block = Block::default()
                    .borders(Borders::ALL)
                    .border_style(card_border_style)
                    .style(Style::default().bg(card_bg))
                    .title(Span::styled(card_title, if is_node_selected {
                        Style::default().fg(Theme::sel_fg()).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Theme::dim())
                    }));

                let card_inner = card_block.inner(card_area);
                f.render_widget(card_block, card_area);

                // Line 1: Name and optional path marker
                let path_marker = if is_node_selected {
                    "▶ "
                } else if is_in_path {
                    "⚡ "
                } else {
                    ""
                };

                let name_line = Line::from(vec![
                    Span::styled(path_marker, if is_node_selected { Style::default().fg(Theme::accent()) } else { Style::default().fg(Theme::cyan()) }),
                    Span::styled(
                        crate::views::sanitize_span_text(&node.name),
                        if is_node_selected {
                            Style::default().fg(Theme::sel_fg()).add_modifier(Modifier::BOLD)
                        } else if is_in_path {
                            Style::default().fg(Theme::fg()).add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Theme::fg())
                        },
                    ),
                ]);

                // Line 2: Health dot + detail (e.g. 3/3 ready, :80 -> :8080)
                let detail_text = if !node.detail.is_empty() {
                    format!(" {}", node.detail)
                } else if let (Some(r), Some(d)) = (node.ready, node.desired) {
                    format!(" {}/{} ready", r, d)
                } else {
                    String::new()
                };

                let status_line = Line::from(vec![
                    Span::styled(format!("{} ", health_dot), health_style),
                    Span::styled(crate::views::sanitize_span_text(&detail_text), Style::default().fg(Theme::dim())),
                ]);

                let card_content = Paragraph::new(vec![name_line, status_line]);
                f.render_widget(card_content, card_inner);
            }
        }
    }

    // Render Bottom Detail & Path Inspector
    if has_detail && chunks.len() > 1 {
        let inspector_area = chunks[1];
        let inspector_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Theme::border_focus()))
            .title(Span::styled(" Flow & Dependency Inspector ", Theme::title()));

        let inspector_inner = inspector_block.inner(inspector_area);
        f.render_widget(inspector_block, inspector_area);

        if let Some(node) = sel_node_opt {
            let (health_label, health_style) = match node.health {
                Health::Ok => ("Ok", Theme::status_ok()),
                Health::Degraded => ("Degraded", Theme::status_warn()),
                Health::Failing => ("Failing", Theme::status_error()),
                Health::Unknown => ("Unknown", Theme::status_dim()),
            };

            let ready_str = if let (Some(r), Some(d)) = (node.ready, node.desired) {
                format!("  Replicas: {}/{} ready", r, d)
            } else if !node.detail.is_empty() {
                format!("  Config: {}", node.detail)
            } else {
                String::new()
            };

            // Format line 1: Target Identity & Health
            let l1 = Line::from(vec![
                Span::styled(" Selected: ", Style::default().fg(Theme::dim())),
                Span::styled(format!("[{}] ", node.kind), Style::default().fg(Theme::accent()).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{} ", node.name), Style::default().fg(Theme::sel_fg()).add_modifier(Modifier::BOLD)),
                Span::styled(format!("({})", node.namespace), Style::default().fg(Theme::dim())),
                Span::styled("   Health: ", Style::default().fg(Theme::dim())),
                Span::styled(format!("● {}", health_label), health_style.add_modifier(Modifier::BOLD)),
                Span::styled(ready_str, Style::default().fg(Theme::fg())),
            ]);

            // Format line 2: Incoming Routes / Sources
            let in_summary = if in_edges.is_empty() {
                "none (direct entry point)".to_string()
            } else {
                in_edges
                    .iter()
                    .map(|e| {
                        let name = e.from.split('/').nth(2).unwrap_or(&e.from);
                        let prov = match e.provenance {
                            Provenance::Topology => "API",
                            Provenance::Declared => "Config",
                            Provenance::Allowed => "Policy",
                            Provenance::Observed => "Observed",
                        };
                        format!("{} ──► [{} ({})]", name, node.name, prov)
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            };

            let l2 = Line::from(vec![
                Span::styled(" Incoming: ", Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD)),
                Span::styled(crate::views::sanitize_span_text(&in_summary), Style::default().fg(Theme::fg())),
            ]);

            // Format line 3: Outgoing Dependencies / Backings
            let out_summary = if out_edges.is_empty() {
                "none (leaf workload)".to_string()
            } else {
                out_edges
                    .iter()
                    .map(|e| {
                        let target = e.to.split('/').nth(2).unwrap_or(&e.to);
                        let prov = match e.provenance {
                            Provenance::Topology => "API",
                            Provenance::Declared => "Config",
                            Provenance::Allowed => "Policy",
                            Provenance::Observed => "Observed",
                        };
                        let detail = if !e.detail.is_empty() {
                            format!(" ({})", e.detail)
                        } else {
                            String::new()
                        };
                        format!("{} ──► {}{} [{}]", node.name, target, detail, prov)
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            };

            let l3 = Line::from(vec![
                Span::styled(" Outgoing: ", Style::default().fg(Theme::green()).add_modifier(Modifier::BOLD)),
                Span::styled(crate::views::sanitize_span_text(&out_summary), Style::default().fg(Theme::fg())),
            ]);

            let inspector_content = Paragraph::new(vec![l1, l2, l3]);
            f.render_widget(inspector_content, inspector_inner);
        } else {
            let msg = Paragraph::new(Span::styled("Select a node above to inspect its traffic path and dependencies.", Style::default().fg(Theme::dim())));
            f.render_widget(msg, inspector_inner);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_graph() -> TopologyGraphOut {
        let nodes = vec![
            TopologyNode {
                id: "Ingress/prod/web-ingress".to_string(),
                kind: "Ingress".to_string(),
                name: "web-ingress".to_string(),
                namespace: "prod".to_string(),
                lane: Lane::Route,
                detail: ":80, :443".to_string(),
                ready: None,
                desired: None,
                health: Health::Ok,
            },
            TopologyNode {
                id: "Service/prod/checkout-svc".to_string(),
                kind: "Service".to_string(),
                name: "checkout-svc".to_string(),
                namespace: "prod".to_string(),
                lane: Lane::Service,
                detail: ":80 -> :8080".to_string(),
                ready: None,
                desired: None,
                health: Health::Ok,
            },
            TopologyNode {
                id: "Deployment/prod/checkout-api".to_string(),
                kind: "Deployment".to_string(),
                name: "checkout-api".to_string(),
                namespace: "prod".to_string(),
                lane: Lane::Workload,
                detail: "3/3 ready".to_string(),
                ready: Some(3),
                desired: Some(3),
                health: Health::Ok,
            },
            TopologyNode {
                id: "External//postgres-primary:5432".to_string(),
                kind: "External".to_string(),
                name: "postgres-primary:5432".to_string(),
                namespace: String::new(),
                lane: Lane::External,
                detail: "postgres-primary:5432".to_string(),
                ready: None,
                desired: None,
                health: Health::Ok,
            },
        ];

        let edges = vec![
            TopologyEdge {
                from: "Ingress/prod/web-ingress".to_string(),
                to: "Service/prod/checkout-svc".to_string(),
                kind: EdgeKind::Routes,
                provenance: Provenance::Topology,
                detail: String::new(),
                weight: None,
                unit: None,
                health: Health::Ok,
            },
            TopologyEdge {
                from: "Service/prod/checkout-svc".to_string(),
                to: "Deployment/prod/checkout-api".to_string(),
                kind: EdgeKind::Routes,
                provenance: Provenance::Topology,
                detail: String::new(),
                weight: None,
                unit: None,
                health: Health::Ok,
            },
            TopologyEdge {
                from: "Deployment/prod/checkout-api".to_string(),
                to: "External//postgres-primary:5432".to_string(),
                kind: EdgeKind::Calls,
                provenance: Provenance::Declared,
                detail: String::new(),
                weight: None,
                unit: None,
                health: Health::Ok,
            },
        ];

        TopologyGraphOut {
            nodes,
            edges,
            probe: None,
        }
    }

    #[test]
    fn test_topology_state_lanes_and_path_tracing() {
        let mut state = TopologyViewState::new(vec!["prod".to_string()]);
        state.set_graph(sample_graph());

        // Verify 4 lanes categorized correctly
        assert_eq!(state.lanes.len(), 4);
        assert_eq!(state.lanes[0].lane, Lane::Route);
        assert_eq!(state.lanes[1].lane, Lane::Service);
        assert_eq!(state.lanes[2].lane, Lane::Workload);
        assert_eq!(state.lanes[3].lane, Lane::External);

        // Auto-selects Workloads lane
        assert_eq!(state.selected_lane, 2);
        let sel = state.selected_node().expect("selected node");
        assert_eq!(sel.name, "checkout-api");

        // Verify connected path tracing:
        // checkout-api connects upstream to checkout-svc and web-ingress, and downstream to postgres-primary
        let (connected_ids, in_edges, out_edges) = state.connected_path_node_ids();
        assert_eq!(connected_ids.len(), 4);
        assert!(connected_ids.contains("Ingress/prod/web-ingress"));
        assert!(connected_ids.contains("Service/prod/checkout-svc"));
        assert!(connected_ids.contains("Deployment/prod/checkout-api"));
        assert!(connected_ids.contains("External//postgres-primary:5432"));

        assert_eq!(in_edges.len(), 1);
        assert_eq!(in_edges[0].from, "Service/prod/checkout-svc");

        assert_eq!(out_edges.len(), 1);
        assert_eq!(out_edges[0].to, "External//postgres-primary:5432");

        // Test navigation
        state.select_next_lane();
        assert_eq!(state.selected_lane, 3); // Moves to External
        assert_eq!(state.selected_node().unwrap().name, "postgres-primary:5432");

        state.select_prev_lane();
        assert_eq!(state.selected_lane, 2); // Moves back to Workloads
    }
}
