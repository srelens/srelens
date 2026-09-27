//! `srelens-sandbox-launch`, the trusted launcher (Linux and macOS). The host
//! starts it with the sidecar's command line; it confines itself and then
//! `exec`s the sidecar, which inherits every layer.
//!
//! ```text
//! srelens-sandbox-launch --cgroup DIR --data DIR -- PROGRAM [ARGS...]   Linux
//! srelens-sandbox-launch --data DIR -- PROGRAM [ARGS...]                macOS
//! ```
//!
//! It is srelens's own code, so it runs unconfined until it has applied the
//! layers, in this order:
//!
//! 1. **Every inherited descriptor above stderr is closed**, so nothing the
//!    host held open without close-on-exec reaches the sidecar.
//! 2. Linux: **it joins the cgroup** the host created with the limits. This
//!    has to come before Landlock, which would refuse the write.
//! 3. Linux: **Landlock**, targeting ABI 5 as the spike did (the filesystem
//!    rights of ABIs 1 to 5, and TCP bind and connect from ABI 4), plus the
//!    ABI 6 scopes (abstract Unix sockets, signals to processes outside the
//!    sandbox). Best effort, so an older kernel enforces the subset it knows,
//!    but a kernel that enforces none of it is refused. It grants the data
//!    directory, read and execute on the program, reads under `/usr`, `/lib`
//!    and `/lib64` for the dynamic loader and libc, and the few `/etc` files
//!    libc's resolver opens.
//! 4. Linux: **the seccomp filter**: `socket` of any family fails with
//!    `EPERM` (the spike allowed `AF_UNIX`; its stdio is pipes, so nothing
//!    needs one, and the ADR names the D-Bus session bus as what one reaches),
//!    as do `io_uring_setup`, `fork`, `vfork` and `clone` without
//!    `CLONE_THREAD`; `clone3` fails with `ENOSYS`, so libc falls back to an
//!    inspectable `clone`. It is a deny-list, which the ADR says a production
//!    filter should not be; an allow-list is the escape-hardening review's.
//! 5. macOS: **`exec` of `/usr/bin/sandbox-exec`** with the Seatbelt profile
//!    (`seatbelt.sb`) and the program and data directory as its parameters.
//!
//! Any failure exits with [`LAYER_FAILED`] before the sidecar runs.

use std::convert::Infallible;
use std::ffi::OsString;
use std::path::PathBuf;

/// The exit status when a layer could not be applied. The sidecar never ran.
pub const LAYER_FAILED: i32 = 125;

/// The Seatbelt profile, in its own file so it can be read and reviewed as
/// SBPL. The spike's `src/seatbelt.sb`, with its parameters renamed.
#[cfg(target_os = "macos")]
pub const SEATBELT_PROFILE: &str = include_str!("seatbelt.sb");

#[derive(Debug, PartialEq, Eq)]
struct Args {
    cgroup: Option<PathBuf>,
    data: PathBuf,
    program: Vec<OsString>,
}

fn parse(args: Vec<OsString>) -> Result<Args, String> {
    let mut args = args.into_iter();
    let mut cgroup = None;
    let mut data = None;
    let mut program = Vec::new();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--cgroup") => {
                cgroup = Some(args.next().ok_or("--cgroup needs a directory")?.into())
            }
            Some("--data") => data = Some(args.next().ok_or("--data needs a directory")?.into()),
            Some("--") => {
                program = args.by_ref().collect();
                break;
            }
            _ => return Err(format!("unknown argument {}", arg.to_string_lossy())),
        }
    }
    let data: PathBuf = data.ok_or("missing --data DIR")?;
    if program.is_empty() {
        return Err("missing -- PROGRAM".into());
    }
    if cfg!(target_os = "linux") && cgroup.is_none() {
        return Err("missing --cgroup DIR: every Linux layer is required".into());
    }
    Ok(Args {
        cgroup,
        data,
        program,
    })
}

/// The launcher's `main`. Never returns: it becomes the sidecar or exits.
pub fn main() -> ! {
    let Err(why) = run(std::env::args_os().skip(1).collect());
    eprintln!("srelens-sandbox-launch: {why}");
    std::process::exit(LAYER_FAILED);
}

fn run(args: Vec<OsString>) -> Result<Infallible, String> {
    let args = parse(args)?;
    close_inherited_descriptors()?;
    #[cfg(target_os = "linux")]
    {
        linux::join_cgroup(args.cgroup.as_deref().expect("parse requires it"))?;
        linux::apply_landlock(&args.data, std::path::Path::new(&args.program[0]))?;
        linux::apply_seccomp()?;
        exec(&args.program[0], &args.program[1..])
    }
    #[cfg(target_os = "macos")]
    {
        let _ = &args.cgroup;
        let argv = macos::sandbox_exec_args(&args.data, &args.program);
        exec(&OsString::from("/usr/bin/sandbox-exec"), &argv)
    }
}

fn exec(program: &OsString, args: &[OsString]) -> Result<Infallible, String> {
    use std::os::unix::process::CommandExt;
    let mut command = std::process::Command::new(program);
    command.args(args);
    // This binary links CoreFoundation (through the library's dependencies),
    // which sets this variable in its own environment when it loads, so the
    // sidecar would get one more variable than the host gave. The host clears
    // the environment and names every variable the sidecar gets.
    #[cfg(target_os = "macos")]
    command.env_remove("__CF_USER_TEXT_ENCODING");
    let error = command.exec();
    Err(format!(
        "could not start {}: {error}",
        program.to_string_lossy()
    ))
}

/// Close every descriptor above 2. The ones Rust opens are close-on-exec
/// already; this is for any the host process inherited or opened without it.
#[cfg(target_os = "linux")]
fn close_inherited_descriptors() -> Result<(), String> {
    // close_range(2) is Linux 5.9; Landlock, which is required, is 5.13.
    // SAFETY: a plain syscall on this process's own descriptor table.
    let rc = unsafe { libc::syscall(libc::SYS_close_range, 3u32, u32::MAX, 0u32) };
    if rc != 0 {
        return Err(format!(
            "could not close inherited descriptors: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn close_inherited_descriptors() -> Result<(), String> {
    // /dev/fd lists this process's open descriptors, the directory's own
    // among them, which closing after the listing is harmless.
    let open: Vec<i32> = std::fs::read_dir("/dev/fd")
        .map_err(|e| format!("could not list inherited descriptors: {e}"))?
        .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok())
        .filter(|fd| *fd > 2)
        .collect();
    for fd in open {
        // SAFETY: closing a descriptor number; one already closed is EBADF.
        unsafe { libc::close(fd) };
    }
    Ok(())
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::OsString;
    use std::path::Path;

    /// `sandbox-exec -p PROFILE -D PROGRAM=… -D DATA=… PROGRAM ARGS…`. The host
    /// passes canonical paths: Seatbelt matches resolved paths, and `/tmp` and
    /// `/var` are symlinks on macOS.
    pub(super) fn sandbox_exec_args(data: &Path, program: &[OsString]) -> Vec<OsString> {
        let define = |key: &str, value: &std::ffi::OsStr| {
            let mut d = OsString::from(format!("{key}="));
            d.push(value);
            d
        };
        let mut argv: Vec<OsString> = vec![
            "-p".into(),
            super::SEATBELT_PROFILE.into(),
            "-D".into(),
            define("PROGRAM", &program[0]),
            "-D".into(),
            define("DATA", data.as_os_str()),
        ];
        argv.extend(program.iter().cloned());
        argv
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use landlock::{
        Access, AccessFs, AccessNet, CompatLevel, Compatible, PathBeneath, PathFd, Ruleset,
        RulesetAttr, RulesetCreated, RulesetCreatedAttr, RulesetStatus, Scope, ABI,
    };
    use seccompiler::{
        apply_filter, BpfProgram, SeccompAction, SeccompCmpArgLen, SeccompCmpOp, SeccompCondition,
        SeccompFilter, SeccompRule,
    };
    use std::collections::BTreeMap;
    use std::path::Path;

    pub(super) fn join_cgroup(dir: &Path) -> Result<(), String> {
        std::fs::write(dir.join("cgroup.procs"), std::process::id().to_string())
            .map_err(|e| format!("could not join cgroup {}: {e}", dir.display()))
    }

    /// Files libc's resolver opens. DNS is refused by the filter, not by a
    /// missing file, so the refusal is the network layer's.
    const RESOLVER_FILES: &[&str] = &[
        "/etc/ld.so.cache",
        "/etc/resolv.conf",
        "/etc/nsswitch.conf",
        "/etc/hosts",
        "/etc/host.conf",
        "/etc/gai.conf",
    ];

    /// The ruleset, created but not yet applied, so it can be built and
    /// checked in a test without confining the test.
    pub(super) fn ruleset(data: &Path, program: &Path) -> Result<RulesetCreated, String> {
        let abi = ABI::V5;
        let fail = |what: &str, e: &dyn std::fmt::Display| format!("Landlock: {what}: {e}");
        let open =
            |path: &Path| PathFd::new(path).map_err(|e| fail(&path.display().to_string(), &e));
        let read = AccessFs::from_read(abi);
        let mut ruleset = Ruleset::default()
            .set_compatibility(CompatLevel::BestEffort)
            .handle_access(AccessFs::from_all(abi))
            .map_err(|e| fail("filesystem rights", &e))?
            .handle_access(AccessNet::from_all(abi))
            .map_err(|e| fail("network rights", &e))?
            .scope(Scope::from_all(ABI::V6))
            .map_err(|e| fail("scopes", &e))?
            .create()
            .map_err(|e| fail("create", &e))?
            .add_rule(PathBeneath::new(open(data)?, AccessFs::from_all(abi)))
            .map_err(|e| fail("data directory", &e))?
            .add_rule(PathBeneath::new(
                open(program)?,
                AccessFs::Execute | AccessFs::ReadFile,
            ))
            .map_err(|e| fail("program", &e))?;
        for dir in ["/usr", "/lib", "/lib64"] {
            if let Ok(fd) = PathFd::new(dir) {
                ruleset = ruleset
                    .add_rule(PathBeneath::new(fd, read))
                    .map_err(|e| fail(dir, &e))?;
            }
        }
        for file in RESOLVER_FILES {
            if let Ok(fd) = PathFd::new(file) {
                ruleset = ruleset
                    .add_rule(PathBeneath::new(fd, AccessFs::ReadFile))
                    .map_err(|e| fail(file, &e))?;
            }
        }
        Ok(ruleset)
    }

    pub(super) fn apply_landlock(data: &Path, program: &Path) -> Result<(), String> {
        let status = ruleset(data, program)?
            .restrict_self()
            .map_err(|e| format!("Landlock: restrict: {e}"))?;
        if status.ruleset == RulesetStatus::NotEnforced {
            return Err("Landlock is not enforced on this kernel".into());
        }
        Ok(())
    }

    /// The two filters: `clone3` answered with `ENOSYS`, then the deny-list.
    pub(super) fn filters() -> Result<(BpfProgram, BpfProgram), String> {
        let fail = |e: &dyn std::fmt::Display| format!("seccomp: {e}");
        let always = Vec::<SeccompRule>::new;
        let mut eperm: BTreeMap<i64, Vec<SeccompRule>> = BTreeMap::new();
        eperm.insert(libc::SYS_socket, always());
        eperm.insert(libc::SYS_io_uring_setup, always());
        let without_thread = SeccompCondition::new(
            0,
            SeccompCmpArgLen::Qword,
            SeccompCmpOp::MaskedEq(libc::CLONE_THREAD as u64),
            0,
        )
        .map_err(|e| fail(&e))?;
        eperm.insert(
            libc::SYS_clone,
            vec![SeccompRule::new(vec![without_thread]).map_err(|e| fail(&e))?],
        );
        #[cfg(target_arch = "x86_64")]
        {
            eperm.insert(libc::SYS_fork, always());
            eperm.insert(libc::SYS_vfork, always());
        }
        let mut enosys: BTreeMap<i64, Vec<SeccompRule>> = BTreeMap::new();
        enosys.insert(libc::SYS_clone3, always());
        let arch: seccompiler::TargetArch = std::env::consts::ARCH
            .try_into()
            .map_err(|e: seccompiler::BackendError| fail(&e))?;
        let filter = |rules, errno: i32| -> Result<BpfProgram, String> {
            SeccompFilter::new(
                rules,
                SeccompAction::Allow,
                SeccompAction::Errno(errno as u32),
                arch,
            )
            .map_err(|e| fail(&e))?
            .try_into()
            .map_err(|e: seccompiler::BackendError| fail(&e))
        };
        Ok((filter(enosys, libc::ENOSYS)?, filter(eperm, libc::EPERM)?))
    }

    pub(super) fn apply_seccomp() -> Result<(), String> {
        let (no_clone3, deny) = filters()?;
        // SAFETY: prctl with constant arguments. Landlock has set it already;
        // the filters need it whether or not it did.
        if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
            return Err(format!("no_new_privs: {}", std::io::Error::last_os_error()));
        }
        apply_filter(&no_clone3).map_err(|e| format!("seccomp: {e}"))?;
        apply_filter(&deny).map_err(|e| format!("seccomp: {e}"))?;
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn the_ruleset_builds_for_a_data_directory_and_a_program() {
            let data = tempfile::tempdir().unwrap();
            let program = std::env::current_exe().unwrap();
            // Created, never applied: this test process stays unconfined.
            ruleset(data.path(), &program).expect("the ruleset builds");
        }

        #[test]
        fn a_missing_data_directory_is_an_error_not_a_narrower_ruleset() {
            let program = std::env::current_exe().unwrap();
            let err = ruleset(Path::new("/nonexistent-srelens-data"), &program).err();
            assert!(err.is_some_and(|e| e.contains("/nonexistent-srelens-data")));
        }

        #[test]
        fn the_filters_compile_for_this_architecture() {
            let (no_clone3, deny) = filters().expect("the filters compile");
            assert!(!no_clone3.is_empty() && !deny.is_empty());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn the_command_line_names_the_data_directory_and_the_program() {
        let parsed = parse(args(&[
            "--cgroup", "/cg", "--data", "/d", "--", "/bin/x", "--flag",
        ]))
        .expect("parses");
        assert_eq!(parsed.cgroup, Some(PathBuf::from("/cg")));
        assert_eq!(parsed.data, PathBuf::from("/d"));
        assert_eq!(parsed.program, args(&["/bin/x", "--flag"]));
    }

    #[test]
    fn a_command_line_without_what_the_launcher_needs_is_refused() {
        for (list, says) in [
            (&["--cgroup", "/cg", "--", "/bin/x"][..], "--data"),
            (&["--cgroup", "/cg", "--data", "/d"][..], "PROGRAM"),
            (&["--cgroup", "/cg", "--data", "/d", "--"][..], "PROGRAM"),
            (&["--data"][..], "--data"),
            (&["--landlock", "/d"][..], "unknown argument --landlock"),
        ] {
            let err = parse(args(list)).expect_err("refused");
            assert!(err.contains(says), "{list:?}: {err}");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn on_linux_the_cgroup_is_required() {
        let err = parse(args(&["--data", "/d", "--", "/bin/x"])).expect_err("refused");
        assert!(err.contains("--cgroup"), "{err}");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn on_macos_sandbox_exec_gets_the_profile_and_both_paths() {
        let argv = macos::sandbox_exec_args(
            std::path::Path::new("/private/var/d"),
            &args(&["/opt/x/bin/scan", "--fast"]),
        );
        assert_eq!(argv[0], "-p");
        assert_eq!(argv[1], SEATBELT_PROFILE);
        assert_eq!(
            argv[2..6],
            args(&["-D", "PROGRAM=/opt/x/bin/scan", "-D", "DATA=/private/var/d"])
        );
        assert_eq!(argv[6..], args(&["/opt/x/bin/scan", "--fast"]));
        assert!(SEATBELT_PROFILE.contains("(deny default)"));
    }
}
