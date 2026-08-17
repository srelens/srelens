//! Deep links (`srelens://`), single-instance focus, and window-state glue
//! for issue #36.

/// A deep link that arrived before the frontend was listening.
///
/// A cold start via `srelens://…` delivers the URL while the WebView is still
/// booting, so an event alone would be emitted into the void. Every link is
/// stashed here and the frontend DRAINS it (see `take_pending_deep_link`),
/// with the event acting only as a nudge — so a link is handled exactly once
/// whether the app was already running or started by the link itself.
#[derive(Default)]
pub struct PendingDeepLink(std::sync::Mutex<Option<String>>);

/// Take the pending deep link, if any. Draining is what makes delivery
/// exactly-once: the nudge event and the cold-start path both call this.
#[tauri::command]
pub fn take_pending_deep_link(pending: tauri::State<'_, PendingDeepLink>) -> Option<String> {
    pending.0.lock().ok().and_then(|mut slot| slot.take())
}

/// Bring the main window to the user. A second launch, or a link click while
/// the app sits minimized behind other windows, should surface it.
pub fn focus_main_window<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    use tauri::Manager;
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Whether the window-state plugin has a saved geometry to restore.
pub fn has_saved_window_state(app: &tauri::App) -> bool {
    use tauri::Manager;
    app.path()
        .app_config_dir()
        .map(|dir| dir.join(tauri_plugin_window_state::DEFAULT_FILENAME).exists())
        .unwrap_or(false)
}

/// Route `srelens://` links to the frontend.
pub fn register_deep_links(app: &tauri::App) {
    use tauri::{Emitter, Manager};
    use tauri_plugin_deep_link::DeepLinkExt;

    // A dev build is never "installed", so no OS registration exists for the
    // scheme. Registering at runtime makes deep links testable with
    // `tauri dev`; packaged builds get theirs from the bundle configuration.
    #[cfg(debug_assertions)]
    let _ = app.deep_link().register_all();

    let handle = app.handle().clone();
    app.deep_link().on_open_url(move |event| {
        let Some(url) = event.urls().into_iter().next() else {
            return;
        };
        if let Some(pending) = handle.try_state::<PendingDeepLink>() {
            if let Ok(mut slot) = pending.0.lock() {
                *slot = Some(url.to_string());
            }
        }
        focus_main_window(&handle);
        // Only a nudge — the frontend drains the slot above, so a link that
        // lands before it is listening is not lost.
        let _ = handle.emit("deep-link-pending", ());
    });

    // A cold start THROUGH a link: the URL is already waiting here, well
    // before the frontend can subscribe to anything.
    if let Ok(Some(urls)) = app.deep_link().get_current() {
        if let Some(url) = urls.into_iter().next() {
            if let Some(slot) = app.try_state::<PendingDeepLink>() {
                if let Ok(mut slot) = slot.0.lock() {
                    *slot = Some(url.to_string());
                }
            }
        }
    }
}
