//! The last `srelens-tui` build installs the `srectl` command beside itself.
//!
//! Copies already installed can only download an archive named `srelens-tui`.
//! This build is that archive. The first time it runs from that name, it
//! copies itself to `srectl` and, on Unix, leaves `srelens-tui` as a wrapper
//! that runs `srectl`. Later updates replace `srectl` only, so the old command
//! keeps working.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::self_update::package_manager_for;

const NOTICE: &str = "srelens-tui is now srectl";

/// The file name this build is published under.
pub fn legacy_file_name() -> &'static str {
    if cfg!(windows) {
        "srelens-tui.exe"
    } else {
        "srelens-tui"
    }
}

/// The command name later releases publish.
pub fn next_file_name() -> &'static str {
    if cfg!(windows) {
        "srectl.exe"
    } else {
        "srectl"
    }
}

/// Whether `current` is an unpacked `srelens-tui` this process should rebrand.
///
/// A package manager installs both names itself. A Cargo `target/` binary is
/// a developer or test build; rewriting it would replace the binary the tests
/// spawn.
pub fn should_rebrand(current: &Path) -> bool {
    file_name_is(current, legacy_file_name())
        && package_manager_for(current).is_none()
        && !in_cargo_target(current)
}

fn file_name_is(path: &Path, name: &str) -> bool {
    path.file_name().and_then(|n| n.to_str()) == Some(name)
}

fn in_cargo_target(path: &Path) -> bool {
    let text = path.to_string_lossy().replace('\\', "/");
    text.contains("/target/debug/") || text.contains("/target/release/")
}

/// Where the `srectl` command is installed, next to `legacy`.
pub fn next_command_path(legacy: &Path) -> PathBuf {
    legacy.with_file_name(next_file_name())
}

/// The Unix wrapper left at the old command name.
pub fn wrapper_script(next: &Path) -> String {
    let quoted = next.display().to_string().replace('\'', "'\\''");
    format!("#!/bin/sh\necho '{NOTICE}' >&2\nexec '{quoted}' \"$@\"\n")
}

/// Copy this binary to `srectl` when that command is not there yet, and on
/// Unix replace the old name with the wrapper.
///
/// An existing `srectl` is left alone: it may already be a newer release.
/// Returns the path the caller should re-exec.
pub fn apply_rebrand(current: &Path) -> Result<PathBuf, String> {
    let next = next_command_path(current);
    if !next.exists() {
        std::fs::copy(current, &next)
            .map_err(|e| format!("could not copy this command to {}: {e}", next.display()))?;
        set_executable(&next)?;
    }
    #[cfg(unix)]
    write_wrapper(current, &next)?;
    Ok(next)
}

#[cfg(unix)]
fn set_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(path)
        .map_err(|e| format!("could not read {}: {e}", path.display()))?
        .permissions();
    permissions.set_mode(permissions.mode() | 0o755);
    std::fs::set_permissions(path, permissions)
        .map_err(|e| format!("could not mark {} executable: {e}", path.display()))
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn write_wrapper(legacy: &Path, next: &Path) -> Result<(), String> {
    let dir = legacy.parent().unwrap_or_else(|| Path::new("."));
    let staged = dir.join(".srelens-tui.rebrand");
    std::fs::write(&staged, wrapper_script(next))
        .map_err(|e| format!("could not write the srelens-tui wrapper: {e}"))?;
    set_executable(&staged)?;
    std::fs::rename(&staged, legacy).map_err(|e| {
        let _ = std::fs::remove_file(&staged);
        format!(
            "could not replace {} with the wrapper: {e}",
            legacy.display()
        )
    })
}

/// Replace this process with `next`, passing along every argument.
pub fn exec_next(next: &Path) -> String {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let error = Command::new(next).args(&args).exec();
        return format!("{error}");
    }
    #[cfg(not(unix))]
    {
        match Command::new(next).args(&args).status() {
            Ok(status) => std::process::exit(status.code().unwrap_or(1)),
            Err(error) => format!("{error}"),
        }
    }
}

pub fn notice() -> &'static str {
    NOTICE
}
