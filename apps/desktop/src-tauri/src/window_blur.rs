//! The blur behind a see-through window.
//!
//! The Appearance pane's window opacity tints the page and leaves the native
//! window clear behind it; this is what softens the desktop showing through.
//!
//! Not `Window::set_effects`. Every effect Tauri offers on macOS is an
//! `NSVisualEffectView` material, and a material is a blur PLUS a system tint
//! — `hudWindow`, the darkest of them, still laid a grey under the theme, and
//! a theme asked to be black glass came out charcoal. The window server will
//! blur behind a window with no tint at all, which is what every translucent
//! terminal on macOS uses; it is a private call, and this app already opts in
//! to the private API to have a transparent window in the first place
//! (`macOSPrivateApi` in `tauri.conf.json`).

use tauri::{Runtime, WebviewWindow};

/// How far the blur reaches, in points. Enough that text behind the window
/// stops being text; much past this the desktop turns into a flat colour and
/// the window may as well be opaque.
const BLUR_RADIUS: i32 = 40;

/// The radius that means "blurred" or "not".
fn radius_for(on: bool) -> i32 {
    if on {
        BLUR_RADIUS
    } else {
        0
    }
}

/// Blur what is behind `window`, or stop.
///
/// Answers `Ok` and does nothing off macOS: the page only asks where the
/// window is see-through (`supportsWindowOpacity` in ui-next), and a stored
/// setting carried to another platform is not an error worth surfacing.
#[tauri::command]
pub fn set_window_blur<R: Runtime>(window: WebviewWindow<R>, on: bool) -> Result<(), String> {
    apply(&window, radius_for(on))
}

#[cfg(target_os = "macos")]
fn apply<R: Runtime>(window: &WebviewWindow<R>, radius: i32) -> Result<(), String> {
    let target = window.clone();
    let (answer, answered) = std::sync::mpsc::channel();
    // AppKit is main-thread only, and a command runs on a worker — so the
    // work is posted there and its result carried back, rather than the
    // command answering `Ok` for something it never saw happen.
    window
        .run_on_main_thread(move || {
            let result = match target.ns_window() {
                // SAFETY: on the main thread, with a pointer Tauri just handed
                // out for a window this closure keeps alive.
                Ok(ns_window) => unsafe { macos::set_background_blur(ns_window, radius) },
                Err(e) => Err(format!("the window has no native handle: {e}")),
            };
            let _ = answer.send(result);
        })
        .map_err(|e| format!("could not reach the window to blur behind it: {e}"))?;
    answered
        .recv_timeout(std::time::Duration::from_secs(2))
        .map_err(|_| "the window did not answer in time".to_string())?
}

#[cfg(not(target_os = "macos"))]
fn apply<R: Runtime>(_window: &WebviewWindow<R>, _radius: i32) -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use std::ffi::c_void;

    // The window server's own blur. Private, long-stable, and what winit's
    // `set_blur`, Alacritty and iTerm2 all call.
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGSMainConnectionID() -> i32;
        fn CGSSetWindowBackgroundBlurRadius(connection: i32, window: i32, radius: i32) -> i32;
    }

    /// Set the blur behind an `NSWindow`. A radius of zero removes it.
    ///
    /// # Safety
    /// `ns_window` must be null or a live `NSWindow`, and this must be called
    /// on the main thread.
    pub unsafe fn set_background_blur(ns_window: *mut c_void, radius: i32) -> Result<(), String> {
        if ns_window.is_null() {
            return Err("the window is already gone".into());
        }
        let number: isize = msg_send![ns_window.cast::<AnyObject>(), windowNumber];
        // A `CGError`: zero is success, and anything else is the window
        // server declining — which the page must hear about, not `Ok`.
        match CGSSetWindowBackgroundBlurRadius(CGSMainConnectionID(), number as i32, radius) {
            0 => Ok(()),
            code => Err(format!("the window server refused the blur (error {code})")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Off is zero, which is how the window server spells "no blur" — any
    /// other value would leave a blur behind a window that asked for none.
    #[test]
    fn off_is_no_radius_and_on_is_some() {
        assert_eq!(radius_for(false), 0);
        assert!(radius_for(true) > 0);
    }

    /// A null handle is a window that is already gone. It must be reported,
    /// not messaged — and not answered `Ok`, which would tell the page a blur
    /// was applied to a window that does not exist.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_window_with_no_handle_is_an_error_not_a_message_to_nothing() {
        let result = unsafe { macos::set_background_blur(std::ptr::null_mut(), BLUR_RADIUS) };
        assert!(result.is_err());
    }
}
