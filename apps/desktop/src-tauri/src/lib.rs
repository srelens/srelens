mod app_log;
mod deep_link;
mod appimage;
mod assistant;
mod assistant_history;
mod assistant_prompts;
mod assistant_skills;
mod bridge;
pub mod capabilities;
mod cluster_oidc;
mod cluster_oidc_cmd;
mod llm_agent;
mod llm_config;
mod exec;
mod external;
mod files;
mod forward;
mod helm;
mod logs;
mod mcp;
mod mcp_confirm;
pub mod mcp_watch;
mod overview_snapshot;
mod settings;
mod sink;
mod terminal;
pub mod vault;
mod vault_biometric;
mod vault_password;
mod toolbox;
mod updater;
mod watch;

use app_log::{app_log_path, read_app_log, reveal_app_log};
use bridge::{invoke_capability, AppRegistry};
use exec::{exec_close, exec_input, exec_resize, start_pod_exec};
use external::open_external;
use files::{pick_kubeconfig_files, save_pasted_kubeconfig, save_text_file};
use forward::{list_forwards, start_port_forward, stop_port_forward};
use helm::{helm_op_close, start_helm_op};
use logs::{start_log_stream, stop_log_stream};
use mcp::{
    install_srelens_cli, mcp_audit_tail, mcp_confirm_respond, mcp_http_start, mcp_http_status,
    mcp_http_stop, mcp_prompt_issues, mcp_token_get, mcp_token_revoke, mcp_token_rotate,
    mcp_token_storage, srelens_cli_status, McpAuditPath, McpHttpManager, McpPromptsDir,
};
use settings::{get_request_timeout, set_request_timeout};
use srelens_kube::client_cache::ClientCache;
use srelens_streams::exec::ExecManager;
use srelens_streams::forward::ForwardManager;
use srelens_streams::helm::HelmManager;
use srelens_streams::logs::LogStreamManager;
use srelens_streams::terminal::TerminalManager;
use srelens_streams::watch::WatchManager;
use tauri::Manager;
use terminal::{start_terminal, terminal_close, terminal_input, terminal_resize};
use toolbox::start_tool_install;
use updater::{update_check, update_install};
use watch::{start_resource_watch, stop_watch};

pub use appimage::gio_module_dir_for_appimage;
pub use capabilities::{
    build_registry, build_registry_with_paths, build_registry_with_paths_and_settings,
    default_settings_path,
};

/// Size the main window to a comfortable default, clamped to the screen it
/// opens on: on a large display it stays at the preferred ~16" size (centered),
/// on a smaller display it shrinks to fit the available work area. A margin
/// keeps it clear of the menu bar / taskbar / dock.
#[cfg(desktop)]
fn size_main_window(app: &tauri::App) {
    use tauri::{LogicalSize, Manager};

    // Preferred size — the "16-inch" window shown on big screens.
    const PREF_W: f64 = 1440.0;
    const PREF_H: f64 = 900.0;
    // Leave room for OS chrome so the window never sits edge-to-edge.
    const MARGIN: f64 = 80.0;

    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let Ok(Some(monitor)) = window.current_monitor() else {
        return;
    };
    let scale = monitor.scale_factor();
    let avail_w = monitor.size().width as f64 / scale - MARGIN;
    let avail_h = monitor.size().height as f64 / scale - MARGIN;

    let width = PREF_W.min(avail_w).max(640.0);
    let height = PREF_H.min(avail_h).max(480.0);
    let _ = window.set_size(LogicalSize::new(width, height));
    let _ = window.center();
}

/// Menu id for the custom "Close Tab" item. macOS routes Cmd+W to this instead
/// of the predefined "Close Window", so the frontend can close the active tab.
#[cfg(target_os = "macos")]
const CLOSE_TAB_MENU_ID: &str = "close-active-tab";

/// Install a custom macOS application menu.
///
/// The default Tauri menu binds Cmd+W to the predefined "Close Window" item,
/// which closes the whole window natively before the webview ever sees the
/// keystroke. We rebuild the standard menu (App / Edit / View / Window) so
/// nothing users expect is lost, but swap the Window submenu's close entry
/// for a custom item that keeps the Cmd+W accelerator and emits a
/// `close-active-tab` event. The frontend then closes the active tab, only
/// falling back to closing the window when no tabs remain.
#[cfg(target_os = "macos")]
fn install_macos_menu(app: &tauri::App) -> tauri::Result<()> {
    use tauri::menu::{AboutMetadata, MenuBuilder, MenuItemBuilder, SubmenuBuilder};
    use tauri::Emitter;

    let handle = app.handle();

    let about = AboutMetadata {
        name: Some("srelens".into()),
        version: Some(env!("CARGO_PKG_VERSION").into()),
        ..Default::default()
    };

    let app_menu = SubmenuBuilder::new(handle, "srelens")
        .about(Some(about))
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .quit()
        .build()?;

    let edit_menu = SubmenuBuilder::new(handle, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;

    let view_menu = SubmenuBuilder::new(handle, "View").fullscreen().build()?;

    // Custom Close item: keeps the familiar Cmd+W accelerator but routes to our
    // menu-event handler instead of the native "Close Window".
    let close_tab = MenuItemBuilder::new("Close")
        .id(CLOSE_TAB_MENU_ID)
        .accelerator("CmdOrCtrl+W")
        .build(handle)?;

    let window_menu = SubmenuBuilder::new(handle, "Window")
        .minimize()
        .item(&close_tab)
        .build()?;

    let menu = MenuBuilder::new(handle)
        .item(&app_menu)
        .item(&edit_menu)
        .item(&view_menu)
        .item(&window_menu)
        .build()?;

    app.set_menu(menu)?;

    app.on_menu_event(move |app, event| {
        if event.id().as_ref() == CLOSE_TAB_MENU_ID {
            let _ = app.emit("close-active-tab", ());
        }
    });

    Ok(())
}

async fn watch_kubeconfig_files(app_handle: tauri::AppHandle, cache: std::sync::Arc<ClientCache>) {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::time::SystemTime;
    use tauri::Emitter;

    // The cache's paths plus everything in the app's own kubeconfig folder.
    // Polling the cache alone only ever compares files it already knows about,
    // so a config pasted in — or dropped there by hand — was invisible until a
    // restart (#256). Enumerating the folder is what makes creation and
    // deletion observable at all.
    async fn watched_paths(cache: &ClientCache) -> Vec<PathBuf> {
        let mut paths = cache.paths().await;
        for managed in srelens_registry::managed_kubeconfig_files() {
            if !paths.contains(&managed) {
                paths.push(managed);
            }
        }
        paths
    }


    let mut last_modified: HashMap<PathBuf, Option<SystemTime>> = HashMap::new();

    // Initialize the map with current files
    for path in watched_paths(&cache).await {
        let modified = tokio::fs::metadata(&path)
            .await
            .and_then(|m| m.modified())
            .ok();
        last_modified.insert(path, modified);
    }

    loop {
        tokio::time::sleep(tokio::time::Duration::from_millis(1500)).await;

        let current_paths = watched_paths(&cache).await;
        let mut changed = false;

        let mut next_modified = HashMap::new();
        for path in current_paths {
            let current_mod = tokio::fs::metadata(&path)
                .await
                .and_then(|m| m.modified())
                .ok();

            if let Some(prev) = last_modified.get(&path) {
                if *prev != current_mod {
                    changed = true;
                }
            } else {
                changed = true;
            }
            next_modified.insert(path.clone(), current_mod);
        }

        // Check if any path was removed
        for path in last_modified.keys() {
            if !next_modified.contains_key(path) {
                changed = true;
            }
        }

        last_modified = next_modified;

        if changed {
            cache.clear().await;
            // A cluster added/removed at runtime (Add-cluster form, edited
            // kubeconfig) may change which contexts are OIDC-managed; rebuild
            // the registry and reinstall the resolver so it's recognized
            // without an app restart.
            if let Some(oidc) =
                app_handle.try_state::<std::sync::Arc<crate::cluster_oidc::DesktopClusterOidc>>()
            {
                oidc.rebuild(&cache).await;
            }
            let _ = app_handle.emit("kubeconfig-changed", ());
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // The SRELENS_TIMEOUT_SECS override is applied in `main()` before dispatch,
    // so it's live here; the Settings UI can adjust it further at runtime.

    // One shared client cache: request/response capabilities AND live watches
    // reuse the same authenticated kube-rs clients.
    let cache = ClientCache::new_many(capabilities::all_kubeconfig_paths());
    let registry = capabilities::build_registry_with(cache.clone());

    // single-instance is registered BEFORE every other plugin, as the plugin
    // requires: it has to claim the lock and hand a second launch's argv over
    // before anything else initializes. Its `deep-link` feature forwards those
    // arguments to the deep-link plugin first, so an `srelens://` link on
    // Windows/Linux reaches the running app rather than starting a rival one.
    #[cfg(desktop)]
    let builder = tauri::Builder::default().plugin(tauri_plugin_single_instance::init(
        |app, _argv, _cwd| {
            deep_link::focus_main_window(app);
        },
    ));
    #[cfg(not(desktop))]
    let builder = tauri::Builder::default();

    let builder = builder
        .manage(deep_link::PendingDeepLink::default())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_biometry::init())
        .plugin(tauri_plugin_opener::init());
    #[cfg(desktop)]
    let builder = builder
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_deep_link::init())
        // Size, position and maximized state, restored at window creation.
        .plugin(tauri_plugin_window_state::Builder::default().build());

    let watcher_cache = cache.clone();
    let oidc_cache = cache.clone();
    builder
        .setup(move |app| {
            // Application logging: always write a rotating file to the OS log
            // directory so the Settings "Application logs" view (and post-hoc
            // debugging of a shipped build) has something to read; mirror to
            // stdout in dev for convenience.
            let mut log_targets = vec![tauri_plugin_log::Target::new(
                tauri_plugin_log::TargetKind::LogDir {
                    file_name: Some("srelens".into()),
                },
            )];
            if cfg!(debug_assertions) {
                log_targets.push(tauri_plugin_log::Target::new(
                    tauri_plugin_log::TargetKind::Stdout,
                ));
            }
            app.handle().plugin(
                tauri_plugin_log::Builder::default()
                    .level(log::LevelFilter::Info)
                    // Keep noisy transport crates out of the file so it stays
                    // readable for triage.
                    .level_for("hyper", log::LevelFilter::Warn)
                    .level_for("rustls", log::LevelFilter::Warn)
                    .max_file_size(5_000_000)
                    .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepOne)
                    .targets(log_targets)
                    .build(),
            )?;
            log::info!("srelens {} starting", env!("CARGO_PKG_VERSION"));
            // Only impose the default geometry on a first launch: after that
            // the window-state plugin has already restored the user's own
            // size and position, and re-centering would undo it every time.
            #[cfg(desktop)]
            if !deep_link::has_saved_window_state(app) {
                size_main_window(app);
            }
            #[cfg(desktop)]
            deep_link::register_deep_links(app);
            #[cfg(target_os = "macos")]
            install_macos_menu(app)?;

            // Best-effort: remove stale helm temp files (kubeconfig/values) left
            // behind by a crashed/killed prior run so they don't accumulate.
            srelens_kube::helm_cli::sweep_stale_temp_files();

            // Use Tauri's managed async runtime — `tokio::spawn` here panics
            // ("no reactor running") because `setup` runs before/outside a Tokio
            // runtime context.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                watch_kubeconfig_files(handle, watcher_cache).await;
            });

            // Managed cluster OIDC (desktop): install the resolver so OIDC
            // contexts sign in through srelens's own browser flow instead of
            // an exec plugin. Non-OIDC contexts are unaffected — the resolver
            // returns `AuthMode::Default` for them, falling back to kube-rs's
            // native kubeconfig auth.
            // Best-effort: a corrupt token db or unwritable config dir logs and
            // is skipped (OIDC contexts then fall back to native exec) rather
            // than aborting app startup.
            let oidc_env = app
                .path()
                .app_config_dir()
                .map_err(|e| e.to_string())
                .map(|dir| dir.join("cluster-oidc"))
                .and_then(|config_dir| {
                    let paths = capabilities::all_kubeconfig_paths();
                    let yamls = cluster_oidc::read_kubeconfig_yamls(&paths);
                    tauri::async_runtime::block_on(cluster_oidc::DesktopClusterOidc::build(
                        &config_dir,
                        &yamls,
                    ))
                });
            match oidc_env {
                Ok(oidc) => {
                    tauri::async_runtime::block_on(oidc.install_on(&oidc_cache));
                    app.manage(std::sync::Arc::new(oidc));
                    // Expose the shared cache so the login commands can clear it
                    // (force a re-resolve) after a token change.
                    app.manage(oidc_cache);
                }
                Err(e) => log::warn!("cluster OIDC unavailable: {e}"),
            }

            // MCP: the token store and audit log live under the app config
            // dir, same convention as cluster OIDC above. Absence of a
            // resolvable config dir is logged and skipped — the MCP token/
            // audit commands simply error until the app is restarted somewhere
            // that dir resolution succeeds, rather than aborting startup.
            //
            // Secrets live in the encrypted vault (`vault.rs`): one
            // `secrets.enc` under the MCP config dir, keyed by ONE keychain
            // entry resolved right here — the process's only keychain touch,
            // so dev builds prompt at most once per launch instead of once
            // per secret. Managed both as the concrete `Arc<Vault>` (so
            // `mcp_token_storage` can report where the master key lives, and
            // the llm key commands can reach it) and, via `VaultTokenStore`,
            // as the `Arc<dyn TokenStore>` the MCP commands take. Same dir
            // `main.rs`'s headless CLI resolves, so a token provisioned in
            // one is usable from the other.
            match app.path().app_config_dir().map(|d| d.join("mcp")) {
                Ok(dir) => {
                    if let Err(e) = std::fs::create_dir_all(&dir) {
                        log::warn!("could not create MCP config dir {}: {e}", dir.display());
                    }
                    let vault = std::sync::Arc::new(vault::Vault::open(&dir));
                    let token_store: std::sync::Arc<dyn srelens_mcp::auth::TokenStore> =
                        std::sync::Arc::new(vault::VaultTokenStore(vault.clone()));
                    app.manage(token_store);
                    app.manage(vault);
                    app.manage(McpAuditPath(dir.join("audit.jsonl")));

                    let prompts_dir = dir.join("prompts");
                    if let Err(e) = std::fs::create_dir_all(&prompts_dir) {
                        log::warn!(
                            "could not create MCP prompts dir {}: {e}",
                            prompts_dir.display()
                        );
                    }
                    app.manage(McpPromptsDir(prompts_dir));
                }
                Err(e) => log::warn!("MCP config dir unavailable: {e}"),
            }
            app.manage(std::sync::Arc::new(mcp_confirm::Pending::default()));

            Ok(())
        })
        .manage(AppRegistry(registry))
        // The cache itself, for commands that need the live kubeconfig paths
        // (overview_snapshot resolves context → cluster identity from them).
        .manage(cache.clone())
        .manage(WatchManager::new(cache.clone()))
        .manage(ExecManager::new(cache.clone()))
        .manage(ForwardManager::new(cache.clone()))
        .manage(McpHttpManager::new(cache.clone()))
        .manage(assistant::ChatManager::default())
        .manage(llm_agent::NativeHistory::default())
        .manage(LogStreamManager::new(cache))
        .manage(TerminalManager::new())
        .manage(HelmManager::new())
        .invoke_handler(tauri::generate_handler![
            deep_link::take_pending_deep_links,
            assistant::agent_list,
            assistant::chat_start,
            assistant::chat_send,
            assistant::chat_cancel,
            llm_agent::llm_get_settings,
            llm_agent::llm_set_settings,
            llm_agent::llm_set_key,
            llm_agent::llm_clear_key,
            llm_agent::llm_key_status,
            llm_agent::llm_list_models,
            assistant_history::chat_history_list,
            assistant_history::chat_history_load,
            assistant_history::chat_history_save,
            assistant_history::chat_history_delete,
            overview_snapshot::overview_snapshot_load,
            overview_snapshot::overview_snapshot_save,
            overview_snapshot::overview_snapshot_clear,
            assistant_prompts::assistant_prompts_list,
            assistant_prompts::assistant_prompt_get,
            assistant_skills::skills_list,
            assistant_skills::skill_load,
            assistant_skills::skill_save,
            assistant_skills::skill_delete,
            invoke_capability,
            start_resource_watch,
            stop_watch,
            start_pod_exec,
            exec_input,
            exec_resize,
            exec_close,
            start_port_forward,
            stop_port_forward,
            list_forwards,
            start_log_stream,
            stop_log_stream,
            save_text_file,
            open_external,
            pick_kubeconfig_files,
            save_pasted_kubeconfig,
            start_tool_install,
            update_check,
            update_install,
            set_request_timeout,
            get_request_timeout,
            mcp_http_start,
            mcp_http_stop,
            mcp_http_status,
            mcp_confirm_respond,
            mcp_token_get,
            mcp_token_rotate,
            mcp_token_revoke,
            mcp_token_storage,
            vault_biometric::vault_biometric_status,
            vault_biometric::vault_biometric_enable,
            vault_biometric::vault_biometric_disable,
            vault_biometric::vault_biometric_unlock,
            vault_password::vault_status,
            vault_password::vault_setup_password,
            vault_password::vault_unlock_password,
            vault_password::vault_recover_password,
            vault_password::vault_change_password,
            mcp_audit_tail,
            mcp_prompt_issues,
            install_srelens_cli,
            srelens_cli_status,
            start_terminal,
            terminal_input,
            terminal_resize,
            terminal_close,
            start_helm_op,
            helm_op_close,
            read_app_log,
            app_log_path,
            reveal_app_log,
            cluster_oidc_cmd::cluster_login,
            cluster_oidc_cmd::cluster_logout,
            cluster_oidc_cmd::list_clusters
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
