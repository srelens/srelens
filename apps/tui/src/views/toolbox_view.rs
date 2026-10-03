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
    // The lookup Helm operations and the toolbox diagnosis use too, so the
    // TUI cannot disagree with them about what is installed.
    let mut resolved_path = std::iter::once(&name)
        .chain(alt_names.iter())
        .find_map(|bin| srelens_kube::path_lookup::find_on_path(bin))
        .map(|path| path.display().to_string());

    // Fallback: check standard ~/.krew/bin if checking krew
    if resolved_path.is_none() && name == "krew" {
        resolved_path = dirs::home_dir().and_then(|home| krew_fallback(&home));
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

/// krew's shim in its own bin directory under `home`, which krew's installer
/// does not put on `PATH` for you. `kubectl-krew.exe` on Windows.
fn krew_fallback(home: &std::path::Path) -> Option<String> {
    srelens_kube::path_lookup::find_executable("kubectl-krew", home.join(".krew").join("bin"))
        .map(|path| path.display().to_string())
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

    let mut max_tool = "TOOL".len();
    let mut max_status = "STATUS".len();
    let mut max_version = "VERSION".len();
    let mut max_path = "PATH".len();

    for tool in &state.tools {
        max_tool = max_tool.max(tool.name.len());
        let status_len = if tool.installed {
            "● INSTALLED".chars().count()
        } else if tool.required {
            "● MISSING (REQUIRED)".chars().count()
        } else {
            "○ NOT INSTALLED".chars().count()
        };
        max_status = max_status.max(status_len);
        max_version = max_version.max(tool.version.as_deref().unwrap_or("-").len());
        max_path = max_path.max(tool.path.as_deref().unwrap_or("-").len());
    }

    let widths = [
        Constraint::Length((max_tool + 1) as u16),
        Constraint::Length((max_status + 1) as u16),
        Constraint::Length((max_version + 1) as u16),
        Constraint::Min((max_path + 1) as u16),
    ];

    let table = Table::new(rows, widths).header(headers);
    f.render_widget(table, inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn krew_is_found_in_its_own_bin_dir_under_the_platform_name() {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join(".krew").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        // `kubectl-krew.exe` on Windows.
        let krew = bin.join(format!("kubectl-krew{}", std::env::consts::EXE_SUFFIX));
        std::fs::write(&krew, b"").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&krew, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        assert_eq!(krew_fallback(home.path()), Some(krew.display().to_string()));
    }

    #[test]
    fn test_toolbox_view_state_and_render() {
        let mut state = ToolboxViewState::new();
        assert!(!state.tools.is_empty());

        // Test navigation
        state.select_next();
        state.select_prev();
        assert_eq!(state.selected_idx, 0);

        // Test mock tools to cover all status branches
        state.tools = vec![
            ToolStatusItem {
                name: "kubectl".to_string(),
                installed: true,
                version: Some("v1.30.0".to_string()),
                path: Some("/usr/local/bin/kubectl".to_string()),
                required: true,
            },
            ToolStatusItem {
                name: "required-tool".to_string(),
                installed: false,
                version: None,
                path: None,
                required: true,
            },
            ToolStatusItem {
                name: "optional-tool".to_string(),
                installed: false,
                version: None,
                path: None,
                required: false,
            },
        ];

        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render_toolbox_view(f, f.area(), &state)).unwrap();

        // Nonexistent tool detection to test failure branches
        let missing = detect_tool("definitely-nonexistent-bin-987", &["also-nonexistent"], false, &["--version"]);
        assert!(!missing.installed);
        assert!(missing.path.is_none());
        assert!(missing.version.is_none());
    }
}
