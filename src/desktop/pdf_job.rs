//! OS lifetime and committed-memory bounds for the owned PDF helper process.
#[cfg(windows)]
mod platform {
    use matrust::Result;
    use std::{
        os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
        process::Child,
    };
    use windows::{
        Win32::{Foundation::HANDLE, System::JobObjects::*},
        core::PCWSTR,
    };
    pub struct Job {
        _handle: OwnedHandle,
    }
    impl Job {
        pub fn new(child: &Child) -> Result<Self> {
            // SAFETY: All handles are owned/borrowed from live Rust objects. The
            // limit struct stays valid through the call and has its exact ABI size.
            unsafe {
                let handle = CreateJobObjectW(None, PCWSTR::null()).map_err(|e| e.to_string())?;
                let owned = OwnedHandle::from_raw_handle(handle.0);
                let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                limits.BasicLimitInformation.LimitFlags =
                    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_PROCESS_MEMORY;
                limits.ProcessMemoryLimit = 512 * 1024 * 1024;
                SetInformationJobObject(
                    handle,
                    JobObjectExtendedLimitInformation,
                    (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                    std::mem::size_of_val(&limits) as u32,
                )
                .map_err(|e| e.to_string())?;
                AssignProcessToJobObject(handle, HANDLE(child.as_raw_handle()))
                    .map_err(|e| e.to_string())?;
                Ok(Self { _handle: owned })
            }
        }
    }
}
#[cfg(not(windows))]
mod platform {
    pub struct Job;
    impl Job {
        pub fn new(_: &std::process::Child) -> matrust::Result<Self> {
            Ok(Self)
        }
    }
}
pub use platform::Job;
