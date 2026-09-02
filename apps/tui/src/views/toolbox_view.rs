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
pub struct ToolStatusItem {
    pub name: String,
    pub installed: bool,
    pub version: Option<String>,
    pub path: Option<String>,
    pub required: bool,
}

pub struct ToolboxViewState {
    pub tools: Vec<ToolStatusItem>,
    pub selected_idx: usize,
}

impl ToolboxViewState {
    pub fn new() -> Self {
        Self {
            tools: vec![
                ToolStatusItem {
                    name: "kubectl".to_string(),
                    installed: true,
                    version: Some("v1.31.0".to_string()),
                    path: Some("/usr/local/bin/kubectl".to_string()),
                    required: true,
                },
                ToolStatusItem {
                    name: "helm".to_string(),
                    installed: true,
                    version: Some("v3.15.0".to_string()),
                    path: Some("/usr/local/bin/helm".to_string()),
                    required: false,
                },
                ToolStatusItem {
                    name: "krew".to_string(),
                    installed: true,
                    version: Some("v0.4.4".to_string()),
                    path: Some("/Users/skatara/.krew/bin/kubectl-krew".to_string()),
                    required: false,
                },
            ],
            selected_idx: 0,
        }
    }

    pub fn select_next(&mut self) {
        if !self.tools.is_empty() && self.selected_idx + 1 < self.tools.len() {
            self.selected_idx += 1;
        }
    }

    pub fn select_prev(&mut self) {
        if self.selected_idx > 0 {
            self.selected_idx -= 1;
        }
    }
}

pub fn render_toolbox_view(f: &mut Frame, area: Rect, state: &ToolboxViewState) {
    let title = " SRElens Toolbox & CLI Environment Diagnostics (<Esc> Back) ";

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::BORDER))
        .title(Span::styled(title, Theme::title()));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let headers = Row::new(vec![
        Cell::from("TOOL").style(Theme::table_header()),
        Cell::from("STATUS").style(Theme::table_header()),
        Cell::from("VERSION").style(Theme::table_header()),
        Cell::from("PATH").style(Theme::table_header()),
    ])
    .height(1)
    .bottom_margin(1);

    let rows: Vec<Row> = state
        .tools
        .iter()
        .enumerate()
        .map(|(i, tool)| {
            let is_selected = i == state.selected_idx;
            let (status_text, status_style) = if tool.installed {
                ("● INSTALLED", Theme::status_ok())
            } else if tool.required {
                ("● MISSING (REQUIRED)", Theme::status_error())
            } else {
                ("○ NOT INSTALLED", Theme::status_warn())
            };

            let row_style = if is_selected {
                Theme::selected_row()
            } else {
                Style::default()
            };

            Row::new(vec![
                Cell::from(tool.name.as_str()),
                Cell::from(status_text).style(status_style),
                Cell::from(tool.version.as_deref().unwrap_or("-")),
                Cell::from(tool.path.as_deref().unwrap_or("-")),
            ])
            .style(row_style)
        })
        .collect();

    let widths = [
        Constraint::Length(15),
        Constraint::Length(25),
        Constraint::Length(16),
        Constraint::Min(35),
    ];

    let table = Table::new(rows, widths).header(headers);
    f.render_widget(table, inner);
}
