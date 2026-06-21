//! eBPF kernel-space probe for docker-diet.
//!
//! Compiled with `cargo build --target bpfel-unknown-none -Z build-std=core`.
//! Two tracepoints are attached:
//!   • syscalls/sys_enter_openat  → captures every file opened at runtime
//!   • syscalls/sys_enter_execve  → captures every binary executed at runtime
//!
//! Events are forwarded to userspace via per-tracepoint ring buffers.

#![no_std]
#![no_main]

use aya_ebpf::{
    helpers::{bpf_get_current_pid_tgid, bpf_get_current_uid_gid, bpf_probe_read_user_str_bytes},
    macros::{map, tracepoint},
    maps::RingBuf,
    programs::TracePointContext,
};
use diet_ebpf_common::{ExecveEvent, OpenAtEvent};

// ─── Ring-buffer maps ────────────────────────────────────────────────────────

#[map]
static OPENAT_EVENTS: RingBuf = RingBuf::with_byte_size(1024 * 1024, 0);

#[map]
static EXECVE_EVENTS: RingBuf = RingBuf::with_byte_size(256 * 1024, 0);

// ─── sys_enter_openat ────────────────────────────────────────────────────────
//
// tracepoint format (x86-64 kernel, 64-bit pointers):
//   offset  0  common_type          u16
//   offset  2  common_flags         u8
//   offset  3  common_preempt_count u8
//   offset  4  common_pid           i32
//   offset  8  __syscall_nr         i32
//   offset 16  dfd                  u64  (AT_FDCWD = -100)
//   offset 24  filename             u64  (pointer)
//   offset 32  flags                u64
//   offset 40  mode                 u64

#[tracepoint]
pub fn handle_openat(ctx: TracePointContext) -> u32 {
    match try_openat(ctx) {
        Ok(_) => 0,
        Err(_) => 1,
    }
}

#[inline(always)]
fn try_openat(ctx: TracePointContext) -> Result<(), i64> {
    let pid_tgid = bpf_get_current_pid_tgid();
    let tgid = (pid_tgid >> 32) as u32;
    let pid = pid_tgid as u32;
    let uid_gid = bpf_get_current_uid_gid();
    let uid = uid_gid as u32;

    let filename_ptr: u64 = unsafe { ctx.read_at(24)? };
    let flags: i32 = unsafe { ctx.read_at(32)? };

    let mut event = OpenAtEvent {
        tgid,
        pid,
        uid,
        filename: [0u8; 256],
        flags,
    };

    unsafe {
        let _ = bpf_probe_read_user_str_bytes(filename_ptr as *const u8, &mut event.filename);
    }

    if let Some(mut slot) = OPENAT_EVENTS.reserve::<OpenAtEvent>(0) {
        unsafe {
            *slot.as_mut_ptr() = event;
        }
        slot.submit(0);
    }

    Ok(())
}

// ─── sys_enter_execve ────────────────────────────────────────────────────────
//
// tracepoint format:
//   offset  8  __syscall_nr   i32
//   offset 16  filename       u64  (pointer)
//   offset 24  argv           u64  (pointer to pointer array)
//   offset 32  envp           u64  (pointer to pointer array)

#[tracepoint]
pub fn handle_execve(ctx: TracePointContext) -> u32 {
    match try_execve(ctx) {
        Ok(_) => 0,
        Err(_) => 1,
    }
}

#[inline(always)]
fn try_execve(ctx: TracePointContext) -> Result<(), i64> {
    let pid_tgid = bpf_get_current_pid_tgid();
    let tgid = (pid_tgid >> 32) as u32;
    let pid = pid_tgid as u32;
    let uid_gid = bpf_get_current_uid_gid();
    let uid = uid_gid as u32;

    let filename_ptr: u64 = unsafe { ctx.read_at(16)? };

    let mut event = ExecveEvent {
        tgid,
        pid,
        uid,
        comm: [0u8; 16],
        filename: [0u8; 256],
    };

    unsafe {
        let _ = bpf_probe_read_user_str_bytes(filename_ptr as *const u8, &mut event.filename);
    }

    if let Some(mut slot) = EXECVE_EVENTS.reserve::<ExecveEvent>(0) {
        unsafe {
            *slot.as_mut_ptr() = event;
        }
        slot.submit(0);
    }

    Ok(())
}

// ─── Panic handler (required for no_std) ────────────────────────────────────

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
