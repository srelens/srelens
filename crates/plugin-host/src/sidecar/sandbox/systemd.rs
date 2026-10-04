//! srelens's own delegated cgroup, on a systemd desktop.
//!
//! A sidecar's cgroup goes under a cgroup v2 directory delegated to srelens,
//! with srelens itself in a leaf of it (`linux.rs`). On a systemd desktop the
//! user manager hands one over: at srelens's first sidecar start, srelens asks
//! it for a transient scope holding itself, `app-srelens-<pid>.scope`, with
//! `Delegate=yes`, moves into the scope's `host/` leaf, and enables `memory`
//! and `cpu` for the scope's children. That is the layout the
//! `sandbox-conformance` CI job builds by hand. Where it cannot, the sidecar is
//! refused with the reason and the way out; it is never started without
//! limits.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

/// The controllers a sidecar's limits need, as `cgroup.controllers` names them.
const CONTROLLERS: [&str; 2] = ["memory", "cpu"];

/// srelens's own leaf in its scope, beside its sidecars' cgroups.
const LEAF: &str = "host";

const NOT_UNIFIED: &str = "srelens needs the unified cgroup v2 hierarchy to limit an app's memory and CPU, so it does not run executable apps on this system";

const NESTED: &str = "srelens runs in a PID namespace of its own, as in a container, where it cannot ask systemd for a cgroup, so it does not run executable apps. Name a delegated cgroup v2 directory in SRELENS_SANDBOX_CGROUP_ROOT";

/// The scope srelens's sidecars' cgroups go in, made at the first call in this
/// process. Later calls answer with the first success. A failure is not kept:
/// the next start tries again, which is safe because every step can be
/// repeated.
pub(super) fn scope_root() -> Result<PathBuf, String> {
    static ROOT: Mutex<Option<PathBuf>> = Mutex::new(None);
    let mut root = ROOT.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(root) = root.as_ref() {
        return Ok(root.clone());
    }
    // A thread of its own: the D-Bus call blocks, and the caller is a tokio task.
    let made = std::thread::spawn(adopt).join().unwrap_or_else(|_| {
        Err(
            "srelens failed while setting up a cgroup for executable apps, so it does not run them"
                .into(),
        )
    })?;
    *root = Some(made.clone());
    Ok(made)
}

/// Every step, from wherever this process is now.
fn adopt() -> Result<PathBuf, String> {
    let status = std::fs::read_to_string("/proc/self/status").map_err(|e| {
        format!("srelens cannot read /proc/self/status ({e}), so it does not run executable apps")
    })?;
    if in_nested_pid_namespace(&status) {
        return Err(NESTED.into());
    }
    let pid = std::process::id();
    let unit = unit_name(pid);
    let now = own_cgroup()?;
    let scope = match scope_of_leaf(&now, &unit) {
        Some(scope) => scope.to_owned(),
        None => {
            start_scope(&unit, pid)?;
            wait_for(&unit)?
        }
    };
    let dir = Path::new("/sys/fs/cgroup").join(scope.trim_start_matches('/'));
    prepare(&dir, |path, value| std::fs::write(path, value))?;
    Ok(dir)
}

/// This process's cgroup path.
fn own_cgroup() -> Result<String, String> {
    let text = std::fs::read_to_string("/proc/self/cgroup").map_err(|e| {
        format!("srelens cannot read /proc/self/cgroup ({e}), so it does not run executable apps")
    })?;
    unified_path(&text).map(str::to_owned)
}

/// Ask the user manager for `unit`, a scope holding this process, delegated.
fn start_scope(unit: &str, pid: u32) -> Result<(), String> {
    use zbus::zvariant::Value;
    let connection = zbus::blocking::connection::Builder::session()
        .and_then(|builder| builder.method_timeout(Duration::from_secs(10)).build())
        .map_err(|e| no_user_manager(&e.to_string()))?;
    let properties: Vec<(&str, Value)> = vec![
        ("PIDs", Value::from(vec![pid])),
        ("Delegate", Value::from(true)),
        ("Description", Value::from("srelens executable apps")),
    ];
    let auxiliary: Vec<(&str, Vec<(&str, Value)>)> = Vec::new();
    connection
        .call_method(
            Some("org.freedesktop.systemd1"),
            "/org/freedesktop/systemd1",
            Some("org.freedesktop.systemd1.Manager"),
            "StartTransientUnit",
            &(unit, "fail", properties, auxiliary),
        )
        .map(drop)
        .map_err(|e| match &e {
            zbus::Error::MethodError(name, ..)
                if name.as_str() == "org.freedesktop.DBus.Error.ServiceUnknown" =>
            {
                no_user_manager(&e.to_string())
            }
            _ => format!(
                "The systemd user manager did not make srelens's scope {unit} ({e}), so srelens does not run executable apps"
            ),
        })
}

/// Wait for systemd to move this process into `unit`, which its job does just
/// after the call returns: every 10 ms, for at most 2 s.
fn wait_for(unit: &str) -> Result<String, String> {
    for _ in 0..200 {
        let now = own_cgroup()?;
        if now.rsplit('/').next() == Some(unit) {
            return Ok(now);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Err(format!(
        "systemd did not move srelens into its scope {unit} within 2 seconds, so srelens does not run executable apps"
    ))
}

/// The refusal where no systemd user manager answers.
fn no_user_manager(error: &str) -> String {
    format!(
        "srelens could not ask the systemd user manager for a cgroup ({error}), so it does not run executable apps. They need a systemd desktop session, or a delegated cgroup v2 directory named in SRELENS_SANDBOX_CGROUP_ROOT"
    )
}

/// The scope srelens asks for, named by the XDG convention for an app's scope.
fn unit_name(pid: u32) -> String {
    format!("app-srelens-{pid}.scope")
}

/// The cgroup path in `/proc/self/cgroup`, when this process is in the unified
/// v2 hierarchy alone: one line, `0::<path>`.
fn unified_path(proc_cgroup: &str) -> Result<&str, String> {
    let mut lines = proc_cgroup.lines().filter(|line| !line.is_empty());
    match (
        lines.next().and_then(|line| line.strip_prefix("0::")),
        lines.next(),
    ) {
        (Some(path), None) if path.starts_with('/') => Ok(path),
        _ => Err(NOT_UNIFIED.into()),
    }
}

/// Whether `/proc/self/status` puts this process in a nested PID namespace,
/// where the PID it would send systemd names some other process.
fn in_nested_pid_namespace(status: &str) -> bool {
    status
        .lines()
        .find_map(|line| line.strip_prefix("NSpid:"))
        .is_some_and(|ids| ids.split_whitespace().count() > 1)
}

/// The scope's path when `path` is already its leaf: an earlier start made it,
/// or failed after the move.
fn scope_of_leaf<'a>(path: &'a str, unit: &str) -> Option<&'a str> {
    let scope = path.strip_suffix(LEAF)?.strip_suffix('/')?;
    scope
        .rsplit('/')
        .next()
        .is_some_and(|name| name == unit)
        .then_some(scope)
}

/// Make `scope` a root sidecars' cgroups can go under: check that it has the
/// controllers their limits need, move every process in it into its leaf —
/// srelens, and any child it started since systemd moved it, which would
/// otherwise make the next write fail with `EBUSY` — then enable the
/// controllers for its children. Each step can be repeated. `write` is the
/// cgroup filesystem's writer, passed in so a test can stand in for it.
fn prepare(scope: &Path, write: impl Fn(&Path, &str) -> io::Result<()>) -> Result<(), String> {
    let read = |file: &str| {
        std::fs::read_to_string(scope.join(file)).map_err(|e| {
            format!(
                "srelens cannot read {}/{file} ({e}), so it does not run executable apps",
                scope.display()
            )
        })
    };
    let controllers = read("cgroup.controllers")?;
    let missing: Vec<&str> = CONTROLLERS
        .into_iter()
        .filter(|wanted| !controllers.split_whitespace().any(|have| have == *wanted))
        .collect();
    if !missing.is_empty() {
        return Err(missing_controllers(&missing));
    }
    let leaf = scope.join(LEAF);
    match std::fs::create_dir(&leaf) {
        Err(e) if e.kind() != io::ErrorKind::AlreadyExists => {
            return Err(format!(
                "srelens cannot create its cgroup {} ({e}), so it does not run executable apps",
                leaf.display()
            ))
        }
        _ => {}
    }
    for pid in read("cgroup.procs")?.split_whitespace() {
        match write(&leaf.join("cgroup.procs"), pid) {
            // A process that ended since the list was read.
            Err(e) if e.raw_os_error() == Some(libc::ESRCH) => {}
            Err(e) => {
                return Err(format!(
                    "srelens cannot move process {pid} into its cgroup {} ({e}), so it does not run executable apps",
                    leaf.display()
                ))
            }
            Ok(()) => {}
        }
    }
    let enable = "+memory +cpu";
    write(&scope.join("cgroup.subtree_control"), enable).map_err(|e| {
        format!(
            "srelens cannot set {}/cgroup.subtree_control to {enable} ({e}), so it does not run executable apps",
            scope.display()
        )
    })
}

/// The refusal when the user manager does not delegate a controller the limits
/// need, with the drop-in that delegates it.
fn missing_controllers(missing: &[&str]) -> String {
    let (names, plural) = match missing {
        [one] => ((*one).to_owned(), ""),
        _ => (missing.join(" and "), "s"),
    };
    format!(
        "The systemd user manager does not delegate the {names} controller{plural} to this session, so srelens cannot limit an app's memory and CPU and does not run executable apps. systemd before 252, and the RHEL 9 family, do not delegate cpu to user sessions. To delegate it, run `sudo mkdir -p /etc/systemd/system/user@.service.d`, write `[Service]` and `Delegate=pids memory cpu` on two lines to /etc/systemd/system/user@.service.d/delegate.conf, run `sudo systemctl daemon-reload`, then log out and in again"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn the_unit_is_named_for_the_app_and_the_process() {
        assert_eq!(unit_name(4242), "app-srelens-4242.scope");
    }

    #[test]
    fn only_the_unified_hierarchy_has_a_path() {
        assert_eq!(
            unified_path("0::/user.slice/a.scope\n"),
            Ok("/user.slice/a.scope")
        );
        for text in [
            "",
            "12:pids:/user.slice\n0::/user.slice\n",
            "1:name=systemd:/x\n",
        ] {
            assert_eq!(unified_path(text), Err(NOT_UNIFIED.to_owned()), "{text:?}");
        }
    }

    #[test]
    fn a_second_pid_in_nspid_is_a_nested_namespace() {
        assert!(!in_nested_pid_namespace("Name:\tsrelens\nNSpid:\t4242\n"));
        assert!(in_nested_pid_namespace("Name:\tsrelens\nNSpid:\t4242\t7\n"));
        assert!(!in_nested_pid_namespace("Name:\tsrelens\n"));
    }

    #[test]
    fn the_leaf_of_this_processs_scope_is_recognised() {
        let unit = unit_name(7);
        let base = "/user.slice/user-1000.slice/user@1000.service/app.slice";
        assert_eq!(
            scope_of_leaf(&format!("{base}/app-srelens-7.scope/host"), &unit),
            Some(format!("{base}/app-srelens-7.scope").as_str())
        );
        assert_eq!(
            scope_of_leaf(&format!("{base}/app-srelens-7.scope"), &unit),
            None
        );
        assert_eq!(
            scope_of_leaf(&format!("{base}/app-srelens-8.scope/host"), &unit),
            None
        );
        assert_eq!(
            scope_of_leaf(&format!("{base}/app-gnome-srelens-7.scope/host"), &unit),
            None
        );
    }

    /// A scope directory as systemd hands it over: its controllers, and the
    /// processes in it.
    fn scope(controllers: &str, procs: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("cgroup.controllers"), controllers).unwrap();
        std::fs::write(dir.path().join("cgroup.procs"), procs).unwrap();
        dir
    }

    /// Each write, in order, as the cgroup filesystem would receive it.
    type Writes = RefCell<Vec<(PathBuf, String)>>;

    fn record(writes: &Writes, path: &Path, value: &str) -> io::Result<()> {
        writes
            .borrow_mut()
            .push((path.to_owned(), value.to_owned()));
        Ok(())
    }

    #[test]
    fn prepare_moves_every_process_into_the_leaf_then_hands_the_controllers_down() {
        let dir = scope("cpu memory pids\n", "123\n456\n");
        let writes = Writes::default();
        prepare(dir.path(), |path, value| record(&writes, path, value)).expect("prepared");
        let leaf = dir.path().join("host");
        assert!(leaf.is_dir());
        assert_eq!(
            writes.into_inner(),
            vec![
                (leaf.join("cgroup.procs"), "123".to_owned()),
                (leaf.join("cgroup.procs"), "456".to_owned()),
                (
                    dir.path().join("cgroup.subtree_control"),
                    "+memory +cpu".to_owned()
                ),
            ]
        );
    }

    #[test]
    fn prepare_again_finds_its_leaf_and_moves_nothing() {
        let dir = scope("cpu memory pids\n", "");
        std::fs::create_dir(dir.path().join("host")).unwrap();
        let writes = Writes::default();
        prepare(dir.path(), |path, value| record(&writes, path, value)).expect("prepared");
        assert_eq!(
            writes.into_inner(),
            vec![(
                dir.path().join("cgroup.subtree_control"),
                "+memory +cpu".to_owned()
            )]
        );
    }

    #[test]
    fn a_process_that_ended_before_its_move_is_not_a_failure() {
        let dir = scope("cpu memory\n", "123\n456\n");
        let gone = |path: &Path, value: &str| {
            if path.ends_with("host/cgroup.procs") && value == "123" {
                Err(io::Error::from_raw_os_error(libc::ESRCH))
            } else {
                Ok(())
            }
        };
        prepare(dir.path(), gone).expect("prepared");
    }

    #[test]
    fn without_cpu_the_scope_is_refused_with_the_drop_in_that_delegates_it() {
        let dir = scope("memory pids\n", "123\n");
        let writes = Writes::default();
        let why =
            prepare(dir.path(), |path, value| record(&writes, path, value)).expect_err("refused");
        for needle in [
            "cpu controller",
            "systemd before 252",
            "RHEL 9",
            "/etc/systemd/system/user@.service.d/delegate.conf",
            "Delegate=pids memory cpu",
            "daemon-reload",
            "log out and in again",
            "does not run executable apps",
        ] {
            assert!(why.contains(needle), "{needle:?} missing from: {why}");
        }
        assert!(writes.into_inner().is_empty(), "wrote before refusing");
        assert!(
            !dir.path().join("host").exists(),
            "made a leaf before refusing"
        );
    }

    #[test]
    fn without_memory_or_cpu_both_are_named() {
        let dir = scope("pids\n", "");
        let why = prepare(dir.path(), |_, _| Ok(())).expect_err("refused");
        assert!(why.contains("memory and cpu controllers"), "{why}");
    }

    #[test]
    fn without_a_user_manager_the_refusal_names_the_way_out() {
        let why = no_user_manager("No such file or directory");
        assert!(why.contains("No such file or directory"), "{why}");
        assert!(why.contains("SRELENS_SANDBOX_CGROUP_ROOT"), "{why}");
        assert!(why.contains("does not run executable apps"), "{why}");
    }

    /// The real user manager: the test process ends up in its own scope's
    /// leaf, with the controllers handed down. CI runs it in the
    /// `sandbox-conformance` job, which starts the runner's user manager.
    #[test]
    #[ignore = "needs a systemd user session; run by the sandbox-conformance CI job"]
    fn a_running_process_gets_a_delegated_scope() {
        let root = scope_root().expect("a delegated scope");
        let unit = unit_name(std::process::id());
        assert!(root.ends_with(&unit), "{}", root.display());
        let now = own_cgroup().expect("in the unified hierarchy");
        assert_eq!(
            scope_of_leaf(&now, &unit),
            root.to_str().unwrap().strip_prefix("/sys/fs/cgroup"),
            "{now}"
        );
        let enabled = std::fs::read_to_string(root.join("cgroup.subtree_control")).unwrap();
        for controller in ["memory", "cpu"] {
            assert!(
                enabled.split_whitespace().any(|c| c == controller),
                "{enabled}"
            );
        }
        assert_eq!(
            scope_root().expect("the same scope"),
            root,
            "a second call answers with the first"
        );
    }

    #[test]
    fn a_leaf_that_cannot_be_entered_names_the_process_and_the_leaf() {
        let dir = scope("cpu memory\n", "123\n");
        let refuse = |_: &Path, _: &str| Err(io::Error::from_raw_os_error(libc::EACCES));
        let why = prepare(dir.path(), refuse).expect_err("refused");
        assert!(why.contains("123") && why.contains("host"), "{why}");
    }
}
