// ============================================================================
// Module:       win::job
// Description:  A job object that ends every process the app started when the
//               app ends, however it ends.
//
// Dependencies: windows-sys (CreateJobObjectW, SetInformationJobObject,
//               AssignProcessToJobObject, GetCurrentProcess, CloseHandle)
// ============================================================================

//! Child processes die with the app.
//!
//! The app does its work in ffmpeg processes (analysis, playback copies,
//! renders) and Windows does not end a process's children when it ends: a
//! window closed mid-analysis, or a crash, left them running, decoding 4K
//! for nobody. The app puts itself in a job object set to kill every
//! process in it when its last handle closes; children join the job when
//! they start, and the handle closes when the app's process does, so
//! nothing it started outlives it.
//!
//! The guard is held for the whole run: closing it early would end the app
//! too.

use windows_sys::Win32::Foundation as foundation;
use windows_sys::Win32::System::JobObjects as jobs;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

/// The job the app runs in; while it lives, children die with the app.
pub struct KillOnExit {
    /// The job object's handle.
    handle: foundation::HANDLE,
}

impl Drop for KillOnExit {
    fn drop(&mut self) {
        close_handle(self.handle);
    }
}

/// Puts this process (and so every process it starts after) in a job that
/// kills them all when the app ends. `None` when Windows refuses (the
/// process is in a job that forbids it, say): the app runs, just without
/// the guarantee.
#[must_use]
pub fn kill_children_on_exit() -> Option<KillOnExit> {
    let handle = create_job();
    if handle.is_null() {
        return None;
    }
    let guard = KillOnExit { handle };
    let mut limits = jobs::JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = jobs::JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    if !set_limits(guard.handle, &limits) || !assign_self(guard.handle) {
        return None;
    }
    Some(guard)
}

/// Creates an unnamed job object with the default security.
fn create_job() -> foundation::HANDLE {
    // SAFETY: null attributes and a null name ask for an unnamed job with
    // the default descriptor; nothing is borrowed.
    unsafe { jobs::CreateJobObjectW(std::ptr::null(), std::ptr::null()) }
}

/// Sets `limits` on the job `handle`; whether Windows accepted them.
fn set_limits(
    handle: foundation::HANDLE,
    limits: &jobs::JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
) -> bool {
    let size = u32::try_from(std::mem::size_of::<
        jobs::JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    >())
    .unwrap_or(u32::MAX);
    let info: *const jobs::JOBOBJECT_EXTENDED_LIMIT_INFORMATION = limits;
    // SAFETY: `handle` is a live job handle owned by the caller's guard;
    // `info` points at `limits`, borrowed for the call, and `size` is its
    // exact size, matching the information class.
    unsafe {
        jobs::SetInformationJobObject(
            handle,
            jobs::JobObjectExtendedLimitInformation,
            info.cast(),
            size,
        ) != 0
    }
}

/// Puts the current process in the job `handle`; whether it took.
fn assign_self(handle: foundation::HANDLE) -> bool {
    let me = current_process();
    // SAFETY: `handle` is a live job handle owned by the caller's guard;
    // `me` is the current-process pseudo-handle, valid for the process's
    // life and never closed.
    unsafe { jobs::AssignProcessToJobObject(handle, me) != 0 }
}

/// The current process's pseudo-handle.
fn current_process() -> foundation::HANDLE {
    // SAFETY: takes no arguments; returns a constant pseudo-handle.
    unsafe { GetCurrentProcess() }
}

/// Closes the job handle; the last close kills the job's processes.
fn close_handle(handle: foundation::HANDLE) {
    // SAFETY: `handle` came from `create_job`, is non-null (the guard is
    // only built for a non-null one) and is closed exactly once, by it.
    unsafe { foundation::CloseHandle(handle) };
}
