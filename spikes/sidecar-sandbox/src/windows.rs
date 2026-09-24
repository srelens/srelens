//! Windows backends: an AppContainer (optionally less-privileged, LPAC) for filesystem and
//! network, and a Job Object for memory, CPU and process-count limits.
//!
//! Everything here is the documented Win32 surface:
//!
//! - `CreateAppContainerProfile` registers the container for the current user and gives
//!   its SID. A SID from `DeriveAppContainerSidFromAppContainerName` alone is refused by
//!   `CreateProcessW`. `cargo run --bin cleanup` deletes the profile afterwards.
//! - The only files the container can open are the ones granted to that SID: read/execute
//!   on the copied probe binary, full control on the scratch directory. The grant is made
//!   with `icacls` on the fixture's own files.
//! - `CreateProcessW` with `STARTUPINFOEXW` carries three attributes:
//!   `PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES` (the container, with no capabilities, so
//!   no `internetClient`), `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` (only the two stdio pipe ends
//!   are inherited) and `PROC_THREAD_ATTRIBUTE_JOB_LIST` (the process starts inside the job,
//!   with no window in which it runs unlimited). LPAC adds
//!   `PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY`.
//! - The job sets `JOB_OBJECT_LIMIT_ACTIVE_PROCESS` = 1, `JOB_OBJECT_LIMIT_PROCESS_MEMORY`,
//!   `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` and a hard CPU-rate cap.

use crate::{Fixture, Limits};
use std::ffi::OsStr;
use std::fs::File;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::FromRawHandle;
use std::process::Command;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, SetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT,
    WAIT_OBJECT_0,
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
    InitializeProcThreadAttributeList, TerminateProcess, UpdateProcThreadAttribute,
    WaitForSingleObject, EXTENDED_STARTUPINFO_PRESENT, LPPROC_THREAD_ATTRIBUTE_LIST,
    PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_JOB_LIST,
    PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, STARTF_USESTDHANDLES, STARTUPINFOEXW,
};

/// `PROCESS_CREATION_ALL_APPLICATION_PACKAGES_OPT_OUT` from WinBase.h.
const ALL_APPLICATION_PACKAGES_OPT_OUT: u32 = 0x01;
const CONTAINER_NAME: &str = "org.srelens.sandbox-spike";

#[derive(Clone, Copy, Debug)]
pub struct Layers {
    pub appcontainer: bool,
    pub lpac: bool,
    pub job: bool,
}

/// A sandboxed process, and the objects that confine it.
pub struct Contained {
    process: HANDLE,
    job: HANDLE,
}

// SAFETY: HANDLEs are process-wide kernel object references; nothing here is thread-affine.
unsafe impl Send for Contained {}

impl Contained {
    pub fn ended(&mut self) -> String {
        // SAFETY: `process` is a live handle owned by `self`.
        unsafe {
            if WaitForSingleObject(self.process, 5_000) != WAIT_OBJECT_0 {
                return "stdout closed but the process is still running".into();
            }
            let mut code = 0u32;
            GetExitCodeProcess(self.process, &mut code);
            format!("exited: code {code:#x}")
        }
    }
}

impl Drop for Contained {
    fn drop(&mut self) {
        // SAFETY: handles owned by `self`, closed exactly once.
        unsafe {
            TerminateProcess(self.process, 1);
            if !self.job.is_null() {
                TerminateJobObject(self.job, 1);
                CloseHandle(self.job);
            }
            CloseHandle(self.process);
        }
    }
}

fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(Some(0)).collect()
}

fn last_error(what: &str) -> io::Error {
    // SAFETY: trivially safe.
    let code = unsafe { GetLastError() };
    io::Error::new(io::Error::from_raw_os_error(code as i32).kind(), format!("{what}: {}", io::Error::from_raw_os_error(code as i32)))
}

struct Sid(PSID);
impl Drop for Sid {
    fn drop(&mut self) {
        // SAFETY: allocated by DeriveAppContainerSidFromAppContainerName, freed once.
        unsafe { FreeSid(self.0) };
    }
}

/// `HRESULT_FROM_WIN32(ERROR_ALREADY_EXISTS)`.
const ALREADY_EXISTS: i32 = 0x800700B7_u32 as i32;

/// The container's SID, registering its profile first.
///
/// `CreateProcessW` refuses a SID that was only derived, with `ERROR_FILE_NOT_FOUND`
/// (observed in this spike), so the profile has to exist. `delete_profile` removes it.
fn container_sid() -> io::Result<Sid> {
    let name = wide(CONTAINER_NAME);
    let mut sid: PSID = null_mut();
    // SAFETY: valid wide strings and out-pointer; no capabilities are passed.
    let hr = unsafe {
        CreateAppContainerProfile(name.as_ptr(), name.as_ptr(), name.as_ptr(), null(), 0, &mut sid)
    };
    if hr == 0 {
        return Ok(Sid(sid));
    }
    if hr != ALREADY_EXISTS {
        return Err(io::Error::other(format!("CreateAppContainerProfile: {hr:#x}")));
    }
    // SAFETY: valid wide string and out-pointer.
    let hr = unsafe { DeriveAppContainerSidFromAppContainerName(name.as_ptr(), &mut sid) };
    if hr != 0 {
        return Err(io::Error::other(format!("DeriveAppContainerSidFromAppContainerName: {hr:#x}")));
    }
    Ok(Sid(sid))
}

/// Remove the profile `container_sid` registered.
pub fn delete_profile() -> io::Result<()> {
    // SAFETY: valid wide string.
    let hr = unsafe { DeleteAppContainerProfile(wide(CONTAINER_NAME).as_ptr()) };
    if hr != 0 {
        return Err(io::Error::other(format!("DeleteAppContainerProfile: {hr:#x}")));
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

fn icacls(path: &std::path::Path, grant: &str) -> io::Result<()> {
    let out = Command::new("icacls").arg(path).arg("/grant").arg(grant).output()?;
    if !out.status.success() {
        return Err(io::Error::other(format!(
            "icacls {} /grant {grant}: {}",
            path.display(),
            String::from_utf8_lossy(&out.stdout)
        )));
    }
    Ok(())
}

fn job(limits: &Limits) -> io::Result<HANDLE> {
    // SAFETY: plain Win32 calls on a job handle this function owns until it returns it.
    unsafe {
        let job = CreateJobObjectW(null(), null());
        if job.is_null() {
            return Err(last_error("CreateJobObjectW"));
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_ACTIVE_PROCESS
            | JOB_OBJECT_LIMIT_PROCESS_MEMORY
            | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        info.BasicLimitInformation.ActiveProcessLimit = 1;
        info.ProcessMemoryLimit = (limits.memory_mib * 1024 * 1024) as usize;
        if SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as _,
            std::mem::size_of_val(&info) as u32,
        ) == 0
        {
            CloseHandle(job);
            return Err(last_error("SetInformationJobObject(limits)"));
        }
        // CpuRate is in 1/100 of a percent of the whole machine, so a limit in CPUs is
        // divided by the number of logical processors.
        let cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1) as f64;
        let mut rate: JOBOBJECT_CPU_RATE_CONTROL_INFORMATION = std::mem::zeroed();
        rate.ControlFlags = JOB_OBJECT_CPU_RATE_CONTROL_ENABLE | JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP;
        rate.Anonymous.CpuRate = ((limits.cpus / cpus) * 10_000.0).round().max(1.0) as u32;
        if SetInformationJobObject(
            job,
            JobObjectCpuRateControlInformation,
            &rate as *const _ as _,
            std::mem::size_of_val(&rate) as u32,
        ) == 0
        {
            CloseHandle(job);
            return Err(last_error("SetInformationJobObject(cpu rate)"));
        }
        Ok(job)
    }
}

/// An anonymous pipe: `(parent end, child end)`, with only the child end inheritable.
fn pipe(child_reads: bool) -> io::Result<(HANDLE, HANDLE)> {
    let sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: null_mut(),
        bInheritHandle: 1,
    };
    let (mut read, mut write): (HANDLE, HANDLE) = (null_mut(), null_mut());
    // SAFETY: valid out-pointers.
    unsafe {
        if CreatePipe(&mut read, &mut write, &sa, 0) == 0 {
            return Err(last_error("CreatePipe"));
        }
        let (parent, child) = if child_reads { (write, read) } else { (read, write) };
        SetHandleInformation(parent, HANDLE_FLAG_INHERIT, 0);
        Ok((parent, child))
    }
}

pub fn launch(
    fixture: &Fixture,
    limits: &Limits,
    layers: Layers,
) -> io::Result<(File, File, Contained)> {
    let sid = if layers.appcontainer {
        let sid = container_sid()?;
        let s = sid_string(&sid)?;
        icacls(&fixture.probe(), &format!("*{s}:(RX)"))?;
        icacls(&fixture.scratch(), &format!("*{s}:(OI)(CI)(F)"))?;
        Some(sid)
    } else {
        None
    };
    let job = if layers.job { job(limits)? } else { null_mut() };

    let (stdin_parent, stdin_child) = pipe(true)?;
    let (stdout_parent, stdout_child) = pipe(false)?;
    let inherited = [stdin_child, stdout_child];
    let jobs = [job];
    let caps = SECURITY_CAPABILITIES {
        AppContainerSid: sid.as_ref().map_or(null_mut(), |s| s.0),
        Capabilities: null_mut(),
        CapabilityCount: 0,
        Reserved: 0,
    };
    let lpac_policy: u32 = ALL_APPLICATION_PACKAGES_OPT_OUT;

    let count = 1 + layers.appcontainer as u32 + layers.lpac as u32 + layers.job as u32;
    // SAFETY: the attribute list buffer is sized by the first call, initialised by the
    // second, updated with pointers to locals that outlive CreateProcessW, and deleted.
    unsafe {
        let mut size = 0usize;
        InitializeProcThreadAttributeList(null_mut(), count, 0, &mut size);
        let mut buf = vec![0u8; size];
        let attrs = buf.as_mut_ptr() as LPPROC_THREAD_ATTRIBUTE_LIST;
        if InitializeProcThreadAttributeList(attrs, count, 0, &mut size) == 0 {
            return Err(last_error("InitializeProcThreadAttributeList"));
        }
        let update =|attr: usize, value: *const std::ffi::c_void, len: usize, what: &str| {
            if UpdateProcThreadAttribute(attrs, 0, attr, value, len, null_mut(), null()) == 0 {
                Err(last_error(what))
            } else {
                Ok(())
            }
        };
        update(
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
            inherited.as_ptr() as _,
            std::mem::size_of_val(&inherited),
            "HANDLE_LIST",
        )?;
        if layers.appcontainer {
            update(
                PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize,
                &caps as *const _ as _,
                std::mem::size_of_val(&caps),
                "SECURITY_CAPABILITIES",
            )?;
        }
        if layers.lpac {
            update(
                PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY as usize,
                &lpac_policy as *const _ as _,
                std::mem::size_of_val(&lpac_policy),
                "ALL_APPLICATION_PACKAGES_POLICY",
            )?;
        }
        if layers.job {
            update(
                PROC_THREAD_ATTRIBUTE_JOB_LIST as usize,
                jobs.as_ptr() as _,
                std::mem::size_of_val(&jobs),
                "JOB_LIST",
            )?;
        }

        let mut si: STARTUPINFOEXW = std::mem::zeroed();
        si.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
        si.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        si.StartupInfo.hStdInput = stdin_child;
        si.StartupInfo.hStdOutput = stdout_child;
        // stderr shares the stdout pipe: the probe writes nothing there unless it panics,
        // and then the panic text arrives as an unparseable reply instead of vanishing.
        si.StartupInfo.hStdError = stdout_child;
        si.lpAttributeList = attrs;

        let app = wide(fixture.probe());
        let mut cmdline = wide(format!("\"{}\"", fixture.probe().display()));
        let cwd = wide(fixture.scratch());
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        let ok = CreateProcessW(
            app.as_ptr(),
            cmdline.as_mut_ptr(),
            null(),
            null(),
            1,
            EXTENDED_STARTUPINFO_PRESENT,
            null(),
            cwd.as_ptr(),
            &si.StartupInfo,
            &mut pi,
        );
        let err = (ok == 0).then(|| last_error("CreateProcessW"));
        DeleteProcThreadAttributeList(attrs);
        CloseHandle(stdin_child);
        CloseHandle(stdout_child);
        if let Some(err) = err {
            CloseHandle(stdin_parent);
            CloseHandle(stdout_parent);
            if !job.is_null() {
                CloseHandle(job);
            }
            return Err(err);
        }
        CloseHandle(pi.hThread);
        Ok((
            File::from_raw_handle(stdin_parent as _),
            File::from_raw_handle(stdout_parent as _),
            Contained { process: pi.hProcess, job },
        ))
    }
}
