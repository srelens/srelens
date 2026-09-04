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

fn detect_tool(name: &str, alt_names: &[&str], required: bool, version_args: &[&str]) -> ToolStatusItem {
    let mut resolved_path = None;

    // Check primary name and alternatives with `which`
    for bin in std::iter::once(&name).chain(alt_names.iter()) {
        if let Ok(output) = std::process::Command::new("which").arg(bin).output() {
            if output.status.success() {
                let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path_str.is_empty() {
                    resolved_path = Some(path_str);
                    break;
                }
            }
        }
    }

    // Fallback: check standard ~/.krew/bin if checking krew
    if resolved_path.is_none() && name == "krew" {
        if let Some(home) = dirs::home_dir() {
            let candidate = home.join(".krew").join("bin").join("kubectl-krew");
            if candidate.is_file() {
                resolved_path = Some(candidate.display().to_string());
            }
        }
    }

    let installed = resolved_path.is_some();
    let version = if let Some(ref path) = resolved_path {
        std::process::Command::new(path)
            .args(version_args)
            .output()
            .ok()
            .and_then(|output| {
                if output.status.success() {
                    let text = String::from_utf8_lossy(&output.stdout);
                    text.lines().next().map(|l| l.trim().to_string())
                } else {
                    None
                }
            })
    } else {
        None
    };

    ToolStatusItem {
        name: name.to_string(),
        installed,
        version,
        path: resolved_path,
        required,
    }
}

impl ToolboxViewState {
    pub fn new() -> Self {
        let kubectl = detect_tool("kubectl", &[], true, &["version", "--client"]);
        let helm = detect_tool("helm", &[], false, &["version", "--short"]);
        let krew = detect_tool("krew", &["kubectl-krew"], false, &["version"]);

        Self {
            tools: vec![kubectl, helm, krew],
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
