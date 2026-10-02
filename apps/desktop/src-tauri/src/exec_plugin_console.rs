//! Kubeconfig exec plugins on a process with no console (#775).
//!
//! On Windows, a process with no console of its own gives every console
//! program it starts a new console window. A release build of this app is
//! such a process (`windows_subsystem = "windows"` in `main.rs`), in every run
//! mode. Exec plugins are console programs (`aws eks get-token`,
//! `gke-gcloud-auth-plugin`, `kubelogin`), so every token fetch flashed a
//! window and took focus. kube-rs starts them with `CREATE_NO_WINDOW` only
//! when [`srelens_kube::connect::HIDE_EXEC_PLUGIN_WINDOWS_ENV`] is `1`; kube
//! 0.96, which srelens used through v0.7.0, always did.

#[cfg(any(windows, test))]
use std::ffi::OsString;

#[cfg(any(windows, test))]
use srelens_kube::connect::HIDE_EXEC_PLUGIN_WINDOWS_ENV;

/// Set kube-rs's variable to `1` unless it already has a value. A value the
/// user set wins: hidden windows also hide a prompt a plugin prints in its
/// console, such as the code from `kubelogin --login devicecode`, so `0` is
/// how to get that window back.
///
/// `get` and `set` stand in for the process environment, so the tests check
/// on every platform which variable is read and written.
#[cfg(any(windows, test))]
fn hide_with(get: impl FnOnce(&str) -> Option<OsString>, set: impl FnOnce(&str, &str)) {
    if get(HIDE_EXEC_PLUGIN_WINDOWS_ENV).is_none() {
        set(HIDE_EXEC_PLUGIN_WINDOWS_ENV, "1");
    }
}

/// Have kube-rs start kubeconfig exec plugins without a console window.
///
/// Call it first thing in `main`, while the process is still single-threaded.
/// Does nothing off Windows.
pub fn hide_exec_plugin_windows() {
    #[cfg(windows)]
    hide_with(
        |name| std::env::var_os(name),
        |name, value| std::env::set_var(name, value),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run [`hide_with`] against a stand-in environment in which kube-rs's
    /// variable holds `existing` and nothing else is set. Returns what it
    /// wrote.
    fn hide_given(existing: Option<&str>) -> Vec<(String, String)> {
        let mut written = Vec::new();
        hide_with(
            |name| {
                (name == HIDE_EXEC_PLUGIN_WINDOWS_ENV)
                    .then_some(existing)
                    .flatten()
                    .map(OsString::from)
            },
            |name, value| written.push((name.to_owned(), value.to_owned())),
        );
        written
    }

    #[test]
    fn sets_the_variable_kube_rs_reads_to_1_when_it_is_unset() {
        assert_eq!(
            hide_given(None),
            [(HIDE_EXEC_PLUGIN_WINDOWS_ENV.to_owned(), "1".to_owned())]
        );
    }

    #[test]
    fn a_value_the_user_set_wins() {
        assert_eq!(hide_given(Some("0")), []);
        assert_eq!(hide_given(Some("1")), []);
    }
}
