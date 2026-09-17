//! Opening a cluster context in a window of its own.
//!
//! Generic over the runtime (#28): the unit suite below drives the command
//! through `tauri::test::MockRuntime`, so this surface counts toward the
//! coverage floor instead of hiding behind the ignore-regex.

use tauri::{AppHandle, Manager, Runtime, WebviewWindowBuilder, WebviewUrl};

/// The label a context's window opens under: `ctx-` plus the hex of its
/// identifier.
///
/// Hex because the identifier the next design hands over is a `stableId` — a
/// kubeconfig PATH plus the context's name inside it — and a label has to be a
/// short, unique, filesystem-safe key. Two contexts that differ anywhere in
/// that pair get different windows; the same context always gets the same one,
/// which is what makes opening it twice a focus rather than a second window.
fn window_label(context_id: &str) -> String {
    let hex_id = context_id.as_bytes().iter().map(|b| format!("{b:02x}")).collect::<String>();
    format!("ctx-{hex_id}")
}

/// Percent-encode a context id for the query string.
///
/// Everything outside the unreserved set goes, and not only the obvious: a
/// `stableId` carries `/`, `\`, `:` and — critically — `#`, which unencoded
/// would end the query and turn the rest of the id into a fragment the
/// frontend never sees. `URLSearchParams.get` decodes this back exactly.
fn encoded_context(context_id: &str) -> String {
    context_id
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Open the context's window, or bring it forward if it is already open.
#[tauri::command]
pub async fn open_context_window<R: Runtime>(
    app: AppHandle<R>,
    context_id: String,
) -> Result<(), String> {
    let label = window_label(&context_id);

    if let Some(win) = app.get_webview_window(&label) {
        // Already open, so the whole job is to bring it forward — and to SAY
        // when that did not happen. All three results used to be discarded and
        // the command answered `Ok(())`, so a window left minimized, hidden or
        // behind another was reported as opened: both callers treat `Ok` as
        // success and had neither a reason to show nor a retry to offer. Linux
        // answers `Ok` here whatever the compositor did (tao only logs), so
        // this surfaces the failures on the platforms that can report them.
        win.unminimize().map_err(|e| format!("could not restore the context window: {e}"))?;
        win.show().map_err(|e| format!("could not show the context window: {e}"))?;
        win.set_focus().map_err(|e| format!("could not focus the context window: {e}"))?;
        return Ok(());
    }

    let url = WebviewUrl::App(format!("index.html?context={}", encoded_context(&context_id)).into());

    WebviewWindowBuilder::new(&app, &label, url)
        .title("srelens")
        .inner_size(1024.0, 768.0)
        .min_inner_size(640.0, 480.0)
        .center()
        // Match `tauri.conf.json`'s `dragDropEnabled: false` — without this,
        // Windows' native handler swallows the HTML5 drag events TabStrip uses.
        .disable_drag_drop_handler()
        .build()
        .map_err(|e| format!("could not open a window for that cluster: {e}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The label is a pure function of the identifier: the same context always
    /// maps to the same window, and two that differ anywhere in the pair do not
    /// share one.
    #[test]
    fn a_context_maps_to_one_label() {
        assert_eq!(window_label("kind-dev"), "ctx-6b696e642d646576");
        assert_eq!(window_label("kind-dev"), window_label("kind-dev"));
        assert_ne!(window_label("/a/config#kind-dev"), window_label("/b/config#kind-dev"));
    }

    /// A `stableId` is a path plus a name, so it is full of characters that
    /// would otherwise end or corrupt the query — `#` above all, which would
    /// make everything after it a fragment the frontend never reads.
    #[test]
    fn the_query_survives_a_stable_id() {
        let encoded = encoded_context(r"C:\Users\sre\.kube\config#kind dev");
        assert_eq!(encoded, "C%3A%5CUsers%5Csre%5C.kube%5Cconfig%23kind%20dev");
        // The unreserved set passes through untouched, so a plain context name
        // is still readable in the URL.
        assert_eq!(encoded_context("kind-dev_1.2~x"), "kind-dev_1.2~x");
    }

    /// The command against a MockRuntime app: the first call creates the window
    /// under the derived label with the encoded context in its URL, and the
    /// second finds that window and focuses it instead of creating another.
    /// Both answer `Ok` — on this runtime the focus calls cannot fail, which is
    /// exactly the case the discarded results used to hide behind.
    #[tokio::test]
    async fn opening_twice_focuses_the_one_window() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();

        open_context_window(handle.clone(), "/home/sre/.kube/config#kind-dev".into())
            .await
            .unwrap();

        let label = window_label("/home/sre/.kube/config#kind-dev");
        let win = handle.get_webview_window(&label).expect("the window was just created");
        let url = win.url().unwrap().to_string();
        assert!(
            url.contains("context=%2Fhome%2Fsre%2F.kube%2Fconfig%23kind-dev"),
            "unexpected url: {url}"
        );

        open_context_window(handle.clone(), "/home/sre/.kube/config#kind-dev".into())
            .await
            .unwrap();
        let ours = handle.webview_windows().keys().filter(|l| l.starts_with("ctx-")).count();
        assert_eq!(ours, 1, "a second open must focus the existing window, not add one");
    }
}
