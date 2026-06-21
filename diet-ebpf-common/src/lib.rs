//! Shared event types that cross the kernel↔userspace boundary.
//!
//! Compiled with `#![no_std]` for the eBPF target; the `user` feature
//! re-enables std-dependent derives for the CLI crate.

#![cfg_attr(not(feature = "user"), no_std)]

#[cfg_attr(feature = "user", derive(serde::Serialize, serde::Deserialize))]
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct OpenAtEvent {
    /// Thread group ID (process ID in Linux userspace parlance)
    pub tgid: u32,
    /// Thread ID (kernel thread PID)
    pub pid: u32,
    pub uid: u32,
    /// Null-terminated filename, truncated to 255 bytes + NUL
    pub filename: [u8; 256],
    /// openat(2) flags argument
    pub flags: i32,
}

#[cfg_attr(feature = "user", derive(serde::Serialize, serde::Deserialize))]
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct ExecveEvent {
    pub tgid: u32,
    pub pid: u32,
    pub uid: u32,
    /// Current task comm string (up to 15 chars + NUL)
    pub comm: [u8; 16],
    /// Null-terminated executable path
    pub filename: [u8; 256],
}

impl OpenAtEvent {
    /// Decode the filename bytes to a UTF-8 string, stopping at the first NUL.
    #[cfg(feature = "user")]
    pub fn filename_str(&self) -> &str {
        let end = self.filename.iter().position(|&b| b == 0).unwrap_or(256);
        core::str::from_utf8(&self.filename[..end]).unwrap_or("<invalid utf-8>")
    }
}

impl ExecveEvent {
    #[cfg(feature = "user")]
    pub fn filename_str(&self) -> &str {
        let end = self.filename.iter().position(|&b| b == 0).unwrap_or(256);
        core::str::from_utf8(&self.filename[..end]).unwrap_or("<invalid utf-8>")
    }

    #[cfg(feature = "user")]
    pub fn comm_str(&self) -> &str {
        let end = self.comm.iter().position(|&b| b == 0).unwrap_or(16);
        core::str::from_utf8(&self.comm[..end]).unwrap_or("<invalid utf-8>")
    }
}
