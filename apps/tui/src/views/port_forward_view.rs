use ratatui::{
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
    Frame,
};
use serde::{Deserialize, Serialize};

use crate::theme::Theme;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortForwardEntry {
    pub id: String,
    pub context: String,
    pub namespace: String,
    pub target_type: String,
    pub target_name: String,
    pub local_port: u16,
    pub container_port: u16,
    pub active_connections: u64,
    pub bytes_rx: u64,
    pub bytes_tx: u64,
    pub status: String,
}

pub struct PortForwardViewState {
    pub forwards: Vec<PortForwardEntry>,
    pub selected_idx: usize,
}

impl PortForwardViewState {
    pub fn new() -> Self {
        Self {
            forwards: Vec::new(),
            selected_idx: 0,
        }
    }

    pub fn set_forwards(&mut self, forwards: Vec<PortForwardEntry>) {
        self.forwards = forwards;
        if self.selected_idx >= self.forwards.len() {
            self.selected_idx = self.forwards.len().saturating_sub(1);
        }
    }

    pub fn select_next(&mut self) {
        if !self.forwards.is_empty() && self.selected_idx + 1 < self.forwards.len() {
            self.selected_idx += 1;
        }
    }

    pub fn select_prev(&mut self) {
        if self.selected_idx > 0 {
            self.selected_idx -= 1;
        }
    }

    pub fn selected_forward(&self) -> Option<&PortForwardEntry> {
        self.forwards.get(self.selected_idx)
    }
}

pub fn render_port_forward_view(f: &mut Frame, area: Rect, state: &PortForwardViewState) {
    let title = format!(" Active Port Forwards [{}] (<d> Stop Forward <shift-f> New Forward <Esc> Back) ", state.forwards.len());

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::BORDER))
        .title(Span::styled(title, Theme::title()));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if state.forwards.is_empty() {
        let empty_msg = Paragraph::new(
            "No active port forwards. Select a Pod or Service and press <shift-f> to start a port forward.",
        )
        .style(Style::default().fg(Theme::DIM));
        f.render_widget(empty_msg, inner);
        return;
    }

    let headers = Row::new(vec![
        Cell::from("STATUS").style(Theme::table_header()),
        Cell::from("LOCAL PORT").style(Theme::table_header()),
        Cell::from("NAMESPACE").style(Theme::table_header()),
        Cell::from("TARGET").style(Theme::table_header()),
        Cell::from("REMOTE PORT").style(Theme::table_header()),
        Cell::from("CONNS").style(Theme::table_header()),
        Cell::from("RX / TX").style(Theme::table_header()),
    ])
    .height(1)
    .bottom_margin(1);

    let rows: Vec<Row> = state
        .forwards
        .iter()
        .enumerate()
        .map(|(i, pf)| {
            let is_selected = i == state.selected_idx;
            let status_style = if pf.status == "active" || pf.status == "running" {
                Theme::status_ok()
            } else {
                Theme::status_warn()
            };

            let row_style = if is_selected {
                Theme::selected_row()
            } else {
                Style::default()
            };

            Row::new(vec![
                Cell::from(format!("● {}", pf.status)).style(status_style),
                Cell::from(format!("127.0.0.1:{}", pf.local_port)),
                Cell::from(pf.namespace.as_str()),
                Cell::from(format!("{}/{}", pf.target_type, pf.target_name)),
                Cell::from(pf.container_port.to_string()),
                Cell::from(pf.active_connections.to_string()),
                Cell::from(format!("{} / {}", format_bytes(pf.bytes_rx), format_bytes(pf.bytes_tx))),
            ])
            .style(row_style)
        })
        .collect();

    let widths = [
        Constraint::Length(12),
        Constraint::Length(18),
        Constraint::Length(18),
        Constraint::Min(25),
        Constraint::Length(14),
        Constraint::Length(10),
        Constraint::Length(18),
    ];

    let table = Table::new(rows, widths).header(headers);
    f.render_widget(table, inner);
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{}B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1}KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1}MB", bytes as f64 / (1024.0 * 1024.0))
    }
}
