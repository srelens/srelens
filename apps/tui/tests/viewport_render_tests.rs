//! Viewport rendering tests verifying that all views render cleanly without
//! panicking and with intact status lines and headers across narrow (80x24) and
//! wide (200x50) terminals.
//!
//! Dimension 7 (Visual & Perceptual Reality) in the Human QA Matrix:
//! Catches layout panics, overflow panics, and crushed status lines permanently.

mod common;

use srectl::app::ActiveView;
use srectl::commands::ResourceKind;
use srectl::ui::dialogs::Modal;
use srectl::ui::statusbar::InputMode;
use srectl::views::argo_detail_view::ArgoDetailViewState;
use srectl::views::argo_view::ArgoViewState;
use srectl::views::bgp_view::BgpViewState;
use srectl::views::changed_view::ChangedViewState;
use srectl::views::describe_view::DescribeViewState;
use srectl::views::gpu_view::GpuViewState;
use srectl::views::helm_detail_view::HelmDetailViewState;
use srectl::views::helm_view::HelmViewState;
use srectl::views::logs_view::LogsViewState;
use srectl::views::node_inspector_view::NodeInspectorState;
use srectl::views::overview_view::OverviewViewState;
use srectl::views::port_forward_view::PortForwardViewState;
use srectl::views::resource_table::ResourceTableState;
use srectl::views::settings_view::SettingsViewState;
use srectl::views::toolbox_view::ToolboxViewState;
use srectl::views::top_view::{TopTab, TopViewState};
use srectl::views::topology_view::TopologyViewState;
use srectl::views::tree_view::TreeViewState;
use srectl::views::tui_config_view::TuiConfigViewState;
use srectl::views::yaml_view::YamlViewState;

const NARROW: (u16, u16) = (80, 24);
const WIDE: (u16, u16) = (200, 50);

#[tokio::test]
async fn test_all_views_render_narrow_and_wide_without_panic() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;

    let views = vec![
        (
            "Table",
            ActiveView::Table(ResourceTableState::new(ResourceKind::Pods)),
        ),
        (
            "Yaml",
            ActiveView::Yaml(YamlViewState::new(
                "demo-pod".into(),
                "Pod".into(),
                Some("default".into()),
                "apiVersion: v1\nkind: Pod\nmetadata:\n  name: demo-pod\n".into(),
            )),
        ),
        (
            "Describe",
            ActiveView::Describe(DescribeViewState::new(
                "demo-pod".into(),
                "Pod".into(),
                Some("default".into()),
                "Name: demo-pod\nNamespace: default\nStatus: Running\n".into(),
            )),
        ),
        (
            "Logs",
            ActiveView::Logs(LogsViewState::new(
                "demo-pod".into(),
                "default".into(),
                Some("main".into()),
                "demo-stream".into(),
            )),
        ),
        (
            "PortForwards",
            ActiveView::PortForwards(PortForwardViewState::new()),
        ),
        ("Helm", ActiveView::Helm(HelmViewState::new())),
        (
            "HelmDetail",
            ActiveView::HelmDetail(HelmDetailViewState::new(
                "demo-release".into(),
                "default".into(),
            )),
        ),
        ("Argo", ActiveView::Argo(ArgoViewState::new())),
        (
            "ArgoDetail",
            ActiveView::ArgoDetail(ArgoDetailViewState::new(
                "demo-app".into(),
                "argocd".into(),
                None,
            )),
        ),
        ("Overview", ActiveView::Overview(OverviewViewState::new())),
        ("Toolbox", ActiveView::Toolbox(ToolboxViewState::new())),
        ("Assistant", ActiveView::Assistant),
        ("Settings", ActiveView::Settings(SettingsViewState::new())),
        (
            "TuiConfig",
            ActiveView::TuiConfig(TuiConfigViewState::new()),
        ),
        (
            "Tree",
            ActiveView::Tree(TreeViewState::new(
                "Pod".into(),
                "demo-pod".into(),
                Some("default".into()),
            )),
        ),
        (
            "NodeInspector",
            ActiveView::NodeInspector(NodeInspectorState::new("node-1".into())),
        ),
        (
            "Topology",
            ActiveView::Topology(TopologyViewState::new(vec!["default".into()])),
        ),
        ("GpuInfo", ActiveView::GpuInfo(GpuViewState::new())),
        ("Bgp", ActiveView::Bgp(BgpViewState::new())),
        ("Top", ActiveView::Top(TopViewState::new(TopTab::Pods))),
        ("Changed", ActiveView::Changed(ChangedViewState::new())),
    ];

    for (name, view) in views {
        app.active_view = view;

        // Narrow viewport: 80x24 standard terminal
        let narrow_lines = common::render_lines(NARROW.0, NARROW.1, |f| app.render(f));
        assert_eq!(
            narrow_lines.len(),
            NARROW.1 as usize,
            "view {name} narrow height mismatch"
        );
        // Header line must contain brand and context info
        assert!(
            narrow_lines[0].contains("SRELENS"),
            "view {name} top header missing SRELENS brand at 80x24"
        );
        // Status bar content row (row height-1, independent of top divider) must contain key hints
        let narrow_status_row = &narrow_lines[NARROW.1 as usize - 1];
        assert!(
            !narrow_status_row.trim().is_empty() && narrow_status_row.contains('<'),
            "view {name} status content row was blank or missing key hints at 80x24: '{narrow_status_row}'"
        );

        // Wide viewport: 200x50 wide terminal
        let wide_lines = common::render_lines(WIDE.0, WIDE.1, |f| app.render(f));
        assert_eq!(
            wide_lines.len(),
            WIDE.1 as usize,
            "view {name} wide height mismatch"
        );
        assert!(
            wide_lines[0].contains("SRELENS"),
            "view {name} top header missing SRELENS brand at 200x50"
        );
        let wide_status_row = &wide_lines[WIDE.1 as usize - 1];
        assert!(
            !wide_status_row.trim().is_empty() && wide_status_row.contains('<'),
            "view {name} status content row was blank or missing key hints at 200x50: '{wide_status_row}'"
        );
    }
}

#[tokio::test]
async fn test_modals_and_overlays_render_narrow_and_wide_without_panic() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;

    // 1. Help Modal overlay
    app.show_help = true;
    for &(w, h) in &[NARROW, WIDE] {
        let lines = common::render_lines(w, h, |f| app.render(f));
        assert_eq!(lines.len(), h as usize);
        assert!(lines
            .iter()
            .any(|l| l.contains("Help") || l.contains("Keybindings")));
    }
    app.show_help = false;

    // 2. Command Popup overlay: assert popup-only box and suggestion list rendered above status bar
    app.input_mode = InputMode::Command;
    app.command_buffer = ":po".to_string();
    for &(w, h) in &[NARROW, WIDE] {
        let lines = common::render_lines(w, h, |f| app.render(f));
        assert_eq!(lines.len(), h as usize);
        // Must find popup header and suggestion content, not just the echoed input line
        assert!(
            lines.iter().any(|l| l.contains("Commands (Tab to complete")
                || l.contains("▶ pods")
                || l.contains("▶")),
            "Command suggestion popup not rendered at {w}x{h}"
        );
    }
    app.input_mode = InputMode::Normal;
    app.command_buffer.clear();

    // 3. Confirm Modal
    app.modal = Some(Modal::Confirm {
        title: "Delete Pod".into(),
        message: "Are you sure you want to delete pod test-pod?".into(),
        action_name: "Delete".into(),
        is_destructive: true,
    });
    for &(w, h) in &[NARROW, WIDE] {
        let lines = common::render_lines(w, h, |f| app.render(f));
        assert_eq!(lines.len(), h as usize);
        assert!(lines.iter().any(|l| l.contains("Delete Pod")));
    }

    // 4. Add Cluster Modal: assert modal-specific title and instruction content
    app.modal = Some(Modal::AddCluster {
        input: "server: https://127.0.0.1:6443".into(),
        cursor_pos: 0,
        error_message: None,
        preview_contexts: vec![],
    });
    for &(w, h) in &[NARROW, WIDE] {
        let lines = common::render_lines(w, h, |f| app.render(f));
        assert_eq!(lines.len(), h as usize);
        assert!(
            lines.iter().any(|l| l.contains("Kubeconfig")
                || l.contains("127.0.0.1:6443")
                || l.contains("Import Cluster")),
            "AddCluster modal content not rendered at {w}x{h}"
        );
    }
    app.modal = None;
}
