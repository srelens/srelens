//! Multi-cluster terminal multiplexer (`:mux`) state and management.
//!
//! Enables side-by-side split comparison of two Kubernetes clusters in a single
//! srectl process without launching a separate terminal.

use std::collections::HashSet;
use std::time::Instant;

use ratatui::layout::Rect;

use crate::app::ActiveView;
use crate::commands::CrdMeta;
use crate::ui::dialogs::ContextPickerItem;
use crate::App;

/// The snapshot of an individual cluster pane's active state.
pub struct ClusterPaneState {
    pub context: String,
    pub namespace: String,
    pub cluster_name: String,
    pub server_url: String,
    pub cluster_version: String,
    pub node_count: usize,
    pub pod_count: usize,
    pub is_connected: bool,
    pub connection_attempt_start: Instant,
    pub cluster_unreachable: bool,
    pub active_view: ActiveView,
    pub nav_stack: Vec<ActiveView>,
    pub filter_buffer: String,
    pub current_watch_channel: Option<String>,
    pub active_watch_channels: HashSet<String>,
    pub active_watch_pool: Vec<String>,
    pub last_active_namespace: String,
    pub crds: Vec<CrdMeta>,
    pub namespaces: Vec<String>,
    pub namespace_discovery_error: Option<String>,
    pub cluster_overview_data: Option<crate::views::overview_view::ClusterOverviewData>,
    pub assistant_state: crate::views::assistant_view::AssistantViewState,
    pub active_log_channel: Option<String>,
    pub node_metrics_history: std::collections::HashMap<
        String,
        std::collections::VecDeque<srelens_kube::metrics::MetricSample>,
    >,
    pub pod_metrics_history: std::collections::HashMap<
        String,
        std::collections::VecDeque<srelens_kube::metrics::MetricSample>,
    >,
}

impl ClusterPaneState {
    /// Extract the active cluster state out of `app` into a pane snapshot.
    pub fn extract_from_app(app: &mut App) -> Self {
        Self {
            context: std::mem::take(&mut app.active_context),
            namespace: std::mem::take(&mut app.active_namespace),
            cluster_name: std::mem::take(&mut app.cluster_name),
            server_url: std::mem::take(&mut app.server_url),
            cluster_version: std::mem::take(&mut app.cluster_version),
            node_count: app.node_count,
            pod_count: app.pod_count,
            is_connected: app.is_connected,
            connection_attempt_start: app.connection_attempt_start,
            cluster_unreachable: app.cluster_unreachable,
            active_view: std::mem::replace(
                &mut app.active_view,
                ActiveView::Overview(crate::views::overview_view::OverviewViewState::new()),
            ),
            nav_stack: std::mem::take(&mut app.nav_stack),
            filter_buffer: std::mem::take(&mut app.filter_buffer),
            current_watch_channel: app.current_watch_channel.take(),
            active_watch_channels: std::mem::take(&mut app.active_watch_channels),
            active_watch_pool: std::mem::take(&mut app.active_watch_pool),
            last_active_namespace: std::mem::take(&mut app.last_active_namespace),
            crds: std::mem::take(&mut app.crds),
            namespaces: std::mem::take(&mut app.namespaces),
            namespace_discovery_error: app.namespace_discovery_error.take(),
            cluster_overview_data: app.cluster_overview_data.take(),
            assistant_state: std::mem::replace(
                &mut app.assistant_state,
                crate::views::assistant_view::AssistantViewState::new(),
            ),
            active_log_channel: app.active_log_channel.take(),
            node_metrics_history: std::mem::take(&mut app.node_metrics_history),
            pod_metrics_history: std::mem::take(&mut app.pod_metrics_history),
        }
    }

    /// Apply this pane snapshot back into `app`.
    pub fn apply_to_app(self, app: &mut App) {
        app.active_context = self.context;
        app.active_namespace = self.namespace;
        app.cluster_name = self.cluster_name;
        app.server_url = self.server_url;
        app.cluster_version = self.cluster_version;
        app.node_count = self.node_count;
        app.pod_count = self.pod_count;
        app.is_connected = self.is_connected;
        app.connection_attempt_start = self.connection_attempt_start;
        app.cluster_unreachable = self.cluster_unreachable;
        app.active_view = self.active_view;
        app.nav_stack = self.nav_stack;
        app.filter_buffer = self.filter_buffer;
        app.current_watch_channel = self.current_watch_channel;
        app.active_watch_channels = self.active_watch_channels;
        app.active_watch_pool = self.active_watch_pool;
        app.last_active_namespace = self.last_active_namespace;
        app.crds = self.crds;
        app.namespaces = self.namespaces;
        app.namespace_discovery_error = self.namespace_discovery_error;
        app.cluster_overview_data = self.cluster_overview_data;
        app.assistant_state = self.assistant_state;
        app.active_log_channel = self.active_log_channel;
        app.node_metrics_history = self.node_metrics_history;
        app.pod_metrics_history = self.pod_metrics_history;
    }

    /// Swap this pane snapshot with `app`'s active cluster state in place.
    pub fn swap_with_app(&mut self, app: &mut App) {
        let mut current = Self::extract_from_app(app);
        std::mem::swap(self, &mut current);
        current.apply_to_app(app);
    }
}

/// State for the right pane context selector.
#[derive(Debug, Clone)]
pub struct ContextPickerState {
    pub contexts: Vec<ContextPickerItem>,
    pub selected_idx: usize,
    pub filter: String,
}

impl ContextPickerState {
    pub fn filtered_items(&self) -> Vec<&ContextPickerItem> {
        let lower = self.filter.to_lowercase();
        self.contexts
            .iter()
            .filter(|c| {
                if lower.is_empty() {
                    true
                } else {
                    c.name.to_lowercase().contains(&lower)
                        || c.cluster.to_lowercase().contains(&lower)
                        || c.provider
                            .as_deref()
                            .unwrap_or("")
                            .to_lowercase()
                            .contains(&lower)
                        || c.source_file.to_lowercase().contains(&lower)
                }
            })
            .collect()
    }
}

/// Overall controller state for the Mux session.
pub struct MuxState {
    /// 0 = Left Pane, 1 = Right Pane
    pub focused_pane: usize,
    /// Percentage split width (default: 50)
    pub split_ratio: u16,
    /// Layout rectangles (left, right) cached during render for mouse hit-testing
    pub pane_rects: (Rect, Rect),
    /// Active when the right pane is prompting for a context
    pub picker: Option<ContextPickerState>,
    /// The inactive cluster pane (parked while the other pane is active in `App`)
    pub inactive_pane: Option<ClusterPaneState>,
}
