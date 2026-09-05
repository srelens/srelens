use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::theme::Theme;

/// A single distinct event reason, its occurrence count (number of events), and its event type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReasonTally {
    pub reason: String,
    pub count: usize,
    pub event_type: String,
}

/// Tallies all distinct reasons across Kubernetes events, ordered by frequency (most frequent first).
///
/// **The count is the number of event objects, NOT the sum of their `count` fields**,
/// strictly adhering to the SRElens GUI §8 specification.
pub fn tally_event_reasons(items: &[Value]) -> Vec<ReasonTally> {
    let mut map: std::collections::HashMap<String, (usize, String)> = std::collections::HashMap::new();
    let mut order: Vec<String> = Vec::new();

    for item in items {
        let reason = item
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown")
            .trim();
        if reason.is_empty() {
            continue;
        }

        let event_type = item
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("Normal")
            .to_string();

        if let Some(entry) = map.get_mut(reason) {
            entry.0 += 1;
        } else {
            order.push(reason.to_string());
            map.insert(reason.to_string(), (1, event_type));
        }
    }

    let mut tallies: Vec<ReasonTally> = order
        .into_iter()
        .filter_map(|reason| {
            let (count, event_type) = map.remove(&reason)?;
            Some(ReasonTally {
                reason,
                count,
                event_type,
            })
        })
        .collect();

    // Stable sort descending by event count
    tallies.sort_by(|a, b| b.count.cmp(&a.count));
    tallies
}

/// Renders the Reason Rail panel (used as an inline sidebar or inside a modal).
pub fn render_reason_rail_widget(
    f: &mut Frame,
    area: Rect,
    tallies: &[ReasonTally],
    selected_idx: usize,
    is_focused: bool,
    active_filter: Option<&str>,
) {
    let border_color = if is_focused {
        Theme::CYAN
    } else {
        Theme::BORDER
    };

    let title = if is_focused {
        " ⚡ Event Reasons [↑/↓ Navigate, Enter Pick, Esc Back] "
    } else {
        " ⚡ Event Reasons [Tab/<R> Focus] "
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(
            title,
            if is_focused {
                Style::default().fg(Theme::CYAN).add_modifier(Modifier::BOLD)
            } else {
                Theme::title()
            },
        ));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if tallies.is_empty() {
        let p = Paragraph::new(Line::from(Span::styled(
            "No events in scope",
            Style::default().fg(Theme::DIM),
        )))
        .alignment(Alignment::Center);
        f.render_widget(p, inner);
        return;
    }

    // Scroll calculation
    let visible_height = inner.height as usize;
    let scroll_offset = if selected_idx >= visible_height {
        selected_idx.saturating_sub(visible_height / 2)
    } else {
        0
    };

    let mut lines = Vec::new();
    for (i, tally) in tallies.iter().enumerate().skip(scroll_offset).take(visible_height) {
        let is_selected = is_focused && i == selected_idx;
        let is_active = active_filter == Some(tally.reason.as_str());

        let is_warning = tally.event_type.eq_ignore_ascii_case("Warning");
        let dot_color = if is_warning {
            Theme::YELLOW
        } else {
            Theme::GREEN
        };

        let dot_symbol = if is_warning { "● " } else { "○ " };

        let cursor = if is_selected { "▸ " } else { "  " };

        let reason_style = if is_active {
            Style::default()
                .fg(Theme::CYAN)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else if is_selected {
            Style::default().fg(Theme::SEL_FG).add_modifier(Modifier::BOLD)
        } else if is_warning {
            Style::default().fg(Theme::YELLOW)
        } else {
            Style::default().fg(Theme::FG)
        };

        let mut spans = vec![
            Span::styled(cursor, Style::default().fg(if is_selected { Theme::CYAN } else { Theme::DIM })),
            Span::styled(dot_symbol, Style::default().fg(dot_color).add_modifier(Modifier::BOLD)),
            Span::styled(tally.reason.clone(), reason_style),
        ];

        let count_str = format!(" ({})", tally.count);
        let count_style = if is_selected {
            Style::default().fg(Theme::SEL_FG).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Theme::DIM)
        };
        spans.push(Span::styled(count_str, count_style));

        if is_active {
            spans.push(Span::styled(" ✔", Style::default().fg(Theme::CYAN).add_modifier(Modifier::BOLD)));
        }

        let line = if is_selected {
            Line::from(spans).style(Theme::selected_row())
        } else {
            Line::from(spans)
        };

        lines.push(line);
    }

    f.render_widget(Paragraph::new(lines), inner);
}

/// Renders the Reason Rail as a centered popup modal on narrow screens.
pub fn render_reason_rail_modal(
    f: &mut Frame,
    area: Rect,
    tallies: &[ReasonTally],
    selected_idx: usize,
    active_filter: Option<&str>,
) {
    let modal_w = 60.min(area.width.saturating_sub(4));
    let modal_h = 20.min(area.height.saturating_sub(4));

    let h_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length((area.width.saturating_sub(modal_w)) / 2),
            Constraint::Length(modal_w),
            Constraint::Length((area.width.saturating_sub(modal_w)) / 2),
        ])
        .split(area);

    let v_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length((area.height.saturating_sub(modal_h)) / 2),
            Constraint::Length(modal_h),
            Constraint::Length((area.height.saturating_sub(modal_h)) / 2),
        ])
        .split(h_chunks[1]);

    let modal_area = v_chunks[1];
    f.render_widget(Clear, modal_area);
    render_reason_rail_widget(f, modal_area, tallies, selected_idx, true, active_filter);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_tally_event_reasons_counts_objects_not_repeat_count() {
        // Spec §8 rule: count number of event items carrying that reason, NOT the sum of repeat counts
        let events = vec![
            json!({
                "reason": "FailedScheduling",
                "count": 450, // high repeat count
                "type": "Warning"
            }),
            json!({
                "reason": "FailedScheduling",
                "count": 1,
                "type": "Warning"
            }),
            json!({
                "reason": "OOMKilled",
                "count": 1,
                "type": "Warning"
            }),
            json!({
                "reason": "BackOff",
                "count": 9999, // high repeat count
                "type": "Warning"
            }),
            json!({
                "reason": "BackOff",
                "count": 2,
                "type": "Warning"
            }),
            json!({
                "reason": "BackOff",
                "count": 1,
                "type": "Warning"
            }),
        ];

        let tallies = tally_event_reasons(&events);
        assert_eq!(tallies.len(), 3);

        // BackOff has 3 event items -> highest frequency
        assert_eq!(tallies[0].reason, "BackOff");
        assert_eq!(tallies[0].count, 3);
        assert_eq!(tallies[0].event_type, "Warning");

        // FailedScheduling has 2 event items
        assert_eq!(tallies[1].reason, "FailedScheduling");
        assert_eq!(tallies[1].count, 2);

        // OOMKilled has 1 event item
        assert_eq!(tallies[2].reason, "OOMKilled");
        assert_eq!(tallies[2].count, 1);
    }

    #[test]
    fn test_tally_event_reasons_empty() {
        let events = vec![];
        let tallies = tally_event_reasons(&events);
        assert!(tallies.is_empty());
    }
}
