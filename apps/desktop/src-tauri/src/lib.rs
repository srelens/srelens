mod bridge;
mod capabilities;
mod exec;
mod files;
mod forward;
mod logs;
mod mcp;
mod settings;
mod updater;
mod watch;

use bridge::{invoke_capability, AppRegistry};
use files::{pick_kubeconfig_files, save_pasted_kubeconfig, save_text_file};
use exec::{exec_close, exec_input, start_pod_exec, ExecManager};
use forward::{start_port_forward, stop_port_forward, ForwardManager};
use srelens_kube::client_cache::ClientCache;
use logs::{start_log_stream, stop_log_stream, LogStreamManager};
use mcp::{
    install_srelens_cli, mcp_http_start, mcp_http_status, mcp_http_stop, srelens_cli_status,
    McpHttpManager,
};
use settings::{get_request_timeout, set_request_timeout};
use updater::{update_check, update_install};
use watch::{start_resource_watch, stop_watch, WatchManager};

pub use capabilities::build_registry;

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

    let mut last_modified: HashMap<PathBuf, Option<SystemTime>> = HashMap::new();

    // Initialize the map with current files
    let initial_paths = cache.paths().await;
    for path in initial_paths {
        let modified = tokio::fs::metadata(&path)
            .await
            .and_then(|m| m.modified())
            .ok();
        last_modified.insert(path, modified);
    }

    loop {
        tokio::time::sleep(tokio::time::Duration::from_millis(1500)).await;

        let current_paths = cache.paths().await;
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
    let cache = ClientCache::new_many(capabilities::default_kubeconfig_paths());
    let registry = capabilities::build_registry_with(cache.clone());

    let builder = tauri::Builder::default().plugin(tauri_plugin_dialog::init());
    #[cfg(desktop)]
    let builder = builder
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init());

    let watcher_cache = cache.clone();
    builder
        .setup(move |app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            #[cfg(desktop)]
            size_main_window(app);
            #[cfg(target_os = "macos")]
            install_macos_menu(app)?;

            // Use Tauri's managed async runtime — `tokio::spawn` here panics
            // ("no reactor running") because `setup` runs before/outside a Tokio
            // runtime context.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                watch_kubeconfig_files(handle, watcher_cache).await;
            });

            Ok(())
        })
        .manage(AppRegistry(registry))
        .manage(WatchManager::new(cache.clone()))
        .manage(ExecManager::new(cache.clone()))
        .manage(ForwardManager::new(cache.clone()))
        .manage(McpHttpManager::new(cache.clone()))
        .manage(LogStreamManager::new(cache))
        .invoke_handler(tauri::generate_handler![
            invoke_capability,
            start_resource_watch,
            stop_watch,
            start_pod_exec,
            exec_input,
            exec_close,
            start_port_forward,
            stop_port_forward,
            start_log_stream,
            stop_log_stream,
            save_text_file,
            pick_kubeconfig_files,
            save_pasted_kubeconfig,
            update_check,
            update_install,
            set_request_timeout,
            get_request_timeout,
            mcp_http_start,
            mcp_http_stop,
            mcp_http_status,
            install_srelens_cli,
            srelens_cli_status
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
