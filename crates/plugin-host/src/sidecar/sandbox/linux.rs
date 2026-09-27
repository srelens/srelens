//! Linux: Landlock, a seccomp filter and a cgroup v2 directory, the backend
//! the #571 spike recommended (`landlock+seccomp+cgroup`, 11 of 11 checks).
//!
//! The host creates the sidecar's cgroup, with its memory and CPU limits,
//! under a delegated root, and starts `srelens-sandbox-launch`, which joins
//! it, applies Landlock and the filter to itself, and `exec`s the sidecar
//! (`launch.rs`). Every layer is required: the ADR's rule to refuse where no
//! sandbox exists applies to each layer, not only the first, so a kernel
//! without Landlock or a machine without a delegated cgroup refuses the app.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU32, Ordering};

use super::{Exit, LaunchError, Launched, SandboxConfig, SidecarCommand};
use crate::sidecar::Limits;

const NO_LAUNCHER: &str = "srelens has no sandbox launcher (srelens-sandbox-launch) to start the app with, so it does not run executable apps";

const NO_CGROUP: &str = "srelens has no delegated cgroup v2 directory to limit an app's memory and CPU in, so it does not run executable apps. Executable apps need one with the memory and cpu controllers enabled for its children";

pub(super) fn launch(
    config: &SandboxConfig,
    command: &SidecarCommand,
    limits: &Limits,
) -> Result<Launched, LaunchError> {
    let launcher = config
        .launcher
        .as_ref()
        .ok_or_else(|| LaunchError::Unavailable(NO_LAUNCHER.into()))?;
    let abi = landlock_abi();
    if abi < 1 {
        return Err(LaunchError::Unavailable(
            "Landlock is not enabled on this kernel, so srelens cannot confine an app's file access and does not run executable apps".into(),
        ));
    }
    let root = config
        .cgroup_root
        .as_ref()
        .ok_or_else(|| LaunchError::Unavailable(NO_CGROUP.into()))?;
    let cgroup = cgroup_in(root, limits).map_err(|e| LaunchError::Unavailable(e.to_string()))?;
    let mut cmd = tokio::process::Command::new(launcher);
    cmd.arg("--cgroup")
        .arg(cgroup.path())
        .arg("--data")
        .arg(&command.data_dir)
        .arg("--max-file-bytes")
        .arg(limits.data_bytes.to_string())
        .arg("--")
        .arg(&command.program)
        .args(&command.args)
        .env_clear()
        .envs(super::temporary_directory(command, &command.data_dir))
        .envs(command.env.iter().map(|(k, v)| (k, v)))
        .current_dir(&command.data_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let child = cmd.spawn().map_err(|e| {
        LaunchError::Failed(format!(
            "srelens could not start its sandbox launcher {}: {e}",
            launcher.display()
        ))
    })?;
    let memory = limits.memory_bytes;
    let current = cgroup.path().join("memory.current");
    // The cgroup goes with the process: `describe` reads its OOM counters once
    // the sidecar has been reaped, then drops it, which removes the directory.
    let mut launched = Launched::from_child(child, move |status| describe(status, cgroup, memory))?;
    launched.process = launched
        .process
        .with_memory(move || memory_current(&current));
    Ok(launched)
}

/// The cgroup's memory use now, from `memory.current`: `None` once the
/// directory is gone with the process.
fn memory_current(path: &Path) -> Option<u64> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// How the sidecar ended, with the cgroup's evidence when its memory limit
/// stopped it: a `SIGKILL`, with `memory.events` recording both that the
/// cgroup reached `memory.max` (`oom`) and that a process in it was OOM-killed
/// (`oom_kill`). The spike's `Stop::Memory` rule.
fn describe(status: io::Result<std::process::ExitStatus>, cgroup: Created, memory: u64) -> Exit {
    let mut exit = Exit::from_status(status);
    if exit.code == Some(super::launch::LAYER_FAILED) {
        exit.description.push_str(
            ", which is also how srelens-sandbox-launch exits when it cannot apply a sandbox layer (its reason is in the log)",
        );
    }
    let events = std::fs::read_to_string(cgroup.path().join("memory.events"))
        .ok()
        .and_then(|text| memory_events(&text));
    if exit.signal == Some(libc::SIGKILL)
        && matches!(events, Some(e) if e.oom > 0 && e.oom_kill > 0)
    {
        exit.memory_limit = true;
        exit.description = format!(
            "was stopped at its {} MiB memory limit",
            memory / (1024 * 1024)
        );
    }
    drop(cgroup);
    exit
}

/// The running kernel's Landlock ABI, 0 when it has none or it is disabled.
fn landlock_abi() -> i64 {
    const LANDLOCK_CREATE_RULESET_VERSION: u32 = 1;
    // SAFETY: the documented version query: no ruleset, size 0, the VERSION flag.
    unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            std::ptr::null::<u8>(),
            0usize,
            LANDLOCK_CREATE_RULESET_VERSION,
        )
    }
    .max(0)
}

/// Two counters from a cgroup's `memory.events`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MemoryEvents {
    /// Times the cgroup's usage reached `memory.max`.
    oom: u64,
    /// Processes in it killed by any OOM killer, the system's included.
    oom_kill: u64,
}

fn memory_events(events: &str) -> Option<MemoryEvents> {
    let count = |key: &str| {
        events
            .lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix(' ')?.trim().parse().ok())
    };
    Some(MemoryEvents {
        oom: count("oom")?,
        oom_kill: count("oom_kill")?,
    })
}

/// A cgroup directory this host created, removed when dropped. Removal fails
/// while a process is still in it, which only a sidecar that outlived its
/// supervisor could be.
#[derive(Debug)]
struct Created(PathBuf);

impl Created {
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Created {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.0);
    }
}

fn cgroup_in(root: &Path, limits: &Limits) -> io::Result<Created> {
    cgroup_with(root, limits, |path, value| std::fs::write(path, value))
}

/// `cgroup_in`, with the writer for the limit files passed in, so a test can
/// refuse one: only a real cgroup filesystem refuses such a write.
fn cgroup_with(
    root: &Path,
    limits: &Limits,
    write: impl Fn(&Path, &str) -> io::Result<()>,
) -> io::Result<Created> {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let dir = root.join(format!(
        "srelens-sidecar-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&dir).map_err(|e| {
        io::Error::other(format!(
            "srelens cannot create the app's cgroup {} ({e}), so it does not run executable apps. \
             Executable apps need a writable, delegated cgroup v2 directory with the memory and cpu \
             controllers enabled for its children",
            dir.display()
        ))
    })?;
    // From here on, a limit that cannot be set removes the directory again.
    let created = Created(dir.clone());
    let set = |file: &str, value: String| {
        write(&dir.join(file), &value).map_err(|e| {
            io::Error::other(format!(
                "srelens cannot set {}/{file} to {value} ({e}), so it does not run executable apps",
                dir.display()
            ))
        })
    };
    set("memory.max", limits.memory_bytes.to_string())?;
    set("memory.swap.max", "0".into())?;
    let period = 100_000u64;
    let quota = ((limits.cpus * period as f64) as u64).max(1_000);
    set("cpu.max", format!("{quota} {period}"))?;
    Ok(created)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("a temporary directory")
    }

    #[test]
    fn a_cgroup_gets_the_memory_and_cpu_limits() {
        let root = fresh_dir();
        let limits = Limits {
            cpus: 0.5,
            ..Limits::default()
        };
        let created = cgroup_in(root.path(), &limits).expect("created");
        let read = |file: &str| std::fs::read_to_string(created.path().join(file)).unwrap();
        assert_eq!(read("memory.max"), "268435456");
        assert_eq!(read("memory.swap.max"), "0");
        assert_eq!(read("cpu.max"), "50000 100000");
    }

    #[test]
    fn a_limit_that_cannot_be_set_removes_the_cgroup_and_names_the_file() {
        let root = fresh_dir();
        let refuse_cpu = |path: &Path, _: &str| {
            if path.ends_with("cpu.max") {
                Err(io::Error::other("refused"))
            } else {
                Ok(())
            }
        };
        let err = cgroup_with(root.path(), &Limits::default(), refuse_cpu).expect_err("refused");
        assert!(err.to_string().contains("cpu.max"), "{err}");
        let left: Vec<_> = std::fs::read_dir(root.path()).unwrap().collect();
        assert!(left.is_empty(), "left behind: {left:?}");
    }

    #[test]
    fn a_dropped_cgroup_is_removed() {
        let root = fresh_dir();
        let created = cgroup_in(root.path(), &Limits::default()).unwrap();
        let path = created.path().to_owned();
        // A real cgroup directory has only interface files, which rmdir
        // ignores; a plain one has to be emptied first.
        for file in ["memory.max", "memory.swap.max", "cpu.max"] {
            std::fs::remove_file(path.join(file)).unwrap();
        }
        drop(created);
        assert!(!path.exists());
    }

    #[test]
    fn an_unusable_cgroup_root_says_what_executable_apps_need() {
        let err = cgroup_in(
            Path::new("/nonexistent-srelens-cgroup-root"),
            &Limits::default(),
        )
        .expect_err("no cgroup under a missing root");
        let message = err.to_string();
        for needle in [
            "/nonexistent-srelens-cgroup-root",
            "delegated",
            "memory and cpu",
        ] {
            assert!(
                message.contains(needle),
                "{needle:?} missing from: {message}"
            );
        }
    }

    #[test]
    fn memory_use_is_read_from_memory_current_until_the_cgroup_is_gone() {
        let dir = fresh_dir();
        let current = dir.path().join("memory.current");
        std::fs::write(&current, "4194304\n").unwrap();
        assert_eq!(memory_current(&current), Some(4_194_304));
        std::fs::remove_file(&current).unwrap();
        assert_eq!(memory_current(&current), None);
    }

    #[test]
    fn the_oom_counts_are_read_from_memory_events() {
        let events = "low 0\nhigh 0\nmax 12\noom 1\noom_kill 1\noom_group_kill 0\n";
        assert_eq!(
            memory_events(events),
            Some(MemoryEvents {
                oom: 1,
                oom_kill: 1
            })
        );
        assert_eq!(memory_events(""), None);
        assert_eq!(memory_events("oom 1\n"), None);
        assert_eq!(memory_events("oom_group_kill 3\noom_kill 1\n"), None);
    }

    fn exited_by(signal: i32) -> io::Result<std::process::ExitStatus> {
        use std::os::unix::process::ExitStatusExt;
        Ok(std::process::ExitStatus::from_raw(signal))
    }

    #[test]
    fn a_sigkill_with_the_cgroups_oom_counts_is_the_memory_limit() {
        let root = fresh_dir();
        let created = Created(root.path().join("leaf"));
        std::fs::create_dir(created.path()).unwrap();
        std::fs::write(created.path().join("memory.events"), "oom 1\noom_kill 1\n").unwrap();
        let exit = describe(exited_by(libc::SIGKILL), created, 256 * 1024 * 1024);
        assert!(exit.memory_limit);
        assert_eq!(exit.description, "was stopped at its 256 MiB memory limit");
    }

    #[test]
    fn a_sigkill_without_an_oom_in_the_cgroup_is_not_the_memory_limit() {
        for events in ["oom 0\noom_kill 1\n", "oom 0\noom_kill 0\n", ""] {
            let root = fresh_dir();
            let created = Created(root.path().join("leaf"));
            std::fs::create_dir(created.path()).unwrap();
            std::fs::write(created.path().join("memory.events"), events).unwrap();
            let exit = describe(exited_by(libc::SIGKILL), created, 1 << 28);
            assert!(!exit.memory_limit, "{events:?}");
            assert!(exit.description.contains("SIGKILL"), "{}", exit.description);
        }
    }

    #[test]
    fn without_a_launcher_or_a_cgroup_root_the_app_is_refused() {
        let command = SidecarCommand {
            app_id: "org.example.a".into(),
            program: "/bin/true".into(),
            args: Vec::new(),
            env: Vec::new(),
            data_dir: std::env::temp_dir(),
        };
        let refused = |config: SandboxConfig| match launch(&config, &command, &Limits::default()) {
            Err(LaunchError::Unavailable(why)) => why,
            Err(other) => panic!("{other:?}"),
            Ok(_) => panic!("launched"),
        };
        assert!(refused(SandboxConfig::default()).contains("sandbox launcher"));
        // With a launcher but no cgroup root: refused for the cgroup, or for
        // Landlock on a kernel without it. Never started unconfined.
        let why = refused(SandboxConfig {
            launcher: Some("/nonexistent/srelens-sandbox-launch".into()),
            cgroup_root: None,
        });
        assert!(why.contains("cgroup") || why.contains("Landlock"), "{why}");
    }
}
