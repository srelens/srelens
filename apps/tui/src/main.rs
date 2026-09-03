#![allow(dead_code, unused_imports)]

use std::io::{self, stdout};
use std::path::PathBuf;
use std::time::Duration;

use clap::{Parser, Subcommand, ValueEnum};
use crossterm::{
    event::{
        DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

mod agent;
mod ai_config;
mod app;
mod commands;
mod deep_link;
mod event;
mod sink;
mod theme;
mod ui;
mod views;

use app::{App, SuspendAction};
use commands::ResourceKind;
use deep_link::DeepLink;
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

    /// Deep link URL (srelens://...) or resource target (e.g. pods, nodes, pods/my-pod)
    pub target: Option<String>,

    #[command(subcommand)]
    pub command: Option<CliCommand>,
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
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture, EnableBracketedPaste)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Initialize event loop and application state
    let events = EventHandler::new(Duration::from_millis(250));

    // Parse target deep link if provided
    let parsed_target = cli.target.as_deref().and_then(|t| DeepLink::parse(t).ok());

    let target_context = cli.context.or_else(|| {
        match &parsed_target {
            Some(DeepLink::Cluster { context }) => Some(context.clone()),
            Some(DeepLink::Resource { context, .. }) if !context.is_empty() => Some(context.clone()),
            Some(DeepLink::View { context, .. }) => context.clone(),
            _ => None,
        }
    });

    let target_namespace = cli.namespace.or_else(|| {
        match &parsed_target {
            Some(DeepLink::Resource { namespace, .. }) => namespace.clone(),
            Some(DeepLink::View { namespace, .. }) => namespace.clone(),
            _ => None,
        }
    });

    let initial_resource = match &parsed_target {
        Some(DeepLink::Resource { kind, .. }) => {
            commands::resolve_command(kind).and_then(|t| match t {
                commands::CommandTarget::Resource(k) => Some(k),
                _ => None,
            })
        }
        Some(DeepLink::View { target, .. }) => {
            match target {
                commands::CommandTarget::Resource(k) => Some(k.clone()),
                _ => None,
            }
        }
        _ => None,
    };

    let mut app = App::new(
        target_context,
        target_namespace,
        cli.all_namespaces,
        initial_resource,
        kubeconfig_paths,
        events.tx.clone(),
    )
    .await
    .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    if let Some(link) = parsed_target {
        let _ = app.navigate_deep_link(&link).await;
    }

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
                            // Scrolling moves content under a screen-anchored
                            // drag selection; drop the stale highlight.
                            app.screen_selection = None;
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
                            app.screen_selection = None;
                            match &mut app.active_view {
                                app::ActiveView::Assistant => app.assistant_state.scroll_down(3),
                                app::ActiveView::Logs(logs) => logs.scroll_down(3),
                                app::ActiveView::Describe(desc) => desc.scroll_down(3),
                                app::ActiveView::Yaml(yaml) => yaml.scroll_down(3),
                                app::ActiveView::Table(table) => table.select_next(),
                                _ => {}
                            }
                        }
                        _ => {
                            app.handle_mouse(mouse).await;
                        }
                    }
                }
                AppEvent::Paste(text) => {
                    app.handle_paste(text);
                }
                AppEvent::Resize(_, _) => {}
                AppEvent::Tick => {
                    app.handle_tick();
                }
                AppEvent::StreamEvent { channel, payload } => {
                    app.handle_stream_event(channel, payload);
                }
                AppEvent::ActionResult { title, result } => {
                    let (action, event_ctx) = if let Some((act, c)) = title.split_once(':') {
                        if act.starts_with("ai_") {
                            (act, Some(c))
                        } else {
                            (title.as_str(), None)
                        }
                    } else {
                        (title.as_str(), None)
                    };

                    match result {
                        Ok(msg) => {
                            if title == "cluster_info_updated" {
                                app.handle_cluster_info_update(&msg);
                            } else if title == "cluster_overview_updated" {
                                app.handle_cluster_overview_update(&msg);
                            } else if title == "crds_updated" {
                                app.handle_crds_update(&msg);
                            } else if title.starts_with("crd_instances:") {
                                app.handle_crd_instances_update(&title, &msg);
                            } else if action.starts_with("ai_") {
                                let target_state = match event_ctx {
                                    Some(ctx) if ctx == app.active_context => &mut app.assistant_state,
                                    Some(ctx) => app.assistant_states.entry(ctx.to_string()).or_insert_with(|| views::assistant_view::AssistantViewState::for_context(ctx)),
                                    None => &mut app.assistant_state,
                                };

                                if action == "ai_reply" {
                                    target_state.add_assistant_message(msg);
                                } else if action == "ai_tool_start" {
                                    let mut parts = msg.splitn(3, '|');
                                    let id = parts.next().unwrap_or_default().to_string();
                                    let tool = parts.next().unwrap_or_default().to_string();
                                    let args = parts.next().unwrap_or_default().to_string();
                                    target_state.add_tool_call_start(id, tool, args);
                                } else if action == "ai_tool_done" {
                                    let mut parts = msg.splitn(2, '|');
                                    let id = parts.next().unwrap_or_default();
                                    let status_str = parts.next().unwrap_or_default();
                                    let status = if status_str == "ok" {
                                        views::assistant_view::ToolCallStatus::Success
                                    } else {
                                        views::assistant_view::ToolCallStatus::Error(status_str.to_string())
                                    };
                                    target_state.finish_tool_call(id, status);
                                } else if action == "ai_usage" {
                                    let parts: Vec<&str> = msg.split('|').collect();
                                    if parts.len() >= 4 {
                                        let prompt = parts[0].parse().unwrap_or(0);
                                        let comp = parts[1].parse().unwrap_or(0);
                                        let cached = parts[2].parse().unwrap_or(0);
                                        let total = parts[3].parse().unwrap_or(prompt + comp);
                                        let duration = parts.get(4).and_then(|s| s.parse().ok());
                                        target_state.set_token_usage(views::assistant_view::TokenUsage {
                                            prompt_tokens: prompt,
                                            completion_tokens: comp,
                                            cached_tokens: cached,
                                            total_tokens: total,
                                            duration_ms: duration,
                                        });
                                    }
                                } else if action == "ai_chunk" {
                                    target_state.append_stream_chunk(&msg);
                                } else if action == "ai_status" {
                                    target_state.set_status(msg);
                                } else if action == "ai_done" {
                                    target_state.finish_turn();
                                }
                            } else if title == "pod_metrics_updated" {
                                app.handle_pod_metrics_update(&msg);
                            } else {
                                app.set_toast(msg, theme::Theme::status_ok());
                            }
                        }
                        Err(err) => {
                            if title == "pod_metrics_updated" {
                                // Best-effort: ignore if metrics-server is unavailable
                            } else if action.starts_with("ai_") {
                                let target_state = match event_ctx {
                                    Some(ctx) if ctx == app.active_context => &mut app.assistant_state,
                                    Some(ctx) => app.assistant_states.entry(ctx.to_string()).or_insert_with(|| views::assistant_view::AssistantViewState::for_context(ctx)),
                                    None => &mut app.assistant_state,
                                };
                                target_state.append_stream_chunk(&format!("\n[Error: {}]", err));
                                target_state.finish_turn();
                                app.set_toast(err, theme::Theme::status_error());
                            } else {
                                app.set_toast(err, theme::Theme::status_error());
                            }
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
            execute!(terminal.backend_mut(), EnterAlternateScreen, EnableMouseCapture, EnableBracketedPaste)?;
            terminal.hide_cursor()?;
            terminal.clear()?;
        }
    }

    // Clean exit
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture, DisableBracketedPaste)?;
    terminal.show_cursor()?;

    Ok(())
}
