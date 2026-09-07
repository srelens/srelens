//! Open an address in the user's own default browser.
//!
//! The WebView cannot do this itself: Tauri doesn't patch `window.open`, and
//! wry's `WKUIDelegate` returns nil for a new window unless the app installs a
//! new-window handler — srelens installs none, so `window.open` and
//! `<a target="_blank">` are silent no-ops on the desktop (#348). The same
//! shape as `files::save_text_file`, which exists because `<a download>` is
//! silently dead in a WebView for the same kind of reason.
//!
//! `tauri-plugin-opener` is already a dependency and already registered.
//! Reaching it from a Rust command needs no JS package and no capability
//! permission: capabilities gate JS-to-plugin calls, not Rust-to-plugin ones.

use tauri::{AppHandle, Runtime};
use tauri_plugin_opener::OpenerExt;

/// Open `url` in the default browser.
///
/// The frontend only ever calls this with an address srelens itself built from
/// a live forward, and [`checked_http_url`] is what keeps that true from this
/// side as well: this must not become a way for a string that arrived from a
/// cluster to reach the OS's URL handlers.
#[tauri::command]
pub async fn open_external<R: Runtime>(app: AppHandle<R>, url: String) -> Result<(), String> {
    let url = checked_http_url(&url)?;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|error| error.to_string())
}

/// `url` back, if it is an absolute http(s) URL with a non-empty authority and
/// nothing in it that a URL cannot contain.
///
/// Deliberately not a repair: a bare `localhost:12492` is refused rather than
/// given a scheme, so the only thing this command can do is open a URL a
/// caller had already decided on. `browsable` in `packages/core` is where an
/// address becomes one, in front of the same gate.
fn checked_http_url(url: &str) -> Result<String, String> {
    const REFUSED: &str = "srelens only opens http and https addresses.";
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(REFUSED.to_string());
    }
    let lower = url.to_ascii_lowercase();
    let after_scheme = lower
        .strip_prefix("http://")
        .or_else(|| lower.strip_prefix("https://"))
        .ok_or(REFUSED)?;
    // The authority runs to the first `/`, `?` or `#`; an empty one is not a
    // host, so `http://` on its own is not a URL either.
    let authority = after_scheme.find(['/', '?', '#']).unwrap_or(after_scheme.len());
    if authority == 0 {
        return Err(REFUSED.to_string());
    }
    Ok(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_an_absolute_http_or_https_url_through_untouched() {
        for url in [
            "http://localhost:12492",
            "http://127.0.0.1:8080/metrics",
            "https://srelens.example/pf/7/",
            // The scheme is matched case-insensitively, and the URL is handed
            // on exactly as it arrived rather than normalised.
            "HTTPS://srelens.example/pf/7/",
        ] {
            assert_eq!(checked_http_url(url).unwrap(), url);
        }
    }

    #[test]
    fn refuses_every_other_scheme() {
        // The whole point of the gate: this command is given a URL srelens
        // built from a live forward, never arbitrary text from a cluster, and
        // it must not be usable as a general "hand anything to the OS" path.
        for url in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            "srelens://open",
            "smb://fileserver/share",
            "mailto:someone@example.com",
        ] {
            assert!(checked_http_url(url).is_err(), "should have been refused: {url}");
        }
    }

    #[test]
    fn refuses_a_bare_authority_and_an_empty_one() {
        // `forwardAddress` answers `localhost:12492` on the desktop. That is
        // not a URL, and repairing it here is the frontend's job (`browsable`)
        // so that this side stays a gate rather than a fixer-upper.
        for url in ["localhost:12492", "/pf/7/", "", "http://", "https://", "http:/localhost"] {
            assert!(checked_http_url(url).is_err(), "should have been refused: {url}");
        }
    }

    #[test]
    fn refuses_whitespace_and_control_characters() {
        // A newline in an argument handed to a launcher is how one URL becomes
        // two commands.
        for url in [
            "http://localhost:12492 --bad",
            "http://localhost:12492\nhttp://evil.example",
            "http://localhost:12492\u{0}",
            "http://local host:12492",
        ] {
            assert!(checked_http_url(url).is_err(), "should have been refused: {url:?}");
        }
    }
}
