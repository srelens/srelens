//! Linux backends. The layered ones start `sandbox-launch` (see `src/bin/sandbox-launch.rs`)
//! with the layers they want; `bwrap` starts bubblewrap directly.
//!
//! cgroups: the host needs a writable cgroup v2 directory with the `memory` and `cpu`
//! controllers enabled for its children. `SPIKE_CGROUP_ROOT` names it (default
//! `/sys/fs/cgroup`, which is writable in a `--privileged` container after `run-linux.sh`
//! has moved the container's processes into a leaf). Each sidecar gets its own child
//! directory with `memory.max`, `memory.swap.max = 0` and `cpu.max`.

use crate::{built_binary, Fixture, Limits};
use std::io;
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
    pub fn ended(&mut self) -> String {
        let status = match self.child.wait() {
            Ok(status) => format!("exited: {status}"),
            Err(e) => format!("wait failed: {e}"),
        };
        match &self.cgroup {
            Some(dir) => {
                let events = std::fs::read_to_string(dir.join("memory.events")).unwrap_or_default();
                let oom = events.lines().find(|l| l.starts_with("oom_kill ")).unwrap_or("oom_kill ?");
                format!("{status}; cgroup memory.events {oom}")
            }
            None => status,
        }
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

fn cgroup(limits: &Limits) -> io::Result<PathBuf> {
    let root = PathBuf::from(std::env::var("SPIKE_CGROUP_ROOT").unwrap_or("/sys/fs/cgroup".into()));
    cgroup_in(&root, limits)
}

fn cgroup_in(root: &std::path::Path, limits: &Limits) -> io::Result<PathBuf> {
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
    let set = |file: &str, value: String| {
        std::fs::write(dir.join(file), &value)
            .map_err(|e| io::Error::other(format!("{}/{file} = {value}: {e}", dir.display())))
    };
    set("memory.max", (limits.memory_mib * 1024 * 1024).to_string())?;
    set("memory.swap.max", "0".into())?;
    let period = 100_000u64;
    set("cpu.max", format!("{} {period}", (limits.cpus * period as f64) as u64))?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::cgroup_in;
    use crate::Limits;
    use std::path::Path;

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
        cmd.arg("--cgroup").arg(dir);
    }
    if layers.landlock {
        cmd.arg("--landlock").arg(fixture.scratch());
    }
    if layers.seccomp {
        cmd.arg("--seccomp");
    }
    cmd.arg("--").arg(fixture.probe());
    let child = spawn(cmd)?;
    Ok(Confined { child, cgroup })
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
