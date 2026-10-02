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
use std::ffi::OsStr;

#[cfg(windows)]
use srelens_kube::connect::HIDE_EXEC_PLUGIN_WINDOWS_ENV;

/// What to set that variable to, given its current value; `None` leaves it
/// alone. A value the user set wins. Hidden windows also
/// hide a prompt a plugin prints in its console, such as the code from
/// `kubelogin --login devicecode`, so `0` is how to get that window back.
#[cfg(any(windows, test))]
fn exec_plugin_window_setting(existing: Option<&OsStr>) -> Option<&'static str> {
    existing.is_none().then_some("1")
}

/// Have kube-rs start kubeconfig exec plugins without a console window.
///
/// Call it first thing in `main`, while the process is still single-threaded.
/// Does nothing off Windows.
pub fn hide_exec_plugin_windows() {
    #[cfg(windows)]
    if let Some(value) =
        exec_plugin_window_setting(std::env::var_os(HIDE_EXEC_PLUGIN_WINDOWS_ENV).as_deref())
    {
        std::env::set_var(HIDE_EXEC_PLUGIN_WINDOWS_ENV, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hides_the_windows_when_the_variable_is_unset() {
        assert_eq!(exec_plugin_window_setting(None), Some("1"));
    }

    #[test]
    fn a_value_the_user_set_wins() {
        assert_eq!(exec_plugin_window_setting(Some(OsStr::new("0"))), None);
        assert_eq!(exec_plugin_window_setting(Some(OsStr::new("1"))), None);
    }
}
