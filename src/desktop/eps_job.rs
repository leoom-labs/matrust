//! Dedicated EPS process budget. The trusted helper blocks on its request pipe
//! until this job owns it; its interpreter child inherits the same job.
#[cfg(windows)]
mod platform {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows::{
        Win32::{Foundation::HANDLE, System::JobObjects::*},
        core::PCWSTR,
    };
    pub struct Job {
        handle: OwnedHandle,
    }
    impl Job {
        pub fn new(child: &std::process::Child) -> matrust::Result<Self> {
            // SAFETY: Owned handles and exact documented ABI structs remain live.
            unsafe {
                let handle = CreateJobObjectW(None, PCWSTR::null()).map_err(|e| e.to_string())?;
                let owned = OwnedHandle::from_raw_handle(handle.0);
                let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
                    | JOB_OBJECT_LIMIT_ACTIVE_PROCESS
                    | JOB_OBJECT_LIMIT_JOB_MEMORY
                    | JOB_OBJECT_LIMIT_PROCESS_MEMORY;
                limits.BasicLimitInformation.ActiveProcessLimit = 2;
                limits.JobMemoryLimit = 512 * 1024 * 1024;
                limits.ProcessMemoryLimit = limits.JobMemoryLimit;
                SetInformationJobObject(
                    handle,
                    JobObjectExtendedLimitInformation,
                    (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                    std::mem::size_of_val(&limits) as u32,
                )
                .map_err(|e| e.to_string())?;
                AssignProcessToJobObject(handle, HANDLE(child.as_raw_handle()))
                    .map_err(|e| e.to_string())?;
                Ok(Self { handle: owned })
            }
        }
        /// Test-only observation; the helper waits for its probe before rendering.
        #[cfg(test)]
        pub fn observed_renderer(
            &self,
            helper_pid: u32,
            renderer: &std::path::Path,
        ) -> matrust::Result<Option<(u32, OwnedHandle)>> {
            use windows::Win32::{
                Foundation::WAIT_TIMEOUT,
                System::Threading::{
                    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
                    PROCESS_SYNCHRONIZE, QueryFullProcessImageNameW, WaitForSingleObject,
                },
            };
            // C-compatible variable-array header, extended to the job's fixed
            // two-process limit. Never truncate a larger or failed query.
            #[repr(C)]
            #[derive(Default)]
            struct TwoProcesses {
                assigned: u32,
                returned: u32,
                ids: [usize; 2],
            }
            let job = HANDLE(self.handle.as_raw_handle());
            let accounting = || -> matrust::Result<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION> {
                let mut value = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
                // SAFETY: Exact SDK structure, writable storage, live job.
                unsafe {
                    QueryInformationJobObject(
                        Some(job),
                        JobObjectBasicAccountingInformation,
                        (&mut value as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                        std::mem::size_of_val(&value) as u32,
                        None,
                    )
                }
                .map_err(|e| e.to_string())?;
                Ok(value)
            };
            let before = accounting()?;
            if before.TotalTerminatedProcesses != 0 || before.TotalProcesses > 3 {
                return Err(format!(
                    "Unexpected EPS job process or limit-termination count: stage=before_renderer_observation helper_pid={helper_pid} TotalProcesses={} ActiveProcesses={} TotalTerminatedProcesses={}",
                    before.TotalProcesses, before.ActiveProcesses, before.TotalTerminatedProcesses,
                ));
            }
            // Lifetime 1 = helper, 2 = helper + probe, 3 = helper + probe + render.
            if before.TotalProcesses != 3 || before.ActiveProcesses != 2 {
                return Ok(None);
            }
            let mut list = TwoProcesses::default();
            // SAFETY: C-compatible header and two pointer-sized IDs, live job.
            unsafe {
                QueryInformationJobObject(
                    Some(job),
                    JobObjectBasicProcessIdList,
                    (&mut list as *mut TwoProcesses).cast(),
                    std::mem::size_of_val(&list) as u32,
                    None,
                )
            }
            .map_err(|e| e.to_string())?;
            if list.assigned != 2
                || list.returned != 2
                || !list.ids.contains(&(helper_pid as usize))
            {
                return Ok(None);
            }
            let pid = list
                .ids
                .into_iter()
                .find(|&id| id != helper_pid as usize)
                .ok_or("Missing distinct EPS renderer process")?;
            let pid = u32::try_from(pid).map_err(|_| "Invalid EPS renderer PID")?;
            // SAFETY: Own a fresh handle; membership and image checks reject PID reuse.
            let process = unsafe {
                OwnedHandle::from_raw_handle(
                    OpenProcess(
                        PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
                        false,
                        pid,
                    )
                    .map_err(|e| e.to_string())?
                    .0,
                )
            };
            let handle = HANDLE(process.as_raw_handle());
            let mut member = windows::core::BOOL::default();
            let mut name = [0_u16; 32768];
            let mut length = name.len() as u32;
            // SAFETY: Owned handles and bounded writable output buffers.
            unsafe {
                IsProcessInJob(handle, Some(job), &mut member).map_err(|e| e.to_string())?;
                QueryFullProcessImageNameW(
                    handle,
                    PROCESS_NAME_WIN32,
                    windows::core::PWSTR(name.as_mut_ptr()),
                    &mut length,
                )
                .map_err(|e| e.to_string())?;
            }
            if !member.as_bool() || length as usize > name.len() {
                return Err("Observed process is not in the owned EPS job".into());
            }
            use std::os::windows::ffi::OsStringExt;
            let actual =
                std::path::PathBuf::from(std::ffi::OsString::from_wide(&name[..length as usize]));
            if std::fs::canonicalize(actual).map_err(|e| e.to_string())? != renderer {
                return Err(
                    "Observed EPS child is not the configured Ghostscript executable".into(),
                );
            }
            let after = accounting()?;
            if after.TotalProcesses != 3
                || after.TotalTerminatedProcesses != 0
                || after.ActiveProcesses != 2
            {
                return Ok(None);
            }
            // SAFETY: Zero-time observation of the held synchronization handle.
            if unsafe { WaitForSingleObject(handle, 0) } != WAIT_TIMEOUT {
                return Ok(None);
            }
            Ok(Some((pid, process)))
        }
        pub fn terminate(&self) {
            // SAFETY: This is our live job, containing only the preview children.
            unsafe {
                let _ = TerminateJobObject(HANDLE(self.handle.as_raw_handle()), 1);
            }
        }
    }
}
#[cfg(not(windows))]
mod platform {
    pub struct Job;
    impl Job {
        pub fn new(_: &std::process::Child) -> matrust::Result<Self> {
            Err("Configured EPS preview currently requires Windows process isolation".into())
        }
        pub fn terminate(&self) {}
    }
}
pub use platform::Job;
