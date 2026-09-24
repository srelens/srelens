//! Unix launcher: confines itself, then `exec`s PROGRAM, which inherits every layer.
//!
//! ```text
//! sandbox-launch [--cgroup DIR] [--landlock SCRATCH] [--seccomp]     (Linux only)
//!                [--rlimit-mem MIB] [--rlimit-cpu SECONDS]            (any Unix)
//!                -- PROGRAM [ARGS...]
//! ```
//!
//! On Linux, PROGRAM is the probe. On macOS it is `/usr/bin/sandbox-exec`, which applies the
//! Seatbelt profile and then runs the probe (see `src/macos.rs` and `src/seatbelt.sb`).
//!
//! It is the host's trusted code, so it runs unconfined until it has applied the layers,
//! in this order:
//!
//! 0. `--rlimit-mem MIB`, `--rlimit-cpu SECONDS`: `setrlimit` for `RLIMIT_DATA` and
//!    `RLIMIT_AS` (both to MIB), and `RLIMIT_CPU` (a lifetime budget of CPU seconds, not a
//!    rate). Each result is printed, whether the kernel accepted the limit or not: macOS is
//!    reported to accept some of these without enforcing them, so the spike records what
//!    happens instead of assuming.
//! 1. `--cgroup DIR`: write its own PID to `DIR/cgroup.procs`. The host created `DIR` with
//!    `memory.max`, `memory.swap.max` and `cpu.max`. This must come before Landlock, which
//!    would refuse the write.
//! 2. `--landlock SCRATCH`: a ruleset that targets **Landlock ABI 5**: the filesystem rights
//!    of ABIs 1 to 5, and TCP bind/connect (ABI 4). It is best-effort, so an older kernel
//!    enforces the subset it knows (the kernel observed in this spike has ABI 3, so no TCP
//!    rules). It does not handle what later ABIs added: ABI 6 scoping of abstract Unix
//!    sockets and signals, ABI 9 `RESOLVE_UNIX` (connecting to a pathname Unix socket) or
//!    ABI 10 UDP. `landlock` 0.4.7 knows ABIs up to 9 and has no UDP rights. It grants only:
//!    everything under SCRATCH; read+execute on the probe; read+execute under `/usr` (the
//!    dynamic loader and libc); and read of the few `/etc` files libc's resolver reads.
//! 3. `--seccomp`: a deny-list filter. `socket` for any family but `AF_UNIX`, `io_uring_setup`,
//!    `fork`, `vfork` and `clone` without `CLONE_THREAD` fail with `EPERM`; `clone3` fails
//!    with `ENOSYS`, so libc falls back to `clone`, where the flags can be inspected. Threads
//!    still work; new processes do not. `execve` stays allowed, which is what lets this
//!    launcher become the probe (it replaces the image, it does not create a process).

#[cfg(unix)]
fn main() {
    if let Err(e) = unix::run() {
        eprintln!("sandbox-launch: {e}");
        std::process::exit(126);
    }
}

#[cfg(not(unix))]
fn main() {
    eprintln!("sandbox-launch is for Linux and macOS");
    std::process::exit(126);
}

#[cfg(unix)]
mod unix {
    use std::error::Error;
    use std::ffi::OsString;
    use std::os::unix::process::CommandExt;
    use std::path::PathBuf;

    pub type Res<T> = Result<T, Box<dyn Error>>;

    #[derive(Default)]
    struct Args {
        cgroup: Option<PathBuf>,
        landlock: Option<PathBuf>,
        seccomp: bool,
        rlimit_mem_mib: Option<u64>,
        rlimit_cpu_seconds: Option<u64>,
        program: Vec<OsString>,
    }

    fn parse() -> Res<Args> {
        let mut args = std::env::args_os().skip(1);
        let mut out = Args::default();
        let number = |v: Option<OsString>, flag: &str| -> Res<u64> {
            Ok(v.and_then(|v| v.into_string().ok()).ok_or(format!("{flag} N"))?.parse()?)
        };
        while let Some(a) = args.next() {
            match a.to_str().unwrap_or("") {
                "--cgroup" => out.cgroup = Some(args.next().ok_or("--cgroup DIR")?.into()),
                "--landlock" => out.landlock = Some(args.next().ok_or("--landlock SCRATCH")?.into()),
                "--seccomp" => out.seccomp = true,
                "--rlimit-mem" => out.rlimit_mem_mib = Some(number(args.next(), "--rlimit-mem")?),
                "--rlimit-cpu" => out.rlimit_cpu_seconds = Some(number(args.next(), "--rlimit-cpu")?),
                "--" => {
                    out.program = args.by_ref().collect();
                    break;
                }
                other => return Err(format!("unknown argument {other}").into()),
            }
        }
        if out.program.is_empty() {
            return Err("missing -- PROGRAM".into());
        }
        Ok(out)
    }

    /// Set one limit (soft and hard) and say what happened.
    fn rlimit(name: &str, resource: libc::c_int, value: u64) {
        let lim = libc::rlimit { rlim_cur: value as libc::rlim_t, rlim_max: value as libc::rlim_t };
        // SAFETY: a valid rlimit struct for a resource constant from libc.
        let rc = unsafe { libc::setrlimit(resource as _, &lim) };
        let result = if rc == 0 {
            "set".to_string()
        } else {
            format!("refused: {}", std::io::Error::last_os_error())
        };
        eprintln!("sandbox-launch: {name} = {value}: {result}");
    }

    pub fn run() -> Res<()> {
        let args = parse()?;
        if let Some(mib) = args.rlimit_mem_mib {
            let bytes = mib * 1024 * 1024;
            rlimit("RLIMIT_DATA", libc::RLIMIT_DATA as _, bytes);
            rlimit("RLIMIT_AS", libc::RLIMIT_AS as _, bytes);
        }
        if let Some(seconds) = args.rlimit_cpu_seconds {
            rlimit("RLIMIT_CPU", libc::RLIMIT_CPU as _, seconds);
        }
        #[cfg(target_os = "linux")]
        crate::linux::apply(args.cgroup.as_deref(), args.landlock.as_deref(), args.seccomp, &args.program[0])?;
        #[cfg(not(target_os = "linux"))]
        if args.cgroup.is_some() || args.landlock.is_some() || args.seccomp {
            return Err("--cgroup, --landlock and --seccomp are Linux-only".into());
        }
        Err(std::process::Command::new(&args.program[0]).args(&args.program[1..]).exec().into())
    }
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
    use crate::unix::Res;
    use std::collections::BTreeMap;
    use std::ffi::OsString;
    use std::path::Path;

    /// The Linux layers, in order. `probe` is the program about to be executed.
    pub fn apply(cgroup: Option<&Path>, landlock: Option<&Path>, seccomp: bool, probe: &OsString) -> Res<()> {
        if let Some(dir) = cgroup {
            std::fs::write(dir.join("cgroup.procs"), std::process::id().to_string())
                .map_err(|e| format!("join cgroup {}: {e}", dir.display()))?;
        }
        if let Some(scratch) = landlock {
            apply_landlock(scratch, Path::new(probe))?;
        }
        if seccomp {
            apply_seccomp()?;
        }
        Ok(())
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
        // The ABI this launcher targets; see the module comment for what that leaves out.
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
