//! Linux launcher: confines itself, then `exec`s the probe, which inherits every layer.
//!
//! ```text
//! sandbox-launch [--cgroup DIR] [--landlock SCRATCH] [--seccomp] -- PROBE
//! ```
//!
//! It is the host's trusted code, so it runs unconfined until it has applied the layers,
//! in this order:
//!
//! 1. `--cgroup DIR`: write its own PID to `DIR/cgroup.procs`. The host created `DIR` with
//!    `memory.max`, `memory.swap.max` and `cpu.max`. This must come before Landlock, which
//!    would refuse the write.
//! 2. `--landlock SCRATCH`: a ruleset that handles every filesystem right the kernel's ABI
//!    knows (and TCP bind/connect when the ABI is 4 or later), granting only: everything
//!    under SCRATCH; read+execute on the probe; read+execute under `/usr` (the dynamic
//!    loader and libc); and read of the few `/etc` files libc's resolver reads.
//! 3. `--seccomp`: a deny-list filter. `socket` for any family but `AF_UNIX`, `io_uring_setup`,
//!    `fork`, `vfork` and `clone` without `CLONE_THREAD` fail with `EPERM`; `clone3` fails
//!    with `ENOSYS`, so libc falls back to `clone`, where the flags can be inspected. Threads
//!    still work; new processes do not. `execve` stays allowed, which is what lets this
//!    launcher become the probe (it replaces the image, it does not create a process).

#[cfg(target_os = "linux")]
fn main() {
    if let Err(e) = linux::run() {
        eprintln!("sandbox-launch: {e}");
        std::process::exit(126);
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("sandbox-launch is Linux-only");
    std::process::exit(126);
}

#[cfg(target_os = "linux")]
mod linux {
    use landlock::{
        Access, AccessFs, AccessNet, CompatLevel, Compatible, PathBeneath, PathFd, Ruleset,
        RulesetAttr, RulesetCreatedAttr, ABI,
    };
    use seccompiler::{
        apply_filter, BpfProgram, SeccompAction, SeccompCmpArgLen, SeccompCmpOp,
        SeccompCondition, SeccompFilter, SeccompRule,
    };
    use std::collections::BTreeMap;
    use std::error::Error;
    use std::os::unix::process::CommandExt;
    use std::path::{Path, PathBuf};

    type Res<T> = Result<T, Box<dyn Error>>;

    pub fn run() -> Res<()> {
        let mut args = std::env::args().skip(1);
        let (mut cgroup, mut landlock, mut seccomp, mut probe) = (None, None, false, None);
        while let Some(a) = args.next() {
            match a.as_str() {
                "--cgroup" => cgroup = Some(PathBuf::from(args.next().ok_or("--cgroup DIR")?)),
                "--landlock" => {
                    landlock = Some(PathBuf::from(args.next().ok_or("--landlock SCRATCH")?))
                }
                "--seccomp" => seccomp = true,
                "--" => probe = Some(PathBuf::from(args.next().ok_or("-- PROBE")?)),
                other => return Err(format!("unknown argument {other}").into()),
            }
        }
        let probe = probe.ok_or("missing -- PROBE")?;

        if let Some(dir) = &cgroup {
            std::fs::write(dir.join("cgroup.procs"), std::process::id().to_string())
                .map_err(|e| format!("join cgroup {}: {e}", dir.display()))?;
        }
        if let Some(scratch) = &landlock {
            apply_landlock(scratch, &probe)?;
        }
        if seccomp {
            apply_seccomp()?;
        }
        Err(std::process::Command::new(&probe).exec().into())
    }

    /// The running kernel's Landlock ABI version (0 = unsupported or disabled).
    fn kernel_abi() -> i64 {
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

    fn apply_landlock(scratch: &Path, probe: &Path) -> Res<()> {
        let abi = ABI::V5;
        let read = AccessFs::from_read(abi);
        let mut ruleset = Ruleset::default()
            .set_compatibility(CompatLevel::BestEffort)
            .handle_access(AccessFs::from_all(abi))?
            .handle_access(AccessNet::from_all(abi))?
            .create()?
            .add_rule(PathBeneath::new(PathFd::new(scratch)?, AccessFs::from_all(abi)))?
            .add_rule(PathBeneath::new(PathFd::new(probe)?, AccessFs::Execute | AccessFs::ReadFile))?
            .add_rule(PathBeneath::new(PathFd::new("/usr")?, read))?;
        for file in [
            "/etc/ld.so.cache",
            "/etc/resolv.conf",
            "/etc/nsswitch.conf",
            "/etc/hosts",
            "/etc/host.conf",
            "/etc/gai.conf",
        ] {
            if let Ok(fd) = PathFd::new(file) {
                ruleset = ruleset.add_rule(PathBeneath::new(fd, AccessFs::ReadFile))?;
            }
        }
        let status = ruleset.restrict_self()?;
        eprintln!(
            "sandbox-launch: kernel Landlock ABI {}; ruleset {:?}",
            kernel_abi(),
            status.ruleset
        );
        Ok(())
    }

    fn apply_seccomp() -> Res<()> {
        let always = || Vec::<SeccompRule>::new();
        let mut eperm: BTreeMap<i64, Vec<SeccompRule>> = BTreeMap::new();
        eperm.insert(
            libc::SYS_socket,
            vec![SeccompRule::new(vec![SeccompCondition::new(
                0,
                SeccompCmpArgLen::Dword,
                SeccompCmpOp::Ne,
                libc::AF_UNIX as u64,
            )?])?],
        );
        eperm.insert(libc::SYS_io_uring_setup, always());
        eperm.insert(
            libc::SYS_clone,
            vec![SeccompRule::new(vec![SeccompCondition::new(
                0,
                SeccompCmpArgLen::Qword,
                SeccompCmpOp::MaskedEq(libc::CLONE_THREAD as u64),
                0,
            )?])?],
        );
        #[cfg(target_arch = "x86_64")]
        {
            eperm.insert(libc::SYS_fork, always());
            eperm.insert(libc::SYS_vfork, always());
        }
        let mut enosys: BTreeMap<i64, Vec<SeccompRule>> = BTreeMap::new();
        enosys.insert(libc::SYS_clone3, always());

        let arch = std::env::consts::ARCH.try_into()?;
        let deny: BpfProgram = SeccompFilter::new(
            eperm,
            SeccompAction::Allow,
            SeccompAction::Errno(libc::EPERM as u32),
            arch,
        )?
        .try_into()?;
        let no_clone3: BpfProgram = SeccompFilter::new(
            enosys,
            SeccompAction::Allow,
            SeccompAction::Errno(libc::ENOSYS as u32),
            arch,
        )?
        .try_into()?;
        // SAFETY: prctl with constant arguments.
        if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        apply_filter(&no_clone3)?;
        apply_filter(&deny)?;
        eprintln!("sandbox-launch: seccomp filters installed");
        Ok(())
    }
}
