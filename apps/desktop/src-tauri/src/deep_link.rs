//! Deep links (`srelens://`), single-instance focus, and window-state glue
//! for issue #36.

/// Deep links that arrived before the frontend consumed them.
///
/// A cold start via `srelens://…` delivers the URL while the WebView is still
/// booting, so an event alone would be emitted into the void. Every link is
/// queued here and the frontend DRAINS the queue (see
/// `take_pending_deep_links`), with the event acting only as a nudge — so a
/// link is handled exactly once whether the app was already running or was
/// started by the link itself.
///
/// A queue rather than a single slot: several links can land back-to-back
/// (a user clicking twice, or one `OpenUrlEvent` carrying several URLs), and
/// overwriting would silently drop all but the last.
#[derive(Default)]
pub struct PendingDeepLink(std::sync::Mutex<Vec<String>>);

/// Ignore links past this depth. Unbounded growth would otherwise be possible
/// if the frontend never drains — a WebView that failed to boot, say — and a
/// backlog that deep is a malfunction, not a user intent worth honouring.
const MAX_PENDING_LINKS: usize = 32;

impl PendingDeepLink {
    fn push<I: IntoIterator<Item = String>>(&self, urls: I) {
        let Ok(mut queue) = self.0.lock() else { return };
        for url in urls {
            if queue.len() >= MAX_PENDING_LINKS {
                log::warn!("dropping deep link, {MAX_PENDING_LINKS} already pending: {url}");
                break;
            }
            queue.push(url);
        }
    }
}

/// Take every pending deep link, oldest first. Draining is what makes delivery
/// exactly-once: the nudge event and the startup path both call this.
#[tauri::command]
pub fn take_pending_deep_links(pending: tauri::State<'_, PendingDeepLink>) -> Vec<String> {
    pending
        .0
        .lock()
        .map(|mut queue| std::mem::take(&mut *queue))
        .unwrap_or_default()
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
        // Every URL in the event, not just the first: one activation can carry
        // several, and dropping the rest loses navigations silently.
        let urls: Vec<String> = event.urls().iter().map(ToString::to_string).collect();
        if urls.is_empty() {
            return;
        }
        if let Some(pending) = handle.try_state::<PendingDeepLink>() {
            pending.push(urls);
        }
        focus_main_window(&handle);
        // Only a nudge — the frontend drains the queue above, so a link that
        // lands before it is listening is not lost.
        let _ = handle.emit("deep-link-pending", ());
    });

    // A cold start THROUGH a link: the URL is already waiting here, well
    // before the frontend can subscribe to anything.
    if let Ok(Some(urls)) = app.deep_link().get_current() {
        if let Some(pending) = app.try_state::<PendingDeepLink>() {
            pending.push(urls.iter().map(ToString::to_string));
        }
    }
}
