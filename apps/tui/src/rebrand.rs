//! The last `srelens-tui` build installs the `srectl` command beside itself.
//!
//! Copies already installed can only download an archive named `srelens-tui`.
//! This build is that archive. The first time it runs from that name, it
//! copies itself to `srectl` (unless one as new is already there) and, on
//! Unix, leaves `srelens-tui` as a wrapper that runs `srectl`. Later updates
//! replace `srectl` only, so the old command keeps working.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::self_update::{is_newer, package_manager_for};

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

/// Whether an existing `srectl` whose `--version` printed `reported` should be
/// replaced by this build, `this_version`.
///
/// Only an older one is. A same or newer one is kept: on Windows there is no
/// wrapper, so `srelens-tui.exe` stays this build and runs this on every
/// launch, and replacing anything but an older `srectl.exe` would downgrade
/// the one self-update has since moved forward. A version that cannot be read
/// (`None`, or output that is not `srectl <semver>`) also keeps it, for the
/// same reason: it may be a later release, or a slow start that hit the time
/// limit, and overwriting it would then be that downgrade, repeated every
/// launch. Keeping it is what this did before versions were compared.
pub fn replaces_existing(reported: Option<&str>, this_version: &str) -> bool {
    // Our own line, `srectl 0.16.0` (`srelens-tui 0.15.0` before the rename),
    // and nothing else: the file at that path is ours only by its name.
    let reported = reported
        .and_then(|out| out.lines().next())
        .and_then(|line| match line.split_whitespace().collect::<Vec<_>>()[..] {
            ["srectl" | "srelens-tui", version] => Some(version.trim_start_matches('v')),
            _ => None,
        });
    reported.is_some_and(|version| is_newer(version, this_version))
}

/// How long an existing `srectl` gets to say its version.
const VERSION_PROBE_LIMIT: Duration = Duration::from_secs(5);

/// `next --version`. It has passed the trust check and is about to be run anyway.
fn version_probe(next: &Path) -> Command {
    let mut probe = Command::new(next);
    probe.arg("--version");
    probe
}

/// What `probe` prints, or `None` if it does not answer within `limit`.
fn reported_version(mut probe: Command, limit: Duration) -> Option<String> {
    let mut child = probe
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + limit;
    // Read on a thread of its own. The probe exiting does not close the pipe
    // while something it started still holds it, and a read here would wait
    // for that, past the limit. The thread is left behind if it never ends.
    let mut stdout = child.stdout.take()?;
    let (sender, output) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut out = String::new();
        if stdout.read_to_string(&mut out).is_ok() {
            let _ = sender.send(out);
        }
    });
    let status = loop {
        if let Some(status) = child.try_wait().ok()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    // A probe that failed said nothing about the version, whatever it printed.
    if !status.success() {
        return None;
    }
    output.recv_timeout(deadline.saturating_duration_since(Instant::now())).ok()
}

/// Copy this binary to `srectl` when that command is not there yet or is older
/// than `this_version`, and on Unix replace the old name with the wrapper.
///
/// An existing `srectl` is verified for safety before handoff.
/// Returns the path the caller should re-exec.
pub fn apply_rebrand(current: &Path, this_version: &str) -> Result<PathBuf, String> {
    let next = next_command_path(current);
    let install = !is_trusted_existing_command(current, &next)?
        || replaces_existing(
            reported_version(version_probe(&next), VERSION_PROBE_LIMIT).as_deref(),
            this_version,
        );
    if install {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_srectl_reporting_an_older_version_is_replaced() {
        // `srectl --version` prints `srectl <version>`; the `version` subcommand
        // prints `srectl v<version>`. Builds from before the rename print
        // `srelens-tui <version>`.
        assert!(replaces_existing(Some("srectl 0.15.0-dev.3\n"), "0.15.0"));
        assert!(replaces_existing(Some("srectl v0.14.2\n"), "0.15.0"));
        assert!(replaces_existing(Some("srelens-tui 0.14.2\n"), "0.15.0"));
        assert!(!replaces_existing(Some("srectl 0.15.0\n"), "0.15.0"));
        assert!(!replaces_existing(Some("srectl 0.16.0-dev.1\n"), "0.15.0"));
        // A version that cannot be read keeps the existing command.
        assert!(!replaces_existing(None, "0.15.0"));
        assert!(!replaces_existing(Some(""), "0.15.0"));
        assert!(!replaces_existing(Some("command not found\n"), "0.15.0"));
        // So does another program's version: the file is only ours by name.
        assert!(!replaces_existing(Some("other-command 0.14.0\n"), "0.15.0"));
        assert!(!replaces_existing(Some("srectl is 0.14.0\n"), "0.15.0"));
    }

    /// A shell command line, run the way this platform runs one.
    fn shell(script: &str) -> Command {
        if cfg!(windows) {
            let mut command = Command::new("cmd");
            command.args(["/C", script]);
            command
        } else {
            let mut command = Command::new("sh");
            command.args(["-c", script]);
            command
        }
    }

    #[test]
    fn a_probe_that_answers_reports_what_it_printed() {
        let out = reported_version(shell("echo srectl 0.1.0"), Duration::from_secs(5));
        assert_eq!(out.as_deref().map(str::trim), Some("srectl 0.1.0"));
    }

    #[test]
    fn a_probe_that_exits_with_a_failure_reports_nothing() {
        let script = if cfg!(windows) { "echo srectl 0.1.0& exit 3" } else { "echo srectl 0.1.0; exit 3" };
        assert_eq!(reported_version(shell(script), Duration::from_secs(5)), None);
    }

    #[test]
    fn a_probe_whose_output_stays_open_after_it_exits_gives_up_at_the_limit() {
        // The command exits at once, but what it started holds the pipe for
        // twenty seconds, so reading to the end would wait that long.
        let script = if cfg!(windows) {
            "echo srectl 0.1.0& start /b ping -n 21 127.0.0.1"
        } else {
            "echo srectl 0.1.0; sleep 20 &"
        };
        let started = Instant::now();
        assert_eq!(reported_version(shell(script), Duration::from_secs(1)), None);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the probe outlived its limit: {:?}",
            started.elapsed()
        );
    }
}
