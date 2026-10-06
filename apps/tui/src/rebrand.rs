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
    if let Ok(target_dir) = std::env::var("CARGO_TARGET_DIR") {
        if !target_dir.is_empty() && path.starts_with(Path::new(&target_dir)) {
            return true;
        }
    }
    let components: Vec<_> = path.components().map(|c| c.as_os_str()).collect();
    if let Some(target_idx) = components.iter().position(|&c| c == "target") {
        components[target_idx..]
            .iter()
            .any(|&c| c == "debug" || c == "release")
    } else {
        false
    }
}

/// Where the `srectl` command is installed, next to `legacy`.
pub fn next_command_path(legacy: &Path) -> PathBuf {
    legacy.with_file_name(next_file_name())
}

/// The Unix wrapper left at the old command name.
pub fn wrapper_script(next: &Path) -> Result<String, String> {
    let path_str = next.to_str().ok_or_else(|| {
        format!(
            "path {} is not valid UTF-8 and cannot be represented in a shell script",
            next.display()
        )
    })?;
    let quoted = path_str.replace('\'', "'\\''");
    Ok(format!(
        "#!/bin/sh\n[ -t 2 ] && echo '{NOTICE}' >&2\nexec '{quoted}' \"$@\"\n"
    ))
}

#[cfg(unix)]
pub fn is_trusted_existing_command(current: &Path, next: &Path) -> Result<bool, String> {
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::fs::PermissionsExt;
    let next_meta = match std::fs::symlink_metadata(next) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => {
            return Err(format!(
                "could not inspect existing {}: {e}",
                next.display()
            ))
        }
    };
    if !next_meta.is_file() {
        return Err(format!(
            "existing {} is not a regular file; refusing to rebrand",
            next.display()
        ));
    }
    let current_meta = std::fs::metadata(current)
        .map_err(|e| format!("could not inspect {}: {e}", current.display()))?;
    if next_meta.uid() != current_meta.uid() {
        return Err(format!(
            "existing {} is owned by a different user; refusing to rebrand",
            next.display()
        ));
    }
    let mode = next_meta.permissions().mode();
    let is_executable = if current_meta.uid() == 0 {
        (mode & 0o111) != 0
    } else {
        (mode & 0o100) != 0
    };
    if !is_executable {
        return Err(format!(
            "existing {} is not executable; refusing to rebrand",
            next.display()
        ));
    }
    Ok(true)
}

#[cfg(not(unix))]
pub fn is_trusted_existing_command(_current: &Path, next: &Path) -> Result<bool, String> {
    match std::fs::metadata(next) {
        Ok(m) if m.is_file() => Ok(true),
        Ok(_) => Err(format!(
            "existing {} is not a regular file; refusing to rebrand",
            next.display()
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!(
            "could not inspect existing {}: {e}",
            next.display()
        )),
    }
}

/// Copy this binary to `srectl` when that command is not there yet, and on
/// Unix replace the old name with the wrapper.
///
/// An existing `srectl` is verified for safety before handoff.
/// Returns the path the caller should re-exec.
pub fn apply_rebrand(current: &Path) -> Result<PathBuf, String> {
    let next = next_command_path(current);
    if !is_trusted_existing_command(current, &next)? {
        let dir = next.parent().unwrap_or_else(|| Path::new("."));
        let (staged_path, mut file) = crate::self_update::create_new_file(dir, ".srectl.new-")
            .map_err(|e| format!("could not create staging file for {}: {e}", next.display()))?;
        let staged = crate::self_update::Staged(staged_path);

        let mut reader = std::fs::File::open(current)
            .map_err(|e| format!("could not read {}: {e}", current.display()))?;
        std::io::copy(&mut reader, &mut file)
            .map_err(|e| format!("could not copy to staging file: {e}"))?;
        set_executable(&staged.0)?;
        file.sync_all()
            .map_err(|e| format!("could not sync staging file: {e}"))?;
        drop(file);

        std::fs::rename(&staged.0, &next)
            .map_err(|e| format!("could not install {}: {e}", next.display()))?;
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
    use std::io::Write;
    let dir = legacy.parent().unwrap_or_else(|| Path::new("."));
    let script = wrapper_script(next)?;
    let (staged_path, mut file) = crate::self_update::create_new_file(dir, ".srelens-tui.wrapper-")
        .map_err(|e| format!("could not create staging file for wrapper: {e}"))?;
    let staged = crate::self_update::Staged(staged_path);

    file.write_all(script.as_bytes())
        .map_err(|e| format!("could not write the srelens-tui wrapper: {e}"))?;
    set_executable(&staged.0)?;
    file.sync_all()
        .map_err(|e| format!("could not sync the srelens-tui wrapper: {e}"))?;
    drop(file);

    std::fs::rename(&staged.0, legacy).map_err(|e| {
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
