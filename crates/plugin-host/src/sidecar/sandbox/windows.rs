//! Windows: an AppContainer with no capabilities, inside a Job Object, the
//! backend the #571 spike recommended (`appcontainer+job`, 11 of 11 checks).
//! A port of `spikes/sidecar-sandbox/src/windows.rs`, with three changes the
//! spike did not need:
//!
//! - **One profile per app**, named by a digest of its ID (the ADR's
//!   recommendation), instead of one for the whole spike. [`delete_profile`]
//!   removes it when the app is uninstalled.
//! - **stderr is its own pipe.** The spike joined it to stdout; here stdout is
//!   protocol only, so a log line there would break the framing.
//! - **The environment is passed, not inherited**: the command's own
//!   variables, and the few Windows needs ([`FROM_HOST`]). The spike passed no
//!   block, so its probe inherited the host's environment.
//!
//! Everything else is as the spike ran it: `CreateAppContainerProfile` (a SID
//! only derived is refused by `CreateProcessW`), read and execute on the
//! program and full control of the data directory granted to the container's
//! SID with `icacls`, and `CreateProcessW` with the handle list (only the
//! three pipe ends are inherited), the security capabilities and the job list
//! (the process is in the job from its first instruction). The job holds it
//! to one active process, the memory limit and a hard CPU rate, and kills it
//! when the job's last handle closes.

use sha2::{Digest, Sha256};
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::ptr::{null, null_mut};
use std::sync::Arc;
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, SetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT,
};
use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows_sys::Win32::Security::Isolation::{
    CreateAppContainerProfile, DeleteAppContainerProfile, DeriveAppContainerSidFromAppContainerName,
};
use windows_sys::Win32::Security::{FreeSid, PSID, SECURITY_ATTRIBUTES, SECURITY_CAPABILITIES};
use windows_sys::Win32::System::JobObjects::{
    CreateJobObjectW, JobObjectCpuRateControlInformation, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_CPU_RATE_CONTROL_INFORMATION,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_CPU_RATE_CONTROL_ENABLE,
    JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP, JOB_OBJECT_LIMIT_ACTIVE_PROCESS,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess,
    InitializeProcThreadAttributeList, UpdateProcThreadAttribute, WaitForSingleObject,
    CREATE_UNICODE_ENVIRONMENT, EXTENDED_STARTUPINFO_PRESENT, INFINITE,
    LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
    PROC_THREAD_ATTRIBUTE_JOB_LIST, PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES,
    STARTF_USESTDHANDLES, STARTUPINFOEXW,
};

use super::{Exit, LaunchError, Launched, Process, SidecarCommand};
use crate::sidecar::Limits;

/// The AppContainer profile name for `app_id`. A name is at most 64
/// characters and an app ID up to 128, so it is a digest: 16 bytes of
/// SHA-256, which no other installed app's ID can be chosen to share.
pub(crate) fn profile_name(app_id: &str) -> String {
    let digest = Sha256::digest(app_id.as_bytes());
    let hex: String = digest[..16].iter().map(|b| format!("{b:02x}")).collect();
    format!("srelens.sidecar.{hex}")
}

fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(Some(0)).collect()
}

fn last_error(what: &str) -> io::Error {
    // SAFETY: trivially safe.
    let code = unsafe { GetLastError() };
    let error = io::Error::from_raw_os_error(code as i32);
    io::Error::new(error.kind(), format!("{what}: {error}"))
}

struct Sid(PSID);

impl Drop for Sid {
    fn drop(&mut self) {
        // SAFETY: allocated by CreateAppContainerProfile or
        // DeriveAppContainerSidFromAppContainerName, freed once.
        unsafe { FreeSid(self.0) };
    }
}

/// `HRESULT_FROM_WIN32(ERROR_ALREADY_EXISTS)`.
const ALREADY_EXISTS: i32 = 0x800700B7_u32 as i32;

/// The app's container SID, registering its profile first.
fn container_sid(app_id: &str) -> io::Result<Sid> {
    // Concurrent CreateAppContainerProfile calls for a profile that does not
    // exist yet failed with 0x8007000A in the spike, three runs out of three.
    static CREATE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _serialized = CREATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let name = wide(profile_name(app_id));
    let display = wide(app_id);
    let description = wide(format!("srelens sidecar for {app_id}"));
    let mut sid: PSID = null_mut();
    // SAFETY: valid wide strings and out-pointer; no capabilities are passed.
    let hr = unsafe {
        CreateAppContainerProfile(
            name.as_ptr(),
            display.as_ptr(),
            description.as_ptr(),
            null(),
            0,
            &mut sid,
        )
    };
    if hr == 0 {
        return Ok(Sid(sid));
    }
    if hr != ALREADY_EXISTS {
        return Err(io::Error::other(format!(
            "CreateAppContainerProfile: {hr:#x}"
        )));
    }
    // SAFETY: valid wide string and out-pointer.
    let hr = unsafe { DeriveAppContainerSidFromAppContainerName(name.as_ptr(), &mut sid) };
    if hr != 0 {
        return Err(io::Error::other(format!(
            "DeriveAppContainerSidFromAppContainerName: {hr:#x}"
        )));
    }
    Ok(Sid(sid))
}

/// Remove the AppContainer profile of `app_id`, when the app is uninstalled.
/// Where the profiles go when srelens itself is uninstalled with apps still
/// installed is open (ADR, "Open questions").
pub fn delete_profile(app_id: &str) -> io::Result<()> {
    // SAFETY: valid wide string.
    let hr = unsafe { DeleteAppContainerProfile(wide(profile_name(app_id)).as_ptr()) };
    if hr != 0 {
        return Err(io::Error::other(format!(
            "DeleteAppContainerProfile: {hr:#x}"
        )));
    }
    Ok(())
}

fn sid_string(sid: &Sid) -> io::Result<String> {
    let mut s: *mut u16 = null_mut();
    // SAFETY: valid SID; the returned buffer is freed with LocalFree.
    unsafe {
        if ConvertSidToStringSidW(sid.0, &mut s) == 0 {
            return Err(last_error("ConvertSidToStringSidW"));
        }
        let len = (0..).take_while(|&i| *s.add(i) != 0).count();
        let out = String::from_utf16_lossy(std::slice::from_raw_parts(s, len));
        LocalFree(s as _);
        Ok(out)
    }
}

/// Grant the container SID access to `path` with `icacls`, from System32 by
/// its full path, not by a search of `PATH`.
fn icacls(path: &Path, grant: &str) -> io::Result<()> {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| OsString::from(r"C:\Windows"));
    let icacls = Path::new(&root).join("System32").join("icacls.exe");
    let out = std::process::Command::new(icacls)
        .arg(path)
        .arg("/grant")
        .arg(grant)
        .output()?;
    if !out.status.success() {
        return Err(io::Error::other(format!(
            "icacls {} /grant {grant}: {}",
            path.display(),
            String::from_utf8_lossy(&out.stdout)
        )));
    }
    Ok(())
}

/// Owned kernel handles, closed on drop.
fn owned(handle: HANDLE) -> OwnedHandle {
    // SAFETY: `handle` is a live handle this module created and owns.
    unsafe { OwnedHandle::from_raw_handle(handle as _) }
}

fn job(limits: &Limits) -> io::Result<OwnedHandle> {
    // SAFETY: plain Win32 calls on a job handle this function owns until it
    // returns it.
    unsafe {
        let job = CreateJobObjectW(null(), null());
        if job.is_null() {
            return Err(last_error("CreateJobObjectW"));
        }
        let job = owned(job);
        let raw = job.as_raw_handle() as HANDLE;
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_ACTIVE_PROCESS
            | JOB_OBJECT_LIMIT_PROCESS_MEMORY
            | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        info.BasicLimitInformation.ActiveProcessLimit = 1;
        info.ProcessMemoryLimit = limits.memory_bytes as usize;
        if SetInformationJobObject(
            raw,
            JobObjectExtendedLimitInformation,
            &info as *const _ as _,
            std::mem::size_of_val(&info) as u32,
        ) == 0
        {
            return Err(last_error("SetInformationJobObject(limits)"));
        }
        // CpuRate is in 1/100 of a percent of the whole machine, so a limit in
        // CPUs is divided by the number of logical processors.
        let cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1) as f64;
        let mut rate: JOBOBJECT_CPU_RATE_CONTROL_INFORMATION = std::mem::zeroed();
        rate.ControlFlags =
            JOB_OBJECT_CPU_RATE_CONTROL_ENABLE | JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP;
        rate.Anonymous.CpuRate = ((limits.cpus / cpus) * 10_000.0)
            .round()
            .clamp(1.0, 10_000.0) as u32;
        if SetInformationJobObject(
            raw,
            JobObjectCpuRateControlInformation,
            &rate as *const _ as _,
            std::mem::size_of_val(&rate) as u32,
        ) == 0
        {
            return Err(last_error("SetInformationJobObject(cpu rate)"));
        }
        Ok(job)
    }
}

/// An anonymous pipe: `(parent end, child end)`, only the child end inheritable.
fn pipe(child_reads: bool) -> io::Result<(OwnedHandle, OwnedHandle)> {
    let sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: null_mut(),
        bInheritHandle: 1,
    };
    let (mut read, mut write): (HANDLE, HANDLE) = (null_mut(), null_mut());
    // SAFETY: valid out-pointers; both ends are owned from here on.
    unsafe {
        if CreatePipe(&mut read, &mut write, &sa, 0) == 0 {
            return Err(last_error("CreatePipe"));
        }
        let (parent, child) = if child_reads {
            (write, read)
        } else {
            (read, write)
        };
        let (parent, child) = (owned(parent), owned(child));
        if SetHandleInformation(parent.as_raw_handle() as HANDLE, HANDLE_FLAG_INHERIT, 0) == 0 {
            return Err(last_error("SetHandleInformation"));
        }
        Ok((parent, child))
    }
}

/// The host variables a sidecar's environment carries, when the command does
/// not name them itself:
///
/// - `SystemRoot`, which Winsock needs to start.
/// - `LOCALAPPDATA`, `TEMP` and `TMP`. Windows reroutes these to the
///   AppContainer's own folder under its profile when it creates the process
///   (Microsoft Learn, "Launch an AppContainer"), so the host's values do not
///   reach the sidecar. `CreateProcessW` failed with `ERROR_ENVVAR_NOT_FOUND`
///   (203) on a block without them, on the first CI run of this backend.
const FROM_HOST: &[&str] = &["SystemRoot", "LOCALAPPDATA", "TEMP", "TMP"];

/// The environment block: the command's variables and [`FROM_HOST`], as
/// `host` reads them, sorted case-insensitively as Windows expects, each
/// `NAME=value\0`, then `\0`.
fn environment_block(
    env: &[(OsString, OsString)],
    host: impl Fn(&str) -> Option<OsString>,
) -> Vec<u16> {
    let mut vars: Vec<(OsString, OsString)> = env.to_vec();
    for name in FROM_HOST {
        let named = vars.iter().any(|(k, _)| k.eq_ignore_ascii_case(name));
        if let (false, Some(value)) = (named, host(name)) {
            vars.push(((*name).into(), value));
        }
    }
    vars.sort_by_key(|(k, _)| k.to_ascii_uppercase());
    let mut block = Vec::new();
    for (key, value) in vars {
        block.extend(key.encode_wide());
        block.push(u16::from(b'='));
        block.extend(value.encode_wide());
        block.push(0);
    }
    if block.is_empty() {
        block.push(0);
    }
    block.push(0);
    block
}

/// One argument quoted as `CommandLineToArgvW` reads it back.
fn quote(arg: &OsStr, into: &mut Vec<u16>) {
    let units: Vec<u16> = arg.encode_wide().collect();
    let plain = !units.is_empty()
        && !units.iter().any(|&u| {
            u == u16::from(b' ')
                || u == u16::from(b'\t')
                || u == u16::from(b'\n')
                || u == 0x0b
                || u == u16::from(b'"')
        });
    if plain {
        into.extend(units);
        return;
    }
    into.push(u16::from(b'"'));
    let mut backslashes = 0;
    for &unit in &units {
        if unit == u16::from(b'\\') {
            backslashes += 1;
            continue;
        }
        if unit == u16::from(b'"') {
            // Each backslash before a quote is doubled, and the quote escaped.
            into.extend(std::iter::repeat(u16::from(b'\\')).take(backslashes * 2 + 1));
        } else {
            into.extend(std::iter::repeat(u16::from(b'\\')).take(backslashes));
        }
        backslashes = 0;
        into.push(unit);
    }
    // Backslashes before the closing quote are doubled.
    into.extend(std::iter::repeat(u16::from(b'\\')).take(backslashes * 2));
    into.push(u16::from(b'"'));
}

fn command_line(program: &Path, args: &[OsString]) -> Vec<u16> {
    let mut line = Vec::new();
    quote(program.as_os_str(), &mut line);
    for arg in args {
        line.push(u16::from(b' '));
        quote(arg, &mut line);
    }
    line.push(0);
    line
}

/// The started process and the job it is in.
struct Contained {
    process: OwnedHandle,
    job: OwnedHandle,
}

/// Terminates the job when told to, and when the last handle to the process
/// goes, so dropping the `Process` stops the sidecar.
struct Killer(Arc<Contained>);

impl Killer {
    fn kill(&self) {
        // SAFETY: a live job handle owned by `Contained`.
        unsafe { TerminateJobObject(self.0.job.as_raw_handle() as HANDLE, 1) };
    }
}

impl Drop for Killer {
    fn drop(&mut self) {
        self.kill();
    }
}

pub(super) fn launch(command: &SidecarCommand, limits: &Limits) -> Result<Launched, LaunchError> {
    let unavailable = |e: io::Error| {
        LaunchError::Unavailable(format!("srelens cannot confine the app on this Windows system ({e}), so it does not run executable apps"))
    };
    let failed =
        |e: io::Error| LaunchError::Failed(format!("srelens could not start the app: {e}"));
    let sid = container_sid(&command.app_id).map_err(unavailable)?;
    let sid_text = sid_string(&sid).map_err(unavailable)?;
    icacls(&command.program, &format!("*{sid_text}:(RX)")).map_err(failed)?;
    icacls(&command.data_dir, &format!("*{sid_text}:(OI)(CI)(F)")).map_err(failed)?;
    let job = job(limits).map_err(unavailable)?;

    let (stdin_parent, stdin_child) = pipe(true).map_err(failed)?;
    let (stdout_parent, stdout_child) = pipe(false).map_err(failed)?;
    let (stderr_parent, stderr_child) = pipe(false).map_err(failed)?;
    let inherited: [HANDLE; 3] = [
        stdin_child.as_raw_handle() as HANDLE,
        stdout_child.as_raw_handle() as HANDLE,
        stderr_child.as_raw_handle() as HANDLE,
    ];
    let jobs: [HANDLE; 1] = [job.as_raw_handle() as HANDLE];
    let caps = SECURITY_CAPABILITIES {
        AppContainerSid: sid.0,
        Capabilities: null_mut(),
        CapabilityCount: 0,
        Reserved: 0,
    };
    let environment = environment_block(&command.env, |name| std::env::var_os(name));
    let app = wide(&command.program);
    let mut cmdline = command_line(&command.program, &command.args);
    let cwd = wide(&command.data_dir);

    // SAFETY: the attribute list buffer is sized by the first call,
    // initialised by the second, updated with pointers to locals that
    // outlive CreateProcessW, and deleted after it.
    let process = unsafe {
        let count = 3;
        let mut size = 0usize;
        InitializeProcThreadAttributeList(null_mut(), count, 0, &mut size);
        let mut buf = vec![0u8; size];
        let attrs = buf.as_mut_ptr() as LPPROC_THREAD_ATTRIBUTE_LIST;
        if InitializeProcThreadAttributeList(attrs, count, 0, &mut size) == 0 {
            return Err(failed(last_error("InitializeProcThreadAttributeList")));
        }
        let update = |attr: usize, value: *const std::ffi::c_void, len: usize, what: &str| {
            if UpdateProcThreadAttribute(attrs, 0, attr, value, len, null_mut(), null()) == 0 {
                Err(failed(last_error(what)))
            } else {
                Ok(())
            }
        };
        let updated = update(
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
            inherited.as_ptr() as _,
            std::mem::size_of_val(&inherited),
            "HANDLE_LIST",
        )
        .and_then(|()| {
            update(
                PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize,
                &caps as *const _ as _,
                std::mem::size_of_val(&caps),
                "SECURITY_CAPABILITIES",
            )
        })
        .and_then(|()| {
            update(
                PROC_THREAD_ATTRIBUTE_JOB_LIST as usize,
                jobs.as_ptr() as _,
                std::mem::size_of_val(&jobs),
                "JOB_LIST",
            )
        });
        if let Err(e) = updated {
            DeleteProcThreadAttributeList(attrs);
            return Err(e);
        }

        let mut si: STARTUPINFOEXW = std::mem::zeroed();
        si.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
        si.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        si.StartupInfo.hStdInput = inherited[0];
        si.StartupInfo.hStdOutput = inherited[1];
        si.StartupInfo.hStdError = inherited[2];
        si.lpAttributeList = attrs;

        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        let ok = CreateProcessW(
            app.as_ptr(),
            cmdline.as_mut_ptr(),
            null(),
            null(),
            1,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
            environment.as_ptr() as _,
            cwd.as_ptr(),
            &si.StartupInfo,
            &mut pi,
        );
        let error = (ok == 0).then(|| last_error("CreateProcessW"));
        DeleteProcThreadAttributeList(attrs);
        if let Some(error) = error {
            return Err(failed(error));
        }
        CloseHandle(pi.hThread);
        (owned(pi.hProcess), pi.dwProcessId)
    };
    // The child's ends belong to the child now.
    drop((stdin_child, stdout_child, stderr_child));
    let (process, pid) = process;
    let contained = Arc::new(Contained { process, job });
    let killer = Arc::new(Killer(contained.clone()));
    let (ended, exit) = tokio::sync::oneshot::channel();
    tokio::task::spawn_blocking(move || {
        let handle = contained.process.as_raw_handle() as HANDLE;
        let mut code = 0u32;
        // SAFETY: a live process handle owned by `contained`.
        let known = unsafe {
            WaitForSingleObject(handle, INFINITE);
            GetExitCodeProcess(handle, &mut code) != 0
        };
        let exit = if known {
            Exit {
                description: format!("exited with code {code:#x}"),
                code: Some(code as i32),
                signal: None,
                memory_limit: false,
            }
        } else {
            Exit {
                description: "ended, and srelens could not learn how".into(),
                code: None,
                signal: None,
                memory_limit: false,
            }
        };
        let _ = ended.send(exit);
    });
    let exit = async move {
        exit.await.unwrap_or_else(|_| Exit {
            description: "ended, and srelens lost track of how".into(),
            code: None,
            signal: None,
            memory_limit: false,
        })
    };
    let file = |handle: OwnedHandle| tokio::fs::File::from_std(File::from(handle));
    Ok(Launched {
        stdin: Box::new(file(stdin_parent)),
        stdout: Box::new(file(stdout_parent)),
        stderr: Box::new(file(stderr_parent)),
        process: Process::new(Some(pid), exit, move || killer.kill()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_profile_name_fits_windows_whatever_the_app_id() {
        let long = format!("org.{}", "a".repeat(124));
        for id in ["org.example.a", long.as_str()] {
            let name = profile_name(id);
            assert!(name.len() <= 64, "{name}");
            assert!(name.starts_with("srelens.sidecar."));
        }
        assert_ne!(profile_name("org.example.a"), profile_name("org.example.b"));
        assert_eq!(profile_name("org.example.a"), profile_name("org.example.a"));
    }

    #[test]
    fn the_environment_is_the_commands_and_the_few_windows_needs() {
        let host = |name: &str| match name {
            "SystemRoot" => Some(OsString::from("C:\\Windows")),
            "TEMP" => Some(OsString::from("C:\\T")),
            "KUBECONFIG" => Some(OsString::from("C:\\kube")),
            _ => None,
        };
        let block = environment_block(
            &[
                ("ZED".into(), "1".into()),
                ("alpha".into(), "2".into()),
                ("tmp".into(), "mine".into()),
            ],
            host,
        );
        let text = String::from_utf16(&block).unwrap();
        // A variable the command names is not read from the host, and
        // nothing outside `FROM_HOST` is.
        assert_eq!(
            text,
            "alpha=2\0SystemRoot=C:\\Windows\0TEMP=C:\\T\0tmp=mine\0ZED=1\0\0"
        );
        let empty = environment_block(&[], |_| None);
        assert_eq!(empty, [0, 0]);
    }

    fn quoted(arg: &str) -> String {
        let mut out = Vec::new();
        quote(OsStr::new(arg), &mut out);
        String::from_utf16(&out).unwrap()
    }

    #[test]
    fn arguments_are_quoted_as_command_line_to_argv_reads_them() {
        assert_eq!(quoted("plain"), "plain");
        assert_eq!(quoted(""), "\"\"");
        assert_eq!(quoted("two words"), "\"two words\"");
        assert_eq!(quoted("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(quoted("C:\\dir with space\\"), "\"C:\\dir with space\\\\\"");
        assert_eq!(quoted("a\\\\b"), "a\\\\b");
    }
}
