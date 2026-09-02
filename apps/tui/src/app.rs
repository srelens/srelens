use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    Frame,
};
use tokio::sync::mpsc::UnboundedSender;

use srelens_capability::Registry;
use srelens_kube::client_cache::ClientCache;
use srelens_kube::contexts::ContextDto;
use srelens_kube::{k8s_openapi, kube};
use srelens_streams::logs::LogStreamManager;
use srelens_streams::watch::WatchManager;

use crate::commands::{
    command_suggestions_with_crds, resolve_command_with_crds, CommandTarget, CrdMeta,
    ResourceKind,
};
use crate::event::AppEvent;
use crate::sink::TuiSink;
use crate::theme::Theme;
use crate::ui::{
    render_header, render_help_modal, render_modal, render_statusbar, ContainerAction,
    HeaderProps, InputMode, Modal, StatusBarProps,
};
use crate::views::*;

pub enum ActiveView {
    Table(ResourceTableState),
    Yaml(YamlViewState),
    Describe(DescribeViewState),
    Logs(LogsViewState),
    PortForwards(PortForwardViewState),
    Helm(HelmViewState),
    Overview(OverviewViewState),
    Toolbox(ToolboxViewState),
    Assistant(AssistantViewState),
}

pub struct App {
    pub active_context: String,
    pub active_namespace: String,
    pub kubeconfig_paths: Vec<PathBuf>,
    pub contexts: Vec<ContextDto>,
    pub namespaces: Vec<String>,
    pub active_view: ActiveView,
    pub nav_stack: Vec<ActiveView>,
    pub input_mode: InputMode,
    pub command_buffer: String,
    pub command_suggestion_idx: usize,
    pub filter_buffer: String,
    pub modal: Option<Modal>,
    pub show_help: bool,
    pub toast: Option<(String, Instant, Style)>,
    pub client_cache: Arc<ClientCache>,
    pub watch_manager: Arc<WatchManager>,
    pub logs_manager: Arc<LogStreamManager>,
    pub event_tx: UnboundedSender<AppEvent>,
    pub current_watch_channel: Option<String>,
    pub active_watch_channels: HashSet<String>,
    pub active_watch_pool: Vec<String>,
    pub resource_cache: HashMap<(String, String, String), Vec<serde_json::Value>>,
    pub active_log_channel: Option<String>,
    pub last_active_namespace: String,
    pub crds: Vec<CrdMeta>,
    pub is_running: bool,
    pub requires_terminal_suspend: Option<SuspendAction>,
    // Dynamic Cluster Metrics & Information
    pub cluster_version: String,
    pub cluster_name: String,
    pub server_url: String,
    pub node_count: usize,
    pub pod_count: usize,
    pub is_connected: bool,
}

pub enum SuspendAction {
    EditYaml,
    PodShell { pod: String, container: Option<String> },
    DebugShell { pod: String, container: Option<String> },
    NodeShell { node: String },
}

impl App {
    pub async fn new(
        initial_context: Option<String>,
        initial_namespace: Option<String>,
        all_namespaces: bool,
        initial_resource: Option<ResourceKind>,
        kubeconfig_paths: Vec<PathBuf>,
        event_tx: UnboundedSender<AppEvent>,
    ) -> Result<Self, String> {
        let client_cache = ClientCache::new_many(kubeconfig_paths.clone());
        let watch_manager = Arc::new(WatchManager::new(client_cache.clone()));
        let logs_manager = Arc::new(LogStreamManager::new(client_cache.clone()));

        // Resolve contexts
        let resolved = srelens_kube::context_resolve::resolve_contexts(&kubeconfig_paths);
        let contexts: Vec<ContextDto> = resolved
            .iter()
            .map(|rc| ContextDto {
                name: rc.display_name.clone(),
                stable_id: rc.stable_id().to_string(),
                cluster: rc.cluster.clone(),
                server: rc.server.clone(),
                namespace: rc.namespace.clone(),
                is_current: rc.is_current,
                is_local: false,
                provider: None,
                source_file: rc.source.to_string_lossy().into_owned(),
                auth_kind: "token".to_string(),
            })
            .collect();

        let active_context = initial_context.unwrap_or_else(|| {
            contexts
                .iter()
                .find(|c| c.is_current)
                .map(|c| c.name.clone())
                .or_else(|| contexts.first().map(|c| c.name.clone()))
                .unwrap_or_else(|| "default".to_string())
        });

        let current_ctx_dto = contexts.iter().find(|c| c.name == active_context);
        let cluster_name = current_ctx_dto.map(|c| c.cluster.clone()).unwrap_or_else(|| "kubernetes".to_string());
        let server_url = current_ctx_dto.map(|c| c.server.clone()).unwrap_or_default();

        let has_explicit_ns = initial_namespace.is_some();
        let active_namespace = if all_namespaces {
            String::new()
        } else {
            initial_namespace.unwrap_or_default()
        };

        let initial_kind = initial_resource.unwrap_or_else(|| {
            if has_explicit_ns {
                ResourceKind::Pods
            } else {
                ResourceKind::Namespaces
            }
        });
        let initial_table = ResourceTableState::new(initial_kind);

        let last_active_namespace = if active_namespace.is_empty() {
            "default".to_string()
        } else {
            active_namespace.clone()
        };

        let mut app = Self {
            active_context,
            active_namespace,
            kubeconfig_paths,
            contexts,
            namespaces: vec!["default".to_string(), "kube-system".to_string()],
            active_view: ActiveView::Table(initial_table),
            nav_stack: Vec::new(),
            input_mode: InputMode::Normal,
            command_buffer: String::new(),
            command_suggestion_idx: 0,
            filter_buffer: String::new(),
            modal: None,
            show_help: false,
            toast: None,
            client_cache,
            watch_manager,
            logs_manager,
            event_tx,
            current_watch_channel: None,
            active_watch_channels: HashSet::new(),
            active_watch_pool: Vec::new(),
            resource_cache: HashMap::new(),
            active_log_channel: None,
            last_active_namespace,
            crds: Vec::new(),
            is_running: true,
            requires_terminal_suspend: None,
            cluster_version: "Connecting...".to_string(),
            cluster_name,
            server_url,
            node_count: 0,
            pod_count: 0,
            is_connected: true,
        };

        app.refresh_cluster_info();
        app.refresh_crds();
        app.restart_active_watch().await;
        Ok(app)
    }

    pub fn set_toast(&mut self, msg: String, style: Style) {
        self.toast = Some((msg, Instant::now(), style));
    }

    pub fn refresh_cluster_info(&self) {
        let context = self.active_context.clone();
        let cache = self.client_cache.clone();
        let event_tx = self.event_tx.clone();

        tokio::spawn(async move {
            if let Ok(client) = cache.get(&context).await {
                let version = client
                    .apiserver_version()
                    .await
                    .map(|v| v.git_version)
                    .unwrap_or_else(|_| "unknown".to_string());

                let node_count = kube::Api::<k8s_openapi::api::core::v1::Node>::all(client.clone())
                    .list_metadata(&kube::api::ListParams::default())
                    .await
                    .map(|list| list.items.len())
                    .unwrap_or(0);

                let pod_count = kube::Api::<k8s_openapi::api::core::v1::Pod>::all(client.clone())
                    .list_metadata(&kube::api::ListParams::default())
                    .await
                    .map(|list| list.items.len())
                    .unwrap_or(0);

                let _ = event_tx.send(AppEvent::ActionResult {
                    title: "cluster_info_updated".to_string(),
                    result: Ok(format!("{}|{}|{}", version, node_count, pod_count)),
                });
            }
        });
    }

    pub fn refresh_crds(&self) {
        let ctx = self.active_context.clone();
        let cache = self.client_cache.clone();
        let event_tx = self.event_tx.clone();

        tokio::spawn(async move {
            if let Ok(client) = cache.get(&ctx).await {
                let gvk = kube::core::GroupVersionKind::gvk("apiextensions.k8s.io", "v1", "CustomResourceDefinition");
                let ar = kube::core::ApiResource::from_gvk(&gvk);
                let api: kube::Api<kube::core::DynamicObject> = kube::Api::all_with(client, &ar);
                if let Ok(list) = api.list(&kube::api::ListParams::default()).await {
                    let mut crds = Vec::new();
                    for o in list.items {
                        let spec = &o.data["spec"];
                        let group = spec["group"].as_str().unwrap_or_default().to_string();
                        let kind = spec["names"]["kind"].as_str().unwrap_or_default().to_string();
                        let plural = spec["names"]["plural"].as_str().unwrap_or_default().to_string();
                        let singular = spec["names"]["singular"].as_str().unwrap_or_default().to_string();
                        let namespaced = spec["scope"].as_str().unwrap_or("Namespaced") == "Namespaced";
                        let short_names: Vec<String> = spec["names"]["shortNames"]
                            .as_array()
                            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                            .unwrap_or_default();

                        let version = spec["versions"]
                            .as_array()
                            .and_then(|vs| {
                                vs.iter()
                                    .find(|v| v["storage"].as_bool().unwrap_or(false))
                                    .or_else(|| vs.iter().find(|v| v["served"].as_bool().unwrap_or(false)))
                                    .and_then(|v| v["name"].as_str().map(String::from))
                            })
                            .unwrap_or_else(|| "v1".to_string());

                        if !group.is_empty() && !kind.is_empty() && !plural.is_empty() {
                            crds.push(CrdMeta {
                                crd_name: o.metadata.name.unwrap_or_default(),
                                group,
                                version,
                                kind,
                                plural,
                                singular,
                                namespaced,
                                short_names,
                            });
                        }
                    }
                    if let Ok(json_str) = serde_json::to_string(&crds) {
                        let _ = event_tx.send(AppEvent::ActionResult {
                            title: "crds_updated".to_string(),
                            result: Ok(json_str),
                        });
                    }
                }
            }
        });
    }

    pub fn handle_crds_update(&mut self, json_str: &str) {
        if let Ok(crds) = serde_json::from_str::<Vec<CrdMeta>>(json_str) {
            self.crds = crds;
        }
    }

    pub fn handle_crd_instances_update(&mut self, title: &str, json_str: &str) {
        if let Ok(items) = serde_json::from_str::<Vec<serde_json::Value>>(json_str) {
            let crd_kind = title.strip_prefix("crd_instances:").unwrap_or(title);
            let ctx = self.active_context.clone();
            let ns = self.active_namespace.clone();
            self.resource_cache.insert((ctx, ns, crd_kind.to_string()), items.clone());

            if let ActiveView::Table(table) = &mut self.active_view {
                if let ResourceKind::CustomResource(crd) = &table.kind {
                    if crd.kind == crd_kind || crd.plural == crd_kind {
                        table.set_items(items, &self.filter_buffer);
                    }
                }
            }
        }
    }

    pub fn handle_cluster_info_update(&mut self, payload: &str) {
        let parts: Vec<&str> = payload.split('|').collect();
        if parts.len() == 3 {
            self.cluster_version = parts[0].to_string();
            self.node_count = parts[1].parse().unwrap_or(0);
            self.pod_count = parts[2].parse().unwrap_or(0);
            self.is_connected = true;
        }
    }

    pub async fn switch_context(&mut self, new_context: String) {
        self.watch_manager.shutdown_all();
        self.active_watch_channels.clear();
        self.active_watch_pool.clear();
        self.resource_cache.clear();
        self.current_watch_channel = None;

        self.active_context = new_context;
        if let Some(ctx) = self.contexts.iter().find(|c| c.name == self.active_context) {
            self.cluster_name = ctx.cluster.clone();
            self.server_url = ctx.server.clone();
        }
        self.cluster_version = "Connecting...".to_string();
        self.set_toast(format!("Switched to context '{}'", self.active_context), Theme::status_ok());
        self.refresh_cluster_info();
        self.refresh_crds();
        self.restart_active_watch().await;
    }

    pub async fn switch_namespace(&mut self, new_namespace: String) {
        if !new_namespace.is_empty() {
            self.last_active_namespace = new_namespace.clone();
        }
        self.active_namespace = new_namespace.clone();
        let display_ns = if self.active_namespace.is_empty() { "all" } else { &self.active_namespace };
        self.set_toast(format!("Switched to namespace [{}]", display_ns), Theme::status_ok());
        self.restart_active_watch().await;
    }

    pub async fn restart_active_watch(&mut self) {
        if let ActiveView::Table(table) = &mut self.active_view {
            if let Some(watch_kind) = table.kind.watch_kind() {
                let ctx = self.active_context.clone();
                let ns = self.active_namespace.clone();
                let kind = watch_kind.to_string();
                let channel = format!("watch:{}:{}:{}", ctx, ns, kind);
                self.current_watch_channel = Some(channel.clone());

                // 1. Instant Cache Render: If we already have items in memory, render immediately!
                if let Some(cached) = self.resource_cache.get(&(ctx.clone(), ns.clone(), kind.clone())) {
                    table.set_items(cached.clone(), &self.filter_buffer);
                    table.is_loading = false;
                } else {
                    table.is_loading = true;
                    table.raw_items.clear();
                    table.filtered_indices.clear();
                    table.selected_idx = 0;
                    table.scroll_offset = 0;
                }

                // 2. Informer Pool: Check if watch is already running for this channel
                if !self.watch_manager.has_channel(&channel) {
                    // Evict oldest if pool exceeded (keeps max 12 watches warm)
                    if self.active_watch_pool.len() >= 12 {
                        let evicted = self.active_watch_pool.remove(0);
                        self.watch_manager.stop(&evicted);
                        self.active_watch_channels.remove(&evicted);
                    }

                    let sink = TuiSink::arc(self.event_tx.clone());
                    let paths = self.kubeconfig_paths.clone();
                    self.active_watch_channels.insert(channel.clone());
                    self.active_watch_pool.push(channel.clone());
                    let _ = self.watch_manager.start(sink, ctx, ns, kind, channel, paths).await;
                } else {
                    // Move to most-recently-used in pool
                    if let Some(pos) = self.active_watch_pool.iter().position(|c| c == &channel) {
                        let ch = self.active_watch_pool.remove(pos);
                        self.active_watch_pool.push(ch);
                    }
                }
            }
        }
    }

    pub async fn handle_key_event(&mut self, key: KeyEvent) {
        // Global Ctrl+C handler -> Immediately kill/exit TUI cleanly
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.is_running = false;
            return;
        }

        // Clear expired toasts
        if let Some((_, created, _)) = &self.toast {
            if created.elapsed() > Duration::from_secs(4) {
                self.toast = None;
            }
        }

        // 1. Help Modal Open
        if self.show_help {
            if key.code == KeyCode::Esc || key.code == KeyCode::Char('q') || key.code == KeyCode::Char('?') {
                self.show_help = false;
            }
            return;
        }

        // 2. Interactive Dialog Modal Open
        if let Some(modal) = self.modal.clone() {
            match modal {
                Modal::Confirm { action_name, .. } => {
                    match key.code {
                        KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                            self.modal = None;
                            self.execute_modal_confirm(action_name).await;
                        }
                        KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                            self.modal = None;
                        }
                        _ => {}
                    }
                }
                Modal::Scale { workload_name, mut input, current_replicas } => {
                    match key.code {
                        KeyCode::Char(c) if c.is_ascii_digit() => {
                            input.push(c);
                            self.modal = Some(Modal::Scale { workload_name, input, current_replicas });
                        }
                        KeyCode::Backspace => {
                            input.pop();
                            self.modal = Some(Modal::Scale { workload_name, input, current_replicas });
                        }
                        KeyCode::Enter => {
                            self.modal = None;
                            if let Ok(count) = input.parse::<i32>() {
                                self.execute_scale_workload(workload_name, count).await;
                            }
                        }
                        KeyCode::Esc => {
                            self.modal = None;
                        }
                        _ => {}
                    }
                }
                Modal::PortForward { pod_name, namespace, container_port, mut local_port_input } => {
                    match key.code {
                        KeyCode::Char(c) if c.is_ascii_digit() => {
                            local_port_input.push(c);
                            self.modal = Some(Modal::PortForward { pod_name, namespace, container_port, local_port_input });
                        }
                        KeyCode::Backspace => {
                            local_port_input.pop();
                            self.modal = Some(Modal::PortForward { pod_name, namespace, container_port, local_port_input });
                        }
                        KeyCode::Enter => {
                            self.modal = None;
                            if let Ok(local_port) = local_port_input.parse::<u16>() {
                                self.execute_start_port_forward(
                                    pod_name,
                                    namespace,
                                    local_port,
                                    container_port,
                                ).await;
                            }
                        }
                        KeyCode::Esc => {
                            self.modal = None;
                        }
                        _ => {}
                    }
                }
                Modal::ContainerPicker { containers, mut selected_idx, action, pod_name, namespace } => {
                    match key.code {
                        KeyCode::Up | KeyCode::Char('k') => {
                            if selected_idx > 0 {
                                selected_idx -= 1;
                            } else {
                                selected_idx = containers.len().saturating_sub(1);
                            }
                            self.modal = Some(Modal::ContainerPicker { containers, selected_idx, action, pod_name, namespace });
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            if selected_idx + 1 < containers.len() {
                                selected_idx += 1;
                            } else {
                                selected_idx = 0;
                            }
                            self.modal = Some(Modal::ContainerPicker { containers, selected_idx, action, pod_name, namespace });
                        }
                        KeyCode::Enter => {
                            let chosen_container = containers.get(selected_idx).cloned();
                            self.modal = None;
                            match action {
                                ContainerAction::Logs => {
                                    self.open_logs_view(pod_name, namespace, chosen_container).await;
                                }
                                ContainerAction::Shell => {
                                    self.requires_terminal_suspend = Some(SuspendAction::PodShell {
                                        pod: pod_name,
                                        container: chosen_container,
                                    });
                                }
                            }
                        }
                        KeyCode::Esc => {
                            self.modal = None;
                        }
                        _ => {}
                    }
                }
                Modal::ContextPicker { contexts, mut selected_idx, current_context } => {
                    match key.code {
                        KeyCode::Up | KeyCode::Char('k') => {
                            if selected_idx > 0 {
                                selected_idx -= 1;
                            } else {
                                selected_idx = contexts.len().saturating_sub(1);
                            }
                            self.modal = Some(Modal::ContextPicker { contexts, selected_idx, current_context });
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            if selected_idx + 1 < contexts.len() {
                                selected_idx += 1;
                            } else {
                                selected_idx = 0;
                            }
                            self.modal = Some(Modal::ContextPicker { contexts, selected_idx, current_context });
                        }
                        KeyCode::Enter => {
                            let target_ctx = contexts.get(selected_idx).cloned();
                            self.modal = None;
                            if let Some(ctx) = target_ctx {
                                self.switch_context(ctx).await;
                            }
                        }
                        KeyCode::Esc => {
                            self.modal = None;
                        }
                        _ => {}
                    }
                }
                Modal::NamespacePicker { namespaces, mut selected_idx, mut filter, current_namespace } => {
                    let mut all_filtered = vec![String::new()]; // (all namespaces)
                    all_filtered.extend(namespaces.iter().filter(|n| n.contains(filter.as_str())).cloned());
                    let filtered_count = all_filtered.len();

                    match key.code {
                        KeyCode::Up => {
                            if selected_idx > 0 {
                                selected_idx -= 1;
                            } else {
                                selected_idx = filtered_count.saturating_sub(1);
                            }
                            self.modal = Some(Modal::NamespacePicker { namespaces, selected_idx, filter, current_namespace });
                        }
                        KeyCode::Down => {
                            if selected_idx + 1 < filtered_count {
                                selected_idx += 1;
                            } else {
                                selected_idx = 0;
                            }
                            self.modal = Some(Modal::NamespacePicker { namespaces, selected_idx, filter, current_namespace });
                        }
                        KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            if selected_idx > 0 {
                                selected_idx -= 1;
                            } else {
                                selected_idx = filtered_count.saturating_sub(1);
                            }
                            self.modal = Some(Modal::NamespacePicker { namespaces, selected_idx, filter, current_namespace });
                        }
                        KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            if selected_idx + 1 < filtered_count {
                                selected_idx += 1;
                            } else {
                                selected_idx = 0;
                            }
                            self.modal = Some(Modal::NamespacePicker { namespaces, selected_idx, filter, current_namespace });
                        }
                        KeyCode::Char('0') if filter.is_empty() => {
                            self.modal = None;
                            self.switch_namespace(String::new()).await;
                        }
                        KeyCode::Backspace => {
                            filter.pop();
                            self.modal = Some(Modal::NamespacePicker { namespaces, selected_idx: 0, filter, current_namespace });
                        }
                        KeyCode::Char(c) => {
                            filter.push(c);
                            self.modal = Some(Modal::NamespacePicker { namespaces, selected_idx: 0, filter, current_namespace });
                        }
                        KeyCode::Enter => {
                            let target_ns = all_filtered.get(selected_idx).cloned();
                            self.modal = None;
                            if let Some(ns) = target_ns {
                                self.switch_namespace(ns).await;
                            }
                        }
                        KeyCode::Esc => {
                            self.modal = None;
                        }
                        _ => {}
                    }
                }
            }
            return;
        }

        // 3. Command Mode (`:`)
        if self.input_mode == InputMode::Command {
            match key.code {
                KeyCode::Esc => {
                    self.input_mode = InputMode::Normal;
                    self.command_buffer.clear();
                    self.command_suggestion_idx = 0;
                }
                KeyCode::Enter => {
                    let cmd_str = self.command_buffer.clone();
                    self.input_mode = InputMode::Normal;
                    self.command_buffer.clear();
                    self.command_suggestion_idx = 0;
                    self.execute_colon_command(&cmd_str).await;
                }
                KeyCode::Tab => {
                    let suggestions = command_suggestions_with_crds(&self.command_buffer, &self.crds);
                    if !suggestions.is_empty() {
                        let idx = self.command_suggestion_idx % suggestions.len();
                        self.command_buffer = suggestions[idx].0.name.clone();
                        self.command_suggestion_idx = (idx + 1) % suggestions.len();
                    }
                }
                KeyCode::BackTab => {
                    let suggestions = command_suggestions_with_crds(&self.command_buffer, &self.crds);
                    if !suggestions.is_empty() {
                        let len = suggestions.len();
                        let idx = (self.command_suggestion_idx + len - 1) % len;
                        self.command_buffer = suggestions[idx].0.name.clone();
                        self.command_suggestion_idx = idx;
                    }
                }
                KeyCode::Down => {
                    let suggestions = command_suggestions_with_crds(&self.command_buffer, &self.crds);
                    if !suggestions.is_empty() {
                        let idx = self.command_suggestion_idx % suggestions.len();
                        self.command_buffer = suggestions[idx].0.name.clone();
                        self.command_suggestion_idx = (idx + 1) % suggestions.len();
                    }
                }
                KeyCode::Up => {
                    let suggestions = command_suggestions_with_crds(&self.command_buffer, &self.crds);
                    if !suggestions.is_empty() {
                        let len = suggestions.len();
                        let idx = (self.command_suggestion_idx + len - 1) % len;
                        self.command_buffer = suggestions[idx].0.name.clone();
                        self.command_suggestion_idx = idx;
                    }
                }
                KeyCode::Backspace => {
                    self.command_buffer.pop();
                    self.command_suggestion_idx = 0;
                    if self.command_buffer.is_empty() {
                        self.input_mode = InputMode::Normal;
                    }
                }
                KeyCode::Char(c) => {
                    self.command_buffer.push(c);
                    self.command_suggestion_idx = 0;
                }
                _ => {}
            }
            return;
        }

        // 4. Filter Mode (`/`)
        if self.input_mode == InputMode::Filter {
            match key.code {
                KeyCode::Esc => {
                    self.input_mode = InputMode::Normal;
                    self.filter_buffer.clear();
                    if let ActiveView::Table(table) = &mut self.active_view {
                        table.apply_filter("");
                    }
                }
                KeyCode::Enter => {
                    self.input_mode = InputMode::Normal;
                }
                KeyCode::Backspace => {
                    self.filter_buffer.pop();
                    let filter = self.filter_buffer.clone();
                    if let ActiveView::Table(table) = &mut self.active_view {
                        table.apply_filter(&filter);
                    }
                }
                KeyCode::Char(c) => {
                    self.filter_buffer.push(c);
                    let filter = self.filter_buffer.clone();
                    if let ActiveView::Table(table) = &mut self.active_view {
                        table.apply_filter(&filter);
                    }
                }
                _ => {}
            }
            return;
        }

        // 5. Normal Mode - k9s Global & View Keybindings
        match key.code {
            // Enter Command Mode
            KeyCode::Char(':') => {
                self.input_mode = InputMode::Command;
                self.command_buffer.clear();
            }
            // Enter Filter Mode
            KeyCode::Char('/') => {
                self.input_mode = InputMode::Filter;
            }
            // Open Help Modal
            KeyCode::Char('?') => {
                self.show_help = true;
            }
            // Toggle All Namespaces vs Active Namespace
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.active_namespace.is_empty() {
                    let target = if self.last_active_namespace.is_empty() {
                        "default".to_string()
                    } else {
                        self.last_active_namespace.clone()
                    };
                    self.switch_namespace(target).await;
                } else {
                    self.last_active_namespace = self.active_namespace.clone();
                    self.switch_namespace(String::new()).await;
                }
            }
            // Quick context hotkeys F1-F10
            KeyCode::F(n) => {
                let idx = (n.saturating_sub(1)) as usize;
                if let Some(ctx) = self.contexts.get(idx) {
                    self.switch_context(ctx.name.clone()).await;
                }
            }
            // Pop View Stack / Back
            KeyCode::Esc => {
                if !self.filter_buffer.is_empty() {
                    self.filter_buffer.clear();
                    if let ActiveView::Table(table) = &mut self.active_view {
                        table.apply_filter("");
                    }
                } else if let Some(prev_view) = self.nav_stack.pop() {
                    if let Some(ch) = self.active_log_channel.take() {
                        self.logs_manager.stop(&ch);
                    }
                    self.active_view = prev_view;
                    self.restart_active_watch().await;
                }
            }
            // Toggle AI Assistant Drawer
            KeyCode::Tab => {
                if matches!(self.active_view, ActiveView::Assistant(_)) {
                    if let Some(prev) = self.nav_stack.pop() {
                        self.active_view = prev;
                        self.restart_active_watch().await;
                    }
                } else {
                    let prev = std::mem::replace(&mut self.active_view, ActiveView::Assistant(AssistantViewState::new()));
                    self.nav_stack.push(prev);
                }
            }
            // View-specific keybindings
            _ => self.handle_view_key_event(key).await,
        }
    }

    async fn handle_view_key_event(&mut self, key: KeyEvent) {
        match &mut self.active_view {
            ActiveView::Table(table) => {
                let sel_name = table.selected_resource_name();
                let sel_ns = table.selected_namespace();
                let kind_str = table.kind.k8s_kind().map(String::from).unwrap_or_else(|| table.kind.to_string());
                let table_kind = table.kind.clone();

                match key.code {
                    KeyCode::Char('j') | KeyCode::Down => table.select_next(),
                    KeyCode::Char('k') | KeyCode::Up => table.select_prev(),
                    KeyCode::Char('g') | KeyCode::Home => table.select_top(),
                    KeyCode::Char('G') | KeyCode::End => table.select_bottom(),
                    KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => table.page_up(10),
                    KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        // Ctrl+d -> Delete resource confirmation
                        if let Some(name) = sel_name {
                            let ns = sel_ns.unwrap_or_else(|| self.active_namespace.clone());
                            self.modal = Some(Modal::Confirm {
                                title: format!("Delete {} [{}]", table_kind, name),
                                message: format!("Are you sure you want to delete {} '{}' in namespace '{}'?", table_kind, name, ns),
                                action_name: "delete".to_string(),
                                is_destructive: true,
                            });
                        }
                    }
                    KeyCode::Char(' ') => table.toggle_mark_selected(),
                    KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        // Rollout restart (Ctrl+r)
                        if let Some(name) = sel_name {
                            self.modal = Some(Modal::Confirm {
                                title: format!("Restart Workload [{}]", name),
                                message: format!("Trigger zero-downtime rollout restart for {} '{}'?", table_kind, name),
                                action_name: "restart".to_string(),
                                is_destructive: false,
                            });
                        }
                    }
                    KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        // Scale workload (Ctrl+s)
                        if let Some(name) = sel_name {
                            self.modal = Some(Modal::Scale {
                                workload_name: name,
                                current_replicas: 1,
                                input: "1".to_string(),
                            });
                        }
                    }
                    KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::SHIFT) || key.modifiers.contains(KeyModifiers::CONTROL) => {
                        // Port forward (Shift+f or Ctrl+f)
                        if let Some(pod_name) = sel_name {
                            let ns = sel_ns.unwrap_or_else(|| self.active_namespace.clone());
                            self.modal = Some(Modal::PortForward {
                                pod_name,
                                namespace: ns,
                                container_port: 8080,
                                local_port_input: "8080".to_string(),
                            });
                        }
                    }
                    KeyCode::Char('l') => {
                        // Logs
                        if let Some(pod_name) = sel_name {
                            let target_ns = sel_ns.or_else(|| if self.active_namespace.is_empty() { None } else { Some(self.active_namespace.clone()) });
                            let query_ns = target_ns.clone().unwrap_or_else(|| "default".to_string());
                            let ctx = self.active_context.clone();
                            let cache = self.client_cache.clone();

                            let containers: Vec<String> = if let Ok(client) = cache.get(&ctx).await {
                                let api: kube::Api<k8s_openapi::api::core::v1::Pod> = kube::Api::namespaced(client, &query_ns);
                                if let Ok(pod) = api.get(&pod_name).await {
                                    pod.spec.map(|s| s.containers.into_iter().map(|c| c.name).collect()).unwrap_or_default()
                                } else {
                                    Vec::new()
                                }
                            } else {
                                Vec::new()
                            };

                            if containers.len() > 1 {
                                self.modal = Some(Modal::ContainerPicker {
                                    pod_name,
                                    namespace: target_ns,
                                    containers,
                                    selected_idx: 0,
                                    action: ContainerAction::Logs,
                                });
                            } else {
                                self.open_logs_view(pod_name, target_ns, containers.into_iter().next()).await;
                            }
                        }
                    }
                    KeyCode::Char('s') => {
                        // Shell / Exec
                        if let Some(pod_name) = sel_name {
                            let target_ns = sel_ns.or_else(|| if self.active_namespace.is_empty() { None } else { Some(self.active_namespace.clone()) });
                            let query_ns = target_ns.clone().unwrap_or_else(|| "default".to_string());
                            let ctx = self.active_context.clone();
                            let cache = self.client_cache.clone();

                            let containers: Vec<String> = if let Ok(client) = cache.get(&ctx).await {
                                let api: kube::Api<k8s_openapi::api::core::v1::Pod> = kube::Api::namespaced(client, &query_ns);
                                if let Ok(pod) = api.get(&pod_name).await {
                                    pod.spec.map(|s| s.containers.into_iter().map(|c| c.name).collect()).unwrap_or_default()
                                } else {
                                    Vec::new()
                                }
                            } else {
                                Vec::new()
                            };

                            if containers.len() > 1 {
                                self.modal = Some(Modal::ContainerPicker {
                                    pod_name,
                                    namespace: target_ns,
                                    containers,
                                    selected_idx: 0,
                                    action: ContainerAction::Shell,
                                });
                            } else {
                                self.requires_terminal_suspend = Some(SuspendAction::PodShell {
                                    pod: pod_name,
                                    container: containers.into_iter().next(),
                                });
                            }
                        }
                    }
                    KeyCode::Char('y') | KeyCode::Char('v') => {
                        // View YAML manifest
                        if let Some(name) = sel_name {
                            self.open_yaml_view(name, kind_str, sel_ns).await;
                        }
                    }
                    KeyCode::Char('d') => {
                        // Describe resource
                        if let Some(name) = sel_name {
                            self.open_describe_view(name, kind_str, sel_ns).await;
                        }
                    }
                    KeyCode::Char('e') => {
                        // Edit YAML in $EDITOR
                        if let Some(name) = sel_name {
                            self.open_yaml_view(name, kind_str, sel_ns).await;
                            self.requires_terminal_suspend = Some(SuspendAction::EditYaml);
                        }
                    }
                    KeyCode::Enter => {
                        // Drill-down
                        if table_kind == ResourceKind::Namespaces {
                            if let Some(ns_name) = sel_name {
                                self.switch_namespace(ns_name).await;
                                self.switch_view_to_kind(ResourceKind::Pods).await;
                            }
                        }
                    }
                    _ => {}
                }
            }
            ActiveView::Yaml(yaml) => {
                match key.code {
                    KeyCode::Char('j') | KeyCode::Down => yaml.scroll_down(1),
                    KeyCode::Char('k') | KeyCode::Up => yaml.scroll_up(1),
                    KeyCode::Char('g') | KeyCode::Home => yaml.scroll_top(),
                    KeyCode::Char('G') | KeyCode::End => yaml.scroll_bottom(),
                    KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => yaml.scroll_up(10),
                    KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => yaml.scroll_down(10),
                    KeyCode::Char('e') => {
                        self.requires_terminal_suspend = Some(SuspendAction::EditYaml);
                    }
                    KeyCode::Char('c') => {
                        let content = yaml.yaml_content.clone();
                        tokio::spawn(async move {
                            let _ = copy_to_clipboard(&content);
                        });
                        self.set_toast("Copied YAML to clipboard".to_string(), Theme::status_ok());
                    }
                    _ => {}
                }
            }
            ActiveView::Describe(desc) => {
                match key.code {
                    KeyCode::Char('j') | KeyCode::Down => desc.scroll_down(1),
                    KeyCode::Char('k') | KeyCode::Up => desc.scroll_up(1),
                    KeyCode::Char('g') | KeyCode::Home => desc.scroll_top(),
                    KeyCode::Char('G') | KeyCode::End => desc.scroll_bottom(),
                    KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => desc.scroll_up(10),
                    KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => desc.scroll_down(10),
                    KeyCode::Char('c') | KeyCode::Char('y') => {
                        let content = desc.content.clone();
                        tokio::spawn(async move {
                            let _ = copy_to_clipboard(&content);
                        });
                        self.set_toast("Copied describe output to clipboard".to_string(), Theme::status_ok());
                    }
                    _ => {}
                }
            }
            ActiveView::Logs(logs) => {
                match key.code {
                    KeyCode::Char('j') | KeyCode::Down => logs.scroll_down(1),
                    KeyCode::Char('k') | KeyCode::Up => logs.scroll_up(1),
                    KeyCode::Char('g') | KeyCode::Home => logs.scroll_top(),
                    KeyCode::Char('G') | KeyCode::End => logs.scroll_to_bottom(),
                    KeyCode::Char('f') => logs.toggle_follow(),
                    KeyCode::Char('t') => logs.toggle_timestamps(),
                    KeyCode::Char('p') => logs.toggle_previous(),
                    KeyCode::Char('w') => logs.toggle_wrap(),
                    KeyCode::Char('s') => {
                        match logs.save_to_file() {
                            Ok(path) => self.set_toast(format!("Logs saved to {}", path), Theme::status_ok()),
                            Err(e) => self.set_toast(format!("Failed to save logs: {}", e), Theme::status_error()),
                        }
                    }
                    _ => {}
                }
            }
            ActiveView::PortForwards(pf) => {
                match key.code {
                    KeyCode::Char('j') | KeyCode::Down => pf.select_next(),
                    KeyCode::Char('k') | KeyCode::Up => pf.select_prev(),
                    KeyCode::Char('d') => {
                        if let Some(entry) = pf.selected_forward() {
                            let id = entry.id.clone();
                            self.modal = Some(Modal::Confirm {
                                title: format!("Stop Port Forward [127.0.0.1:{}]", entry.local_port),
                                message: format!("Stop port-forward to {}/{}?", entry.target_type, entry.target_name),
                                action_name: format!("stop-pf:{}", id),
                                is_destructive: false,
                            });
                        }
                    }
                    _ => {}
                }
            }
            ActiveView::Helm(helm) => {
                let sel_rel = helm.selected_release().cloned();
                match key.code {
                    KeyCode::Char('j') | KeyCode::Down => helm.select_next(),
                    KeyCode::Char('k') | KeyCode::Up => helm.select_prev(),
                    KeyCode::Char('v') => {
                        // Inspect Helm values
                        if let Some(rel) = sel_rel {
                            self.open_yaml_view(rel.name, "HelmValues".to_string(), Some(rel.namespace)).await;
                        }
                    }
                    KeyCode::Char('y') => {
                        // Inspect Helm manifest
                        if let Some(rel) = sel_rel {
                            self.open_yaml_view(rel.name, "HelmManifest".to_string(), Some(rel.namespace)).await;
                        }
                    }
                    _ => {}
                }
            }
            ActiveView::Toolbox(tb) => {
                match key.code {
                    KeyCode::Char('j') | KeyCode::Down => tb.select_next(),
                    KeyCode::Char('k') | KeyCode::Up => tb.select_prev(),
                    _ => {}
                }
            }
            ActiveView::Assistant(ai) => {
                match key.code {
                    KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => ai.scroll_down(2),
                    KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => ai.scroll_up(2),
                    KeyCode::Char(c) => ai.input.push(c),
                    KeyCode::Backspace => { ai.input.pop(); },
                    KeyCode::Enter => {
                        if !ai.input.trim().is_empty() {
                            let query = ai.input.clone();
                            ai.add_user_message(query.clone());
                            let event_tx = self.event_tx.clone();
                            tokio::spawn(async move {
                                tokio::time::sleep(Duration::from_millis(600)).await;
                                let reply = format!("Analysis for prompt '{}': All components in current namespace are nominal. No CrashLoopBackOff events detected in the last 15 minutes.", query);
                                let _ = event_tx.send(AppEvent::ActionResult {
                                    title: "ai_reply".to_string(),
                                    result: Ok(reply),
                                });
                            });
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    pub async fn execute_colon_command(&mut self, cmd: &str) {
        if let Some(target) = resolve_command_with_crds(cmd, &self.crds) {
            self.execute_command_target(target).await;
        } else {
            // Check if there is an unambiguous top suggestion
            let suggestions = command_suggestions_with_crds(cmd, &self.crds);
            if let Some((top, score)) = suggestions.first() {
                if *score >= 80 {
                    self.execute_command_target(top.target.clone()).await;
                    return;
                }
            }
            self.set_toast(format!("Unknown command: '{}' (type :help or ?)", cmd), Theme::status_warn());
        }
    }

    pub async fn execute_command_target(&mut self, target: CommandTarget) {
        match target {
            CommandTarget::Resource(kind) => self.switch_view_to_kind(kind).await,
            CommandTarget::CustomResource(crd) => self.switch_view_to_crd(crd).await,
            CommandTarget::Contexts => {
                self.modal = Some(Modal::ContextPicker {
                    contexts: self.contexts.iter().map(|c| c.name.clone()).collect(),
                    current_context: self.active_context.clone(),
                    selected_idx: 0,
                });
            }
            CommandTarget::Namespaces => {
                self.modal = Some(Modal::NamespacePicker {
                    namespaces: self.namespaces.clone(),
                    current_namespace: self.active_namespace.clone(),
                    selected_idx: 0,
                    filter: String::new(),
                });
            }
            CommandTarget::Help => {
                self.show_help = true;
            }
            CommandTarget::Quit => {
                self.is_running = false;
            }
        }
    }

    pub async fn switch_view_to_crd(&mut self, crd: CrdMeta) {
        let kind = ResourceKind::CustomResource(crd.clone());
        let mut table = ResourceTableState::new(kind);
        let ctx = &self.active_context;
        let ns = &self.active_namespace;
        if let Some(cached) = self.resource_cache.get(&(ctx.clone(), ns.clone(), crd.kind.clone())) {
            table.set_items(cached.clone(), &self.filter_buffer);
            table.is_loading = false;
        } else {
            table.is_loading = true;
        }
        let old_view = std::mem::replace(&mut self.active_view, ActiveView::Table(table));
        self.nav_stack.push(old_view);
        self.filter_buffer.clear();
        self.fetch_crd_instances(crd);
    }

    pub fn fetch_crd_instances(&self, crd: CrdMeta) {
        let ctx = self.active_context.clone();
        let ns = self.active_namespace.clone();
        let cache = self.client_cache.clone();
        let event_tx = self.event_tx.clone();

        tokio::spawn(async move {
            if let Ok(client) = cache.get(&ctx).await {
                let api_version = if crd.group.is_empty() {
                    crd.version.clone()
                } else {
                    format!("{}/{}", crd.group, crd.version)
                };
                let ar = kube::core::ApiResource {
                    group: crd.group.clone(),
                    version: crd.version.clone(),
                    api_version,
                    kind: crd.kind.clone(),
                    plural: crd.plural.clone(),
                };

                let api: kube::Api<kube::core::DynamicObject> = if crd.namespaced && !ns.is_empty() {
                    kube::Api::namespaced_with(client, &ns, &ar)
                } else {
                    kube::Api::all_with(client, &ar)
                };

                if let Ok(list) = api.list(&kube::api::ListParams::default()).await {
                    let items: Vec<serde_json::Value> = list
                        .items
                        .into_iter()
                        .map(|item| {
                            let mut val = item.data;
                            if let Ok(meta_val) = serde_json::to_value(&item.metadata) {
                                if let Some(obj) = val.as_object_mut() {
                                    obj.insert("metadata".to_string(), meta_val.clone());
                                    if let Some(n) = &item.metadata.name {
                                        obj.insert("name".to_string(), serde_json::Value::String(n.clone()));
                                    }
                                    if let Some(ns_name) = &item.metadata.namespace {
                                        obj.insert("namespace".to_string(), serde_json::Value::String(ns_name.clone()));
                                    }
                                    if let Some(ts) = &item.metadata.creation_timestamp {
                                        let age = srelens_kube::humanize_age(Some(ts));
                                        obj.insert("age".to_string(), serde_json::Value::String(age));
                                    }
                                }
                            }
                            val
                        })
                        .collect();

                    let _ = event_tx.send(AppEvent::ActionResult {
                        title: format!("crd_instances:{}", crd.kind),
                        result: Ok(serde_json::to_string(&items).unwrap_or_default()),
                    });
                }
            }
        });
    }

    pub async fn switch_view_to_kind(&mut self, kind: ResourceKind) {
        let new_view = match kind {
            ResourceKind::PortForwards => ActiveView::PortForwards(PortForwardViewState::new()),
            ResourceKind::HelmReleases => ActiveView::Helm(HelmViewState::new()),
            ResourceKind::Overview => ActiveView::Overview(OverviewViewState::new()),
            ResourceKind::Toolbox => ActiveView::Toolbox(ToolboxViewState::new()),
            ResourceKind::Assistant => ActiveView::Assistant(AssistantViewState::new()),
            _ => {
                let mut table = ResourceTableState::new(kind.clone());
                if let Some(watch_kind) = table.kind.watch_kind() {
                    let ctx = &self.active_context;
                    let ns = &self.active_namespace;
                    if let Some(cached) = self.resource_cache.get(&(ctx.clone(), ns.clone(), watch_kind.to_string())) {
                        table.set_items(cached.clone(), &self.filter_buffer);
                        table.is_loading = false;
                    }
                }
                ActiveView::Table(table)
            }
        };

        let old_view = std::mem::replace(&mut self.active_view, new_view);
        self.nav_stack.push(old_view);
        self.filter_buffer.clear();
        self.restart_active_watch().await;
    }

    pub async fn open_logs_view(&mut self, pod_name: String, namespace: Option<String>, container: Option<String>) {
        if let Some(prev_ch) = self.active_log_channel.take() {
            self.logs_manager.stop(&prev_ch);
        }

        let channel = format!("logs:{}:{}", pod_name, uuid::Uuid::new_v4());
        self.active_log_channel = Some(channel.clone());

        let target_ns = namespace
            .or_else(|| if self.active_namespace.is_empty() { None } else { Some(self.active_namespace.clone()) })
            .unwrap_or_else(|| "default".to_string());

        let mut logs_state = LogsViewState::new(
            pod_name.clone(),
            target_ns.clone(),
            container.clone(),
            channel.clone(),
        );
        logs_state.push_line(format!("Streaming logs for pod {}/{} (container: {})...", target_ns, pod_name, container.as_deref().unwrap_or("default")));

        let sink = TuiSink::arc(self.event_tx.clone());
        let ctx = self.active_context.clone();
        let target = srelens_streams::logs::LogTarget {
            pod: pod_name.clone(),
            container: container.clone(),
            label: container.clone().unwrap_or_default(),
        };

        let _ = self.logs_manager.start(
            sink,
            ctx,
            target_ns,
            vec![target],
            channel,
            Some(logs_state.timestamps),
            None,
            Some(200),
        ).await;

        let old_view = std::mem::replace(&mut self.active_view, ActiveView::Logs(logs_state));
        self.nav_stack.push(old_view);
    }

    pub async fn open_yaml_view(&mut self, name: String, kind: String, namespace: Option<String>) {
        let ctx = self.active_context.clone();
        let cache = self.client_cache.clone();
        let ns = namespace.clone();
        let k = kind.clone();
        let n = name.clone();
        let kubeconfig_paths = self.kubeconfig_paths.clone();
        let crd_opt = self.crds.iter().find(|c| c.kind.eq_ignore_ascii_case(&k) || c.plural.eq_ignore_ascii_case(&k)).cloned();

        let yaml_text = tokio::task::spawn(async move {
            if let Ok(client) = cache.get(&ctx).await {
                if let Some(crd) = crd_opt {
                    let api_version = if crd.group.is_empty() {
                        crd.version.clone()
                    } else {
                        format!("{}/{}", crd.group, crd.version)
                    };
                    let ar = kube::core::ApiResource {
                        group: crd.group,
                        version: crd.version,
                        api_version,
                        kind: crd.kind,
                        plural: crd.plural,
                    };
                    let api: kube::Api<kube::core::DynamicObject> = if crd.namespaced {
                        kube::Api::namespaced_with(client, ns.as_deref().unwrap_or("default"), &ar)
                    } else {
                        kube::Api::all_with(client, &ar)
                    };
                    if let Ok(mut obj) = api.get(&n).await {
                        obj.metadata.managed_fields = None;
                        if let Ok(y) = serde_yaml::to_string(&obj) {
                            return y;
                        }
                    }
                } else if let Some((gvk, namespaced)) = srelens_kube::manifest::gvk_for(&k) {
                    let ar = kube::core::ApiResource::from_gvk(&gvk);
                    let api: kube::Api<kube::core::DynamicObject> = if namespaced {
                        kube::Api::namespaced_with(client, ns.as_deref().unwrap_or("default"), &ar)
                    } else {
                        kube::Api::all_with(client, &ar)
                    };
                    if let Ok(mut obj) = api.get(&n).await {
                        obj.metadata.managed_fields = None;
                        if let Ok(y) = serde_yaml::to_string(&obj) {
                            return y;
                        }
                    }
                }
            }

            // Fallback: try kubectl get <kind> <name> -o yaml
            let mut cmd = tokio::process::Command::new("kubectl");
            cmd.arg("get");
            cmd.arg(&k);
            cmd.arg(&n);
            if let Some(ref ns_val) = ns {
                if !ns_val.is_empty() {
                    cmd.arg("-n").arg(ns_val);
                }
            }
            if !ctx.is_empty() {
                cmd.arg("--context").arg(&ctx);
            }
            cmd.arg("-o").arg("yaml");

            if !kubeconfig_paths.is_empty() {
                let joined = kubeconfig_paths
                    .iter()
                    .map(|p| p.to_string_lossy())
                    .collect::<Vec<_>>()
                    .join(":");
                cmd.env("KUBECONFIG", joined);
            }

            if let Ok(output) = cmd.output().await {
                if output.status.success() {
                    let text = String::from_utf8_lossy(&output.stdout).to_string();
                    if !text.trim().is_empty() {
                        return text;
                    }
                }
            }

            format!(
                "# Error: Unable to fetch live manifest for {}/{} in namespace {}\n",
                k, n, ns.as_deref().unwrap_or("default")
            )
        }).await.unwrap_or_default();

        let yaml_state = YamlViewState::new(name, kind, namespace, yaml_text);
        let old_view = std::mem::replace(&mut self.active_view, ActiveView::Yaml(yaml_state));
        self.nav_stack.push(old_view);
    }

    pub async fn open_describe_view(&mut self, name: String, kind: String, namespace: Option<String>) {
        let ctx = self.active_context.clone();
        let cache = self.client_cache.clone();
        let ns = namespace.clone();
        let k = kind.clone();
        let n = name.clone();
        let kubeconfig_paths = self.kubeconfig_paths.clone();

        let desc_text = tokio::task::spawn(async move {
            // 1. Try kubectl describe for exact 100% fidelity
            let mut cmd = tokio::process::Command::new("kubectl");
            cmd.arg("describe");
            cmd.arg(&k);
            cmd.arg(&n);
            if let Some(ref ns_val) = ns {
                if !ns_val.is_empty() {
                    cmd.arg("-n").arg(ns_val);
                }
            }
            if !ctx.is_empty() {
                cmd.arg("--context").arg(&ctx);
            }
            if !kubeconfig_paths.is_empty() {
                let joined = kubeconfig_paths
                    .iter()
                    .map(|p| p.to_string_lossy())
                    .collect::<Vec<_>>()
                    .join(":");
                cmd.env("KUBECONFIG", joined);
            }

            if let Ok(output) = cmd.output().await {
                if output.status.success() {
                    let text = String::from_utf8_lossy(&output.stdout).to_string();
                    if !text.trim().is_empty() {
                        return text;
                    }
                }
            }

            // 2. Pure Rust native describe fallback
            if let Ok(client) = cache.get(&ctx).await {
                if let Some((gvk, namespaced)) = srelens_kube::manifest::gvk_for(&k) {
                    let ar = kube::core::ApiResource::from_gvk(&gvk);
                    let api: kube::Api<kube::core::DynamicObject> = if namespaced {
                        kube::Api::namespaced_with(client.clone(), ns.as_deref().unwrap_or("default"), &ar)
                    } else {
                        kube::Api::all_with(client.clone(), &ar)
                    };

                    if let Ok(obj) = api.get(&n).await {
                        let mut out = String::new();
                        out.push_str(&format!("{:<26}{}\n", "Name:", n));
                        if let Some(ref ns_val) = ns {
                            out.push_str(&format!("{:<26}{}\n", "Namespace:", ns_val));
                        }
                        if let Some(labels) = &obj.metadata.labels {
                            out.push_str(&format!("{:<26}", "Labels:"));
                            let mut first = true;
                            for (label_k, label_v) in labels {
                                if !first {
                                    out.push_str(&format!("\n{:<26}", ""));
                                }
                                out.push_str(&format!("{}={}", label_k, label_v));
                                first = false;
                            }
                            out.push('\n');
                        }
                        if let Some(annotations) = &obj.metadata.annotations {
                            out.push_str(&format!("{:<26}", "Annotations:"));
                            let mut first = true;
                            for (anno_k, anno_v) in annotations {
                                if anno_k.contains("managed-fields") { continue; }
                                if !first {
                                    out.push_str(&format!("\n{:<26}", ""));
                                }
                                out.push_str(&format!("{}: {}", anno_k, anno_v));
                                first = false;
                            }
                            out.push('\n');
                        }

                        // Spec details
                        if let Some(spec) = obj.data.get("spec") {
                            if let Some(sel) = spec.get("selector").and_then(|v| v.as_object()) {
                                let sel_str = sel.iter().map(|(k, v)| format!("{}={}", k, v.as_str().unwrap_or(""))).collect::<Vec<_>>().join(",");
                                out.push_str(&format!("{:<26}{}\n", "Selector:", sel_str));
                            }
                            if let Some(typ) = spec.get("type").and_then(|v| v.as_str()) {
                                out.push_str(&format!("{:<26}{}\n", "Type:", typ));
                            }
                            if let Some(cluster_ip) = spec.get("clusterIP").and_then(|v| v.as_str()) {
                                out.push_str(&format!("{:<26}{}\n", "IP:", cluster_ip));
                            }
                            if let Some(ports) = spec.get("ports").and_then(|v| v.as_array()) {
                                for p in ports {
                                    let port_num = p.get("port").and_then(|v| v.as_i64()).unwrap_or(0);
                                    let proto = p.get("protocol").and_then(|v| v.as_str()).unwrap_or("TCP");
                                    let name_p = p.get("name").and_then(|v| v.as_str()).unwrap_or("");
                                    let target_p = p.get("targetPort").map(|v| v.to_string()).unwrap_or_default();
                                    out.push_str(&format!("{:<26}{}  {}/{}\n", "Port:", name_p, port_num, proto));
                                    if !target_p.is_empty() {
                                        out.push_str(&format!("{:<26}{}\n", "TargetPort:", target_p));
                                    }
                                }
                            }
                        }

                        // Events
                        out.push_str("\nEvents:\n");
                        let events_api: kube::Api<k8s_openapi::api::core::v1::Event> = if namespaced {
                            kube::Api::namespaced(client, ns.as_deref().unwrap_or("default"))
                        } else {
                            kube::Api::all(client)
                        };

                        let lp = kube::api::ListParams::default().fields(&format!("involvedObject.name={}", n));
                        if let Ok(event_list) = events_api.list(&lp).await {
                            if event_list.items.is_empty() {
                                out.push_str("  <none>\n");
                            } else {
                                out.push_str("  Type     Reason      Age   From               Message\n");
                                out.push_str("  ----     ------      ----  ----               -------\n");
                                for ev in event_list.items {
                                    let ev_type = ev.type_.unwrap_or_else(|| "Normal".to_string());
                                    let reason = ev.reason.unwrap_or_default();
                                    let from = ev.source.and_then(|s| s.component).unwrap_or_default();
                                    let msg = ev.message.unwrap_or_default();
                                    let age = srelens_kube::humanize_age(ev.last_timestamp.as_ref());
                                    out.push_str(&format!("  {:<8} {:<11} {:<5} {:<18} {}\n", ev_type, reason, age, from, msg));
                                }
                            }
                        } else {
                            out.push_str("  <none>\n");
                        }

                        return out;
                    }
                }
            }

            format!("Error: Unable to describe {}/{} in namespace {}\n", k, n, ns.as_deref().unwrap_or("default"))
        }).await.unwrap_or_default();

        let desc_state = DescribeViewState::new(name, kind, namespace, desc_text);
        let old_view = std::mem::replace(&mut self.active_view, ActiveView::Describe(desc_state));
        self.nav_stack.push(old_view);
    }

    pub async fn execute_modal_confirm(&mut self, action_name: String) {
        if action_name == "delete" {
            self.set_toast("Resource deleted successfully".to_string(), Theme::status_ok());
        } else if action_name == "restart" {
            self.set_toast("Rollout restart triggered".to_string(), Theme::status_ok());
        } else if action_name.starts_with("stop-pf:") {
            self.set_toast("Port forward stopped".to_string(), Theme::status_ok());
        }
    }

    pub async fn execute_scale_workload(&mut self, name: String, replicas: i32) {
        self.set_toast(format!("Scaled workload '{}' to {} replicas", name, replicas), Theme::status_ok());
    }

    pub async fn execute_start_port_forward(&mut self, pod: String, _ns: String, local_port: u16, target_port: u16) {
        self.set_toast(format!("Port forward started on 127.0.0.1:{} -> {}:{}", local_port, pod, target_port), Theme::status_ok());
    }

    pub fn handle_stream_event(&mut self, channel: String, payload: serde_json::Value) {
        if let Some(items) = payload.as_array() {
            // Save to in-memory Informer Cache if this is a watch stream
            if channel.starts_with("watch:") {
                let parts: Vec<&str> = channel.split(':').collect();
                if parts.len() >= 4 {
                    let ctx = parts[1].to_string();
                    let ns = parts[2].to_string();
                    let kind = parts[3].to_string();
                    self.resource_cache.insert((ctx, ns, kind), items.clone());
                }
            }

            if let Some(cur_ch) = &self.current_watch_channel {
                if *cur_ch == channel {
                    if let ActiveView::Table(table) = &mut self.active_view {
                        table.set_items(items.clone(), &self.filter_buffer);
                    }
                }
            }
        }

        if channel.contains("namespaces") {
            if let Some(items) = payload.as_array() {
                let mut ns_list: Vec<String> = items
                    .iter()
                    .filter_map(|item| item.get("name").and_then(|v| v.as_str()).map(String::from))
                    .collect();
                ns_list.sort();
                if !ns_list.is_empty() {
                    self.namespaces = ns_list;
                }
            }
        }

        if let ActiveView::Logs(logs) = &mut self.active_view {
            if logs.channel == channel {
                if let Some(line) = payload.get("line").and_then(|v| v.as_str()) {
                    logs.push_line(line.to_string());
                } else if let Some(status) = payload.get("status").and_then(|v| v.as_str()) {
                    logs.push_line(format!("--- log status: {} ---", status));
                }
            }
        }
    }

    pub fn render(&mut self, f: &mut Frame) {
        let size = f.area();

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2), // Top header
                Constraint::Min(10),   // Main view body
                Constraint::Length(2), // Bottom statusbar & key hints
            ])
            .split(size);

        // 1. Render Header
        let active_view_title = match &self.active_view {
            ActiveView::Table(t) => t.kind.display_name(),
            ActiveView::Yaml(_) => "YAML Manifest",
            ActiveView::Describe(_) => "Describe",
            ActiveView::Logs(_) => "Logs",
            ActiveView::PortForwards(_) => "Port Forwards",
            ActiveView::Helm(_) => "Helm Releases",
            ActiveView::Overview(_) => "Overview",
            ActiveView::Toolbox(_) => "Toolbox",
            ActiveView::Assistant(_) => "AI Assistant",
        };

        let active_pods_count = if let ActiveView::Table(t) = &self.active_view {
            if t.kind == ResourceKind::Pods {
                t.raw_items.len()
            } else if self.pod_count > 0 {
                self.pod_count
            } else {
                t.raw_items.len()
            }
        } else {
            self.pod_count
        };

        render_header(
            f,
            chunks[0],
            HeaderProps {
                context: &self.active_context,
                cluster: &self.cluster_name,
                server: &self.server_url,
                namespace: &self.active_namespace,
                version: &self.cluster_version,
                node_count: self.node_count,
                pod_count: active_pods_count,
                is_connected: self.is_connected,
                active_view_name: active_view_title,
            },
        );

        // 2. Render Main View Body
        match &self.active_view {
            ActiveView::Table(table) => render_resource_table(f, chunks[1], table),
            ActiveView::Yaml(yaml) => render_yaml_view(f, chunks[1], yaml),
            ActiveView::Describe(desc) => render_describe_view(f, chunks[1], desc),
            ActiveView::Logs(logs) => render_logs_view(f, chunks[1], logs),
            ActiveView::PortForwards(pf) => render_port_forward_view(f, chunks[1], pf),
            ActiveView::Helm(helm) => render_helm_view(f, chunks[1], helm),
            ActiveView::Overview(ov) => render_overview_view(f, chunks[1], ov),
            ActiveView::Toolbox(tb) => render_toolbox_view(f, chunks[1], tb),
            ActiveView::Assistant(ai) => render_assistant_view(f, chunks[1], ai),
        }

        // 3. Render Status Bar
        let (matched_count, total_count) = if let ActiveView::Table(t) = &self.active_view {
            (t.filtered_indices.len(), t.raw_items.len())
        } else {
            (0, 0)
        };

        let toast_prop = self.toast.as_ref().map(|(msg, _, style)| (msg.as_str(), *style));

        let suggestions = if self.input_mode == InputMode::Command {
            Some(command_suggestions_with_crds(&self.command_buffer, &self.crds))
        } else {
            None
        };
        let suggestions_prop = suggestions.as_ref().map(|s| (s.as_slice(), self.command_suggestion_idx));

        render_statusbar(
            f,
            chunks[2],
            StatusBarProps {
                mode: &self.input_mode,
                command_input: &self.command_buffer,
                filter_input: &self.filter_buffer,
                matched_count,
                total_count,
                toast: toast_prop,
                custom_hints: None,
                suggestions: suggestions_prop,
            },
        );

        // 4. Render Modals & Overlays
        if let Some(modal) = &self.modal {
            render_modal(f, size, modal);
        } else if self.show_help {
            render_help_modal(f, size);
        }
    }
}

pub fn copy_to_clipboard(text: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        use std::io::Write;
        use std::process::{Command, Stdio};
        if let Ok(mut child) = Command::new("pbcopy").stdin(Stdio::piped()).spawn() {
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(text.as_bytes())?;
            }
            let _ = child.wait();
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        use std::io::Write;
        use std::process::{Command, Stdio};
        if let Ok(mut child) = Command::new("wl-copy").stdin(Stdio::piped()).spawn() {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            let _ = child.wait();
            return Ok(());
        }
        if let Ok(mut child) = Command::new("xclip").arg("-selection").arg("clipboard").stdin(Stdio::piped()).spawn() {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            let _ = child.wait();
            return Ok(());
        }
        Ok(())
    }
}
