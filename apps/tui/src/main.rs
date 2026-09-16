#![allow(dead_code, unused_imports)]

use std::io::{self, stdout, Write};
use std::path::PathBuf;
use std::time::Duration;

use clap::{Parser, Subcommand, ValueEnum};
use crossterm::{
    cursor::{MoveTo, Show},
    event::{
        DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    },
    execute,
    style::ResetColor,
    terminal::{
        disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

mod agent;
mod ai_config;
mod ai_skills;
mod app;
mod commands;
mod deep_link;
mod event;
mod sink;
mod theme;
mod tui_config;
mod ui;
mod views;

use srelens_tui::self_update;

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

    #[command(subcommand)]
    pub command: Option<CliCommand>,

    /// Deep link URL (srelens://...) or resource target (e.g. pods, nodes, pods/my-pod)
    pub target: Option<String>,
}

#[derive(Subcommand, Debug)]
pub enum CliCommand {
    /// Print cluster overview & reachability information
    Info,
    /// Check toolbox diagnostics (kubectl, helm, krew)
    Toolbox,
    /// Print version information
    Version,
    /// Update srelens-tui to the latest release
    Update {
        /// Report what an update would do, without changing anything
        #[arg(long)]
        check: bool,
        /// Which releases to consider: stable, or the rolling dev
        /// pre-releases. Defaults to the channel this binary came from.
        #[arg(long, value_parser = ["stable", "dev"])]
        channel: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Install rustls crypto provider immediately so any reqwest client (e.g. update check)
    // has a provider available without panicking.
    let _ = rustls::crypto::ring::default_provider().install_default();

    // BEFORE parsing: clap exits during `--version` and `--help`, which is
    // exactly what someone whose binary vanished is likely to type first.
    //
    // An update interrupted between its two renames leaves this binary at
    // `.srelens-tui.exe.srelens-update.old` with nothing at the real name — and no
    // run `update` to repair it, since there is nothing left to run. If
    // this process IS that displaced file, put it back. A no-op anywhere
    // else, and off Windows entirely.
    if let Ok(exe) = std::env::current_exe() {
        match srelens_tui::self_update::recover_interrupted_update(&exe) {
            Ok(Some(restored)) => eprintln!(
                "srelens-tui: an interrupted update left this binary beside its own name; restored it to {}",
                restored.display()
            ),
            Ok(None) => {}
            // Needed and failed, which is not the same as nothing to do.
            // The command path is still missing, so say so rather than
            // letting someone rediscover it later.
            Err(why) => eprintln!(
                "srelens-tui: an interrupted update left this binary at {}, and it could not be moved back: {why}. Rename it yourself to restore the command.",
                exe.display()
            ),
        }
    }

    let cli = Cli::parse();

    // Resolved BEFORE the subcommand match, because those arms return early.
    // `info` used to call `all_kubeconfig_paths()` itself and so ignored
    // `--kubeconfig` entirely: a user pointing at a file outside the default
    // locations was shown the contexts of the DEFAULT kubeconfig and told they
    // were theirs.
    let kubeconfig_paths = match &cli.kubeconfig {
        Some(path) => vec![path.clone()],
        None => srelens_registry::all_kubeconfig_paths(),
    };

    // Handle non-interactive CLI subcommands if requested
    if let Some(cmd) = cli.command {
        match cmd {
            CliCommand::Version => {
                println!("srelens-tui v{}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            CliCommand::Info => {
                println!("SRElens Kubernetes TUI (srelens-tui)");
                let contexts = srelens_kube::context_resolve::resolve_contexts(&kubeconfig_paths);
                println!("Found {} contexts across kubeconfigs:", contexts.len());
                for ctx in contexts {
                    let mark = if ctx.is_current { "* " } else { "  " };
                    println!("{}{} -> cluster: {}, server: {}", mark, ctx.display_name, ctx.cluster, ctx.server);
                }
                return Ok(());
            }
            CliCommand::Update { check, channel } => {
                return run_update(check, channel);
            }
            CliCommand::Toolbox => {
                let state = views::ToolboxViewState::new();
                println!("SRElens Toolbox Status:");
                for tool in &state.tools {
                    let status = if tool.installed {
                        format!("available ({})", tool.version.as_deref().unwrap_or("unknown version"))
                    } else if tool.required {
                        "missing (required)".to_string()
                    } else {
                        "not installed (optional)".to_string()
                    };
                    let path_info = tool.path.as_deref().map(|p| format!(" [{}]", p)).unwrap_or_default();
                    println!("  {:<8} : {}{}", tool.name, status, path_info);
                }
                return Ok(());
            }
        }
    }

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
    let mut events = EventHandler::new(Duration::from_millis(250));

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

    if parsed_target.is_none() && app.tui_config.show_feature_banner {
        app.modal = Some(ui::Modal::FeatureBanner {
            show_on_startup: true,
            update_available: app.tui_config.update_available.clone(),
        });
    }

    if app.tui_config.check_updates {
        app.spawn_update_check();
    }

    if let Some(link) = parsed_target {
        let _ = app.navigate_deep_link(&link).await;
    }

    // Main TUI Event Loop
    while app.is_running {
        terminal.draw(|f| app.render(f))?;

        if let Some(first_event) = events.recv().await {
            let mut current_event = Some(first_event);
            let mut batch_count = 0;

            while let Some(event) = current_event {
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
                                app::ActiveView::Helm(helm) => helm.select_prev(),
                                app::ActiveView::HelmDetail(detail) => {
                                    if detail.active_tab == views::HelmDetailTab::Revisions {
                                        detail.select_prev_revision();
                                    } else {
                                        detail.scroll_up(3);
                                    }
                                }
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
                                app::ActiveView::Helm(helm) => helm.select_next(),
                                app::ActiveView::HelmDetail(detail) => {
                                    if detail.active_tab == views::HelmDetailTab::Revisions {
                                        detail.select_next_revision();
                                    } else {
                                        detail.scroll_down(3);
                                    }
                                }
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
                            } else if title == "update_available" {
                                app.handle_update_available(&msg);
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
                            } else if title == "node_metrics_updated" {
                                app.handle_node_metrics_update(&msg);
                            } else if title.starts_with("yaml_applied") {
                                app.handle_yaml_applied(&title, &msg);
                            } else {
                                if title.starts_with("helm_rollback:") || title.starts_with("helm_uninstall:") {
                                    app.refresh_helm_releases();
                                    if let app::ActiveView::HelmDetail(detail) = &app.active_view {
                                        let name = detail.release_name.clone();
                                        let ns = detail.namespace.clone();
                                        app.reload_helm_detail(&name, &ns);
                                    }
                                }
                                app.set_toast(msg, theme::Theme::status_ok());
                            }
                        }
                        Err(err) => {
                            if title == "cluster_info_updated" || title == "cluster_info_failed" {
                                app.handle_cluster_info_failure(&err);
                            } else if title == "crds_updated" || title == "crds_failed" {
                                app.handle_crds_failed(&err);
                            } else if title.starts_with("crd_instances:") || title.starts_with("crd_instances_failed:") {
                                app.handle_crd_instances_failed(&title, &err);
                            } else if title == "cluster_overview_updated" {
                                // Best-effort: ignore if cluster overview cannot connect
                            } else if title == "pod_metrics_updated" || title == "node_metrics_updated" {
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
                            } else if title.starts_with("yaml_error") {
                                app.handle_yaml_error(&err);
                            } else {
                                app.set_toast(err, theme::Theme::status_error());
                            }
                        }
                    }
                }
                AppEvent::LineageResult { kind, name, result } => {
                    app.handle_lineage_result(&kind, &name, result);
                }
                AppEvent::NodeInspectorResult { node_name, result } => {
                    app.handle_node_inspector_result(&node_name, result);
                }
                AppEvent::TopologyResult { context, namespaces, result } => {
                    app.handle_topology_result(&context, namespaces, result);
                }
                AppEvent::GpuInfoResult { context, result } => {
                    app.handle_gpu_info_result(&context, result);
                }
                AppEvent::HelmReleasesResult { context, namespace, result } => {
                    app.handle_helm_releases_result(&context, &namespace, result);
                }
                AppEvent::HelmDetailResult { context, namespace, name, revision, result } => {
                    app.handle_helm_detail_result(&context, &namespace, &name, revision, result);
                }
                AppEvent::ArgoApplicationsResult { context, is_remote_hub, hub_context, result } => {
                    app.handle_argo_applications_result(&context, is_remote_hub, hub_context, result);
                }
                AppEvent::ArgoDetailResult { context, namespace, name, result } => {
                    app.handle_argo_detail_result(&context, &namespace, &name, result);
                }
                AppEvent::ArgoActionResult { action, result } => {
                    app.handle_argo_action_result(&action, result);
                }
                AppEvent::BgpResult { context, result } => {
                    app.handle_bgp_result(&context, result);
                }
            }

            if !app.is_running {
                break;
            }

            batch_count += 1;
            if batch_count >= 256 {
                break;
            }

            current_event = events.try_recv().ok();
        }
    }

        // Handle external tool suspend actions ($EDITOR, Pod shell, etc.)
        if let Some(action) = app.requires_terminal_suspend.take() {
            // 1. Pause background event listener and wait for it to release stdin
            events.pause();
            tokio::time::sleep(Duration::from_millis(20)).await;
            while events.try_recv().is_ok() {}

            // Temporarily restore terminal for external interactive session on primary screen
            disable_raw_mode()?;
            execute!(
                terminal.backend_mut(),
                LeaveAlternateScreen,
                DisableMouseCapture,
                DisableBracketedPaste
            )?;
            execute!(std::io::stdout(), Show)?;
            let _ = terminal.flush();
            let _ = std::io::stdout().flush();

            // 2. Run external action
            match action {
                SuspendAction::EditYaml => {
                    let (editor_res, res_kind, res_ns) = if let app::ActiveView::Yaml(yaml) = &mut app.active_view {
                        (yaml.spawn_editor(), yaml.resource_kind.clone(), yaml.namespace.clone())
                    } else {
                        (Ok(None), String::new(), None)
                    };

                    match editor_res {
                        Ok(Some(new_yaml)) => {
                            let ctx = app.active_context.clone();
                            let active_ns = app.active_namespace.clone();
                            let fallback_ns = res_ns.as_deref().or_else(|| {
                                if active_ns.is_empty() || active_ns == "all" {
                                    None
                                } else {
                                    Some(active_ns.as_str())
                                }
                            });

                            let client = match app.client_cache.get(&ctx).await {
                                Ok(c) => Some(c),
                                Err(e) => {
                                    app.handle_yaml_error(&format!("Cluster connect error: {}", e));
                                    None
                                }
                            };

                            if let Some(client) = client {
                                match srelens_kube::manifest::split_documents(&new_yaml) {
                                    Ok(docs) if !docs.is_empty() => {
                                        let results = srelens_kube::manifest::apply_documents(&client, docs, fallback_ns, true).await;
                                        let applied_docs: Vec<_> = results.iter().filter(|d| d.applied).collect();
                                        let failed_docs: Vec<_> = results.iter().filter(|d| !d.applied).collect();

                                        // Invalidate cache for every document that was applied
                                        for doc in &applied_docs {
                                            let doc_ns = res_ns.as_deref().unwrap_or("");
                                            app.invalidate_resource_cache_for(&doc.kind, doc_ns);
                                        }

                                        if failed_docs.is_empty() && !applied_docs.is_empty() {
                                            let updated_names: Vec<String> = applied_docs.iter().map(|d| format!("{}/{}", d.kind, d.name)).collect();
                                            let msg = format!("Updated {} in cluster", updated_names.join(", "));
                                            if let app::ActiveView::Yaml(yaml) = &mut app.active_view {
                                                yaml.commit_content(new_yaml);
                                            }
                                            app.handle_yaml_applied(
                                                &format!("yaml_applied:{}:{}", res_kind, res_ns.as_deref().unwrap_or("")),
                                                &msg,
                                            );
                                        } else if !applied_docs.is_empty() {
                                            let applied_names: Vec<String> = applied_docs.iter().map(|d| format!("{}/{}", d.kind, d.name)).collect();
                                            let error_msgs: Vec<String> = failed_docs
                                                .iter()
                                                .map(|d| {
                                                    let err = d.error.as_deref().unwrap_or("unknown apply error");
                                                    if !d.kind.is_empty() && !d.name.is_empty() {
                                                        format!("{}/{}: {}", d.kind, d.name, err)
                                                    } else {
                                                        err.to_string()
                                                    }
                                                })
                                                .collect();
                                            if let app::ActiveView::Yaml(yaml) = &mut app.active_view {
                                                yaml.commit_content(new_yaml);
                                            }
                                            app.handle_yaml_partial_applied(&applied_names.join(", "), &error_msgs.join("; "));
                                        } else {
                                            let errors: Vec<String> = failed_docs
                                                .iter()
                                                .map(|d| {
                                                    let err = d.error.as_deref().unwrap_or("unknown apply error");
                                                    if !d.kind.is_empty() && !d.name.is_empty() {
                                                        format!("{}/{}: {}", d.kind, d.name, err)
                                                    } else {
                                                        err.to_string()
                                                    }
                                                })
                                                .collect();
                                            app.handle_yaml_error(&errors.join("; "));
                                        }
                                    }
                                    Ok(_) => {
                                        app.handle_yaml_error("No YAML documents found in file");
                                    }
                                    Err(e) => {
                                        app.handle_yaml_error(&format!("YAML parse error: {}", e));
                                    }
                                }
                            }
                        }
                        Ok(None) => {
                            app.set_toast("No changes made in $EDITOR".to_string(), theme::Theme::status_dim());
                        }
                        Err(e) => {
                            app.set_toast(format!("Editor error: {}", e), theme::Theme::status_error());
                        }
                    }
                }
                SuspendAction::PodShell { pod, namespace, container } => {
                    let target_ns = namespace.as_deref().unwrap_or(app.active_namespace.as_str());
                    if let Err(e) = views::ExecRunner::run_pod_shell(
                        &app.active_context,
                        target_ns,
                        &pod,
                        container.as_deref(),
                        None,
                    ) {
                        app.set_toast(e, theme::Theme::status_error());
                    }
                }
                SuspendAction::DebugShell { pod, namespace, container } => {
                    let target_ns = namespace.as_deref().unwrap_or(app.active_namespace.as_str());
                    if let Err(e) = views::ExecRunner::run_debug_shell(
                        &app.active_context,
                        target_ns,
                        &pod,
                        container.as_deref(),
                    ) {
                        app.set_toast(e, theme::Theme::status_error());
                    }
                }
                SuspendAction::NodeShell { node } => {
                    if let Err(e) = views::ExecRunner::run_node_shell(&app.active_context, &node) {
                        app.set_toast(e, theme::Theme::status_error());
                    }
                }
                SuspendAction::NodeSsh { destination } => {
                    if let Err(e) = views::ExecRunner::run_node_ssh(&destination) {
                        app.set_toast(e, theme::Theme::status_error());
                    }
                }
            }

            // 3. Flush any leftover leaked sequences from child process (e.g. vim OSC queries)
            #[cfg(unix)]
            unsafe {
                libc::tcflush(libc::STDIN_FILENO, libc::TCIFLUSH);
            }

            // 4. Re-enter TUI mode
            enable_raw_mode()?;
            execute!(
                terminal.backend_mut(),
                EnterAlternateScreen,
                ResetColor,
                EnableMouseCapture,
                EnableBracketedPaste
            )?;
            terminal.hide_cursor()?;
            terminal.clear()?;
            let _ = terminal.flush();

            // Drain any pending crossterm events before resuming the event handler
            while crossterm::event::poll(Duration::from_millis(15)).unwrap_or(false) {
                let _ = crossterm::event::read();
            }
            while events.try_recv().is_ok() {}

            // 5. Resume background event listener
            events.resume();
        }
    }

    // Clean exit
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture, DisableBracketedPaste)?;
    terminal.show_cursor()?;

    Ok(())
}

/// `srelens-tui update` — see `self_update` for why each step is where it is.
///
/// Written as a plain synchronous function: it runs before the terminal is
/// touched and exits, so there is nothing to interleave with.
fn run_update(
    check_only: bool,
    channel: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    // `#[tokio::main]` means this function is called ON a runtime worker
    // thread. `reqwest::blocking` drives its own runtime on a private thread
    // and parks the caller on a channel until it answers; doing that from a
    // worker ties up a thread the runtime owns, and reqwest documents using it
    // from inside a runtime as unsupported. It does not in fact panic here —
    // the command was run end to end against the real API before this was
    // written — but there is no reason to depend on that. Nothing in the update
    // path is async, so it runs on a thread of its own and the question does
    // not arise.
    // `String` rather than `Box<dyn Error>`: the boxed trait object is not
    // `Send`, so it cannot come back across a thread boundary.
    std::thread::spawn(move || update_off_the_runtime(check_only, channel))
        .join()
        .map_err(|_| "the update thread panicked")??;
    Ok(())
}

fn update_off_the_runtime(check_only: bool, channel: Option<String>) -> Result<(), String> {
    use srelens_tui::self_update::{self, Channel, Check, UpdateError};

    // reqwest is built with `rustls-no-provider`, which does NOT pick a
    // provider on its own: building a client without one panics inside
    // reqwest's runtime thread, which surfaces as "event loop thread panicked"
    // and tells the user nothing. Elsewhere in the app a kube client is built
    // first and leaves a provider installed as a side effect; `update` runs
    // before anything touches a cluster, so it has to say so itself.
    //
    // `ring` to match kube-rs. Installing a SECOND provider would be worse than
    // installing none: rustls refuses to choose between two and panics on the
    // first handshake. `Err` here means one is already installed, which is the
    // state we want.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let current = env!("CARGO_PKG_VERSION");
    // Default to the channel this binary came from, so `update` keeps someone
    // where they are instead of quietly moving a dev user onto stable.
    let requested = channel.is_some();
    let channel = match channel {
        Some(name) => Channel::parse(&name)
            .ok_or_else(|| format!("unknown channel {name:?} — use \"stable\" or \"dev\""))?,
        None => Channel::of_version(current),
    };
    let exe = std::env::current_exe()
        .map_err(|e| format!("could not find this binary on disk: {e}"))?;
    // After a recovery this process is still reported as running from the
    // displaced name; updating that path would leave the real one alone.
    let exe = self_update::installed_path(&exe);

    let fetch = |url: &str| -> Result<Vec<u8>, UpdateError> {
        let client = reqwest::blocking::Client::builder()
            .user_agent(concat!("srelens-tui/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| UpdateError::Download(e.to_string()))?;
        let response = client
            .get(url)
            .send()
            .map_err(|e| UpdateError::Download(e.to_string()))?;
        if !response.status().is_success() {
            return Err(UpdateError::Download(format!(
                "{} for {url}",
                response.status()
            )));
        }
        response
            .bytes()
            .map(|b| b.to_vec())
            .map_err(|e| UpdateError::Download(e.to_string()))
    };

    // Errors are printed here rather than returned. `main` reports a
    // `Box<dyn Error>` with its DEBUG formatting, so returning one turns a
    // written-out sentence into `Download("404 Not Found for https://…")` —
    // the quotes and the variant name are noise, and the message is the part
    // that tells the user what to do.
    let plan = match self_update::plan(current, channel, requested, exe.clone(), &fetch) {
        Ok(Check::Available(plan)) => *plan,
        Ok(Check::UpToDate { channel, .. }) => {
            println!(
                "srelens-tui {current} is the latest {} release.",
                channel.as_str()
            );
            return Ok(());
        }
        Ok(Check::AheadOfChannel { channel, latest }) => {
            // Only this channel was consulted, so that is all that can be
            // claimed. Naming the other one turns a dead end into a next step.
            print!(
                "srelens-tui {current} is ahead of the latest {} release ({latest}); there is no {} update to install.",
                channel.as_str(),
                channel.as_str()
            );
            match channel {
                Channel::Stable => println!(" Try `srelens-tui update --channel dev`."),
                Channel::Dev => println!(),
            }
            return Ok(());
        }
        Err(error) => fail(error),
    };

    if check_only {
        // The hint has to carry the channel when it is not the default one, or
        // copying the line installs from a channel the user did not ask about —
        // which for a stable binary checking dev is the whole point of asking.
        let flag = if channel == Channel::of_version(current) {
            String::new()
        } else {
            format!(" --channel {}", channel.as_str())
        };
        println!(
            "srelens-tui {} is available (you have {}).\n  {}\nRun `srelens-tui update{}` to install it.",
            plan.latest, plan.current, plan.archive_url, flag
        );
        return Ok(());
    }

    let verb = if self_update::is_newer(&plan.current, &plan.latest) {
        "Updating"
    } else {
        "Switching"
    };
    println!(
        "{verb} srelens-tui {} -> {} ({} channel)…",
        plan.current,
        plan.latest,
        channel.as_str()
    );
    if let Err(error) = self_update::apply(&plan, &fetch) {
        fail(error);
    }
    println!("Installed {} to {}", plan.latest, exe.display());
    Ok(())
}

/// Report an update failure the way a command-line tool should: the sentence
/// the error carries, on stderr, and a non-zero status so a script wrapping
/// this can tell.
fn fail(error: srelens_tui::self_update::UpdateError) -> ! {
    eprintln!("srelens-tui: {error}");
    std::process::exit(1);
}
