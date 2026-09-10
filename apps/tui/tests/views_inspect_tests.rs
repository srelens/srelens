//! Integration tests for the four "inspect something" views: the node
//! inspector, the settings screen, the cluster overview and the YAML viewer.
//!
//! Every test builds a state through its public constructors and mutators,
//! asserts the state, then renders it through ratatui's `TestBackend` at a
//! wide and a narrow size and asserts on text that only the intended branch
//! could have drawn. Nothing here touches a cluster, the network, the user's
//! config file or an editor.

mod common;

use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::Color;
use ratatui::{Frame, Terminal};

use srelens_kube::node_inspector::{
    NodeConditionInfo, NodeInspectorDetails, NodePodItem, NodeTaintInfo,
};
use srelens_tui::ai_config::{AiProvider, AiSettings};
use srelens_tui::theme::Theme;
use srelens_tui::views::node_inspector_view::{render_node_inspector_view, NodeInspectorState};
use srelens_tui::views::overview_view::{
    render_overview_view, ClusterOverviewData, OverviewViewState,
};
use srelens_tui::views::settings_view::{render_settings_view, SettingField, SettingsViewState};
use srelens_tui::views::tui_config_view::{render_tui_config_view, TuiConfigViewState};
use srelens_tui::views::yaml_view::{render_yaml_view, YamlViewState};
use srelens_tui::TuiConfig;

/// Render one frame and hand back the raw buffer, for the few assertions that
/// need a cell's style rather than its text.
fn render_buffer<F>(width: u16, height: u16, draw: F) -> Buffer
where
    F: FnOnce(&mut Frame),
{
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal.draw(draw).expect("draw");
    terminal.backend().buffer().clone()
}

/// The screen column at which `needle` starts in a rendered row. `str::find`
/// gives a byte offset, which drifts from the column as soon as a row holds
/// a box-drawing or bullet character.
fn col(line: &str, needle: &str) -> u16 {
    let byte = line
        .find(needle)
        .unwrap_or_else(|| panic!("no {needle:?} in {line:?}"));
    line[..byte].chars().count() as u16
}

/// The row index of the first rendered line containing `needle`.
fn row_of(lines: &[String], needle: &str) -> usize {
    lines
        .iter()
        .position(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("no row contains {needle:?}:\n{}", lines.join("\n")))
}

// ───────────────────────────── node inspector ─────────────────────────────

fn pod(namespace: &str, name: &str, phase: &str) -> NodePodItem {
    NodePodItem {
        name: name.to_string(),
        namespace: namespace.to_string(),
        phase: phase.to_string(),
        ready_containers: "1/1".to_string(),
        restarts: 0,
        age: "3d".to_string(),
        cpu_requests_millicores: 100,
        mem_requests_mib: 128,
        gpu_requests: 0,
        gpu_mem_requests_mib: 0,
        pod_ip: "10.244.1.5".to_string(),
    }
}

fn node_details(name: &str) -> NodeInspectorDetails {
    NodeInspectorDetails {
        name: name.to_string(),
        status: "Ready".to_string(),
        unschedulable: false,
        roles: "worker".to_string(),
        instance_type: "m5.xlarge".to_string(),
        zone: None,
        region: None,
        nodepool: None,
        internal_ip: None,
        external_ip: None,
        os_image: "Ubuntu 22.04".to_string(),
        kernel_version: "5.15.0".to_string(),
        container_runtime: "containerd://1.7".to_string(),
        kubelet_version: "v1.30.2".to_string(),
        architecture: "amd64".to_string(),
        created_at: "2026-01-01T00:00:00Z".to_string(),
        cpu_capacity_millicores: 8000,
        cpu_allocatable_millicores: 7800,
        cpu_requests_millicores: 2500,
        mem_capacity_mib: 32768,
        mem_allocatable_mib: 31000,
        mem_requests_mib: 14000,
        pods_capacity: 110,
        pods_allocatable: 110,
        pods_count: 2,
        has_gpu: false,
        gpu_model: None,
        gpu_driver_version: None,
        gpu_cuda_version: None,
        gpu_capacity_count: 0,
        gpu_allocatable_count: 0,
        gpu_requests_count: 0,
        gpu_memory_total_mib: None,
        gpu_memory_requests_mib: 0,
        conditions: vec![NodeConditionInfo {
            type_: "Ready".to_string(),
            status: "True".to_string(),
            reason: None,
            message: None,
        }],
        taints: vec![],
        pods: vec![
            pod("kube-system", "coredns-abc", "Running"),
            pod("monitoring", "node-exporter", "Running"),
        ],
    }
}

fn node_state(details: NodeInspectorDetails) -> NodeInspectorState {
    let mut state = NodeInspectorState::new(details.name.clone());
    state.set_details(details);
    state
}

fn render_node(width: u16, height: u16, state: &NodeInspectorState) -> String {
    common::render_text(width, height, |f| {
        render_node_inspector_view(f, f.area(), state)
    })
}

#[test]
fn node_inspector_starts_loading_and_draws_the_loading_placeholder() {
    let state = NodeInspectorState::new("node-1".to_string());
    assert!(state.is_loading);
    assert!(state.details.is_none());
    assert_eq!(state.pods_len(), 0);
    assert!(state.selected_pod().is_none());

    let wide = render_node(120, 40, &state);
    assert!(wide.contains("Node Inspector: node-1"), "{wide}");
    assert!(wide.contains("Inspecting node 'node-1'..."), "{wide}");
    assert!(wide.contains("Fetching hardware capacity"), "{wide}");
    assert!(!wide.contains("Scheduled Pods"), "{wide}");

    let narrow = render_node(60, 20, &state);
    assert!(narrow.contains("Node Inspector: node-1"), "{narrow}");
    assert!(narrow.contains("Inspecting node 'node-1'"), "{narrow}");
}

#[test]
fn node_inspector_error_without_details_draws_the_error_panel_with_retry_hint() {
    let mut state = NodeInspectorState::new("node-1".to_string());
    state.set_error("nodes \"node-1\" not found".to_string());
    assert!(!state.is_loading);
    assert_eq!(state.error.as_deref(), Some("nodes \"node-1\" not found"));

    let text = render_node(120, 40, &state);
    assert!(text.contains("Node Inspector Error: node-1"), "{text}");
    assert!(text.contains("Failed to inspect node 'node-1':"), "{text}");
    assert!(text.contains("nodes \"node-1\" not found"), "{text}");
    assert!(
        text.contains("Press <Esc> or <q> to return, or <r> to retry."),
        "{text}"
    );
    assert!(!text.contains("Scheduled Pods"), "{text}");
}

#[test]
fn node_inspector_error_after_details_keeps_showing_the_dashboard() {
    let mut state = node_state(node_details("node-1"));
    state.set_error("refresh failed".to_string());
    assert!(state.details.is_some());

    let text = render_node(120, 40, &state);
    assert!(!text.contains("Node Inspector Error"), "{text}");
    assert!(text.contains("Scheduled Pods (2 Total)"), "{text}");
}

#[test]
fn node_inspector_not_loading_with_no_details_and_no_error_draws_nothing() {
    let mut state = NodeInspectorState::new("node-1".to_string());
    state.is_loading = false;
    let lines = common::render_lines(60, 20, |f| render_node_inspector_view(f, f.area(), &state));
    assert!(lines.iter().all(|l| l.is_empty()), "{lines:?}");
}

#[test]
fn node_inspector_set_details_clears_error_and_clamps_the_selection() {
    let mut state = NodeInspectorState::new("node-1".to_string());
    state.set_error("boom".to_string());
    state.selected_pod_idx = 7;
    state.set_details(node_details("node-1"));
    assert!(state.error.is_none());
    assert!(!state.is_loading);
    assert_eq!(state.pods_len(), 2);
    assert_eq!(state.selected_pod_idx, 1);
    assert_eq!(state.selected_pod().unwrap().name, "node-exporter");

    // With no pods the selection clamps to zero rather than underflowing.
    let mut empty = node_details("node-1");
    empty.pods.clear();
    state.selected_pod_idx = 3;
    state.set_details(empty);
    assert_eq!(state.selected_pod_idx, 0);
    assert!(state.selected_pod().is_none());
}

#[test]
fn node_inspector_selection_moves_within_bounds_and_pages() {
    let mut details = node_details("node-1");
    details.pods = (0..10)
        .map(|i| pod("default", &format!("pod-{i}"), "Running"))
        .collect();
    let mut state = node_state(details);

    state.select_prev();
    assert_eq!(state.selected_pod_idx, 0);
    state.select_next();
    state.select_next();
    assert_eq!(state.selected_pod_idx, 2);
    state.page_down(5);
    assert_eq!(state.selected_pod_idx, 7);
    state.page_down(50);
    assert_eq!(state.selected_pod_idx, 9);
    state.select_next();
    assert_eq!(
        state.selected_pod_idx, 9,
        "select_next stops at the last pod"
    );
    state.page_up(4);
    assert_eq!(state.selected_pod_idx, 5);
    state.page_up(100);
    assert_eq!(state.selected_pod_idx, 0);
    state.select_last();
    assert_eq!(state.selected_pod_idx, 9);
    state.scroll_offset = 4;
    state.select_first();
    assert_eq!(state.selected_pod_idx, 0);
    assert_eq!(state.scroll_offset, 0);

    // Empty pod lists never move the selection.
    let mut empty = NodeInspectorState::new("n".to_string());
    empty.select_next();
    empty.select_last();
    empty.page_down(3);
    assert_eq!(empty.selected_pod_idx, 0);
}

#[test]
fn node_inspector_metrics_history_is_replaced_wholesale() {
    let mut state = NodeInspectorState::new("node-1".to_string());
    state.update_metrics_history(&[1, 2, 3], &[10, 20]);
    assert_eq!(state.cpu_history, vec![1, 2, 3]);
    assert_eq!(state.mem_history, vec![10, 20]);
    state.update_metrics_history(&[9], &[]);
    assert_eq!(state.cpu_history, vec![9]);
    assert!(state.mem_history.is_empty());
}

#[test]
fn node_inspector_header_shows_status_role_type_kubelet_os_kernel_and_runtime() {
    let state = node_state(node_details("node-1"));
    let text = render_node(160, 40, &state);
    assert!(text.contains("Node: node-1"), "{text}");
    assert!(text.contains("● Ready"), "{text}");
    assert!(!text.contains("CORDONED"), "{text}");
    assert!(text.contains("Role: worker"), "{text}");
    assert!(text.contains("Type: m5.xlarge"), "{text}");
    assert!(!text.contains("Zone:"), "{text}");
    assert!(!text.contains("Pool:"), "{text}");
    assert!(text.contains("Kubelet: v1.30.2"), "{text}");
    assert!(text.contains("OS: Ubuntu 22.04 (amd64)"), "{text}");
    assert!(text.contains("Kernel: 5.15.0"), "{text}");
    assert!(text.contains("Runtime: containerd://1.7"), "{text}");
    assert!(!text.contains("IP:"), "{text}");
}

#[test]
fn node_inspector_header_shows_cordon_badge_gpu_model_zone_pool_and_ip() {
    let mut details = node_details("gpu-node");
    details.status = "NotReady".to_string();
    details.unschedulable = true;
    details.has_gpu = true;
    details.gpu_model = Some("Tesla T4".to_string());
    details.zone = Some("eu-west-1b".to_string());
    details.nodepool = Some("gpu-pool".to_string());
    details.internal_ip = Some("10.0.1.50".to_string());
    let state = node_state(details);

    let text = render_node(180, 40, &state);
    assert!(text.contains("● NotReady"), "{text}");
    assert!(text.contains("[CORDONED / UNSCHEDULABLE]"), "{text}");
    assert!(text.contains("Tesla T4]"), "{text}");
    assert!(text.contains("Zone: eu-west-1b"), "{text}");
    assert!(text.contains("Pool: gpu-pool"), "{text}");
    assert!(text.contains("IP: 10.0.1.50"), "{text}");
    // The footer flips its cordon hint when the node is already cordoned.
    assert!(text.contains("<c>:Uncordon"), "{text}");
    assert!(!text.contains(":Cordon "), "{text}");

    let buf = render_buffer(180, 40, |f| render_node_inspector_view(f, f.area(), &state));
    let lines = common::render_lines(180, 40, |f| render_node_inspector_view(f, f.area(), &state));
    let y = row_of(&lines, "● NotReady") as u16;
    let x = col(&lines[y as usize], "● NotReady");
    assert_eq!(
        buf[(x, y)].fg,
        Theme::RED,
        "a non-Ready status is drawn red"
    );
}

#[test]
fn node_inspector_gpu_model_falls_back_to_a_generic_label() {
    let mut details = node_details("gpu-node");
    details.has_gpu = true;
    details.gpu_model = None;
    details.gpu_capacity_count = 1;
    details.gpu_allocatable_count = 1;
    let state = node_state(details);
    let text = render_node(160, 40, &state);
    assert!(text.contains("GPU Accelerator]"), "{text}");
    assert!(text.contains("GPU: 0/1 (0%)"), "{text}");
}

#[test]
fn node_inspector_footer_lists_every_shortcut_with_the_cordon_hint() {
    let state = node_state(node_details("node-1"));
    let lines = common::render_lines(200, 40, |f| render_node_inspector_view(f, f.area(), &state));
    let footer = lines.last().unwrap();
    for hint in [
        "<↑/↓>:Select Pod",
        "<Enter>:Jump",
        "<l>:Logs",
        "<d>:Pod Describe",
        "<D>:Node Describe",
        "<y>:Pod YAML",
        "<Y>:Node YAML",
        "<x>:Actions",
        "<c>:Cordon",
        "<s>:Shell",
        "<S>:Node Debug",
        "<r>:Refresh",
        "<Esc>:Back",
    ] {
        assert!(footer.contains(hint), "missing {hint} in {footer}");
    }
}

#[test]
fn node_inspector_gauges_show_cpu_memory_and_pod_percentages_without_a_gpu() {
    let state = node_state(node_details("node-1"));
    let text = render_node(120, 40, &state);
    assert!(text.contains("CPU: 2.5/7.8 Cores (32%)"), "{text}");
    assert!(text.contains("Memory: 13.7/30.3 GiB (45%)"), "{text}");
    assert!(text.contains("Pods: 2/110 (2%)"), "{text}");
    assert!(!text.contains("VRAM"), "{text}");
    assert!(!text.contains("Slices"), "{text}");
}

#[test]
fn node_inspector_gauges_report_zero_when_allocatable_is_zero() {
    let mut details = node_details("node-1");
    details.cpu_allocatable_millicores = 0;
    details.mem_allocatable_mib = 0;
    details.pods_allocatable = 0;
    details.pods_count = 3;
    let state = node_state(details);
    let text = render_node(120, 40, &state);
    assert!(text.contains("CPU: 2.5/0.0 Cores (0%)"), "{text}");
    assert!(text.contains("Memory: 13.7/0.0 GiB (0%)"), "{text}");
    // pods_allocatable is floored to 1 so the ratio does not divide by zero.
    assert!(text.contains("Pods: 3/0 (300%)"), "{text}");
}

#[test]
fn node_inspector_gauges_go_yellow_then_red_as_pressure_rises() {
    let mut details = node_details("node-1");
    details.cpu_requests_millicores = 6000; // 77%
    details.mem_requests_mib = 30000; // 97%
    details.pods_count = 80; // 73%
    let state = node_state(details);
    let lines = common::render_lines(160, 40, |f| render_node_inspector_view(f, f.area(), &state));
    let buf = render_buffer(160, 40, |f| render_node_inspector_view(f, f.area(), &state));
    let title_row = row_of(&lines, "CPU: 6.0/7.8 Cores (77%)");
    let cpu_x = col(&lines[title_row], "CPU: 6.0");
    let mem_x = col(&lines[title_row], "Memory: 29.3/30.3 GiB (97%)");
    let pods_x = col(&lines[title_row], "Pods: 80/110 (73%)");
    // The gauge body sits one row under its title; the filled part carries the fg colour.
    let body = title_row as u16 + 1;
    assert_eq!(buf[(cpu_x, body)].fg, Theme::YELLOW);
    assert_eq!(buf[(mem_x, body)].fg, Theme::RED);
    assert_eq!(buf[(pods_x, body)].fg, Theme::YELLOW);
}

#[test]
fn node_inspector_gpu_gauge_prefers_vram_when_memory_requests_are_known() {
    let mut details = node_details("gpu-node");
    details.has_gpu = true;
    details.gpu_model = Some("Tesla T4".to_string());
    details.gpu_capacity_count = 1;
    details.gpu_allocatable_count = 1;
    details.gpu_requests_count = 1;
    details.gpu_memory_total_mib = Some(15360);
    details.gpu_memory_requests_mib = 7168;
    let state = node_state(details);
    let text = render_node(200, 40, &state);
    assert!(text.contains("Tesla T4: 7.0/15.0 GiB VRAM (47%)"), "{text}");
}

#[test]
fn node_inspector_gpu_gauge_counts_slices_when_more_than_one_gpu_is_allocatable() {
    let mut details = node_details("gpu-node");
    details.has_gpu = true;
    details.gpu_model = Some("A100".to_string());
    details.gpu_capacity_count = 8;
    details.gpu_allocatable_count = 8;
    details.gpu_requests_count = 3;
    let state = node_state(details);
    let text = render_node(160, 40, &state);
    assert!(text.contains("A100: 3/8 Slices (38%)"), "{text}");

    // Over-commit beyond 90% turns the gauge red.
    let mut hot = node_details("gpu-node");
    hot.has_gpu = true;
    hot.gpu_model = Some("A100".to_string());
    hot.gpu_capacity_count = 4;
    hot.gpu_allocatable_count = 4;
    hot.gpu_requests_count = 4;
    let hot_state = node_state(hot);
    let lines = common::render_lines(160, 40, |f| {
        render_node_inspector_view(f, f.area(), &hot_state)
    });
    let buf = render_buffer(160, 40, |f| {
        render_node_inspector_view(f, f.area(), &hot_state)
    });
    let row = row_of(&lines, "A100: 4/4 Slices (100%)");
    let x = col(&lines[row], "A100: 4/4");
    assert_eq!(buf[(x, row as u16 + 1)].fg, Theme::RED);
}

#[test]
fn node_inspector_sparklines_only_appear_when_the_terminal_is_tall_enough() {
    let mut state = node_state(node_details("node-1"));

    // No samples yet: both cards explain they are waiting on metrics-server.
    let tall = render_node(160, 40, &state);
    assert!(
        tall.contains("CPU Usage Trend [cur: 2500m | peak: 2500m | alloc: 7800m]"),
        "{tall}"
    );
    assert!(
        tall.contains("Memory Usage Trend [cur: 14000MiB | peak: 14000MiB | alloc: 31000MiB]"),
        "{tall}"
    );
    assert_eq!(
        tall.matches("Awaiting metrics-server samples...").count(),
        2,
        "{tall}"
    );

    // With samples the sparkline replaces the placeholder and the title tracks cur/peak.
    state.update_metrics_history(&[100, 900, 400], &[2048, 4096]);
    let with_data = render_node(160, 40, &state);
    assert!(
        with_data.contains("CPU Usage Trend [cur: 400m | peak: 900m | alloc: 7800m]"),
        "{with_data}"
    );
    assert!(
        with_data.contains("Memory Usage Trend [cur: 4096MiB | peak: 4096MiB"),
        "{with_data}"
    );
    assert!(
        !with_data.contains("Awaiting metrics-server samples"),
        "{with_data}"
    );

    // Under 26 rows the timeline is dropped entirely.
    let short = render_node(160, 25, &state);
    assert!(!short.contains("Usage Trend"), "{short}");
    assert!(short.contains("Scheduled Pods"), "{short}");
}

#[test]
fn node_inspector_conditions_and_taints_render_or_say_none() {
    let mut state = node_state(node_details("node-1"));
    let text = render_node(160, 40, &state);
    assert!(text.contains("Health Conditions & Taints"), "{text}");
    assert!(text.contains("Conditions: Ready:True"), "{text}");
    assert!(text.contains("Taints: None"), "{text}");

    let mut details = node_details("node-1");
    details.conditions = vec![
        NodeConditionInfo {
            type_: "Ready".to_string(),
            status: "False".to_string(),
            reason: None,
            message: None,
        },
        NodeConditionInfo {
            type_: "MemoryPressure".to_string(),
            status: "True".to_string(),
            reason: None,
            message: None,
        },
        NodeConditionInfo {
            type_: "DiskPressure".to_string(),
            status: "False".to_string(),
            reason: None,
            message: None,
        },
    ];
    details.taints = vec![
        NodeTaintInfo {
            key: "nvidia.com/gpu".to_string(),
            value: Some("present".to_string()),
            effect: "NoSchedule".to_string(),
        },
        NodeTaintInfo {
            key: "dedicated".to_string(),
            value: None,
            effect: "NoExecute".to_string(),
        },
    ];
    state.set_details(details);
    let lines = common::render_lines(160, 40, |f| render_node_inspector_view(f, f.area(), &state));
    let buf = render_buffer(160, 40, |f| render_node_inspector_view(f, f.area(), &state));
    let row = row_of(&lines, "Conditions:");
    let line = &lines[row];
    assert!(
        line.contains("Ready:False MemoryPressure:True DiskPressure:False"),
        "{line}"
    );
    assert!(
        line.contains("Taints: nvidia.com/gpu=present:NoSchedule dedicated:NoExecute"),
        "{line}"
    );
    let bad = col(&line, "Ready:False");
    let good = col(&line, "DiskPressure:False");
    assert_eq!(buf[(bad, row as u16)].fg, Theme::RED);
    assert_eq!(buf[(good, row as u16)].fg, Theme::GREEN);

    let mut none = node_details("node-1");
    none.conditions.clear();
    state.set_details(none);
    let text = render_node(160, 40, &state);
    assert!(text.contains("Conditions: None"), "{text}");
}

#[test]
fn node_inspector_pods_table_lists_pods_and_marks_the_selected_one() {
    let mut details = node_details("node-1");
    details.pods = vec![
        {
            let mut p = pod("ai-prod", "vllm-serve-7b", "Running");
            p.cpu_requests_millicores = 2000;
            p.mem_requests_mib = 12000;
            p.gpu_requests = 1;
            p.gpu_mem_requests_mib = 7168;
            p.restarts = 3;
            p
        },
        {
            let mut p = pod("batch", "trainer", "Pending");
            p.cpu_requests_millicores = 0;
            p.mem_requests_mib = 0;
            p.gpu_requests = 2;
            p.pod_ip = String::new();
            p
        },
        {
            let mut p = pod("batch", "small-vram", "Failed");
            p.gpu_mem_requests_mib = 512;
            p
        },
        pod(
            "kube-system",
            "a-very-long-pod-name-that-will-be-truncated",
            "Succeeded",
        ),
    ];
    let mut state = node_state(details);
    state.select_next();

    let lines = common::render_lines(160, 40, |f| render_node_inspector_view(f, f.area(), &state));
    let text = lines.join("\n");
    // Emoji take two cells, so assert on the text either side of them.
    assert!(text.contains("Scheduled Pods (4 Total |"), "{text}");
    assert!(text.contains("3 GPU Workloads)"), "{text}");
    let header = &lines[row_of(&lines, "NAMESPACE")];
    for col in [
        "NAME", "IP", "STATUS", "READY", "REST", "CPU REQ", "MEM REQ", "GPU REQ", "AGE",
    ] {
        assert!(header.contains(col), "missing {col} in {header}");
    }

    // Rows sit inside the table's block, so each one opens with its border.
    let vllm = &lines[row_of(&lines, "vllm-serve-7b")];
    assert!(
        vllm.starts_with("│  ai-prod"),
        "unselected rows have a blank marker: {vllm}"
    );
    assert!(vllm.contains("2.0c"), "{vllm}");
    assert!(vllm.contains("11.7 GiB"), "{vllm}");
    assert!(vllm.contains("7.0 GiB"), "{vllm}");
    assert!(vllm.contains("10.244.1.5"), "{vllm}");

    let trainer = &lines[row_of(&lines, "trainer")];
    assert!(
        trainer.starts_with("│▶ batch"),
        "selected row carries the marker: {trainer}"
    );
    assert!(
        trainer.contains(" -  "),
        "missing ip/cpu/mem show as dashes: {trainer}"
    );
    assert!(trainer.contains("2 GPU"), "{trainer}");
    assert!(trainer.contains("Pending"), "{trainer}");

    let small = &lines[row_of(&lines, "small-vram")];
    assert!(small.contains("512 MiB"), "{small}");
    assert!(small.contains("100m"), "{small}");
    assert!(small.contains("128 MiB"), "{small}");

    let long = &lines[row_of(&lines, "a-very-long-pod-name")];
    assert!(long.contains("a-very-long-pod-name-that-will-be-truncated"), "{long}");

    let buf = render_buffer(160, 40, |f| render_node_inspector_view(f, f.area(), &state));
    let sel_row = row_of(&lines, "trainer") as u16;
    assert_eq!(
        buf[(4, sel_row)].bg,
        Theme::SEL_BG,
        "selected row is highlighted"
    );
    let other_row = row_of(&lines, "vllm-serve-7b") as u16;
    assert_eq!(buf[(4, other_row)].bg, Color::Reset);
    let status_x = col(&vllm, "Running");
    assert_eq!(buf[(status_x, other_row)].fg, Theme::GREEN);
    let pending_x = col(&trainer, "Pending");
    assert_eq!(buf[(pending_x, sel_row)].fg, Theme::YELLOW);
    let failed_row = row_of(&lines, "small-vram") as u16;
    let failed_x = col(&small, "Failed");
    assert_eq!(buf[(failed_x, failed_row)].fg, Theme::RED);
    let succeeded_row = row_of(&lines, "a-very-long-pod-name") as u16;
    let succeeded_x = col(&long, "Succeeded");
    assert_eq!(buf[(succeeded_x, succeeded_row)].fg, Theme::CYAN);
}

#[test]
fn node_inspector_pods_table_says_so_when_the_node_is_empty() {
    let mut details = node_details("node-1");
    details.pods.clear();
    let state = node_state(details);
    let text = render_node(120, 40, &state);
    assert!(text.contains("Scheduled Pods (0 Total)"), "{text}");
    assert!(
        text.contains("No pods currently scheduled on this node."),
        "{text}"
    );
    assert!(!text.contains("NAMESPACE"), "{text}");
}

#[test]
fn node_inspector_pods_table_scrolls_to_keep_the_selection_visible() {
    let mut details = node_details("node-1");
    details.pods = (0..30)
        .map(|i| pod("default", &format!("pod-{i:02}"), "Running"))
        .collect();
    let mut state = node_state(details);

    // Wide/tall: 21 visible rows. Selecting the last pod scrolls down.
    state.select_last();
    let text = render_node(120, 40, &state);
    assert!(text.contains("▶ default"), "{text}");
    assert!(text.contains("pod-29"), "{text}");
    assert!(!text.contains("pod-00"), "{text}");
    assert_eq!(state.last_scroll_offset.get(), 9);
    assert!(state.last_pods_table_rect.get().height > 0);

    // A stale scroll offset below the selection snaps back up to it.
    state.select_first();
    state.scroll_offset = 5;
    let text = render_node(120, 40, &state);
    assert!(text.contains("pod-00"), "{text}");
    assert_eq!(state.last_scroll_offset.get(), 0);

    // Narrow terminal: only a handful of rows fit, the selection still shows.
    state.scroll_offset = 0;
    state.page_down(12);
    let narrow = render_node(60, 20, &state);
    assert!(narrow.contains("pod-12"), "{narrow}");
    assert!(!narrow.contains("pod-00"), "{narrow}");
    assert!(narrow.contains("Scheduled Pods (30 Total)"), "{narrow}");
    assert!(state.last_scroll_offset.get() > 0);
}

#[test]
fn node_inspector_pods_table_skips_rows_when_there_is_no_room_for_them() {
    let state = node_state(node_details("node-1"));
    // At three rows the layout squeezes the table down to its borders, so
    // the renderer bails before recording where the rows would have gone.
    let sentinel = ratatui::layout::Rect::new(7, 7, 7, 7);
    state.last_pods_table_rect.set(sentinel);
    let text = render_node(80, 3, &state);
    assert!(!text.contains("NAMESPACE"), "{text}");
    assert!(!text.contains("coredns"), "{text}");
    assert_eq!(
        state.last_pods_table_rect.get(),
        sentinel,
        "no table rect is recorded"
    );
}

// ─────────────────────────────── settings ───────────────────────────────

fn settings_state() -> SettingsViewState {
    SettingsViewState {
        settings: AiSettings::default(),
        selected_provider_idx: 0,
        selected_field: SettingField::ProviderToggle,
        is_editing: false,
        edit_buffer: String::new(),
        toast: None,
    }
}

fn render_settings(width: u16, height: u16, state: &SettingsViewState) -> String {
    common::render_text(width, height, |f| render_settings_view(f, f.area(), state))
}

#[test]
fn settings_view_new_selects_the_configured_default_provider() {
    // `new` loads whatever the machine has on disk; the invariant that holds
    // regardless is that the cursor starts on the default provider.
    let state = SettingsViewState::new();
    assert_eq!(state.current_provider(), state.settings.default_provider);
    assert_eq!(state.selected_field, SettingField::ProviderToggle);
    assert!(!state.is_editing);
    assert!(state.edit_buffer.is_empty());
    assert!(state.toast.is_none());
}

#[test]
fn settings_view_provider_selection_wraps_both_ways_and_freezes_while_editing() {
    let mut state = settings_state();
    assert_eq!(state.current_provider(), AiProvider::Anthropic);
    state.select_prev_provider();
    assert_eq!(state.current_provider(), AiProvider::Cursor);
    state.select_next_provider();
    assert_eq!(state.current_provider(), AiProvider::Anthropic);
    state.select_next_provider();
    assert_eq!(state.current_provider(), AiProvider::OpenAi);
    state.select_prev_provider();
    assert_eq!(state.current_provider(), AiProvider::Anthropic);

    state.is_editing = true;
    state.select_next_provider();
    state.select_prev_provider();
    assert_eq!(state.current_provider(), AiProvider::Anthropic);

    // An out-of-range index is folded back into the provider list.
    state.selected_provider_idx = 7;
    assert_eq!(state.current_provider(), AiProvider::Gemini);
}

#[test]
fn settings_view_field_cycle_skips_base_url_unless_the_provider_is_custom() {
    let mut state = settings_state();
    let mut seen = vec![state.selected_field];
    for _ in 0..4 {
        state.select_next_field();
        seen.push(state.selected_field);
    }
    assert_eq!(
        seen,
        vec![
            SettingField::ProviderToggle,
            SettingField::ApiKey,
            SettingField::Model,
            SettingField::Timeout,
            SettingField::ProviderToggle,
        ]
    );

    let mut back = vec![state.selected_field];
    for _ in 0..4 {
        state.select_prev_field();
        back.push(state.selected_field);
    }
    assert_eq!(
        back,
        vec![
            SettingField::ProviderToggle,
            SettingField::Timeout,
            SettingField::Model,
            SettingField::ApiKey,
            SettingField::ProviderToggle,
        ]
    );

    // The OpenAI-compatible provider adds Base URL between Model and Timeout.
    state.selected_provider_idx = 3;
    assert_eq!(state.current_provider(), AiProvider::OpenAiCompatible);
    let mut custom = vec![state.selected_field];
    for _ in 0..5 {
        state.select_next_field();
        custom.push(state.selected_field);
    }
    assert_eq!(
        custom,
        vec![
            SettingField::ProviderToggle,
            SettingField::ApiKey,
            SettingField::Model,
            SettingField::BaseUrl,
            SettingField::Timeout,
            SettingField::ProviderToggle,
        ]
    );
    state.select_prev_field();
    assert_eq!(state.selected_field, SettingField::Timeout);
    state.select_prev_field();
    assert_eq!(state.selected_field, SettingField::BaseUrl);
    state.select_prev_field();
    assert_eq!(state.selected_field, SettingField::Model);

    // Field navigation is frozen while an edit is open.
    state.is_editing = true;
    state.select_next_field();
    state.select_prev_field();
    assert_eq!(state.selected_field, SettingField::Model);
}

#[test]
fn settings_view_enter_on_the_provider_row_activates_it_instead_of_editing() {
    let mut state = settings_state();
    state.selected_provider_idx = 2;
    state.start_editing();
    assert!(!state.is_editing);
    assert_eq!(state.settings.default_provider, AiProvider::Gemini);
    assert!(state.edit_buffer.is_empty());

    state.selected_provider_idx = 4;
    state.set_active_provider();
    assert_eq!(state.settings.default_provider, AiProvider::Cursor);
}

#[test]
fn settings_view_start_editing_prefills_the_buffer_from_the_current_value() {
    let mut state = settings_state();
    state
        .settings
        .api_keys
        .insert("anthropic".to_string(), "sk-ant-secret".to_string());

    state.selected_field = SettingField::ApiKey;
    state.start_editing();
    assert!(state.is_editing);
    assert_eq!(state.edit_buffer, "sk-ant-secret");
    state.cancel_editing();
    assert!(!state.is_editing);
    assert!(state.edit_buffer.is_empty());

    state.selected_field = SettingField::Model;
    state.start_editing();
    assert_eq!(state.edit_buffer, "claude-3-7-sonnet-20250219");
    state.cancel_editing();

    state.selected_field = SettingField::Timeout;
    state.start_editing();
    assert_eq!(state.edit_buffer, "120");
    state.cancel_editing();

    state.selected_provider_idx = 3;
    state.selected_field = SettingField::BaseUrl;
    state.start_editing();
    assert_eq!(state.edit_buffer, "http://localhost:11434/v1");
    state.cancel_editing();

    // A provider with no stored key starts from an empty buffer.
    state.selected_provider_idx = 1;
    state.selected_field = SettingField::ApiKey;
    state.start_editing();
    assert_eq!(state.edit_buffer, "");
}

#[test]
fn settings_view_finish_editing_stores_trimmed_values_and_removes_blanks() {
    let mut state = settings_state();
    state.selected_provider_idx = 1; // OpenAI

    state.selected_field = SettingField::ApiKey;
    state.start_editing();
    state.edit_buffer = "  sk-openai-1234  ".to_string();
    state.finish_editing();
    assert!(!state.is_editing);
    assert!(state.edit_buffer.is_empty());
    assert_eq!(
        state.settings.api_keys.get("openai").map(String::as_str),
        Some("sk-openai-1234")
    );

    state.start_editing();
    state.edit_buffer = "   ".to_string();
    state.finish_editing();
    assert!(!state.settings.api_keys.contains_key("openai"));

    state.selected_field = SettingField::Model;
    state.start_editing();
    state.edit_buffer = "gpt-4.1-mini".to_string();
    state.finish_editing();
    assert_eq!(state.settings.get_model(AiProvider::OpenAi), "gpt-4.1-mini");
    state.start_editing();
    state.edit_buffer.clear();
    state.finish_editing();
    assert!(!state.settings.models.contains_key("openai"));
    assert_eq!(
        state.settings.get_model(AiProvider::OpenAi),
        "gpt-4o",
        "falls back to the default"
    );

    state.selected_provider_idx = 3; // OpenAI-compatible
    state.selected_field = SettingField::BaseUrl;
    state.start_editing();
    state.edit_buffer = "http://ollama.internal:11434/v1".to_string();
    state.finish_editing();
    assert_eq!(
        state.settings.get_base_url(AiProvider::OpenAiCompatible),
        "http://ollama.internal:11434/v1"
    );
    state.start_editing();
    state.edit_buffer.clear();
    state.finish_editing();
    assert!(!state.settings.base_urls.contains_key("openai-compatible"));
    assert_eq!(
        state.settings.get_base_url(AiProvider::OpenAiCompatible),
        "http://localhost:11434/v1"
    );
}

#[test]
fn settings_view_timeout_edits_are_parsed_and_clamped() {
    let mut state = settings_state();
    state.selected_field = SettingField::Timeout;

    state.start_editing();
    state.edit_buffer = "300".to_string();
    state.finish_editing();
    assert_eq!(
        state.settings.get_timeout_seconds(AiProvider::Anthropic),
        300
    );

    state.start_editing();
    state.edit_buffer = "1".to_string();
    state.finish_editing();
    assert_eq!(state.settings.get_timeout_seconds(AiProvider::Anthropic), 5);

    state.start_editing();
    state.edit_buffer = "99999".to_string();
    state.finish_editing();
    assert_eq!(
        state.settings.get_timeout_seconds(AiProvider::Anthropic),
        3600
    );

    state.start_editing();
    state.edit_buffer = "soon".to_string();
    state.finish_editing();
    assert_eq!(
        state.settings.get_timeout_seconds(AiProvider::Anthropic),
        3600,
        "garbage is ignored"
    );
    assert!(!state.is_editing);

    // Finishing on the provider row changes nothing.
    state.selected_field = SettingField::ProviderToggle;
    state.is_editing = true;
    state.edit_buffer = "ignored".to_string();
    let before = state.settings.clone();
    state.finish_editing();
    assert_eq!(state.settings, before);
    assert!(!state.is_editing);
}

#[test]
fn settings_view_renders_the_active_provider_summary_and_every_provider_card() {
    let mut state = settings_state();
    state.settings.default_provider = AiProvider::Gemini;
    state.selected_provider_idx = 2;
    let text = render_settings(120, 40, &state);

    assert!(text.contains("SRElens AI & Assistant Settings"), "{text}");
    assert!(
        text.contains("Active AI Provider: ● Google Gemini (model: gemini-2.5-flash)"),
        "{text}"
    );
    assert!(
        text.contains("Configure default provider, API keys, models"),
        "{text}"
    );
    assert!(text.contains("○ 1. Anthropic (Claude)"), "{text}");
    assert!(text.contains("○ 2. OpenAI (GPT-4o)"), "{text}");
    assert!(
        text.contains("● 3. Google Gemini  [ACTIVE DEFAULT]"),
        "{text}"
    );
    assert!(
        text.contains("○ 4. OpenAI-Compatible / Ollama (Local)"),
        "{text}"
    );
    assert!(text.contains("○ 5. Cursor Agent (cursor-agent)"), "{text}");
    assert_eq!(text.matches("[ACTIVE DEFAULT]").count(), 1, "{text}");
    assert!(
        text.contains("Model:   claude-3-7-sonnet-20250219   │   Timeout: 120s"),
        "{text}"
    );
    assert!(text.contains("Model:   llama3.2"), "{text}");
    assert!(
        text.contains("Auth:    auto (uses logged-in cursor auth or CURSOR_API_KEY)"),
        "{text}"
    );
    assert!(
        text.contains("[installed: ") || text.contains("[not found on PATH]"),
        "the cursor card reports whether the binary was found: {text}"
    );
    assert!(
        text.contains("[↑/↓/j/k] Provider  [Tab/←/→] Field  [Space] Set Active"),
        "{text}"
    );
    assert!(
        text.contains("[e/Enter] Edit  [s] Save to Disk  [Esc/q] Back"),
        "{text}"
    );
    assert!(
        !text.contains("Enter new value"),
        "no modal unless editing: {text}"
    );
}

#[test]
fn settings_view_masks_stored_api_keys_and_flags_missing_ones() {
    let mut state = settings_state();
    state.settings.api_keys.insert(
        "anthropic".to_string(),
        "sk-ant-api03-verylongkey-wxyz".to_string(),
    );
    state
        .settings
        .api_keys
        .insert("gemini".to_string(), "short".to_string());
    state
        .settings
        .api_keys
        .insert("openai".to_string(), "   ".to_string());
    let text = render_settings(120, 40, &state);

    assert!(
        text.contains("API Key: sk-a••••••••wxyz [stored in config]"),
        "{text}"
    );
    assert!(
        !text.contains("verylongkey"),
        "the middle of the key never reaches the screen"
    );
    assert!(
        text.contains("API Key: •••••••• [stored in config]"),
        "{text}"
    );
    assert!(!text.contains("short"), "{text}");
    // A blank stored key counts as unset. (Guarded: a developer's shell may
    // export the key, which legitimately takes the env branch instead.)
    if std::env::var("OPENAI_API_KEY").is_err() {
        assert!(
            text.contains("API Key: no key set (press 'e' to set or export OPENAI_API_KEY)"),
            "{text}"
        );
    }
    if std::env::var("OPENAI_COMPATIBLE_API_KEY").is_err() {
        assert!(text.contains("API Key: optional (local Ollama)"), "{text}");
    }
}

#[test]
fn settings_view_reports_an_api_key_that_comes_from_the_environment() {
    // Only this test touches GEMINI_API_KEY, and no other test in this file
    // asserts on Gemini's key line, so the parallel runner cannot race it.
    std::env::set_var("GEMINI_API_KEY", "from-env");
    let state = settings_state();
    let text = render_settings(120, 40, &state);
    std::env::remove_var("GEMINI_API_KEY");
    assert!(
        text.contains("API Key: [env: GEMINI_API_KEY set]"),
        "{text}"
    );
    assert!(
        !text.contains("from-env"),
        "the value itself is never shown: {text}"
    );
}

#[test]
fn settings_view_highlights_the_focused_field_on_the_selected_card() {
    let mut state = settings_state();
    state.selected_provider_idx = 1;
    state.selected_field = SettingField::Model;
    let lines = common::render_lines(120, 40, |f| render_settings_view(f, f.area(), &state));
    let buf = render_buffer(120, 40, |f| render_settings_view(f, f.area(), &state));

    let openai_row = row_of(&lines, "2. OpenAI (GPT-4o)");
    let model_row = openai_row + 2;
    assert!(
        lines[model_row].contains("Model:   gpt-4o"),
        "{}",
        lines[model_row]
    );
    let model_x = col(&lines[model_row], "Model:");
    assert_eq!(
        buf[(model_x, model_row as u16)].fg,
        Theme::YELLOW,
        "focused label is yellow"
    );
    let value_x = col(&lines[model_row], "gpt-4o");
    assert_eq!(
        buf[(value_x, model_row as u16)].fg,
        Theme::YELLOW,
        "focused value is yellow"
    );
    let timeout_x = col(&lines[model_row], "Timeout:");
    assert_eq!(
        buf[(timeout_x, model_row as u16)].fg,
        Theme::DIM,
        "unfocused label stays dim"
    );

    // The selected card's border is cyan, the others' are the plain border colour.
    let card_top = openai_row as u16 - 1;
    assert_eq!(buf[(1, card_top)].fg, Theme::CYAN);
    let anthropic_top = row_of(&lines, "1. Anthropic (Claude)") as u16 - 1;
    assert_eq!(buf[(1, anthropic_top)].fg, Theme::BORDER);

    // Timeout, API key and Base URL focus each light up their own label.
    let mut s = settings_state();
    s.selected_field = SettingField::Timeout;
    let b = render_buffer(120, 40, |f| render_settings_view(f, f.area(), &s));
    let l = common::render_lines(120, 40, |f| render_settings_view(f, f.area(), &s));
    let r = row_of(&l, "1. Anthropic (Claude)") + 2;
    let tx = col(&l[r], "Timeout:");
    assert_eq!(b[(tx, r as u16)].fg, Theme::YELLOW);

    s.selected_field = SettingField::ApiKey;
    let b = render_buffer(120, 40, |f| render_settings_view(f, f.area(), &s));
    let r = row_of(&l, "1. Anthropic (Claude)") + 1;
    let kx = col(&l[r], "API Key:");
    assert_eq!(b[(kx, r as u16)].fg, Theme::YELLOW);
}

#[test]
fn settings_view_edit_modal_names_the_field_and_shows_the_buffer() {
    let mut state = settings_state();
    state.selected_provider_idx = 1;
    state.selected_field = SettingField::Model;
    state.start_editing();
    state.edit_buffer.push_str("-mini");
    let text = render_settings(160, 40, &state);
    assert!(text.contains("Edit Model ID for OpenAI (GPT-4o)"), "{text}");
    assert!(text.contains("Enter new value for Model ID:"), "{text}");
    assert!(text.contains("gpt-4o-mini█"), "{text}");
    assert!(
        text.contains("<Enter> Confirm  |  <Ctrl+v> Paste  |  <Ctrl+w> Rubout  |  <Esc> Cancel"),
        "{text}"
    );
    // The modal is drawn over the cards, which stay visible around it.
    assert!(text.contains("5. Cursor Agent (cursor-agent)"), "{text}");

    state.cancel_editing();
    state.selected_field = SettingField::Timeout;
    state.start_editing();
    let text = render_settings(160, 40, &state);
    assert!(
        text.contains("Edit Turn Timeout (seconds, e.g. 120) for OpenAI (GPT-4o)"),
        "{text}"
    );
    assert!(text.contains("120█"), "{text}");

    state.cancel_editing();
    state.selected_field = SettingField::ApiKey;
    state.start_editing();
    let text = render_settings(160, 40, &state);
    assert!(text.contains("Edit API Key for OpenAI (GPT-4o)"), "{text}");
    assert!(text.contains("Enter new value for API Key:"), "{text}");

    state.cancel_editing();
    state.selected_provider_idx = 4;
    state.start_editing();
    let text = render_settings(160, 40, &state);
    assert!(
        text.contains("Edit API Key (or leave blank for cursor login) for Cursor Agent"),
        "{text}"
    );

    state.cancel_editing();
    state.selected_provider_idx = 3;
    state.selected_field = SettingField::BaseUrl;
    state.start_editing();
    let text = render_settings(160, 40, &state);
    assert!(
        text.contains("Edit Base URL (e.g. http://localhost:11434/v1) for OpenAI-Compatible"),
        "{text}"
    );
    assert!(text.contains("http://localhost:11434/v1█"), "{text}");

    // A forced edit on the provider row still labels itself sensibly.
    state.cancel_editing();
    state.selected_field = SettingField::ProviderToggle;
    state.is_editing = true;
    let text = render_settings(160, 40, &state);
    assert!(
        text.contains("Edit Provider for OpenAI-Compatible"),
        "{text}"
    );
}

#[test]
fn settings_view_narrow_terminal_still_shows_the_header_and_first_cards() {
    let state = settings_state();
    let text = render_settings(60, 20, &state);
    assert!(text.contains("SRElens AI & Assistant Settings"), "{text}");
    assert!(
        text.contains("Active AI Provider: ● Anthropic (Claude)"),
        "{text}"
    );
    assert!(
        text.contains("● 1. Anthropic (Claude)  [ACTIVE DEFAULT]"),
        "{text}"
    );
    // The cards shrink to their borders and header line: no field rows fit.
    assert!(!text.contains("API Key:"), "{text}");
    assert!(!text.contains("Model:"), "{text}");
}

// ─────────────────────────────── overview ───────────────────────────────

fn overview_data() -> ClusterOverviewData {
    ClusterOverviewData {
        context_name: "prod-eu".to_string(),
        cluster_name: "eks-prod".to_string(),
        server_url: "https://k8s.example.com:6443".to_string(),
        k8s_version: "v1.30.2".to_string(),
        is_reachable: true,
        node_count: 3,
        ready_nodes: 3,
        total_pods: 42,
        running_pods: 40,
        pending_pods: 0,
        failed_pods: 0,
        total_cpu_millicores: 8000,
        used_cpu_millicores: 2500,
        total_mem_mib: 32768,
        used_mem_mib: 14000,
        total_gpus: 0,
        allocated_gpus: 0,
        total_gpu_mem_mib: 0,
        used_gpu_mem_mib: 0,
    }
}

fn render_overview(width: u16, height: u16, state: &OverviewViewState) -> String {
    common::render_text(width, height, |f| render_overview_view(f, f.area(), state))
}

#[test]
fn overview_summary_text_covers_every_optional_line() {
    let empty = ClusterOverviewData::default();
    let text = empty.to_summary_text();
    assert!(text.contains("Cluster:  ()"), "{text}");
    assert!(text.contains("Status: Unreachable"), "{text}");
    assert!(!text.contains("Server:"), "{text}");
    assert!(!text.contains("Kubernetes Version:"), "{text}");
    assert!(text.contains("Nodes: 0/0 Ready"), "{text}");
    assert!(
        text.contains("CPU Allocation: 0.0 / 0.0 Cores (0%)"),
        "{text}"
    );
    assert!(
        text.contains("Memory Allocation: 0.0 / 0.0 GiB (0%)"),
        "{text}"
    );
    assert!(!text.contains("GPU Allocation"), "{text}");
    assert!(
        text.contains("Workloads: 0 Total (0 Running, 0 Pending, 0 Failed/Crash)"),
        "{text}"
    );

    let mut full = overview_data();
    full.pending_pods = 1;
    full.failed_pods = 1;
    full.total_gpus = 4;
    full.allocated_gpus = 3;
    full.used_gpu_mem_mib = 20480;
    let text = full.to_summary_text();
    assert!(text.contains("Cluster: eks-prod (prod-eu)"), "{text}");
    assert!(text.contains("Status: Reachable / Healthy"), "{text}");
    assert!(
        text.contains("Server: https://k8s.example.com:6443"),
        "{text}"
    );
    assert!(text.contains("Kubernetes Version: v1.30.2"), "{text}");
    assert!(text.contains("Nodes: 3/3 Ready"), "{text}");
    assert!(
        text.contains("CPU Allocation: 2.5 / 8.0 Cores (31%)"),
        "{text}"
    );
    assert!(
        text.contains("Memory Allocation: 13.7 / 32.0 GiB (43%)"),
        "{text}"
    );
    assert!(
        text.contains("GPU Allocation: 3 / 4 GPUs (75%), VRAM Allocated: 20.0 GiB"),
        "{text}"
    );
    assert!(
        text.contains("Workloads: 42 Total (40 Running, 1 Pending, 1 Failed/Crash)"),
        "{text}"
    );

    // VRAM without any counted GPUs still earns a GPU line, at 0%.
    let mut vram_only = overview_data();
    vram_only.used_gpu_mem_mib = 1024;
    let text = vram_only.to_summary_text();
    assert!(
        text.contains("GPU Allocation: 0 / 0 GPUs (0%), VRAM Allocated: 1.0 GiB"),
        "{text}"
    );

    // GPUs without VRAM accounting omit the VRAM suffix.
    let mut gpus_only = overview_data();
    gpus_only.total_gpus = 2;
    let text = gpus_only.to_summary_text();
    assert!(text.contains("GPU Allocation: 0 / 2 GPUs (0%)\n"), "{text}");
}

#[test]
fn overview_state_constructors_and_set_data_replace_the_snapshot() {
    let state = OverviewViewState::new();
    assert!(state.data.context_name.is_empty());
    assert!(!state.data.is_reachable);

    let mut state = OverviewViewState::with_data(overview_data());
    assert_eq!(state.data.cluster_name, "eks-prod");

    let mut next = overview_data();
    next.cluster_name = "eks-staging".to_string();
    next.is_reachable = false;
    state.set_data(next);
    assert_eq!(state.data.cluster_name, "eks-staging");
    assert!(!state.data.is_reachable);
}

#[test]
fn overview_view_before_any_data_shows_placeholders_and_zeroed_gauges() {
    let state = OverviewViewState::new();
    let text = render_overview(120, 40, &state);
    assert!(
        text.contains(
            "Cluster Health & Resource Overview [<s> Summarise, <c> Copy, <r> Refresh, <Esc> Back]"
        ),
        "{text}"
    );
    assert!(text.contains("● UNREACHABLE"), "{text}");
    assert!(text.contains("K8s Version:   Connecting..."), "{text}");
    assert!(text.contains("Nodes:         Fetching..."), "{text}");
    assert!(text.contains("CPU Allocation: 0m / 0m (0%)"), "{text}");
    assert!(
        text.contains("Memory Allocation: 0MiB / 0MiB (0%)"),
        "{text}"
    );
    assert!(!text.contains("GPU"), "{text}");
    assert!(text.contains("Workload Health Distribution"), "{text}");
    assert!(text.contains("Total Pods:     0"), "{text}");
    assert!(text.contains("● Running:      0"), "{text}");
    assert!(text.contains("● Pending:      0"), "{text}");
    assert!(!text.contains("Unschedulable"), "{text}");
    assert!(text.contains("● Failed/Crash: 0"), "{text}");
    assert!(!text.contains("OOMKilled"), "{text}");
}

#[test]
fn overview_view_populated_cluster_shows_health_version_nodes_and_gauges() {
    let state = OverviewViewState::with_data(overview_data());
    let lines = common::render_lines(120, 40, |f| render_overview_view(f, f.area(), &state));
    let text = lines.join("\n");
    assert!(text.contains("Context:     prod-eu"), "{text}");
    assert!(text.contains("● REACHABLE / HEALTHY"), "{text}");
    assert!(text.contains("Cluster:     eks-prod"), "{text}");
    assert!(text.contains("K8s Version:   v1.30.2"), "{text}");
    assert!(
        text.contains("Server:      https://k8s.example.com:6443"),
        "{text}"
    );
    assert!(text.contains("Nodes:         3/3 Ready"), "{text}");
    assert!(
        text.contains("CPU Allocation: 2.5 / 8.0 Cores (31%)"),
        "{text}"
    );
    assert!(
        text.contains("Memory Allocation: 13.7 / 32.0 GiB (42%)"),
        "{text}"
    );
    assert!(text.contains("Total Pods:     42"), "{text}");
    assert!(text.contains("● Running:      40"), "{text}");
    assert!(!text.contains("GPU Slices"), "{text}");

    let buf = render_buffer(120, 40, |f| render_overview_view(f, f.area(), &state));
    let cpu_row = row_of(&lines, "CPU Allocation") as u16 + 1;
    assert_eq!(
        buf[(2, cpu_row)].fg,
        Theme::CYAN,
        "a cool CPU gauge is cyan"
    );
    let mem_row = row_of(&lines, "Memory Allocation") as u16 + 1;
    assert_eq!(
        buf[(2, mem_row)].fg,
        Theme::ACCENT,
        "a cool memory gauge uses the accent"
    );
}

#[test]
fn overview_view_prefixes_a_bare_version_with_v_and_uses_small_units_for_small_totals() {
    let mut data = overview_data();
    data.k8s_version = "1.29.0".to_string();
    data.total_cpu_millicores = 900;
    data.used_cpu_millicores = 800;
    data.total_mem_mib = 1000;
    data.used_mem_mib = 900;
    let state = OverviewViewState::with_data(data);
    let lines = common::render_lines(120, 40, |f| render_overview_view(f, f.area(), &state));
    let text = lines.join("\n");
    assert!(text.contains("K8s Version:   v1.29.0"), "{text}");
    assert!(text.contains("CPU Allocation: 800m / 900m (88%)"), "{text}");
    assert!(
        text.contains("Memory Allocation: 900MiB / 1000MiB (90%)"),
        "{text}"
    );

    let buf = render_buffer(120, 40, |f| render_overview_view(f, f.area(), &state));
    let cpu_row = row_of(&lines, "CPU Allocation") as u16 + 1;
    assert_eq!(buf[(2, cpu_row)].fg, Theme::RED, "over 85% turns red");
    let mem_row = row_of(&lines, "Memory Allocation") as u16 + 1;
    assert_eq!(buf[(2, mem_row)].fg, Theme::RED);

    // 70-85% is the yellow band.
    let mut warm = overview_data();
    warm.used_cpu_millicores = 6000; // 75%
    warm.used_mem_mib = 26000; // 79%
    let warm_state = OverviewViewState::with_data(warm);
    let lines = common::render_lines(120, 40, |f| render_overview_view(f, f.area(), &warm_state));
    let buf = render_buffer(120, 40, |f| render_overview_view(f, f.area(), &warm_state));
    let cpu_row = row_of(&lines, "CPU Allocation: 6.0 / 8.0 Cores (75%)") as u16 + 1;
    assert_eq!(buf[(2, cpu_row)].fg, Theme::YELLOW);
    let mem_row = row_of(&lines, "Memory Allocation: 25.4 / 32.0 GiB (79%)") as u16 + 1;
    assert_eq!(buf[(2, mem_row)].fg, Theme::YELLOW);
}

#[test]
fn overview_view_explains_pending_and_failed_pods_when_there_are_any() {
    let mut data = overview_data();
    data.pending_pods = 2;
    data.failed_pods = 1;
    data.ready_nodes = 2;
    data.is_reachable = false;
    let state = OverviewViewState::with_data(data);
    let text = render_overview(120, 40, &state);
    assert!(
        text.contains("● Pending:      2  (Unschedulable or waiting for resources/PVC)"),
        "{text}"
    );
    assert!(
        text.contains("● Failed/Crash: 1  (OOMKilled, CrashLoopBackOff, or Error)"),
        "{text}"
    );
    assert!(text.contains("Nodes:         2/3 Ready"), "{text}");
    assert!(text.contains("● UNREACHABLE"), "{text}");
}

#[test]
fn overview_view_adds_a_gpu_gauge_and_slice_line_when_the_cluster_has_gpus() {
    let mut data = overview_data();
    data.total_gpus = 4;
    data.allocated_gpus = 3;
    let state = OverviewViewState::with_data(data.clone());
    let text = render_overview(120, 40, &state);
    assert!(text.contains("GPU Allocation: 3 / 4 GPUs (75%)"), "{text}");
    assert!(!text.contains("VRAM"), "{text}");
    assert!(text.contains("GPU Slices:     3/4 Physical GPUs"), "{text}");

    // VRAM used but no total: allocated GiB only.
    data.used_gpu_mem_mib = 20480;
    let state = OverviewViewState::with_data(data.clone());
    let text = render_overview(120, 40, &state);
    assert!(
        text.contains("GPU Allocation: 3 / 4 GPUs (75%)  •  VRAM Allocated: 20.0 GiB"),
        "{text}"
    );
    assert!(
        text.contains("GPU Slices:     3/4 Physical GPUs  [VRAM: 20.0 GiB allocated]"),
        "{text}"
    );

    // VRAM used and total known: a ratio and a percentage.
    data.total_gpu_mem_mib = 65536;
    let state = OverviewViewState::with_data(data);
    let text = render_overview(120, 40, &state);
    assert!(
        text.contains("GPU Allocation: 3 / 4 GPUs (75%)  •  VRAM: 20.0 / 64.0 GiB (31%)"),
        "{text}"
    );
    assert!(
        text.contains("GPU Slices:     3/4 Physical GPUs  [VRAM: 20.0 / 64.0 GiB allocated]"),
        "{text}"
    );
}

#[test]
fn overview_view_vram_without_counted_gpus_lists_slices_but_draws_no_gauge() {
    let mut data = overview_data();
    data.used_gpu_mem_mib = 2048;
    let state = OverviewViewState::with_data(data);
    let text = render_overview(120, 40, &state);
    assert!(
        !text.contains("GPU Allocation"),
        "no gauge without physical GPUs: {text}"
    );
    assert!(
        text.contains("GPU Slices:     0/0 Physical GPUs  [VRAM: 2.0 GiB allocated]"),
        "{text}"
    );
}

#[test]
fn overview_view_narrow_terminal_keeps_the_key_facts_visible() {
    let state = OverviewViewState::with_data(overview_data());
    let text = render_overview(60, 20, &state);
    assert!(
        text.contains("Cluster Health & Resource Overview"),
        "{text}"
    );
    assert!(text.contains("prod-eu"), "{text}");
    assert!(
        text.contains("CPU Allocation: 2.5 / 8.0 Cores (31%)"),
        "{text}"
    );
    assert!(text.contains("Total Pods:     42"), "{text}");
}

// ───────────────────────────────── yaml ─────────────────────────────────

const POD_YAML: &str = "\
apiVersion: v1
kind: Pod
metadata:
  name: test-pod
  namespace: default
  labels:
    app: \"web\"
spec:
  containers:
    - name: nginx
      image: nginx:1.25
      ports:
        - containerPort: 80
      readinessProbe:
        enabled: true
# trailing comment
---
status:
  phase: Running";

fn yaml_state(yaml: &str) -> YamlViewState {
    YamlViewState::new(
        "test-pod".to_string(),
        "Pod".to_string(),
        Some("default".to_string()),
        yaml.to_string(),
    )
}

fn render_yaml(width: u16, height: u16, state: &YamlViewState) -> String {
    common::render_text(width, height, |f| render_yaml_view(f, f.area(), state))
}

#[test]
fn yaml_view_new_splits_lines_and_neutralises_tabs_escapes_and_control_characters() {
    let raw = "kind: Pod\nmessage: a\tb\n\u{1b}[31mred\u{1b}[0m: value\nbell:\u{07} x\r\nplain: ok";
    let state = yaml_state(raw);
    assert_eq!(state.resource_name, "test-pod");
    assert_eq!(state.resource_kind, "Pod");
    assert_eq!(state.namespace.as_deref(), Some("default"));
    assert_eq!(
        state.yaml_content, raw,
        "the raw content is kept for editing"
    );
    assert_eq!(state.scroll_offset, 0);
    assert!(state.search_matches.is_empty());
    assert!(state.selection.is_none());
    assert!(!state.is_diff);

    assert_eq!(state.lines.len(), 5);
    assert_eq!(state.lines[0], "kind: Pod");
    assert_eq!(
        state.lines[1], "message: a      b",
        "tab expands to the next 8-column stop"
    );
    assert_eq!(
        state.lines[2], "red: value",
        "ANSI colour sequences are dropped"
    );
    assert_eq!(state.lines[3], "bell: x", "BEL and CR are removed");
    assert_eq!(state.lines[4], "plain: ok");
    for line in &state.lines {
        assert!(!line.chars().any(char::is_control), "{line:?}");
    }

    // What reaches the screen is the sanitised text.
    let text = render_yaml(80, 12, &state);
    assert!(text.contains("2 │ message: a      b"), "{text}");
    assert!(text.contains("3 │ red: value"), "{text}");
    assert!(!text.contains('\u{1b}'), "{text}");
}

#[test]
fn yaml_view_update_content_resanitises_and_resets_navigation_state() {
    let mut state = yaml_state(POD_YAML);
    state.set_search_query("name");
    state.scroll_down(3);
    state.start_selection(1);
    state.update_selection(2);
    assert!(state.selection.is_some());
    assert!(!state.search_matches.is_empty());

    state.update_content("a: 1\nb:\tc\n\u{1b}[1mbold\u{1b}[0m: yes".to_string());
    assert_eq!(state.lines, vec!["a: 1", "b:      c", "bold: yes"]);
    assert_eq!(
        state.yaml_content,
        "a: 1\nb:\tc\n\u{1b}[1mbold\u{1b}[0m: yes"
    );
    assert_eq!(state.scroll_offset, 0);
    assert!(state.search_matches.is_empty());
    assert!(state.current_match_idx.is_none());
    assert!(state.selection.is_none());
    assert!(!state.is_selecting);
    // The query string itself survives, so the badge still reports zero matches.
    assert_eq!(state.search_query, "name");
    // Wide enough that the block title is not truncated to fit the border.
    let text = render_yaml(160, 10, &state);
    assert!(text.contains("[Search: \"name\" (0 matches)]"), "{text}");
}

#[test]
fn yaml_view_search_is_case_insensitive_jumps_to_the_first_match_and_cycles() {
    let mut state = yaml_state(POD_YAML);
    state.set_search_query("NGINX");
    assert_eq!(state.search_query, "NGINX");
    assert_eq!(state.search_matches, vec![9, 10]);
    assert_eq!(state.current_match_idx, Some(0));
    assert_eq!(state.scroll_offset, 9);

    state.next_match();
    assert_eq!(state.current_match_idx, Some(1));
    assert_eq!(state.scroll_offset, 10);
    state.next_match();
    assert_eq!(state.current_match_idx, Some(0), "wraps to the first match");
    assert_eq!(state.scroll_offset, 9);
    state.prev_match();
    assert_eq!(state.current_match_idx, Some(1), "wraps to the last match");
    assert_eq!(state.scroll_offset, 10);
    state.prev_match();
    assert_eq!(state.current_match_idx, Some(0));

    // With the index cleared, next goes to the first and prev to the last.
    state.current_match_idx = None;
    state.next_match();
    assert_eq!(state.current_match_idx, Some(0));
    state.current_match_idx = None;
    state.prev_match();
    assert_eq!(state.current_match_idx, Some(1));

    // No matches: nothing to cycle, index stays empty.
    state.set_search_query("zzz-not-here");
    assert!(state.search_matches.is_empty());
    assert!(state.current_match_idx.is_none());
    let before = state.scroll_offset;
    state.next_match();
    state.prev_match();
    assert_eq!(state.scroll_offset, before);

    // An empty query clears the match list.
    state.set_search_query("name");
    state.set_search_query("");
    assert!(state.search_matches.is_empty());
    assert!(state.current_match_idx.is_none());
    assert!(state.search_query.is_empty());

    // "name" also lives inside "namespace", so substring matching finds three.
    state.set_search_query("name");
    assert_eq!(state.search_matches, vec![3, 4, 9]);
    state.clear_search();
    assert!(state.search_query.is_empty());
    assert!(state.search_matches.is_empty());
    assert!(state.current_match_idx.is_none());
}

#[test]
fn yaml_view_selection_tracks_a_drag_and_yields_the_covered_lines() {
    let mut state = yaml_state(POD_YAML);
    assert!(state.selected_text().is_none());
    assert!(
        state.finish_selection(2).is_none(),
        "no selection to finish"
    );

    state.start_selection(1);
    assert!(state.is_selecting);
    assert_eq!(state.selection, Some((1, 1)));
    state.update_selection(3);
    assert_eq!(state.selection, Some((1, 3)));
    assert_eq!(
        state.selected_text().as_deref(),
        Some("kind: Pod\nmetadata:\n  name: test-pod")
    );

    // Dragging upwards past the anchor still yields the lines in order.
    let text = state.finish_selection(0).unwrap();
    assert_eq!(text, "apiVersion: v1\nkind: Pod");
    assert!(!state.is_selecting);
    assert_eq!(state.selection, Some((1, 0)));

    // Updates after the drag ended are ignored.
    state.update_selection(5);
    assert_eq!(state.selection, Some((1, 0)));

    // Indices past the end clamp to the last line.
    state.start_selection(999);
    let last = state.lines.len() - 1;
    assert_eq!(state.selection, Some((last, last)));
    assert_eq!(state.selected_text().as_deref(), Some("  phase: Running"));

    state.clear_selection();
    assert!(state.selection.is_none());
    assert!(!state.is_selecting);

    // An empty document never selects anything.
    let mut empty = yaml_state("");
    empty.start_selection(0);
    empty.update_selection(0);
    assert!(empty.selection.is_none());
    assert!(!empty.is_selecting);
    assert!(empty.finish_selection(0).is_none());
    assert!(empty.selected_text().is_none());
}

#[test]
fn yaml_view_scrolling_stays_within_the_document() {
    let mut state = yaml_state(POD_YAML);
    let last = state.lines.len() - 1;
    state.scroll_down(5);
    assert_eq!(state.scroll_offset, 5);
    state.scroll_down(1000);
    assert_eq!(state.scroll_offset, 5, "a jump past the end is refused");
    state.scroll_up(2);
    assert_eq!(state.scroll_offset, 3);
    state.scroll_up(100);
    assert_eq!(state.scroll_offset, 0);
    state.scroll_bottom();
    assert_eq!(state.scroll_offset, last);
    state.scroll_down(1);
    assert_eq!(state.scroll_offset, last);
    state.scroll_top();
    assert_eq!(state.scroll_offset, 0);

    let mut empty = yaml_state("");
    empty.scroll_bottom();
    empty.scroll_down(1);
    assert_eq!(empty.scroll_offset, 0);
}

#[test]
fn yaml_view_spawn_editor_reports_a_missing_editor_without_running_anything() {
    // Point $EDITOR at a binary that cannot exist so the spawn fails fast.
    // The quoted form also exercises the shlex split.
    std::env::set_var("EDITOR", "\"srelens-no-such-editor-0x5b\" --wait");
    let state = yaml_state(POD_YAML);
    let result = state.spawn_editor();
    std::env::remove_var("EDITOR");
    let err = result.expect_err("a nonexistent editor cannot be spawned");
    assert!(
        err.starts_with("Failed to spawn editor '\"srelens-no-such-editor-0x5b\" --wait':"),
        "{err}"
    );
}

#[test]
fn yaml_view_title_reports_kind_name_namespace_position_and_hints() {
    let state = yaml_state(POD_YAML);
    let lines = common::render_lines(120, 40, |f| render_yaml_view(f, f.area(), &state));
    let title = &lines[0];
    assert!(
        title.contains(
            "YAML: Pod/test-pod (default) (Line 1/19) [e: Edit, c: Copy, /: Search, Esc: Back]"
        ),
        "{title}"
    );
    assert!(!title.contains("Search:"), "{title}");
    assert_eq!(state.last_viewport_rect.get().height, 38);
    assert_eq!(state.last_viewport_rect.get().width, 118);

    let text = lines.join("\n");
    assert!(text.contains("   1 │ apiVersion: v1"), "{text}");
    assert!(text.contains("  19 │   phase: Running"), "{text}");

    // Cluster-scoped resources have no namespace.
    let node = YamlViewState::new(
        "worker-1".to_string(),
        "Node".to_string(),
        None,
        "kind: Node".to_string(),
    );
    let text = render_yaml(100, 5, &node);
    assert!(text.contains("YAML: Node/worker-1  (Line 1/1)"), "{text}");
}

#[test]
fn yaml_view_renders_only_the_visible_window_from_the_scroll_offset() {
    let mut state = yaml_state(POD_YAML);
    state.scroll_down(4);
    let lines = common::render_lines(60, 8, |f| render_yaml_view(f, f.area(), &state));
    assert!(lines[0].contains("(Line 5/19)"), "{}", lines[0]);
    assert!(
        lines[1].contains("   5 │   namespace: default"),
        "{}",
        lines[1]
    );
    assert!(
        lines[6].contains("  10 │     - name: nginx"),
        "{}",
        lines[6]
    );
    assert!(lines.iter().all(|l| !l.contains("apiVersion")), "{lines:?}");
    assert!(lines.iter().all(|l| !l.contains("image:")), "{lines:?}");

    // Narrow terminal: long lines are cut, not wrapped onto extra rows.
    let long = yaml_state(
        "key: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nnext: 1",
    );
    let lines = common::render_lines(40, 6, |f| render_yaml_view(f, f.area(), &long));
    assert!(lines[1].contains("   1 │ key: aaaa"), "{}", lines[1]);
    assert!(lines[2].contains("   2 │ next: 1"), "{}", lines[2]);
}

#[test]
fn yaml_view_search_badge_counts_matches_and_highlights_them_in_yellow() {
    let mut state = yaml_state(POD_YAML);
    state.set_search_query("Nginx");
    // 160 columns keep the whole title, badge included, off the border.
    let lines = common::render_lines(160, 40, |f| render_yaml_view(f, f.area(), &state));
    assert!(
        lines[0].contains("[Search: \"Nginx\" (1/2 matches, n/N)]"),
        "{}",
        lines[0]
    );
    // The first match is scrolled to the top of the viewport.
    assert!(
        lines[1].contains("  10 │     - name: nginx"),
        "{}",
        lines[1]
    );

    let buf = render_buffer(160, 40, |f| render_yaml_view(f, f.area(), &state));
    let x = col(&lines[1], "nginx");
    assert_eq!(
        buf[(x, 1)].bg,
        Theme::YELLOW,
        "the matched text gets the match background"
    );
    let before = col(&lines[1], "name:");
    assert_ne!(
        buf[(before, 1)].bg,
        Theme::YELLOW,
        "the rest of the line is not highlighted"
    );

    state.next_match();
    let lines = common::render_lines(160, 40, |f| render_yaml_view(f, f.area(), &state));
    assert!(lines[0].contains("(2/2 matches, n/N)"), "{}", lines[0]);
    assert!(
        lines[1].contains("  11 │       image: nginx:1.25"),
        "{}",
        lines[1]
    );

    state.set_search_query("nowhere");
    let text = render_yaml(160, 40, &state);
    assert!(text.contains("[Search: \"nowhere\" (0 matches)]"), "{text}");
}

#[test]
fn yaml_view_selection_changes_the_hint_and_shades_the_selected_rows() {
    let mut state = yaml_state(POD_YAML);
    state.start_selection(2);
    state.update_selection(0);
    let lines = common::render_lines(120, 40, |f| render_yaml_view(f, f.area(), &state));
    assert!(
        lines[0].contains("[c: Copy Selection, Esc: Clear Selection]"),
        "{}",
        lines[0]
    );
    assert!(!lines[0].contains("e: Edit"), "{}", lines[0]);

    let buf = render_buffer(120, 40, |f| render_yaml_view(f, f.area(), &state));
    let shaded = Color::Rgb(35, 55, 95);
    for row in 1..=3u16 {
        assert_eq!(
            buf[(10, row)].bg,
            shaded,
            "row {row} is inside the selection"
        );
        assert_eq!(
            buf[(1, row)].fg,
            Theme::CYAN,
            "selected line numbers turn cyan"
        );
    }
    assert_ne!(buf[(10, 4)].bg, shaded, "row 4 is outside the selection");
    assert_eq!(buf[(1, 4)].fg, Theme::DIM);
}

#[test]
fn yaml_view_colours_comments_separators_keys_and_scalar_values() {
    let state = yaml_state(POD_YAML);
    let lines = common::render_lines(120, 40, |f| render_yaml_view(f, f.area(), &state));
    let buf = render_buffer(120, 40, |f| render_yaml_view(f, f.area(), &state));
    let style_at = |needle: &str, offset: usize| {
        let row = row_of(&lines, needle);
        let x = col(&lines[row], needle) + offset as u16;
        buf[(x, row as u16)].fg
    };

    assert_eq!(style_at("# trailing comment", 0), Theme::DIM);
    assert_eq!(style_at("---", 0), Theme::YELLOW);
    assert_eq!(style_at("apiVersion:", 0), Theme::CYAN, "keys are cyan");
    assert_eq!(
        style_at("apiVersion: v1", 12),
        Theme::FG,
        "bare scalars use the foreground"
    );
    assert_eq!(
        style_at("app: \"web\"", 5),
        Theme::GREEN,
        "quoted strings are green"
    );
    assert_eq!(
        style_at("containerPort: 80", 15),
        Theme::YELLOW,
        "numbers are yellow"
    );
    assert_eq!(
        style_at("enabled: true", 9),
        Theme::YELLOW,
        "booleans are yellow"
    );
    assert_eq!(
        style_at("- name: nginx", 0),
        Theme::YELLOW,
        "list dashes are yellow"
    );
    assert_eq!(
        style_at("- name: nginx", 2),
        Theme::CYAN,
        "the key after the dash is cyan"
    );

    // A line with no colon at all is plain foreground, and single-quoted
    // scalars count as strings too.
    let plain = yaml_state("just words here\nq: 'single'\nempty:");
    let lines = common::render_lines(60, 6, |f| render_yaml_view(f, f.area(), &plain));
    let buf = render_buffer(60, 6, |f| render_yaml_view(f, f.area(), &plain));
    let x = col(&lines[1], "just");
    assert_eq!(buf[(x, 1)].fg, Theme::FG);
    let x = col(&lines[2], "'single'");
    assert_eq!(buf[(x, 2)].fg, Theme::GREEN);
    assert!(lines[3].contains("   3 │ empty:"), "{}", lines[3]);
}

#[test]
fn yaml_view_empty_document_renders_a_bare_frame() {
    let state = yaml_state("");
    assert!(state.lines.is_empty());
    let lines = common::render_lines(60, 20, |f| render_yaml_view(f, f.area(), &state));
    assert!(
        lines[0].contains("YAML: Pod/test-pod (default) (Line 1/0)"),
        "{}",
        lines[0]
    );
    assert!(
        lines[1..19].iter().all(|l| !l.contains(" │ ")),
        "no numbered rows: {lines:?}"
    );
}

// ───────────────────────────── tui config view ─────────────────────────────

#[test]
fn tui_config_view_state_field_navigation_and_adjustments() {
    let mut state = TuiConfigViewState::new();
    assert_eq!(state.selected_field, 0);

    state.select_next_field();
    assert_eq!(state.selected_field, 1);

    state.select_next_field();
    assert_eq!(state.selected_field, 0);

    state.select_prev_field();
    assert_eq!(state.selected_field, 1);

    let mut config = TuiConfig::default();
    assert_eq!(config.command_popup_max_width, 65);
    assert_eq!(config.command_popup_max_visible, 6);

    // Selected field 1: Visible Rows (step 1, range 3..=20)
    state.adjust_current(1, &mut config);
    assert_eq!(config.command_popup_max_visible, 7);

    state.adjust_current(-3, &mut config);
    assert_eq!(config.command_popup_max_visible, 4);

    state.adjust_current(-10, &mut config);
    assert_eq!(config.command_popup_max_visible, 3); // Clamped at 3

    // Switch to field 0: Width (step 5, range 40..=200)
    state.select_prev_field();
    assert_eq!(state.selected_field, 0);

    state.adjust_current(2, &mut config);
    assert_eq!(config.command_popup_max_width, 75);

    state.adjust_current(50, &mut config);
    assert_eq!(config.command_popup_max_width, 200); // Clamped at 200

    state.adjust_current(-50, &mut config);
    assert_eq!(config.command_popup_max_width, 40); // Clamped at 40

    // Reset defaults
    state.reset_defaults(&mut config);
    assert_eq!(config.command_popup_max_width, 65);
    assert_eq!(config.command_popup_max_visible, 6);
}

#[test]
fn tui_config_view_renders_cards_and_live_preview_at_wide_and_narrow() {
    let state = TuiConfigViewState::new();
    let config = TuiConfig {
        command_popup_max_width: 80,
        command_popup_max_visible: 8,
    };

    // Wide render (120x30)
    let lines = common::render_lines(120, 30, |f| {
        render_tui_config_view(f, f.area(), &state, &config)
    });
    let full = lines.join("\n");

    assert!(full.contains("TUI Configuration"), "has title");
    assert!(full.contains("Command Popup Max Width"), "has width setting card");
    assert!(full.contains("Command Popup Max Visible Rows"), "has rows setting card");
    assert!(full.contains("80 cols"), "shows configured width");
    assert!(full.contains("8 rows"), "shows configured visible rows");
    assert!(full.contains("Live Preview: Command Popup"), "shows live preview title");
    assert!(full.contains(":po█"), "shows simulated command bar prompt");
    assert!(full.contains("pods"), "shows sample suggestions in preview");

    // Narrow render (70x24) — should not panic, uses vertical split layout
    let narrow_lines = common::render_lines(70, 24, |f| {
        render_tui_config_view(f, f.area(), &state, &config)
    });
    let narrow_full = narrow_lines.join("\n");
    assert!(narrow_full.contains("TUI Configuration"));
    assert!(narrow_full.contains("Command Popup Max Width"));
}

