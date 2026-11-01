//! The notification history overlay.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::notifications::NotificationCenter;
use crate::theme::Theme;
use crate::ui::help::centered_rect;

fn ago(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s ago"),
        60..=3599 => format!("{}m ago", secs / 60),
        _ => format!("{}h ago", secs / 3600),
    }
}

pub fn render_notifications_modal(f: &mut Frame, area: Rect, center: &NotificationCenter) {
    let modal_area = centered_rect(80, 80, area);
    f.render_widget(Clear, modal_area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(Theme::border_type())
        .border_style(Style::default().fg(Theme::accent()))
        .title(" Notifications  (c clear, Esc close) ");
    let inner = block.inner(modal_area);
    f.render_widget(block, modal_area);

    let mut lines: Vec<Line> = Vec::new();
    for n in center.items().take(inner.height as usize) {
        let mut spans = vec![
            Span::styled(n.level.label(), n.level.style()),
            Span::raw(" "),
            Span::styled(
                format!("{:>8}", ago(n.at.elapsed().as_secs())),
                Theme::status_dim(),
            ),
            Span::raw("  "),
            Span::styled(n.source.clone(), Theme::status_dim()),
            Span::raw("  "),
            Span::styled(n.message.clone(), Style::default().add_modifier(Modifier::BOLD)),
        ];
        if n.count > 1 {
            spans.push(Span::styled(format!("  x{}", n.count), Theme::status_dim()));
        }
        lines.push(Line::from(spans));
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "Nothing to report. Failing pods, restarts and errors will appear here.",
            Theme::status_dim(),
        )));
    }
    f.render_widget(Paragraph::new(lines), inner);
}
