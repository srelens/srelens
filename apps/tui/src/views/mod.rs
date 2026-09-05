pub mod assistant_view;
pub mod cracked_lens;
pub mod describe_view;
pub mod exec_view;
pub mod helm_view;
pub mod logs_view;
pub mod metrics_panel_view;
pub mod overview_view;
pub mod port_forward_view;
pub mod reason_rail;
pub mod resource_table;
pub mod settings_view;
pub mod toolbox_view;
pub mod tree_view;
pub mod node_inspector_view;
pub mod yaml_view;

/// Strip everything from cluster-controlled text that would desynchronise
/// ratatui's buffer from the real terminal. A raw `\t` written into a cell
/// makes the terminal cursor jump to the next tab stop while ratatui counts
/// one column; from then on its screen model is wrong and later diffs leave
/// ghost text behind (tab-delimited istio-proxy logs trigger exactly this).
/// Tabs are expanded to 8-column stops, ANSI escape sequences are dropped,
/// embedded newlines become spaces (cells are one line tall), and remaining
/// control characters are removed. Applies to any Span/Cell text that comes
/// from the cluster: log lines, event messages, table cells, describe/YAML.
pub fn sanitize_span_text(text: &str) -> String {
    // Fast path: plain text passes through with a single copy.
    if !text.chars().any(|c| c.is_control()) {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut col = 0usize;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\t' => {
                let pad = 8 - (col % 8);
                for _ in 0..pad {
                    out.push(' ');
                }
                col += pad;
            }
            '\n' => {
                out.push(' ');
                col += 1;
            }
            '\u{1b}' => {
                // CSI: ESC '[' parameters… final byte in @..=~. Other escape
                // sequences (OSC aside, rare in logs) are ESC + one byte.
                if chars.peek() == Some(&'[') {
                    chars.next();
                    for e in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&e) {
                            break;
                        }
                    }
                } else {
                    chars.next();
                }
            }
            c if c.is_control() => {}
            c => {
                out.push(c);
                col += 1;
            }
        }
    }
    out
}

use ratatui::style::Style;
use ratatui::text::Span;

/// Splits `text` into Spans, highlighting every case-insensitive occurrence of `query`
/// with `match_style` while styling non-matching portions with `base_style`.
pub fn highlight_text_matches<'a>(
    text: &'a str,
    query: &str,
    base_style: Style,
    match_style: Style,
) -> Vec<Span<'a>> {
    if query.is_empty() {
        return vec![Span::styled(text.to_string(), base_style)];
    }
    let query_lower = query.to_lowercase();
    let text_lower = text.to_lowercase();
    let mut spans = Vec::new();
    let mut last_idx = 0;

    for (match_start, _) in text_lower.match_indices(&query_lower) {
        if match_start > last_idx {
            spans.push(Span::styled(text[last_idx..match_start].to_string(), base_style));
        }
        let match_end = (match_start + query.len()).min(text.len());
        spans.push(Span::styled(text[match_start..match_end].to_string(), match_style));
        last_idx = match_end;
    }

    if last_idx < text.len() {
        spans.push(Span::styled(text[last_idx..].to_string(), base_style));
    }

    if spans.is_empty() {
        vec![Span::styled(text.to_string(), base_style)]
    } else {
        spans
    }
}

pub use assistant_view::{render_assistant_view, AssistantViewState};
pub use cracked_lens::render_cracked_lens;
pub use describe_view::{render_describe_view, DescribeViewState};
pub use exec_view::ExecRunner;
pub use helm_view::{render_helm_view, HelmViewState};
pub use logs_view::{render_logs_view, LogsViewState};
pub use metrics_panel_view::{render_metrics_panel_modal, MetricsPanelState, MetricsTimeRange};
pub use overview_view::{render_overview_view, OverviewViewState};
pub use port_forward_view::{render_port_forward_view, PortForwardViewState};
pub use reason_rail::{render_reason_rail_modal, render_reason_rail_widget, tally_event_reasons, ReasonTally};
pub use resource_table::{render_resource_table, ResourceTableState};
pub use settings_view::{render_settings_view, SettingField, SettingsViewState};
pub use toolbox_view::{render_toolbox_view, ToolboxViewState};
pub use tree_view::{render_tree_view, TreeViewState};
pub use node_inspector_view::{render_node_inspector_view, NodeInspectorState};
pub use yaml_view::{render_yaml_view, YamlViewState};
