//! Opening a cluster context in a window of its own.
//!
//! Generic over the runtime (#28): the unit suite below drives the command
//! through `tauri::test::MockRuntime`, so this surface counts toward the
//! coverage floor instead of hiding behind the ignore-regex.

use tauri::{AppHandle, Manager, Runtime, WebviewWindowBuilder, WebviewUrl};

/// The label a context's window opens under: `ctx-` plus the hex of its
/// identifier.
///
/// The identifier is a context's `key` (`ResolvedContext::key`), NOT its
/// `stableId`. Both are a kubeconfig path plus the context's name inside it,
/// but `stableId` joins them with a bare `#`, so a path `a` with context `b#c`
/// and a path `a#b` with context `c` produce the same string — one window
/// label for two different clusters. Opening the second focused the first's
/// window, and the frontend resolving `?context=` could land on either. `key`
/// percent-encodes `#` and `%` in each part, so the first `#` is always the
/// delimiter and no two contexts share one (#623). `stableId` stays the
/// PERSISTED identity elsewhere and must not change; window identity is not
/// persisted, so it can use the unambiguous form.
///
/// Hex because a label has to be a short, filesystem-safe token. Two contexts
/// that differ anywhere in the pair get different windows; the same context
/// always gets the same one, which is what makes opening it twice a focus
/// rather than a second window.
fn window_label(context_id: &str) -> String {
    let hex_id = context_id.as_bytes().iter().map(|b| format!("{b:02x}")).collect::<String>();
    format!("ctx-{hex_id}")
}

/// Percent-encode a context id for the query string.
///
/// Everything outside the unreserved set goes, and not only the obvious: a
/// context key carries `/`, `\`, `:` and — critically — `#`, which unencoded
/// would end the query and turn the rest of the id into a fragment the
/// frontend never sees. `URLSearchParams.get` decodes this back exactly, so
/// the frontend reads the key byte for byte and matches it against
/// `ClusterContext.key`.
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
///
/// `context_id` is the context's `key` — see [`window_label`] for why it is
/// not the `stableId`. Both frontends send `ClusterContext.key`.
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

    /// Built from the real thing rather than from hand-written strings, so
    /// this cannot drift from `ResolvedContext`'s own spelling.
    fn context(source: &str, name: &str) -> srelens_kube::context_resolve::ResolvedContext {
        srelens_kube::context_resolve::ResolvedContext {
            display_name: name.into(),
            original_name: name.into(),
            source: std::path::PathBuf::from(source),
            cluster: "c".into(),
            server: "https://127.0.0.1:6443".into(),
            user: "u".into(),
            namespace: String::new(),
            is_current: false,
            exec_command: None,
            auth_provider: None,
            auth_kind: "none".into(),
        }
    }

    /// The two contexts a `stableId` cannot tell apart get two windows.
    ///
    /// A kubeconfig `a` declaring `b#c` and a kubeconfig `a#b` declaring `c`
    /// both spell `a#b#c` as a stable ID. Keying the label on that made the
    /// second cluster's "Open in new window" focus the first cluster's window,
    /// and left the second unopenable. Their keys differ, so their labels do.
    #[test]
    fn the_two_contexts_a_stable_id_confuses_get_two_windows() {
        let one = context("a", "b#c");
        let other = context("a#b", "c");

        // The collision this fix is about — asserted, not assumed.
        assert_eq!(one.stable_id(), other.stable_id());
        assert_ne!(one.key(), other.key());

        assert_ne!(window_label(&one.key()), window_label(&other.key()));
        // ...where the old keying gave them one and the same window.
        assert_eq!(
            window_label(&one.stable_id()),
            window_label(&other.stable_id())
        );
    }

    /// A context key is a path plus a name, so it is full of characters that
    /// would otherwise end or corrupt the query — `#` above all, which would
    /// make everything after it a fragment the frontend never reads. A `%`
    /// from the key's own encoding has to survive too, or `%23` would arrive
    /// as a literal `#` and turn one key back into the other.
    #[test]
    fn the_query_survives_a_context_key() {
        let encoded = encoded_context(r"C:\Users\sre\.kube\config#kind dev");
        assert_eq!(encoded, "C%3A%5CUsers%5Csre%5C.kube%5Cconfig%23kind%20dev");
        assert_eq!(encoded_context("a#b%23c"), "a%23b%2523c");
        assert_ne!(encoded_context("a#b%23c"), encoded_context("a%23b#c"));
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
