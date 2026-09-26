//! Linux backends. The layered ones start `sandbox-launch` (see `src/bin/sandbox-launch.rs`)
//! with the layers they want; `bwrap` starts bubblewrap directly.
//!
//! cgroups: the host needs a writable cgroup v2 directory with the `memory` and `cpu`
//! controllers enabled for its children. `SPIKE_CGROUP_ROOT` names it (default
//! `/sys/fs/cgroup`, which is writable in a `--privileged` container after `run-linux.sh`
//! has moved the container's processes into a leaf). Each sidecar gets its own child
//! directory with `memory.max`, `memory.swap.max = 0` and `cpu.max`.

use crate::{built_binary, Ended, Fixture, Limits, MemoryEvents};
use std::io;
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

#[derive(Clone, Copy, Debug)]
pub struct Layers {
    pub landlock: bool,
    pub seccomp: bool,
    pub cgroup: bool,
}

pub struct Confined {
    pub child: Child,
    cgroup: Option<PathBuf>,
}

impl Confined {
    pub fn ended(&mut self) -> Ended {
        let (status, signal) = match self.child.wait() {
            Ok(status) => (format!("exited: {status}"), status.signal()),
            Err(e) => (format!("wait failed: {e}"), None),
        };
        let (text, counts) = match &self.cgroup {
            Some(dir) => match std::fs::read_to_string(dir.join("memory.events")) {
                Ok(events) => match memory_events(&events) {
                    Some(e) => {
                        let note = format!("oom {}, oom_kill {}", e.oom, e.oom_kill);
                        (format!("{status}; cgroup memory.events {note}"), Some(e))
                    }
                    None => (format!("{status}; cgroup memory.events has no oom counts"), None),
                },
                Err(e) => (format!("{status}; cgroup memory.events unreadable: {e}"), None),
            },
            None => (status, None),
        };
        Ended { text, signal, memory_events: counts }
    }
}

impl Drop for Confined {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(dir) = &self.cgroup {
            let _ = std::fs::remove_dir(dir);
        }
    }
}

/// A cgroup directory this host created. It is removed on drop unless kept, so a launch
/// that fails after creating it leaves nothing behind.
#[derive(Debug)]
struct Created(Option<PathBuf>);

impl Created {
    fn path(&self) -> &std::path::Path {
        self.0.as_deref().expect("not yet kept")
    }

    fn keep(mut self) -> PathBuf {
        self.0.take().expect("kept once")
    }
}

impl Drop for Created {
    fn drop(&mut self) {
        if let Some(dir) = &self.0 {
            let _ = std::fs::remove_dir(dir);
        }
    }
}

/// The `oom` and `oom_kill` counts in a cgroup's `memory.events`, if both are readable.
fn memory_events(events: &str) -> Option<MemoryEvents> {
    let count = |key: &str| {
        events.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix(' ')?.trim().parse().ok())
    };
    Some(MemoryEvents { oom: count("oom")?, oom_kill: count("oom_kill")? })
}

fn cgroup(limits: &Limits) -> io::Result<Created> {
    let root = PathBuf::from(std::env::var("SPIKE_CGROUP_ROOT").unwrap_or("/sys/fs/cgroup".into()));
    cgroup_in(&root, limits)
}

fn cgroup_in(root: &std::path::Path, limits: &Limits) -> io::Result<Created> {
    cgroup_with(root, limits, |path, value| std::fs::write(path, value))
}

/// `cgroup_in`, with the writer for the limit files passed in, so a test can refuse one:
/// only a real cgroup filesystem refuses such a write, and a unit test has none.
fn cgroup_with(
    root: &std::path::Path,
    limits: &Limits,
    write: impl Fn(&std::path::Path, &str) -> io::Result<()>,
) -> io::Result<Created> {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let dir = root.join(format!(
        "srelens-sidecar-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&dir).map_err(|e| {
        io::Error::other(format!(
            "the cgroup layer cannot create {} ({e}). The cgroup backends need a writable, \
             delegated cgroup v2 directory with the memory and cpu controllers enabled for its \
             children; an ordinary user has none under /sys/fs/cgroup. Set SPIKE_CGROUP_ROOT to \
             one (for example a systemd scope started with Delegate=yes), or run run-linux.sh \
             in Docker",
            dir.display()
        ))
    })?;
    // From here on, a limit that cannot be set removes the directory again.
    let created = Created(Some(dir.clone()));
    let set = |file: &str, value: String| {
        write(&dir.join(file), &value)
            .map_err(|e| io::Error::other(format!("{}/{file} = {value}: {e}", dir.display())))
    };
    set("memory.max", (limits.memory_mib * 1024 * 1024).to_string())?;
    set("memory.swap.max", "0".into())?;
    let period = 100_000u64;
    set("cpu.max", format!("{} {period}", (limits.cpus * period as f64) as u64))?;
    Ok(created)
}

#[cfg(test)]
mod tests {
    use super::{cgroup_in, cgroup_with, memory_events, start, Created};
    use crate::{Limits, MemoryEvents};
    use std::io;
    use std::path::Path;
    use std::process::Command;

    fn fresh_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("srelens-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir(&dir);
        std::fs::create_dir(&dir).expect("a temporary directory");
        dir
    }

    #[test]
    fn a_created_cgroup_that_is_not_kept_is_removed() {
        // A launch that fails after creating the cgroup drops it without keeping it.
        let dir = fresh_dir("created-dropped");
        drop(Created(Some(dir.clone())));
        assert!(!dir.exists(), "{} was left behind", dir.display());
    }

    #[test]
    fn a_limit_that_cannot_be_set_removes_the_cgroup() {
        let root = fresh_dir("cgroup-root");
        let refuse_cpu = |path: &Path, _: &str| {
            if path.ends_with("cpu.max") {
                Err(io::Error::other("refused"))
            } else {
                Ok(())
            }
        };
        let err = cgroup_with(&root, &Limits::SPIKE, refuse_cpu).expect_err("cpu.max is refused");
        assert!(err.to_string().contains("cpu.max"), "{err}");
        let left: Vec<_> = std::fs::read_dir(&root).expect("the root").collect();
        assert!(left.is_empty(), "left behind under {}: {left:?}", root.display());
        std::fs::remove_dir(&root).expect("cleanup");
    }

    #[test]
    fn a_launcher_that_cannot_start_removes_the_cgroup() {
        let dir = fresh_dir("cgroup-unstarted");
        let cannot_start = |_: Command| Err(io::Error::other("cannot start"));
        let cgroup = Some(Created(Some(dir.clone())));
        assert!(start(Command::new("sandbox-launch"), cgroup, cannot_start).is_err());
        assert!(!dir.exists(), "{} was left behind", dir.display());
    }

    #[test]
    fn a_kept_cgroup_stays_for_its_sidecar() {
        let dir = fresh_dir("created-kept");
        let kept = Created(Some(dir.clone())).keep();
        assert!(kept.exists());
        std::fs::remove_dir(&kept).expect("cleanup");
    }

    #[test]
    fn the_oom_counts_are_read_from_memory_events() {
        let events = "low 0\nhigh 0\nmax 12\noom 1\noom_kill 1\noom_group_kill 0\n";
        assert_eq!(memory_events(events), Some(MemoryEvents { oom: 1, oom_kill: 1 }));
        let global = "oom 0\noom_kill 1\n";
        assert_eq!(memory_events(global), Some(MemoryEvents { oom: 0, oom_kill: 1 }));
    }

    #[test]
    fn memory_events_without_both_counts_give_none() {
        assert_eq!(memory_events(""), None);
        assert_eq!(memory_events("oom 1\n"), None);
        assert_eq!(memory_events("oom_kill 1\n"), None);
        assert_eq!(memory_events("oom 1\noom_kill many\n"), None);
        // Different counters with similar names.
        assert_eq!(memory_events("oom_group_kill 3\noom_kill 1\n"), None);
    }

    #[test]
    fn an_unusable_cgroup_root_says_what_the_backend_needs() {
        let err = cgroup_in(Path::new("/nonexistent-srelens-cgroup-root"), &Limits::SPIKE)
            .expect_err("no cgroup can be made under a missing root");
        let message = err.to_string();
        for needle in ["/nonexistent-srelens-cgroup-root", "delegated", "SPIKE_CGROUP_ROOT", "run-linux.sh"] {
            assert!(message.contains(needle), "{needle:?} missing from: {message}");
        }
    }
}

pub fn launch_layered(fixture: &Fixture, limits: &Limits, layers: Layers) -> io::Result<Confined> {
    let mut cmd = Command::new(built_binary("sandbox-launch")?);
    let cgroup = if layers.cgroup { Some(cgroup(limits)?) } else { None };
    if let Some(dir) = &cgroup {
        cmd.arg("--cgroup").arg(dir.path());
    }
    if layers.landlock {
        cmd.arg("--landlock").arg(fixture.scratch());
    }
    if layers.seccomp {
        cmd.arg("--seccomp");
    }
    cmd.arg("--").arg(fixture.probe());
    start(cmd, cgroup, spawn)
}

/// Start the launcher that joins `cgroup`. One that cannot start drops `cgroup` here, which
/// removes it; one that starts hands it to `Confined`, which removes it once the sidecar is
/// done. `spawn` is passed in so a test can make the start fail.
fn start(
    cmd: Command,
    cgroup: Option<Created>,
    spawn: impl FnOnce(Command) -> io::Result<Child>,
) -> io::Result<Confined> {
    let child = spawn(cmd)?;
    Ok(Confined { child, cgroup: cgroup.map(Created::keep) })
}

/// bubblewrap with every namespace unshared and only the scratch directory, the probe and
/// `/usr` (read-only) mounted. No cgroup and no seccomp filter: those are what bwrap alone
/// does not provide.
pub fn launch_bwrap(fixture: &Fixture) -> io::Result<Confined> {
    let probe = fixture.probe();
    let scratch = fixture.scratch();
    let mut cmd = Command::new("bwrap");
    cmd.args(["--unshare-all", "--die-with-parent", "--new-session"])
        .args(["--ro-bind", "/usr", "/usr"])
        .args(["--symlink", "usr/lib", "/lib"])
        .args(["--symlink", "usr/lib64", "/lib64"])
        .args(["--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp"])
        .arg("--ro-bind")
        .arg(&probe)
        .arg(&probe)
        .arg("--bind")
        .arg(&scratch)
        .arg(&scratch)
        .arg("--chdir")
        .arg(&scratch)
        .arg(&probe);
    let child = spawn(cmd)?;
    Ok(Confined { child, cgroup: None })
}

fn spawn(mut cmd: Command) -> io::Result<Child> {
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn()
}
