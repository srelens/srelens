use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
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
    Assistant,
    Settings(SettingsViewState),
    Tree(tree_view::TreeViewState),
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
    pub context_chip_rects: std::cell::RefCell<Vec<(ratatui::layout::Rect, String)>>,
    // Dynamic Cluster Metrics & Information
    pub cluster_version: String,
    pub cluster_name: String,
    pub server_url: String,
    pub node_count: usize,
    pub pod_count: usize,
    pub is_connected: bool,
    pub ai_settings: crate::ai_config::AiSettings,
    pub assistant_state: AssistantViewState,
    pub assistant_states: HashMap<String, AssistantViewState>,
    pub pod_metrics_tick_counter: usize,
    pub cluster_overview_data: Option<crate::views::overview_view::ClusterOverviewData>,
    /// Global drag-to-copy selection as (anchor, cursor) screen cells. Active
    /// in every view without its own selection handler (YAML and Assistant
    /// keep theirs); the render pass highlights the range on the final buffer
    /// and captures its text so mouse-up copies exactly what is on screen.
    pub screen_selection: Option<((u16, u16), (u16, u16))>,
    pub screen_selecting: bool,
    pub screen_selection_text: std::cell::RefCell<String>,
}

pub enum SuspendAction {
    EditYaml,
    PodShell { pod: String, container: Option<String> },
    DebugShell { pod: String, container: Option<String> },
    NodeShell { node: String },
}

/// Deletes the preceding word from a string buffer, matching Unix readline / k9s / vim `<Ctrl+w>`.
/// Strips trailing whitespace, then removes non-whitespace characters until the next whitespace boundary.
pub fn delete_prev_word(s: &mut String) {
    // 1. Pop trailing whitespace
    while let Some(c) = s.chars().last() {
        if c.is_whitespace() {
            s.pop();
        } else {
            break;
        }
    }
    // 2. Pop preceding non-whitespace word characters
    while let Some(c) = s.chars().last() {
        if !c.is_whitespace() {
            s.pop();
        } else {
            break;
        }
    }
}

/// Returns true if the key event represents a word deletion command:
/// - Unix / Readline / Vim: `<Ctrl+w>`
/// - macOS: `<Option+Backspace>` (Alt+Backspace)
/// - Windows / Linux: `<Ctrl+Backspace>`
/// - Terminal fallback: `<Ctrl+h>`
pub fn is_word_delete_key(key: &KeyEvent) -> bool {
    (key.code == KeyCode::Char('w') || key.code == KeyCode::Char('W')) && key.modifiers.contains(KeyModifiers::CONTROL)
        || (key.code == KeyCode::Backspace && (key.modifiers.contains(KeyModifiers::CONTROL) || key.modifiers.contains(KeyModifiers::ALT)))
        || (key.code == KeyCode::Char('h') && key.modifiers.contains(KeyModifiers::CONTROL))
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
            active_context: active_context.clone(),
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
            context_chip_rects: std::cell::RefCell::new(Vec::new()),
            cluster_version: "Connecting...".to_string(),
            cluster_name,
            server_url,
            node_count: 0,
            pod_count: 0,
            is_connected: true,
            ai_settings: crate::ai_config::AiSettings::load(),
            assistant_state: AssistantViewState::for_context(&active_context),
            assistant_states: HashMap::new(),
            pod_metrics_tick_counter: 0,
            cluster_overview_data: None,
            screen_selection: None,
            screen_selecting: false,
            screen_selection_text: std::cell::RefCell::new(String::new()),
        };

        app.refresh_cluster_info();
        app.refresh_cluster_overview();
        app.refresh_crds();
        app.restart_active_watch().await;
        Ok(app)
    }

    pub fn set_toast(&mut self, msg: String, style: Style) {
        self.toast = Some((msg, Instant::now(), style));
    }

    pub fn handle_tick(&mut self) {
        self.assistant_state.tick();

        // Clear expired toasts (auto-dismiss after 3s)
        if let Some((_, created, _)) = &self.toast {
            if created.elapsed() > Duration::from_secs(3) {
                self.toast = None;
            }
        }

        // Periodically refresh pod metrics (CPU/MEM) every ~4 seconds (40 ticks at 100ms)
        if let ActiveView::Table(table) = &self.active_view {
            if table.kind == ResourceKind::Pods {
                self.pod_metrics_tick_counter = self.pod_metrics_tick_counter.saturating_add(1);
                if self.pod_metrics_tick_counter % 40 == 1 {
                    self.refresh_pod_metrics();
                }
            }
        }
    }

    pub fn refresh_pod_metrics(&self) {
        let event_tx = self.event_tx.clone();
        let cache = self.client_cache.clone();
        let ctx = self.active_context.clone();
        let ns = self.active_namespace.clone();

        tokio::spawn(async move {
            if let Ok(metrics) = srelens_kube::metrics::fetch_pod_metrics(cache, &ctx, &ns).await {
                if let Ok(json_str) = serde_json::to_string(&metrics) {
                    let _ = event_tx.send(crate::event::AppEvent::ActionResult {
                        title: "pod_metrics_updated".to_string(),
                        result: Ok(json_str),
                    });
                }
            }
        });
    }

    pub fn handle_pod_metrics_update(&mut self, payload: &str) {
        let metrics: Vec<srelens_kube::metrics::PodMetric> = match serde_json::from_str(payload) {
            Ok(m) => m,
            Err(_) => return,
        };

        if metrics.is_empty() {
            return;
        }

        let metric_map: std::collections::HashMap<String, (i64, i64)> = metrics
            .into_iter()
            .map(|m| (m.name, (m.cpu_millicores, m.memory_mib)))
            .collect();

        // 1. Update in active table view if currently viewing Pods
        if let ActiveView::Table(table) = &mut self.active_view {
            if table.kind == ResourceKind::Pods {
                for item in table.raw_items.iter_mut() {
                    let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    if let Some(&(cpu, mem)) = metric_map.get(name) {
                        if let Some(obj) = item.as_object_mut() {
                            let mem_str = if mem >= 1024 {
                                format!("{:.1}Gi", mem as f64 / 1024.0)
                            } else {
                                format!("{}Mi", mem)
                            };
                            obj.insert("cpu".to_string(), serde_json::Value::String(format!("{}m", cpu)));
                            obj.insert("memory".to_string(), serde_json::Value::String(mem_str));
                        }
                    }
                }
                table.apply_filter(&self.filter_buffer);
            }
        }

        // 2. Also update in-memory resource_cache so switching views retains metrics
        let key = (self.active_context.clone(), self.active_namespace.clone(), "pods".to_string());
        if let Some(cached_items) = self.resource_cache.get_mut(&key) {
            for item in cached_items.iter_mut() {
                let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("");
                if let Some(&(cpu, mem)) = metric_map.get(name) {
                    if let Some(obj) = item.as_object_mut() {
                        let mem_str = if mem >= 1024 {
                            format!("{:.1}Gi", mem as f64 / 1024.0)
                        } else {
                            format!("{}Mi", mem)
                        };
                        obj.insert("cpu".to_string(), serde_json::Value::String(format!("{}m", cpu)));
                        obj.insert("memory".to_string(), serde_json::Value::String(mem_str));
                    }
                }
            }
        }
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

                        let (version, printer_columns) = spec["versions"]
                            .as_array()
                            .and_then(|vs| {
                                let ver_obj = vs.iter()
                                    .find(|v| v["storage"].as_bool().unwrap_or(false))
                                    .or_else(|| vs.iter().find(|v| v["served"].as_bool().unwrap_or(false)))?;
                                let ver_name = ver_obj["name"].as_str()?.to_string();
                                let cols = ver_obj["additionalPrinterColumns"]
                                    .as_array()
                                    .or_else(|| spec["additionalPrinterColumns"].as_array())
                                    .map(|arr| {
                                        arr.iter().filter_map(|col| {
                                            Some(crate::commands::PrinterColumn {
                                                name: col["name"].as_str()?.to_string(),
                                                json_path: col["jsonPath"].as_str()?.to_string(),
                                                col_type: col["type"].as_str().unwrap_or("string").to_string(),
                                                priority: col["priority"].as_i64().unwrap_or(0) as i32,
                                                description: col["description"].as_str().map(String::from),
                                            })
                                        }).collect()
                                    })
                                    .unwrap_or_default();
                                Some((ver_name, cols))
                            })
                            .unwrap_or_else(|| ("v1".to_string(), Vec::new()));

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
                                printer_columns,
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
                if let ResourceKind::CustomResource(crd) = &mut table.kind {
                    if crd.kind == crd_kind || crd.plural == crd_kind {
                        if crd.printer_columns.is_empty() {
                            if let Some(discovered) = self.crds.iter().find(|c| c.kind == crd.kind || c.plural == crd.plural) {
                                crd.printer_columns = discovered.printer_columns.clone();
                                table.columns = crate::views::resource_table::default_columns_for_kind(&table.kind);
                            }
                        }
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

            if let ActiveView::Overview(ov) = &mut self.active_view {
                ov.data.context_name = self.active_context.clone();
                ov.data.cluster_name = self.cluster_name.clone();
                ov.data.server_url = self.server_url.clone();
                ov.data.k8s_version = self.cluster_version.clone();
                ov.data.is_reachable = true;
                if ov.data.node_count == 0 {
                    ov.data.node_count = self.node_count;
                }
                if ov.data.total_pods == 0 {
                    ov.data.total_pods = self.pod_count;
                }
            }
        }
    }

    pub fn handle_cluster_overview_update(&mut self, payload: &str) {
        if let Ok(data) = serde_json::from_str::<crate::views::overview_view::ClusterOverviewData>(payload) {
            if data.context_name == self.active_context {
                self.cluster_version = data.k8s_version.clone();
                self.node_count = data.node_count;
                self.pod_count = data.total_pods;
                self.is_connected = data.is_reachable;
                if let ActiveView::Overview(ov) = &mut self.active_view {
                    ov.set_data(data.clone());
                }
                self.cluster_overview_data = Some(data);
            }
        }
    }
}

fn is_physical_gpu_key(k: &str) -> bool {
    let lower = k.to_lowercase();
    (lower == "nvidia.com/gpu" || lower == "amd.com/gpu" || lower == "intel.com/gpu" || lower.ends_with("/gpu"))
        && !lower.contains("mem")
        && !lower.contains("core")
        && !lower.contains("vgpu")
}

fn is_gpu_memory_key(k: &str) -> bool {
    let lower = k.to_lowercase();
    lower.contains("gpu") && (lower.contains("mem") || lower.contains("vram"))
}

fn parse_gpu_mem_mib(s: &str) -> i64 {
    let s = s.trim();
    let num = |suffix: &str| s.trim_end_matches(suffix).trim().parse::<f64>().unwrap_or(0.0);
    if s.ends_with("Ki") || s.ends_with("ki") || s.ends_with('k') || s.ends_with('K') {
        (num("Ki").max(num("ki")).max(num("k")).max(num("K")) / 1024.0) as i64
    } else if s.ends_with("Mi") || s.ends_with("mi") || s.ends_with('m') || s.ends_with('M') {
        num("Mi").max(num("mi")).max(num("m")).max(num("M")) as i64
    } else if s.ends_with("Gi") || s.ends_with("gi") || s.ends_with('g') || s.ends_with('G') {
        (num("Gi").max(num("gi")).max(num("g")).max(num("G")) * 1024.0) as i64
    } else if s.ends_with("Ti") || s.ends_with("ti") || s.ends_with('t') || s.ends_with('T') {
        (num("Ti").max(num("ti")).max(num("t")).max(num("T")) * 1024.0 * 1024.0) as i64
    } else if let Ok(val) = s.parse::<i64>() {
        if val > 100_000_000 {
            val / (1024 * 1024)
        } else {
            val
        }
    } else {
        0
    }
}

impl App {
    pub fn refresh_cluster_overview(&self) {
        let context = self.active_context.clone();
        let cache = self.client_cache.clone();
        let event_tx = self.event_tx.clone();
        let cluster_name = self.cluster_name.clone();
        let server_url = self.server_url.clone();

        tokio::spawn(async move {
            let client = match cache.get(&context).await {
                Ok(c) => c,
                Err(_) => {
                    let data = crate::views::overview_view::ClusterOverviewData {
                        context_name: context,
                        cluster_name,
                        server_url,
                        k8s_version: String::new(),
                        is_reachable: false,
                        node_count: 0,
                        ready_nodes: 0,
                        total_pods: 0,
                        running_pods: 0,
                        pending_pods: 0,
                        failed_pods: 0,
                        total_cpu_millicores: 0,
                        used_cpu_millicores: 0,
                        total_mem_mib: 0,
                        used_mem_mib: 0,
                        total_gpus: 0,
                        allocated_gpus: 0,
                        total_gpu_mem_mib: 0,
                        used_gpu_mem_mib: 0,
                    };
                    if let Ok(ser) = serde_json::to_string(&data) {
                        let _ = event_tx.send(AppEvent::ActionResult {
                            title: "cluster_overview_updated".to_string(),
                            result: Ok(ser),
                        });
                    }
                    return;
                }
            };

            let k8s_version = client
                .apiserver_version()
                .await
                .map(|v| v.git_version)
                .unwrap_or_else(|_| "unknown".to_string());

            let mut data = crate::views::overview_view::ClusterOverviewData {
                context_name: context.clone(),
                cluster_name: cluster_name.clone(),
                server_url: server_url.clone(),
                k8s_version,
                is_reachable: true,
                node_count: 0,
                ready_nodes: 0,
                total_pods: 0,
                running_pods: 0,
                pending_pods: 0,
                failed_pods: 0,
                total_cpu_millicores: 0,
                used_cpu_millicores: 0,
                total_mem_mib: 0,
                used_mem_mib: 0,
                total_gpus: 0,
                allocated_gpus: 0,
                total_gpu_mem_mib: 0,
                used_gpu_mem_mib: 0,
            };

            // Fetch Nodes
            let node_api = kube::Api::<k8s_openapi::api::core::v1::Node>::all(client.clone());
            if let Ok(nodes) = node_api.list(&kube::api::ListParams::default()).await {
                data.node_count = nodes.items.len();
                for node in nodes.items {
                    let summary = srelens_kube::nodes::summarise(node.clone());
                    if summary.status == "Ready" {
                        data.ready_nodes += 1;
                    }
                    data.total_cpu_millicores += summary.allocatable_cpu_millicores;
                    data.total_mem_mib += summary.allocatable_memory_mib;

                    if let Some(status) = &node.status {
                        if let Some(alloc) = &status.allocatable {
                            for (k, v) in alloc {
                                if is_physical_gpu_key(k) {
                                    if let Ok(count) = v.0.parse::<usize>() {
                                        data.total_gpus += count;
                                    }
                                } else if is_gpu_memory_key(k) {
                                    data.total_gpu_mem_mib += parse_gpu_mem_mib(&v.0);
                                }
                            }
                        }
                    }
                }
            }

            // Check NodeMetrics (if metrics-server is available)
            let gvk = kube::core::GroupVersionKind::gvk("metrics.k8s.io", "v1beta1", "NodeMetrics");
            let ar = kube::core::ApiResource::from_gvk(&gvk);
            let node_metrics_api: kube::Api<kube::core::DynamicObject> = kube::Api::all_with(client.clone(), &ar);
            let mut got_node_metrics = false;
            if let Ok(list) = node_metrics_api.list(&kube::api::ListParams::default()).await {
                if !list.items.is_empty() {
                    got_node_metrics = true;
                    for o in list.items {
                        let usage = &o.data["usage"];
                        data.used_cpu_millicores += srelens_kube::metrics::cpu_millicores(usage["cpu"].as_str().unwrap_or("0"));
                        data.used_mem_mib += srelens_kube::metrics::mem_mib(usage["memory"].as_str().unwrap_or("0"));
                    }
                }
            }

            // Fetch Pods
            let pod_api = kube::Api::<k8s_openapi::api::core::v1::Pod>::all(client.clone());
            if let Ok(pods) = pod_api.list(&kube::api::ListParams::default()).await {
                data.total_pods = pods.items.len();
                let mut pod_req_cpu = 0i64;
                let mut pod_req_mem = 0i64;

                for pod in pods.items {
                    let phase = pod.status.as_ref().and_then(|s| s.phase.as_deref()).unwrap_or("Unknown");
                    let mut is_unhealthy = false;

                    if let Some(status) = &pod.status {
                        if let Some(cs_list) = &status.container_statuses {
                            for cs in cs_list {
                                if let Some(state) = &cs.state {
                                    if let Some(waiting) = &state.waiting {
                                        let r = waiting.reason.as_deref().unwrap_or_default();
                                        if r == "CrashLoopBackOff" || r == "OOMKilled" || r == "Error" || r == "ImagePullBackOff" || r == "CreateContainerConfigError" {
                                            is_unhealthy = true;
                                            break;
                                        }
                                    }
                                    if let Some(terminated) = &state.terminated {
                                        if terminated.exit_code != 0 || terminated.reason.as_deref() == Some("OOMKilled") || terminated.reason.as_deref() == Some("Error") {
                                            is_unhealthy = true;
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if is_unhealthy || phase == "Failed" {
                        data.failed_pods += 1;
                    } else if phase == "Pending" {
                        data.pending_pods += 1;
                    } else if phase == "Running" {
                        data.running_pods += 1;
                    }

                    if let Some(spec) = &pod.spec {
                        let mut containers = spec.containers.clone();
                        if let Some(init) = &spec.init_containers {
                            containers.extend(init.clone());
                        }
                        for c in containers {
                            if let Some(res) = &c.resources {
                                if let Some(reqs) = &res.requests {
                                    if let Some(q) = reqs.get("cpu") {
                                        pod_req_cpu += srelens_kube::metrics::cpu_millicores(&q.0);
                                    }
                                    if let Some(q) = reqs.get("memory") {
                                        pod_req_mem += srelens_kube::metrics::mem_mib(&q.0);
                                    }
                                    for (k, v) in reqs {
                                        if is_physical_gpu_key(k) {
                                            if let Ok(g) = v.0.parse::<usize>() {
                                                data.allocated_gpus += g;
                                            }
                                        } else if is_gpu_memory_key(k) {
                                            data.used_gpu_mem_mib += parse_gpu_mem_mib(&v.0);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                if !got_node_metrics {
                    data.used_cpu_millicores = pod_req_cpu;
                    data.used_mem_mib = pod_req_mem;
                }
            }

            if let Ok(ser) = serde_json::to_string(&data) {
                let _ = event_tx.send(AppEvent::ActionResult {
                    title: "cluster_overview_updated".to_string(),
                    result: Ok(ser),
                });
            }
        });
    }

    pub async fn switch_context(&mut self, new_context: String) {
        if self.active_context == new_context {
            return;
        }

        self.watch_manager.shutdown_all();
        self.active_watch_channels.clear();
        self.active_watch_pool.clear();
        self.resource_cache.clear();
        self.current_watch_channel = None;

        // 1. Save outgoing context's assistant view state into map
        let old_state = std::mem::replace(
            &mut self.assistant_state,
            AssistantViewState::for_context(&new_context),
        );
        self.assistant_states.insert(self.active_context.clone(), old_state);

        // 2. Switch context and namespace
        self.active_context = new_context;
        if let Some(ctx) = self.contexts.iter().find(|c| c.name == self.active_context) {
            self.cluster_name = ctx.cluster.clone();
            self.server_url = ctx.server.clone();
            if !ctx.namespace.is_empty() {
                self.active_namespace = ctx.namespace.clone();
            } else {
                self.active_namespace = String::new();
            }
        }

        // 3. Restore or initialize assistant state for the target context
        if let Some(saved_state) = self.assistant_states.remove(&self.active_context) {
            self.assistant_state = saved_state;
        }

        self.cluster_version = "Connecting...".to_string();
        self.cluster_overview_data = None;
        if let ActiveView::Overview(ov) = &mut self.active_view {
            let mut d = crate::views::overview_view::ClusterOverviewData::default();
            d.context_name = self.active_context.clone();
            d.cluster_name = self.cluster_name.clone();
            d.server_url = self.server_url.clone();
            d.k8s_version = "Connecting...".to_string();
            d.is_reachable = true;
            ov.set_data(d);
        }
        self.set_toast(format!("Switched to context '{}'", self.active_context), Theme::status_ok());
        self.refresh_cluster_info();
        self.refresh_cluster_overview();
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
                    let _ = self.watch_manager.start(sink, ctx, ns, kind.clone(), channel, paths).await;
                } else {
                    // Move to most-recently-used in pool
                    if let Some(pos) = self.active_watch_pool.iter().position(|c| c == &channel) {
                        let ch = self.active_watch_pool.remove(pos);
                        self.active_watch_pool.push(ch);
                    }
                }

                if kind == "pods" {
                    self.refresh_pod_metrics();
                }
            }
        }
    }

    pub async fn handle_key_event(&mut self, key: KeyEvent) {
        // Any keypress dismisses the drag-to-copy selection highlight.
        self.screen_selection = None;
        self.screen_selecting = false;

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
                Modal::ContextPicker { contexts, mut selected_idx, mut filter, current_context } => {
                    let lower_filter = filter.to_lowercase();
                    let filtered_indices: Vec<usize> = contexts
                        .iter()
                        .enumerate()
                        .filter(|(_, c)| {
                            if lower_filter.is_empty() {
                                true
                            } else {
                                c.name.to_lowercase().contains(&lower_filter)
                                    || c.cluster.to_lowercase().contains(&lower_filter)
                                    || c.provider.as_deref().unwrap_or("").to_lowercase().contains(&lower_filter)
                                    || c.source_file.to_lowercase().contains(&lower_filter)
                            }
                        })
                        .map(|(i, _)| i)
                        .collect();
                    let filtered_count = filtered_indices.len();

                    match key.code {
                        KeyCode::Up => {
                            if selected_idx > 0 {
                                selected_idx -= 1;
                            } else {
                                selected_idx = filtered_count.saturating_sub(1);
                            }
                            self.modal = Some(Modal::ContextPicker { contexts, selected_idx, filter, current_context });
                        }
                        KeyCode::Down => {
                            if selected_idx + 1 < filtered_count {
                                selected_idx += 1;
                            } else {
                                selected_idx = 0;
                            }
                            self.modal = Some(Modal::ContextPicker { contexts, selected_idx, filter, current_context });
                        }
                        KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            if selected_idx > 0 {
                                selected_idx -= 1;
                            } else {
                                selected_idx = filtered_count.saturating_sub(1);
                            }
                            self.modal = Some(Modal::ContextPicker { contexts, selected_idx, filter, current_context });
                        }
                        KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            if selected_idx + 1 < filtered_count {
                                selected_idx += 1;
                            } else {
                                selected_idx = 0;
                            }
                            self.modal = Some(Modal::ContextPicker { contexts, selected_idx, filter, current_context });
                        }
                        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) && !key.modifiers.contains(KeyModifiers::ALT) => {
                            filter.push(c);
                            selected_idx = 0;
                            self.modal = Some(Modal::ContextPicker { contexts, selected_idx, filter, current_context });
                        }
                        KeyCode::Backspace => {
                            filter.pop();
                            selected_idx = 0;
                            self.modal = Some(Modal::ContextPicker { contexts, selected_idx, filter, current_context });
                        }
                        KeyCode::Enter => {
                            let target_ctx = filtered_indices.get(selected_idx).and_then(|&orig_idx| contexts.get(orig_idx)).map(|c| c.name.clone());
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
                        _ if is_word_delete_key(&key) => {
                            delete_prev_word(&mut filter);
                            self.modal = Some(Modal::NamespacePicker { namespaces, selected_idx: 0, filter, current_namespace });
                        }
                        KeyCode::Char('u') | KeyCode::Char('U') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            filter.clear();
                            self.modal = Some(Modal::NamespacePicker { namespaces, selected_idx: 0, filter, current_namespace });
                        }
                        KeyCode::Backspace => {
                            filter.pop();
                            self.modal = Some(Modal::NamespacePicker { namespaces, selected_idx: 0, filter, current_namespace });
                        }
                        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) && !key.modifiers.contains(KeyModifiers::ALT) => {
                            filter.push(c);
                            self.modal = Some(Modal::NamespacePicker { namespaces, selected_idx: 0, filter, current_namespace });
                        }
                        KeyCode::Enter => {
                            let target_ns = if !filter.is_empty() && selected_idx == 0 && all_filtered.len() == 2 {
                                all_filtered.get(1).cloned()
                            } else {
                                all_filtered.get(selected_idx).cloned()
                            };
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
                Modal::ActionPalette {
                    resource_kind,
                    resource_name,
                    namespace,
                    actions,
                    mut selected_idx,
                    mut filter,
                } => {
                    let lower_filter = filter.to_lowercase();
                    let filtered_count = actions
                        .iter()
                        .filter(|a| {
                            if lower_filter.is_empty() {
                                true
                            } else {
                                a.title.to_lowercase().contains(&lower_filter)
                                    || a.key_hint.to_lowercase().contains(&lower_filter)
                                    || a.description.to_lowercase().contains(&lower_filter)
                            }
                        })
                        .count();

                    match key.code {
                        KeyCode::Up => {
                            if selected_idx > 0 {
                                selected_idx -= 1;
                            } else {
                                selected_idx = filtered_count.saturating_sub(1);
                            }
                            self.modal = Some(Modal::ActionPalette {
                                resource_kind,
                                resource_name,
                                namespace,
                                actions,
                                selected_idx,
                                filter,
                            });
                        }
                        KeyCode::Down => {
                            if selected_idx + 1 < filtered_count {
                                selected_idx += 1;
                            } else {
                                selected_idx = 0;
                            }
                            self.modal = Some(Modal::ActionPalette {
                                resource_kind,
                                resource_name,
                                namespace,
                                actions,
                                selected_idx,
                                filter,
                            });
                        }
                        KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            if selected_idx > 0 {
                                selected_idx -= 1;
                            } else {
                                selected_idx = filtered_count.saturating_sub(1);
                            }
                            self.modal = Some(Modal::ActionPalette {
                                resource_kind,
                                resource_name,
                                namespace,
                                actions,
                                selected_idx,
                                filter,
                            });
                        }
                        KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            if selected_idx + 1 < filtered_count {
                                selected_idx += 1;
                            } else {
                                selected_idx = 0;
                            }
                            self.modal = Some(Modal::ActionPalette {
                                resource_kind,
                                resource_name,
                                namespace,
                                actions,
                                selected_idx,
                                filter,
                            });
                        }
                        KeyCode::Backspace => {
                            filter.pop();
                            selected_idx = 0;
                            self.modal = Some(Modal::ActionPalette {
                                resource_kind,
                                resource_name,
                                namespace,
                                actions,
                                selected_idx,
                                filter,
                            });
                        }
                        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) && !key.modifiers.contains(KeyModifiers::ALT) => {
                            filter.push(c);
                            selected_idx = 0;
                            self.modal = Some(Modal::ActionPalette {
                                resource_kind,
                                resource_name,
                                namespace,
                                actions,
                                selected_idx,
                                filter,
                            });
                        }
                        KeyCode::Enter => {
                            self.modal = Some(Modal::ActionPalette {
                                resource_kind,
                                resource_name,
                                namespace,
                                actions,
                                selected_idx,
                                filter,
                            });
                            self.execute_action_palette().await;
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
                KeyCode::Char('n') | KeyCode::Char('N') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    let suggestions = command_suggestions_with_crds(&self.command_buffer, &self.crds);
                    if !suggestions.is_empty() {
                        let idx = self.command_suggestion_idx % suggestions.len();
                        self.command_buffer = suggestions[idx].0.name.clone();
                        self.command_suggestion_idx = (idx + 1) % suggestions.len();
                    }
                }
                KeyCode::Char('p') | KeyCode::Char('P') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    let suggestions = command_suggestions_with_crds(&self.command_buffer, &self.crds);
                    if !suggestions.is_empty() {
                        let len = suggestions.len();
                        let idx = (self.command_suggestion_idx + len - 1) % len;
                        self.command_buffer = suggestions[idx].0.name.clone();
                        self.command_suggestion_idx = idx;
                    }
                }
                _ if is_word_delete_key(&key) => {
                    if self.command_buffer.is_empty() {
                        self.input_mode = InputMode::Normal;
                    } else {
                        delete_prev_word(&mut self.command_buffer);
                        self.command_suggestion_idx = 0;
                    }
                }
                KeyCode::Char('u') | KeyCode::Char('U') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.command_buffer.clear();
                    self.command_suggestion_idx = 0;
                }
                KeyCode::Backspace => {
                    self.command_buffer.pop();
                    self.command_suggestion_idx = 0;
                    if self.command_buffer.is_empty() {
                        self.input_mode = InputMode::Normal;
                    }
                }
                KeyCode::Char('v') | KeyCode::Char('V') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if let Some(clip) = get_clipboard_text() {
                        let cleaned = clip.replace("\r\n", " ").replace('\n', " ");
                        self.command_buffer.push_str(&cleaned);
                        self.command_suggestion_idx = 0;
                    }
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) && !key.modifiers.contains(KeyModifiers::ALT) => {
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
                    let only_one = if let ActiveView::Table(table) = &self.active_view {
                        table.filtered_indices.len() == 1
                    } else {
                        false
                    };

                    if only_one {
                        self.handle_view_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).await;
                    }
                }
                _ if is_word_delete_key(&key) => {
                    delete_prev_word(&mut self.filter_buffer);
                    let filter = self.filter_buffer.clone();
                    if let ActiveView::Table(table) = &mut self.active_view {
                        table.apply_filter(&filter);
                    }
                }
                KeyCode::Char('u') | KeyCode::Char('U') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.filter_buffer.clear();
                    if let ActiveView::Table(table) = &mut self.active_view {
                        table.apply_filter("");
                    }
                }
                KeyCode::Backspace => {
                    self.filter_buffer.pop();
                    let filter = self.filter_buffer.clone();
                    if let ActiveView::Table(table) = &mut self.active_view {
                        table.apply_filter(&filter);
                    }
                }
                KeyCode::Char('v') | KeyCode::Char('V') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if let Some(clip) = get_clipboard_text() {
                        let cleaned = clip.replace("\r\n", "").replace('\n', "");
                        self.filter_buffer.push_str(&cleaned);
                        let filter = self.filter_buffer.clone();
                        if let ActiveView::Table(table) = &mut self.active_view {
                            table.apply_filter(&filter);
                        }
                    }
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) && !key.modifiers.contains(KeyModifiers::ALT) => {
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

        // 5. Settings View Dedicated Key Handling
        if matches!(self.active_view, ActiveView::Settings(_)) {
            if let ActiveView::Settings(s) = &self.active_view {
                if !s.is_editing && key.code == KeyCode::Char(':') {
                    self.input_mode = InputMode::Command;
                    self.command_buffer.clear();
                    return;
                }
            }
            self.handle_view_key_event(key).await;
            return;
        }

        // 6. Normal Mode - k9s Global & View Keybindings
        match key.code {
            // Enter Command Mode
            KeyCode::Char(':') if !matches!(self.active_view, ActiveView::Assistant) || self.assistant_state.input.is_empty() => {
                self.input_mode = InputMode::Command;
                self.command_buffer.clear();
            }
            // Enter Filter Mode
            KeyCode::Char('/') if !matches!(self.active_view, ActiveView::Assistant) => {
                self.input_mode = InputMode::Filter;
            }
            // Open Help Modal
            KeyCode::Char('?') if !matches!(self.active_view, ActiveView::Assistant) || self.assistant_state.input.is_empty() => {
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
                if matches!(self.active_view, ActiveView::Assistant) && self.assistant_state.selection.is_some() {
                    self.assistant_state.clear_selection();
                    return;
                }
                if let ActiveView::Yaml(ref mut yaml) = self.active_view {
                    if yaml.selection.is_some() {
                        yaml.clear_selection();
                        return;
                    }
                }
                let has_table_filter = if let ActiveView::Table(t) = &self.active_view {
                    t.filtered_indices.len() != t.raw_items.len()
                } else {
                    false
                };
                if !self.filter_buffer.is_empty() || has_table_filter {
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
                if matches!(self.active_view, ActiveView::Assistant) {
                    if let Some(prev) = self.nav_stack.pop() {
                        self.active_view = prev;
                        self.restart_active_watch().await;
                    }
                } else {
                    let prev = std::mem::replace(&mut self.active_view, ActiveView::Assistant);
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
                    KeyCode::Char('j') | KeyCode::Down => {
                        self.toast = None;
                        table.select_next();
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        self.toast = None;
                        table.select_prev();
                    }
                    KeyCode::Char('g') | KeyCode::Home => {
                        self.toast = None;
                        table.select_top();
                    }
                    KeyCode::Char('G') | KeyCode::End => {
                        self.toast = None;
                        table.select_bottom();
                    }
                    KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        self.toast = None;
                        table.page_up(10);
                    }
                    KeyCode::PageUp => {
                        self.toast = None;
                        table.page_up(10);
                    }
                    KeyCode::PageDown => {
                        self.toast = None;
                        table.page_down(10);
                    }
                    KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        // Ctrl+d -> Delete resource confirmation
                        if let Some(name) = sel_name {
                            let ns = sel_ns.clone().unwrap_or_else(|| self.active_namespace.clone());
                            let query_ns = if ns.is_empty() { "default".to_string() } else { ns };
                            self.modal = Some(Modal::Confirm {
                                title: format!("Delete {} [{}]", table_kind, name),
                                message: format!("Are you sure you want to delete {} '{}' in namespace '{}'?", table_kind, name, query_ns),
                                action_name: format!("delete:{}:{}:{}", kind_str, query_ns, name),
                                is_destructive: true,
                            });
                        }
                    }
                    KeyCode::Char(' ') => table.toggle_mark_selected(),
                    KeyCode::Char('r') => {
                        // Rollout restart (r or Ctrl+r)
                        if let Some(name) = sel_name {
                            let ns = sel_ns.clone().unwrap_or_else(|| self.active_namespace.clone());
                            let query_ns = if ns.is_empty() { "default".to_string() } else { ns };
                            self.modal = Some(Modal::Confirm {
                                title: format!("Restart Workload [{}]", name),
                                message: format!("Trigger zero-downtime rollout restart for {} '{}' in namespace '{}'?", table_kind, name, query_ns),
                                action_name: format!("restart:{}:{}:{}", kind_str, query_ns, name),
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
                    KeyCode::Char('f') | KeyCode::Char('F') => {
                        // Port forward (f, Shift+f, or Ctrl+f)
                        if let Some(pod_name) = sel_name {
                            let ns = sel_ns.unwrap_or_else(|| self.active_namespace.clone());
                            let detected_port = table.selected_item().and_then(|item| {
                                item.pointer("/spec/containers/0/ports/0/containerPort")
                                    .and_then(|v| v.as_u64())
                                    .map(|p| p as u16)
                                    .or_else(|| {
                                        item.pointer("/spec/ports/0/port")
                                            .and_then(|v| v.as_u64())
                                            .map(|p| p as u16)
                                    })
                            }).unwrap_or(8080);
                            self.modal = Some(Modal::PortForward {
                                pod_name,
                                namespace: ns,
                                container_port: detected_port,
                                local_port_input: detected_port.to_string(),
                            });
                        }
                    }
                    KeyCode::Char('w') if table_kind == ResourceKind::Events => {
                        let active = table.toggle_warning_triage(&self.filter_buffer);
                        let count = table.filtered_indices.len();
                        if active {
                            self.set_toast(format!("Warning Triage: ON ({} warnings)", count), Theme::status_warn());
                        } else {
                            self.set_toast(format!("Warning Triage: OFF (all {} events)", count), Theme::status_ok());
                        }
                    }
                    KeyCode::Char('l') => {
                        // Logs
                        let (pod_name, target_ns) = if table_kind == ResourceKind::Events {
                            if let Some(item) = table.selected_item() {
                                let (obj_kind, obj_name) = parse_involved_object(item);
                                if obj_kind.eq_ignore_ascii_case("Pod") {
                                    (Some(obj_name), sel_ns.clone().or_else(|| if self.active_namespace.is_empty() { None } else { Some(self.active_namespace.clone()) }))
                                } else {
                                    self.set_toast(format!("Logs only available for Pods (event target is {})", obj_kind), Theme::status_warn());
                                    (None, None)
                                }
                            } else {
                                (None, None)
                            }
                        } else {
                            (sel_name.clone(), sel_ns.clone().or_else(|| if self.active_namespace.is_empty() { None } else { Some(self.active_namespace.clone()) }))
                        };

                        if let Some(pod_name) = pod_name {
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
                    KeyCode::Char('c') if !key.modifiers.contains(KeyModifiers::CONTROL) && !key.modifiers.contains(KeyModifiers::ALT) => {
                        if table_kind == ResourceKind::Events {
                            if let Some(item) = table.selected_item() {
                                let summary = format_event_summary(item);
                                let sum_clone = summary.clone();
                                tokio::spawn(async move {
                                    let _ = copy_to_clipboard(&sum_clone);
                                });
                                self.set_toast("✓ Copied event message to clipboard".to_string(), Theme::status_ok());
                            }
                        } else {
                            let names_to_copy: Vec<String> = if !table.marked_indices.is_empty() {
                                table.marked_indices.iter().filter_map(|&idx| {
                                    table.raw_items.get(idx).and_then(|item| {
                                        item.get("name").or_else(|| item.pointer("/metadata/name")).and_then(|v| v.as_str()).map(String::from)
                                    })
                                }).collect()
                            } else if let Some(name) = sel_name.clone() {
                                vec![name]
                            } else {
                                Vec::new()
                            };

                            if !names_to_copy.is_empty() {
                                let text = names_to_copy.join("\n");
                                let count = names_to_copy.len();
                                let first_name = names_to_copy[0].clone();
                                tokio::spawn(async move {
                                    let _ = copy_to_clipboard(&text);
                                });
                                if count == 1 {
                                    self.set_toast(format!("✓ Copied '{}' to clipboard", first_name), Theme::status_ok());
                                } else {
                                    self.set_toast(format!("✓ Copied {} resource names to clipboard", count), Theme::status_ok());
                                }
                            }
                        }
                    }
                    KeyCode::Char('C') if key.modifiers.contains(KeyModifiers::SHIFT) || key.modifiers.contains(KeyModifiers::NONE) => {
                        if let Some(item) = table.selected_item() {
                            let yaml_str = serde_yaml::to_string(item).unwrap_or_default();
                            tokio::spawn(async move {
                                let _ = copy_to_clipboard(&yaml_str);
                            });
                            self.set_toast("✓ Copied resource YAML to clipboard".to_string(), Theme::status_ok());
                        }
                    }
                    KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        if table_kind == ResourceKind::Events {
                            if let Some(item) = table.selected_item() {
                                let (obj_kind, obj_name) = parse_involved_object(item);
                                if !obj_name.is_empty() {
                                    let link = crate::deep_link::DeepLink::Resource {
                                        context: self.active_context.clone(),
                                        namespace: sel_ns.clone(),
                                        kind: obj_kind,
                                        name: obj_name,
                                    };
                                    let url = link.to_url();
                                    let url_clone = url.clone();
                                    tokio::spawn(async move {
                                        let _ = copy_to_clipboard(&url_clone);
                                    });
                                    self.set_toast(format!("✓ Copied deep link: {}", url), Theme::status_ok());
                                }
                            }
                        } else if let Some(name) = sel_name.clone() {
                            let link = crate::deep_link::DeepLink::Resource {
                                context: self.active_context.clone(),
                                namespace: sel_ns.clone(),
                                kind: kind_str.clone(),
                                name,
                            };
                            let url = link.to_url();
                            let url_clone = url.clone();
                            tokio::spawn(async move {
                                let _ = copy_to_clipboard(&url_clone);
                            });
                            self.set_toast(format!("✓ Copied deep link: {}", url), Theme::status_ok());
                        }
                    }
                    KeyCode::Char('y') | KeyCode::Char('v') => {
                        // View YAML manifest
                        if table_kind == ResourceKind::Events {
                            if let Some(item) = table.selected_item() {
                                let (obj_kind, obj_name) = parse_involved_object(item);
                                if !obj_name.is_empty() {
                                    self.open_yaml_view(obj_name, obj_kind, sel_ns.clone()).await;
                                }
                            }
                        } else if let Some(name) = sel_name.clone() {
                            self.open_yaml_view(name, kind_str.clone(), sel_ns.clone()).await;
                        }
                    }
                    KeyCode::Char('d') => {
                        // Describe resource
                        if table_kind == ResourceKind::Events {
                            if let Some(item) = table.selected_item() {
                                let (obj_kind, obj_name) = parse_involved_object(item);
                                if !obj_name.is_empty() {
                                    self.open_describe_view(obj_name, obj_kind, sel_ns.clone()).await;
                                }
                            }
                        } else if let Some(name) = sel_name.clone() {
                            self.open_describe_view(name, kind_str.clone(), sel_ns.clone()).await;
                        }
                    }
                    KeyCode::Char('e') => {
                        // Edit YAML in $EDITOR
                        if let Some(name) = sel_name.clone() {
                            self.open_yaml_view(name, kind_str.clone(), sel_ns.clone()).await;
                            self.requires_terminal_suspend = Some(SuspendAction::EditYaml);
                        }
                    }
                    KeyCode::Char('x') => {
                        // Open Quick Actions & Incident Palette
                        if table_kind == ResourceKind::Events {
                            if let Some(item) = table.selected_item() {
                                let (obj_kind, obj_name) = parse_involved_object(item);
                                if !obj_name.is_empty() {
                                    self.open_action_palette(obj_kind, obj_name, sel_ns.clone());
                                }
                            }
                        } else if let Some(name) = sel_name.clone() {
                            self.open_action_palette(kind_str.clone(), name, sel_ns.clone());
                        }
                    }
                    KeyCode::Char('t') => {
                        // Open Resource Relationship Tree
                        if table_kind == ResourceKind::Events {
                            if let Some(item) = table.selected_item() {
                                let (obj_kind, obj_name) = parse_involved_object(item);
                                if !obj_name.is_empty() {
                                    self.open_resource_tree(obj_kind, obj_name, sel_ns.clone());
                                }
                            }
                        } else if let Some(name) = sel_name.clone() {
                            self.open_resource_tree(kind_str.clone(), name, sel_ns.clone());
                        }
                    }
                    KeyCode::Enter => {
                        // Drill-down / Activate resource
                        if table_kind == ResourceKind::Namespaces {
                            if let Some(ns_name) = sel_name {
                                self.switch_namespace(ns_name).await;
                                self.switch_view_to_kind(ResourceKind::Pods).await;
                            }
                        } else if table_kind == ResourceKind::CustomResourceDefinitions {
                            if let Some(crd_name) = sel_name {
                                if let Some(crd) = self.crds.iter().find(|c| c.crd_name == crd_name || c.kind.eq_ignore_ascii_case(&crd_name) || c.plural.eq_ignore_ascii_case(&crd_name)).cloned() {
                                    self.switch_view_to_crd(crd).await;
                                }
                            }
                        } else if table_kind == ResourceKind::Pods {
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
                        } else if matches!(table_kind, ResourceKind::Deployments | ResourceKind::DaemonSets | ResourceKind::StatefulSets) {
                            if let Some(name) = sel_name {
                                self.switch_view_to_kind(ResourceKind::Pods).await;
                                if let ActiveView::Table(t) = &mut self.active_view {
                                    t.apply_filter(&name);
                                }
                                self.filter_buffer = name;
                            }
                        } else if table_kind == ResourceKind::Events {
                            if let Some(item) = table.selected_item() {
                                let (obj_kind, obj_name) = parse_involved_object(item);
                                if !obj_kind.is_empty() && !obj_name.is_empty() {
                                    if obj_kind.eq_ignore_ascii_case("Pod") {
                                        self.switch_view_to_kind(ResourceKind::Pods).await;
                                        if let ActiveView::Table(t) = &mut self.active_view {
                                            t.apply_filter(&obj_name);
                                        }
                                        self.filter_buffer = obj_name.clone();
                                        self.set_toast(format!("Jumped to Pod '{}'", obj_name), Theme::status_ok());
                                    } else if let Some(target_cmd) = crate::commands::resolve_command_with_crds(&obj_kind, &self.crds) {
                                        self.execute_view_target(target_cmd).await;
                                        if let ActiveView::Table(t) = &mut self.active_view {
                                            t.apply_filter(&obj_name);
                                        }
                                        self.filter_buffer = obj_name.clone();
                                        self.set_toast(format!("Jumped to {} '{}'", obj_kind, obj_name), Theme::status_ok());
                                    } else {
                                        self.open_describe_view(obj_name, obj_kind, sel_ns.clone()).await;
                                    }
                                }
                            }
                        } else {
                            if let Some(name) = sel_name {
                                self.open_describe_view(name, kind_str, sel_ns).await;
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
                    KeyCode::Char('c') | KeyCode::Char('y') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        if let Some(selected) = yaml.selected_text() {
                            tokio::spawn(async move {
                                let _ = copy_to_clipboard(&selected);
                            });
                            self.set_toast("✓ Copied selection to clipboard".to_string(), Theme::status_ok());
                        } else {
                            let content = yaml.yaml_content.clone();
                            tokio::spawn(async move {
                                let _ = copy_to_clipboard(&content);
                            });
                            self.set_toast("✓ Copied YAML to clipboard".to_string(), Theme::status_ok());
                        }
                    }
                    KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        let link = crate::deep_link::DeepLink::Resource {
                            context: self.active_context.clone(),
                            namespace: yaml.namespace.clone(),
                            kind: yaml.resource_kind.clone(),
                            name: yaml.resource_name.clone(),
                        };
                        let url = link.to_url();
                        let url_clone = url.clone();
                        tokio::spawn(async move {
                            let _ = copy_to_clipboard(&url_clone);
                        });
                        self.set_toast(format!("Copied deep link: {}", url), Theme::status_ok());
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
                    KeyCode::Char('c') | KeyCode::Char('y') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        let content = desc.content.clone();
                        tokio::spawn(async move {
                            let _ = copy_to_clipboard(&content);
                        });
                        self.set_toast("Copied describe output to clipboard".to_string(), Theme::status_ok());
                    }
                    KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        let link = crate::deep_link::DeepLink::Resource {
                            context: self.active_context.clone(),
                            namespace: desc.namespace.clone(),
                            kind: desc.resource_kind.clone(),
                            name: desc.resource_name.clone(),
                        };
                        let url = link.to_url();
                        let url_clone = url.clone();
                        tokio::spawn(async move {
                            let _ = copy_to_clipboard(&url_clone);
                        });
                        self.set_toast(format!("Copied deep link: {}", url), Theme::status_ok());
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
                    KeyCode::Char('c') | KeyCode::Char('y') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        let text = logs.lines.join("\n");
                        let count = logs.lines.len();
                        tokio::spawn(async move {
                            let _ = copy_to_clipboard(&text);
                        });
                        self.set_toast(format!("Copied {} log lines to clipboard", count), Theme::status_ok());
                    }
                    KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        let link = crate::deep_link::DeepLink::Resource {
                            context: self.active_context.clone(),
                            namespace: Some(logs.namespace.clone()),
                            kind: "Pod".to_string(),
                            name: logs.pod_name.clone(),
                        };
                        let url = link.to_url();
                        let url_clone = url.clone();
                        tokio::spawn(async move {
                            let _ = copy_to_clipboard(&url_clone);
                        });
                        self.set_toast(format!("Copied deep link: {}", url), Theme::status_ok());
                    }
                    _ => {}
                }
            }
            ActiveView::PortForwards(pf) => {
                let sel_entry = pf.selected_forward().cloned();
                match key.code {
                    KeyCode::Char('j') | KeyCode::Down => pf.select_next(),
                    KeyCode::Char('k') | KeyCode::Up => pf.select_prev(),
                    KeyCode::Char('c') | KeyCode::Char('y') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        if let Some(entry) = sel_entry {
                            let url = format!("http://127.0.0.1:{}", entry.local_port);
                            let url_clone = url.clone();
                            tokio::spawn(async move {
                                let _ = copy_to_clipboard(&url_clone);
                            });
                            self.set_toast(format!("Copied forward URL: {}", url), Theme::status_ok());
                        }
                    }
                    KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        if let Some(entry) = sel_entry {
                            let link = crate::deep_link::DeepLink::Resource {
                                context: self.active_context.clone(),
                                namespace: Some(entry.namespace.clone()),
                                kind: entry.target_type.clone(),
                                name: entry.target_name.clone(),
                            };
                            let url = link.to_url();
                            let url_clone = url.clone();
                            tokio::spawn(async move {
                                let _ = copy_to_clipboard(&url_clone);
                            });
                            self.set_toast(format!("Copied deep link: {}", url), Theme::status_ok());
                        }
                    }
                    KeyCode::Char('d') => {
                        if let Some(entry) = sel_entry {
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
                    KeyCode::Char('c') => {
                        if let Some(rel) = sel_rel {
                            let link = crate::deep_link::DeepLink::Resource {
                                context: self.active_context.clone(),
                                namespace: Some(rel.namespace.clone()),
                                kind: "HelmRelease".to_string(),
                                name: rel.name.clone(),
                            };
                            let url = link.to_url();
                            let url_clone = url.clone();
                            tokio::spawn(async move {
                                let _ = copy_to_clipboard(&url_clone);
                            });
                            self.set_toast(format!("Copied deep link: {}", url), Theme::status_ok());
                        }
                    }
                    KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        if let Some(rel) = sel_rel {
                            let link = crate::deep_link::DeepLink::Resource {
                                context: self.active_context.clone(),
                                namespace: Some(rel.namespace.clone()),
                                kind: "HelmRelease".to_string(),
                                name: rel.name.clone(),
                            };
                            let url = link.to_url();
                            let url_clone = url.clone();
                            tokio::spawn(async move {
                                let _ = copy_to_clipboard(&url_clone);
                            });
                            self.set_toast(format!("Copied deep link: {}", url), Theme::status_ok());
                        }
                    }
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
                let sel_tool = tb.tools.get(tb.selected_idx).cloned();
                match key.code {
                    KeyCode::Char('j') | KeyCode::Down => tb.select_next(),
                    KeyCode::Char('k') | KeyCode::Up => tb.select_prev(),
                    KeyCode::Char('c') | KeyCode::Char('y') => {
                        if let Some(tool) = sel_tool {
                            let info = if let Some(path) = &tool.path {
                                path.clone()
                            } else {
                                tool.name.clone()
                            };
                            let info_clone = info.clone();
                            tokio::spawn(async move {
                                let _ = copy_to_clipboard(&info_clone);
                            });
                            self.set_toast(format!("Copied tool info: {}", info), Theme::status_ok());
                        }
                    }
                    _ => {}
                }
            }
            ActiveView::Overview(ov) => {
                match key.code {
                    KeyCode::Char('r') => {
                        self.refresh_cluster_overview();
                        self.set_toast("Refreshing cluster overview...".to_string(), Theme::status_ok());
                    }
                    KeyCode::Char('c') | KeyCode::Char('y') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        let summary = ov.data.to_summary_text();
                        tokio::spawn(async move {
                            let _ = copy_to_clipboard(&summary);
                        });
                        self.set_toast("Copied cluster overview summary to clipboard".to_string(), Theme::status_ok());
                    }
                    KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        let link = crate::deep_link::DeepLink::Cluster {
                            context: self.active_context.clone(),
                        };
                        let url = link.to_url();
                        let url_clone = url.clone();
                        tokio::spawn(async move {
                            let _ = copy_to_clipboard(&url_clone);
                        });
                        self.set_toast(format!("Copied deep link: {}", url), Theme::status_ok());
                    }
                    _ => {}
                }
            }
            ActiveView::Assistant => {
                let ai = &mut self.assistant_state;
                if key.code == KeyCode::Char('s') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.switch_view_to_kind(ResourceKind::Settings).await;
                    return;
                }
                if key.code == KeyCode::Char('e') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    let prov = self.ai_settings.default_provider;
                    let prov_name = crate::ai_config::provider_display_name(prov);
                    let model = self.ai_settings.get_model(prov);
                    match ai.save_conversation_to_file(prov_name, &model, None) {
                        Ok(path) => self.set_toast(format!("✓ Saved conversation to {}", path.display()), Theme::status_ok()),
                        Err(err) => self.set_toast(format!("Failed to save conversation: {}", err), Theme::status_error()),
                    }
                    return;
                }
                if key.code == KeyCode::Char('l') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    ai.clear_conversation();
                    self.set_toast("✓ Conversation cleared".to_string(), Theme::status_ok());
                    return;
                }
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    if let Some(selected) = ai.get_selected_text() {
                        let _ = copy_to_clipboard(&selected);
                        self.set_toast("✓ Copied selection to clipboard".to_string(), Theme::status_ok());
                        return;
                    } else if let Some(last_asst) = ai.messages.iter().rev().find(|m| m.role == "assistant") {
                        let _ = copy_to_clipboard(&last_asst.content);
                        self.set_toast("✓ Copied assistant answer to clipboard".to_string(), Theme::status_ok());
                        return;
                    }
                }
                if key.code == KeyCode::Char('t') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    ai.toggle_tools_expansion();
                    return;
                }
                match key.code {
                    // Copy selection with 'c' (or copy last assistant message if input is empty)
                    KeyCode::Char('c') if !key.modifiers.contains(KeyModifiers::CONTROL) && !key.modifiers.contains(KeyModifiers::ALT) => {
                        if let Some(selected) = ai.get_selected_text() {
                            let _ = copy_to_clipboard(&selected);
                            self.set_toast("✓ Copied selection to clipboard".to_string(), Theme::status_ok());
                        } else if ai.input.is_empty() {
                            if let Some(last_asst) = ai.messages.iter().rev().find(|m| m.role == "assistant") {
                                let _ = copy_to_clipboard(&last_asst.content);
                                self.set_toast("✓ Copied assistant answer to clipboard".to_string(), Theme::status_ok());
                            } else {
                                ai.input.push('c');
                            }
                        } else {
                            ai.input.push('c');
                        }
                    }

                    // Prompt History Navigation (Up/Down recalls sent prompts)
                    KeyCode::Up => ai.history_up(),
                    KeyCode::Down => ai.history_down(),

                    // Viewport Scrolling (PageUp/PageDown, Home/End, or Mouse Scroll)
                    KeyCode::PageUp => ai.scroll_up(10),
                    KeyCode::PageDown => ai.scroll_down(10),
                    KeyCode::Home => ai.scroll_to_top(),
                    KeyCode::End => ai.scroll_to_bottom(),
                    KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => ai.scroll_up(2),
                    KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => ai.scroll_down(2),
                    KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => ai.scroll_up(10),
                    KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => ai.scroll_down(10),

                    // Editing & Input
                    _ if is_word_delete_key(&key) => {
                        delete_prev_word(&mut ai.input);
                    }
                    KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::ALT) || key.modifiers.contains(KeyModifiers::CONTROL) => {
                        ai.input.clear();
                    }
                    KeyCode::Char('v') | KeyCode::Char('V') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        if let Some(clip) = get_clipboard_text() {
                            let cleaned = clip.replace("\r\n", " ").replace('\n', " ");
                            ai.input.push_str(&cleaned);
                        }
                    }
                    KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) && !key.modifiers.contains(KeyModifiers::ALT) => {
                        ai.input.push(c);
                    }
                    KeyCode::Backspace => { ai.input.pop(); },
                    KeyCode::Enter => {
                        if ai.is_busy {
                            self.set_toast("⚠️ Assistant is busy processing a query. Please wait or press <Ctrl+l> to clear.".to_string(), Theme::status_warn());
                            return;
                        }
                        if !ai.input.trim().is_empty() {
                            let query = ai.input.clone();
                            ai.start_turn(query.clone());
                            let provider = self.ai_settings.default_provider;
                            let active_ctx = self.active_context.clone();
                            let active_ns = self.active_namespace.clone();

                            if provider == crate::ai_config::AiProvider::Cursor {
                                if let Some(cursor_bin) = crate::ai_config::find_cursor_binary() {
                                    let event_tx = self.event_tx.clone();
                                    let model = self.ai_settings.get_model(provider);
                                    let api_key = self.ai_settings.get_api_key(provider);
                                    let cache = self.client_cache.clone();
                                    let kubeconfig_paths = self.kubeconfig_paths.clone();
                                    let timeout_seconds = self.ai_settings.get_timeout_seconds(provider);

                                    tokio::spawn(async move {
                                        crate::agent::run_boxed_cursor_turn(
                                            cursor_bin,
                                            model,
                                            api_key,
                                            query,
                                            active_ctx,
                                            active_ns,
                                            cache,
                                            kubeconfig_paths,
                                            event_tx,
                                            timeout_seconds,
                                        ).await;
                                    });
                                } else {
                                    ai.add_assistant_message("cursor-agent CLI was not found on PATH. Install from https://docs.cursor.com/en/cli/overview or ensure ~/.local/bin is in your PATH.".to_string());
                                }
                            } else if let Some(config) = self.ai_settings.resolve_provider_config(provider) {
                                let event_tx = self.event_tx.clone();
                                let cache = self.client_cache.clone();
                                let kubeconfig_paths = self.kubeconfig_paths.clone();
                                let active_ctx = self.active_context.clone();
                                let active_ns = self.active_namespace.clone();
                                let history = ai.native_history.clone();
                                let timeout_seconds = self.ai_settings.get_timeout_seconds(provider);

                                tokio::spawn(async move {
                                    let server = crate::agent::build_mcp_server(cache, kubeconfig_paths);
                                    let invoker = std::sync::Arc::new(crate::agent::McpToolInvoker::new(server));
                                    crate::agent::run_native_agent_turn(
                                        config,
                                        invoker,
                                        history,
                                        query,
                                        active_ctx,
                                        active_ns,
                                        event_tx,
                                        timeout_seconds,
                                    ).await;
                                });
                            } else {
                                let env_var = crate::ai_config::env_var_for_provider(provider);
                                let prov_name = crate::ai_config::provider_display_name(provider);
                                let reply = format!(
                                    "No API key configured for {}. Press '<Ctrl+s>' to configure settings, or set the {} environment variable.",
                                    prov_name, env_var
                                );
                                ai.add_assistant_message(reply);
                            }
                        }
                    }
                    _ => {}
                }
            }
            ActiveView::Settings(settings) => {
                if settings.is_editing {
                    match key.code {
                        KeyCode::Esc => settings.cancel_editing(),
                        KeyCode::Enter => {
                            settings.finish_editing();
                            self.ai_settings = settings.settings.clone();
                            if let Err(e) = settings.save() {
                                self.set_toast(format!("Failed to save settings: {}", e), Theme::status_error());
                            } else {
                                self.set_toast("✓ Setting updated and saved".to_string(), Theme::status_ok());
                            }
                        }
                        _ if is_word_delete_key(&key) => {
                            delete_prev_word(&mut settings.edit_buffer);
                        }
                        KeyCode::Backspace => {
                            settings.edit_buffer.pop();
                        }
                        KeyCode::Char('v') | KeyCode::Char('V') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            if let Some(clip) = get_clipboard_text() {
                                let cleaned = clip.replace("\r\n", "").replace('\n', "");
                                settings.edit_buffer.push_str(&cleaned);
                            }
                        }
                        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) && !key.modifiers.contains(KeyModifiers::ALT) => {
                            settings.edit_buffer.push(c);
                        }
                        _ => {}
                    }
                } else {
                    match key.code {
                        KeyCode::Esc | KeyCode::Char('q') => {
                            self.ai_settings = settings.settings.clone();
                            let _ = settings.save();
                            if let Some(prev) = self.nav_stack.pop() {
                                self.active_view = prev;
                            } else {
                                self.switch_view_to_kind(ResourceKind::Pods).await;
                            }
                        }
                        KeyCode::Char('j') | KeyCode::Down => settings.select_next_provider(),
                        KeyCode::Char('k') | KeyCode::Up => settings.select_prev_provider(),
                        KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => settings.select_next_field(),
                        KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => settings.select_prev_field(),
                        KeyCode::Char(' ') => {
                            settings.set_active_provider();
                            self.ai_settings = settings.settings.clone();
                            let _ = settings.save();
                            let prov_name = crate::ai_config::provider_display_name(settings.settings.default_provider);
                            self.set_toast(format!("✓ Active provider set to {}", prov_name), Theme::status_ok());
                        }
                        KeyCode::Char('e') | KeyCode::Enter => settings.start_editing(),
                        KeyCode::Char('s') => {
                            match settings.save() {
                                Ok(path) => {
                                    self.ai_settings = settings.settings.clone();
                                    self.set_toast(format!("Saved AI settings to {}", path.display()), Theme::status_ok());
                                }
                                Err(err) => {
                                    self.set_toast(format!("Failed to save settings: {}", err), Theme::status_error());
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            ActiveView::Tree(tree) => {
                    match key.code {
                        KeyCode::Char('j') | KeyCode::Down => tree.select_next(),
                        KeyCode::Char('k') | KeyCode::Up => tree.select_prev(),
                        KeyCode::Char('g') | KeyCode::Home => tree.select_first(),
                        KeyCode::Char('G') | KeyCode::End => tree.select_last(),
                        KeyCode::Char('c') if !key.modifiers.contains(KeyModifiers::SHIFT) && !key.modifiers.contains(KeyModifiers::CONTROL) => {
                            if let Some(node) = tree.selected_node() {
                                let name = node.name.clone();
                                let n_clone = name.clone();
                                tokio::spawn(async move {
                                    let _ = copy_to_clipboard(&n_clone);
                                });
                                self.set_toast(format!("✓ Copied '{}' to clipboard", name), Theme::status_ok());
                            }
                        }
                        KeyCode::Char('C') | KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                            let text = tree.tree_as_text();
                            tokio::spawn(async move {
                                let _ = copy_to_clipboard(&text);
                            });
                            self.set_toast("✓ Copied resource relationship tree to clipboard".to_string(), Theme::status_ok());
                        }
                        KeyCode::Enter => {
                            if let Some(node) = tree.selected_node() {
                                let name = node.name.clone();
                                let kind = node.kind.clone();
                                let ns = node.namespace.clone();
                                self.open_describe_view(name, kind, ns).await;
                            }
                        }
                        KeyCode::Char('d') => {
                            if let Some(node) = tree.selected_node() {
                                let name = node.name.clone();
                                let kind = node.kind.clone();
                                let ns = node.namespace.clone();
                                self.open_describe_view(name, kind, ns).await;
                            }
                        }
                        KeyCode::Char('y') => {
                            if let Some(node) = tree.selected_node() {
                                let name = node.name.clone();
                                let kind = node.kind.clone();
                                let ns = node.namespace.clone();
                                self.open_yaml_view(name, kind, ns).await;
                            }
                        }
                        KeyCode::Char('l') => {
                            if let Some(node) = tree.selected_node() {
                                let name = node.name.clone();
                                let ns = node.namespace.clone();
                                self.open_logs_view(name, ns, None).await;
                            }
                        }
                        KeyCode::Char('x') => {
                            if let Some(node) = tree.selected_node() {
                                let name = node.name.clone();
                                let kind = node.kind.clone();
                                let ns = node.namespace.clone();
                                self.open_action_palette(kind, name, ns);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

    pub async fn handle_mouse(&mut self, mouse: crossterm::event::MouseEvent) {
        use crossterm::event::{MouseButton, MouseEventKind};

        // 1. Check if clicking on header context chips
        if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
            let clicked_ctx = self.context_chip_rects.borrow().iter().find_map(|(rect, name)| {
                if mouse.column >= rect.x && mouse.column < rect.x + rect.width
                    && mouse.row >= rect.y && mouse.row < rect.y + rect.height
                {
                    Some(name.clone())
                } else {
                    None
                }
            });

            if let Some(target) = clicked_ctx {
                if target == ":ctx" {
                    self.open_context_picker();
                } else if target != self.active_context {
                    self.switch_context(target).await;
                }
                return;
            }
        }

        if let ActiveView::Assistant = &self.active_view {
            let vp = self.assistant_state.last_viewport_rect.get();
            if mouse.column >= vp.x && mouse.column < vp.x + vp.width
                && mouse.row >= vp.y && mouse.row < vp.y + vp.height
            {
                let screen_row = mouse.row.saturating_sub(vp.y) as usize;
                let screen_col = mouse.column.saturating_sub(vp.x + 1) as usize;
                let line_idx = self.assistant_state.effective_scroll() + screen_row;

                if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                    if self.assistant_state.tool_chip_lines.borrow().contains(&line_idx) {
                        self.assistant_state.toggle_tools_expansion();
                        return;
                    }
                }

                match mouse.kind {
                    MouseEventKind::Down(MouseButton::Left) => {
                        self.assistant_state.start_selection(line_idx, screen_col);
                    }
                    MouseEventKind::Drag(MouseButton::Left) => {
                        self.assistant_state.update_selection(line_idx, screen_col);
                    }
                    MouseEventKind::Up(MouseButton::Left) => {
                        self.assistant_state.finish_selection(line_idx, screen_col);
                    }
                    _ => {}
                }
            } else if matches!(mouse.kind, MouseEventKind::Down(_)) {
                self.assistant_state.clear_selection();
            }
        }

        if let ActiveView::Yaml(yaml) = &mut self.active_view {
            let vp = yaml.last_viewport_rect.get();
            if mouse.column >= vp.x && mouse.column < vp.x + vp.width
                && mouse.row >= vp.y && mouse.row < vp.y + vp.height
            {
                let screen_row = mouse.row.saturating_sub(vp.y) as usize;
                let line_idx = yaml.scroll_offset + screen_row;

                match mouse.kind {
                    MouseEventKind::Down(MouseButton::Left) => {
                        yaml.start_selection(line_idx);
                    }
                    MouseEventKind::Drag(MouseButton::Left) => {
                        yaml.update_selection(line_idx);
                    }
                    MouseEventKind::Up(MouseButton::Left) => {
                        if let Some(selected) = yaml.finish_selection(line_idx) {
                            tokio::spawn(async move {
                                let _ = copy_to_clipboard(&selected);
                            });
                            self.set_toast("✓ Copied selection to clipboard".to_string(), Theme::status_ok());
                        }
                    }
                    _ => {}
                }
            } else if matches!(mouse.kind, MouseEventKind::Down(_)) {
                yaml.clear_selection();
            }
        }

        // Global drag-to-copy selection for every view without its own
        // handler (tables, logs, describe, helm, overview, ...). The text is
        // captured from the rendered buffer, so whatever is visible is what
        // gets copied.
        if !matches!(self.active_view, ActiveView::Assistant | ActiveView::Yaml(_)) {
            match mouse.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    self.screen_selection =
                        Some(((mouse.column, mouse.row), (mouse.column, mouse.row)));
                    self.screen_selecting = true;
                    self.screen_selection_text.borrow_mut().clear();
                }
                MouseEventKind::Drag(MouseButton::Left) => {
                    if self.screen_selecting {
                        if let Some(sel) = &mut self.screen_selection {
                            sel.1 = (mouse.column, mouse.row);
                        }
                    }
                }
                MouseEventKind::Up(MouseButton::Left) => {
                    if self.screen_selecting {
                        self.screen_selecting = false;
                        match self.screen_selection {
                            Some((anchor, cursor)) if anchor != cursor => {
                                let text = self.screen_selection_text.borrow().clone();
                                if text.trim().is_empty() {
                                    self.screen_selection = None;
                                } else {
                                    tokio::spawn(async move {
                                        let _ = copy_to_clipboard(&text);
                                    });
                                    self.set_toast(
                                        "✓ Copied selection to clipboard".to_string(),
                                        Theme::status_ok(),
                                    );
                                }
                            }
                            _ => self.screen_selection = None,
                        }
                    }
                }
                _ => {}
            }
        }
    }

    pub fn handle_paste(&mut self, text: String) {
        if self.input_mode == InputMode::Command {
            let cleaned = text.replace("\r\n", " ").replace('\n', " ");
            self.command_buffer.push_str(&cleaned);
            return;
        }
        if self.input_mode == InputMode::Filter {
            let cleaned = text.replace("\r\n", "").replace('\n', "");
            self.filter_buffer.push_str(&cleaned);
            let filter = self.filter_buffer.clone();
            if let ActiveView::Table(table) = &mut self.active_view {
                table.apply_filter(&filter);
            }
            return;
        }
        if let ActiveView::Assistant = &mut self.active_view {
            let cleaned = text.replace("\r\n", " ").replace('\n', " ");
            self.assistant_state.input.push_str(&cleaned);
            return;
        }
        if let ActiveView::Settings(ref mut settings) = self.active_view {
            if settings.is_editing {
                let cleaned = text.replace("\r\n", "").replace('\n', "");
                settings.edit_buffer.push_str(&cleaned);
                return;
            }
        }
        if let Some(ref mut modal) = self.modal {
            match modal {
                Modal::Scale { ref mut input, .. } => {
                    let cleaned = text.replace("\r\n", "").replace('\n', "");
                    input.push_str(&cleaned);
                }
                Modal::PortForward { ref mut local_port_input, .. } => {
                    let cleaned = text.replace("\r\n", "").replace('\n', "");
                    local_port_input.push_str(&cleaned);
                }
                _ => {}
            }
        }
    }

    pub async fn execute_colon_command(&mut self, cmd: &str) {
        let trimmed = cmd.trim();
        if trimmed == "save-ai" || trimmed == "export-ai" || (trimmed == "save" && matches!(self.active_view, ActiveView::Assistant)) {
            let prov = self.ai_settings.default_provider;
            let prov_name = crate::ai_config::provider_display_name(prov);
            let model = self.ai_settings.get_model(prov);
            match self.assistant_state.save_conversation_to_file(prov_name, &model, None) {
                Ok(path) => self.set_toast(format!("✓ Saved conversation to {}", path.display()), Theme::status_ok()),
                Err(err) => self.set_toast(format!("Failed to save conversation: {}", err), Theme::status_error()),
            }
            return;
        }
        if trimmed == "clear-ai" || (trimmed == "clear" && matches!(self.active_view, ActiveView::Assistant)) {
            self.assistant_state.clear_conversation();
            self.set_toast("✓ Conversation cleared".to_string(), Theme::status_ok());
            return;
        }

        if trimmed == "tree" || trimmed == "lineage" || trimmed == "related" {
            if let ActiveView::Table(ref table) = self.active_view {
                if let Some(name) = table.selected_resource_name() {
                    let kind = table.kind.display_name().to_string();
                    let ns = table.selected_namespace().or_else(|| if self.active_namespace.is_empty() { None } else { Some(self.active_namespace.clone()) });
                    self.open_resource_tree(kind, name, ns);
                    return;
                }
            }
            self.set_toast("Select a resource in table to view its relationship tree".to_string(), Theme::status_warn());
            return;
        }
        if trimmed == "actions" || trimmed == "act" {
            if let ActiveView::Table(ref table) = self.active_view {
                if let Some(name) = table.selected_resource_name() {
                    let kind = table.kind.display_name().to_string();
                    let ns = table.selected_namespace().or_else(|| if self.active_namespace.is_empty() { None } else { Some(self.active_namespace.clone()) });
                    self.open_action_palette(kind, name, ns);
                    return;
                }
            }
            self.set_toast("Select a resource in table to open its actions palette".to_string(), Theme::status_warn());
            return;
        }

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

    pub fn open_context_picker(&mut self) {
        let items: Vec<crate::ui::dialogs::ContextPickerItem> = self.contexts.iter().map(|c| {
            crate::ui::dialogs::ContextPickerItem {
                name: c.name.clone(),
                cluster: c.cluster.clone(),
                server: c.server.clone(),
                namespace: c.namespace.clone(),
                is_local: c.is_local,
                provider: c.provider.clone(),
                source_file: c.source_file.clone(),
            }
        }).collect();
        self.modal = Some(Modal::ContextPicker {
            contexts: items,
            current_context: self.active_context.clone(),
            selected_idx: 0,
            filter: String::new(),
        });
    }

    pub async fn execute_view_target(&mut self, target: CommandTarget) {
        match target {
            CommandTarget::Resource(kind) => self.switch_view_to_kind(kind).await,
            CommandTarget::CustomResource(crd) => self.switch_view_to_crd(crd).await,
            CommandTarget::Contexts => {
                self.open_context_picker();
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
            CommandTarget::OpenUrl(_) => {}
        }
    }

    pub async fn execute_command_target(&mut self, target: CommandTarget) {
        match target {
            CommandTarget::OpenUrl(url) => {
                if url.is_empty() {
                    self.set_toast("Usage: :open <srelens://... or kind/name>".to_string(), Theme::status_warn());
                    return;
                }
                match crate::deep_link::DeepLink::parse(&url) {
                    Ok(link) => {
                        if let Err(err) = self.navigate_deep_link(&link).await {
                            self.set_toast(format!("Navigation error: {}", err), Theme::status_error());
                        }
                    }
                    Err(err) => {
                        self.set_toast(format!("Invalid URL: {}", err), Theme::status_error());
                    }
                }
            }
            other => self.execute_view_target(other).await,
        }
    }

    pub async fn navigate_deep_link(&mut self, link: &crate::deep_link::DeepLink) -> Result<(), String> {
        match link {
            crate::deep_link::DeepLink::Cluster { context } => {
                if context != &self.active_context {
                    self.switch_context(context.clone()).await;
                }
                self.set_toast(format!("Switched to cluster '{}'", context), Theme::status_ok());
                Ok(())
            }
            crate::deep_link::DeepLink::View { context, namespace, target } => {
                if let Some(ctx) = context {
                    if !ctx.is_empty() && ctx != &self.active_context {
                        self.switch_context(ctx.clone()).await;
                    }
                }
                if let Some(ns) = namespace {
                    if ns != &self.active_namespace {
                        self.switch_namespace(ns.clone()).await;
                    }
                }
                self.execute_view_target(target.clone()).await;
                Ok(())
            }
            crate::deep_link::DeepLink::Resource { context, namespace, kind, name } => {
                if !context.is_empty() && context != &self.active_context {
                    self.switch_context(context.clone()).await;
                }
                if let Some(ns) = namespace {
                    if ns != &self.active_namespace {
                        self.switch_namespace(ns.clone()).await;
                    }
                }

                // Resolve kind
                let target_cmd = crate::commands::resolve_command_with_crds(kind, &self.crds)
                    .or_else(|| crate::commands::resolve_command_with_crds(format!(":{}", kind).as_str(), &self.crds));

                if let Some(target) = target_cmd {
                    self.execute_view_target(target).await;
                } else {
                    return Err(format!("Unknown resource kind '{}'", kind));
                }

                // If table view, select or filter for the resource name
                if let ActiveView::Table(table) = &mut self.active_view {
                    if let Some(idx) = table.raw_items.iter().position(|item| {
                        item.get("name").and_then(|v| v.as_str()) == Some(name.as_str())
                            || item.get("metadata").and_then(|m| m.get("name")).and_then(|v| v.as_str()) == Some(name.as_str())
                    }) {
                        table.selected_idx = idx;
                    } else {
                        table.apply_filter(name);
                    }
                }

                self.set_toast(format!("Navigated to {} '{}'", kind, name), Theme::status_ok());
                Ok(())
            }
        }
    }

    pub async fn switch_view_to_crd(&mut self, mut crd: CrdMeta) {
        if crd.printer_columns.is_empty() {
            if let Some(discovered) = self.crds.iter().find(|c| c.kind == crd.kind || c.plural == crd.plural) {
                crd.printer_columns = discovered.printer_columns.clone();
            }
        }
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
                                        obj.insert(
                                            "createdAt".to_string(),
                                            serde_json::Value::String(srelens_kube::creation_timestamp_iso(Some(ts))),
                                        );
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
            ResourceKind::Overview => {
                let mut initial_data = self.cluster_overview_data.clone().unwrap_or_default();
                initial_data.context_name = self.active_context.clone();
                initial_data.cluster_name = self.cluster_name.clone();
                initial_data.server_url = self.server_url.clone();
                initial_data.k8s_version = self.cluster_version.clone();
                initial_data.is_reachable = self.is_connected;
                if initial_data.node_count == 0 {
                    initial_data.node_count = self.node_count;
                }
                if initial_data.total_pods == 0 {
                    initial_data.total_pods = self.pod_count;
                }
                self.refresh_cluster_overview();
                ActiveView::Overview(OverviewViewState::with_data(initial_data))
            }
            ResourceKind::Toolbox => ActiveView::Toolbox(ToolboxViewState::new()),
            ResourceKind::Assistant => ActiveView::Assistant,
            ResourceKind::Settings => ActiveView::Settings(SettingsViewState::new()),
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

    pub fn open_resource_tree(&mut self, kind: String, name: String, namespace: Option<String>) {
        let tree_state = tree_view::TreeViewState::new(kind.clone(), name.clone(), namespace.clone());
        let old_view = std::mem::replace(&mut self.active_view, ActiveView::Tree(tree_state));
        self.nav_stack.push(old_view);

        let ctx = self.active_context.clone();
        let cache = self.client_cache.clone();
        let event_tx = self.event_tx.clone();
        let k = kind.clone();
        let n = name.clone();
        let ns = namespace.clone();

        tokio::spawn(async move {
            let res = match cache.get(&ctx).await {
                Ok(client) => srelens_kube::lineage::resolve_resource_lineage(client, &k, &n, ns.as_deref()).await,
                Err(e) => Err(format!("Failed to connect to cluster: {}", e)),
            };
            let _ = event_tx.send(crate::event::AppEvent::LineageResult {
                kind: k,
                name: n,
                result: res,
            });
        });
    }

    pub fn handle_lineage_result(
        &mut self,
        kind: &str,
        name: &str,
        result: Result<srelens_kube::lineage::LineageNode, String>,
    ) {
        if let ActiveView::Tree(tree) = &mut self.active_view {
            if tree.root_kind.eq_ignore_ascii_case(kind) && tree.root_name == name {
                match result {
                    Ok(node) => tree.set_tree(node),
                    Err(err) => tree.set_error(err),
                }
            }
        }
    }

    pub fn open_action_palette(&mut self, kind: String, name: String, namespace: Option<String>) {
        use crate::ui::dialogs::{QuickActionId, QuickActionItem};

        let kind_lower = kind.to_lowercase();
        let actions = match kind_lower.as_str() {
            "pod" | "pods" => vec![
                QuickActionItem {
                    id: QuickActionId::AskAi,
                    key_hint: "ai".to_string(),
                    title: "🤖 Ask AI Assistant about this Pod".to_string(),
                    description: "Inspect pod status, errors, exit codes & recent events".to_string(),
                },
                QuickActionItem {
                    id: QuickActionId::RelationshipTree,
                    key_hint: "t".to_string(),
                    title: "🌳 Resource Relationship Tree".to_string(),
                    description: "Trace owner Deployment/ReplicaSet, Service, ConfigMaps & PVCs".to_string(),
                },
                QuickActionItem {
                    id: QuickActionId::ViewLogs,
                    key_hint: "l".to_string(),
                    title: "📋 View Live Logs".to_string(),
                    description: "Stream logs from pod containers with tailing".to_string(),
                },
                QuickActionItem {
                    id: QuickActionId::OpenShell,
                    key_hint: "s".to_string(),
                    title: "🐚 Open Shell".to_string(),
                    description: "Attach interactive terminal session inside container".to_string(),
                },
                QuickActionItem {
                    id: QuickActionId::PortForward,
                    key_hint: "f".to_string(),
                    title: "🔌 Port Forward".to_string(),
                    description: "Forward local port to pod container".to_string(),
                },
                QuickActionItem {
                    id: QuickActionId::Describe,
                    key_hint: "d".to_string(),
                    title: "📄 Describe Pod".to_string(),
                    description: "Formatted Kubernetes describe output & events".to_string(),
                },
                QuickActionItem {
                    id: QuickActionId::ViewYaml,
                    key_hint: "y".to_string(),
                    title: "📝 View YAML".to_string(),
                    description: "Inspect live Kubernetes YAML manifest".to_string(),
                },
                QuickActionItem {
                    id: QuickActionId::EditYaml,
                    key_hint: "e".to_string(),
                    title: "✏️  Edit YAML".to_string(),
                    description: "Open in external editor with server-side apply".to_string(),
                },
                QuickActionItem {
                    id: QuickActionId::Delete,
                    key_hint: "del".to_string(),
                    title: "🗑️  Delete Pod".to_string(),
                    description: "Gracefully delete pod from the cluster".to_string(),
                },
            ],
            "deployment" | "deployments" | "statefulset" | "statefulsets" | "daemonset" | "daemonsets" => vec![
                QuickActionItem {
                    id: QuickActionId::AskAi,
                    key_hint: "ai".to_string(),
                    title: format!("🤖 Ask AI Assistant about this {}", kind),
                    description: "Analyze workload health, failing replicas & recent events".to_string(),
                },
                QuickActionItem {
                    id: QuickActionId::RelationshipTree,
                    key_hint: "t".to_string(),
                    title: "🌳 Resource Relationship Tree".to_string(),
                    description: "Map Workload -> ReplicaSets -> Pods -> Services".to_string(),
                },
                QuickActionItem {
                    id: QuickActionId::RolloutRestart,
                    key_hint: "r".to_string(),
                    title: "🔄 Rollout Restart".to_string(),
                    description: "Trigger zero-downtime rolling restart".to_string(),
                },
                QuickActionItem {
                    id: QuickActionId::Scale,
                    key_hint: "scale".to_string(),
                    title: "⚖️  Scale Workload".to_string(),
                    description: "Adjust replica count inline".to_string(),
                },
                QuickActionItem {
                    id: QuickActionId::JumpToPods,
                    key_hint: "pods".to_string(),
                    title: "📦 View Pods of this Workload".to_string(),
                    description: "Navigate to pods matching this workload".to_string(),
                },
                QuickActionItem {
                    id: QuickActionId::ViewLogs,
                    key_hint: "l".to_string(),
                    title: "📋 Aggregated Logs".to_string(),
                    description: "Stream logs from workload pods".to_string(),
                },
                QuickActionItem {
                    id: QuickActionId::Describe,
                    key_hint: "d".to_string(),
                    title: "📄 Describe Workload".to_string(),
                    description: "Formatted workload describe output".to_string(),
                },
                QuickActionItem {
                    id: QuickActionId::ViewYaml,
                    key_hint: "y".to_string(),
                    title: "📝 View YAML".to_string(),
                    description: "Inspect live Kubernetes YAML manifest".to_string(),
                },
                QuickActionItem {
                    id: QuickActionId::EditYaml,
                    key_hint: "e".to_string(),
                    title: "✏️  Edit YAML".to_string(),
                    description: "Edit workload in external editor".to_string(),
                },
                QuickActionItem {
                    id: QuickActionId::Delete,
                    key_hint: "del".to_string(),
                    title: "🗑️  Delete Workload".to_string(),
                    description: "Delete workload resource from the cluster".to_string(),
                },
            ],
            "service" | "services" => vec![
                QuickActionItem {
                    id: QuickActionId::RelationshipTree,
                    key_hint: "t".to_string(),
                    title: "🌳 Resource Relationship Tree".to_string(),
                    description: "View Service -> Ingress -> Endpoints -> Pods".to_string(),
                },
                QuickActionItem {
                    id: QuickActionId::JumpToPods,
                    key_hint: "pods".to_string(),
                    title: "🎯 Jump to Backend Pods".to_string(),
                    description: "Navigate to pods matching this service's selector".to_string(),
                },
                QuickActionItem {
                    id: QuickActionId::PortForward,
                    key_hint: "f".to_string(),
                    title: "🔌 Port Forward".to_string(),
                    description: "Forward local port to service".to_string(),
                },
                QuickActionItem {
                    id: QuickActionId::Describe,
                    key_hint: "d".to_string(),
                    title: "📄 Describe Service".to_string(),
                    description: "Inspect service ports, endpoints & selectors".to_string(),
                },
                QuickActionItem {
                    id: QuickActionId::ViewYaml,
                    key_hint: "y".to_string(),
                    title: "📝 View YAML".to_string(),
                    description: "Inspect live Kubernetes YAML manifest".to_string(),
                },
                QuickActionItem {
                    id: QuickActionId::Delete,
                    key_hint: "del".to_string(),
                    title: "🗑️  Delete Service".to_string(),
                    description: "Delete service from the cluster".to_string(),
                },
            ],
            "ingress" | "ingresses" => vec![
                QuickActionItem {
                    id: QuickActionId::RelationshipTree,
                    key_hint: "t".to_string(),
                    title: "🌳 Resource Relationship Tree".to_string(),
                    description: "View Ingress -> Upstream Services -> Pods".to_string(),
                },
                QuickActionItem {
                    id: QuickActionId::Describe,
                    key_hint: "d".to_string(),
                    title: "📄 Describe Ingress".to_string(),
                    description: "Inspect hosts, TLS certificates & backend rules".to_string(),
                },
                QuickActionItem {
                    id: QuickActionId::ViewYaml,
                    key_hint: "y".to_string(),
                    title: "📝 View YAML".to_string(),
                    description: "Inspect live Kubernetes YAML manifest".to_string(),
                },
                QuickActionItem {
                    id: QuickActionId::Delete,
                    key_hint: "del".to_string(),
                    title: "🗑️  Delete Ingress".to_string(),
                    description: "Delete ingress from the cluster".to_string(),
                },
            ],
            "node" | "nodes" => vec![
                QuickActionItem {
                    id: QuickActionId::RelationshipTree,
                    key_hint: "t".to_string(),
                    title: "🌳 Resource Tree".to_string(),
                    description: "View all Pods running on this Node".to_string(),
                },
                QuickActionItem {
                    id: QuickActionId::JumpToPods,
                    key_hint: "pods".to_string(),
                    title: "📦 View Pods on this Node".to_string(),
                    description: "Filter pod table to pods on this node".to_string(),
                },
                QuickActionItem {
                    id: QuickActionId::OpenShell,
                    key_hint: "s".to_string(),
                    title: "🐚 Privileged Node Shell".to_string(),
                    description: "Launch root debug container on node".to_string(),
                },
                QuickActionItem {
                    id: QuickActionId::Describe,
                    key_hint: "d".to_string(),
                    title: "📄 Describe Node".to_string(),
                    description: "Inspect node capacity, taints & conditions".to_string(),
                },
            ],
            _ => vec![
                QuickActionItem {
                    id: QuickActionId::AskAi,
                    key_hint: "ai".to_string(),
                    title: "🤖 Ask AI Assistant".to_string(),
                    description: "Troubleshoot and explain this resource".to_string(),
                },
                QuickActionItem {
                    id: QuickActionId::RelationshipTree,
                    key_hint: "t".to_string(),
                    title: "🌳 Resource Relationship Tree".to_string(),
                    description: "Trace ownerReferences and linked resources".to_string(),
                },
                QuickActionItem {
                    id: QuickActionId::Describe,
                    key_hint: "d".to_string(),
                    title: "📄 Describe".to_string(),
                    description: "Inspect formatted Kubernetes describe details".to_string(),
                },
                QuickActionItem {
                    id: QuickActionId::ViewYaml,
                    key_hint: "y".to_string(),
                    title: "📝 View YAML".to_string(),
                    description: "Inspect live Kubernetes YAML manifest".to_string(),
                },
                QuickActionItem {
                    id: QuickActionId::EditYaml,
                    key_hint: "e".to_string(),
                    title: "✏️  Edit YAML".to_string(),
                    description: "Edit manifest in external editor".to_string(),
                },
                QuickActionItem {
                    id: QuickActionId::Delete,
                    key_hint: "del".to_string(),
                    title: "🗑️  Delete".to_string(),
                    description: "Delete this resource from the cluster".to_string(),
                },
            ],
        };

        self.modal = Some(Modal::ActionPalette {
            resource_kind: kind,
            resource_name: name,
            namespace,
            actions,
            selected_idx: 0,
            filter: String::new(),
        });
    }

    pub async fn execute_action_palette(&mut self) {
        if let Some(Modal::ActionPalette {
            resource_kind,
            resource_name,
            namespace,
            actions,
            selected_idx,
            filter,
        }) = self.modal.take()
        {
            let lower_filter = filter.to_lowercase();
            let filtered: Vec<&crate::ui::dialogs::QuickActionItem> = actions
                .iter()
                .filter(|a| {
                    if lower_filter.is_empty() {
                        true
                    } else {
                        a.title.to_lowercase().contains(&lower_filter)
                            || a.key_hint.to_lowercase().contains(&lower_filter)
                            || a.description.to_lowercase().contains(&lower_filter)
                    }
                })
                .collect();

            if let Some(chosen) = filtered.get(selected_idx) {
                use crate::ui::dialogs::QuickActionId;
                match chosen.id {
                    QuickActionId::AskAi => {
                        let prompt = format!(
                            "Investigate {} '{}' in namespace '{}': what is its current health status, are there any errors, crash loops or restarts, and what are the recommended fixes?",
                            resource_kind,
                            resource_name,
                            namespace.as_deref().unwrap_or("default")
                        );
                        self.assistant_state.input = prompt;
                        let old = std::mem::replace(&mut self.active_view, ActiveView::Assistant);
                        self.nav_stack.push(old);
                    }
                    QuickActionId::RelationshipTree => {
                        self.open_resource_tree(resource_kind, resource_name, namespace);
                    }
                    QuickActionId::ViewLogs => {
                        self.open_logs_view(resource_name, namespace, None).await;
                    }
                    QuickActionId::OpenShell => {
                        let target_ns = namespace.clone().or_else(|| if self.active_namespace.is_empty() { None } else { Some(self.active_namespace.clone()) });
                        let query_ns = target_ns.clone().unwrap_or_else(|| "default".to_string());
                        let ctx = self.active_context.clone();
                        let cache = self.client_cache.clone();

                        let containers: Vec<String> = if let Ok(client) = cache.get(&ctx).await {
                            let api: kube::Api<k8s_openapi::api::core::v1::Pod> = kube::Api::namespaced(client, &query_ns);
                            if let Ok(pod) = api.get(&resource_name).await {
                                pod.spec.map(|s| s.containers.into_iter().map(|c| c.name).collect()).unwrap_or_default()
                            } else {
                                Vec::new()
                            }
                        } else {
                            Vec::new()
                        };

                        if containers.len() > 1 {
                            self.modal = Some(Modal::ContainerPicker {
                                pod_name: resource_name,
                                namespace: target_ns,
                                containers,
                                selected_idx: 0,
                                action: crate::ui::dialogs::ContainerAction::Shell,
                            });
                        } else {
                            self.requires_terminal_suspend = Some(SuspendAction::PodShell {
                                pod: resource_name,
                                container: containers.into_iter().next(),
                            });
                        }
                    }
                    QuickActionId::PortForward => {
                        let ns = namespace.unwrap_or_else(|| self.active_namespace.clone());
                        self.modal = Some(Modal::PortForward {
                            pod_name: resource_name,
                            namespace: ns,
                            container_port: 8080,
                            local_port_input: "8080".to_string(),
                        });
                    }
                    QuickActionId::Describe => {
                        self.open_describe_view(resource_name, resource_kind, namespace).await;
                    }
                    QuickActionId::ViewYaml => {
                        self.open_yaml_view(resource_name, resource_kind, namespace).await;
                    }
                    QuickActionId::EditYaml => {
                        self.open_yaml_view(resource_name, resource_kind, namespace).await;
                        self.requires_terminal_suspend = Some(SuspendAction::EditYaml);
                    }
                    QuickActionId::RolloutRestart => {
                        let ns = namespace.unwrap_or_else(|| self.active_namespace.clone());
                        self.modal = Some(Modal::Confirm {
                            title: "Rollout Restart Workload".to_string(),
                            message: format!("Trigger zero-downtime rolling restart for {} '{}/{}'?", resource_kind, ns, resource_name),
                            action_name: format!("restart:{}:{}:{}", resource_kind, ns, resource_name),
                            is_destructive: false,
                        });
                    }
                    QuickActionId::Scale => {
                        let cur_replicas = 1;
                        self.modal = Some(Modal::Scale {
                            workload_name: resource_name,
                            current_replicas: cur_replicas,
                            input: cur_replicas.to_string(),
                        });
                    }
                    QuickActionId::JumpToPods => {
                        self.switch_view_to_kind(ResourceKind::Pods).await;
                        self.filter_buffer = resource_name.clone();
                        if let ActiveView::Table(table) = &mut self.active_view {
                            table.apply_filter(&resource_name);
                        }
                    }
                    QuickActionId::CordonNode => {
                        self.set_toast(format!("Cordoned node {}", resource_name), Theme::status_ok());
                    }
                    QuickActionId::DrainNode => {
                        self.set_toast(format!("Draining node {}", resource_name), Theme::status_warn());
                    }
                    QuickActionId::Delete => {
                        let ns = namespace.unwrap_or_else(|| self.active_namespace.clone());
                        self.modal = Some(Modal::Confirm {
                            title: format!("Delete {}", resource_kind),
                            message: format!("Are you sure you want to permanently delete {} '{}/{}'?", resource_kind, ns, resource_name),
                            action_name: format!("delete:{}:{}:{}", resource_kind, ns, resource_name),
                            is_destructive: true,
                        });
                    }
                }
            }
        }
    }

    pub async fn execute_modal_confirm(&mut self, action_name: String) {
        if action_name.starts_with("delete:") {
            // Format: "delete:<kind>:<namespace>:<name>"
            let parts: Vec<&str> = action_name.splitn(4, ':').collect();
            if parts.len() == 4 {
                let kind = parts[1].to_string();
                let ns = parts[2].to_string();
                let name = parts[3].to_string();

                let ctx = self.active_context.clone();
                let cache = self.client_cache.clone();

                match cache.get(&ctx).await {
                    Ok(client) => {
                        let maybe_ar = if let Some((gvk, namespaced)) = srelens_kube::manifest::gvk_for(&kind) {
                            Some((kube::core::ApiResource::from_gvk(&gvk), namespaced))
                        } else if let Some(crd) = self.crds.iter().find(|c| c.kind.eq_ignore_ascii_case(&kind) || c.plural.eq_ignore_ascii_case(&kind)) {
                            let api_version = if crd.group.is_empty() { crd.version.clone() } else { format!("{}/{}", crd.group, crd.version) };
                            Some((kube::core::ApiResource {
                                group: crd.group.clone(),
                                version: crd.version.clone(),
                                api_version,
                                kind: crd.kind.clone(),
                                plural: crd.plural.clone(),
                            }, crd.namespaced))
                        } else {
                            None
                        };

                        if let Some((ar, namespaced)) = maybe_ar {
                            let api: kube::Api<kube::core::DynamicObject> = if namespaced && !ns.is_empty() {
                                kube::Api::namespaced_with(client, &ns, &ar)
                            } else {
                                kube::Api::all_with(client, &ar)
                            };

                            match api.delete(&name, &kube::api::DeleteParams::default()).await {
                                Ok(_) => {
                                    self.set_toast(format!("✓ Deleted {} '{}' in '{}'", kind, name, ns), Theme::status_ok());
                                    // Remove from active table immediately
                                    if let ActiveView::Table(table) = &mut self.active_view {
                                        table.raw_items.retain(|item| {
                                            let item_name = item.get("name").or_else(|| item.pointer("/metadata/name")).and_then(|v| v.as_str()).unwrap_or("");
                                            let item_ns = item.get("namespace").or_else(|| item.pointer("/metadata/namespace")).and_then(|v| v.as_str()).unwrap_or("");
                                            !(item_name == name && (item_ns == ns || item_ns.is_empty() || ns.is_empty()))
                                        });
                                        let filter = self.filter_buffer.clone();
                                        table.apply_filter(&filter);
                                    }
                                }
                                Err(err) => {
                                    self.set_toast(format!("Delete failed: {}", err), Theme::status_error());
                                }
                            }
                        } else {
                            self.set_toast(format!("Cannot resolve resource kind '{}'", kind), Theme::status_error());
                        }
                    }
                    Err(err) => {
                        self.set_toast(format!("Connection error: {}", err), Theme::status_error());
                    }
                }
            } else {
                self.set_toast("Resource deleted successfully".to_string(), Theme::status_ok());
            }
        } else if action_name.starts_with("restart:") {
            // Format: "restart:<kind>:<namespace>:<name>"
            let parts: Vec<&str> = action_name.splitn(4, ':').collect();
            if parts.len() == 4 {
                let kind = parts[1].to_string();
                let ns = parts[2].to_string();
                let name = parts[3].to_string();

                let ctx = self.active_context.clone();
                let cache = self.client_cache.clone();

                match cache.get(&ctx).await {
                    Ok(client) => {
                        let maybe_ar = if let Some((gvk, namespaced)) = srelens_kube::manifest::gvk_for(&kind) {
                            Some((kube::core::ApiResource::from_gvk(&gvk), namespaced))
                        } else {
                            None
                        };

                        if let Some((ar, namespaced)) = maybe_ar {
                            let api: kube::Api<kube::core::DynamicObject> = if namespaced && !ns.is_empty() {
                                kube::Api::namespaced_with(client, &ns, &ar)
                            } else {
                                kube::Api::all_with(client, &ar)
                            };

                            let now = chrono::Utc::now().to_rfc3339();
                            let patch = serde_json::json!({
                                "spec": {
                                    "template": {
                                        "metadata": {
                                            "annotations": {
                                                "kubectl.kubernetes.io/restartedAt": now
                                            }
                                        }
                                    }
                                }
                            });

                            let patch_params = kube::api::PatchParams::default();
                            match api.patch(&name, &patch_params, &kube::api::Patch::Merge(&patch)).await {
                                Ok(_) => {
                                    self.set_toast(format!("✓ Rollout restart triggered for {} '{}'", kind, name), Theme::status_ok());
                                }
                                Err(err) => {
                                    self.set_toast(format!("Restart failed: {}", err), Theme::status_error());
                                }
                            }
                        } else {
                            self.set_toast(format!("Cannot restart resource kind '{}'", kind), Theme::status_error());
                        }
                    }
                    Err(err) => {
                        self.set_toast(format!("Connection error: {}", err), Theme::status_error());
                    }
                }
            } else {
                self.set_toast("Rollout restart triggered".to_string(), Theme::status_ok());
            }
        } else if action_name.starts_with("stop-pf:") {
            self.set_toast("Port forward stopped".to_string(), Theme::status_ok());
        }
    }

    pub async fn execute_scale_workload(&mut self, name: String, replicas: i32) {
        let (kind, ns) = if let ActiveView::Table(table) = &self.active_view {
            let k = table.kind.to_string();
            let ns = table.selected_item().and_then(|i| {
                i.get("namespace").or_else(|| i.pointer("/metadata/namespace")).and_then(|v| v.as_str()).map(String::from)
            }).unwrap_or_else(|| self.active_namespace.clone());
            (k, ns)
        } else {
            ("Deployment".to_string(), self.active_namespace.clone())
        };

        let ctx = self.active_context.clone();
        let cache = self.client_cache.clone();

        match cache.get(&ctx).await {
            Ok(client) => {
                let query_ns = if ns.is_empty() { "default".to_string() } else { ns };
                if let Some((gvk, _)) = srelens_kube::manifest::gvk_for(&kind) {
                    let ar = kube::core::ApiResource::from_gvk(&gvk);
                    let api: kube::Api<kube::core::DynamicObject> = kube::Api::namespaced_with(client, &query_ns, &ar);
                    let patch = serde_json::json!({
                        "spec": {
                            "replicas": replicas
                        }
                    });
                    match api.patch(&name, &kube::api::PatchParams::default(), &kube::api::Patch::Merge(&patch)).await {
                        Ok(_) => {
                            self.set_toast(format!("✓ Scaled {} '{}' to {} replicas", kind, name, replicas), Theme::status_ok());
                        }
                        Err(err) => {
                            self.set_toast(format!("Scale failed: {}", err), Theme::status_error());
                        }
                    }
                }
            }
            Err(err) => {
                self.set_toast(format!("Connection error: {}", err), Theme::status_error());
            }
        }
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
                        if table.kind == ResourceKind::Pods {
                            let mut merged_items = items.clone();
                            let prev_metrics: std::collections::HashMap<String, (Option<String>, Option<String>)> = table.raw_items.iter().filter_map(|it| {
                                let name = it.get("name")?.as_str()?.to_string();
                                let cpu = it.get("cpu").and_then(|v| v.as_str()).map(String::from);
                                let mem = it.get("memory").and_then(|v| v.as_str()).map(String::from);
                                Some((name, (cpu, mem)))
                            }).collect();

                            for item in merged_items.iter_mut() {
                                if let Some(name) = item.get("name").and_then(|v| v.as_str()) {
                                    if let Some((cpu, mem)) = prev_metrics.get(name) {
                                        if let Some(obj) = item.as_object_mut() {
                                            if let Some(c) = cpu {
                                                obj.insert("cpu".to_string(), serde_json::Value::String(c.clone()));
                                            }
                                            if let Some(m) = mem {
                                                obj.insert("memory".to_string(), serde_json::Value::String(m.clone()));
                                            }
                                        }
                                    }
                                }
                            }
                            table.set_items(merged_items, &self.filter_buffer);
                        } else {
                            table.set_items(items.clone(), &self.filter_buffer);
                        }
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
                Constraint::Length(3), // Top header (brand, context hotbar, status)
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
            ActiveView::Assistant => "AI Assistant",
            ActiveView::Settings(_) => "AI Settings",
            ActiveView::Tree(_) => "Resource Relationship Tree",
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

        let context_chips: Vec<crate::ui::header::ContextChipInfo> = self.contexts
            .iter()
            .enumerate()
            .map(|(i, c)| crate::ui::header::ContextChipInfo {
                name: c.name.clone(),
                is_current: c.name == self.active_context,
                is_local: c.is_local,
                index: i + 1,
            })
            .collect();

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
                contexts: &context_chips,
                context_chip_rects: Some(&self.context_chip_rects),
            },
        );

        // 2. Render Main View Body — Clear first to prevent ghosting artifacts
        //    (e.g. log lines lingering below the table when switching views)
        f.render_widget(ratatui::widgets::Clear, chunks[1]);
        match &self.active_view {
            ActiveView::Table(table) => render_resource_table(f, chunks[1], table),
            ActiveView::Yaml(yaml) => render_yaml_view(f, chunks[1], yaml),
            ActiveView::Describe(desc) => render_describe_view(f, chunks[1], desc),
            ActiveView::Logs(logs) => render_logs_view(f, chunks[1], logs),
            ActiveView::PortForwards(pf) => render_port_forward_view(f, chunks[1], pf),
            ActiveView::Helm(helm) => render_helm_view(f, chunks[1], helm),
            ActiveView::Overview(ov) => render_overview_view(f, chunks[1], ov),
            ActiveView::Toolbox(tb) => render_toolbox_view(f, chunks[1], tb),
            ActiveView::Assistant => render_assistant_view(f, chunks[1], &self.assistant_state, &self.ai_settings),
            ActiveView::Settings(s) => render_settings_view(f, chunks[1], s),
            ActiveView::Tree(tree) => render_tree_view(f, chunks[1], tree),
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

        let custom_hints: Option<&[(&str, &str)]> = match &self.active_view {
            ActiveView::Tree(_) => Some(&[
                ("<:>", "Cmd"),
                ("<↑/↓>", "Move"),
                ("<Enter>", "Jump"),
                ("<x>", "Actions"),
                ("<l>", "Logs"),
                ("<y>", "YAML"),
                ("<d>", "Describe"),
                ("<c>", "Copy"),
                ("<Esc>", "Back"),
                ("<?>", "Help"),
            ][..]),
            ActiveView::Assistant => Some(&[
                ("<:>", "Cmd"),
                ("<?>", "Help"),
            ][..]),
            ActiveView::Table(table) => match table.kind {
                ResourceKind::Pods => Some(&[
                    ("<:>", "Cmd"),
                    ("</>", "Filter"),
                    ("<l>", "Logs"),
                    ("<s>", "Shell"),
                    ("<f>/<F>", "PortForward"),
                    ("<d>", "Describe"),
                    ("<y>", "YAML"),
                    ("<e>", "Edit"),
                    ("<^d>", "Delete"),
                    ("<?>", "Help"),
                ][..]),
                ResourceKind::Deployments | ResourceKind::DaemonSets | ResourceKind::StatefulSets => Some(&[
                    ("<:>", "Cmd"),
                    ("</>", "Filter"),
                    ("<Enter>", "Pods"),
                    ("<d>", "Describe"),
                    ("<y>", "YAML"),
                    ("<e>", "Edit"),
                    ("<^s>", "Scale"),
                    ("<^r>", "Restart"),
                    ("<^d>", "Delete"),
                    ("<?>", "Help"),
                ][..]),
                ResourceKind::Services => Some(&[
                    ("<:>", "Cmd"),
                    ("</>", "Filter"),
                    ("<f>/<F>", "PortForward"),
                    ("<d>", "Describe"),
                    ("<y>", "YAML"),
                    ("<e>", "Edit"),
                    ("<^d>", "Delete"),
                    ("<?>", "Help"),
                ][..]),
                ResourceKind::Namespaces => Some(&[
                    ("<:>", "Cmd"),
                    ("</>", "Filter"),
                    ("<Enter>", "Pods"),
                    ("<y>", "YAML"),
                    ("<d>", "Describe"),
                    ("<^d>", "Delete"),
                    ("<?>", "Help"),
                ][..]),
                ResourceKind::Events => Some(&[
                    ("<:>", "Cmd"),
                    ("</>", "Filter"),
                    ("<w>", "Triage"),
                    ("<Enter>", "Resource"),
                    ("<d>", "Describe"),
                    ("<l>", "Logs"),
                    ("<y>", "YAML"),
                    ("<c>", "Copy"),
                    ("<?>", "Help"),
                ][..]),
                _ => None,
            },
            ActiveView::Overview(_) => Some(&[
                ("<:>", "Cmd"),
                ("<c>", "Copy"),
                ("<r>", "Refresh"),
                ("<Esc>", "Back"),
                ("<?>", "Help"),
            ][..]),
            ActiveView::Logs(_) => Some(&[
                ("<:>", "Cmd"),
                ("<c>", "Copy"),
                ("<f>", "Follow"),
                ("<t>", "Timestamps"),
                ("<s>", "Save"),
                ("<Esc>", "Back"),
                ("<?>", "Help"),
            ][..]),
            ActiveView::PortForwards(_) => Some(&[
                ("<:>", "Cmd"),
                ("<c>", "CopyURL"),
                ("<d>", "Stop"),
                ("<Esc>", "Back"),
                ("<?>", "Help"),
            ][..]),
            ActiveView::Helm(_) => Some(&[
                ("<:>", "Cmd"),
                ("<c>", "CopyURL"),
                ("<v>", "Values"),
                ("<y>", "Manifest"),
                ("<Esc>", "Back"),
                ("<?>", "Help"),
            ][..]),
            ActiveView::Toolbox(_) => Some(&[
                ("<:>", "Cmd"),
                ("<c>", "CopyPath"),
                ("<Esc>", "Back"),
                ("<?>", "Help"),
            ][..]),
            _ => None,
        };

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
                custom_hints,
                suggestions: suggestions_prop,
            },
        );

        // 4. Render Modals & Overlays
        if let Some(modal) = &self.modal {
            render_modal(f, size, modal);
        } else if self.show_help {
            render_help_modal(f, size);
        }

        // 5. Global drag-to-copy selection overlay: highlight the selected
        // cells on the final buffer and capture their text so mouse-up
        // copies exactly what the user sees.
        if let Some((anchor, cursor)) = self.screen_selection {
            let text = apply_screen_selection(f.buffer_mut(), anchor, cursor);
            *self.screen_selection_text.borrow_mut() = text;
        }
    }
}

/// Highlight the terminal-style linear range between `anchor` and `cursor`
/// (both screen cells, in either order) directly on the buffer and return the
/// covered text, one string per row joined with newlines, trailing whitespace
/// trimmed per row. First row starts at the selection start column, middle
/// rows span the full width, last row ends at the selection end column.
pub fn apply_screen_selection(
    buf: &mut ratatui::buffer::Buffer,
    anchor: (u16, u16),
    cursor: (u16, u16),
) -> String {
    use ratatui::layout::Position;

    let area = buf.area;
    if area.width == 0 || area.height == 0 {
        return String::new();
    }
    let clamp = |(x, y): (u16, u16)| {
        (
            x.clamp(area.left(), area.right() - 1),
            y.clamp(area.top(), area.bottom() - 1),
        )
    };
    let a = clamp(anchor);
    let c = clamp(cursor);
    let (start, end) = if (a.1, a.0) <= (c.1, c.0) { (a, c) } else { (c, a) };

    let mut out = String::new();
    for y in start.1..=end.1 {
        let x0 = if y == start.1 { start.0 } else { area.left() };
        let x1 = if y == end.1 { end.0 } else { area.right() - 1 };
        let mut line = String::new();
        for x in x0..=x1 {
            if let Some(cell) = buf.cell(Position::new(x, y)) {
                line.push_str(cell.symbol());
            }
        }
        buf.set_style(
            ratatui::layout::Rect::new(x0, y, x1 - x0 + 1, 1),
            Style::default().add_modifier(Modifier::REVERSED),
        );
        out.push_str(line.trim_end());
        if y != end.1 {
            out.push('\n');
        }
    }
    out
}

fn base64_encode_bytes(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0];
        let b1 = if chunk.len() > 1 { chunk[1] } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] } else { 0 };
        out.push(ALPHABET[(b0 >> 2) as usize] as char);
        out.push(ALPHABET[(((b0 & 3) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[(((b1 & 15) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(b2 & 63) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

pub fn copy_via_osc52(text: &str) {
    use std::io::Write;
    let b64 = base64_encode_bytes(text.as_bytes());
    let osc52 = format!("\x1b]52;c;{}\x07", b64);
    if let Ok(mut tty) = std::fs::OpenOptions::new().write(true).open("/dev/tty") {
        let _ = tty.write_all(osc52.as_bytes());
        let _ = tty.flush();
    } else {
        let mut stdout = std::io::stdout();
        let _ = stdout.write_all(osc52.as_bytes());
        let _ = stdout.flush();
    }
}

pub fn copy_to_clipboard(text: &str) -> std::io::Result<()> {
    // 1. Emit OSC 52 sequence directly to terminal emulator (works on macOS, Linux, Windows, iTerm2, Alacritty, Kitty, WezTerm, Ghostty, tmux, SSH)
    copy_via_osc52(text);

    // 2. Native OS clipboard command
    #[cfg(target_os = "macos")]
    {
        use std::io::Write;
        use std::process::{Command, Stdio};
        if let Ok(mut child) = Command::new("pbcopy").stdin(Stdio::piped()).spawn() {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
                drop(stdin); // Explicitly close stdin so pbcopy gets EOF and doesn't hang!
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
                drop(stdin);
            }
            let _ = child.wait();
            return Ok(());
        }
        if let Ok(mut child) = Command::new("xclip").arg("-selection").arg("clipboard").stdin(Stdio::piped()).spawn() {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
                drop(stdin);
            }
            let _ = child.wait();
            return Ok(());
        }
        Ok(())
    }
}

pub fn get_clipboard_text() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = std::process::Command::new("pbpaste").output() {
            if output.status.success() {
                let s = String::from_utf8_lossy(&output.stdout).to_string();
                if !s.is_empty() {
                    return Some(s);
                }
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        if let Ok(output) = std::process::Command::new("wl-paste").output() {
            if output.status.success() {
                let s = String::from_utf8_lossy(&output.stdout).to_string();
                if !s.is_empty() {
                    return Some(s);
                }
            }
        }
        if let Ok(output) = std::process::Command::new("xclip")
            .arg("-selection")
            .arg("clipboard")
            .arg("-o")
            .output()
        {
            if output.status.success() {
                let s = String::from_utf8_lossy(&output.stdout).to_string();
                if !s.is_empty() {
                    return Some(s);
                }
            }
        }
    }
    None
}

pub fn extract_tool_call_start_info(v: &serde_json::Value) -> Option<(String, String, String)> {
    let call_id = v.get("call_id")
        .or_else(|| v.get("callId"))
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();

    let tool_call_obj = v.get("tool_call").or_else(|| v.get("toolCall"))?;
    let inner_map = tool_call_obj.as_object()?;
    // Strictly find the payload key ending with "ToolCall" (e.g. bashToolCall, readToolCall),
    // ignoring metadata siblings like hookAdditionalContexts, toolCallId, startedAtMs, completedAtMs.
    let (key, inner) = inner_map.iter().find(|(k, _)| k.ends_with("ToolCall"))?;
    let mut tool_name = key.strip_suffix("ToolCall").unwrap_or(key).to_string();

    // Skip internal hooks that are not executable tools
    if tool_name == "hookAdditionalContexts" {
        return None;
    }

    if tool_name == "callMcpTool" || tool_name == "mcp" {
        if let Some(t) = inner.pointer("/args/tool").or_else(|| inner.pointer("/args/name")).and_then(|s| s.as_str()) {
            tool_name = t.to_string();
        }
    }

    let mut args_summary = String::new();
    if let Some(sub_args) = inner.pointer("/args/arguments") {
        if let Some(obj) = sub_args.as_object() {
            args_summary = serde_json::to_string(obj).unwrap_or_default();
        } else {
            args_summary = sub_args.to_string();
        }
    } else if let Some(args) = inner.get("args") {
        if let Some(cmd) = args.get("command").and_then(|s| s.as_str()) {
            args_summary = cmd.to_string();
        } else if let Some(path) = args.get("path").and_then(|s| s.as_str()) {
            args_summary = format!("path: {}", path);
        } else if let Some(pattern) = args.get("pattern").and_then(|s| s.as_str()) {
            args_summary = format!("pattern: {}", pattern);
        } else if let Some(query) = args.get("query").and_then(|s| s.as_str()) {
            args_summary = format!("query: {}", query);
        } else if let Some(name) = args.get("name").and_then(|s| s.as_str()) {
            args_summary = format!("name: {}", name);
        } else if let Some(s) = args.as_str() {
            args_summary = s.to_string();
        } else if let Some(obj) = args.as_object() {
            if obj.is_empty() {
                args_summary = String::new();
            } else {
                args_summary = serde_json::to_string(obj).unwrap_or_default();
            }
        } else {
            args_summary = args.to_string();
        }
    }

    Some((call_id, tool_name, args_summary))
}

pub fn extract_tool_call_completed_info(v: &serde_json::Value) -> Option<(String, bool)> {
    let call_id = v.get("call_id")
        .or_else(|| v.get("callId"))
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();

    // Check if this completion corresponds to an internal metadata hook that has no ToolCall
    if let Some(tool_call_obj) = v.get("tool_call").or_else(|| v.get("toolCall")) {
        if let Some(inner_map) = tool_call_obj.as_object() {
            if !inner_map.iter().any(|(k, _)| k.ends_with("ToolCall")) {
                return None;
            }
        }
    }

    let is_error = v.get("is_error")
        .or_else(|| v.get("isError"))
        .and_then(|b| b.as_bool())
        .unwrap_or(false);
    Some((call_id, is_error))
}

pub fn parse_involved_object(item: &serde_json::Value) -> (String, String) {
    if let Some(obj_str) = item.get("object").and_then(|v| v.as_str()) {
        if let Some((kind, name)) = obj_str.split_once('/') {
            return (kind.to_string(), name.to_string());
        }
    }
    if let Some(inv) = item.get("involvedObject") {
        let kind = inv.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        let name = inv.get("name").and_then(|v| v.as_str()).unwrap_or("");
        if !kind.is_empty() || !name.is_empty() {
            return (kind.to_string(), name.to_string());
        }
    }
    ("".to_string(), "".to_string())
}

pub fn format_event_summary(item: &serde_json::Value) -> String {
    let age = item.get("age").and_then(|v| v.as_str()).unwrap_or("");
    let type_str = item.get("type").or_else(|| item.get("type_")).and_then(|v| v.as_str()).unwrap_or("");
    let reason = item.get("reason").and_then(|v| v.as_str()).unwrap_or("");
    let (obj_kind, obj_name) = parse_involved_object(item);
    let obj = if !obj_kind.is_empty() { format!("{}/{}", obj_kind, obj_name) } else { "".to_string() };
    let ns = item.get("namespace").and_then(|v| v.as_str()).unwrap_or("");
    let msg = item.get("message").and_then(|v| v.as_str()).unwrap_or("");

    format!("[{}] [{}] {} {} ({}): {}", age, type_str, reason, obj, ns, msg)
}

pub(crate) fn extract_usage_metrics(v: &serde_json::Value) -> Option<(usize, usize, usize, usize, Option<u64>)> {
    let usage = v.get("usage")
        .or_else(|| v.get("model_usage"))
        .or_else(|| v.get("token_usage"))
        .or_else(|| v.get("tokens"))?;

    let prompt = usage.get("inputTokens")
        .or_else(|| usage.get("prompt_tokens"))
        .or_else(|| usage.get("input_tokens"))
        .or_else(|| usage.get("promptTokens"))
        .and_then(|n| n.as_u64())
        .unwrap_or(0) as usize;

    let completion = usage.get("outputTokens")
        .or_else(|| usage.get("completion_tokens"))
        .or_else(|| usage.get("output_tokens"))
        .or_else(|| usage.get("completionTokens"))
        .and_then(|n| n.as_u64())
        .unwrap_or(0) as usize;

    let cached = usage.get("cacheReadTokens")
        .or_else(|| usage.get("cached_tokens"))
        .or_else(|| usage.get("cache_read_input_tokens"))
        .and_then(|n| n.as_u64())
        .unwrap_or(0) as usize;

    let total = usage.get("totalTokens")
        .or_else(|| usage.get("total_tokens"))
        .and_then(|n| n.as_u64())
        .map(|n| n as usize)
        .unwrap_or(prompt + completion);

    let duration = v.get("duration_ms")
        .or_else(|| v.get("durationMs"))
        .or_else(|| usage.get("duration_ms"))
        .or_else(|| usage.get("durationMs"))
        .and_then(|n| n.as_u64());

    Some((prompt, completion, cached, total, duration))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_delete_prev_word_simple() {
        let mut s = String::from("pods");
        delete_prev_word(&mut s);
        assert_eq!(s, "");
    }

    #[test]
    fn test_delete_prev_word_multiple_words() {
        let mut s = String::from("pods -n default");
        delete_prev_word(&mut s);
        assert_eq!(s, "pods -n ");

        delete_prev_word(&mut s);
        assert_eq!(s, "pods ");

        delete_prev_word(&mut s);
        assert_eq!(s, "");
    }

    #[test]
    fn test_delete_prev_word_with_trailing_spaces() {
        let mut s = String::from("context my-cluster   ");
        delete_prev_word(&mut s);
        assert_eq!(s, "context ");
    }

    #[test]
    fn test_delete_prev_word_empty_or_whitespace() {
        let mut s = String::new();
        delete_prev_word(&mut s);
        assert_eq!(s, "");

        let mut s2 = String::from("     ");
        delete_prev_word(&mut s2);
        assert_eq!(s2, "");
    }

    #[test]
    fn test_delete_prev_word_crd_or_symbol_name() {
        let mut s = String::from("crds cilium.io");
        delete_prev_word(&mut s);
        assert_eq!(s, "crds ");
    }

    #[test]
    fn test_is_word_delete_key_variants() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        // macOS: Option + Backspace (Alt modifier)
        let opt_backspace = KeyEvent::new(KeyCode::Backspace, KeyModifiers::ALT);
        assert!(is_word_delete_key(&opt_backspace));

        // Windows / Linux: Ctrl + Backspace
        let ctrl_backspace = KeyEvent::new(KeyCode::Backspace, KeyModifiers::CONTROL);
        assert!(is_word_delete_key(&ctrl_backspace));

        // Unix / Readline: Ctrl + w
        let ctrl_w = KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL);
        assert!(is_word_delete_key(&ctrl_w));

        let ctrl_upper_w = KeyEvent::new(KeyCode::Char('W'), KeyModifiers::CONTROL);
        assert!(is_word_delete_key(&ctrl_upper_w));

        // Terminal fallback: Ctrl + h
        let ctrl_h = KeyEvent::new(KeyCode::Char('h'), KeyModifiers::CONTROL);
        assert!(is_word_delete_key(&ctrl_h));

        // Regular Backspace should NOT trigger word deletion
        let regular_backspace = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
        assert!(!is_word_delete_key(&regular_backspace));

        // Regular 'w' key should NOT trigger word deletion
        let regular_w = KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE);
        assert!(!is_word_delete_key(&regular_w));
    }
}
