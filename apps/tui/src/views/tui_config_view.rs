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
    ("toolbox", &["tools"], "Diagnostic", "Toolbox diagnostics (kubectl, helm, krew plugins)", ":toolbox"),
    ("overview", &["info"], "Overview", "Cluster overview, health summary and node/pod capacity", ":overview"),
    ("quit", &["q", "exit"], "System", "Quit SRElens TUI session", ":quit"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuiConfigViewState {
    pub selected_field: usize, // 0 = Width, 1 = Visible Rows, 2 = Text Size / Density, 3 = Startup Banner
}

impl Default for TuiConfigViewState {
    fn default() -> Self {
        Self::new()
    }
}

impl TuiConfigViewState {
    pub fn new() -> Self {
        Self { selected_field: 0 }
    }

    pub fn select_next_field(&mut self) {
        self.selected_field = (self.selected_field + 1) % 4;
    }

    pub fn select_prev_field(&mut self) {
        if self.selected_field == 0 {
            self.selected_field = 3;
        } else {
            self.selected_field -= 1;
        }
    }

    pub fn adjust_current(&mut self, delta: i32, config: &mut TuiConfig) {
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
            _ => {}
        }
        let _ = config.save();
    }

    pub fn cycle_current(&mut self, config: &mut TuiConfig) {
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
            _ => {}
        }
        let _ = config.save();
    }

    pub fn reset_defaults(&mut self, config: &mut TuiConfig) {
        *config = TuiConfig::default();
        let _ = config.save();
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
            "Settings are saved automatically to ~/.config/srelens/tui.json and applied immediately.",
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
            .constraints([Constraint::Length(21), Constraint::Min(8)])
            .split(chunks[1]);
        (v_chunks[0], v_chunks[1])
    };

    // Setting cards
    let control_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5), // Width card
            Constraint::Length(5), // Visible rows card
            Constraint::Length(5), // Text size card
            Constraint::Length(5), // Startup banner card
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

    // 3. Live Preview (Feature Banner or Command Popup)
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
        Span::styled("Adjust  ", Style::default().fg(Theme::dim())),
        Span::styled("<Space or Enter> ", Style::default().fg(Theme::yellow()).add_modifier(Modifier::BOLD)),
        Span::styled("Toggle  ", Style::default().fg(Theme::dim())),
        Span::styled("<r> ", Style::default().fg(Theme::yellow()).add_modifier(Modifier::BOLD)),
        Span::styled("Reset Defaults  ", Style::default().fg(Theme::dim())),
        Span::styled("<Esc or q> ", Style::default().fg(Theme::yellow()).add_modifier(Modifier::BOLD)),
        Span::styled("Back", Style::default().fg(Theme::dim())),
    ]);
    f.render_widget(Paragraph::new(hints_line), chunks[2]);
}
