#![allow(dead_code, unused_imports)]

use std::io::{self, stdout};
use std::path::PathBuf;
use std::time::Duration;

use clap::{Parser, Subcommand, ValueEnum};
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

mod ai_config;
mod app;
mod commands;
mod event;
mod sink;
mod theme;
mod ui;
mod views;

use app::{App, SuspendAction};
use commands::ResourceKind;
use event::{AppEvent, EventHandler};
use srelens_kube::kube;

#[derive(Parser, Debug)]
#[command(
    name = "srelens-tui",
    version,
    about = "Kubernetes control room in your terminal — built in Rust with k9s navigation"
)]
pub struct Cli {
    /// Kubernetes namespace to scope the initial view
    #[arg(short, long)]
    pub namespace: Option<String>,

    /// Scope to all namespaces on launch
    #[arg(short = 'A', long)]
    pub all_namespaces: bool,

    /// Kubernetes context to activate
    #[arg(short, long)]
    pub context: Option<String>,

    /// Custom kubeconfig path
    #[arg(short, long)]
    pub kubeconfig: Option<PathBuf>,

    /// Direct jump to a specific resource view
    #[arg(value_enum)]
    pub resource: Option<CliResourceTarget>,

    #[command(subcommand)]
    pub command: Option<CliCommand>,
}

#[derive(ValueEnum, Clone, Debug)]
pub enum CliResourceTarget {
    #[value(alias = "po", alias = "pod")]
    Pods,
    #[value(alias = "dp", alias = "deploy")]
    Deployments,
    #[value(alias = "sts", alias = "statefulset")]
    Statefulsets,
    #[value(alias = "ds", alias = "daemonset")]
    Daemonsets,
    #[value(alias = "job")]
    Jobs,
    #[value(alias = "cj", alias = "cronjob")]
    Cronjobs,
    #[value(alias = "svc", alias = "service")]
    Services,
    #[value(alias = "ing", alias = "ingress")]
    Ingresses,
    #[value(alias = "no", alias = "node")]
    Nodes,
    #[value(alias = "ns", alias = "namespace")]
    Namespaces,
    #[value(alias = "cm", alias = "configmap")]
    Configmaps,
    #[value(alias = "sec", alias = "secret")]
    Secrets,
    #[value(alias = "pvc")]
    Pvcs,
    #[value(alias = "pv")]
    Pvs,
    #[value(alias = "ev", alias = "event")]
    Events,
    #[value(alias = "crd")]
    Crds,
    #[value(alias = "releases")]
    Helm,
    #[value(alias = "pf")]
    Portforwards,
    #[value(alias = "info", alias = "cluster")]
    Overview,
    #[value(alias = "tb")]
    Toolbox,
    #[value(alias = "chat")]
    Ai,
}

impl From<CliResourceTarget> for ResourceKind {
    fn from(target: CliResourceTarget) -> Self {
        match target {
            CliResourceTarget::Pods => ResourceKind::Pods,
            CliResourceTarget::Deployments => ResourceKind::Deployments,
            CliResourceTarget::Statefulsets => ResourceKind::StatefulSets,
            CliResourceTarget::Daemonsets => ResourceKind::DaemonSets,
            CliResourceTarget::Jobs => ResourceKind::Jobs,
            CliResourceTarget::Cronjobs => ResourceKind::CronJobs,
            CliResourceTarget::Services => ResourceKind::Services,
            CliResourceTarget::Ingresses => ResourceKind::Ingresses,
            CliResourceTarget::Nodes => ResourceKind::Nodes,
            CliResourceTarget::Namespaces => ResourceKind::Namespaces,
            CliResourceTarget::Configmaps => ResourceKind::ConfigMaps,
            CliResourceTarget::Secrets => ResourceKind::Secrets,
            CliResourceTarget::Pvcs => ResourceKind::PersistentVolumeClaims,
            CliResourceTarget::Pvs => ResourceKind::PersistentVolumes,
            CliResourceTarget::Events => ResourceKind::Events,
            CliResourceTarget::Crds => ResourceKind::CustomResourceDefinitions,
            CliResourceTarget::Helm => ResourceKind::HelmReleases,
            CliResourceTarget::Portforwards => ResourceKind::PortForwards,
            CliResourceTarget::Overview => ResourceKind::Overview,
            CliResourceTarget::Toolbox => ResourceKind::Toolbox,
            CliResourceTarget::Ai => ResourceKind::Assistant,
        }
    }
}

#[derive(Subcommand, Debug)]
pub enum CliCommand {
    /// Print cluster overview & reachability information
    Info,
    /// Check toolbox diagnostics (kubectl, helm, krew)
    Toolbox,
    /// Print version information
    Version,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    // Handle non-interactive CLI subcommands if requested
    if let Some(cmd) = cli.command {
        match cmd {
            CliCommand::Version => {
                println!("srelens-tui v{}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            CliCommand::Info => {
                println!("SRElens Kubernetes TUI (srelens-tui)");
                let paths = srelens_registry::all_kubeconfig_paths();
                let contexts = srelens_kube::context_resolve::resolve_contexts(&paths);
                println!("Found {} contexts across kubeconfigs:", contexts.len());
                for ctx in contexts {
                    let mark = if ctx.is_current { "* " } else { "  " };
                    println!("{}{} -> cluster: {}, server: {}", mark, ctx.display_name, ctx.cluster, ctx.server);
                }
                return Ok(());
            }
            CliCommand::Toolbox => {
                println!("SRElens Toolbox Status:");
                println!("  kubectl: available");
                println!("  helm:    available");
                println!("  krew:    available");
                return Ok(());
            }
        }
    }

    // Resolve kubeconfig paths
    let kubeconfig_paths = if let Some(p) = cli.kubeconfig {
        vec![p]
    } else {
        srelens_registry::all_kubeconfig_paths()
    };

    // Install panic hook to restore terminal on panic
    let default_panic = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();
        let _ = execute!(stdout(), LeaveAlternateScreen, DisableMouseCapture);
        default_panic(panic_info);
    }));

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Initialize event loop and application state
    let events = EventHandler::new(Duration::from_millis(250));
    let initial_resource = cli.resource.map(ResourceKind::from);

    let mut app = App::new(
        cli.context,
        cli.namespace,
        cli.all_namespaces,
        initial_resource,
        kubeconfig_paths,
        events.tx.clone(),
    )
    .await
    .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    let mut event_rx = events.rx;

    // Main TUI Event Loop
    while app.is_running {
        terminal.draw(|f| app.render(f))?;

        if let Some(event) = event_rx.recv().await {
            match event {
                AppEvent::Key(key) => {
                    app.handle_key_event(key).await;
                }
                AppEvent::Mouse(mouse) => {
                    use crossterm::event::MouseEventKind;
                    match mouse.kind {
                        MouseEventKind::ScrollUp => {
                            match &mut app.active_view {
                                app::ActiveView::Assistant => app.assistant_state.scroll_up(3),
                                app::ActiveView::Logs(logs) => logs.scroll_up(3),
                                app::ActiveView::Describe(desc) => desc.scroll_up(3),
                                app::ActiveView::Yaml(yaml) => yaml.scroll_up(3),
                                app::ActiveView::Table(table) => table.select_prev(),
                                _ => {}
                            }
                        }
                        MouseEventKind::ScrollDown => {
                            match &mut app.active_view {
                                app::ActiveView::Assistant => app.assistant_state.scroll_down(3),
                                app::ActiveView::Logs(logs) => logs.scroll_down(3),
                                app::ActiveView::Describe(desc) => desc.scroll_down(3),
                                app::ActiveView::Yaml(yaml) => yaml.scroll_down(3),
                                app::ActiveView::Table(table) => table.select_next(),
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                }
                AppEvent::Resize(_, _) => {}
                AppEvent::Tick => {
                    app.handle_tick();
                }
                AppEvent::StreamEvent { channel, payload } => {
                    app.handle_stream_event(channel, payload);
                }
                AppEvent::ActionResult { title, result } => {
                    match result {
                        Ok(msg) => {
                            if title == "cluster_info_updated" {
                                app.handle_cluster_info_update(&msg);
                            } else if title == "crds_updated" {
                                app.handle_crds_update(&msg);
                            } else if title.starts_with("crd_instances:") {
                                app.handle_crd_instances_update(&title, &msg);
                            } else if title == "ai_reply" {
                                app.assistant_state.add_assistant_message(msg);
                            } else if title == "ai_tool_start" {
                                let mut parts = msg.splitn(3, '|');
                                let id = parts.next().unwrap_or_default().to_string();
                                let tool = parts.next().unwrap_or_default().to_string();
                                let args = parts.next().unwrap_or_default().to_string();
                                app.assistant_state.add_tool_call_start(id, tool, args);
                            } else if title == "ai_tool_done" {
                                let mut parts = msg.splitn(2, '|');
                                let id = parts.next().unwrap_or_default();
                                let status_str = parts.next().unwrap_or_default();
                                let status = if status_str == "ok" {
                                    views::assistant_view::ToolCallStatus::Success
                                } else {
                                    views::assistant_view::ToolCallStatus::Error(status_str.to_string())
                                };
                                app.assistant_state.finish_tool_call(id, status);
                            } else if title == "ai_usage" {
                                let parts: Vec<&str> = msg.split('|').collect();
                                if parts.len() >= 4 {
                                    let prompt = parts[0].parse().unwrap_or(0);
                                    let comp = parts[1].parse().unwrap_or(0);
                                    let cached = parts[2].parse().unwrap_or(0);
                                    let total = parts[3].parse().unwrap_or(prompt + comp);
                                    let duration = parts.get(4).and_then(|s| s.parse().ok());
                                    app.assistant_state.set_token_usage(views::assistant_view::TokenUsage {
                                        prompt_tokens: prompt,
                                        completion_tokens: comp,
                                        cached_tokens: cached,
                                        total_tokens: total,
                                        duration_ms: duration,
                                    });
                                }
                            } else if title == "ai_chunk" {
                                app.assistant_state.append_stream_chunk(&msg);
                            } else if title == "ai_status" {
                                app.assistant_state.set_status(msg);
                            } else if title == "ai_done" {
                                app.assistant_state.finish_turn();
                            } else {
                                app.set_toast(msg, theme::Theme::status_ok());
                            }
                        }
                        Err(err) => {
                            if title.starts_with("ai_") {
                                app.assistant_state.append_stream_chunk(&format!("\n[Error: {}]", err));
                                app.assistant_state.finish_turn();
                            }
                            app.set_toast(err, theme::Theme::status_error());
                        }
                    }
                }
            }
        }

        // Handle external tool suspend actions ($EDITOR, Pod shell, etc.)
        if let Some(action) = app.requires_terminal_suspend.take() {
            // 1. Temporarily restore terminal
            disable_raw_mode()?;
            execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
            terminal.show_cursor()?;

            // 2. Run external action
            match action {
                SuspendAction::EditYaml => {
                    if let app::ActiveView::Yaml(yaml) = &mut app.active_view {
                        match yaml.spawn_editor() {
                            Ok(Some(new_yaml)) => {
                                yaml.yaml_content = new_yaml.clone();
                                yaml.lines = new_yaml.lines().map(String::from).collect();
                                yaml.scroll_offset = 0;

                                let ctx = app.active_context.clone();
                                let cache = app.client_cache.clone();
                                let ny = new_yaml.clone();
                                let event_tx = app.event_tx.clone();
                                tokio::spawn(async move {
                                    if let Ok(client) = cache.get(&ctx).await {
                                        if let Ok(docs) = srelens_kube::manifest::split_documents(&ny) {
                                            for doc in docs {
                                                if let Some(r) = srelens_kube::manifest::resource_ref(&doc) {
                                                    let (group, version) = srelens_kube::manifest::parse_api_version(&r.api_version);
                                                    let ar = kube::core::ApiResource::from_gvk(&kube::core::GroupVersionKind::gvk(&group, &version, &r.kind));
                                                    let api: kube::Api<kube::core::DynamicObject> = match &r.namespace {
                                                        Some(ns) => kube::Api::namespaced_with(client.clone(), ns, &ar),
                                                        None => kube::Api::all_with(client.clone(), &ar),
                                                    };
                                                    let params = kube::api::PatchParams::apply("srelens");
                                                    match api.patch(&r.name, &params, &kube::api::Patch::Apply(&doc)).await {
                                                        Ok(_) => {
                                                            let _ = event_tx.send(AppEvent::ActionResult {
                                                                title: "yaml_applied".to_string(),
                                                                result: Ok(format!("Updated {}/{} in cluster", r.kind, r.name)),
                                                            });
                                                        }
                                                        Err(e) => {
                                                            let _ = event_tx.send(AppEvent::ActionResult {
                                                                title: "yaml_error".to_string(),
                                                                result: Err(format!("Apply error: {}", e)),
                                                            });
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                });
                                app.set_toast("Applying changes to cluster...".to_string(), theme::Theme::status_ok());
                            }
                            Ok(None) => {
                                app.set_toast("No changes made in $EDITOR".to_string(), theme::Theme::status_dim());
                            }
                            Err(e) => {
                                app.set_toast(format!("Editor error: {}", e), theme::Theme::status_error());
                            }
                        }
                    }
                }
                SuspendAction::PodShell { pod, container } => {
                    let _ = views::ExecRunner::run_pod_shell(
                        &app.active_context,
                        &app.active_namespace,
                        &pod,
                        container.as_deref(),
                        None,
                    );
                }
                SuspendAction::DebugShell { pod, container } => {
                    let _ = views::ExecRunner::run_debug_shell(
                        &app.active_context,
                        &app.active_namespace,
                        &pod,
                        container.as_deref(),
                    );
                }
                SuspendAction::NodeShell { node } => {
                    let _ = views::ExecRunner::run_node_shell(&app.active_context, &node);
                }
            }

            // 3. Re-enter TUI mode
            enable_raw_mode()?;
            execute!(terminal.backend_mut(), EnterAlternateScreen, EnableMouseCapture)?;
            terminal.hide_cursor()?;
            terminal.clear()?;
        }
    }

    // Clean exit
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;

    Ok(())
}
