//! Redirect the native side's `eprintln!` (process stderr) to a log file.
//!
//! All native diagnostics use `eprintln!`, which writes to the process stderr.
//! When Minecraft is launched normally (not from a console), that stderr is not
//! captured by log4j's `latest.log`/`debug.log`, so the `[windowmod]` lines are
//! lost and impossible to inspect afterwards.
//!
//! `init_once()` opens (truncating) a `windowmod_native.log` file in the current
//! working directory and points the process STDERR handle at it via
//! `SetStdHandle` + `freopen`-equivalent. After this, every `eprintln!` from any
//! thread lands in that file, so we can read it back later.

use std::sync::Once;

use windows::core::PCWSTR;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE,
};
use windows::Win32::System::Console::{SetStdHandle, STD_ERROR_HANDLE};

/// GENERIC_WRITE access right (winnt.h). Passed as a raw u32 to avoid depending
/// on the exact newtype used by this `windows` crate version's CreateFileW.
const GENERIC_WRITE_ACCESS: u32 = 0x4000_0000;


static INIT: Once = Once::new();

/// Redirect process stderr to `windowmod_native.log` (idempotent / once only).
///
/// Safe to call multiple times; only the first call has any effect. Failures are
/// swallowed (logging is best-effort): if the file cannot be opened we simply
/// leave stderr untouched.
pub fn init_once() {
    INIT.call_once(|| {
        // File path: current working directory (which is `run/` under gradle).
        let name: Vec<u16> = "windowmod_native.log"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        let handle: HANDLE = unsafe {
            match CreateFileW(
                PCWSTR(name.as_ptr()),
                GENERIC_WRITE_ACCESS,
                FILE_SHARE_READ | FILE_SHARE_WRITE,

                None,
                CREATE_ALWAYS,
                FILE_ATTRIBUTE_NORMAL,
                None,
            ) {
                Ok(h) => h,
                Err(e) => {
                    eprintln!("[windowmod] logsink: CreateFileW failed: {e}");
                    return;
                }
            }
        };

        // Point the process's STD_ERROR_HANDLE at the file so the Win32 layer
        // and anything querying GetStdHandle(STD_ERROR_HANDLE) sees it.
        unsafe {
            if let Err(e) = SetStdHandle(STD_ERROR_HANDLE, handle) {
                eprintln!("[windowmod] logsink: SetStdHandle failed: {e}");
                return;
            }
        }

        // Rust's `eprintln!` writes through the C runtime / std Stderr, which on
        // Windows is bound to file descriptor 2. Rebind fd 2 to the new handle
        // so Rust's own stderr writes also reach the file.
        rebind_rust_stderr(handle);

        eprintln!("[windowmod] logsink: native stderr now redirected to windowmod_native.log");
    });
}

/// Bind C runtime file descriptor 2 (stderr) to the given OS handle so Rust's
/// `std::io::stderr()` (used by `eprintln!`) writes to the same file.
fn rebind_rust_stderr(handle: HANDLE) {
    extern "C" {
        fn _open_osfhandle(osfhandle: isize, flags: i32) -> i32;
        fn _dup2(fd1: i32, fd2: i32) -> i32;
    }
    const O_WRONLY: i32 = 0x0001;
    const O_APPEND: i32 = 0x0008;
    unsafe {
        let fd = _open_osfhandle(handle.0 as isize, O_WRONLY | O_APPEND);
        if fd >= 0 {
            // 2 == stderr file descriptor.
            let _ = _dup2(fd, 2);
        }
    }
}
