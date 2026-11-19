// ============================================================================
// Module:       win::console
// Description:  Reattaches a GUI-subsystem release build to the terminal it
//               was started from, so pipeline subcommands can print.
//
// Dependencies: windows-sys (AttachConsole)
// ============================================================================

//! The parent console.
//!
//! A release build is a Windows-subsystem binary (no console window behind
//! the GUI), so a subcommand run from a terminal would write into nothing.
//! [`attach_parent`] joins the console of the process that started us, if
//! there is one; from Explorer there is none and nothing changes.

use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};

/// Joins the parent process's console. Returns whether one was attached.
pub fn attach_parent() -> bool {
    attach(ATTACH_PARENT_PROCESS)
}

/// Attaches to the console of process `pid`.
fn attach(pid: u32) -> bool {
    // SAFETY: AttachConsole takes a process id by value and touches no
    // caller memory; failure is reported through the return value.
    unsafe { AttachConsole(pid) != 0 }
}
