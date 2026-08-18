//! Channel-aware auto-update.
//!
//! Tauri's updater endpoints are static config, so they can't be switched at
//! runtime. To support a user-selectable channel ("stable" / "dev") we build
//! the updater per check with the endpoint for the chosen channel via
//! `app.updater_builder().endpoints(...)`.
//!
//! - stable → the latest non-prerelease `latest.json`
//! - dev    → `latest.json` attached to the permanent `dev-channel` pre-release
//!
//! Both manifests are signed with the same updater key (configured in
//! tauri.conf.json `plugins.updater.pubkey`), so switching channels is safe.
//! Tauri only updates to a *higher* version, so stable→dev pulls newer dev
//! builds; dev→stable does not auto-downgrade.

use serde::Serialize;
use tauri::utils::config::BundleType;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

const STABLE_ENDPOINT: &str =
    "https://github.com/srelens/srelens/releases/latest/download/latest.json";
const DEV_ENDPOINT: &str =
    "https://github.com/srelens/srelens/releases/download/dev-channel/latest.json";

fn endpoint_for(channel: &str) -> &'static str {
    if channel == "dev" {
        DEV_ENDPOINT
    } else {
        STABLE_ENDPOINT
    }
}

/// Whether this install can update itself, or must defer to a system package
/// manager. The bundler stamps the bundle type into the binary, so a pacman
/// (AUR) repack of the .deb still reports `Deb` — the tell is that the tool
/// the updater would run (`dpkg -i` / `rpm -U`) isn't there. In that case the
/// install belongs to a package manager the updater knows nothing about, and
/// self-updating would both fail and desync that manager's database.
fn self_updatable(os: &str, bundle: Option<BundleType>, has_dpkg: bool, has_rpm: bool) -> bool {
    if os != "linux" {
        return true;
    }
    match bundle {
        Some(BundleType::AppImage) => true,
        Some(BundleType::Deb) => has_dpkg,
        Some(BundleType::Rpm) => has_rpm,
        _ => false,
    }
}

/// Whether installing the update will ask for administrator rights.
///
/// The AppImage is rewritten in place and macOS/Windows installers run as the
/// user, but a .deb or .rpm is applied with `dpkg -i` / `rpm -U` through
/// pkexec (falling back to a graphical or terminal sudo). Saying so beforehand
/// is the difference between an expected password prompt and one that appears
/// out of nowhere over a Kubernetes console.
fn needs_privileges(os: &str, bundle: Option<BundleType>) -> bool {
    os == "linux" && matches!(bundle, Some(BundleType::Deb) | Some(BundleType::Rpm))
}

fn externally_managed() -> bool {
    let rpm_db = std::path::Path::new("/var/lib/rpm").exists()
        || std::path::Path::new("/usr/lib/sysimage/rpm").exists();
    !self_updatable(
        std::env::consts::OS,
        tauri::utils::platform::bundle_type(),
        std::path::Path::new("/var/lib/dpkg").exists(),
        rpm_db,
    )
}

#[derive(Serialize, Clone)]
pub struct UpdateMeta {
    pub version: String,
    pub current_version: String,
    pub notes: Option<String>,
    /// True when the install is owned by a system package manager the updater
    /// can't drive (e.g. pacman for the AUR package) — the UI should point at
    /// that manager instead of offering an in-app install.
    pub external: bool,
    /// True when applying the update needs administrator rights (a .deb or
    /// .rpm install, which runs `dpkg -i` / `rpm -U` under pkexec or sudo), so
    /// the UI can warn before a password prompt appears unannounced.
    pub elevates: bool,
}

#[derive(Serialize, Clone)]
struct DownloadProgress {
    downloaded: usize,
    total: Option<u64>,
}

async fn fetch_update(
    app: &AppHandle,
    channel: &str,
) -> Result<Option<tauri_plugin_updater::Update>, String> {
    let url = endpoint_for(channel)
        .parse()
        .map_err(|e| format!("invalid update endpoint: {e}"))?;
    let updater = app
        .updater_builder()
        .endpoints(vec![url])
        .map_err(|e| e.to_string())?
        .build()
        .map_err(|e| e.to_string())?;
    updater.check().await.map_err(|e| e.to_string())
}

/// Check the given channel's manifest for an available update.
#[tauri::command]
pub async fn update_check(app: AppHandle, channel: String) -> Result<Option<UpdateMeta>, String> {
    let update = fetch_update(&app, &channel).await?;
    Ok(update.map(|u| UpdateMeta {
        version: u.version.clone(),
        current_version: u.current_version.clone(),
        notes: u.body.clone(),
        external: externally_managed(),
        elevates: needs_privileges(std::env::consts::OS, tauri::utils::platform::bundle_type()),
    }))
}

/// Download + install the available update on the given channel, emitting
/// `update://progress` events. The caller relaunches the app afterward.
#[tauri::command]
pub async fn update_install(app: AppHandle, channel: String) -> Result<(), String> {
    if externally_managed() {
        return Err(
            "This install is managed by a system package manager; update it there instead \
             (AUR package: pacman/paru)"
                .to_string(),
        );
    }
    let update = fetch_update(&app, &channel)
        .await?
        .ok_or_else(|| "No update available".to_string())?;
    let mut downloaded: usize = 0;
    let app2 = app.clone();
    update
        .download_and_install(
            move |chunk, total| {
                downloaded += chunk;
                let _ = app2.emit("update://progress", DownloadProgress { downloaded, total });
            },
            || {},
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_channel_uses_the_dev_manifest() {
        assert_eq!(endpoint_for("dev"), DEV_ENDPOINT);
    }

    #[test]
    fn everything_else_falls_back_to_stable() {
        assert_eq!(endpoint_for("stable"), STABLE_ENDPOINT);
        assert_eq!(endpoint_for("nightly"), STABLE_ENDPOINT);
        assert_eq!(endpoint_for(""), STABLE_ENDPOINT);
    }

    // The Tauri bundler stamps the bundle type into the binary, so an AUR/
    // pacman repack of the upstream .deb still reports BundleType::Deb. What
    // separates it from a real Debian install is whether dpkg exists to run
    // the update.

    #[test]
    fn aur_repack_of_the_deb_is_externally_managed() {
        assert!(!self_updatable("linux", Some(BundleType::Deb), false, false));
    }

    #[test]
    fn dpkg_managed_deb_install_self_updates() {
        assert!(self_updatable("linux", Some(BundleType::Deb), true, false));
    }

    #[test]
    fn rpm_install_self_updates_only_with_rpm_present() {
        assert!(self_updatable("linux", Some(BundleType::Rpm), false, true));
        assert!(!self_updatable("linux", Some(BundleType::Rpm), false, false));
    }

    #[test]
    fn appimage_self_updates_without_any_package_tools() {
        assert!(self_updatable("linux", Some(BundleType::AppImage), false, false));
    }

    #[test]
    fn unknown_linux_packaging_is_externally_managed() {
        assert!(!self_updatable("linux", None, true, true));
    }

    #[test]
    fn deb_and_rpm_updates_need_a_password_prompt() {
        // dpkg/rpm run under pkexec or sudo; the AppImage rewrites itself in
        // place and needs nothing.
        assert!(needs_privileges("linux", Some(BundleType::Deb)));
        assert!(needs_privileges("linux", Some(BundleType::Rpm)));
        assert!(!needs_privileges("linux", Some(BundleType::AppImage)));
    }

    #[test]
    fn other_platforms_never_prompt_for_a_password() {
        assert!(!needs_privileges("macos", Some(BundleType::App)));
        assert!(!needs_privileges("windows", Some(BundleType::Nsis)));
        assert!(!needs_privileges("linux", None));
    }

    #[test]
    fn non_linux_platforms_always_self_update() {
        assert!(self_updatable("macos", Some(BundleType::App), false, false));
        assert!(self_updatable("windows", None, false, false));
    }
}
