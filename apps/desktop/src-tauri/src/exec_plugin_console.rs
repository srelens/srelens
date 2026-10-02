//! Kubeconfig exec plugins on a process with no console (#775).
//!
//! On Windows, a process with no console of its own gives every console
//! program it starts a new console window. A release build of this app is
//! such a process (`windows_subsystem = "windows"` in `main.rs`), in every run
//! mode. Exec plugins are console programs (`aws eks get-token`,
//! `gke-gcloud-auth-plugin`, `kubelogin`), so every token fetch flashed a
//! window and took focus. kube-rs starts them with `CREATE_NO_WINDOW` only
//! when `KUBE_RS_UNSTABLE_CREATE_NO_WINDOW` is `1` (kube-rs/kube#1901); kube
//! 0.96, which srelens used through v0.7.0, always did.

/// Have kube-rs start kubeconfig exec plugins without a console window.
///
/// Call it first thing in `main`, while the process is still single-threaded.
/// Does nothing off Windows.
pub fn hide_exec_plugin_windows() {
    #[cfg(windows)]
    std::env::set_var("KUBE_RS_UNSTABLE_CREATE_NO_WINDOW", "1");
}
