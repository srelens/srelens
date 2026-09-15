use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

use crate::theme::Theme;
use crate::tui_config::{CommandPopupDensity, TuiConfig};
use crate::ui::command_popup_rect;

pub const SAMPLE_SUGGESTIONS: &[(&str, &[&str], &str, &str, &str)] = &[
    ("pods", &["po", "pod"], "Workload", "List, inspect, and tail Kubernetes pods across namespaces", ":pods [namespace]"),
    ("deployments", &["deploy", "dp"], "Workload", "Manage, inspect, and scale deployment workloads", ":deployments [ns]"),
    ("services", &["svc", "service"], "Network", "Service routing, cluster IPs, NodePorts and LoadBalancers", ":services [ns]"),
    ("configmaps", &["cm"], "Config", "Key-value configuration maps and application data", ":configmaps [ns]"),
    ("secrets", &["sec"], "Config", "Kubernetes secrets with masked base64 credentials", ":secrets [ns]"),
    ("nodes", &["no", "node"], "Cluster", "Cluster worker and control-plane node hardware and health", ":nodes"),
    ("events", &["ev", "event"], "Cluster", "Cluster-wide event stream, errors, warnings & scheduling", ":events [ns]"),
    ("helm", &["releases"], "Helm", "Helm 3 release revisions, status, values and manifests", ":helm [ns]"),
    ("argo", &["argocd", "apps"], "GitOps", "ArgoCD applications, sync status, drift & GitOps control", ":argo"),
    ("workloads", &["wl"], "Workload", "Unified view of Pods, Deployments, STS & DS", ":workloads [ns]"),
    ("statefulsets", &["sts"], "Workload", "Stateful set workloads and distributed replicas", ":statefulsets [ns]"),
    ("daemonsets", &["ds"], "Workload", "Node-local daemonset agent workloads", ":daemonsets [ns]"),
    ("jobs", &["job"], "Workload", "Batch job runs and execution completion status", ":jobs [ns]"),
    ("cronjobs", &["cj"], "Workload", "Scheduled cron jobs and recurring execution history", ":cronjobs [ns]"),
    ("ingresses", &["ing"], "Network", "HTTP/HTTPS ingress routing rules and TLS certs", ":ingresses [ns]"),
    ("namespaces", &["ns"], "Cluster", "Cluster tenancy namespaces switcher and viewer", ":namespaces"),
    ("persistentvolumes", &["pv"], "Storage", "Cluster-wide persistent storage volumes", ":persistentvolumes"),
    ("persistentvolumeclaims", &["pvc"], "Storage", "Persistent storage volume claims by namespace", ":persistentvolumeclaims [ns]"),
    ("storageclasses", &["sc"], "Storage", "Storage provisioners, volume plugins & reclaim policies", ":storageclasses"),
    ("serviceaccounts", &["sa"], "Auth / RBAC", "Service account identities and RBAC token bindings", ":serviceaccounts [ns]"),
    ("roles", &["role"], "Auth / RBAC", "Namespace-scoped RBAC roles and resource permissions", ":roles [ns]"),
    ("top", &["toppods"], "Hotspots", "Top Hotspots ranking (Pods & Nodes by CPU/Memory)", ":top"),
    ("assistant", &["ai", "chat"], "AI Assistant", "SRElens AI Assistant Chat for troubleshooting", ":assistant"),
    ("config", &["tui"], "Configuration", "Configure command popup dimensions, visible rows & text size", ":config"),
    ("argo", &["applications", "argocd"], "GitOps", "ArgoCD applications, sync status & drift", ":argo [ns]"),
    ("banner", &["features", "guide"], "Guide", "Startup feature banner and SRElens guide", ":banner"),
    ("toolbox", &["tools"], "Diagnostic", "Toolbox diagnostics (kubectl, helm, krew plugins)", ":toolbox"),
    ("overview", &["info"], "Overview", "Cluster overview, health summary and node/pod capacity", ":overview"),
    ("quit", &["q", "exit"], "System", "Quit SRElens TUI session", ":quit"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuiConfigViewState {
    pub selected_field: usize, // 0 = Width, 1 = Visible Rows, 2 = Text Size / Density, 3 = Startup Banner, 4 = Argo Hub Context, 5 = Argo Hub Kubeconfig
    pub is_editing: bool,
    pub edit_buffer: String,
    pub edit_cursor: usize,
    pub available_contexts: Vec<String>,
}

impl Default for TuiConfigViewState {
    fn default() -> Self {
        Self::new()
    }
}

impl TuiConfigViewState {
    pub fn new() -> Self {
        Self {
            selected_field: 0,
            is_editing: false,
            edit_buffer: String::new(),
            edit_cursor: 0,
            available_contexts: Vec::new(),
        }
    }

    pub fn select_next_field(&mut self) {
        if self.is_editing {
            return;
        }
        self.selected_field = (self.selected_field + 1) % 6;
    }

    pub fn select_prev_field(&mut self) {
        if self.is_editing {
            return;
        }
        if self.selected_field == 0 {
            self.selected_field = 5;
        } else {
            self.selected_field -= 1;
        }
    }

    pub fn start_editing(&mut self, config: &TuiConfig) {
        match self.selected_field {
            4 => {
                self.edit_buffer = config.argo_hub_context.clone().unwrap_or_default();
                self.edit_cursor = self.edit_buffer.chars().count();
                self.is_editing = true;
            }
            5 => {
                self.edit_buffer = config
                    .argo_hub_kubeconfig
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                self.edit_cursor = self.edit_buffer.chars().count();
                self.is_editing = true;
            }
            _ => {}
        }
    }

    pub fn cancel_editing(&mut self) {
        self.is_editing = false;
        self.edit_buffer.clear();
        self.edit_cursor = 0;
    }

    pub fn finish_editing(&mut self, config: &mut TuiConfig) -> Result<(), String> {
        let val = self.edit_buffer.trim().to_string();
        match self.selected_field {
            4 => {
                config.argo_hub_context = if val.is_empty() { None } else { Some(val) };
            }
            5 => {
                config.argo_hub_kubeconfig = if val.is_empty() {
                    None
                } else {
                    Some(std::path::PathBuf::from(val))
                };
            }
            _ => {}
        }
        self.is_editing = false;
        self.edit_buffer.clear();
        self.edit_cursor = 0;
        config.save()
    }

    pub fn cursor_pos(&self) -> usize {
        let len = self.edit_buffer.chars().count();
        self.edit_cursor.min(len)
    }

    pub fn move_cursor_left(&mut self) {
        let pos = self.cursor_pos();
        if pos > 0 {
            self.edit_cursor = pos - 1;
        } else {
            self.edit_cursor = 0;
        }
    }

    pub fn move_cursor_right(&mut self) {
        let pos = self.cursor_pos();
        let len = self.edit_buffer.chars().count();
        if pos < len {
            self.edit_cursor = pos + 1;
        } else {
            self.edit_cursor = len;
        }
    }

    pub fn move_cursor_home(&mut self) {
        self.edit_cursor = 0;
    }

    pub fn move_cursor_end(&mut self) {
        self.edit_cursor = self.edit_buffer.chars().count();
    }

    pub fn insert_char(&mut self, c: char) {
        let pos = self.cursor_pos();
        let mut chars: Vec<char> = self.edit_buffer.chars().collect();
        chars.insert(pos, c);
        self.edit_buffer = chars.into_iter().collect();
        self.edit_cursor = pos + 1;
    }

    pub fn insert_str(&mut self, s: &str) {
        let pos = self.cursor_pos();
        let mut chars: Vec<char> = self.edit_buffer.chars().collect();
        let added_len = s.chars().count();
        for (i, c) in s.chars().enumerate() {
            chars.insert(pos + i, c);
        }
        self.edit_buffer = chars.into_iter().collect();
        self.edit_cursor = pos + added_len;
    }

    pub fn backspace(&mut self) {
        let pos = self.cursor_pos();
        if pos > 0 {
            let mut chars: Vec<char> = self.edit_buffer.chars().collect();
            chars.remove(pos - 1);
            self.edit_buffer = chars.into_iter().collect();
            self.edit_cursor = pos - 1;
        }
    }

    pub fn delete(&mut self) {
        let pos = self.cursor_pos();
        let mut chars: Vec<char> = self.edit_buffer.chars().collect();
        if pos < chars.len() {
            chars.remove(pos);
            self.edit_buffer = chars.into_iter().collect();
            self.edit_cursor = pos;
        }
    }

    pub fn delete_word_back(&mut self) {
        let pos = self.cursor_pos();
        if pos == 0 {
            return;
        }
        let chars: Vec<char> = self.edit_buffer.chars().collect();
        let mut i = pos;
        while i > 0 && (chars[i - 1].is_whitespace() || chars[i - 1] == '/' || chars[i - 1] == '-') {
            i -= 1;
        }
        while i > 0 && !chars[i - 1].is_whitespace() && chars[i - 1] != '/' && chars[i - 1] != '-' {
            i -= 1;
        }
        let mut new_chars = chars[..i].to_vec();
        new_chars.extend_from_slice(&chars[pos..]);
        self.edit_buffer = new_chars.into_iter().collect();
        self.edit_cursor = i;
    }

    pub fn clear_input(&mut self) {
        self.edit_buffer.clear();
        self.edit_cursor = 0;
    }

    pub fn clear_current(&mut self, config: &mut TuiConfig) -> Result<(), String> {
        if self.is_editing {
            return Ok(());
        }
        match self.selected_field {
            4 => config.argo_hub_context = None,
            5 => config.argo_hub_kubeconfig = None,
            _ => {}
        }
        config.save()
    }

    pub fn adjust_current(&mut self, delta: i32, config: &mut TuiConfig) -> Result<(), String> {
        if self.is_editing {
            return Ok(());
        }
        match self.selected_field {
            0 => {
                let current = config.command_popup_max_width as i32;
                let next = (current + delta * 5).clamp(40, 200) as u16;
                config.command_popup_max_width = next;
            }
            1 => {
                let current = config.command_popup_max_visible as i32;
                let next = (current + delta).clamp(3, 20) as usize;
                config.command_popup_max_visible = next;
            }
            2 => {
                let current = config.command_popup_density.scale() as i32;
                let next = (current + delta).clamp(1, 4) as u8;
                config.command_popup_density = CommandPopupDensity::from_scale(next);
            }
            3 => {
                config.show_feature_banner = !config.show_feature_banner;
            }
            4 => {
                let mut options: Vec<Option<String>> = vec![None];
                for ctx in &self.available_contexts {
                    options.push(Some(ctx.clone()));
                }
                let current_idx = options
                    .iter()
                    .position(|opt| *opt == config.argo_hub_context)
                    .unwrap_or(0);
                let next_idx = if delta > 0 {
                    (current_idx + 1) % options.len()
                } else if current_idx == 0 {
                    options.len().saturating_sub(1)
                } else {
                    current_idx - 1
                };
                config.argo_hub_context = options[next_idx].clone();
            }
            5 => {
                if delta < 0 {
                    config.argo_hub_kubeconfig = None;
                }
            }
            _ => {}
        }
        config.save()
    }

    pub fn cycle_current(&mut self, config: &mut TuiConfig) -> Result<(), String> {
        if self.is_editing {
            return Ok(());
        }
        match self.selected_field {
            0 => {
                let current = config.command_popup_max_width;
                config.command_popup_max_width = if current >= 200 { 40 } else { (current + 20).min(200) };
            }
            1 => {
                let current = config.command_popup_max_visible;
                config.command_popup_max_visible = if current >= 20 { 3 } else { (current + 3).min(20) };
            }
            2 => {
                config.command_popup_density = config.command_popup_density.toggle();
            }
            3 => {
                config.show_feature_banner = !config.show_feature_banner;
            }
            4 => {
                self.start_editing(config);
            }
            5 => {
                self.start_editing(config);
            }
            _ => {}
        }
        config.save()
    }

    pub fn reset_defaults(&mut self, config: &mut TuiConfig) -> Result<(), String> {
        *config = TuiConfig::default();
        config.save()
    }
}

fn render_slider(value: f32, total_width: usize) -> String {
    let clamped = value.clamp(0.0, 1.0);
    let filled = (clamped * total_width as f32).round() as usize;
    let empty = total_width.saturating_sub(filled);
    format!("[{}{}]", "█".repeat(filled), "░".repeat(empty))
}

pub fn render_tui_config_view(
    f: &mut Frame,
    area: Rect,
    state: &TuiConfigViewState,
    config: &TuiConfig,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::accent()))
        .title(Span::styled(" TUI Configuration ", Theme::title()));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // Top description
            Constraint::Min(10),   // Controls and Live Preview
            Constraint::Length(1), // Bottom key hints
        ])
        .split(inner);

    // 1. Top Header Description
    let desc_lines = vec![
        Line::from(vec![
            Span::styled("TUI Display & Layout Options: ", Theme::header_label()),
            Span::styled(
                "Configure command popup dimensions, visible rows, text size & startup banner.",
                Style::default().fg(Theme::fg()).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![Span::styled(
            "Changes apply immediately and are saved automatically. Save failures are reported below.",
            Style::default().fg(Theme::dim()),
        )]),
    ];
    f.render_widget(Paragraph::new(desc_lines), chunks[0]);

    // 2. Middle Region: Controls (left) & Live Preview (right) or stacked if narrow
    let (controls_area, preview_area) = if chunks[1].width >= 90 {
        let h_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
            .split(chunks[1]);
        (h_chunks[0], h_chunks[1])
    } else {
        let v_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(25), Constraint::Min(8)])
            .split(chunks[1]);
        (v_chunks[0], v_chunks[1])
    };

    // Setting cards: 6 cards total (0..=5)
    let card_height = if controls_area.height >= 24 { 4 } else { 3 };
    let control_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(card_height), // 0: Popup Width
            Constraint::Length(card_height), // 1: Visible Rows
            Constraint::Length(card_height), // 2: Text Size / Density
            Constraint::Length(card_height), // 3: Startup Feature Banner
            Constraint::Length(card_height), // 4: ArgoCD Hub Context
            Constraint::Length(card_height), // 5: ArgoCD Hub Kubeconfig Path
            Constraint::Min(0),
        ])
        .split(controls_area);

    // Setting 0: Command Popup Max Width
    let is_width_selected = state.selected_field == 0;
    let width_border_color = if is_width_selected {
        Theme::cyan()
    } else {
        Theme::border()
    };
    let width_title = if is_width_selected {
        " ▶ Command Popup Max Width "
    } else {
        "   Command Popup Max Width "
    };
    let width_block = Block::default()
        .borders(Borders::ALL)
        .border_type(Theme::border_type())
        .border_style(Style::default().fg(width_border_color))
        .title(Span::styled(
            width_title,
            if is_width_selected {
                Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Theme::fg())
            },
        ));
    let width_inner = width_block.inner(control_chunks[0]);
    f.render_widget(width_block, control_chunks[0]);

    let width_val = config.command_popup_max_width;
    let width_pct = (width_val.saturating_sub(40) as f32) / (160.0);
    let slider_width = (width_inner.width.saturating_sub(4) as usize).min(24).max(10);
    let width_lines = vec![
        Line::from(vec![
            Span::styled("Width: ", Style::default().fg(Theme::dim())),
            Span::styled(
                format!("{width_val:>3} cols "),
                Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD),
            ),
            Span::styled(render_slider(width_pct, slider_width), Style::default().fg(Theme::accent())),
            Span::styled(" [40..=200]", Style::default().fg(Theme::dim())),
        ]),
        Line::from(vec![
            Span::styled("Step: ±5  |  Use ", Style::default().fg(Theme::dim())),
            Span::styled("h/l", Style::default().fg(Theme::yellow())),
            Span::styled(" or ", Style::default().fg(Theme::dim())),
            Span::styled("←/→", Style::default().fg(Theme::yellow())),
            Span::styled(" or ", Style::default().fg(Theme::dim())),
            Span::styled("-/+", Style::default().fg(Theme::yellow())),
            Span::styled(" to adjust", Style::default().fg(Theme::dim())),
        ]),
    ];
    f.render_widget(Paragraph::new(width_lines), width_inner);

    // Setting 1: Command Popup Max Visible Rows
    let is_rows_selected = state.selected_field == 1;
    let rows_border_color = if is_rows_selected {
        Theme::cyan()
    } else {
        Theme::border()
    };
    let rows_title = if is_rows_selected {
        " ▶ Command Popup Max Visible Rows "
    } else {
        "   Command Popup Max Visible Rows "
    };
    let rows_block = Block::default()
        .borders(Borders::ALL)
        .border_type(Theme::border_type())
        .border_style(Style::default().fg(rows_border_color))
        .title(Span::styled(
            rows_title,
            if is_rows_selected {
                Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Theme::fg())
            },
        ));
    let rows_inner = rows_block.inner(control_chunks[1]);
    f.render_widget(rows_block, control_chunks[1]);

    let rows_val = config.command_popup_max_visible;
    let rows_pct = (rows_val.saturating_sub(3) as f32) / (17.0);
    let slider_rows = (rows_inner.width.saturating_sub(4) as usize).min(24).max(10);
    let rows_lines = vec![
        Line::from(vec![
            Span::styled("Rows:  ", Style::default().fg(Theme::dim())),
            Span::styled(
                format!("{rows_val:>3} rows "),
                Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD),
            ),
            Span::styled(render_slider(rows_pct, slider_rows), Style::default().fg(Theme::accent())),
            Span::styled(" [3..=20]", Style::default().fg(Theme::dim())),
        ]),
        Line::from(vec![
            Span::styled("Step: ±1  |  Use ", Style::default().fg(Theme::dim())),
            Span::styled("h/l", Style::default().fg(Theme::yellow())),
            Span::styled(" or ", Style::default().fg(Theme::dim())),
            Span::styled("←/→", Style::default().fg(Theme::yellow())),
            Span::styled(" or ", Style::default().fg(Theme::dim())),
            Span::styled("-/+", Style::default().fg(Theme::yellow())),
            Span::styled(" to adjust", Style::default().fg(Theme::dim())),
        ]),
    ];
    f.render_widget(Paragraph::new(rows_lines), rows_inner);

    // Setting 2: Command Popup Text Size / Density
    let is_density_selected = state.selected_field == 2;
    let density_border_color = if is_density_selected {
        Theme::cyan()
    } else {
        Theme::border()
    };
    let density_title = if is_density_selected {
        " ▶ Command Popup Text Size "
    } else {
        "   Command Popup Text Size "
    };
    let density_block = Block::default()
        .borders(Borders::ALL)
        .border_type(Theme::border_type())
        .border_style(Style::default().fg(density_border_color))
        .title(Span::styled(
            density_title,
            if is_density_selected {
                Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Theme::fg())
            },
        ));
    let density_inner = density_block.inner(control_chunks[2]);
    f.render_widget(density_block, control_chunks[2]);

    let scale_val = config.command_popup_density.scale();
    let scale_pct = (scale_val - 1) as f32 / 3.0;
    let slider_density = (density_inner.width.saturating_sub(4) as usize).min(24).max(10);
    let density_lines = vec![
        Line::from(vec![
            Span::styled("Scale: ", Style::default().fg(Theme::dim())),
            Span::styled(
                format!("{scale_val} / 4  "),
                Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD),
            ),
            Span::styled(render_slider(scale_pct, slider_density), Style::default().fg(Theme::accent())),
            Span::styled(" [1..=4] ", Style::default().fg(Theme::dim())),
            Span::styled(
                format!("({})", config.command_popup_density.label()),
                Style::default().fg(Theme::yellow()).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Step: ±1  |  Use ", Style::default().fg(Theme::dim())),
            Span::styled("h/l", Style::default().fg(Theme::yellow())),
            Span::styled(" or ", Style::default().fg(Theme::dim())),
            Span::styled("←/→", Style::default().fg(Theme::yellow())),
            Span::styled(" or ", Style::default().fg(Theme::dim())),
            Span::styled("-/+", Style::default().fg(Theme::yellow())),
            Span::styled(" or ", Style::default().fg(Theme::dim())),
            Span::styled("Enter", Style::default().fg(Theme::yellow())),
            Span::styled(" to adjust", Style::default().fg(Theme::dim())),
        ]),
    ];
    f.render_widget(Paragraph::new(density_lines), density_inner);

    // Setting 3: Startup Feature Banner
    let is_banner_selected = state.selected_field == 3;
    let banner_border_color = if is_banner_selected {
        Theme::cyan()
    } else {
        Theme::border()
    };
    let banner_title = if is_banner_selected {
        " ▶ Startup Feature Banner "
    } else {
        "   Startup Feature Banner "
    };
    let banner_block = Block::default()
        .borders(Borders::ALL)
        .border_type(Theme::border_type())
        .border_style(Style::default().fg(banner_border_color))
        .title(Span::styled(
            banner_title,
            if is_banner_selected {
                Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Theme::fg())
            },
        ));
    let banner_inner = banner_block.inner(control_chunks[3]);
    f.render_widget(banner_block, control_chunks[3]);

    let (checkbox_str, status_str, status_style) = if config.show_feature_banner {
        ("[● Show on startup]", "Enabled", Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD))
    } else {
        ("[○ Don't show]", "Disabled", Style::default().fg(Theme::dim()))
    };

    let banner_lines = vec![
        Line::from(vec![
            Span::styled("Banner: ", Style::default().fg(Theme::dim())),
            Span::styled(
                checkbox_str,
                if config.show_feature_banner {
                    Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Theme::dim())
                },
            ),
            Span::styled("  Status: ", Style::default().fg(Theme::dim())),
            Span::styled(status_str, status_style),
            Span::styled("  Command: ", Style::default().fg(Theme::dim())),
            Span::styled(":banner", Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD)),
            Span::styled(" / ", Style::default().fg(Theme::dim())),
            Span::styled(":features", Style::default().fg(Theme::cyan())),
        ]),
        Line::from(vec![
            Span::styled("Action: Toggle  |  Use ", Style::default().fg(Theme::dim())),
            Span::styled("Space", Style::default().fg(Theme::yellow())),
            Span::styled(" or ", Style::default().fg(Theme::dim())),
            Span::styled("Enter", Style::default().fg(Theme::yellow())),
            Span::styled(" or ", Style::default().fg(Theme::dim())),
            Span::styled("←/→", Style::default().fg(Theme::yellow())),
            Span::styled(" to toggle", Style::default().fg(Theme::dim())),
        ]),
    ];
    f.render_widget(Paragraph::new(banner_lines), banner_inner);

    // Setting 4: ArgoCD Hub Context
    let is_hub_ctx_selected = state.selected_field == 4;
    let hub_ctx_border_color = if is_hub_ctx_selected {
        Theme::cyan()
    } else {
        Theme::border()
    };
    let hub_ctx_title = if is_hub_ctx_selected {
        " ▶ ArgoCD Hub Context "
    } else {
        "   ArgoCD Hub Context "
    };
    let hub_ctx_block = Block::default()
        .borders(Borders::ALL)
        .border_type(Theme::border_type())
        .border_style(Style::default().fg(hub_ctx_border_color))
        .title(Span::styled(
            hub_ctx_title,
            if is_hub_ctx_selected {
                Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Theme::fg())
            },
        ));
    let hub_ctx_inner = hub_ctx_block.inner(control_chunks[4]);
    f.render_widget(hub_ctx_block, control_chunks[4]);

    let (ctx_display, ctx_style) = match &config.argo_hub_context {
        Some(ctx) if !ctx.trim().is_empty() => (
            ctx.as_str(),
            Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD),
        ),
        _ => (
            "(none - local cluster)",
            Style::default().fg(Theme::dim()),
        ),
    };

    let mut hub_ctx_lines = vec![
        Line::from(vec![
            Span::styled("Context: ", Style::default().fg(Theme::dim())),
            Span::styled(ctx_display, ctx_style),
        ]),
    ];
    if hub_ctx_inner.height > 1 {
        hub_ctx_lines.push(Line::from(vec![
            Span::styled("Action: ", Style::default().fg(Theme::dim())),
            Span::styled("e/Enter", Style::default().fg(Theme::yellow())),
            Span::styled(" Edit  |  ", Style::default().fg(Theme::dim())),
            Span::styled("h/l", Style::default().fg(Theme::yellow())),
            Span::styled(" Cycle  |  ", Style::default().fg(Theme::dim())),
            Span::styled("c", Style::default().fg(Theme::yellow())),
            Span::styled(" Clear", Style::default().fg(Theme::dim())),
        ]));
    }
    f.render_widget(Paragraph::new(hub_ctx_lines), hub_ctx_inner);

    // Setting 5: ArgoCD Hub Kubeconfig Path
    let is_hub_cfg_selected = state.selected_field == 5;
    let hub_cfg_border_color = if is_hub_cfg_selected {
        Theme::cyan()
    } else {
        Theme::border()
    };
    let hub_cfg_title = if is_hub_cfg_selected {
        " ▶ ArgoCD Hub Kubeconfig Path "
    } else {
        "   ArgoCD Hub Kubeconfig Path "
    };
    let hub_cfg_block = Block::default()
        .borders(Borders::ALL)
        .border_type(Theme::border_type())
        .border_style(Style::default().fg(hub_cfg_border_color))
        .title(Span::styled(
            hub_cfg_title,
            if is_hub_cfg_selected {
                Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Theme::fg())
            },
        ));
    let hub_cfg_inner = hub_cfg_block.inner(control_chunks[5]);
    f.render_widget(hub_cfg_block, control_chunks[5]);

    let (cfg_display, cfg_style) = match &config.argo_hub_kubeconfig {
        Some(p) if !p.as_os_str().is_empty() => (
            p.display().to_string(),
            Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD),
        ),
        _ => (
            "(default: in-cluster / $KUBECONFIG)".to_string(),
            Style::default().fg(Theme::dim()),
        ),
    };

    let mut hub_cfg_lines = vec![
        Line::from(vec![
            Span::styled("Path: ", Style::default().fg(Theme::dim())),
            Span::styled(cfg_display, cfg_style),
        ]),
    ];
    if hub_cfg_inner.height > 1 {
        hub_cfg_lines.push(Line::from(vec![
            Span::styled("Action: ", Style::default().fg(Theme::dim())),
            Span::styled("e/Enter", Style::default().fg(Theme::yellow())),
            Span::styled(" Edit Path  |  ", Style::default().fg(Theme::dim())),
            Span::styled("c", Style::default().fg(Theme::yellow())),
            Span::styled(" Clear to Default", Style::default().fg(Theme::dim())),
        ]));
    }
    f.render_widget(Paragraph::new(hub_cfg_lines), hub_cfg_inner);

    // 3. Live Preview (Feature Banner, ArgoCD GitOps, or Command Popup)
    if state.selected_field == 3 {
        let preview_block = Block::default()
            .borders(Borders::ALL)
            .border_type(Theme::border_type())
            .border_style(Style::default().fg(Theme::border()))
            .title(Span::styled(
                " Live Preview: Startup Feature Banner (:features, :banner) ",
                Style::default().fg(Theme::accent()).add_modifier(Modifier::BOLD),
            ));
        let preview_inner = preview_block.inner(preview_area);
        f.render_widget(preview_block, preview_area);

        crate::ui::render_feature_banner_modal(f, preview_inner, config.show_feature_banner);
    } else if state.selected_field == 4 || state.selected_field == 5 {
        let preview_block = Block::default()
            .borders(Borders::ALL)
            .border_type(Theme::border_type())
            .border_style(Style::default().fg(Theme::border()))
            .title(Span::styled(
                " Live Preview: ArgoCD GitOps Hub-and-Spoke Topology (:argo) ",
                Style::default().fg(Theme::accent()).add_modifier(Modifier::BOLD),
            ));
        let preview_inner = preview_block.inner(preview_area);
        f.render_widget(preview_block, preview_area);

        let resolved_hub_ctx = config.resolved_argo_hub_context();
        let resolved_hub_kc = config.resolved_argo_hub_kubeconfig();
        let is_remote = resolved_hub_ctx.is_some() || resolved_hub_kc.is_some();
        let topology_mode_span = if is_remote {
            Span::styled("Hub-and-Spoke (Central Management Cluster)", Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD))
        } else {
            Span::styled("Local / In-Cluster (Single Cluster)", Theme::status_ok().add_modifier(Modifier::BOLD))
        };

        let hub_context_str = resolved_hub_ctx.as_deref().unwrap_or("(active cluster)");
        let hub_kubeconfig_str = resolved_hub_kc.as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "(default $KUBECONFIG)".to_string());

        let mut lines = vec![
            Line::from(vec![
                Span::styled("Topology Mode: ", Theme::header_label()),
                topology_mode_span,
            ]),
            Line::from(vec![
                Span::styled("Hub Context:   ", Theme::header_label()),
                Span::styled(hub_context_str, Style::default().fg(Theme::yellow()).add_modifier(Modifier::BOLD)),
            ]),
            Line::from(vec![
                Span::styled("Hub Kubeconfig:", Theme::header_label()),
                Span::styled(format!(" {}", hub_kubeconfig_str), Style::default().fg(Theme::fg())),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("How ArgoCD Hub-and-Spoke Works:", Style::default().fg(Theme::accent()).add_modifier(Modifier::BOLD)),
            ]),
            Line::from(vec![
                Span::styled("• In multi-cluster setups, ArgoCD runs in a dedicated ", Style::default().fg(Theme::dim())),
                Span::styled("Hub cluster.", Style::default().fg(Theme::fg())),
            ]),
            Line::from(vec![
                Span::styled("• Spoke clusters do not run ArgoCD, so querying them locally returns a 404.", Style::default().fg(Theme::dim())),
            ]),
            Line::from(vec![
                Span::styled("• Setting ", Style::default().fg(Theme::dim())),
                Span::styled("argo_hub_context", Style::default().fg(Theme::yellow())),
                Span::styled(" routes requests to the Hub and filters for the current spoke.", Style::default().fg(Theme::dim())),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled(format!("Detected Contexts ({}):", state.available_contexts.len()), Theme::header_label()),
            ]),
        ];

        if state.available_contexts.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("  (No other contexts detected in kubeconfig)", Style::default().fg(Theme::dim())),
            ]));
        } else {
            for ctx in state.available_contexts.iter().take(6) {
                let is_current_hub = Some(ctx) == config.argo_hub_context.as_ref();
                let marker = if is_current_hub { " ▶ [HUB] " } else { "   •     " };
                let style = if is_current_hub {
                    Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Theme::fg())
                };
                lines.push(Line::from(vec![
                    Span::styled(marker, if is_current_hub { Style::default().fg(Theme::cyan()) } else { Style::default().fg(Theme::dim()) }),
                    Span::styled(ctx, style),
                ]));
            }
            if state.available_contexts.len() > 6 {
                lines.push(Line::from(vec![
                    Span::styled(format!("  ... and {} more contexts", state.available_contexts.len() - 6), Style::default().fg(Theme::dim())),
                ]));
            }
        }

        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("Quick Actions: ", Style::default().fg(Theme::dim())),
            Span::styled("<e>", Style::default().fg(Theme::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled(" Edit text  |  ", Style::default().fg(Theme::dim())),
            Span::styled("<h/l>", Style::default().fg(Theme::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled(" Cycle contexts  |  ", Style::default().fg(Theme::dim())),
            Span::styled("<c>", Style::default().fg(Theme::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled(" Clear  |  ", Style::default().fg(Theme::dim())),
            Span::styled(":argo", Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD)),
            Span::styled(" Jump to ArgoCD", Style::default().fg(Theme::dim())),
        ]));

        let p = Paragraph::new(lines).wrap(ratatui::widgets::Wrap { trim: true });
        f.render_widget(p, preview_inner);
    } else {
        let preview_block = Block::default()
            .borders(Borders::ALL)
            .border_type(Theme::border_type())
            .border_style(Style::default().fg(Theme::border()))
            .title(Span::styled(
                " Live Preview: Command Popup (:) ",
                Style::default().fg(Theme::accent()).add_modifier(Modifier::BOLD),
            ));
        let preview_inner = preview_block.inner(preview_area);
        f.render_widget(preview_block, preview_area);

        if preview_inner.height >= 4 && preview_inner.width >= 20 {
        // Simulated statusbar at bottom of preview area
        let sim_bar_y = preview_inner.y + preview_inner.height.saturating_sub(1);
        let sim_bar_area = Rect {
            x: preview_inner.x,
            y: sim_bar_y,
            width: preview_inner.width,
            height: 1,
        };
        let sim_cmd_line = Line::from(vec![
            Span::styled(":", Theme::prompt()),
            Span::styled("po", Style::default().fg(Theme::fg())),
            Span::styled("█", Style::default().fg(Theme::cyan())),
        ]);
        f.render_widget(Paragraph::new(sim_cmd_line), sim_bar_area);

        // Compute popup area relative to sim_bar_area using command_popup_rect
        let calc_popup = command_popup_rect(
            sim_bar_area,
            SAMPLE_SUGGESTIONS.len(),
            config.command_popup_max_width,
            config.command_popup_max_visible,
            config.command_popup_density,
        );

        // Clamp popup vertically so it stays within preview_inner
        let max_avail_height = preview_inner.height.saturating_sub(1);
        let popup_height = calc_popup.height.min(max_avail_height);
        let popup_y = sim_bar_y.saturating_sub(popup_height).max(preview_inner.y);
        let popup_width = calc_popup.width.min(preview_inner.width.saturating_sub(2));

        let popup_area = Rect {
            x: preview_inner.x + 1,
            y: popup_y,
            width: popup_width,
            height: popup_height,
        };

        f.render_widget(Clear, popup_area);

        let density = config.command_popup_density;
        let inner_height = popup_area.height.saturating_sub(2);
        let item_lines = density.item_height();
        let max_fits = (inner_height / item_lines) as usize;
        let visible_count = SAMPLE_SUGGESTIONS
            .len()
            .min(config.command_popup_max_visible)
            .min(max_fits.max(1));
        let visible_slice = &SAMPLE_SUGGESTIONS[0..visible_count];
        let popup_inner_w = popup_area.width.saturating_sub(2) as usize;

        let items: Vec<ListItem> = visible_slice
            .iter()
            .enumerate()
            .map(|(i, (name, aliases, cat, desc, syntax))| {
                let is_selected = i == 0;
                let prefix = if is_selected { "▶ " } else { "  " };

                match density {
                    CommandPopupDensity::ExtraLarge => {
                        let alias_str = if !aliases.is_empty() {
                            format!(" ({})", aliases.join(", "))
                        } else {
                            String::new()
                        };
                        let cat_badge = format!("[{}]", cat);
                        let name_text = format!("{}{}{}", prefix, name.to_uppercase(), alias_str);

                        let name_len = name_text.chars().count();
                        let badge_len = cat_badge.chars().count();
                        let spacer_len = popup_inner_w.saturating_sub(name_len + badge_len + 1).max(2);
                        let spacer = " ".repeat(spacer_len);

                        let line1 = Line::from(vec![
                            Span::styled(
                                name_text,
                                if is_selected {
                                    Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD)
                                } else {
                                    Style::default().fg(Theme::fg()).add_modifier(Modifier::BOLD)
                                },
                            ),
                            Span::raw(spacer),
                            Span::styled(
                                cat_badge,
                                if is_selected {
                                    Style::default().fg(Theme::accent()).add_modifier(Modifier::BOLD)
                                } else {
                                    Style::default().fg(Theme::dim())
                                },
                            ),
                        ]);

                        let line2 = Line::from(vec![
                            Span::raw("    "),
                            Span::styled(
                                *desc,
                                if is_selected {
                                    Style::default().fg(Theme::fg())
                                } else {
                                    Style::default().fg(Theme::dim())
                                },
                            ),
                        ]);

                        let mut line3_spans = vec![
                            Span::raw("    "),
                            Span::styled("Usage: ", Style::default().fg(Theme::dim())),
                            Span::styled(
                                if !syntax.is_empty() { *syntax } else { *name },
                                if is_selected {
                                    Style::default().fg(Theme::yellow()).add_modifier(Modifier::BOLD)
                                } else {
                                    Style::default().fg(Theme::yellow())
                                },
                            ),
                        ];
                        if !aliases.is_empty() {
                            line3_spans.push(Span::styled("  |  Aliases: ", Style::default().fg(Theme::dim())));
                            line3_spans.push(Span::styled(
                                aliases.join(", "),
                                if is_selected {
                                    Style::default().fg(Theme::fg())
                                } else {
                                    Style::default().fg(Theme::dim())
                                },
                            ));
                        }
                        let line3 = Line::from(line3_spans);

                        ListItem::new(vec![line1, line2, line3]).style(if is_selected {
                            Theme::selected_row()
                        } else {
                            Style::default()
                        })
                    }
                    CommandPopupDensity::Large => {
                        let alias_str = if !aliases.is_empty() {
                            format!(" ({})", aliases.join(", "))
                        } else {
                            String::new()
                        };
                        let cat_badge = format!("[{}]", cat);
                        let name_text = format!("{}{}{}", prefix, name.to_uppercase(), alias_str);

                        let name_len = name_text.chars().count();
                        let badge_len = cat_badge.chars().count();
                        let spacer_len = popup_inner_w.saturating_sub(name_len + badge_len + 1).max(2);
                        let spacer = " ".repeat(spacer_len);

                        let line1 = Line::from(vec![
                            Span::styled(
                                name_text,
                                if is_selected {
                                    Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD)
                                } else {
                                    Style::default().fg(Theme::fg()).add_modifier(Modifier::BOLD)
                                },
                            ),
                            Span::raw(spacer),
                            Span::styled(
                                cat_badge,
                                if is_selected {
                                    Style::default().fg(Theme::accent()).add_modifier(Modifier::BOLD)
                                } else {
                                    Style::default().fg(Theme::dim())
                                },
                            ),
                        ]);

                        let mut line2_spans = vec![
                            Span::raw("    "),
                            Span::styled(
                                *desc,
                                if is_selected {
                                    Style::default().fg(Theme::fg())
                                } else {
                                    Style::default().fg(Theme::dim())
                                },
                            ),
                        ];

                        if !syntax.is_empty() && popup_inner_w >= 65 {
                            line2_spans.push(Span::styled("  •  ", Style::default().fg(Theme::dim())));
                            line2_spans.push(Span::styled(
                                *syntax,
                                if is_selected {
                                    Style::default().fg(Theme::yellow()).add_modifier(Modifier::BOLD)
                                } else {
                                    Style::default().fg(Theme::yellow())
                                },
                            ));
                        }

                        let line2 = Line::from(line2_spans);

                        ListItem::new(vec![line1, line2]).style(if is_selected {
                            Theme::selected_row()
                        } else {
                            Style::default()
                        })
                    }
                    CommandPopupDensity::Standard => {
                        let alias_str = if !aliases.is_empty() {
                            format!(" ({})", aliases.join(", "))
                        } else {
                            String::new()
                        };
                        let name_col = format!("{}{}{}", prefix, name, alias_str);
                        let pad_width = if popup_inner_w >= 90 { 26 } else { 20 };
                        let padded_name = format!("{:<pad_width$}", name_col, pad_width = pad_width);

                        let mut spans = vec![
                            Span::styled(
                                padded_name,
                                if is_selected {
                                    Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD)
                                } else {
                                    Style::default().fg(Theme::fg()).add_modifier(Modifier::BOLD)
                                },
                            ),
                        ];

                        if popup_inner_w >= 60 {
                            spans.push(Span::styled(
                                format!("[{}] ", cat),
                                if is_selected {
                                    Style::default().fg(Theme::accent())
                                } else {
                                    Style::default().fg(Theme::dim())
                                },
                            ));
                        }

                        spans.push(Span::styled(
                            format!(" {}", desc),
                            if is_selected {
                                Style::default().fg(Theme::fg())
                            } else {
                                Style::default().fg(Theme::dim())
                            },
                        ));

                        if !syntax.is_empty() && popup_inner_w >= 85 {
                            spans.push(Span::styled("  |  ", Style::default().fg(Theme::dim())));
                            spans.push(Span::styled(
                                *syntax,
                                if is_selected {
                                    Style::default().fg(Theme::yellow())
                                } else {
                                    Style::default().fg(Theme::dim())
                                },
                            ));
                        }

                        ListItem::new(Line::from(spans)).style(if is_selected {
                            Theme::selected_row()
                        } else {
                            Style::default()
                        })
                    }
                    CommandPopupDensity::Compact => {
                        let alias_str = if !aliases.is_empty() {
                            format!(" ({})", aliases.join(", "))
                        } else {
                            String::new()
                        };
                        let name_col = format!("{}{}{}", prefix, name, alias_str);
                        let pad_width = if popup_inner_w >= 90 { 24 } else { 18 };
                        let padded_name = format!("{:<pad_width$}", name_col, pad_width = pad_width);

                        let mut spans = vec![
                            Span::styled(
                                padded_name,
                                if is_selected {
                                    Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD)
                                } else {
                                    Style::default().fg(Theme::fg())
                                },
                            ),
                            Span::styled(
                                format!(" {}", desc),
                                if is_selected {
                                    Style::default().fg(Theme::fg())
                                } else {
                                    Style::default().fg(Theme::dim())
                                },
                            ),
                        ];

                        if !syntax.is_empty() && popup_inner_w >= 95 {
                            spans.push(Span::styled("  |  ", Style::default().fg(Theme::dim())));
                            spans.push(Span::styled(
                                *syntax,
                                if is_selected {
                                    Style::default().fg(Theme::yellow())
                                } else {
                                    Style::default().fg(Theme::dim())
                                },
                            ));
                        }

                        ListItem::new(Line::from(spans)).style(if is_selected {
                            Theme::selected_row()
                        } else {
                            Style::default()
                        })
                    }
                }
            })
            .collect();

        let title = if visible_count < config.command_popup_max_visible {
            format!(
                " Commands [1/{}] (window limited: {} of {}) ",
                SAMPLE_SUGGESTIONS.len(),
                visible_count,
                config.command_popup_max_visible
            )
        } else if SAMPLE_SUGGESTIONS.len() > visible_count {
            format!(
                " Commands [1/{}] (Tab: complete, Enter: run) ",
                SAMPLE_SUGGESTIONS.len()
            )
        } else {
            " Commands (Tab to complete, Enter to run) ".to_string()
        };

        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(Theme::border_type())
                .border_style(Style::default().fg(Theme::accent()))
                .title(title),
        );
        f.render_widget(list, popup_area);
    }
}

    // 4. Bottom Key Hints
    let hints_line = Line::from(vec![
        Span::styled("<j/k or ↑/↓> ", Style::default().fg(Theme::yellow()).add_modifier(Modifier::BOLD)),
        Span::styled("Select  ", Style::default().fg(Theme::dim())),
        Span::styled("<h/l, ←/→, -/+> ", Style::default().fg(Theme::yellow()).add_modifier(Modifier::BOLD)),
        Span::styled("Adjust/Cycle  ", Style::default().fg(Theme::dim())),
        Span::styled("<e/Enter> ", Style::default().fg(Theme::yellow()).add_modifier(Modifier::BOLD)),
        Span::styled("Edit  ", Style::default().fg(Theme::dim())),
        Span::styled("<c> ", Style::default().fg(Theme::yellow()).add_modifier(Modifier::BOLD)),
        Span::styled("Clear  ", Style::default().fg(Theme::dim())),
        Span::styled("<Space> ", Style::default().fg(Theme::yellow()).add_modifier(Modifier::BOLD)),
        Span::styled("Toggle  ", Style::default().fg(Theme::dim())),
        Span::styled("<r> ", Style::default().fg(Theme::yellow()).add_modifier(Modifier::BOLD)),
        Span::styled("Reset Defaults  ", Style::default().fg(Theme::dim())),
        Span::styled("<Esc or q> ", Style::default().fg(Theme::yellow()).add_modifier(Modifier::BOLD)),
        Span::styled("Back", Style::default().fg(Theme::dim())),
    ]);
    f.render_widget(Paragraph::new(hints_line), chunks[2]);

    // 5. Edit Modal Dialog (if currently editing field 4 or 5)
    if state.is_editing {
        let modal_area = crate::ui::help::centered_rect(65, 35, area);
        f.render_widget(Clear, modal_area);

        let title = match state.selected_field {
            4 => " Edit ArgoCD Hub Context ",
            5 => " Edit ArgoCD Hub Kubeconfig Path ",
            _ => " Edit Setting ",
        };

        let edit_block = Block::default()
            .borders(Borders::ALL)
            .border_type(ratatui::widgets::BorderType::Double)
            .border_style(Style::default().fg(Theme::cyan()))
            .title(Span::styled(
                title,
                Style::default().fg(Theme::cyan()).add_modifier(Modifier::BOLD),
            ));
        let inner = edit_block.inner(modal_area);
        f.render_widget(edit_block, modal_area);

        let v_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Prompt
                Constraint::Length(3), // Input
                Constraint::Length(1), // Hints
                Constraint::Min(0),
            ])
            .split(inner);

        let prompt_text = match state.selected_field {
            4 => "Enter context name pointing to the ArgoCD cluster (leave empty to clear):",
            5 => "Enter absolute path to the kubeconfig for ArgoCD (leave empty to clear):",
            _ => "Enter new value:",
        };
        let prompt_p = Paragraph::new(prompt_text).style(Style::default().fg(Theme::dim()));
        f.render_widget(prompt_p, v_chunks[0]);

        let chars: Vec<char> = state.edit_buffer.chars().collect();
        let pos = state.cursor_pos();
        let before: String = chars[..pos].iter().collect();
        let after: String = chars[pos..].iter().collect();

        let input_line = Line::from(vec![
            Span::styled(before, Style::default().fg(Theme::fg()).add_modifier(Modifier::BOLD)),
            Span::styled("█", Style::default().fg(Theme::cyan())),
            Span::styled(after, Style::default().fg(Theme::fg()).add_modifier(Modifier::BOLD)),
        ]);
        let input_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Theme::yellow()));

        let inner_width = v_chunks[1].width.saturating_sub(2) as usize;
        let scroll_x = if inner_width > 0 && pos >= inner_width {
            (pos + 1 - inner_width) as u16
        } else {
            0
        };

        let input_p = Paragraph::new(input_line)
            .block(input_block)
            .scroll((0, scroll_x));
        f.render_widget(input_p, v_chunks[1]);

        let hint_line = Line::from(vec![
            Span::styled("<Enter> ", Style::default().fg(Theme::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("Save  ", Style::default().fg(Theme::dim())),
            Span::styled("<Esc> ", Style::default().fg(Theme::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("Cancel  ", Style::default().fg(Theme::dim())),
            Span::styled("<Ctrl+V/Cmd+V> ", Style::default().fg(Theme::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("Paste  ", Style::default().fg(Theme::dim())),
            Span::styled("<←/→> ", Style::default().fg(Theme::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("Move  ", Style::default().fg(Theme::dim())),
            Span::styled("<Ctrl+U> ", Style::default().fg(Theme::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("Clear", Style::default().fg(Theme::dim())),
        ]);
        f.render_widget(Paragraph::new(hint_line), v_chunks[2]);
    }
}
