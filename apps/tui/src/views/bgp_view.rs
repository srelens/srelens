use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;
use std::cell::Cell;

use crate::theme::Theme;
use srelens_kube::bgp::{
    BgpAdvertisedService, BgpClusterSummary, BgpEngineType, BgpIpPool, BgpNeighbor, BgpSessionState,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BgpTab {
    Peers,
    Services,
    IpPools,
}

impl BgpTab {
    pub fn next(&self) -> Self {
        match self {
            Self::Peers => Self::Services,
            Self::Services => Self::IpPools,
            Self::IpPools => Self::Peers,
        }
    }

    pub fn prev(&self) -> Self {
        match self {
            Self::Peers => Self::IpPools,
            Self::Services => Self::Peers,
            Self::IpPools => Self::Services,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BgpViewState {
    pub summary: Option<BgpClusterSummary>,
    pub active_tab: BgpTab,
    pub selected_peer_idx: usize,
    pub selected_svc_idx: usize,
    pub selected_pool_idx: usize,
    pub is_loading: bool,
    pub error: Option<String>,
    pub search_query: String,
    pub is_searching: bool,
    pub last_peers_start_idx: Cell<usize>,
    pub last_svc_start_idx: Cell<usize>,
    pub last_pool_start_idx: Cell<usize>,
}

impl Default for BgpViewState {
    fn default() -> Self {
        Self::new()
    }
}

impl BgpViewState {
    pub fn new() -> Self {
        Self {
            summary: None,
            active_tab: BgpTab::Peers,
            selected_peer_idx: 0,
            selected_svc_idx: 0,
            selected_pool_idx: 0,
            is_loading: true,
            error: None,
            search_query: String::new(),
            is_searching: false,
            last_peers_start_idx: Cell::new(0),
            last_svc_start_idx: Cell::new(0),
            last_pool_start_idx: Cell::new(0),
        }
    }

    pub fn set_summary(&mut self, summary: BgpClusterSummary) {
        self.is_loading = false;
        self.error = None;
        self.summary = Some(summary);
        self.clamp_selection();
    }

    pub fn set_error(&mut self, err: String) {
        self.is_loading = false;
        self.error = Some(err);
    }

    /// Why the view cannot speak for the cluster: the fetch itself failed, or
    /// it came back with a lookup refused or timed out. A cluster that
    /// answered and runs no BGP control plane reports neither — "no engine"
    /// and "we were not allowed to look" are different facts.
    pub fn discovery_error(&self) -> Option<&str> {
        self.error.as_deref().or_else(|| {
            self.summary
                .as_ref()
                .and_then(|s| s.error.as_deref())
                .filter(|e| !e.is_empty())
        })
    }

    pub fn filtered_peers(&self) -> Vec<&BgpNeighbor> {
        let peers = self.summary.as_ref().map(|s| &s.peers[..]).unwrap_or(&[]);
        if self.search_query.trim().is_empty() {
            return peers.iter().collect();
        }
        let q = self.search_query.to_lowercase();
        peers
            .iter()
            .filter(|n| {
                n.node_name.to_lowercase().contains(&q)
                    || n.peer_address.to_lowercase().contains(&q)
                    || n.peer_asn.to_string().contains(&q)
                    || n.local_asn.to_string().contains(&q)
                    || n.policy_name.to_lowercase().contains(&q)
                    || n.session_state.to_string().to_lowercase().contains(&q)
            })
            .collect()
    }

    pub fn filtered_services(&self) -> Vec<&BgpAdvertisedService> {
        let svcs = self
            .summary
            .as_ref()
            .map(|s| &s.advertised_services[..])
            .unwrap_or(&[]);
        if self.search_query.trim().is_empty() {
            return svcs.iter().collect();
        }
        let q = self.search_query.to_lowercase();
        svcs.iter()
            .filter(|s| {
                s.service_name.to_lowercase().contains(&q)
                    || s.namespace.to_lowercase().contains(&q)
                    || s.load_balancer_ip.to_lowercase().contains(&q)
                    || s.ip_pool
                        .as_deref()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&q)
            })
            .collect()
    }

    pub fn filtered_pools(&self) -> Vec<&BgpIpPool> {
        let pools = self
            .summary
            .as_ref()
            .map(|s| &s.ip_pools[..])
            .unwrap_or(&[]);
        if self.search_query.trim().is_empty() {
            return pools.iter().collect();
        }
        let q = self.search_query.to_lowercase();
        pools
            .iter()
            .filter(|p| {
                p.name.to_lowercase().contains(&q)
                    || p.cidrs.iter().any(|c| c.to_lowercase().contains(&q))
                    || p.service_selector.to_lowercase().contains(&q)
            })
            .collect()
    }

    pub fn next_tab(&mut self) {
        self.active_tab = self.active_tab.next();
    }

    pub fn prev_tab(&mut self) {
        self.active_tab = self.active_tab.prev();
    }

    pub fn select_next(&mut self) {
        match self.active_tab {
            BgpTab::Peers => {
                let len = self.filtered_peers().len();
                if len > 0 && self.selected_peer_idx + 1 < len {
                    self.selected_peer_idx += 1;
                }
            }
            BgpTab::Services => {
                let len = self.filtered_services().len();
                if len > 0 && self.selected_svc_idx + 1 < len {
                    self.selected_svc_idx += 1;
                }
            }
            BgpTab::IpPools => {
                let len = self.filtered_pools().len();
                if len > 0 && self.selected_pool_idx + 1 < len {
                    self.selected_pool_idx += 1;
                }
            }
        }
    }

    pub fn select_prev(&mut self) {
        match self.active_tab {
            BgpTab::Peers => {
                if self.selected_peer_idx > 0 {
                    self.selected_peer_idx -= 1;
                }
            }
            BgpTab::Services => {
                if self.selected_svc_idx > 0 {
                    self.selected_svc_idx -= 1;
                }
            }
            BgpTab::IpPools => {
                if self.selected_pool_idx > 0 {
                    self.selected_pool_idx -= 1;
                }
            }
        }
    }

    pub fn select_first(&mut self) {
        match self.active_tab {
            BgpTab::Peers => self.selected_peer_idx = 0,
            BgpTab::Services => self.selected_svc_idx = 0,
            BgpTab::IpPools => self.selected_pool_idx = 0,
        }
    }

    pub fn select_last(&mut self) {
        match self.active_tab {
            BgpTab::Peers => {
                let len = self.filtered_peers().len();
                self.selected_peer_idx = len.saturating_sub(1);
            }
            BgpTab::Services => {
                let len = self.filtered_services().len();
                self.selected_svc_idx = len.saturating_sub(1);
            }
            BgpTab::IpPools => {
                let len = self.filtered_pools().len();
                self.selected_pool_idx = len.saturating_sub(1);
            }
        }
    }

    pub fn clamp_selection(&mut self) {
        let peers_len = self.filtered_peers().len();
        if self.selected_peer_idx >= peers_len {
            self.selected_peer_idx = peers_len.saturating_sub(1);
        }
        let svcs_len = self.filtered_services().len();
        if self.selected_svc_idx >= svcs_len {
            self.selected_svc_idx = svcs_len.saturating_sub(1);
        }
        let pools_len = self.filtered_pools().len();
        if self.selected_pool_idx >= pools_len {
            self.selected_pool_idx = pools_len.saturating_sub(1);
        }
    }

    pub fn selected_peer(&self) -> Option<&BgpNeighbor> {
        let list = self.filtered_peers();
        list.get(self.selected_peer_idx).copied()
    }

    pub fn selected_service(&self) -> Option<&BgpAdvertisedService> {
        let list = self.filtered_services();
        list.get(self.selected_svc_idx).copied()
    }

    pub fn selected_pool(&self) -> Option<&BgpIpPool> {
        let list = self.filtered_pools();
        list.get(self.selected_pool_idx).copied()
    }
}

pub fn render_bgp_view(f: &mut Frame, area: Rect, state: &BgpViewState) {
    if area.width < 10 || area.height < 5 {
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4), // Top BGP summary stats card
            Constraint::Min(8),    // Main Tab & Data table
            Constraint::Length(7), // Bottom Peer details / Timers inspector
        ])
        .split(area);

    render_summary_header(f, chunks[0], state);
    render_tab_content(f, chunks[1], state);
    render_detail_footer(f, chunks[2], state);
}

fn render_summary_header(f: &mut Frame, area: Rect, state: &BgpViewState) {
    let summary = state.summary.as_ref();
    let discovery_error = state.discovery_error();
    // "No BGP Engine Detected" is a claim about the cluster. Only make it
    // when the cluster actually answered.
    let engine_str = match (summary, discovery_error) {
        (Some(s), Some(_)) if s.engine == BgpEngineType::None => "Discovery Failed".to_string(),
        (None, Some(_)) => "Discovery Failed".to_string(),
        (Some(s), _) => s.engine.to_string(),
        (None, None) => "Scanning...".to_string(),
    };
    let engine_color = if engine_str == "Discovery Failed" {
        Theme::red()
    } else {
        match summary.map(|s| &s.engine) {
            Some(BgpEngineType::CiliumV2 | BgpEngineType::CiliumV2Alpha1) => Theme::cyan(),
            Some(BgpEngineType::MetalLB | BgpEngineType::Calico) => Theme::accent(),
            _ => Theme::dim(),
        }
    };

    let total_nodes = summary.map(|s| s.total_nodes).unwrap_or(0);
    let bgp_nodes = summary.map(|s| s.bgp_nodes).unwrap_or(0);
    let total_peers = summary.map(|s| s.total_peers).unwrap_or(0);
    let established = summary.map(|s| s.established_peers).unwrap_or(0);
    let degraded = summary.map(|s| s.degraded_peers).unwrap_or(0);
    let vips_count = summary.map(|s| s.advertised_services.len()).unwrap_or(0);
    let pools_count = summary.map(|s| s.ip_pools.len()).unwrap_or(0);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::border()))
        .title(Span::styled(
            " 🌐 BGP CONTROL PLANE & PEERING TOPOLOGY ",
            Style::default()
                .fg(Theme::cyan())
                .add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let l1 = Line::from(vec![
        Span::styled(" Engine: ", Theme::header_label()),
        Span::styled(
            format!(" {} ", engine_str),
            Style::default()
                .fg(engine_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" │ Nodes: ", Theme::header_label()),
        Span::styled(
            format!("{}/{} active BGP nodes", bgp_nodes, total_nodes),
            Style::default().fg(Theme::fg()),
        ),
        Span::styled(" │ Peering Sessions: ", Theme::header_label()),
        Span::styled(
            format!("{} Total", total_peers),
            Style::default()
                .fg(Theme::fg())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  ● {} Established", established),
            Style::default()
                .fg(Theme::green())
                .add_modifier(Modifier::BOLD),
        ),
        if degraded > 0 {
            Span::styled(
                format!("  ▲ {} Degraded/Down", degraded),
                Style::default()
                    .fg(Theme::red())
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled("  ▲ 0 Degraded", Style::default().fg(Theme::dim()))
        },
    ]);

    let l2 = Line::from(vec![
        Span::styled(" Advertised VIPs: ", Theme::header_label()),
        Span::styled(
            format!("{} LoadBalancer Services", vips_count),
            Style::default()
                .fg(Theme::yellow())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" │ IP Pools: ", Theme::header_label()),
        Span::styled(
            format!("{} CIDR Pools", pools_count),
            Style::default().fg(Theme::accent()),
        ),
        // An engine was found but a lookup was refused or timed out: say so,
        // rather than presenting a partial read as the whole cluster.
        match discovery_error {
            Some(err) => Span::styled(
                format!(" │ ▲ Discovery incomplete: {}", err),
                Style::default().fg(Theme::red()),
            ),
            None => Span::styled(" │ Hotkeys: ", Theme::header_label()),
        },
        match discovery_error {
            Some(_) => Span::raw(""),
            None => Span::styled(
                "<Tab/1-3> Switch Tab  </> Filter  <r> Refresh  <Esc> Back",
                Style::default().fg(Theme::dim()),
            ),
        },
    ]);

    let p = Paragraph::new(vec![l1, l2]);
    f.render_widget(p, inner);
}

fn render_tab_content(f: &mut Frame, area: Rect, state: &BgpViewState) {
    let t1_style = if state.active_tab == BgpTab::Peers {
        Style::default()
            .fg(Theme::cyan())
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
    } else {
        Style::default().fg(Theme::dim())
    };
    let t2_style = if state.active_tab == BgpTab::Services {
        Style::default()
            .fg(Theme::cyan())
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
    } else {
        Style::default().fg(Theme::dim())
    };
    let t3_style = if state.active_tab == BgpTab::IpPools {
        Style::default()
            .fg(Theme::cyan())
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
    } else {
        Style::default().fg(Theme::dim())
    };

    let p_count = state.filtered_peers().len();
    let s_count = state.filtered_services().len();
    let pool_count = state.filtered_pools().len();

    let tab_title = Line::from(vec![
        Span::styled(format!(" [1] Peers & Neighbors ({}) ", p_count), t1_style),
        Span::styled(" │ ", Style::default().fg(Theme::border())),
        Span::styled(format!(" [2] Advertised VIPs ({}) ", s_count), t2_style),
        Span::styled(" │ ", Style::default().fg(Theme::border())),
        Span::styled(format!(" [3] IP Pools ({}) ", pool_count), t3_style),
        if !state.search_query.is_empty() {
            Span::styled(
                format!(" [Filter: \"{}\"]", state.search_query),
                Style::default().fg(Theme::yellow()),
            )
        } else {
            Span::raw("")
        },
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::border()))
        .title(tab_title);

    let inner = block.inner(area);
    f.render_widget(block, area);

    if state.is_loading {
        let p = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "  ⚡ Querying BGP control plane & peering topology...",
                Style::default().fg(Theme::cyan()),
            )),
        ]);
        f.render_widget(p, inner);
        return;
    }

    // A refused or timed-out lookup is shown instead of the tab when it left
    // nothing to show, so a restricted RBAC never reads as an empty cluster.
    // With an engine found, the tab still holds what was read and the header
    // carries the warning.
    let nothing_found = state
        .summary
        .as_ref()
        .map_or(true, |s| s.engine == BgpEngineType::None);
    if let Some(err) = state.discovery_error().filter(|_| nothing_found) {
        let p = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                format!("  ✖ BGP Error: {}", err),
                Style::default().fg(Theme::red()),
            )),
            Line::from(Span::styled(
                "  Press 'r' to retry query.",
                Style::default().fg(Theme::dim()),
            )),
        ]);
        f.render_widget(p, inner);
        return;
    }

    match state.active_tab {
        BgpTab::Peers => render_peers_table(f, inner, state),
        BgpTab::Services => render_services_table(f, inner, state),
        BgpTab::IpPools => render_pools_table(f, inner, state),
    }
}

fn render_peers_table(f: &mut Frame, area: Rect, state: &BgpViewState) {
    let peers = state.filtered_peers();
    if peers.is_empty() {
        let p = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "  ⊘ No BGP peering sessions detected in cluster.",
                Style::default().fg(Theme::yellow()),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "  Deploy CiliumBGPPeeringPolicy, CiliumBGPNodeConfig, or MetalLB BGPPeer",
                Style::default().fg(Theme::dim()),
            )),
            Line::from(Span::styled(
                "  to establish BGP peering with upstream Top-of-Rack (ToR) switches.",
                Style::default().fg(Theme::dim()),
            )),
        ]);
        f.render_widget(p, area);
        return;
    }

    let visible_rows = area.height.saturating_sub(2) as usize;
    if visible_rows == 0 {
        return;
    }

    let selected = state.selected_peer_idx;
    let mut start_idx = state.last_peers_start_idx.get();
    if selected < start_idx {
        start_idx = selected;
    } else if selected >= start_idx + visible_rows {
        start_idx = selected + 1 - visible_rows;
    }
    state.last_peers_start_idx.set(start_idx);

    // Compute dynamic column widths from content
    let mut max_node = "NODE".len();
    let mut max_ip = "NEIGHBOR IP".len();
    let mut max_pasn = "PEER ASN".len();
    let mut max_lasn = "LOCAL ASN".len();
    let mut max_pol = "POLICY / CONFIG".len();
    let mut max_pod = "POD CIDR".len();
    let mut max_routes = "ROUTES (IN/OUT)".len();
    let mut max_uptime = "UPTIME".len();

    for n in &peers {
        max_node = max_node.max(n.node_name.len());
        max_ip = max_ip.max(n.peer_address.len());
        max_pasn = max_pasn.max(n.peer_asn.to_string().len());
        max_lasn = max_lasn.max(n.local_asn.to_string().len());
        max_pol = max_pol.max(n.policy_name.len());
        let pod_str = if n.export_pod_cidr {
            "Enabled"
        } else {
            "Disabled"
        };
        max_pod = max_pod.max(pod_str.len());
        let routes_str = if n.routes_received > 0 || n.routes_count > 0 {
            format!("{} in / {} out", n.routes_received, n.routes_count)
        } else {
            "0 / 0".to_string()
        };
        max_routes = max_routes.max(routes_str.len());
        let uptime_str = format_uptime_display(n.uptime_or_last_change.as_deref());
        max_uptime = max_uptime.max(uptime_str.len());
    }

    let mut col_node = max_node.max(12);
    let col_ip = max_ip.max(13);
    let col_pasn = max_pasn.max(9);
    let col_lasn = max_lasn.max(10);
    let col_state = 15; // "● Established"
    let mut col_pol = max_pol.max(16);
    let col_pod = max_pod.max(9);
    let col_routes = max_routes.max(15);
    let mut col_uptime = max_uptime.max(8);

    // Total fixed column spacing: 1 leading space + col_node + 1 space + col_ip + 1 space + ...
    let fixed_width = 1
        + col_ip
        + 1
        + col_pasn
        + 1
        + col_lasn
        + 1
        + col_state
        + 1
        + col_pod
        + 1
        + col_routes
        + 1
        + col_uptime;
    let avail_w = area.width as usize;

    if avail_w > fixed_width + col_node + 1 + col_pol {
        let surplus = avail_w - (fixed_width + col_node + 1 + col_pol);
        let node_extra = surplus * 50 / 100;
        let pol_extra = surplus * 40 / 100;
        let uptime_extra = surplus.saturating_sub(node_extra + pol_extra);
        col_node += node_extra;
        col_pol += pol_extra;
        col_uptime += uptime_extra;
    } else if avail_w < fixed_width + col_node + 1 + col_pol && avail_w > fixed_width + 10 {
        let flexible = avail_w.saturating_sub(fixed_width + 1);
        col_node = (flexible * 55 / 100).max(10);
        col_pol = (flexible.saturating_sub(col_node)).max(10);
    }

    let mut lines = Vec::new();
    lines.push(Line::from(vec![
        Span::styled(format!(" {:<col_node$} ", "NODE"), Theme::table_header()),
        Span::styled(
            format!("{:<col_ip$} ", "NEIGHBOR IP"),
            Theme::table_header(),
        ),
        Span::styled(format!("{:<col_pasn$} ", "PEER ASN"), Theme::table_header()),
        Span::styled(
            format!("{:<col_lasn$} ", "LOCAL ASN"),
            Theme::table_header(),
        ),
        Span::styled(
            format!("{:<col_state$} ", "SESSION STATE"),
            Theme::table_header(),
        ),
        Span::styled(
            format!("{:<col_pol$} ", "POLICY / CONFIG"),
            Theme::table_header(),
        ),
        Span::styled(format!("{:<col_pod$} ", "POD CIDR"), Theme::table_header()),
        Span::styled(
            format!("{:<col_routes$} ", "ROUTES (IN/OUT)"),
            Theme::table_header(),
        ),
        Span::styled(format!("{:<col_uptime$}", "UPTIME"), Theme::table_header()),
    ]));
    lines.push(Line::from(Span::styled(
        "─".repeat(area.width as usize),
        Style::default().fg(Theme::border()),
    )));

    let end_idx = (start_idx + visible_rows).min(peers.len());
    for i in start_idx..end_idx {
        let n = peers[i];
        let is_sel = i == selected;
        let row_style = if is_sel {
            Style::default()
                .bg(Theme::sel_bg())
                .fg(Theme::fg())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Theme::fg())
        };

        let state_badge = match n.session_state {
            BgpSessionState::Established => Span::styled(
                format!("{:<col_state$} ", "● Established"),
                Style::default()
                    .fg(Theme::green())
                    .add_modifier(Modifier::BOLD),
            ),
            BgpSessionState::Configured => Span::styled(
                format!("{:<col_state$} ", "● Configured"),
                Style::default().fg(Theme::cyan()),
            ),
            BgpSessionState::Active | BgpSessionState::Connect => Span::styled(
                format!("{:<col_state$} ", format!("▲ {}", n.session_state)),
                Style::default().fg(Theme::yellow()),
            ),
            BgpSessionState::Idle | BgpSessionState::Unknown => Span::styled(
                format!("{:<col_state$} ", format!("✖ {}", n.session_state)),
                Style::default().fg(Theme::red()),
            ),
            _ => Span::styled(
                format!("{:<col_state$} ", format!("○ {}", n.session_state)),
                Style::default().fg(Theme::accent()),
            ),
        };

        let pod_cidr_str = if n.export_pod_cidr {
            "Enabled"
        } else {
            "Disabled"
        };
        let routes_str = if n.routes_received > 0 || n.routes_count > 0 {
            format!("{} in / {} out", n.routes_received, n.routes_count)
        } else {
            "0 / 0".to_string()
        };
        let uptime_str = format_uptime_display(n.uptime_or_last_change.as_deref());
        let show_node_name = i == start_idx || (i > 0 && peers[i - 1].node_name != n.node_name);
        let node_text = if show_node_name {
            truncate_str(&n.node_name, col_node)
        } else {
            String::new()
        };

        let spans = vec![
            Span::styled(format!(" {:<col_node$} ", node_text), row_style),
            Span::styled(
                format!("{:<col_ip$} ", truncate_str(&n.peer_address, col_ip)),
                Style::default().fg(Theme::cyan()),
            ),
            Span::styled(
                format!("{:<col_pasn$} ", n.peer_asn),
                Style::default().fg(Theme::fg()),
            ),
            Span::styled(
                format!("{:<col_lasn$} ", n.local_asn),
                Style::default().fg(Theme::dim()),
            ),
            state_badge,
            Span::styled(
                format!("{:<col_pol$} ", truncate_str(&n.policy_name, col_pol)),
                Style::default().fg(Theme::dim()),
            ),
            Span::styled(
                format!("{:<col_pod$} ", pod_cidr_str),
                Style::default().fg(if n.export_pod_cidr {
                    Theme::green()
                } else {
                    Theme::dim()
                }),
            ),
            Span::styled(
                format!("{:<col_routes$} ", routes_str),
                Style::default().fg(Theme::yellow()),
            ),
            Span::styled(
                format!("{:<col_uptime$}", truncate_str(&uptime_str, col_uptime)),
                Style::default().fg(Theme::dim()),
            ),
        ];

        lines.push(Line::from(spans));
    }

    let p = Paragraph::new(lines);
    f.render_widget(p, area);
}

fn render_services_table(f: &mut Frame, area: Rect, state: &BgpViewState) {
    let svcs = state.filtered_services();
    if svcs.is_empty() {
        let p = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "  ⊘ No LoadBalancer VIPs currently advertised via BGP.",
                Style::default().fg(Theme::yellow()),
            )),
        ]);
        f.render_widget(p, area);
        return;
    }

    let visible_rows = area.height.saturating_sub(2) as usize;
    if visible_rows == 0 {
        return;
    }

    let selected = state.selected_svc_idx;
    let mut start_idx = state.last_svc_start_idx.get();
    if selected < start_idx {
        start_idx = selected;
    } else if selected >= start_idx + visible_rows {
        start_idx = selected + 1 - visible_rows;
    }
    state.last_svc_start_idx.set(start_idx);

    let mut max_svc = "SERVICE".len();
    let mut max_ns = "NAMESPACE".len();
    let mut max_ip = "LOADBALANCER IP".len();
    let mut max_pool = "IP POOL".len();
    let mut max_nodes = "ANNOUNCING NODES".len();

    for s in &svcs {
        max_svc = max_svc.max(s.service_name.len());
        max_ns = max_ns.max(s.namespace.len());
        max_ip = max_ip.max(s.load_balancer_ip.len());
        let pool_str = s.ip_pool.as_deref().unwrap_or("default");
        max_pool = max_pool.max(pool_str.len());
        let nodes_str = if s.announcing_nodes.is_empty() {
            "All BGP Nodes".to_string()
        } else {
            format!("{} nodes", s.announcing_nodes.len())
        };
        max_nodes = max_nodes.max(nodes_str.len());
    }

    let mut col_svc = max_svc.max(16);
    let col_ns = max_ns.max(12);
    let col_ip = max_ip.max(16);
    let mut col_pool = max_pool.max(12);
    let mut col_nodes = max_nodes.max(16);

    let fixed_w = 1 + col_ns + 1 + col_ip + 1;
    let avail_w = area.width as usize;

    if avail_w > fixed_w + col_svc + 1 + col_pool + 1 + col_nodes {
        let surplus = avail_w - (fixed_w + col_svc + 1 + col_pool + 1 + col_nodes);
        let svc_extra = surplus * 45 / 100;
        let pool_extra = surplus * 30 / 100;
        let nodes_extra = surplus.saturating_sub(svc_extra + pool_extra);
        col_svc += svc_extra;
        col_pool += pool_extra;
        col_nodes += nodes_extra;
    }

    let mut lines = Vec::new();
    lines.push(Line::from(vec![
        Span::styled(format!(" {:<col_svc$} ", "SERVICE"), Theme::table_header()),
        Span::styled(format!("{:<col_ns$} ", "NAMESPACE"), Theme::table_header()),
        Span::styled(
            format!("{:<col_ip$} ", "LOADBALANCER IP"),
            Theme::table_header(),
        ),
        Span::styled(format!("{:<col_pool$} ", "IP POOL"), Theme::table_header()),
        Span::styled(
            format!("{:<col_nodes$}", "ANNOUNCING NODES"),
            Theme::table_header(),
        ),
    ]));
    lines.push(Line::from(Span::styled(
        "─".repeat(area.width as usize),
        Style::default().fg(Theme::border()),
    )));

    let end_idx = (start_idx + visible_rows).min(svcs.len());
    for i in start_idx..end_idx {
        let s = svcs[i];
        let is_sel = i == selected;
        let row_style = if is_sel {
            Style::default()
                .bg(Theme::sel_bg())
                .fg(Theme::fg())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Theme::fg())
        };

        let pool_str = s.ip_pool.as_deref().unwrap_or("default");
        let nodes_str = if s.announcing_nodes.is_empty() {
            "All BGP Nodes".to_string()
        } else {
            format!("{} nodes", s.announcing_nodes.len())
        };

        let spans = vec![
            Span::styled(
                format!(" {:<col_svc$} ", truncate_str(&s.service_name, col_svc)),
                row_style,
            ),
            Span::styled(
                format!("{:<col_ns$} ", truncate_str(&s.namespace, col_ns)),
                Style::default().fg(Theme::dim()),
            ),
            Span::styled(
                format!("{:<col_ip$} ", truncate_str(&s.load_balancer_ip, col_ip)),
                Style::default()
                    .fg(Theme::yellow())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:<col_pool$} ", truncate_str(pool_str, col_pool)),
                Style::default().fg(Theme::accent()),
            ),
            Span::styled(
                format!("{:<col_nodes$}", truncate_str(&nodes_str, col_nodes)),
                Style::default().fg(Theme::cyan()),
            ),
        ];
        lines.push(Line::from(spans));
    }

    let p = Paragraph::new(lines);
    f.render_widget(p, area);
}

fn render_pools_table(f: &mut Frame, area: Rect, state: &BgpViewState) {
    let pools = state.filtered_pools();
    if pools.is_empty() {
        let p = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "  ⊘ No Cilium/MetalLB IP Pools found.",
                Style::default().fg(Theme::yellow()),
            )),
        ]);
        f.render_widget(p, area);
        return;
    }

    let visible_rows = area.height.saturating_sub(2) as usize;
    if visible_rows == 0 {
        return;
    }

    let selected = state.selected_pool_idx;
    let mut start_idx = state.last_pool_start_idx.get();
    if selected < start_idx {
        start_idx = selected;
    } else if selected >= start_idx + visible_rows {
        start_idx = selected + 1 - visible_rows;
    }
    state.last_pool_start_idx.set(start_idx);

    let mut max_pool = "POOL NAME".len();
    let mut max_cidrs = "CIDRS / ADDRESS RANGES".len();
    let mut max_status = "STATUS".len();
    let mut max_sel = "SERVICE SELECTOR".len();

    for p in &pools {
        max_pool = max_pool.max(p.name.len());
        max_cidrs = max_cidrs.max(p.cidrs.join(", ").len());
        let status_str = if p.disabled { "Disabled" } else { "Active" };
        max_status = max_status.max(status_str.len());
        max_sel = max_sel.max(p.service_selector.len());
    }

    let mut col_pool = max_pool.max(16);
    let mut col_cidrs = max_cidrs.max(22);
    let col_status = max_status.max(8);
    let mut col_sel = max_sel.max(18);

    let fixed_w = 1 + col_status + 1;
    let avail_w = area.width as usize;

    if avail_w > fixed_w + col_pool + 1 + col_cidrs + 1 + col_sel {
        let surplus = avail_w - (fixed_w + col_pool + 1 + col_cidrs + 1 + col_sel);
        let pool_extra = surplus * 30 / 100;
        let cidrs_extra = surplus * 40 / 100;
        let sel_extra = surplus.saturating_sub(pool_extra + cidrs_extra);
        col_pool += pool_extra;
        col_cidrs += cidrs_extra;
        col_sel += sel_extra;
    }

    let mut lines = Vec::new();
    lines.push(Line::from(vec![
        Span::styled(
            format!(" {:<col_pool$} ", "POOL NAME"),
            Theme::table_header(),
        ),
        Span::styled(
            format!("{:<col_cidrs$} ", "CIDRS / ADDRESS RANGES"),
            Theme::table_header(),
        ),
        Span::styled(format!("{:<col_status$} ", "STATUS"), Theme::table_header()),
        Span::styled(
            format!("{:<col_sel$}", "SERVICE SELECTOR"),
            Theme::table_header(),
        ),
    ]));
    lines.push(Line::from(Span::styled(
        "─".repeat(area.width as usize),
        Style::default().fg(Theme::border()),
    )));

    let end_idx = (start_idx + visible_rows).min(pools.len());
    for i in start_idx..end_idx {
        let pool = pools[i];
        let is_sel = i == selected;
        let row_style = if is_sel {
            Style::default()
                .bg(Theme::sel_bg())
                .fg(Theme::fg())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Theme::fg())
        };

        let cidrs_str = pool.cidrs.join(", ");
        let status_str = if pool.disabled { "Disabled" } else { "Active" };
        let status_style = if pool.disabled {
            Style::default().fg(Theme::dim())
        } else {
            Style::default()
                .fg(Theme::green())
                .add_modifier(Modifier::BOLD)
        };

        let spans = vec![
            Span::styled(
                format!(" {:<col_pool$} ", truncate_str(&pool.name, col_pool)),
                row_style,
            ),
            Span::styled(
                format!("{:<col_cidrs$} ", truncate_str(&cidrs_str, col_cidrs)),
                Style::default().fg(Theme::cyan()),
            ),
            Span::styled(format!("{:<col_status$} ", status_str), status_style),
            Span::styled(
                format!(
                    "{:<col_sel$}",
                    truncate_str(&pool.service_selector, col_sel)
                ),
                Style::default().fg(Theme::dim()),
            ),
        ];
        lines.push(Line::from(spans));
    }

    let p = Paragraph::new(lines);
    f.render_widget(p, area);
}

fn render_detail_footer(f: &mut Frame, area: Rect, state: &BgpViewState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::border()))
        .title(Span::styled(
            " 🔍 PEER TIMERS & POLICY INSPECTOR ",
            Style::default()
                .fg(Theme::yellow())
                .add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if let Some(peer) = state.selected_peer() {
        let hold = peer
            .hold_time_seconds
            .map(|s| format!("{}s", s))
            .unwrap_or_else(|| "90s (default)".to_string());
        let keepalive = peer
            .keepalive_time_seconds
            .map(|s| format!("{}s", s))
            .unwrap_or_else(|| "30s (default)".to_string());
        let connect_retry = peer
            .connect_retry_seconds
            .map(|s| format!("{}s", s))
            .unwrap_or_else(|| "120s".to_string());
        let multihop = peer
            .multihop_ttl
            .map(|t| format!("TTL {}", t))
            .unwrap_or_else(|| "Disabled (direct L2)".to_string());
        let graceful = if peer.graceful_restart {
            "Enabled"
        } else {
            "Disabled"
        };

        let uptime_display = format_uptime_display(peer.uptime_or_last_change.as_deref());

        let l1 = Line::from(vec![
            Span::styled(" Selected Peer: ", Theme::header_label()),
            Span::styled(
                format!("{} (Node: {})", peer.peer_address, peer.node_name),
                Style::default()
                    .fg(Theme::cyan())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" │ Policy: ", Theme::header_label()),
            Span::styled(&peer.policy_name, Style::default().fg(Theme::fg())),
            Span::styled(" │ ASNs: ", Theme::header_label()),
            Span::styled(
                format!("Local {} ➔ Peer {}", peer.local_asn, peer.peer_asn),
                Style::default().fg(Theme::yellow()),
            ),
            Span::styled(" │ Uptime: ", Theme::header_label()),
            Span::styled(
                uptime_display,
                Style::default().fg(if peer.uptime_or_last_change.is_some() {
                    Theme::green()
                } else {
                    Theme::dim()
                }),
            ),
        ]);

        let l2 = Line::from(vec![
            Span::styled(" Hold Time: ", Theme::header_label()),
            Span::styled(hold, Style::default().fg(Theme::fg())),
            Span::styled(" │ KeepAlive: ", Theme::header_label()),
            Span::styled(keepalive, Style::default().fg(Theme::fg())),
            Span::styled(" │ Connect Retry: ", Theme::header_label()),
            Span::styled(connect_retry, Style::default().fg(Theme::fg())),
            Span::styled(" │ MultiHop: ", Theme::header_label()),
            Span::styled(multihop, Style::default().fg(Theme::fg())),
            Span::styled(" │ Graceful Restart: ", Theme::header_label()),
            Span::styled(
                graceful,
                Style::default().fg(if peer.graceful_restart {
                    Theme::green()
                } else {
                    Theme::dim()
                }),
            ),
        ]);

        // LoadBalancer VIPs are no longer copied onto every neighbour — they
        // are listed once, on the Advertised VIPs tab. So an empty prefix list
        // beside a non-zero route count is not "this peer advertises
        // nothing"; say where the VIPs are instead.
        let prefixes_summary = if !peer.advertised_prefixes.is_empty() {
            peer.advertised_prefixes.join(" │ ")
        } else if peer.routes_count > 0 {
            "LoadBalancer VIPs — see the Advertised VIPs tab".to_string()
        } else {
            "None".to_string()
        };

        let l3 = Line::from(vec![
            Span::styled(" Routes: ", Theme::header_label()),
            Span::styled(
                format!(
                    "{} Received │ {} Advertised",
                    peer.routes_received, peer.routes_count
                ),
                Style::default()
                    .fg(Theme::yellow())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" │ Advertised Prefixes: ", Theme::header_label()),
            Span::styled(
                truncate_str(&prefixes_summary, area.width.saturating_sub(45) as usize),
                Style::default().fg(Theme::dim()),
            ),
        ]);

        let p = Paragraph::new(vec![l1, l2, l3]);
        f.render_widget(p, inner);
    } else {
        let p = Paragraph::new(vec![
            Line::from(Span::styled("  Select a peer above to inspect BGP timers, MultiHop TTL, and advertised route prefixes.", Style::default().fg(Theme::dim()))),
        ]);
        f.render_widget(p, inner);
    }
}

pub fn format_uptime_display(uptime: Option<&str>) -> String {
    let raw = match uptime {
        Some(s) if !s.trim().is_empty() => s.trim(),
        _ => return "-".to_string(),
    };
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(raw) {
        let now = chrono::Utc::now();
        let secs = (now - dt.with_timezone(&chrono::Utc)).num_seconds();
        srelens_kube::format_age(secs)
    } else {
        raw.to_string()
    }
}

fn truncate_str(s: &str, max_len: usize) -> String {
    let clean = crate::views::sanitize_span_text(s);
    let char_count = clean.chars().count();
    if char_count > max_len && max_len > 3 {
        let prefix: String = clean.chars().take(max_len - 3).collect();
        format!("{}...", prefix)
    } else {
        clean
    }
}
