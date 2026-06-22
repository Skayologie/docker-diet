//! Shared event types that cross the kernel↔userspace boundary.
//!
//! Compiled with `#![no_std]` for the eBPF target; the `user` feature
//! re-enables std string helpers for the CLI crate.
//! Serde is intentionally omitted: these types are read from eBPF ring
//! buffers via raw pointer casting, not via serialisation.

#![cfg_attr(not(feature = "user"), no_std)]

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OpenAtEvent {
    pub tgid: u32,
    pub pid: u32,
    pub uid: u32,
    pub filename: [u8; 256],
    pub flags: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ExecveEvent {
    pub tgid: u32,
    pub pid: u32,
    pub uid: u32,
    pub comm: [u8; 16],
    pub filename: [u8; 256],
}

#[cfg(feature = "user")]
impl OpenAtEvent {
    /// Decode the filename bytes to a UTF-8 str, stopping at the first NUL.
    pub fn filename_str(&self) -> &str {
        let end = self.filename.iter().position(|&b| b == 0).unwrap_or(256);
        core::str::from_utf8(&self.filename[..end]).unwrap_or("<invalid utf-8>")
    }
}

#[cfg(feature = "user")]
impl ExecveEvent {
    pub fn filename_str(&self) -> &str {
        let end = self.filename.iter().position(|&b| b == 0).unwrap_or(256);
        core::str::from_utf8(&self.filename[..end]).unwrap_or("<invalid utf-8>")
    }

    pub fn comm_str(&self) -> &str {
        let end = self.comm.iter().position(|&b| b == 0).unwrap_or(16);
        core::str::from_utf8(&self.comm[..end]).unwrap_or("<invalid utf-8>")
    }
}
